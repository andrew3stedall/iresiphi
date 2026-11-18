//! Local adapters preserve source observations and enforce admission before publication.
use crate::{
    model::*,
    scalar::{Decimal, Rounding},
};
use calamine::{Data, Reader, Xlsx};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    fmt,
    fs::File,
    io::{Cursor, Read},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};

#[path = "sqlite.rs"]
mod sqlite;

fn err(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, "source", message)
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(err("CANCELLED", "source read cancelled"))
    } else {
        Ok(())
    }
}
fn limit(ok: bool, what: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(err("RESOURCE_LIMIT", what))
    }
}
fn io_error(e: impl fmt::Display) -> Error {
    err("SOURCE_ACCESS", e.to_string())
}

/// Open below an approved root without following any path-component symlink.
fn open_permitted(path: &Path, roots: &[std::path::PathBuf]) -> Result<File> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(io_error)?.join(path)
    };
    if absolute
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(err("POLICY_DENIED", "parent path components are forbidden"));
    }
    for root in roots {
        let root = std::fs::canonicalize(root).map_err(io_error)?;
        let Ok(relative) = absolute.strip_prefix(&root) else {
            continue;
        };
        let root_c = CString::new(root.as_os_str().as_encoded_bytes()).map_err(io_error)?;
        let fd = unsafe {
            libc::open(
                root_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let parts: Vec<_> = relative
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .collect();
        if parts.is_empty() {
            return Err(err("SOURCE_ACCESS", "source must be a regular file"));
        }
        for (index, part) in parts.iter().enumerate() {
            let Component::Normal(name) = part else {
                return Err(err("POLICY_DENIED", "invalid source path"));
            };
            let name = CString::new(name.as_encoded_bytes()).map_err(io_error)?;
            let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
            if index + 1 < parts.len() {
                flags |= libc::O_DIRECTORY;
            }
            let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(err(
                    "POLICY_DENIED",
                    std::io::Error::last_os_error().to_string(),
                ));
            }
            file = unsafe { File::from_raw_fd(fd) };
        }
        if !file.metadata().map_err(io_error)?.is_file() {
            return Err(err("SOURCE_ACCESS", "source must be a regular file"));
        }
        return Ok(file);
    }
    Err(err(
        "POLICY_DENIED",
        "source lies outside allowed read roots",
    ))
}

fn read_bytes(
    binding: &Binding,
    policy: &Policy,
    total: &mut usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    let mut file = open_permitted(&binding.path, &policy.allowed_read_roots)?;
    let before = file.metadata().map_err(io_error)?;
    limit(
        before.len() <= policy.limits.file_bytes as u64,
        "encoded file bytes",
    )?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut chunk).map_err(io_error)?;
        if n == 0 {
            break;
        }
        limit(
            bytes.len().saturating_add(n) <= policy.limits.file_bytes,
            "encoded file bytes",
        )?;
        *total = total.saturating_add(n);
        limit(
            *total <= policy.limits.input_bytes,
            "aggregate encoded input bytes",
        )?;
        bytes.extend_from_slice(&chunk[..n]);
    }
    let after = file.metadata().map_err(io_error)?;
    if (
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    ) != (
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
        after.ctime(),
        after.ctime_nsec(),
    ) {
        return Err(err(
            "SOURCE_CHANGED",
            "source changed while reading; materialization is not certified",
        ));
    }
    Ok(bytes)
}

/// A stable, path-free identity for one binding: a digest of its adapter, supplied
/// location and declared settings. Rebinding the same bytes elsewhere yields a new identity.
fn binding_identity(binding: &Binding) -> Result<String> {
    let declared = serde_json::to_vec(&(
        &binding.adapter,
        binding.path.as_os_str().as_encoded_bytes(),
        &binding.settings,
    ))
    .map_err(io_error)?;
    Ok(format!(
        "binding:{}",
        &crate::validation::hex(&Sha256::digest(&declared))[..32]
    ))
}

/// JSON and CSV dialects are explicit: an unknown (for example misspelt) setting fails
/// rather than silently falling back to a default.
fn check_text_settings(binding: &Binding) -> Result<()> {
    let allowed: &[&str] = match binding.adapter.as_str() {
        "json" => &[],
        "csv" => &["delimiter", "quote", "null_tokens"],
        "sqlite" => &["table"],
        _ => return Ok(()),
    };
    match binding
        .settings
        .keys()
        .find(|k| !allowed.contains(&k.as_str()))
    {
        Some(key) => Err(err(
            "INVALID_BINDING",
            format!("unsupported {} setting {key}", binding.adapter),
        )),
        None => Ok(()),
    }
}

/// One admitted immutable SQLite image for each exact declared path spelling.
struct SqliteSnapshot {
    owner: String,
    access_us: u64,
    bytes: Vec<u8>,
    fingerprint: String,
    conn: rusqlite::Connection,
}
fn sqlite_snapshots(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    total: &mut usize,
    cancel: &AtomicBool,
) -> Result<BTreeMap<Vec<u8>, SqliteSnapshot>> {
    let mut snapshots = BTreeMap::new();
    for source in &recipe.sources {
        let binding = bindings
            .get(&source.id)
            .ok_or_else(|| err("MISSING_BINDING", &source.id))?;
        if binding.adapter != "sqlite" {
            continue;
        }
        let admitted = || -> Result<()> {
            check_text_settings(binding)?;
            // Every binding is independently admitted even when its bytes are shared.
            let file = open_permitted(&binding.path, &policy.allowed_read_roots)?;
            limit(
                file.metadata().map_err(io_error)?.len() <= policy.limits.file_bytes as u64,
                "encoded file bytes",
            )
        };
        admitted().map_err(|e| tag_source(e, &source.id))?;
        let key = binding.path.as_os_str().as_encoded_bytes().to_vec();
        if !snapshots.contains_key(&key) {
            let accessed = std::time::Instant::now();
            let bytes = read_bytes(binding, policy, total, cancel)
                .map_err(|e| tag_source(e, &source.id))?;
            let fingerprint = crate::validation::hex(&Sha256::digest(&bytes));
            let conn = sqlite::open(&bytes).map_err(|e| tag_source(e, &source.id))?;
            snapshots.insert(
                key,
                SqliteSnapshot {
                    owner: source.id.clone(),
                    access_us: micros(accessed),
                    bytes,
                    fingerprint,
                    conn,
                },
            );
        }
    }
    Ok(snapshots)
}

