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
) -> Result<BTreeMap<String, Vec<Row>>> {
    let mut result = BTreeMap::new();
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
        check_cancel(cancel)?;
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
        let owned_bytes;
        let bytes = if let Some(snapshot) = snapshot {
            &snapshot.bytes
        } else {
            owned_bytes = read_bytes(binding, policy, &mut total, cancel).map_err(subject)?;
            &owned_bytes
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
            source: source.id.clone(),
            binding: binding_identity(binding)?,
            fingerprint,
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
        let mut rows = match binding.adapter.as_str() {
            "sqlite" => sqlite::read(
                source,
                binding,
                &snapshot.unwrap().conn,
                &reference,
                &admission,
                cancel,
                transfer_pushed.as_ref().or(plan.pushed.get(&source.id)),
                transfer_fallback.as_ref().or(fallbacks.get(&source.id)),
            )
            .map(|mut read| {
                if let (Some(placement), Some(transferred)) = (&mut read.placement, transferred) {
                    if placement["scan"] == "source" {
                        placement["reduction"] = serde_json::json!("transferred_keys");
                        placement["transfer"] = transferred;
                    }
                }
                scan = Some((read.total_rows, read.placement));
                read.rows
            }),
            "json" => read_json(source, &bytes, &reference, &admission, cancel),
            "csv" => read_csv(source, binding, &bytes, &reference, &admission, cancel),
            "xlsx" => read_xlsx(
                source,
                binding,
                &bytes,
                &reference,
                &admission,
                &mut expanded,
                cancel,
            ),
            _ => return Err(err("UNSUPPORTED_CAPABILITY", "unsupported source adapter")),
        }
        .map_err(subject)?;
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
            check_cancel(cancel)?;
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
            let size = serde_json::to_vec(row).map_err(io_error)?.len();
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
            retained = retained.saturating_add(size);
            limit(
                retained <= policy.limits.retained_bytes,
                "retained source materialization bytes",
            )?;
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
        if let Some(reads) = reads.as_deref_mut() {
            reads.insert(
                source.id.clone(),
                SourceRead {
                    adapter: binding.adapter.clone(),
                    capabilities: adapter_capabilities(&binding.adapter),
                    binding: reference.binding.clone(),
                    fingerprint: reference.fingerprint.clone(),
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
                    parse_us: micros(parsed),
                    conversion_us: 0,
                },
            );
        }
        result.insert(source.id.clone(), rows);
    }
    Ok(result)
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
    fn push(&mut self, row: Row) -> Result<()> {
        limit(
            self.rows.len() < self.policy.limits.lookup_rows,
            "source row count",
        )?;
        let size = serde_json::to_vec(&row).map_err(io_error)?.len();
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
) -> Result<Vec<Row>> {
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
    let mut rows = RowCollector::new(policy);
    let mut failures = ValueFailures::default();
    for (index, record) in records.into_iter().enumerate() {
        check_cancel(cancel)?;
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
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
    Ok(rows.rows)
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
) -> Result<Vec<Row>> {
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
    let mut rows = RowCollector::new(policy);
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
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
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
    Ok(rows.rows)
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
) -> Result<Vec<Row>> {
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
    let mut rows = RowCollector::new(policy);
    let mut failures = ValueFailures::default();
    let end_row = range.end().unwrap().0;
    let date_system = if epoch1904 { "1904" } else { "1900" };
    for row_index in header_row + 1..=end_row {
        check_cancel(cancel)?;
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
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
    Ok(rows.rows)
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
    let rows = materialize_sources(recipe, bindings, policy, cancel, None)?;
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
    let mut reads = BTreeMap::new();
    let rows = materialize_sources(recipe, bindings, policy, cancel, Some(&mut reads))?;
    let sources = into_batches(recipe, policy, cancel, rows)?;
    let mut result = BTreeMap::new();
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
    Ok((result, reads))
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