/// Read immutable materializations. The public executor converts these owned rows to typed batches.
/// This adapter boundary is bounded materialization, not a zero-copy or streaming guarantee.
fn materialize_sources(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
    mut reads: Option<&mut BTreeMap<String, SourceRead>>,
    streamed: &BTreeSet<String>,
    spillable: &BTreeSet<String>,
    periods_only: &BTreeSet<String>,
) -> Result<(BTreeMap<String, Vec<Row>>, Streams)> {
    let mut result = BTreeMap::new();
    let mut deferred = BTreeMap::new();
    let mut targets = BTreeMap::new();
    let mut periods = BTreeMap::new();
    let mut period_records = 0usize;
    let mut total = 0;
    let mut expanded = 0;
    let mut retained = 0usize;
    // Placement is decided from declarations and policy before any source opens.
    let plan = crate::placement::plan(recipe, bindings, policy)?;
    let snapshots = sqlite_snapshots(recipe, bindings, policy, &mut total, cancel)?;
    let mut fallbacks = BTreeMap::<String, sqlite::Fallback>::new();
    // Connected candidate groups include all sibling targets, even a candidate
    // kept local by static planning. Finish every check before delivering rows.
    let mut remaining = plan.candidates.clone();
    while let Some((a, b)) = remaining.pop() {
        let mut group = BTreeSet::from([a, b]);
        loop {
            let before = group.len();
            remaining.retain(|(a, b)| {
                if group.contains(a) || group.contains(b) {
                    group.insert(a.clone());
                    group.insert(b.clone());
                    false
                } else {
                    true
                }
            });
            if group.len() == before {
                break;
            }
        }
        if !group.iter().any(|id| plan.reductions.contains_key(id)) {
            continue;
        }
        let mut fallback = None;
        for source in recipe.sources.iter().filter(|s| group.contains(&s.id)) {
            check_cancel(cancel)?;
            let binding = &bindings[&source.id];
            let snapshot = &snapshots[binding.path.as_os_str().as_encoded_bytes()];
            if let Some(reason) = sqlite::preflight(&snapshot.conn, source, binding, policy, cancel)
                .map_err(|e| tag_source(e, &source.id))?
            {
                if policy.placement == Placement::RequireSource && reason.is_encoding() {
                    let pushed = group
                        .iter()
                        .filter(|id| plan.reductions.contains_key(*id))
                        .find_map(|id| plan.pushed.get(id))
                        .unwrap();
                    return Err(Error::new(
                        "PLACEMENT_INFEASIBLE",
                        "source",
                        format!(
                            "policy placement require_source cannot be met for {}: {reason}",
                            pushed.stage
                        ),
                    )
                    .with_subject(pushed.stage.clone())
                    .with_remediation(
                        "Use placement prefer_source or local; the policy is not overridden.",
                    ));
                }
                fallback.get_or_insert(reason);
            }
        }
        if let Some(reason) = fallback {
            for id in group {
                fallbacks.insert(id, reason.clone());
            }
        }
    }
    // A transfer target is read after every other source, so the keys of its
    // delivered from records exist before its scan (docs/spec/slice-5.md).
    let ordered = recipe
        .sources
        .iter()
        .filter(|s| !plan.transfers.contains_key(&s.id))
        .chain(
            recipe
                .sources
                .iter()
                .filter(|s| plan.transfers.contains_key(&s.id)),
        );
    for source in ordered {
        check_cancel(cancel).map_err(|e| tag_source(e, &source.id))?;
        let binding = bindings
            .get(&source.id)
            .ok_or_else(|| err("MISSING_BINDING", format!("missing binding {}", source.id)))?;
        let subject = |e: Error| tag_source(e, &source.id);
        check_text_settings(binding).map_err(subject)?;
        // Build the transferred key condition, or the reason it stays local.
        let mut transferred = None;
        let mut transfer_fallback = None;
        let mut transfer_pushed = None;
        if let Some(transfer) = plan.transfers.get(&source.id) {
            let from: &Vec<Row> = result.get(&transfer.from).ok_or_else(|| {
                err(
                    "MISSING_BINDING",
                    format!("missing binding {}", transfer.from),
                )
            })?;
            let keys = transfer.keys(from).map_err(subject)?;
            let values = keys.len().saturating_mul(transfer.from_keys.len());
            let allowed = policy
                .limits
                .transfer_keys
                .min(crate::placement::TRANSFER_PARAMETERS);
            if values > allowed {
                let reason = format!(
                    "the transfer {} would send {values} key values, above limits.transfer_keys ({allowed})",
                    transfer.id
                );
                if policy.placement == Placement::RequireSource {
                    return Err(Error::new(
                        "PLACEMENT_INFEASIBLE",
                        "source",
                        format!(
                            "policy placement require_source cannot be met for {}: {reason}",
                            transfer.stage
                        ),
                    )
                    .with_subject(transfer.stage.clone())
                    .with_remediation(
                        "Raise limits.transfer_keys, or use placement prefer_source or local, or transfers none; the policy is not overridden.",
                    ));
                }
                transfer_pushed = Some(transfer.pushed(Default::default()));
                transfer_fallback = Some(sqlite::Fallback::Data(reason));
            } else {
                transferred = Some(serde_json::json!({
                    "id": transfer.id,
                    "from": format!("source:{}", transfer.from),
                    "keys": keys.len(),
                    "key_values": values,
                    "mechanism": "bound query parameters on the in-memory snapshot; nothing was written or staged",
                }));
                transfer_pushed = Some(transfer.pushed(keys));
            }
        }
        let accessed = std::time::Instant::now();
        let snapshot = snapshots
            .get(binding.path.as_os_str().as_encoded_bytes())
            .filter(|_| binding.adapter == "sqlite");
        let owned_bytes = if snapshot.is_some() {
            Vec::new()
        } else {
            read_bytes(binding, policy, &mut total, cancel).map_err(subject)?
        };
        let bytes: &[u8] = match snapshot {
            Some(snapshot) => &snapshot.bytes,
            None => &owned_bytes,
        };
        let fingerprint = snapshot
            .map(|s| s.fingerprint.clone())
            .unwrap_or_else(|| crate::validation::hex(&Sha256::digest(bytes)));
        let access_us = snapshot.map_or_else(
            || micros(accessed),
            |s| if s.owner == source.id { s.access_us } else { 0 },
        );
        let parsed = std::time::Instant::now();
        let reference = SourceReference {
            source: source.id.as_str().into(),
            binding: binding_identity(binding)?.into(),
            fingerprint: fingerprint.into(),
            location: String::new(),
        };
        let max = if recipe
            .outputs
            .iter()
            .any(|o| o.anchor == source.id && !o.detail)
        {
            policy.limits.anchor_rows
        } else if recipe
            .relationships
            .iter()
            .any(|r| r.to == source.id && r.cardinality == Cardinality::OneToMany)
        {
            policy.limits.component_rows
        } else {
            policy.limits.lookup_rows
        };
        let mut admission = policy.clone();
        admission.limits.lookup_rows = max;
        admission.limits.retained_bytes = policy.limits.retained_bytes.saturating_sub(retained);
        let mut scan = None;
        // A streamed anchor keeps only its key records (docs/spec/slice-7.md, phase 3).
        let streaming = streamed.contains(&source.id);
        // A lookup target or a one-to-many detail over limits.target_bytes
        // keeps only its key records (docs/spec/slice-7.md, phases 4 and 5).
        // A SQLite target is queried again at its snapshot when placement
        // allows work at the source.
        let spilling = !streaming && spillable.contains(&source.id);
        // A source read only for a population's periods keeps only its
        // distinct periods (phase 6). Its first read checks every record and
        // keeps key records until those periods are taken.
        let period_only = !streaming && !spilling && periods_only.contains(&source.id);
        let at_source = binding.adapter == "sqlite" && policy.placement != Placement::Local;
        let mut held = RowCollector::new(&admission);
        let mut keyed =
            (streaming || period_only).then(|| KeyCollector::new(recipe, source, &admission));
        let mut target =
            spilling.then(|| TargetCollector::new(recipe, source, &admission, at_source));
        let sink: &mut dyn RowSink = match (keyed.as_mut(), target.as_mut()) {
            (Some(keyed), _) => keyed,
            (None, Some(target)) => target,
            (None, None) => &mut held,
        };
        match binding.adapter.as_str() {
            "sqlite" => sqlite::read(
                source,
                binding,
                &snapshot.unwrap().conn,
                &reference,
                &admission,
                cancel,
                transfer_pushed.as_ref().or(plan.pushed.get(&source.id)),
                transfer_fallback.as_ref().or(fallbacks.get(&source.id)),
                sink,
            )
            .map(|mut read| {
                if let (Some(placement), Some(transferred)) = (&mut read.placement, transferred) {
                    if placement["scan"] == "source" {
                        placement["reduction"] = serde_json::json!("transferred_keys");
                        placement["transfer"] = transferred;
                    }
                }
                scan = Some((read.total_rows, read.placement));
            }),
            "json" => read_json(source, bytes, &reference, &admission, cancel, sink),
            "csv" => read_csv(source, binding, bytes, &reference, &admission, cancel, sink),
            "xlsx" => read_xlsx(
                source,
                binding,
                bytes,
                &reference,
                &admission,
                &mut expanded,
                cancel,
                sink,
            ),
            _ => return Err(err("UNSUPPORTED_CAPABILITY", "unsupported source adapter")),
        }
        .map_err(subject)?;
        let mut spill = None;
        let (mut rows, conversion_us) = match (keyed, target) {
            (Some(keyed), _) => keyed.finish().map_err(subject)?,
            (None, Some(target)) => {
                let (rows, conversion_us, store) = target.finish().map_err(subject)?;
                spill = store;
                (rows, conversion_us)
            }
            (None, None) => (held.rows, 0),
        };
        let max = if recipe
            .outputs
            .iter()
            .any(|o| o.anchor == source.id && !o.detail)
        {
            policy.limits.anchor_rows
        } else if recipe
            .relationships
            .iter()
            .any(|r| r.to == source.id && r.cardinality == Cardinality::OneToMany)
        {
            policy.limits.component_rows
        } else {
            policy.limits.lookup_rows
        };
        limit(rows.len() <= max, "source row count").map_err(subject)?;
        let mut identities: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut null_identities = Vec::new();
        let mut batch_bytes = 0usize;
        for (i, row) in rows.iter_mut().enumerate() {
            check_cancel(cancel).map_err(subject)?;
            let key: Vec<_> = source
                .identity
                .iter()
                .map(|id| row.fields.get(id).cloned().unwrap_or(Value::Null))
                .collect();
            if key.iter().any(|v| matches!(v, Value::Null)) {
                null_identities.push(i);
            } else {
                let key = serde_json::to_string(&key).map_err(io_error)?;
                identities.entry(key).or_default().push(i);
            }
            let size = crate::admission::serialized_size(row).map_err(io_error)?;
            limit(
                size <= policy.limits.batch_bytes,
                "single decoded record bytes",
            )?;
            if i % policy.limits.batch_rows.max(1) == 0 {
                batch_bytes = 0;
            }
            batch_bytes = batch_bytes.saturating_add(size);
            limit(
                batch_bytes <= policy.limits.batch_bytes,
                "decoded batch bytes",
            )?;
            if !period_only {
                retained = retained.saturating_add(size);
                limit(
                    retained <= policy.limits.retained_bytes,
                    "retained source materialization bytes",
                )?;
            }
        }
        let duplicates: Vec<usize> = identities
            .into_values()
            .filter(|rows| rows.len() > 1)
            .flatten()
            .collect();
        for (code, affected, what) in [
            (
                "NULL_IDENTITY",
                &null_identities,
                "a null identity component",
            ),
            ("DUPLICATE_IDENTITY", &duplicates, "a duplicated identity"),
        ] {
            if affected.is_empty() {
                continue;
            }
            let values = affected
                .iter()
                .take(Error::SAMPLE_CAP)
                .map(|&i| identity_values(&rows[i], &source.identity))
                .collect();
            return Err(err(
                code,
                format!(
                    "{} records of {} have {what}; identity must be present and unique",
                    affected.len(),
                    source.id
                ),
            )
            .with_subject(format!("source:{}", source.id))
            .with_check("data.identity")
            .with_affected(affected.len(), affected.iter().map(|&i| &rows[i].reference))
            .with_values(policy.diagnostic_values, values));
        }
        for field in period_fields(recipe, &source.id) {
            periods
                .entry((source.id.clone(), field.clone()))
                .or_insert_with(|| Periods::from_rows(&rows, field));
        }
        if let Some(reads) = reads.as_deref_mut() {
            reads.insert(
                source.id.clone(),
                SourceRead {
                    adapter: binding.adapter.clone(),
                    capabilities: adapter_capabilities(&binding.adapter),
                    binding: reference.binding.to_string(),
                    fingerprint: reference.fingerprint.to_string(),
                    bytes_read: bytes.len(),
                    shared_read: recipe
                        .sources
                        .iter()
                        .filter(|s| {
                            s.id != source.id
                                && bindings
                                    .get(&s.id)
                                    .is_some_and(|b| crate::placement::colocated(binding, b))
                        })
                        .map(|s| s.id.clone())
                        .collect(),
                    rows: scan.as_ref().map_or(rows.len(), |(total, _)| *total),
                    scanned_rows: scan
                        .as_ref()
                        .filter(|(_, placement)| placement.is_some())
                        .map(|_| rows.len()),
                    placement: scan.and_then(|(_, placement)| placement),
                    access_us,
                    parse_us: micros(parsed).saturating_sub(conversion_us),
                    conversion_us,
                    streamed: streaming.then(|| {
                        serde_json::json!({
                            "kept_fields": key_fields(recipe, source).to_vec(),
                            "reads": 1 + recipe
                                .outputs
                                .iter()
                                .filter(|o| o.anchor == source.id)
                                .count(),
                            "batch_rows": policy.limits.batch_rows.max(1),
                            "statement": "Only the identity and relationship key fields of each record and its reference are held; each output anchored here decodes the same bytes again in batches.",
                        })
                    }),
                    spilled: spill
                        .as_ref()
                        .map(|store| store.describe(recipe, source, rows.len(), policy)),
                    periods: period_only.then(|| {
                        serde_json::json!({
                            "distinct": period_fields(recipe, &source.id)
                                .map(|field| {
                                    let found = &periods[&(source.id.clone(), field.clone())];
                                    (field.clone(), serde_json::json!(found.distinct.len()))
                                })
                                .collect::<serde_json::Map<_, _>>(),
                            "statement": "The source is read only for a population's periods. Every record was checked as a held source's would be; only the distinct periods are kept.",
                        })
                    }),
                },
            );
        }
        if let Some(store) = spill {
            targets.insert(
                source.id.clone(),
                Target {
                    source: source.clone(),
                    binding: binding.clone(),
                    reference: reference.clone(),
                    admission: admission.clone(),
                    store,
                },
            );
        }
        if streaming {
            deferred.insert(
                source.id.clone(),
                Deferred {
                    source: source.clone(),
                    binding: binding.clone(),
                    bytes: owned_bytes,
                    reference: reference.clone(),
                    admission: admission.clone(),
                    pushed: plan.pushed.get(&source.id).cloned(),
                    fallback: fallbacks.get(&source.id).cloned(),
                    rows: rows.len(),
                },
            );
        }
        if period_only {
            period_records += rows.len();
            rows = Vec::new();
        }
        result.insert(source.id.clone(), rows);
    }
    // A streamed SQLite anchor, and a target queried at its source, read the
    // open snapshot again.
    let snapshots = if deferred
        .values()
        .any(|d: &Deferred| d.binding.adapter == "sqlite")
        || targets
            .values()
            .any(|t: &Target| matches!(t.store, Store::Snapshot))
    {
        snapshots
    } else {
        BTreeMap::new()
    };
    Ok((
        result,
        Streams {
            sources: deferred,
            targets,
            snapshots,
            periods,
            period_records,
        },
    ))
}

/// What one run actually read for a source: its safe (path-free) binding identity,
/// the fingerprint of the bytes read, the byte count and the admitted row count.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceRead {
    pub adapter: String,
    #[serde(default)]
    pub capabilities: serde_json::Value,
    pub binding: String,
    pub fingerprint: String,
    pub bytes_read: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_read: Vec<String>,
    /// Records in the source. Under a pushed predicate this still counts every
    /// record of the table; `scanned_rows` counts those that were read.
    pub rows: usize,
    /// Records that crossed into the executor, when a predicate was planned
    /// for this source's scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_rows: Option<usize>,
    /// Where the planned predicate ran, and what the source-side checks covered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<serde_json::Value>,
    /// Opening, reading and fingerprinting the bytes, in microseconds.
    #[serde(default)]
    pub access_us: u64,
    /// Decoding the bytes into typed, identity-checked observations.
    #[serde(default)]
    pub parse_us: u64,
    /// Converting the observations to typed Arrow batches and back.
    #[serde(default)]
    pub conversion_us: u64,
    /// Present when the executor streamed this source as an output anchor: the
    /// fields it held of each record and how many times it read the source
    /// (docs/spec/slice-7.md, phase 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamed: Option<serde_json::Value>,
    /// Present when this relationship target exceeded `limits.target_bytes`: the
    /// fields it held of each record and where its records were read again
    /// (docs/spec/slice-7.md, phase 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spilled: Option<serde_json::Value>,
    /// Present when the source was read only for a population's periods: the
    /// number of distinct periods each field gave (docs/spec/slice-7.md,
    /// phase 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub periods: Option<serde_json::Value>,
}

/// Concrete capabilities of the built-in source readers. Describing a reader
/// never opens its binding or implies that it supports executor pushdown.
pub(crate) fn adapter_capabilities(adapter: &str) -> serde_json::Value {
    if ["json", "csv", "xlsx"].contains(&adapter) {
        serde_json::json!({
            "location": "local", "typed_read": true,
            "pushdown": false, "streaming": false,
            "materialization": "bounded", "snapshot": "none",
            "source_identity": "fingerprint_bytes_read",
        })
    } else if adapter == "sqlite" {
        serde_json::json!({
            "location": "local", "typed_read": true,
            "pushdown": true,
            "pushdown_operations": ["anchor_eligibility_filter", "relationship_target_reduction", "relationship_target_key_transfer"],
            "transfers_accepted": ["keys"],
            "co_location": "equal declared path spelling, byte-for-byte",
            "pushdown_types": ["int64", "string", "date", "boolean"],
            "streaming": false,
            "materialization": "bounded", "snapshot": "bytes_read",
            "source_identity": "fingerprint_bytes_read",
        })
    } else {
        serde_json::Value::Null
    }
}

/// Microseconds elapsed since `started`, saturating.
pub(crate) fn micros(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Attribute a diagnostic to the source being read unless it already names one.
fn tag_source(mut e: Error, source: &str) -> Error {
    if e.subject.is_none() {
        e.subject = Some(format!("source:{source}"));
    }
    e
}

/// Identity components as plain JSON scalars, for opted-in diagnostic values.
fn identity_values(row: &Row, keys: &[String]) -> serde_json::Value {
    serde_json::Value::Object(
        keys.iter()
            .map(|k| {
                (
                    k.clone(),
                    plain_value(row.fields.get(k).unwrap_or(&Value::Null)),
                )
            })
            .collect(),
    )
}

/// A typed value as a plain JSON scalar (numbers stay exact text).
pub fn plain_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Boolean { value } => serde_json::Value::Bool(*value),
        Value::Int64 { value }
        | Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Date { value } => serde_json::Value::String(value.clone()),
    }
}

/// Value failures collected across a whole source, so the diagnostic carries the
/// exact number of affected records and bounded references, not only the first.
/// Structural, policy and budget errors still stop reading at once.
#[derive(Default)]
struct ValueFailures {
    first: Option<Error>,
    codes: BTreeSet<String>,
    count: usize,
    samples: Vec<SourceReference>,
}
impl ValueFailures {
    /// Codes that describe one record's value rather than the source's structure.
    const CODES: [&'static str; 5] = [
        "INVALID_VALUE",
        "INVALID_DECIMAL",
        "INVALID_DATE",
        "NULL_REQUIRED",
        "XLSX_CELL_ERROR",
    ];
    /// Count a record whose value failed, or return any other error unchanged.
    fn record(&mut self, error: Error, reference: &SourceReference) -> Result<()> {
        if !Self::CODES.contains(&error.code.as_str()) {
            return Err(error);
        }
        self.count += 1;
        self.codes.insert(error.code.clone());
        if self.samples.len() < Error::SAMPLE_CAP {
            self.samples.push(reference.clone());
        }
        self.first.get_or_insert(error);
        Ok(())
    }
    /// Preserve a homogeneous failure code; mixed kinds get a source-wide code.
    fn finish(self) -> Result<()> {
        match self.first {
            None => Ok(()),
            Some(error) => {
                let error = if self.codes.len() > 1 {
                    err(
                        "INVALID_VALUE",
                        format!(
                            "source records have multiple value error kinds: {}",
                            self.codes.into_iter().collect::<Vec<_>>().join(", ")
                        ),
                    )
                } else {
                    error
                };
                Err(error.with_affected(self.count, self.samples.iter()))
            }
        }
    }
}

/// Field names shared by every record a source projection reads.
pub(crate) fn projected_names(source: &Source) -> crate::record::FieldNames {
    std::sync::Arc::new(source.projection.fields.keys().cloned().collect())
}
/// An empty record of a projection; filling its fields in name order keeps
/// the shared name list (docs/spec/slice-7.md).
pub(crate) fn blank_row(names: &crate::record::FieldNames, reference: &SourceReference) -> Row {
    Row {
        fields: crate::record::FieldMap::with_names(names.clone()),
        raw: crate::record::FieldMap::with_names(names.clone()),
        reference: reference.clone(),
    }
}
/// Where an adapter delivers the records it decodes, in source order.
pub(crate) trait RowSink {
    fn push(&mut self, row: Row) -> Result<()>;
}

/// Holds every admitted record of a source.
struct RowCollector<'a> {
    rows: Vec<Row>,
    bytes: usize,
    batch_bytes: usize,
    policy: &'a Policy,
}
impl<'a> RowCollector<'a> {
    fn new(policy: &'a Policy) -> Self {
        Self {
            rows: Vec::new(),
            bytes: 0,
            batch_bytes: 0,
            policy,
        }
    }
}
impl RowSink for RowCollector<'_> {
    fn push(&mut self, row: Row) -> Result<()> {
        limit(
            self.rows.len() < self.policy.limits.lookup_rows,
            "source row count",
        )?;
        let size = crate::admission::serialized_size(&row).map_err(io_error)?;
        if self.rows.len() % self.policy.limits.batch_rows.max(1) == 0 {
            self.batch_bytes = 0;
        }
        self.batch_bytes = self.batch_bytes.saturating_add(size);
        self.bytes = self.bytes.saturating_add(size);
        limit(
            self.batch_bytes <= self.policy.limits.batch_bytes,
            "decoded batch bytes",
        )?;
        limit(
            self.bytes <= self.policy.limits.retained_bytes,
            "retained source materialization bytes",
        )?;
        self.rows.push(row);
        Ok(())
    }
}

/// Fields a streamed anchor keeps of each record: its identity, the fields
/// its relationships key on (docs/spec/slice-7.md, phase 3) and the fields a
/// population takes its periods from (phase 6). Lookups, transfers, identity
/// checks, detail parents and period sets read nothing else of an anchor
/// record.
pub(crate) fn key_fields(recipe: &Recipe, source: &Source) -> crate::record::FieldNames {
    let mut names: BTreeSet<&String> = source.identity.iter().collect();
    for rel in recipe.relationships.iter().filter(|r| r.from == source.id) {
        names.extend(rel.from_keys.iter().filter(|k| !k.contains('.')));
        names.extend(rel.validity.as_ref().map(|v| &v.date));
        names.extend(rel.period.as_ref().map(|p| &p.key));
    }
    names.extend(period_fields(recipe, &source.id));
    std::sync::Arc::new(
        names
            .into_iter()
            .filter(|name| source.projection.fields.contains_key(*name))
            .cloned()
            .collect(),
    )
}

/// The fields of `row` named in `keep`, and its reference.
fn key_record(row: &Row, keep: &crate::record::FieldNames) -> Row {
    let mut kept = Row {
        fields: crate::record::FieldMap::with_names(keep.clone()),
        raw: crate::record::FieldMap::new(),
        reference: row.reference.clone(),
    };
    for name in keep.iter() {
        if let Some(value) = row.fields.get(name) {
            kept.fields.insert(name.clone(), value.clone());
        }
    }
    kept
}

/// Fields of source `id` that populations take their periods from.
fn period_fields<'r>(recipe: &'r Recipe, id: &str) -> impl Iterator<Item = &'r String> {
    recipe
        .populations
        .iter()
        .filter_map(|p| p.period.from.as_ref())
        .filter(move |from| from.source == id)
        .map(|from| &from.field)
}

/// The period set one field of a source gives a population
/// (docs/spec/slice-7.md, phase 6): its distinct calendar months, and the
/// records whose value is not one, counted with capped samples.
#[derive(Debug, Default)]
pub(crate) struct Periods {
    pub(crate) distinct: BTreeSet<String>,
    pub(crate) invalid: usize,
    pub(crate) samples: Vec<SourceReference>,
}
impl Periods {
    fn from_rows(rows: &[Row], field: &str) -> Self {
        let mut periods = Self::default();
        for row in rows {
            match row.fields.get(field) {
                Some(Value::String { value }) if crate::validation::calendar_month(value) => {
                    if !periods.distinct.contains(value) {
                        periods.distinct.insert(value.clone());
                    }
                }
                _ => {
                    periods.invalid += 1;
                    if periods.samples.len() < Error::SAMPLE_CAP {
                        periods.samples.push(row.reference.clone());
                    }
                }
            }
        }
        periods
    }
}

/// Fields a relationship target over `limits.target_bytes` keeps of each
/// record (docs/spec/slice-7.md, phases 4 and 5): what a streamed anchor keeps
/// (identity and the keys of its own relationships, so a spilled detail can be
/// a parent), the fields relationships into it key on (`to_keys`, validity
/// interval, period date) and the fields a dotted key of a later relationship
/// reads from it. Matching, cardinality, interval, identity and detail parent
/// checks read nothing else of it.
pub(crate) fn target_key_fields(recipe: &Recipe, source: &Source) -> crate::record::FieldNames {
    let mut names: BTreeSet<String> = key_fields(recipe, source).iter().cloned().collect();
    for rel in recipe.relationships.iter().filter(|r| r.to == source.id) {
        names.extend(rel.to_keys.iter().cloned());
        if let Some(v) = &rel.validity {
            names.insert(v.valid_from.clone());
            names.insert(v.valid_to.clone());
        }
        names.extend(rel.period.as_ref().map(|p| p.date.clone()));
    }
    for rel in &recipe.relationships {
        for key in &rel.from_keys {
            if let Some((dependency, field)) = key.split_once('.') {
                if recipe
                    .relationships
                    .iter()
                    .any(|prior| prior.id == dependency && prior.to == source.id)
                {
                    names.insert(field.to_owned());
                }
            }
        }
    }
    std::sync::Arc::new(
        names
            .into_iter()
            .filter(|name| source.projection.fields.contains_key(name))
            .collect(),
    )
}

/// First read of a relationship target that may exceed `limits.target_bytes`
/// (docs/spec/slice-7.md, phases 4 and 5). Records are held as a held source's are
/// until their serialized bytes pass the budget. From then on every record,
/// including those already held, goes to the spill store (a temporary file,
/// or nothing when the SQLite snapshot is queried again), crosses the
/// typed-batch boundary in batches, and only its key record is kept.
/// `limits.retained_bytes` counts what is kept.
struct TargetCollector<'a> {
    policy: &'a Policy,
    fields: Vec<(String, Field)>,
    keep: crate::record::FieldNames,
    at_source: bool,
    /// Held records, or key records once spilling.
    rows: Vec<Row>,
    spilling: bool,
    file: Option<std::io::BufWriter<File>>,
    /// Start of each written record, and the end of the last.
    offsets: Vec<u64>,
    buffer: Vec<u8>,
    pending: Vec<Row>,
    decoded_bytes: usize,
    bytes: usize,
    batch_bytes: usize,
    conversion_us: u64,
}
impl<'a> TargetCollector<'a> {
    fn new(recipe: &Recipe, source: &Source, policy: &'a Policy, at_source: bool) -> Self {
        Self {
            policy,
            fields: source
                .projection
                .fields
                .iter()
                .map(|(n, f)| (n.clone(), f.clone()))
                .collect(),
            keep: target_key_fields(recipe, source),
            at_source,
            rows: Vec::new(),
            spilling: false,
            file: None,
            offsets: vec![0],
            buffer: Vec::new(),
            pending: Vec::new(),
            decoded_bytes: 0,
            bytes: 0,
            batch_bytes: 0,
            conversion_us: 0,
        }
    }
    /// Switch to spilling: every record held so far is spilled in order.
    fn begin(&mut self) -> Result<()> {
        self.spilling = true;
        if !self.at_source {
            self.file = Some(std::io::BufWriter::new(spill_file()?));
        }
        self.bytes = 0;
        for row in std::mem::take(&mut self.rows) {
            self.spill(row)?;
        }
        Ok(())
    }
    fn spill(&mut self, row: Row) -> Result<()> {
        if let Some(file) = &mut self.file {
            use std::io::Write;
            self.buffer.clear();
            serde_json::to_writer(&mut self.buffer, &row).map_err(io_error)?;
            file.write_all(&self.buffer).map_err(spill_error)?;
            let end = self.offsets[self.offsets.len() - 1] + self.buffer.len() as u64;
            self.offsets.push(end);
        }
        let kept = key_record(&row, &self.keep);
        let size = crate::admission::serialized_size(&kept).map_err(io_error)?;
        self.bytes = self.bytes.saturating_add(size);
        limit(
            self.bytes <= self.policy.limits.retained_bytes,
            "retained source materialization bytes",
        )?;
        self.rows.push(kept);
        self.pending.push(row);
        if self.pending.len() >= self.policy.limits.batch_rows.max(1) {
            self.convert()?;
        }
        Ok(())
    }
    fn convert(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let converted = std::time::Instant::now();
        typed_batch(&self.fields, std::mem::take(&mut self.pending))?;
        self.conversion_us = self.conversion_us.saturating_add(micros(converted));
        Ok(())
    }
    /// The held records, or the key records and the spill store, and the time
    /// spent converting (held records are converted by the caller).
    fn finish(mut self) -> Result<(Vec<Row>, u64, Option<Store>)> {
        self.convert()?;
        if !self.spilling {
            return Ok((self.rows, 0, None));
        }
        let store = match self.file {
            Some(writer) => Store::File {
                file: writer.into_inner().map_err(|e| spill_error(e.error()))?,
                offsets: self.offsets,
            },
            None => Store::Snapshot,
        };
        Ok((self.rows, self.conversion_us, Some(store)))
    }
}
impl RowSink for TargetCollector<'_> {
    fn push(&mut self, row: Row) -> Result<()> {
        limit(
            self.rows.len() < self.policy.limits.lookup_rows,
            "source row count",
        )?;
        let size = crate::admission::serialized_size(&row).map_err(io_error)?;
        if self.rows.len() % self.policy.limits.batch_rows.max(1) == 0 {
            self.batch_bytes = 0;
        }
        self.batch_bytes = self.batch_bytes.saturating_add(size);
        limit(
            self.batch_bytes <= self.policy.limits.batch_bytes,
            "decoded batch bytes",
        )?;
        self.decoded_bytes = self.decoded_bytes.saturating_add(size);
        if !self.spilling && self.decoded_bytes > self.policy.limits.target_bytes {
            self.begin()?;
        }
        if self.spilling {
            return self.spill(row);
        }
        self.bytes = self.bytes.saturating_add(size);
        limit(
            self.bytes <= self.policy.limits.retained_bytes,
            "retained source materialization bytes",
        )?;
        self.rows.push(row);
        Ok(())
    }
}

fn spill_error(e: impl fmt::Display) -> Error {
    err(
        "RESOURCE_LIMIT",
        format!("temporary file for a source over limits.target_bytes: {e}"),
    )
}

/// A new file in the system temporary directory, readable only by this user
/// and removed from the directory as soon as it is open: it holds a lookup
/// target's serialized records for one run and disappears when it is closed.
fn spill_file() -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "iresiphi-spill-{}-{}-{stamp}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(spill_error)?;
    std::fs::remove_file(&path).map_err(spill_error)?;
    Ok(file)
}

/// Where a spilled source's records are read again.
pub(crate) enum Store {
    /// Each record's serialized form in an unnamed temporary file; record `i`
    /// is the bytes `offsets[i]..offsets[i + 1]`.
    File { file: File, offsets: Vec<u64> },
    /// The SQLite snapshot the first read queried, by rowid.
    Snapshot,
}
impl Store {
    fn describe(
        &self,
        recipe: &Recipe,
        source: &Source,
        records: usize,
        policy: &Policy,
    ) -> serde_json::Value {
        let mut described = serde_json::json!({
            "kept_fields": target_key_fields(recipe, source).to_vec(),
            "records": records,
            "target_bytes": policy.limits.target_bytes,
        });
        match self {
            Store::File { offsets, .. } => {
                described["store"] = "temporary_file".into();
                described["spilled_bytes"] = offsets[offsets.len() - 1].into();
                described["statement"] = "The target's records exceeded limits.target_bytes. Only the identity and relationship key fields of each record and its reference are held; every record was written to a temporary file that has no name in any directory, and each batch of anchor records reads the records it needs from there: its own when an output is anchored here, and those it reached through relationships.".into();
            }
            Store::Snapshot => {
                described["store"] = "source".into();
                described["statement"] = "The target's records exceeded limits.target_bytes. Only the identity and relationship key fields of each record and its reference are held; each batch of anchor records queries the records it needs from the source's snapshot by rowid: its own when an output is anchored here, and those it reached through relationships.".into();
            }
        }
        described
    }
}

/// A spilled relationship target of one run (docs/spec/slice-7.md, phases 4
/// and 5).
struct Target {
    source: Source,
    binding: Binding,
    reference: SourceReference,
    admission: Policy,
    store: Store,
}

/// First read of a streamed anchor. Every record is admitted and crosses the
/// typed-batch boundary as a held record would; then only its key fields and
/// its reference are kept. `limits.retained_bytes` counts what is kept.
struct KeyCollector<'a> {
    policy: &'a Policy,
    fields: Vec<(String, Field)>,
    keep: crate::record::FieldNames,
    /// Decoded records waiting for typed conversion; at most one batch.
    pending: Vec<Row>,
    rows: Vec<Row>,
    bytes: usize,
    batch_bytes: usize,
    conversion_us: u64,
}
impl<'a> KeyCollector<'a> {
    fn new(recipe: &Recipe, source: &Source, policy: &'a Policy) -> Self {
        Self {
            policy,
            fields: source
                .projection
                .fields
                .iter()
                .map(|(n, f)| (n.clone(), f.clone()))
                .collect(),
            keep: key_fields(recipe, source),
            pending: Vec::new(),
            rows: Vec::new(),
            bytes: 0,
            batch_bytes: 0,
            conversion_us: 0,
        }
    }
    fn convert(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let converted = std::time::Instant::now();
        typed_batch(&self.fields, std::mem::take(&mut self.pending))?;
        self.conversion_us = self.conversion_us.saturating_add(micros(converted));
        Ok(())
    }
    /// The kept records, in source order, and the time spent converting.
    fn finish(mut self) -> Result<(Vec<Row>, u64)> {
        self.convert()?;
        Ok((self.rows, self.conversion_us))
    }
}
impl RowSink for KeyCollector<'_> {
    fn push(&mut self, row: Row) -> Result<()> {
        limit(
            self.rows.len() < self.policy.limits.lookup_rows,
            "source row count",
        )?;
        let size = crate::admission::serialized_size(&row).map_err(io_error)?;
        if self.rows.len() % self.policy.limits.batch_rows.max(1) == 0 {
            self.batch_bytes = 0;
        }
        self.batch_bytes = self.batch_bytes.saturating_add(size);
        limit(
            self.batch_bytes <= self.policy.limits.batch_bytes,
            "decoded batch bytes",
        )?;
        let kept = key_record(&row, &self.keep);
        let size = crate::admission::serialized_size(&kept).map_err(io_error)?;
        self.bytes = self.bytes.saturating_add(size);
        limit(
            self.bytes <= self.policy.limits.retained_bytes,
            "retained source materialization bytes",
        )?;
        self.rows.push(kept);
        self.pending.push(row);
        if self.pending.len() >= self.policy.limits.batch_rows.max(1) {
            self.convert()?;
        }
        Ok(())
    }
}

/// A later read of a streamed anchor: records reach `each` in batches, with the
/// position of each batch's first record.
struct Batches<'a> {
    rows: Vec<Row>,
    size: usize,
    first: usize,
    each: &'a mut dyn FnMut(usize, &[Row]) -> Result<()>,
}
impl Batches<'_> {
    fn flush(&mut self) -> Result<()> {
        if !self.rows.is_empty() {
            (self.each)(self.first, &self.rows)?;
            self.first += self.rows.len();
            self.rows.clear();
        }
        Ok(())
    }
}
impl RowSink for Batches<'_> {
    fn push(&mut self, row: Row) -> Result<()> {
        self.rows.push(row);
        if self.rows.len() >= self.size {
            self.flush()?;
        }
        Ok(())
    }
}

/// What a later read of one streamed anchor needs: its declarations, the bytes
/// the first read fingerprinted (a SQLite source reads its open snapshot), the
/// admission and the placement that read used.
struct Deferred {
    source: Source,
    binding: Binding,
    bytes: Vec<u8>,
    reference: SourceReference,
    admission: Policy,
    pushed: Option<crate::placement::Pushed>,
    fallback: Option<sqlite::Fallback>,
    rows: usize,
}

/// What a run's first read leaves for later reads (docs/spec/slice-7.md):
/// streamed anchors (phase 3), whose key records the executor holds and whose
/// records each output reads again from here in batches; spilled relationship
/// targets (phases 4 and 5); and the period sets of population period sources
/// (phase 6). Every read decodes the same bytes, so every read sees the
/// records the first read checked.
#[derive(Default)]
pub(crate) struct Streams {
    sources: BTreeMap<String, Deferred>,
    /// Relationship targets over `limits.target_bytes` (phases 4 and 5).
    targets: BTreeMap<String, Target>,
    snapshots: BTreeMap<Vec<u8>, SqliteSnapshot>,
    /// Period sets taken by the first read, by source and field (phase 6).
    periods: BTreeMap<(String, String), Periods>,
    /// Records of sources read only for periods, which are not kept.
    period_records: usize,
}
impl Streams {
    /// The period set field `field` of source `source` gave its first read.
    pub(crate) fn periods(&self, source: &str, field: &str) -> Option<&Periods> {
        self.periods.get(&(source.to_owned(), field.to_owned()))
    }
    /// Records read only for periods; the executor holds none of them.
    pub(crate) fn period_records(&self) -> usize {
        self.period_records
    }
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.sources.contains_key(id)
    }
    /// Whether any source of this run was spilled.
    pub(crate) fn spills(&self) -> bool {
        !self.targets.is_empty()
    }
    /// Whether source `id` was spilled; the executor holds its key records.
    pub(crate) fn spilled(&self, id: &str) -> bool {
        self.targets.contains_key(id)
    }
    /// The records of spilled target `id` at `positions` (ascending, distinct),
    /// exactly as its first read decoded them. `keys` are its key records.
    pub(crate) fn fetch(
        &self,
        id: &str,
        keys: &[Row],
        positions: &[usize],
        cancel: &AtomicBool,
    ) -> Result<Vec<Row>> {
        let target = self
            .targets
            .get(id)
            .ok_or_else(|| err("INVALID_SOURCE", format!("{id} is not spilled")))?;
        let changed = |what: String| {
            err(
                "SOURCE_CHANGED",
                format!("a record of {id} read again {what}"),
            )
            .with_subject(format!("source:{id}"))
        };
        let rows = match &target.store {
            Store::File { file, offsets } => {
                use std::io::{Seek, SeekFrom};
                let mut handle: &File = file;
                let mut rows = Vec::with_capacity(positions.len());
                let mut buffer = Vec::new();
                let mut i = 0;
                // Neighbouring records are read with one read.
                while i < positions.len() {
                    check_cancel(cancel)?;
                    let mut end = i + 1;
                    while end < positions.len() && positions[end] == positions[end - 1] + 1 {
                        end += 1;
                    }
                    let (first, last) = (positions[i], positions[end - 1]);
                    let start = offsets[first];
                    buffer.resize((offsets[last + 1] - start) as usize, 0);
                    handle
                        .seek(SeekFrom::Start(start))
                        .and_then(|_| handle.read_exact(&mut buffer))
                        .map_err(spill_error)?;
                    for position in first..=last {
                        let span = (offsets[position] - start) as usize
                            ..(offsets[position + 1] - start) as usize;
                        rows.push(
                            serde_json::from_slice::<Row>(&buffer[span])
                                .map_err(|e| changed(format!("did not decode: {e}")))?,
                        );
                    }
                    i = end;
                }
                rows
            }
            Store::Snapshot => {
                let rowids = positions
                    .iter()
                    .map(|&p| {
                        keys[p]
                            .reference
                            .location
                            .strip_prefix("rowid:")
                            .and_then(|n| n.parse::<i64>().ok())
                            .ok_or_else(|| changed("has no rowid".into()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let snapshot = self
                    .snapshots
                    .get(target.binding.path.as_os_str().as_encoded_bytes())
                    .ok_or_else(|| err("SOURCE_ACCESS", "SQLite snapshot is not open"))?;
                sqlite::fetch(
                    &target.source,
                    &target.binding,
                    &snapshot.conn,
                    &target.reference,
                    &target.admission,
                    cancel,
                    &rowids,
                )
                .map_err(|e| tag_source(e, id))?
            }
        };
        for (row, &p) in rows.iter().zip(positions) {
            if row.reference.location != keys[p].reference.location {
                return Err(changed(format!(
                    "is at {}, not {}",
                    row.reference.location, keys[p].reference.location
                )));
            }
        }
        Ok(rows)
    }
    /// Decode streamed anchor `id` again and hand its records to `each` in
    /// batches of at most `size`. Errors from `each` are returned unchanged.
    pub(crate) fn read(
        &self,
        id: &str,
        size: usize,
        cancel: &AtomicBool,
        each: &mut dyn FnMut(usize, &[Row]) -> Result<()>,
    ) -> Result<()> {
        let deferred = self
            .sources
            .get(id)
            .ok_or_else(|| err("INVALID_SOURCE", format!("{id} is not streamed")))?;
        let mut batches = Batches {
            rows: Vec::with_capacity(size.max(1)),
            size: size.max(1),
            first: 0,
            each,
        };
        let (source, binding, reference, policy) = (
            &deferred.source,
            &deferred.binding,
            &deferred.reference,
            &deferred.admission,
        );
        // A later read never charges the workbook expansion budget again.
        let mut expanded = 0;
        let read = match binding.adapter.as_str() {
            "sqlite" => {
                let snapshot = self
                    .snapshots
                    .get(binding.path.as_os_str().as_encoded_bytes())
                    .ok_or_else(|| err("SOURCE_ACCESS", "SQLite snapshot is not open"))?;
                sqlite::read(
                    source,
                    binding,
                    &snapshot.conn,
                    reference,
                    policy,
                    cancel,
                    deferred.pushed.as_ref(),
                    deferred.fallback.as_ref(),
                    &mut batches,
                )
                .map(|_| ())
            }
            "json" => read_json(
                source,
                &deferred.bytes,
                reference,
                policy,
                cancel,
                &mut batches,
            ),
            "csv" => read_csv(
                source,
                binding,
                &deferred.bytes,
                reference,
                policy,
                cancel,
                &mut batches,
            ),
            "xlsx" => read_xlsx(
                source,
                binding,
                &deferred.bytes,
                reference,
                policy,
                &mut expanded,
                cancel,
                &mut batches,
            ),
            _ => Err(err("UNSUPPORTED_CAPABILITY", "unsupported source adapter")),
        };
        read.and_then(|()| batches.flush()).map_err(|e| {
            // Cancellation noticed by the reader is the run's cancellation.
            if e.code == "CANCELLED" {
                Error::new("CANCELLED", "execution", "run cancelled")
            } else {
                e
            }
        })?;
        if batches.first != deferred.rows {
            return Err(err(
                "SOURCE_CHANGED",
                format!(
                    "a later read of {id} decoded {} records, the first read {}",
                    batches.first, deferred.rows
                ),
            )
            .with_subject(format!("source:{id}")));
        }
        Ok(())
    }
}

fn value_from_text(text: Option<&str>, field: &Field) -> Result<Value> {
    let Some(text) = text else {
        return if field.nullable {
            Ok(Value::Null)
        } else {
            Err(err("NULL_REQUIRED", "required field is missing or null"))
        };
    };
    let bad = || {
        err(
            "INVALID_VALUE",
            "source value does not satisfy declared type",
        )
    };
    Ok(match field.data_type {
        Type::String => Value::String { value: text.into() },
        Type::Boolean => Value::Boolean {
            value: match text {
                "true" => true,
                "false" => false,
                _ => return Err(bad()),
            },
        },
        Type::Int64 => {
            if text.is_empty() || text.trim() != text {
                return Err(bad());
            }
            let n = text.parse::<i64>().map_err(|_| bad())?;
            Value::Int64 {
                value: n.to_string(),
            }
        }
        Type::Decimal { precision, scale } => {
            let fixed = expand_decimal(text)?;
            let d = Decimal::parse(&fixed, precision, scale)
                .map_err(|e| err("INVALID_DECIMAL", e.to_string()))?;
            Value::Decimal {
                value: d.to_string(),
                precision,
                scale,
            }
        }
        Type::Date => {
            if text.len() != 10 || text.as_bytes()[4] != b'-' || text.as_bytes()[7] != b'-' {
                return Err(bad());
            }
            let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| bad())?;
            if date.format("%Y-%m-%d").to_string() != text {
                return Err(bad());
            }
            Value::Date { value: text.into() }
        }
    })
}

fn expand_decimal(text: &str) -> Result<String> {
    let Some(at) = text.find(['e', 'E']) else {
        return Ok(text.into());
    };
    let exponent = text[at + 1..]
        .parse::<i32>()
        .map_err(|_| err("INVALID_DECIMAL", "invalid exponent"))?;
    if exponent.unsigned_abs() > 1024 {
        return Err(err(
            "INVALID_DECIMAL",
            "decimal exponent exceeds representable range",
        ));
    }
    let mantissa = &text[..at];
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches(['-', '+']);
    let mut split = mantissa.split('.');
    let integer = split.next().unwrap_or("");
    let fraction = split.next().unwrap_or("");
    if integer.is_empty()
        || split.next().is_some()
        || !integer
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(err("INVALID_DECIMAL", "invalid exponent mantissa"));
    }
    let digits = format!("{integer}{fraction}");
    let point = integer.len() as i32 + exponent;
    let value = if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    };
    Ok(if negative { format!("-{value}") } else { value })
}

// A validation pass catches duplicate keys even in unprojected subtrees. Parsing into
// serde_json::Value alone would silently overwrite them.
struct UniqueJson;
/// serde_json's reader API lets cancellation interrupt both parsing passes,
/// including a large unprojected subtree, rather than waiting for a whole tree.
struct JsonReader<'a> {
    bytes: &'a [u8],
    cancel: &'a AtomicBool,
}
impl Read for JsonReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("source read cancelled"));
        }
        self.bytes.read(buffer)
    }
}
fn parse_json<T: serde::de::DeserializeOwned>(bytes: &[u8], cancel: &AtomicBool) -> Result<T> {
    serde_json::from_reader(JsonReader { bytes, cancel }).map_err(|error| {
        if cancel.load(Ordering::Relaxed) {
            err("CANCELLED", "source read cancelled")
        } else if error.to_string().contains("duplicate JSON key") {
            err("DUPLICATE_JSON_KEY", error.to_string())
        } else {
            err("INVALID_JSON", error.to_string())
        }
    })
}
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<UniqueJson, M::Error> {
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    map.next_value::<UniqueJson>()?;
                }
                Ok(UniqueJson)
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<UniqueJson, S::Error> {
                while seq.next_element::<UniqueJson>()?.is_some() {}
                Ok(UniqueJson)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
        }
        d.deserialize_any(V)
    }
}
fn json_preflight(bytes: &[u8], limits: &Limits, cancel: &AtomicBool) -> Result<()> {
    let (mut depth, mut token) = (0usize, 0usize);
    let (mut string, mut escaped) = (false, false);
    for (i, &b) in bytes.iter().enumerate() {
        if i % 65536 == 0 {
            check_cancel(cancel)?;
        }
        if string {
            token += 1;
            if escaped {
                escaped = false
            } else if b == b'\\' {
                escaped = true
            } else if b == b'"' {
                string = false;
                token = 0;
            }
        } else {
            match b {
                b'"' => {
                    string = true;
                    token = 0
                }
                b'{' | b'[' => {
                    depth += 1;
                    token = 0;
                    limit(depth <= limits.json_depth, "JSON nesting depth")?
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    token = 0
                }
                b',' | b':' | b' ' | b'\n' | b'\r' | b'\t' => token = 0,
                _ => token += 1,
            }
        }
        limit(token <= limits.field_bytes, "JSON token bytes")?;
    }
    Ok(())
}
/// Split an RFC 6901 pointer into unescaped reference tokens. A path that is not a
/// pointer is a projection error, never a silently missing (null) field.
fn json_pointer_tokens(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(vec![]);
    }
    let invalid = || {
        err(
            "INVALID_PROJECTION",
            format!("{pointer:?} is not a JSON pointer"),
        )
    };
    let rest = pointer.strip_prefix('/').ok_or_else(invalid)?;
    rest.split('/')
        .map(|token| {
            let mut out = String::with_capacity(token.len());
            let mut chars = token.chars();
            while let Some(c) = chars.next() {
                out.push(match c {
                    '~' => match chars.next() {
                        Some('0') => '~',
                        Some('1') => '/',
                        _ => return Err(invalid()),
                    },
                    c => c,
                });
            }
            Ok(out)
        })
        .collect()
}
/// Resolve a pointer. `None` means absent: an object lacks the key, an array lacks the
/// index, or an ancestor is null. Descending into a present scalar is malformed data.
fn json_lookup<'v>(
    root: &'v serde_json::Value,
    pointer: &str,
) -> Result<Option<&'v serde_json::Value>> {
    let mut current = root;
    for token in json_pointer_tokens(pointer)? {
        current = match current {
            serde_json::Value::Object(map) => match map.get(&token) {
                Some(next) => next,
                None => return Ok(None),
            },
            serde_json::Value::Array(items) => {
                let index = (!token.is_empty()
                    && token.bytes().all(|b| b.is_ascii_digit())
                    && (token == "0" || !token.starts_with('0')))
                .then(|| token.parse::<usize>().ok())
                .flatten()
                .ok_or_else(|| {
                    err(
                        "INVALID_VALUE",
                        format!("{pointer} indexes an array with {token:?}"),
                    )
                })?;
                match items.get(index) {
                    Some(next) => next,
                    None => return Ok(None),
                }
            }
            serde_json::Value::Null => return Ok(None),
            _ => {
                return Err(err(
                    "INVALID_VALUE",
                    format!("{pointer} descends into a JSON scalar"),
                ));
            }
        };
    }
    Ok(Some(current))
}

/// Inspect pointers using borrowed JSON spans. Array records remain raw spans;
/// schema-only validation never allocates their objects or decodes their values.
fn raw_json_lookup<'a>(
    root: &'a serde_json::value::RawValue,
    pointer: &str,
) -> Result<Option<&'a serde_json::value::RawValue>> {
    let mut current = root;
    for token in json_pointer_tokens(pointer)? {
        current = match current.get().trim_start().as_bytes().first() {
            Some(b'{') => {
                let fields: BTreeMap<String, &serde_json::value::RawValue> =
                    serde_json::from_str(current.get())
                        .map_err(|e| err("INVALID_JSON", e.to_string()))?;
                match fields.get(&token).copied() {
                    Some(value) => value,
                    None => return Ok(None),
                }
            }
            Some(b'[') => {
                let index = (!token.is_empty()
                    && token.bytes().all(|b| b.is_ascii_digit())
                    && (token == "0" || !token.starts_with('0')))
                .then(|| token.parse::<usize>().ok())
                .flatten()
                .ok_or_else(|| {
                    err(
                        "INVALID_VALUE",
                        format!("{pointer} indexes an array with {token:?}"),
                    )
                })?;
                let values: Vec<&serde_json::value::RawValue> = serde_json::from_str(current.get())
                    .map_err(|e| err("INVALID_JSON", e.to_string()))?;
                match values.get(index).copied() {
                    Some(value) => value,
                    None => return Ok(None),
                }
            }
            Some(b'n') => return Ok(None),
            _ => {
                return Err(err(
                    "INVALID_VALUE",
                    format!("{pointer} descends into a JSON scalar"),
                ));
            }
        };
    }
    Ok(Some(current))
}
fn read_json(
    source: &Source,
    bytes: &[u8],
    reference: &SourceReference,
    policy: &Policy,
    cancel: &AtomicBool,
    rows: &mut dyn RowSink,
) -> Result<()> {
    for path in source
        .projection
        .array
        .iter()
        .chain(source.projection.fields.values().map(|f| &f.path))
    {
        json_pointer_tokens(path)?;
    }
    json_preflight(bytes, &policy.limits, cancel)?;
    parse_json::<UniqueJson>(bytes, cancel)?;
    let root: serde_json::Value = parse_json(bytes, cancel)?;
    let records: Vec<&serde_json::Value> = match &source.projection.array {
        Some(path) => json_lookup(&root, path)?
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                err(
                    "INVALID_PROJECTION",
                    "declared JSON array is missing or not an array",
                )
            })?
            .iter()
            .collect(),
        None => vec![&root],
    };
    let names = projected_names(source);
    let mut failures = ValueFailures::default();
    for (index, record) in records.into_iter().enumerate() {
        check_cancel(cancel)?;
        let mut row = blank_row(&names, &reference);
        row.reference.location = source
            .projection
            .array
            .as_ref()
            .map(|p| format!("{p}/{index}"))
            .unwrap_or_default();
        let filled = (|| -> Result<()> {
            for (name, field) in &source.projection.fields {
                let raw = json_lookup(if field.header { &root } else { record }, &field.path)?
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let text = match &raw {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n)
                        if matches!(field.data_type, Type::Int64 | Type::Decimal { .. }) =>
                    {
                        Some(n.to_string())
                    }
                    serde_json::Value::Bool(b) if field.data_type == Type::Boolean => {
                        Some(b.to_string())
                    }
                    _ => {
                        return Err(err(
                            "INVALID_VALUE",
                            "JSON value type differs from declared scalar type",
                        ));
                    }
                };
                if matches!(field.data_type, Type::String) && !raw.is_string() && !raw.is_null() {
                    return Err(err("INVALID_VALUE", "JSON string required"));
                }
                let value = value_from_text(text.as_deref(), field)?;
                row.raw.insert(name.clone(), raw);
                row.fields.insert(name.clone(), value);
            }
            Ok(())
        })();
        match filled {
            Ok(()) => rows.push(row)?,
            Err(error) => failures.record(error, &row.reference)?,
        }
    }
    failures.finish()?;
    Ok(())
}

fn setting_byte(binding: &Binding, key: &str, default: u8) -> Result<u8> {
    match binding.settings.get(key) {
        None => Ok(default),
        Some(serde_json::Value::String(s)) if s.len() == 1 && s.is_ascii() => Ok(s.as_bytes()[0]),
        _ => Err(err(
            "INVALID_BINDING",
            format!("{key} must be one ASCII byte"),
        )),
    }
}

/// Explicit CSV dialect: UTF-8, one header record, RFC 4180 quoting, LF or CRLF endings.
struct CsvDialect {
    delimiter: u8,
    quote: u8,
    nulls: Vec<String>,
}
impl CsvDialect {
    fn from_binding(binding: &Binding) -> Result<Self> {
        let delimiter = setting_byte(binding, "delimiter", b',')?;
        let quote = setting_byte(binding, "quote", b'"')?;
        let reserved = [b'\r', b'\n', b'\\'];
        if delimiter == quote || reserved.contains(&delimiter) || reserved.contains(&quote) {
            return Err(err(
                "INVALID_BINDING",
                "CSV delimiter and quote must differ and must not be a line ending or backslash",
            ));
        }
        let nulls = match binding.settings.get("null_tokens") {
            None => vec!["\\N".to_owned()],
            Some(serde_json::Value::Array(tokens)) => tokens
                .iter()
                .map(|t| match t.as_str() {
                    Some(t) if !t.is_empty() => Ok(t.to_owned()),
                    _ => Err(err(
                        "INVALID_BINDING",
                        "null_tokens must be non-empty strings; empty text is never null",
                    )),
                })
                .collect::<Result<_>>()?,
            Some(_) => {
                return Err(err(
                    "INVALID_BINDING",
                    "null_tokens must be an array of strings",
                ));
            }
        };
        Ok(Self {
            delimiter,
            quote,
            nulls,
        })
    }
}

/// Strict RFC 4180 record reader. Unlike lenient readers it rejects unterminated or
/// stray quotes, text after a closing quote and bare carriage returns, and it never
/// skips blank lines (they are one-field records, so they fail the width check).
/// Field bytes are bounded while scanning and cancellation is checked between chunks.
struct CsvRecords<'a> {
    bytes: &'a [u8],
    pos: usize,
    dialect: &'a CsvDialect,
    field_bytes: usize,
    cancel: &'a AtomicBool,
    next_check: usize,
}
impl<'a> CsvRecords<'a> {
    fn new(
        bytes: &'a [u8],
        dialect: &'a CsvDialect,
        field_bytes: usize,
        cancel: &'a AtomicBool,
    ) -> Self {
        Self {
            bytes: bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes),
            pos: 0,
            dialect,
            field_bytes,
            cancel,
            next_check: 0,
        }
    }
    fn invalid(&self, what: &str) -> Error {
        err("INVALID_CSV", format!("{what} at byte {}", self.pos))
    }
    fn push(&self, field: &mut Vec<u8>, b: u8) -> Result<()> {
        limit(field.len() < self.field_bytes, "CSV field bytes")?;
        field.push(b);
        Ok(())
    }
    fn next_record(&mut self) -> Result<Option<Vec<String>>> {
        if self.pos >= self.bytes.len() {
            return Ok(None);
        }
        let (delimiter, quote) = (self.dialect.delimiter, self.dialect.quote);
        let mut record = Vec::new();
        loop {
            let mut field = Vec::new();
            let mut quoted = false;
            if self.bytes.get(self.pos) == Some(&quote) {
                quoted = true;
                self.pos += 1;
                loop {
                    if self.pos >= self.next_check {
                        check_cancel(self.cancel)?;
                        self.next_check = self.pos + 65536;
                    }
                    match self.bytes.get(self.pos) {
                        None => return Err(self.invalid("unterminated quoted field")),
                        Some(&b) if b == quote => {
                            if self.bytes.get(self.pos + 1) == Some(&quote) {
                                self.push(&mut field, quote)?;
                                self.pos += 2;
                            } else {
                                self.pos += 1;
                                break;
                            }
                        }
                        Some(&b) => {
                            self.push(&mut field, b)?;
                            self.pos += 1;
                        }
                    }
                }
            }
            // After an unquoted run or a closing quote: delimiter, line ending or EOF.
            let end_of_record = loop {
                if self.pos >= self.next_check {
                    check_cancel(self.cancel)?;
                    self.next_check = self.pos + 65536;
                }
                match self.bytes.get(self.pos) {
                    None => break true,
                    Some(&b) if b == delimiter => {
                        self.pos += 1;
                        break false;
                    }
                    Some(b'\n') => {
                        self.pos += 1;
                        break true;
                    }
                    Some(b'\r') if self.bytes.get(self.pos + 1) == Some(&b'\n') => {
                        self.pos += 2;
                        break true;
                    }
                    Some(b'\r') => return Err(self.invalid("bare carriage return")),
                    Some(_) if quoted => return Err(self.invalid("text after closing quote")),
                    Some(&b) if b == quote => return Err(self.invalid("quote in unquoted field")),
                    Some(&b) => {
                        self.push(&mut field, b)?;
                        self.pos += 1;
                    }
                }
            };
            record.push(String::from_utf8(field).map_err(|_| self.invalid("invalid UTF-8"))?);
            if end_of_record {
                return Ok(Some(record));
            }
        }
    }
}

fn read_csv(
    source: &Source,
    binding: &Binding,
    bytes: &[u8],
    reference: &SourceReference,
    policy: &Policy,
    cancel: &AtomicBool,
    rows: &mut dyn RowSink,
) -> Result<()> {
    let dialect = CsvDialect::from_binding(binding)?;
    let mut reader = CsvRecords::new(bytes, &dialect, policy.limits.field_bytes, cancel);
    let headers = reader
        .next_record()?
        .ok_or_else(|| err("INVALID_CSV", "CSV header record is required"))?;
    let mut seen = BTreeSet::new();
    if headers.iter().any(|h| !seen.insert(h)) {
        return Err(err("INVALID_CSV", "duplicate header"));
    }
    let columns: Vec<_> = source
        .projection
        .fields
        .iter()
        .map(|(name, f)| {
            let path = f.path.strip_prefix('/').unwrap_or(&f.path);
            headers
                .iter()
                .position(|h| h == path)
                .map(|i| (name, f, i))
                .ok_or_else(|| err("INVALID_PROJECTION", format!("missing CSV column {path}")))
        })
        .collect::<Result<_>>()?;
    let names = projected_names(source);
    let mut failures = ValueFailures::default();
    let mut index = 0usize;
    while let Some(record) = reader.next_record()? {
        check_cancel(cancel)?;
        let number = index + 2;
        index += 1;
        if record.len() != headers.len() {
            return Err(err(
                "INVALID_CSV",
                format!(
                    "record {number} has {} fields; header declares {}",
                    record.len(),
                    headers.len()
                ),
            ));
        }
        let mut row = blank_row(&names, &reference);
        row.reference.location = format!("record:{number}");
        let filled = (|| -> Result<()> {
            for (name, field, col) in &columns {
                let original = record[*col].as_str();
                let decoded = if original.starts_with("\\\\") {
                    &original[1..]
                } else {
                    original
                };
                let text = if dialect.nulls.iter().any(|n| n == original) {
                    None
                } else {
                    Some(decoded)
                };
                row.fields
                    .insert((*name).clone(), value_from_text(text, field)?);
                row.raw
                    .insert((*name).clone(), serde_json::Value::String(original.into()));
            }
            Ok(())
        })();
        match filled {
            Ok(()) => rows.push(row)?,
            Err(error) => failures.record(error, &row.reference)?,
        }
    }
    failures.finish()?;
    Ok(())
}

fn xml_attr(event: &quick_xml::events::BytesStart<'_>, name: &str) -> Result<Option<String>> {
    for attribute in event.attributes() {
        let a = attribute.map_err(|e| err("INVALID_XLSX", e.to_string()))?;
        if a.key.as_ref() == name {
            return Ok(Some(
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|e| err("INVALID_XLSX", e.to_string()))?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}
fn coordinate(text: &str) -> Result<(usize, usize)> {
    let mut col = 0usize;
    let mut split = 0;
    for b in text.bytes() {
        if b.is_ascii_uppercase() {
            col = col
                .checked_mul(26)
                .and_then(|x| x.checked_add((b - b'A' + 1) as usize))
                .ok_or_else(|| err("RESOURCE_LIMIT", "worksheet coordinate overflow"))?;
            split += 1;
        } else {
            break;
        }
    }
    let row = text[split..]
        .parse::<usize>()
        .map_err(|_| err("INVALID_XLSX", "invalid cell coordinate"))?;
    if col == 0 || row == 0 {
        return Err(err("INVALID_XLSX", "invalid cell coordinate"));
    }
    Ok((row, col))
}
/// Workbook facts established by preflight that calamine does not expose.
struct XlsxPreflight {
    /// A workbook part asks consumers to recalculate on load, so stored formula
    /// results are placeholders whose presence cannot be trusted.
    full_calc_on_load: bool,
}
/// Decoded byte length of an XML entity or character reference.
fn xlsx_reference_bytes(reference: &quick_xml::events::BytesRef<'_>) -> Result<usize> {
    if reference.is_char_ref() {
        return reference
            .resolve_char_ref()
            .map_err(|e| err("INVALID_XLSX", e.to_string()))?
            .map(char::len_utf8)
            .ok_or_else(|| err("INVALID_XLSX", "invalid XML character reference"));
    }
    Ok(1)
}
/// Inspect archive actual decompression and sparse-cell extents before calamine allocates a range.
///
/// Every entry is fully decompressed and counted against the aggregate budget. Worksheets
/// are identified by their root element, not by a conventional part name, so relocating a
/// sheet (calamine follows package relationships) cannot bypass cell/range accounting.
/// Parts that are not named `.xml` are inspected only when their content is a worksheet.
fn xlsx_preflight(
    bytes: &[u8],
    limits: &Limits,
    expanded: &mut usize,
    cancel: &AtomicBool,
) -> Result<XlsxPreflight> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| err("INVALID_XLSX", e.to_string()))?;
    let mut full_calc_on_load = false;
    for i in 0..archive.len() {
        check_cancel(cancel)?;
        let mut entry = archive
            .by_index(i)
            .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
        limit(
            entry.size() <= limits.xlsx_bytes as u64,
            "XLSX advertised entry bytes",
        )?;
        let name = entry
            .name()
            .map_err(|e| err("INVALID_XLSX", e.to_string()))?
            .into_owned();
        let mut data = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            check_cancel(cancel)?;
            let n = entry
                .read(&mut chunk)
                .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
            if n == 0 {
                break;
            }
            *expanded = expanded.saturating_add(n);
            limit(
                *expanded <= limits.xlsx_bytes,
                "aggregate actual XLSX decompression bytes",
            )?;
            data.extend_from_slice(&chunk[..n]);
        }
        // Malformed `.xml` parts fail; other parts only matter if they hold a worksheet.
        let strict = name.to_ascii_lowercase().ends_with(".xml");
        let starts_xml = data
            .strip_prefix(b"\xEF\xBB\xBF")
            .unwrap_or(&data)
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            == Some(&b'<');
        if !strict && !starts_xml {
            continue;
        }
        let mut reader = quick_xml::Reader::from_reader(data.as_slice());
        let (mut worksheet, mut workbook, mut root_seen) = (false, false, false);
        let (mut max_row, mut max_col, mut cells) = (0usize, 0usize, 0usize);
        let mut cell_type = String::new();
        let (mut in_cell, mut formula, mut cache, mut cache_text, mut in_value) =
            (false, false, false, false, false);
        // Bytes of the current text-bearing field. Rich-text runs inside a shared or
        // inline string (`si`/`is`) form one field; other elements each start a new token.
        let (mut token, mut rich) = (0usize, false);
        loop {
            check_cancel(cancel)?;
            let event = match reader.read_event() {
                Ok(event) => event,
                Err(e) if strict || root_seen => return Err(err("INVALID_XLSX", e.to_string())),
                Err(_) => break,
            };
            use quick_xml::events::Event;
            let text_bytes = match &event {
                Event::Text(e) => Some(e.len()),
                Event::CData(e) => Some(e.len()),
                Event::GeneralRef(e) => Some(xlsx_reference_bytes(e)?),
                _ => None,
            };
            if let Some(n) = text_bytes {
                token = token.saturating_add(n);
                limit(token <= limits.field_bytes, "XLSX field/token bytes")?;
                if in_value && n > 0 {
                    cache_text = true;
                }
            }
            match event {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let empty = matches!(event, Event::Empty(_));
                    let local = e.local_name();
                    let local = local.as_ref();
                    if local == "si" || local == "is" {
                        rich = !empty;
                        token = 0;
                    } else if !rich {
                        token = 0;
                    }
                    if !root_seen {
                        root_seen = true;
                        worksheet = matches!(local, "worksheet" | "dialogsheet");
                        workbook = local == "workbook";
                        if !strict && !worksheet {
                            break;
                        }
                    }
                    match local {
                        "calcPr" if workbook => {
                            if matches!(
                                xml_attr(e, "fullCalcOnLoad")?.as_deref(),
                                Some("1" | "true")
                            ) {
                                full_calc_on_load = true;
                            }
                        }
                        "dimension" if worksheet => {
                            if let Some(reference) = xml_attr(e, "ref")? {
                                let end = reference.rsplit(':').next().unwrap_or(&reference);
                                let (r, c) = coordinate(end)?;
                                limit(
                                    r.checked_mul(c)
                                        .is_some_and(|n| n <= limits.worksheet_cells),
                                    "worksheet declared range cells",
                                )?;
                            }
                        }
                        "c" if worksheet => {
                            in_cell = !empty;
                            formula = false;
                            cache = false;
                            cache_text = false;
                            in_value = false;
                            cell_type = xml_attr(e, "t")?.unwrap_or_default();
                            cells += 1;
                            let reference = xml_attr(e, "r")?.ok_or_else(|| {
                                err(
                                    "INVALID_XLSX",
                                    "cell coordinates required for bounded range preflight",
                                )
                            })?;
                            let (r, c) = coordinate(&reference)?;
                            max_row = max_row.max(r);
                            max_col = max_col.max(c);
                            limit(
                                cells <= limits.worksheet_cells
                                    && max_row
                                        .checked_mul(max_col)
                                        .is_some_and(|n| n <= limits.worksheet_cells),
                                "worksheet cell/range limit",
                            )?;
                        }
                        "f" if in_cell => formula = true,
                        // An empty `<v/>` holds no stored value.
                        "v" if in_cell && !empty => {
                            cache = true;
                            in_value = true;
                        }
                        _ => {}
                    }
                }
                Event::End(ref e) => {
                    let local = e.local_name();
                    match local.as_ref() {
                        "si" | "is" => {
                            rich = false;
                            token = 0;
                        }
                        _ if !rich => token = 0,
                        _ => {}
                    }
                    match local.as_ref() {
                        "v" => in_value = false,
                        "c" if in_cell => {
                            if formula && (!cache || (!cache_text && cell_type != "str")) {
                                return Err(err(
                                    "FORMULA_CACHE_MISSING",
                                    "formula requires an unambiguous stored cached value; formulas are not evaluated",
                                ));
                            }
                            in_cell = false;
                        }
                        _ => {}
                    }
                }
                Event::DocType(_) => {
                    return Err(err("INVALID_XLSX", "XML document types are forbidden"));
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(XlsxPreflight { full_calc_on_load })
}
fn cell_label(mut col: u32, row: u32) -> String {
    let mut label = String::new();
    loop {
        label.insert(0, (b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    format!("{label}{}", row + 1)
}
fn excel_date(serial: f64, epoch1904: bool) -> Result<Value> {
    if !serial.is_finite()
        || serial.fract() != 0.0
        // Serial 0 in the 1900 system is the fictitious 1900-01-00.
        || serial < if epoch1904 { 0.0 } else { 1.0 }
        || serial > 3_000_000.0
        || (!epoch1904 && serial == 60.0)
    {
        return Err(err(
            "INVALID_DATE",
            "invalid or fractional Excel date serial",
        ));
    }
    let epoch = chrono::NaiveDate::from_ymd_opt(
        if epoch1904 { 1904 } else { 1899 },
        if epoch1904 { 1 } else { 12 },
        if epoch1904 { 1 } else { 31 },
    )
    .unwrap();
    let days = serial as i64 - if !epoch1904 && serial > 60.0 { 1 } else { 0 };
    let date = epoch
        .checked_add_signed(chrono::Duration::days(days))
        .ok_or_else(|| err("INVALID_DATE", "Excel date out of range"))?;
    let text = date.format("%Y-%m-%d").to_string();
    if text.len() != 10 {
        return Err(err(
            "INVALID_DATE",
            "Excel date outside ISO four-digit year range",
        ));
    }
    Ok(Value::Date { value: text })
}
/// Convert a finite stored double to its shortest round-trip decimal text, then apply
/// the declared checked quantization. This does not recover an originally intended decimal.
fn numeric_decimal(number: f64, field: &Field, mode: Rounding) -> Result<Value> {
    if !number.is_finite() {
        return Err(err("INVALID_DECIMAL", "nonfinite XLSX numeric cell"));
    }
    let text = expand_decimal(&number.to_string())?;
    let source_scale = text.split('.').nth(1).map(str::len).unwrap_or(0);
    let Type::Decimal { precision, scale } = field.data_type else {
        unreachable!()
    };
    let digits = text.replace('.', "");
    let mut coefficient = num_bigint::BigInt::parse_bytes(digits.as_bytes(), 10)
        .ok_or_else(|| err("INVALID_DECIMAL", "invalid observed number"))?;
    if source_scale > scale as usize {
        let divisor = num_bigint::BigInt::from(10).pow((source_scale - scale as usize) as u32);
        let remainder = &coefficient % &divisor;
        let twice = if remainder < num_bigint::BigInt::from(0) {
            -&remainder * 2
        } else {
            &remainder * 2
        };
        let negative = coefficient < num_bigint::BigInt::from(0);
        coefficient /= &divisor;
        if mode == Rounding::HalfEven
            && (twice > divisor
                || (twice == divisor && (&coefficient % 2) != num_bigint::BigInt::from(0)))
        {
            coefficient += if negative { -1 } else { 1 };
        }
    } else {
        coefficient *= num_bigint::BigInt::from(10).pow((scale as usize - source_scale) as u32);
    }
    let negative = coefficient < num_bigint::BigInt::from(0);
    let digits = coefficient.to_string().trim_start_matches('-').to_owned();
    let padded = format!("{:0>width$}", digits, width = scale as usize + 1);
    let split = padded.len() - scale as usize;
    let text = if scale == 0 {
        padded
    } else {
        format!("{}.{}", &padded[..split], &padded[split..])
    };
    let text = if negative { format!("-{text}") } else { text };
    let decimal = Decimal::parse(&text, precision, scale)
        .map_err(|e| err("INVALID_DECIMAL", e.to_string()))?;
    Ok(Value::Decimal {
        value: decimal.to_string(),
        precision,
        scale,
    })
}
/// Explicit XLSX binding policy, validated before the workbook is opened.
struct XlsxSettings {
    sheet: String,
    /// Zero-based worksheet row holding the declared column names.
    header_row: u32,
    /// Declared rounding for numeric cells of decimal fields, keyed by field name.
    quantization: BTreeMap<String, Rounding>,
    /// Date fields whose numeric cells are declared to be Excel date serials.
    excel_dates: BTreeSet<String>,
}
fn xlsx_settings(source: &Source, binding: &Binding) -> Result<XlsxSettings> {
    let invalid = |message: String| err("INVALID_BINDING", message);
    for key in binding.settings.keys() {
        if !["sheet", "header_row", "decimal_quantization", "excel_dates"].contains(&key.as_str()) {
            return Err(invalid(format!("unknown XLSX binding setting {key}")));
        }
    }
    let sheet = binding
        .settings
        .get("sheet")
        .and_then(|v| v.as_str())
        .ok_or_else(|| invalid("XLSX sheet must be explicitly named".into()))?
        .to_owned();
    let header_row = binding
        .settings
        .get("header_row")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| invalid("XLSX one-based header_row must be declared".into()))?;
    if header_row == 0 || header_row > u32::MAX as u64 {
        return Err(invalid("header_row out of range".into()));
    }
    let fields = &source.projection.fields;
    let mut quantization = BTreeMap::new();
    if let Some(value) = binding.settings.get("decimal_quantization") {
        let policies = value.as_object().ok_or_else(|| {
            invalid("decimal_quantization must map decimal field names to a rounding mode".into())
        })?;
        for (name, mode) in policies {
            if !matches!(
                fields.get(name).map(|f| &f.data_type),
                Some(Type::Decimal { .. })
            ) {
                return Err(invalid(format!(
                    "decimal_quantization names {name}, which is not a declared decimal field"
                )));
            }
            let mode = match mode.as_str() {
                Some("half_even") => Rounding::HalfEven,
                Some("toward_zero") => Rounding::TowardZero,
                _ => {
                    return Err(invalid(
                        "numeric XLSX decimal policy must be half_even or toward_zero".into(),
                    ));
                }
            };
            quantization.insert(name.clone(), mode);
        }
    }
    let mut excel_dates = BTreeSet::new();
    if let Some(value) = binding.settings.get("excel_dates") {
        let names = value
            .as_array()
            .ok_or_else(|| invalid("excel_dates must list declared date fields".into()))?;
        for name in names {
            let name = name
                .as_str()
                .filter(|n| matches!(fields.get(*n).map(|f| &f.data_type), Some(Type::Date)))
                .ok_or_else(|| invalid("excel_dates must list declared date fields".into()))?;
            excel_dates.insert(name.to_owned());
        }
    }
    Ok(XlsxSettings {
        sheet,
        header_row: header_row as u32 - 1,
        quantization,
        excel_dates,
    })
}
/// Stable cell-kind name and observed representation recorded as source evidence.
/// Numbers use Rust's shortest round-trip formatting of the stored binary value.
fn xlsx_observed(cell: &Data) -> (&'static str, String) {
    match cell {
        Data::Empty => ("blank", String::new()),
        Data::String(text) => ("string", text.clone()),
        Data::Float(number) => ("number", format!("{number}")),
        Data::Int(number) => ("number", number.to_string()),
        Data::Bool(value) => ("boolean", value.to_string()),
        Data::Error(error) => ("error", error.to_string()),
        Data::DateTime(serial) => ("date_serial", format!("{}", serial.as_f64())),
        Data::DateTimeIso(text) => ("iso_date", text.clone()),
        Data::DurationIso(text) => ("iso_duration", text.clone()),
    }
}
fn read_xlsx(
    source: &Source,
    binding: &Binding,
    bytes: &[u8],
    reference: &SourceReference,
    policy: &Policy,
    expanded: &mut usize,
    cancel: &AtomicBool,
    rows: &mut dyn RowSink,
) -> Result<()> {
    let settings = xlsx_settings(source, binding)?;
    let preflight = xlsx_preflight(bytes, &policy.limits, expanded, cancel)?;
    let sheet = settings.sheet.as_str();
    let header_row = settings.header_row;
    let mut book: Xlsx<_> = calamine::open_workbook_from_rs(Cursor::new(bytes))
        .map_err(|e: calamine::XlsxError| err("INVALID_XLSX", e.to_string()))?;
    if !book.sheet_names().iter().any(|name| name == sheet) {
        return Err(err(
            "INVALID_BINDING",
            format!("declared XLSX sheet {sheet} does not exist"),
        ));
    }
    // The date system of the workbook part calamine actually designates and reads.
    let epoch1904 = book.has_1904_epoch();
    check_cancel(cancel)?;
    let range = book
        .worksheet_range(sheet)
        .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
    check_cancel(cancel)?;
    let formulas = book
        .worksheet_formula(sheet)
        .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
    let Some((_, end_col)) = range.end() else {
        return Err(err("INVALID_PROJECTION", "worksheet has no header"));
    };
    let mut headers = BTreeMap::new();
    for col in 0..=end_col {
        if let Some(Data::String(name)) = range.get_value((header_row, col)) {
            if headers.insert(name.clone(), col).is_some() {
                return Err(err("INVALID_XLSX", "duplicate worksheet header"));
            }
        }
    }
    let columns: Vec<_> = source
        .projection
        .fields
        .iter()
        .map(|(name, f)| {
            let header = f.path.strip_prefix('/').unwrap_or(&f.path);
            headers
                .get(header)
                .map(|&col| (name, f, col))
                .ok_or_else(|| {
                    err(
                        "INVALID_PROJECTION",
                        format!("missing XLSX column {header}"),
                    )
                })
        })
        .collect::<Result<_>>()?;
    let names = projected_names(source);
    let mut failures = ValueFailures::default();
    let end_row = range.end().unwrap().0;
    let date_system = if epoch1904 { "1904" } else { "1900" };
    for row_index in header_row + 1..=end_row {
        check_cancel(cancel)?;
        let mut row = blank_row(&names, &reference);
        row.reference.location = format!("{sheet}!{}", row_index + 1);
        let filled = (|| -> Result<()> {
            for (name, field, col) in &columns {
                let cell = range.get_value((row_index, *col)).unwrap_or(&Data::Empty);
                let formula = formulas
                    .get_value((row_index, *col))
                    .filter(|s| !s.is_empty());
                if formula.is_some() {
                    if matches!(cell, Data::Empty) {
                        return Err(err(
                            "FORMULA_CACHE_MISSING",
                            "formula requires a stored cached value; formulas are not evaluated",
                        ));
                    }
                    if preflight.full_calc_on_load {
                        return Err(err(
                            "FORMULA_CACHE_UNCERTAIN",
                            "workbook requests recalculation on load, so stored formula results are not established",
                        ));
                    }
                }
                let rounding = settings.quantization.get(name.as_str()).copied();
                let quantization = || {
                    rounding.ok_or_else(|| {
                        err(
                            "XLSX_QUANTIZATION_REQUIRED",
                            "numeric decimal cell requires explicit quantization policy",
                        )
                    })
                };
                let declared_date = || {
                    if settings.excel_dates.contains(name.as_str()) {
                        Ok(())
                    } else {
                        Err(err(
                            "INVALID_BINDING",
                            "numeric date requires excel_dates field declaration",
                        ))
                    }
                };
                let mut conversion = None;
                let value = match cell {
                    Data::Empty => value_from_text(None, field)?,
                    Data::Error(_) => return Err(err("XLSX_CELL_ERROR", "spreadsheet error cell")),
                    Data::String(text) => {
                        limit(text.len() <= policy.limits.field_bytes, "XLSX field bytes")?;
                        value_from_text(Some(text), field)?
                    }
                    Data::Bool(value) if field.data_type == Type::Boolean => {
                        Value::Boolean { value: *value }
                    }
                    Data::Int(value) if field.data_type == Type::Int64 => Value::Int64 {
                        value: value.to_string(),
                    },
                    Data::Float(value) if field.data_type == Type::Int64 => {
                        // Beyond 2^53 a stored double no longer identifies one integer.
                        if !value.is_finite()
                            || value.fract() != 0.0
                            || value.abs() >= 9007199254740992.0
                        {
                            return Err(err(
                                "INVALID_VALUE",
                                "XLSX integer is fractional or not exactly representable",
                            ));
                        }
                        Value::Int64 {
                            value: (*value as i64).to_string(),
                        }
                    }
                    Data::Int(value) if matches!(field.data_type, Type::Decimal { .. }) => {
                        quantization()?;
                        value_from_text(Some(&value.to_string()), field)?
                    }
                    Data::Float(value) if matches!(field.data_type, Type::Decimal { .. }) => {
                        let mode = quantization()?;
                        conversion = Some("shortest_roundtrip_decimal");
                        numeric_decimal(*value, field, mode)?
                    }
                    Data::DateTime(value) if field.data_type == Type::Date => {
                        declared_date()?;
                        if value.is_duration() {
                            return Err(err(
                                "INVALID_VALUE",
                                "XLSX duration cell is not a calendar date",
                            ));
                        }
                        excel_date(value.as_f64(), epoch1904)?
                    }
                    Data::DateTimeIso(value) if field.data_type == Type::Date => {
                        value_from_text(Some(value), field)?
                    }
                    Data::Int(value) if field.data_type == Type::Date => {
                        declared_date()?;
                        excel_date(*value as f64, epoch1904)?
                    }
                    Data::Float(value) if field.data_type == Type::Date => {
                        declared_date()?;
                        excel_date(*value, epoch1904)?
                    }
                    _ => {
                        return Err(err(
                            "INVALID_VALUE",
                            "XLSX cell type differs from declared field",
                        ));
                    }
                };
                let (cell_type, observed) = xlsx_observed(cell);
                row.raw.insert(
                    (*name).clone(),
                    serde_json::json!({
                        "cell": format!("{sheet}!{}", cell_label(*col, row_index)),
                        "cell_type": cell_type,
                        "observed": observed,
                        "formula": formula,
                        "quantization": rounding,
                        "conversion": conversion,
                        "date_system": date_system,
                    }),
                );
                row.fields.insert((*name).clone(), value);
            }
            Ok(())
        })();
        match filled {
            Ok(()) => rows.push(row)?,
            Err(error) => failures.record(error, &row.reference)?,
        }
    }
    failures.finish()?;
    Ok(())
}

/// One owned typed batch and its source observations. Exact decimals and dates use
/// Arrow Decimal128 and Date32; evidence remains separate from calculation arrays.
pub struct AdapterBatch {
    pub batch: datafusion::arrow::record_batch::RecordBatch,
    pub observations: Vec<Row>,
}

/// Pull delivery over an admitted source materialization. Only the requested batch
/// is converted to Arrow. Input parsing currently materializes the bounded source.
pub struct SourceBatches<'a> {
    cancel: &'a AtomicBool,
    rows: std::vec::IntoIter<Row>,
    fields: Vec<(String, Field)>,
    batch_rows: usize,
}
impl Iterator for SourceBatches<'_> {
    type Item = Result<AdapterBatch>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Err(error) = check_cancel(self.cancel) {
            if self.rows.len() == 0 {
                return None;
            }
            self.rows = Vec::new().into_iter();
            return Some(Err(error));
        }
        let observations: Vec<_> = self.rows.by_ref().take(self.batch_rows).collect();
        if observations.is_empty() {
            return None;
        }
        Some(typed_batch(&self.fields, observations))
    }
}

fn typed_batch(fields: &[(String, Field)], observations: Vec<Row>) -> Result<AdapterBatch> {
    use datafusion::arrow::{
        array::{ArrayRef, BooleanArray, Date32Array, Decimal128Array, Int64Array, StringArray},
        datatypes::{DataType, Field as ArrowField, Schema},
        record_batch::RecordBatch,
    };
    use std::sync::Arc;
    let mut arrays: Vec<ArrayRef> = Vec::new();
    let mut schema = Vec::new();
    for (name, field) in fields {
        let values: Vec<_> = observations
            .iter()
            .map(|r| r.fields.get(name).unwrap_or(&Value::Null))
            .collect();
        let (data_type, array): (DataType, ArrayRef) = match field.data_type {
            Type::Boolean => (
                DataType::Boolean,
                Arc::new(BooleanArray::from(
                    values
                        .iter()
                        .map(|v| match v {
                            Value::Boolean { value } => Some(*value),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )),
            ),
            Type::Int64 => {
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Int64 { value } => value
                            .parse::<i64>()
                            .map(Some)
                            .map_err(|_| err("INVALID_VALUE", "integer transport")),
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                (DataType::Int64, Arc::new(Int64Array::from(values)))
            }
            Type::String => (
                DataType::Utf8,
                Arc::new(StringArray::from(
                    values
                        .iter()
                        .map(|v| match v {
                            Value::String { value } => Some(value.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )),
            ),
            Type::Decimal { precision, scale } => {
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Decimal { value, .. } => Decimal::parse(value, precision, scale)
                            .map(|d| Some(d.coefficient))
                            .map_err(|e| err("INVALID_DECIMAL", e.to_string())),
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                let array = Decimal128Array::from(values)
                    .with_precision_and_scale(precision, scale as i8)
                    .map_err(|e| err("INVALID_SCHEMA", e.to_string()))?;
                (
                    DataType::Decimal128(precision, scale as i8),
                    Arc::new(array),
                )
            }
            Type::Date => {
                let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Date { value } => {
                            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                                .map(|d| Some(d.signed_duration_since(epoch).num_days() as i32))
                                .map_err(|_| err("INVALID_DATE", "date transport"))
                        }
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                (DataType::Date32, Arc::new(Date32Array::from(values)))
            }
        };
        schema.push(ArrowField::new(name, data_type, field.nullable));
        arrays.push(array);
    }
    let batch = RecordBatch::try_new(Arc::new(Schema::new(schema)), arrays)
        .map_err(|e| err("INVALID_SCHEMA", e.to_string()))?;
    Ok(AdapterBatch {
        batch,
        observations,
    })
}

/// Open all requested local sources as owned, typed pull-delivery iterators.
/// Admission/parsing is eager; this is not a streaming parser or whole-process RSS bound.
pub fn read_batches<'a>(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &'a AtomicBool,
) -> Result<BTreeMap<String, SourceBatches<'a>>> {
    let (rows, _) = materialize_sources(
        recipe,
        bindings,
        policy,
        cancel,
        None,
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
    )?;
    into_batches(recipe, policy, cancel, rows)
}

fn into_batches<'a>(
    recipe: &Recipe,
    policy: &Policy,
    cancel: &'a AtomicBool,
    rows: BTreeMap<String, Vec<Row>>,
) -> Result<BTreeMap<String, SourceBatches<'a>>> {
    rows.into_iter()
        .map(|(id, rows)| {
            let source = recipe
                .sources
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| err("INVALID_SOURCE", "missing source declaration"))?;
            Ok((
                id,
                SourceBatches {
                    cancel,
                    rows: rows.into_iter(),
                    fields: source
                        .projection
                        .fields
                        .iter()
                        .map(|(n, f)| (n.clone(), f.clone()))
                        .collect(),
                    batch_rows: policy.limits.batch_rows.max(1),
                },
            ))
        })
        .collect()
}

/// Compatibility materializer for the local executor; each source crosses the
/// same typed-batch boundary before its observations are retained for evidence.
pub fn read_sources(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Vec<Row>>> {
    read_sources_observed(recipe, bindings, policy, cancel).map(|(rows, _)| rows)
}

/// [`read_sources`] plus what was read for each source, for the run manifest.
pub fn read_sources_observed(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<(BTreeMap<String, Vec<Row>>, BTreeMap<String, SourceRead>)> {
    read_for_run(
        recipe,
        bindings,
        policy,
        cancel,
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
    .map(|(rows, reads, _)| (rows, reads))
}

/// The executor's read. Sources in `streamed` are checked in full but only
/// their key records are returned; [`Streams`] reads their records again
/// (docs/spec/slice-7.md, phase 3). A source in `spillable` that exceeds
/// `limits.target_bytes` is returned the same way, and [`Streams::fetch`]
/// reads the records a batch needs (phases 4 and 5).
pub(crate) fn read_for_run(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
    streamed: &BTreeSet<String>,
    spillable: &BTreeSet<String>,
    periods_only: &BTreeSet<String>,
) -> Result<(
    BTreeMap<String, Vec<Row>>,
    BTreeMap<String, SourceRead>,
    Streams,
)> {
    let mut reads = BTreeMap::new();
    let (mut rows, streams) = materialize_sources(
        recipe,
        bindings,
        policy,
        cancel,
        Some(&mut reads),
        streamed,
        spillable,
        periods_only,
    )?;
    // Key records already crossed the typed-batch boundary as whole records.
    let keyed: Vec<String> = streamed
        .iter()
        .chain(periods_only)
        .chain(streams.targets.keys())
        .cloned()
        .collect();
    let mut result: BTreeMap<String, Vec<Row>> = keyed
        .iter()
        .filter_map(|id| rows.remove_entry(id))
        .collect();
    let sources = into_batches(recipe, policy, cancel, rows)?;
    for (id, batches) in sources {
        let converted = std::time::Instant::now();
        let mut rows = Vec::new();
        for batch in batches {
            check_cancel(cancel)?;
            rows.extend(batch?.observations);
        }
        if let Some(read) = reads.get_mut(&id) {
            read.conversion_us = micros(converted);
        }
        result.insert(id, rows);
    }
    Ok((result, reads, streams))
}

/// Bound validation's schema inspection (`data_checks = "none"`): open every bound
/// source under the read policy and check its structure against the declared
/// projection without decoding records. CSV: the header record. XLSX: package
/// safety preflight, the named sheet and its declared header row, read cell by cell
/// up to that row. JSON: the declared array pointer and required header pointers.
/// Record values, identities, relationships and intervals are data checks and are
/// left to `data_checks = "full"` and to every actual run.
pub fn inspect_schemas(recipe: &Recipe, bindings: &Bindings, policy: &Policy) -> Result<usize> {
    let cancel = AtomicBool::new(false);
    let mut total = 0;
    let mut expanded = 0;
    let plan = crate::placement::plan(recipe, bindings, policy)?;
    let snapshots = sqlite_snapshots(recipe, bindings, policy, &mut total, &cancel)?;
    for source in &recipe.sources {
        let binding = bindings
            .get(&source.id)
            .ok_or_else(|| err("MISSING_BINDING", format!("missing binding {}", source.id)))?;
        if binding.adapter == "sqlite" {
            // A transfer target compares text at the source, like any pushed scan.
            let transfer_stage = plan
                .transfers
                .get(&source.id)
                .map(|t| t.pushed(Default::default()));
            sqlite::inspect_connection(
                source,
                binding,
                &snapshots[binding.path.as_os_str().as_encoded_bytes()].conn,
                if policy.placement == Placement::RequireSource {
                    plan.reductions
                        .keys()
                        .filter(|id| {
                            bindings
                                .get(*id)
                                .is_some_and(|b| crate::placement::colocated(binding, b))
                        })
                        .find_map(|id| plan.pushed.get(id))
                        .or_else(|| plan.pushed.get(&source.id))
                        .or(transfer_stage.as_ref())
                } else {
                    None
                },
            )
            .map_err(|e| tag_source(e, &source.id).with_check("source_schema"))?;
            continue;
        }
        inspect_schema(
            source,
            binding,
            policy,
            &mut total,
            &mut expanded,
            &cancel,
            plan.pushed.get(&source.id),
        )
        .map_err(|e| tag_source(e, &source.id).with_check("source_schema"))?;
    }
    Ok(recipe.sources.len())
}

fn inspect_schema(
    source: &Source,
    binding: &Binding,
    policy: &Policy,
    total: &mut usize,
    expanded: &mut usize,
    cancel: &AtomicBool,
    pushed: Option<&crate::placement::Pushed>,
) -> Result<()> {
    check_text_settings(binding)?;
    let missing = |what: &str, name: &str| {
        err(
            "INVALID_PROJECTION",
            format!("declared {what} {name} is not present in the source"),
        )
    };
    match binding.adapter.as_str() {
        "sqlite" => {
            let bytes = read_bytes(binding, policy, total, cancel)?;
            sqlite::inspect(
                source,
                binding,
                &bytes,
                if policy.placement == Placement::RequireSource {
                    pushed
                } else {
                    None
                },
            )?;
        }
        "csv" => {
            let dialect = CsvDialect::from_binding(binding)?;
            let bytes = read_bytes(binding, policy, total, cancel)?;
            let mut reader = CsvRecords::new(&bytes, &dialect, policy.limits.field_bytes, cancel);
            let headers = reader
                .next_record()?
                .ok_or_else(|| err("INVALID_CSV", "CSV header record is required"))?;
            let mut seen = BTreeSet::new();
            if headers.iter().any(|h| !seen.insert(h)) {
                return Err(err("INVALID_CSV", "duplicate header"));
            }
            for field in source.projection.fields.values() {
                let path = field.path.strip_prefix('/').unwrap_or(&field.path);
                if !headers.iter().any(|h| h == path) {
                    return Err(missing("CSV column", path));
                }
            }
        }
        "xlsx" => {
            let settings = xlsx_settings(source, binding)?;
            let bytes = read_bytes(binding, policy, total, cancel)?;
            xlsx_preflight(&bytes, &policy.limits, expanded, cancel)?;
            let mut book: Xlsx<_> = calamine::open_workbook_from_rs(Cursor::new(&bytes[..]))
                .map_err(|e: calamine::XlsxError| err("INVALID_XLSX", e.to_string()))?;
            if !book.sheet_names().iter().any(|n| *n == settings.sheet) {
                return Err(err(
                    "INVALID_BINDING",
                    format!("declared XLSX sheet {} does not exist", settings.sheet),
                ));
            }
            let mut cells = book
                .worksheet_cells_reader(&settings.sheet)
                .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
            let mut headers = BTreeSet::new();
            let mut decoded = 0usize;
            while let Some(cell) = cells
                .next_cell()
                .map_err(|e| err("INVALID_XLSX", e.to_string()))?
            {
                decoded += 1;
                limit(
                    decoded <= policy.limits.worksheet_cells,
                    "decoded worksheet cells",
                )?;
                let (row, _) = cell.get_position();
                if row > settings.header_row {
                    break;
                }
                if row == settings.header_row {
                    match cell.get_value() {
                        calamine::DataRef::String(name) => {
                            if !headers.insert(name.clone()) {
                                return Err(err("INVALID_XLSX", "duplicate worksheet header"));
                            }
                        }
                        calamine::DataRef::SharedString(name) => {
                            if !headers.insert((*name).to_owned()) {
                                return Err(err("INVALID_XLSX", "duplicate worksheet header"));
                            }
                        }
                        _ => {}
                    }
                }
            }
            for field in source.projection.fields.values() {
                let header = field.path.strip_prefix('/').unwrap_or(&field.path);
                if !headers.contains(header) {
                    return Err(missing("XLSX column", header));
                }
            }
        }
        "json" => {
            for path in source
                .projection
                .array
                .iter()
                .chain(source.projection.fields.values().map(|f| &f.path))
            {
                json_pointer_tokens(path)?;
            }
            let bytes = read_bytes(binding, policy, total, cancel)?;
            json_preflight(&bytes, &policy.limits, cancel)?;
            let root: &serde_json::value::RawValue =
                serde_json::from_slice(&bytes).map_err(|e| err("INVALID_JSON", e.to_string()))?;
            if let Some(array) = &source.projection.array {
                if !raw_json_lookup(root, array)?
                    .is_some_and(|v| v.get().trim_start().starts_with('['))
                {
                    return Err(missing("JSON array", array));
                }
            }
            // Header fields (and every field of a whole-document projection) live at
            // document level; per-record fields are data and are not scanned here.
            for field in source.projection.fields.values() {
                if (field.header || source.projection.array.is_none())
                    && !field.nullable
                    && raw_json_lookup(root, &field.path)?.is_none_or(|v| v.get().trim() == "null")
                {
                    return Err(missing("JSON field", &field.path));
                }
            }
        }
        other => {
            return Err(err(
                "UNSUPPORTED_ADAPTER",
                format!("unsupported source adapter {other}"),
            ));
        }
    }
    Ok(())
}

/// A spilled record read again must be the record the first read stored
/// (docs/spec/slice-7.md, phase 4 clause 5). A source changed on disk is never
/// seen, so these tests damage the run's own store to reach the guard.
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileExt;

    const ORDERS: &str = "order_no,cust\nO1,C1\nO2,C2\nO3,C3\n";
    const CUSTOMERS: &str = "CustNo,SalesArea\nC1,North\nC2,South\nC3,East\n";

    /// Orders reach customers through a to-one relationship; customers spill.
    fn spilled_customers(
        adapter: &str,
        placement: Placement,
    ) -> (tempfile::TempDir, Vec<Row>, Streams) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("orders.csv"), ORDERS).unwrap();
        let customers = if adapter == "sqlite" {
            let path = root.join("crm.db");
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute_batch(
                    "CREATE TABLE customers (CustNo TEXT, SalesArea TEXT);
                     INSERT INTO customers VALUES ('C1','North'),('C2','South'),('C3','East');",
                )
                .unwrap();
            path
        } else {
            let path = root.join("customers.csv");
            std::fs::write(&path, CUSTOMERS).unwrap();
            path
        };
        let column = |p: &str| serde_json::json!({"path":p,"data_type":{"kind":"string"}});
        let recipe: Recipe = serde_json::from_value(serde_json::json!({
            "format_version":"0.1", "name":"orders",
            "sources":[
                {"id":"orders","identity":["order_no"],"grain":["order_no"],"projection":{"fields":{
                    "order_no":column("order_no"),"cust":column("cust")}}},
                {"id":"customers","identity":["CustNo"],"grain":["CustNo"],"projection":{"fields":{
                    "CustNo":column("CustNo"),"SalesArea":column("SalesArea")}}}],
            "relationships":[{"id":"customer","from":"orders","to":"customers",
                "from_keys":["cust"],"to_keys":["CustNo"],"cardinality":"many_to_one"}],
            "outputs":[{"id":"orders_out","anchor":"orders","columns":[
                {"name":"area","expr":{"op":"field","source":"customers","field":"SalesArea","path":["customer"]}}
            ]}]
        }))
        .unwrap();
        let mut bindings = Bindings::new();
        bindings.insert(
            "orders".into(),
            Binding {
                path: root.join("orders.csv"),
                adapter: "csv".into(),
                settings: BTreeMap::new(),
            },
        );
        bindings.insert(
            "customers".into(),
            Binding {
                path: customers,
                adapter: adapter.into(),
                settings: if adapter == "sqlite" {
                    BTreeMap::from([("table".into(), serde_json::json!("customers"))])
                } else {
                    BTreeMap::new()
                },
            },
        );
        let mut policy = Policy {
            allowed_read_roots: vec![root.clone()],
            allowed_output_roots: vec![root],
            limits: Limits::default(),
            diagnostic_values: false,
            placement,
            transfers: Transfers::None,
            evidence: Evidence::Full,
        };
        policy.limits.target_bytes = 0;
        let (mut rows, _, streams) = read_for_run(
            &recipe,
            &bindings,
            &policy,
            &AtomicBool::new(false),
            &BTreeSet::new(),
            &BTreeSet::from(["customers".to_string()]),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(streams.spilled("customers"));
        let keys = rows.remove("customers").unwrap();
        assert_eq!(keys.len(), 3);
        (dir, keys, streams)
    }

    fn file_store(streams: &mut Streams) -> (&File, &mut Vec<u64>) {
        match &mut streams.targets.get_mut("customers").unwrap().store {
            Store::File { file, offsets } => (file, offsets),
            Store::Snapshot => panic!("customers was not spilled to a file"),
        }
    }

    fn assert_changed(result: Result<Vec<Row>>) {
        match result {
            Err(error) => {
                assert_eq!(error.code, "SOURCE_CHANGED", "{}", error.message);
                assert_eq!(error.subject.as_deref(), Some("source:customers"));
            }
            Ok(rows) => panic!("a changed record was read again: {rows:?}"),
        }
    }

    #[test]
    fn an_undamaged_spilled_record_reads_again() {
        let (_dir, keys, streams) = spilled_customers("csv", Placement::Local);
        let rows = streams
            .fetch("customers", &keys, &[0, 2], &AtomicBool::new(false))
            .unwrap();
        let locations: Vec<_> = rows.iter().map(|r| r.reference.location.clone()).collect();
        assert_eq!(
            locations,
            [&keys[0], &keys[2]].map(|k| k.reference.location.clone())
        );
    }

    #[test]
    fn a_spilled_record_that_no_longer_decodes_is_a_changed_source() {
        let (_dir, keys, mut streams) = spilled_customers("csv", Placement::Local);
        let (file, offsets) = file_store(&mut streams);
        let start = offsets[1];
        file.write_all_at(b"\xff\xff\xff", start).unwrap();
        assert_changed(streams.fetch("customers", &keys, &[1], &AtomicBool::new(false)));
    }

    #[test]
    fn a_spilled_record_at_another_position_is_a_changed_source() {
        let (_dir, keys, mut streams) = spilled_customers("csv", Placement::Local);
        let (_, offsets) = file_store(&mut streams);
        // Record 0's span now holds record 1, which decodes but is elsewhere.
        offsets.remove(0);
        assert_changed(streams.fetch("customers", &keys, &[0], &AtomicBool::new(false)));
    }

    #[test]
    fn a_snapshot_record_no_longer_found_is_a_changed_source() {
        let (dir, keys, mut streams) = spilled_customers("sqlite", Placement::PreferSource);
        assert!(matches!(
            streams.targets["customers"].store,
            Store::Snapshot
        ));
        let smaller = dir.path().join("smaller.db");
        rusqlite::Connection::open(&smaller)
            .unwrap()
            .execute_batch(
                "CREATE TABLE customers (CustNo TEXT, SalesArea TEXT);
                 INSERT INTO customers VALUES ('C1','North');",
            )
            .unwrap();
        let bytes = std::fs::read(&smaller).unwrap();
        for snapshot in streams.snapshots.values_mut() {
            snapshot.conn = sqlite::open(&bytes).unwrap();
        }
        assert_changed(streams.fetch("customers", &keys, &[1, 2], &AtomicBool::new(false)));
    }
}
