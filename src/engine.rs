use crate::{
    admission::{PARTITIONS, RunControl, WORKER_THREAD, busy, engine_batch_rows, run_id},
    cancel::{Cancellation, RunPhase},
    model::*,
};
use datafusion::{
    arrow::{
        array::{StringArray, UInt64Array},
        datatypes::{DataType, Field as ArrowField, Schema},
        record_batch::RecordBatch,
    },
    common::JoinType,
    execution::{
        context::{SessionConfig, SessionContext},
        disk_manager::{DiskManagerBuilder, DiskManagerMode},
        runtime_env::RuntimeEnvBuilder,
    },
    prelude::col,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub type Table = Vec<BTreeMap<String, Value>>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub status: String,
    pub tables: BTreeMap<String, Table>,
    /// Primary evidence: one record per output line.
    pub evidence: Vec<serde_json::Value>,
    /// One record per qualified to-one lookup candidate of an output line.
    #[serde(default)]
    pub candidate_evidence: Vec<serde_json::Value>,
    /// One record per observed detail record linked to an output line.
    #[serde(default)]
    pub detail_evidence: Vec<serde_json::Value>,
    pub diagnostics: Vec<Error>,
    pub manifest: serde_json::Value,
}
#[derive(Default)]
pub struct ExecutionContext {
    busy: AtomicBool,
}
pub(crate) struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl ExecutionContext {
    /// Run `recipe` to memory results. `cancel` is any [`Cancellation`] source:
    /// `&AtomicBool`, `&Arc<AtomicBool>` or a [`crate::CancellationToken`].
    pub fn run<C: Cancellation + ?Sized>(
        &self,
        recipe: &Recipe,
        bindings: &Bindings,
        policy: &Policy,
        cancel: &C,
    ) -> RunReport {
        let Some(_busy) = self.admit() else {
            return busy(policy);
        };
        let control = RunControl::new(&cancel, policy);
        let mut report = self.run_admitted(recipe, bindings, policy, &control);
        control.record(&mut report, recipe, policy);
        report
    }
    /// Admission: one active run per execution context.
    fn admit(&self) -> Option<Busy<'_>> {
        if self.busy.swap(true, Ordering::AcqRel) {
            None
        } else {
            Some(Busy(&self.busy))
        }
    }
    fn run_admitted(
        &self,
        recipe: &Recipe,
        bindings: &Bindings,
        policy: &Policy,
        cancel: &RunControl<'_>,
    ) -> RunReport {
        let id = run_id();
        let started = Instant::now();
        let mut progress = Progress::default();
        let report = crate::validation::validate(recipe);
        if !report.valid {
            // Fail with the failing check's own diagnostic (for example
            // UNSUPPORTED_OPERATION), keeping the full static report as context.
            let mut error = report
                .checks
                .iter()
                .find_map(|c| c.diagnostic.clone())
                .unwrap_or_else(|| {
                    Error::new("INVALID_RECIPE", "validation", "static validation failed")
                });
            error.context = Some(serde_json::to_string(&report).unwrap_or_default());
            return finish_failed(id, error, started, recipe, policy, &progress);
        }
        progress.pass(
            "static",
            "version, references, types, named paths, grain and capabilities",
        );
        match execute(recipe, bindings, policy, cancel, &mut progress) {
            Ok(run) => {
                let manifest = success_manifest(&id, recipe, policy, &progress, &run, started);
                RunReport {
                    run_id: id,
                    status: if run.unresolved > 0 {
                        "succeeded_with_unresolved"
                    } else {
                        "succeeded"
                    }
                    .into(),
                    tables: run.tables,
                    evidence: run.evidence,
                    candidate_evidence: run.candidate_evidence,
                    detail_evidence: run.detail_evidence,
                    diagnostics: finalize(run.diagnostics, recipe),
                    manifest,
                }
            }
            Err(error) => finish_failed(id, error, started, recipe, policy, &progress),
        }
    }
}
pub(crate) fn failed(id: String, error: Error, started: Instant) -> RunReport {
    let status = if error.code == "CANCELLED" {
        "cancelled"
    } else {
        "failed"
    };
    RunReport {
        run_id: id.clone(),
        status: status.into(),
        tables: BTreeMap::new(),
        evidence: vec![],
        candidate_evidence: vec![],
        detail_evidence: vec![],
        diagnostics: vec![error],
        manifest: serde_json::json!({"run_id":id,"complete":false,"checks":{"data":"incomplete"},"elapsed_ms":started.elapsed().as_millis()}),
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::new("CANCELLED", "execution", "run cancelled"))
    } else {
        Ok(())
    }
}
fn engine_error(e: impl std::fmt::Display + 'static) -> Error {
    // A stateful operator that cannot reserve within the tracked pool fails; it never spills.
    let exhausted = (&e as &dyn std::any::Any)
        .downcast_ref::<datafusion::error::DataFusionError>()
        .is_some_and(|e| {
            matches!(
                e.find_root(),
                datafusion::error::DataFusionError::ResourcesExhausted(_)
            )
        });
    let code = if exhausted {
        "RESOURCE_LIMIT"
    } else {
        "EXECUTION_ERROR"
    };
    Error::new(code, "execution", e.to_string())
}
fn key(row: &Row, keys: &[String]) -> Option<String> {
    let values: Vec<_> = keys
        .iter()
        .map(|k| row.fields.get(k).unwrap_or(&Value::Null))
        .collect();
    if values.iter().any(|v| matches!(v, Value::Null)) {
        None
    } else {
        Some(serde_json::to_string(&values).unwrap())
    }
}
fn batch(
    prefix: &str,
    rows: &[Row],
    keys: &[String],
    dates: [Option<&str>; 3],
) -> Result<RecordBatch> {
    let name = |n: &str| format!("{prefix}{n}");
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new(name("key"), DataType::Utf8, true),
        ArrowField::new(name("idx"), DataType::UInt64, false),
        ArrowField::new(name("date"), DataType::Utf8, true),
        ArrowField::new(name("start"), DataType::Utf8, true),
        ArrowField::new(name("end"), DataType::Utf8, true),
    ]));
    let mut arrays: Vec<Arc<dyn datafusion::arrow::array::Array>> = vec![
        Arc::new(StringArray::from(
            rows.iter().map(|r| key(r, keys)).collect::<Vec<_>>(),
        )),
        Arc::new(UInt64Array::from_iter_values(0..rows.len() as u64)),
    ];
    for field in dates {
        arrays.push(Arc::new(StringArray::from(
            rows.iter()
                .map(|r| {
                    field.and_then(|f| match r.fields.get(f) {
                        Some(Value::Date { value }) => Some(value.clone()),
                        _ => None,
                    })
                })
                .collect::<Vec<_>>(),
        )));
    }
    RecordBatch::try_new(schema, arrays).map_err(engine_error)
}

type Matches = BTreeMap<String, Vec<Vec<usize>>>;
fn qualified(rel: &Relationship, left: &Row, right: &Row) -> Result<bool> {
    if let Some(v) = &rel.validity {
        let date = |r: &Row, k: &str| match r.fields.get(k) {
            Some(Value::Date { value }) => Ok(value.clone()),
            _ => Err(Error::new(
                "INVALID_DATE",
                "data",
                format!("required date {k}"),
            )),
        };
        let d = date(left, &v.date)?;
        let start = date(right, &v.valid_from)?;
        let end = match right.fields.get(&v.valid_to) {
            Some(Value::Null) => None,
            _ => Some(date(right, &v.valid_to)?),
        };
        if end.as_ref().is_some_and(|e| e <= &start) {
            return Err(Error::new(
                "INVALID_INTERVAL",
                "data",
                "rate interval must have start < end",
            ));
        }
        Ok(d >= start && end.as_ref().is_none_or(|e| &d < e))
    } else {
        Ok(true)
    }
}
/// Checks every actual run performs, in execution order. A failed run reports the
/// checks it completed, the one that failed and the rest as not-run.
const RUN_CHECKS: [&str; 10] = [
    "static",
    "access",
    "data.types",
    "data.identity",
    "data.rate_validity",
    "data.relationships",
    "lookup.cardinality",
    "lookup.coverage",
    "detail.parent",
    "evidence",
];
/// What a run has established so far; retained on failure for the manifest.
#[derive(Default)]
struct Progress {
    checks: Vec<(String, String)>,
    reads: BTreeMap<String, crate::adapters::SourceRead>,
    nodes: Vec<String>,
    timings: BTreeMap<&'static str, u128>,
}
impl Progress {
    fn pass(&mut self, id: &str, coverage: impl Into<String>) {
        self.checks.push((id.into(), coverage.into()));
    }
    fn results(&self, failure: Option<&Error>) -> Vec<serde_json::Value> {
        let failed = failure.map(|e| {
            e.check
                .clone()
                .unwrap_or_else(|| default_check(&e.code, &e.phase))
        });
        let mut results: Vec<_> = RUN_CHECKS
            .iter()
            .map(|id| match self.checks.iter().find(|(c, _)| c == id) {
                Some((_, coverage)) => {
                    serde_json::json!({"id":id,"status":"passed","coverage":coverage})
                }
                None if failed.as_deref() == Some(*id) => serde_json::json!({"id":id,"status":"failed","coverage":"incomplete; the run stopped at this check"}),
                None => serde_json::json!({"id":id,"status":"not-run","coverage":"incomplete; the run stopped before this check"}),
            })
            .collect();
        if let Some(id) = failed.filter(|id| !RUN_CHECKS.contains(&id.as_str())) {
            results.push(serde_json::json!({"id":id,"status":"failed","coverage":"incomplete; the run stopped at this check"}));
        }
        results
    }
}

/// Status of one anchor's to-one lookup, decided after counting every qualified
/// candidate. Only `matched` selects a value.
#[derive(Clone, Debug)]
struct Outcome {
    status: &'static str,
    reason: Option<String>,
    /// Unresolved because a key component comes from an unresolved earlier lookup.
    inherited: bool,
}
impl Outcome {
    fn new(rel: &Relationship, status: &'static str, detail: Option<String>) -> Self {
        Self {
            status,
            reason: detail.map(|d| format!("{} {status}: {d}", rel.id)),
            inherited: false,
        }
    }
}
type Outcomes = BTreeMap<String, Vec<Outcome>>;

struct Execution {
    tables: BTreeMap<String, Table>,
    evidence: Vec<serde_json::Value>,
    candidate_evidence: Vec<serde_json::Value>,
    detail_evidence: Vec<serde_json::Value>,
    diagnostics: Vec<Error>,
    counts: serde_json::Map<String, serde_json::Value>,
    totals: serde_json::Value,
    resolved: usize,
    unresolved: usize,
}

fn evidence_budget(message: impl Into<String>) -> Error {
    Error::new("RESOURCE_LIMIT", "evidence", message)
        .with_check("budget.evidence")
        .with_remediation(
            "Raise limits.candidate_pairs or limits.output_bytes explicitly, or narrow the selected population; evidence is never truncated.",
        )
}

/// Key components of a lookup as plain JSON, for opted-in diagnostic values.
fn key_values(row: &Row, keys: &[String]) -> serde_json::Value {
    serde_json::Value::Object(
        keys.iter()
            .map(|k| {
                (
                    k.clone(),
                    crate::adapters::plain_value(row.fields.get(k).unwrap_or(&Value::Null)),
                )
            })
            .collect(),
    )
}

/// Every validity interval of a lookup target must satisfy `valid_from < valid_to`
/// or have an explicitly null (open) `valid_to`, whether or not any line selects it.
fn check_intervals(rel: &Relationship, rows: &[Row], policy: &Policy) -> Result<()> {
    let Some(v) = &rel.validity else {
        return Ok(());
    };
    let bad: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| match row.fields.get(&v.valid_from) {
            Some(Value::Date { value: start }) => !matches!(
                row.fields.get(&v.valid_to),
                Some(Value::Null) | None
            ) && !matches!(row.fields.get(&v.valid_to), Some(Value::Date { value: end }) if end > start),
            _ => true,
        })
        .map(|(i, _)| i)
        .collect();
    if bad.is_empty() {
        return Ok(());
    }
    let values = bad
        .iter()
        .take(Error::SAMPLE_CAP)
        .map(|&i| key_values(&rows[i], &[v.valid_from.clone(), v.valid_to.clone()]))
        .collect();
    Err(Error::new(
        "INVALID_INTERVAL",
        "data",
        format!(
            "{} {} records have an empty or inverted validity interval; valid_from must be before valid_to, or valid_to null for an open upper bound",
            bad.len(),
            rel.to
        ),
    )
    .with_subject(format!("source:{}", rel.to))
    .with_check("data.rate_validity")
    .with_affected(bad.len(), bad.iter().map(|&i| &rows[i].reference))
    .with_values(policy.diagnostic_values, values))
}

/// Parent lines of each detail record: every one-to-many relationship into the
/// detail source that matched it.
fn detail_parents<'a>(
    recipe: &'a Recipe,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
    detail: &str,
) -> Vec<Vec<(&'a Relationship, usize)>> {
    let mut parents = vec![vec![]; sources[detail].len()];
    for rel in recipe
        .relationships
        .iter()
        .filter(|r| r.to == detail && r.cardinality == Cardinality::OneToMany)
    {
        for (parent, found) in matches[&rel.id].iter().enumerate() {
            for &child in found {
                parents[child].push((rel, parent));
            }
        }
    }
    parents
}

fn execute(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &RunControl<'_>,
    progress: &mut Progress,
) -> Result<Execution> {
    check(cancel)?;
    cancel.entered(RunPhase::Parse);
    let phase = Instant::now();
    let (sources, reads) =
        crate::adapters::read_sources_observed(recipe, bindings, policy, cancel)?;
    progress.reads = reads;
    let rows_read: usize = sources.values().map(Vec::len).sum();
    progress.pass(
        "access",
        format!(
            "{} bound sources opened under the read policy",
            sources.len()
        ),
    );
    progress.pass(
        "data.types",
        format!("full: every projected field of every record; {rows_read} source records"),
    );
    for source in &recipe.sources {
        let rows = &sources[&source.id];
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            let k = key(row, &source.identity).ok_or_else(|| {
                Error::new("NULL_IDENTITY", "data", source.id.clone())
                    .with_subject(format!("source:{}", source.id))
            })?;
            if !seen.insert(k) {
                return Err(Error::new("DUPLICATE_IDENTITY", "data", source.id.clone())
                    .with_subject(format!("source:{}", source.id)));
            }
        }
        progress.nodes.push(format!("source:{}", source.id));
    }
    progress.pass(
        "data.identity",
        format!("full: declared identity of every record; {rows_read} source records"),
    );
    progress.timings.insert("read", phase.elapsed().as_millis());
    let phase = Instant::now();
    let mut validated = 0;
    for rel in &recipe.relationships {
        check_intervals(rel, &sources[&rel.to], policy)?;
        if rel.validity.is_some() {
            validated += sources[&rel.to].len();
        }
    }
    progress.pass(
        "data.rate_validity",
        format!("full: every validity interval of every date-qualified lookup target; {validated} interval records"),
    );
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_pool(cancel.memory_pool())
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()
        .map_err(engine_error)?;
    let ctx = SessionContext::new_with_config_rt(
        SessionConfig::new()
            .with_target_partitions(PARTITIONS)
            .with_batch_size(engine_batch_rows(policy))
            .with_repartition_joins(false)
            .with_round_robin_repartition(false),
        runtime,
    );
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(PARTITIONS)
        .thread_name(WORKER_THREAD)
        .enable_all()
        .build()
        .map_err(engine_error)?;
    let mut matches = Matches::new();
    let mut outcomes = Outcomes::new();
    let mut diagnostics = vec![];
    let mut total = 0;
    let mut quarantined = 0;
    let mut unmatched_anchors = 0;
    let mut ordered = vec![];
    let mut pending: Vec<_> = recipe.relationships.iter().collect();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|rel| {
            let ready = rel
                .from_keys
                .iter()
                .filter_map(|k| k.split_once('.').map(|(id, _)| id))
                .all(|id| ordered.iter().any(|r: &&Relationship| r.id == id));
            if ready {
                ordered.push(*rel);
                false
            } else {
                true
            }
        });
        if pending.len() == before {
            return Err(Error::new(
                "DEPENDENCY_CYCLE",
                "validation",
                "relationship key dependencies",
            ));
        }
    }
    for rel in ordered {
        check(cancel)?;
        progress.nodes.push(format!("relationship:{}", rel.id));
        let subject = format!("relationship:{}", rel.id);
        let mut effective_left = sources[&rel.from].clone();
        let mut blocked: Vec<Option<Outcome>> = vec![None; effective_left.len()];
        for key_name in &rel.from_keys {
            if let Some((dependency, field)) = key_name.split_once('.') {
                let prior = recipe
                    .relationships
                    .iter()
                    .find(|r| r.id == dependency)
                    .ok_or_else(|| Error::new("UNKNOWN_PATH", "execution", dependency))?;
                for (i, row) in effective_left.iter_mut().enumerate() {
                    let outcome = &outcomes[dependency][i];
                    let value = if outcome.status == "matched" {
                        sources[&prior.to][matches[dependency][i][0]]
                            .fields
                            .get(field)
                            .cloned()
                            .unwrap_or(Value::Null)
                    } else {
                        if blocked[i].is_none() {
                            blocked[i] = Some(Outcome {
                                status: outcome.status,
                                reason: Some(format!(
                                    "{} {}: requires {dependency} ({})",
                                    rel.id,
                                    outcome.status,
                                    outcome.reason.as_deref().unwrap_or(outcome.status)
                                )),
                                inherited: true,
                            });
                        }
                        Value::Null
                    };
                    row.fields.insert(key_name.clone(), value);
                }
            }
        }
        let left = &effective_left;
        let right = &sources[&rel.to];
        let mut index_by_key: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, row) in right.iter().enumerate() {
            if let Some(k) = key(row, &rel.to_keys) {
                index_by_key.entry(k).or_default().push(i);
            }
        }
        // Count every date-qualified candidate before any selection.
        let mut key_candidates = vec![0usize; left.len()];
        for (l, row) in left.iter().enumerate() {
            if let Some(candidates) = key(row, &rel.from_keys).and_then(|k| index_by_key.get(&k)) {
                key_candidates[l] = candidates.len();
                for candidate in candidates {
                    if qualified(rel, row, &right[*candidate])? {
                        total += 1;
                        if total > policy.limits.candidate_pairs {
                            return Err(evidence_budget(format!(
                                "qualified lookup candidates exceed limits.candidate_pairs ({}); candidate evidence would be incomplete",
                                policy.limits.candidate_pairs
                            ))
                            .with_subject(subject.clone()));
                        }
                    }
                }
            }
        }
        // Candidate pairs come from an engine equi-join; ISO dates compare correctly as text.
        let left_frame = ctx
            .read_batch(batch(
                "l_",
                left,
                &rel.from_keys,
                [rel.validity.as_ref().map(|v| v.date.as_str()), None, None],
            )?)
            .map_err(engine_error)?;
        let right_frame = ctx
            .read_batch(batch(
                "r_",
                right,
                &rel.to_keys,
                [
                    None,
                    rel.validity.as_ref().map(|v| v.valid_from.as_str()),
                    rel.validity.as_ref().map(|v| v.valid_to.as_str()),
                ],
            )?)
            .map_err(engine_error)?;
        let temporal = rel.validity.as_ref().map(|_| {
            col("l_date")
                .gt_eq(col("r_start"))
                .and(col("r_end").is_null().or(col("l_date").lt(col("r_end"))))
        });
        cancel.entered(RunPhase::Join);
        let batches: Vec<RecordBatch> = rt.block_on(async {
            let frame=left_frame.join(right_frame,JoinType::Inner,&["l_key"],&["r_key"],temporal)?.select_columns(&["l_idx","r_idx"])?;
            let work=frame.collect();tokio::pin!(work);
            loop {tokio::select! {result=&mut work => break result, _=tokio::time::sleep(std::time::Duration::from_millis(50)) => {if cancel.load(Ordering::Acquire){break Err(datafusion::error::DataFusionError::Execution("run cancelled".into()));}}}}
        }).map_err(|e|if cancel.load(Ordering::Acquire){Error::new("CANCELLED","execution","run cancelled")}else{engine_error(e)})?;
        let mut found = vec![vec![]; left.len()];
        for b in batches {
            check(cancel)?;
            let li = b.column(0).as_any().downcast_ref::<UInt64Array>().unwrap();
            let ri = b.column(1).as_any().downcast_ref::<UInt64Array>().unwrap();
            for n in 0..b.num_rows() {
                let l = li.value(n) as usize;
                let r = ri.value(n) as usize;
                if qualified(rel, &left[l], &right[r])? {
                    found[l].push(r);
                }
            }
        }
        for candidates in &mut found {
            candidates.sort_unstable();
        }
        // One-to-one and one-to-many targets belong to at most one anchor.
        let mut uses = vec![0usize; right.len()];
        for &r in found.iter().flatten() {
            uses[r] += 1;
        }
        let shared: Vec<bool> = found
            .iter()
            .map(|f| rel.cardinality != Cardinality::ManyToOne && f.iter().any(|&r| uses[r] > 1))
            .collect();
        let to_one = rel.cardinality != Cardinality::OneToMany;
        let violating: Vec<usize> = (0..left.len())
            .filter(|&l| shared[l] || (to_one && found[l].len() > 1))
            .collect();
        let sample_values = |rows: &[usize]| -> Vec<serde_json::Value> {
            rows.iter()
                .take(Error::SAMPLE_CAP)
                .map(|&l| key_values(&left[l], &rel.from_keys))
                .collect()
        };
        if rel.strict && !violating.is_empty() {
            return Err(Error::new(
                "CARDINALITY_VIOLATION",
                "lookup",
                format!(
                    "{} {} records violate the declared {:?} cardinality of strict lookup {}",
                    violating.len(),
                    rel.from,
                    rel.cardinality,
                    rel.id
                ),
            )
            .with_subject(subject.clone())
            .with_check("data.relationships")
            .with_affected(
                violating.len(),
                violating.iter().map(|&l| &left[l].reference),
            )
            .with_values(policy.diagnostic_values, sample_values(&violating)));
        }
        let mut results = Vec::with_capacity(left.len());
        for l in 0..left.len() {
            let n = found[l].len();
            let outcome = if !to_one {
                Outcome::new(rel, if n > 0 { "matched" } else { "unmatched" }, None)
            } else if shared[l] {
                Outcome::new(
                    rel,
                    "ambiguous",
                    Some("one_to_one target record is also matched by another anchor".into()),
                )
            } else if n == 1 {
                Outcome::new(rel, "matched", None)
            } else if n > 1 {
                Outcome::new(rel, "ambiguous", Some(format!("{n} qualified candidates")))
            } else if let Some(inherited) = blocked[l].take() {
                inherited
            } else if let Some(k) = rel
                .from_keys
                .iter()
                .find(|k| matches!(left[l].fields.get(*k), None | Some(Value::Null)))
            {
                Outcome::new(rel, "unmatched", Some(format!("null key component {k}")))
            } else if let (Some(v), true) = (&rel.validity, key_candidates[l] > 0) {
                Outcome::new(
                    rel,
                    "unmatched",
                    Some(format!(
                        "{} key candidates, none valid on {}",
                        key_candidates[l], v.date
                    )),
                )
            } else {
                Outcome::new(
                    rel,
                    "unmatched",
                    Some("no candidate with an equal key".into()),
                )
            };
            results.push(outcome);
        }
        if to_one {
            for (code, status, check_id, what) in [
                (
                    "LOOKUP_AMBIGUOUS",
                    "ambiguous",
                    "lookup.cardinality",
                    "more than one qualified candidate",
                ),
                (
                    "LOOKUP_UNMATCHED",
                    "unmatched",
                    "lookup.coverage",
                    "no qualified candidate",
                ),
            ] {
                let affected: Vec<usize> = (0..left.len())
                    .filter(|&l| !results[l].inherited && results[l].status == status)
                    .collect();
                if affected.is_empty() {
                    continue;
                }
                if status == "ambiguous" {
                    quarantined += affected.len();
                } else {
                    unmatched_anchors += affected.len();
                }
                diagnostics.push(
                    Error::new(
                        code,
                        "lookup",
                        format!(
                            "{} {} records have {what} in {}; each is kept with null dependent values and status {status}",
                            affected.len(),
                            rel.from,
                            rel.id
                        ),
                    )
                    .warning()
                    .with_subject(subject.clone())
                    .with_check(check_id)
                    .with_affected(affected.len(), affected.iter().map(|&l| &left[l].reference))
                    .with_values(policy.diagnostic_values, sample_values(&affected)),
                );
            }
        } else if !violating.is_empty() {
            diagnostics.push(
                Error::new(
                    "DETAIL_MULTIPLE_PARENTS",
                    "lookup",
                    format!(
                        "{} {} records share detail records through {}; shared detail has no single parent",
                        violating.len(),
                        rel.from,
                        rel.id
                    ),
                )
                .warning()
                .with_subject(subject.clone())
                .with_check("detail.parent")
                .with_affected(violating.len(), violating.iter().map(|&l| &left[l].reference))
                .with_values(policy.diagnostic_values, sample_values(&violating)),
            );
        }
        matches.insert(rel.id.clone(), found);
        outcomes.insert(rel.id.clone(), results);
    }
    progress.pass(
        "data.relationships",
        "full: declared cardinality of every lookup over every anchor record; no strict lookup was violated",
    );
    progress.pass(
        "lookup.cardinality",
        format!("full: qualified candidates counted before selection for every anchor record; {quarantined} anchor records quarantined as ambiguous"),
    );
    progress.pass(
        "lookup.coverage",
        format!("full: every anchor record of every lookup; {unmatched_anchors} anchor records unmatched"),
    );
    diagnostics.extend(orphan_details(recipe, &sources, &matches));
    progress.pass(
        "detail.parent",
        "full: every detail record checked for a parent line",
    );
    progress
        .timings
        .insert("lookups", phase.elapsed().as_millis());
    let phase = Instant::now();
    let mut tables = BTreeMap::new();
    let mut evidence = vec![];
    let mut candidate_evidence = vec![];
    let mut detail_evidence = vec![];
    let mut counts = serde_json::Map::new();
    let mut populations: Vec<(&Output, Vec<(String, bool)>)> = vec![];
    let (mut resolved, mut unresolved) = (0, 0);
    for output in &recipe.outputs {
        progress.nodes.push(format!("output:{}", output.id));
        let parents = output
            .detail
            .then(|| detail_parents(recipe, &sources, &matches, &output.anchor));
        let anchor_rows = &sources[&output.anchor];
        let mut table = vec![];
        let mut population = vec![];
        let mut eligible = 0usize;
        let mut statuses: BTreeMap<&'static str, usize> = BTreeMap::new();
        for (row_index, row) in anchor_rows.iter().enumerate() {
            check(cancel)?;
            let mut reasons: Vec<Outcome> = vec![];
            let mut resolver = |source: &str, field: &str, path: &[String]| -> Result<Value> {
                let mut current = output.anchor.as_str();
                let mut position = row_index;
                for rel_id in path {
                    let rel = recipe
                        .relationships
                        .iter()
                        .find(|r| &r.id == rel_id)
                        .ok_or_else(|| Error::new("UNKNOWN_PATH", "execution", rel_id.clone()))?;
                    if rel.from != current {
                        return Err(Error::new("INVALID_PATH", "execution", rel.id.clone()));
                    }
                    let outcome = &outcomes[&rel.id][position];
                    if outcome.status != "matched" || matches[&rel.id][position].len() != 1 {
                        reasons.push(outcome.clone());
                        return Ok(Value::Null);
                    }
                    position = matches[&rel.id][position][0];
                    current = &rel.to;
                }
                if current != source {
                    return Err(Error::new("INVALID_PATH", "execution", source.to_string()));
                }
                Ok(sources[current][position]
                    .fields
                    .get(field)
                    .cloned()
                    .unwrap_or(Value::Null))
            };
            if let Some(predicate) = &output.eligibility {
                if !matches!(
                    crate::eval::eval(predicate, recipe, &mut resolver)?,
                    Value::Boolean { value: true }
                ) {
                    continue;
                }
            }
            eligible += 1;
            let mut values = BTreeMap::new();
            for column in &output.columns {
                values.insert(
                    column.name.clone(),
                    crate::eval::eval(
                        &prepare(
                            &column.expr,
                            recipe,
                            &sources,
                            &matches,
                            &output.anchor,
                            row_index,
                        )?,
                        recipe,
                        &mut resolver,
                    )?,
                );
            }
            for aggregate in &output.aggregates {
                values.insert(
                    format!("__aggregate_{}", aggregate.name),
                    crate::eval::eval(
                        &prepare(
                            &aggregate.expr,
                            recipe,
                            &sources,
                            &matches,
                            &output.anchor,
                            row_index,
                        )?,
                        recipe,
                        &mut resolver,
                    )?,
                );
            }
            if let Some(predicate) = &output.filter {
                if !matches!(
                    crate::eval::eval(predicate, recipe, &mut resolver)?,
                    Value::Boolean { value: true }
                ) {
                    continue;
                }
            }
            let mut parent_reference = None;
            if let Some(parents) = &parents {
                let found = &parents[row_index];
                let (status, detail) = match found.len() {
                    1 => ("matched", None),
                    0 => ("unmatched", Some("no parent line".to_string())),
                    n => ("ambiguous", Some(format!("{n} parent lines"))),
                };
                reasons.insert(
                    0,
                    Outcome {
                        status,
                        reason: detail.map(|d| format!("parent {status}: {d}")),
                        inherited: false,
                    },
                );
                let parent = match found.as_slice() {
                    [(rel, index)] => {
                        let parent_source = recipe.sources.iter().find(|s| s.id == rel.from);
                        let row = &sources[&rel.from][*index];
                        parent_reference = Some(row.reference.clone());
                        let identity = parent_source
                            .map(|s| key_values(row, &s.identity))
                            .unwrap_or_default();
                        Value::String {
                            value: serde_json::json!({"relationship":rel.id,"source":rel.from,"identity":identity}).to_string(),
                        }
                    }
                    _ => Value::Null,
                };
                values.insert("_parent".into(), parent);
            }
            reasons.retain(|o| o.status != "matched");
            let status = if reasons.iter().any(|o| o.status == "ambiguous") {
                "ambiguous"
            } else if reasons.is_empty() {
                "matched"
            } else {
                "unmatched"
            };
            let mut texts: Vec<String> = vec![];
            for text in reasons.iter().filter_map(|o| o.reason.clone()) {
                if !texts.contains(&text) {
                    texts.push(text);
                }
            }
            *statuses.entry(status).or_default() += 1;
            if !output.detail {
                if status == "matched" {
                    resolved += 1;
                } else {
                    unresolved += 1;
                }
            }
            values.insert(
                "_status".into(),
                Value::String {
                    value: status.into(),
                },
            );
            values.insert(
                "_reason".into(),
                if texts.is_empty() {
                    Value::Null
                } else {
                    Value::String {
                        value: texts.join("; "),
                    }
                },
            );
            let currency = match row.fields.get("currency") {
                Some(Value::String { value }) => value.clone(),
                _ => "undeclared".into(),
            };
            population.push((currency, status == "matched"));
            let mut lookups = vec![];
            for rel in recipe
                .relationships
                .iter()
                .filter(|r| r.from == output.anchor)
            {
                let found = &matches[&rel.id][row_index];
                let outcome = &outcomes[&rel.id][row_index];
                let to_one = rel.cardinality != Cardinality::OneToMany;
                let references = |ids: &[usize]| {
                    ids.iter()
                        .map(|i| &sources[&rel.to][*i].reference)
                        .collect::<Vec<_>>()
                };
                for &candidate in found {
                    let target = &sources[&rel.to][candidate];
                    if to_one {
                        candidate_evidence.push(serde_json::json!({"output":output.id,"relationship":rel.id,"anchor":row.reference,"status":outcome.status,"candidate_count":found.len(),"selected":outcome.status == "matched","candidate":target.reference,"original":target.raw}));
                    } else {
                        detail_evidence.push(serde_json::json!({"output":output.id,"relationship":rel.id,"parent":row.reference,"detail":target.reference,"original":target.raw}));
                    }
                }
                let selected = (to_one && outcome.status == "matched")
                    .then(|| &sources[&rel.to][found[0]].reference);
                let ambiguous = if to_one && outcome.status == "ambiguous" {
                    references(found)
                } else {
                    vec![]
                };
                lookups.push(serde_json::json!({"relationship":rel.id,"cardinality":rel.cardinality,"status":outcome.status,"reason":outcome.reason,"candidate_count":found.len(),"selected":selected,"ambiguous_references":ambiguous,"candidates":found.iter().map(|i|&sources[&rel.to][*i]).collect::<Vec<_>>()}));
            }
            let mut record = serde_json::json!({"output":output.id,"source":row.reference,"original":row.raw,"status":status,"reason":values["_reason"],"relationships":lookups});
            if output.detail {
                record["parent"] = serde_json::json!(parent_reference);
            }
            evidence.push(record);
            table.push(values);
        }
        let output_rows = table.len();
        let mut count = serde_json::json!({"source_rows":anchor_rows.len(),"eligible_rows":eligible,"output_rows":output_rows});
        let tally = |s: &str| statuses.get(s).copied().unwrap_or(0);
        if output.detail {
            count["parented_rows"] = tally("matched").into();
            count["orphan_rows"] = tally("unmatched").into();
            count["ambiguous_parent_rows"] = tally("ambiguous").into();
        } else {
            count["resolved_rows"] = tally("matched").into();
            count["unresolved_rows"] = (output_rows - tally("matched")).into();
        }
        counts.insert(output.id.clone(), count);
        if !output.aggregates.is_empty() {
            let mut groups: BTreeMap<String, Vec<BTreeMap<String, Value>>> = BTreeMap::new();
            for row in table {
                let key = serde_json::to_string(
                    &output
                        .group_by
                        .iter()
                        .map(|k| row.get(k).cloned().unwrap_or(Value::Null))
                        .collect::<Vec<_>>(),
                )
                .map_err(engine_error)?;
                groups.entry(key).or_default().push(row);
            }
            table = vec![];
            for rows in groups.values() {
                let mut result = BTreeMap::new();
                for key in &output.group_by {
                    result.insert(key.clone(), rows[0][key].clone());
                }
                for aggregate in &output.aggregates {
                    let values = rows
                        .iter()
                        .map(|r| r[&format!("__aggregate_{}", aggregate.name)].clone())
                        .collect::<Vec<_>>();
                    result.insert(
                        aggregate.name.clone(),
                        reduce(&values, &aggregate.function)?,
                    );
                }
                result.insert(
                    "_status".into(),
                    Value::String {
                        value: if rows.iter().any(|r| {
                            r["_status"]
                                != (Value::String {
                                    value: "matched".into(),
                                })
                        }) {
                            "unresolved"
                        } else {
                            "matched"
                        }
                        .into(),
                    },
                );
                table.push(result);
            }
        } else if !output.detail {
            populations.push((output, population));
        }
        tables.insert(output.id.clone(), table);
    }
    let totals = review_totals(recipe, &tables, &populations)?;
    let evidence_rows = candidate_evidence.len() + detail_evidence.len();
    if evidence_rows > policy.limits.candidate_pairs {
        return Err(evidence_budget(format!(
            "{evidence_rows} candidate and detail evidence records exceed limits.candidate_pairs ({})",
            policy.limits.candidate_pairs
        )));
    }
    let bytes = serde_json::to_vec(&(&tables, &evidence, &candidate_evidence, &detail_evidence))
        .map_err(engine_error)?
        .len();
    if bytes > policy.limits.output_bytes || bytes > policy.limits.retained_bytes {
        // The memory result is retained only if it fits; this is a delivery budget.
        let mut error = evidence_budget(format!(
            "result and evidence tables ({bytes} bytes) exceed the output/retained byte budget"
        ));
        error.phase = "delivery".into();
        return Err(error);
    }
    progress.pass(
        "evidence",
        format!(
            "complete: {} primary, {} candidate and {} detail evidence records within the declared bounds",
            evidence.len(),
            candidate_evidence.len(),
            detail_evidence.len()
        ),
    );
    progress
        .timings
        .insert("outputs", phase.elapsed().as_millis());
    Ok(Execution {
        tables,
        evidence,
        candidate_evidence,
        detail_evidence,
        diagnostics,
        counts,
        totals,
        resolved,
        unresolved,
    })
}

/// Exact sum of one numeric column over a subset of lines. A subset containing a
/// null has no total: the total is reported as null with the null count beside it.
#[derive(Default, Clone, Copy)]
struct Sum {
    value: i128,
    nulls: usize,
}
#[derive(Clone, Copy, PartialEq)]
enum Numeric {
    Int,
    Decimal(u8),
}
#[derive(Default, Clone)]
struct ColumnTotals {
    population: Sum,
    comparable: Sum,
    unresolved: Sum,
}
#[derive(Default, Clone)]
struct CurrencyTotals {
    rows: [usize; 3],
    columns: BTreeMap<String, ColumnTotals>,
}
fn add(sum: &mut Sum, value: Option<i128>) -> Result<()> {
    match value {
        None => sum.nulls += 1,
        Some(v) => {
            sum.value = sum.value.checked_add(v).ok_or_else(|| {
                Error::new("ARITHMETIC_OVERFLOW", "report", "review total overflows")
            })?
        }
    }
    Ok(())
}
fn render(sum: Sum, kind: Numeric) -> Result<serde_json::Value> {
    if sum.nulls > 0 {
        return Ok(serde_json::Value::Null);
    }
    Ok(match kind {
        Numeric::Int => serde_json::json!(sum.value.to_string()),
        Numeric::Decimal(scale) => {
            if sum.value.unsigned_abs() >= 10u128.pow(38) {
                return Err(Error::new(
                    "ARITHMETIC_OVERFLOW",
                    "report",
                    "review total exceeds decimal precision 38",
                ));
            }
            serde_json::json!(
                crate::scalar::Decimal {
                    coefficient: sum.value,
                    precision: 38,
                    scale,
                }
                .to_string()
            )
        }
    })
}
fn currency_json(
    totals: &CurrencyTotals,
    kinds: &BTreeMap<String, Numeric>,
) -> Result<serde_json::Value> {
    let mut columns = serde_json::Map::new();
    for (name, t) in &totals.columns {
        let kind = kinds[name];
        columns.insert(
            name.clone(),
            serde_json::json!({"population":render(t.population, kind)?,"comparable":render(t.comparable, kind)?,"unresolved":render(t.unresolved, kind)?,"null_rows":t.population.nulls}),
        );
    }
    Ok(
        serde_json::json!({"rows":{"population":totals.rows[0],"resolved":totals.rows[1],"unresolved":totals.rows[2]},"columns":columns}),
    )
}
/// Full-population, comparable (resolved) and unresolved totals of every numeric
/// column, by currency, per output and combined across the primary outputs.
fn review_totals(
    recipe: &Recipe,
    tables: &BTreeMap<String, Table>,
    populations: &[(&Output, Vec<(String, bool)>)],
) -> Result<serde_json::Value> {
    let mut by_output = serde_json::Map::new();
    let mut combined: BTreeMap<String, CurrencyTotals> = BTreeMap::new();
    let mut combined_kinds: BTreeMap<String, Option<Numeric>> = BTreeMap::new();
    for (output, population) in populations {
        let mut kinds = BTreeMap::new();
        for column in &output.columns {
            let kind = match crate::validation::expression_type(&column.expr, recipe) {
                Ok(Some(Type::Int64)) => Numeric::Int,
                Ok(Some(Type::Decimal { scale, .. })) => Numeric::Decimal(scale),
                _ => continue,
            };
            kinds.insert(column.name.clone(), kind);
            let entry = combined_kinds
                .entry(column.name.clone())
                .or_insert(Some(kind));
            if *entry != Some(kind) {
                *entry = None;
            }
        }
        let mut per_currency: BTreeMap<String, CurrencyTotals> = BTreeMap::new();
        for (row, (currency, matched)) in tables[&output.id].iter().zip(population) {
            for totals in [
                per_currency.entry(currency.clone()).or_default(),
                combined.entry(currency.clone()).or_default(),
            ] {
                totals.rows[0] += 1;
                totals.rows[if *matched { 1 } else { 2 }] += 1;
                for name in kinds.keys() {
                    let value = match &row[name] {
                        Value::Null => None,
                        Value::Int64 { value } => {
                            Some(value.parse::<i128>().map_err(engine_error)?)
                        }
                        Value::Decimal {
                            value,
                            precision,
                            scale,
                        } => Some(
                            crate::scalar::Decimal::parse(value, *precision, *scale)
                                .map_err(engine_error)?
                                .coefficient,
                        ),
                        _ => return Err(engine_error("numeric column holds a non-numeric value")),
                    };
                    let column = totals.columns.entry(name.clone()).or_default();
                    add(&mut column.population, value)?;
                    add(
                        if *matched {
                            &mut column.comparable
                        } else {
                            &mut column.unresolved
                        },
                        value,
                    )?;
                }
            }
        }
        let mut currencies = serde_json::Map::new();
        for (currency, totals) in &per_currency {
            currencies.insert(currency.clone(), currency_json(totals, &kinds)?);
        }
        by_output.insert(output.id.clone(), currencies.into());
    }
    let kinds: BTreeMap<String, Numeric> = combined_kinds
        .into_iter()
        .filter_map(|(name, kind)| kind.map(|k| (name, k)))
        .collect();
    let mut all = serde_json::Map::new();
    for (currency, mut totals) in combined {
        totals.columns.retain(|name, _| kinds.contains_key(name));
        all.insert(currency, currency_json(&totals, &kinds)?);
    }
    Ok(serde_json::json!({"by_output":by_output,"combined":all}))
}

/// Outputs that read a source: anchored at it or reaching it through relationships.
fn outputs_reading(recipe: &Recipe, source: &str) -> Vec<String> {
    recipe
        .outputs
        .iter()
        .filter(|output| {
            let mut seen = std::collections::BTreeSet::from([output.anchor.as_str()]);
            let mut frontier = vec![output.anchor.as_str()];
            while let Some(current) = frontier.pop() {
                for rel in recipe.relationships.iter().filter(|r| r.from == current) {
                    if seen.insert(rel.to.as_str()) {
                        frontier.push(rel.to.as_str());
                    }
                }
            }
            seen.contains(source)
        })
        .map(|o| o.id.clone())
        .collect()
}

/// Complete every published diagnostic: affected outputs from its subject, check
/// ID and remediation from its code.
fn finalize(diagnostics: Vec<Error>, recipe: &Recipe) -> Vec<Error> {
    diagnostics
        .into_iter()
        .map(|mut d| {
            if d.affected_outputs.is_empty() {
                d.affected_outputs = match d.subject.as_deref().and_then(|s| s.split_once(':')) {
                    Some(("source", id)) => outputs_reading(recipe, id),
                    Some(("relationship", id)) => recipe
                        .relationships
                        .iter()
                        .find(|r| r.id == id)
                        .map(|r| outputs_reading(recipe, &r.from))
                        .unwrap_or_default(),
                    Some(("output", id)) => vec![id.to_owned()],
                    _ => vec![],
                };
            }
            d.completed()
        })
        .collect()
}

fn manifest_base(
    id: &str,
    recipe: &Recipe,
    policy: &Policy,
    progress: &Progress,
    started: Instant,
) -> serde_json::Value {
    let bytes_read: usize = progress.reads.values().map(|r| r.bytes_read).sum();
    let mut timings: serde_json::Map<_, _> = progress
        .timings
        .iter()
        .map(|(k, v)| ((*k).to_string(), serde_json::json!(*v as u64)))
        .collect();
    timings.insert(
        "total".into(),
        serde_json::json!(started.elapsed().as_millis() as u64),
    );
    serde_json::json!({
        "run_id": id,
        "format_version": "0.1",
        "package_version": env!("CARGO_PKG_VERSION"),
        "recipe_hash": recipe.hash().ok(),
        "policy": policy,
        "diagnostic_values": policy.diagnostic_values,
        "data_checks": "full",
        "sources": progress.reads,
        "bytes_read": bytes_read,
        "executed_nodes": progress.nodes,
        "elapsed_ms": started.elapsed().as_millis(),
        "timings_ms": timings,
        "peak_rss": null,
        "engine_peak_reservation": null,
        "bytes_written": null,
        "unknown_measurements": ["peak_rss", "engine_peak_reservation", "bytes_written"],
    })
}

fn success_manifest(
    id: &str,
    recipe: &Recipe,
    policy: &Policy,
    progress: &Progress,
    run: &Execution,
    started: Instant,
) -> serde_json::Value {
    let mut manifest = manifest_base(id, recipe, policy, progress, started);
    let lines = run.resolved + run.unresolved;
    let reconciliation = if lines == 0 {
        serde_json::json!({"status":"empty","statement":"The selected population is empty; nothing about excluded inputs is claimed."})
    } else if run.unresolved == 0 {
        serde_json::json!({"status":"complete","statement":"Every selected line resolved; comparable and full-population totals coincide."})
    } else {
        serde_json::json!({"status":"partial","statement":format!("Comparable totals cover {} resolved lines only; {} unresolved lines keep null expected values and are not reconciled.", run.resolved, run.unresolved)})
    };
    let mut reconciliation = reconciliation;
    reconciliation["resolved_rows"] = run.resolved.into();
    reconciliation["unresolved_rows"] = run.unresolved.into();
    let fields = serde_json::json!({
        "source_rows": progress.reads.iter().map(|(k, r)| (k.clone(), r.rows)).collect::<BTreeMap<_, _>>(),
        "counts": run.counts,
        "resolved_rows": run.resolved,
        "unresolved_rows": run.unresolved,
        "totals": run.totals,
        "reconciliation": reconciliation,
        "evidence": {
            "primary_rows": run.evidence.len(),
            "candidate_rows": run.candidate_evidence.len(),
            "detail_rows": run.detail_evidence.len(),
            "complete": true,
            "bounds": {"candidate_pairs": policy.limits.candidate_pairs, "output_bytes": policy.limits.output_bytes},
        },
        "checks": {"static": "passed", "data": "passed"},
        "check_results": progress.results(None),
        "complete": true,
    });
    if let (Some(target), serde_json::Value::Object(fields)) = (manifest.as_object_mut(), fields) {
        target.extend(fields);
    }
    manifest
}

fn finish_failed(
    id: String,
    error: Error,
    started: Instant,
    recipe: &Recipe,
    policy: &Policy,
    progress: &Progress,
) -> RunReport {
    let error = finalize(vec![error], recipe).remove(0);
    let mut manifest = manifest_base(&id, recipe, policy, progress, started);
    let fields = serde_json::json!({
        "source_rows": "incomplete",
        "counts": "incomplete",
        "totals": "incomplete",
        "reconciliation": "incomplete",
        "evidence": "incomplete",
        "checks": {"data": "incomplete"},
        "check_results": progress.results(Some(&error)),
        "complete": false,
    });
    if let (Some(target), serde_json::Value::Object(fields)) = (manifest.as_object_mut(), fields) {
        target.extend(fields);
    }
    let mut report = failed(id, error, started);
    report.manifest = manifest;
    report
}

/// Detail records that no one-to-many relationship attaches to any parent are reported
/// as source diagnostics (bounded reference samples, no values); they never become lines.
fn orphan_details(
    recipe: &Recipe,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
) -> Vec<Error> {
    let mut attached: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    for rel in &recipe.relationships {
        if rel.cardinality != Cardinality::OneToMany {
            continue;
        }
        let seen = attached
            .entry(rel.to.as_str())
            .or_insert_with(|| vec![false; sources[&rel.to].len()]);
        for &child in matches[&rel.id].iter().flatten() {
            seen[child] = true;
        }
    }
    attached
        .into_iter()
        .filter_map(|(detail, seen)| {
            let orphans: Vec<_> = seen
                .iter()
                .enumerate()
                .filter(|(_, attached)| !**attached)
                .map(|(i, _)| &sources[detail][i].reference)
                .collect();
            if orphans.is_empty() {
                return None;
            }
            let mut diagnostic = Error::new(
                "ORPHAN_DETAIL",
                "source",
                format!(
                    "{} {detail} detail records have no parent line; they are not output lines",
                    orphans.len()
                ),
            )
            .warning()
            .with_subject(format!("source:{detail}"))
            .with_check("detail.parent")
            .with_affected(orphans.len(), orphans.iter().copied());
            diagnostic.context = Some(
                serde_json::json!({"severity":"warning","source":detail,"count":orphans.len(),"samples":orphans.iter().take(Error::SAMPLE_CAP).collect::<Vec<_>>(),"truncated":orphans.len() > Error::SAMPLE_CAP})
                    .to_string(),
            );
            Some(diagnostic)
        })
        .collect()
}
impl ExecutionContext {
    /// Run `recipe` and deliver one local directory bundle at a NEW `destination`.
    /// The context stays busy until delivery finishes; after the atomic commit the
    /// run cannot be cancelled.
    pub fn run_bundle<C: Cancellation + ?Sized>(
        &self,
        recipe: &Recipe,
        bindings: &Bindings,
        policy: &Policy,
        destination: &std::path::Path,
        cancel: &C,
    ) -> RunReport {
        let Some(_busy) = self.admit() else {
            return busy(policy);
        };
        let control = RunControl::new(&cancel, policy);
        let mut report = self.run_admitted(recipe, bindings, policy, &control);
        control.record(&mut report, recipe, policy);
        if report.status == "failed" || report.status == "cancelled" {
            return report;
        }
        crate::admission::deliver(&mut report, recipe, policy, destination, &control);
        report
    }
}
fn reduce(values: &[Value], function: &str) -> Result<Value> {
    if function == "count_records" {
        return Ok(Value::Int64 {
            value: values.len().to_string(),
        });
    }
    let values: Vec<_> = values.iter().filter(|v| **v != Value::Null).collect();
    if function == "count_non_null" {
        return Ok(Value::Int64 {
            value: values.len().to_string(),
        });
    }
    let Some(first) = values.first() else {
        return Ok(Value::Null);
    };
    match function {
        "min" | "max" => {
            let mut chosen = (*first).clone();
            for value in values.into_iter().skip(1) {
                let cmp = crate::eval::compare(value, &chosen)?;
                if (function == "min" && cmp == Some(std::cmp::Ordering::Less))
                    || (function == "max" && cmp == Some(std::cmp::Ordering::Greater))
                {
                    chosen = value.clone();
                }
            }
            Ok(chosen)
        }
        "sum" => match first {
            Value::Int64 { .. } => {
                let mut sum = 0i64;
                for value in values {
                    if let Value::Int64 { value } = value {
                        sum = sum
                            .checked_add(value.parse().map_err(engine_error)?)
                            .ok_or_else(|| {
                                Error::new("ARITHMETIC_OVERFLOW", "aggregate", "Int64 sum")
                            })?;
                    } else {
                        return Err(Error::new("TYPE_MISMATCH", "aggregate", "mixed sum types"));
                    }
                }
                Ok(Value::Int64 {
                    value: sum.to_string(),
                })
            }
            Value::Decimal { scale, .. } => {
                let scale = *scale;
                let mut sum = num_bigint::BigInt::from(0);
                for value in values {
                    if let Value::Decimal {
                        value,
                        precision,
                        scale: s,
                    } = value
                    {
                        if *s != scale {
                            return Err(Error::new(
                                "TYPE_MISMATCH",
                                "aggregate",
                                "mixed sum scales",
                            ));
                        }
                        let d = crate::scalar::Decimal::parse(value, *precision, *s)
                            .map_err(engine_error)?;
                        sum += d.coefficient;
                    } else {
                        return Err(Error::new("TYPE_MISMATCH", "aggregate", "mixed sum types"));
                    }
                }
                use num_traits::ToPrimitive;
                let coefficient = sum
                    .to_i128()
                    .ok_or_else(|| Error::new("ARITHMETIC_OVERFLOW", "aggregate", "decimal sum"))?;
                if coefficient.unsigned_abs() >= 10u128.pow(38) {
                    return Err(Error::new(
                        "ARITHMETIC_OVERFLOW",
                        "aggregate",
                        "decimal sum precision",
                    ));
                }
                let d = crate::scalar::Decimal {
                    coefficient,
                    precision: 38,
                    scale,
                };
                Ok(Value::Decimal {
                    value: d.to_string(),
                    precision: 38,
                    scale,
                })
            }
            _ => Err(Error::new(
                "TYPE_MISMATCH",
                "aggregate",
                "sum requires numeric values",
            )),
        },
        _ => Err(Error::new(
            "UNSUPPORTED_OPERATION",
            "aggregate",
            function.to_string(),
        )),
    }
}
fn prepare(
    expr: &Expr,
    recipe: &Recipe,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
    anchor: &str,
    row_index: usize,
) -> Result<Expr> {
    let mut value = serde_json::to_value(expr).map_err(engine_error)?;
    fn visit(
        value: &mut serde_json::Value,
        recipe: &Recipe,
        sources: &BTreeMap<String, Vec<Row>>,
        matches: &Matches,
        anchor: &str,
        row_index: usize,
    ) -> Result<()> {
        if value.get("op").and_then(|x| x.as_str()) == Some("aggregate") {
            let expr: Expr = serde_json::from_value(value.clone()).map_err(engine_error)?;
            if let Expr::Aggregate {
                relationship,
                function,
                field,
                filter,
            } = expr
            {
                let rel = recipe
                    .relationships
                    .iter()
                    .find(|r| r.id == relationship)
                    .unwrap();
                if rel.from != anchor {
                    return Err(Error::new(
                        "INVALID_PATH",
                        "aggregate",
                        "aggregate must start at anchor",
                    ));
                }
                let mut values = vec![];
                for candidate in &matches[&relationship][row_index] {
                    let row = &sources[&rel.to][*candidate];
                    if let Some(p) = &filter {
                        let v = crate::eval::eval(p, recipe, &mut |source, f, path| {
                            if source != rel.to || !path.is_empty() {
                                return Err(Error::new(
                                    "INVALID_PATH",
                                    "aggregate",
                                    "aggregate filter must refer to detail source",
                                ));
                            }
                            Ok(row.fields.get(f).cloned().unwrap_or(Value::Null))
                        })?;
                        if v != (Value::Boolean { value: true }) {
                            continue;
                        }
                    }
                    values.push(
                        field
                            .as_ref()
                            .and_then(|f| row.fields.get(f))
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                *value = serde_json::to_value(Expr::Literal {
                    value: reduce(&values, &function)?,
                })
                .map_err(engine_error)?;
                return Ok(());
            }
        }
        if value.get("op").and_then(|x| x.as_str()) == Some("feature") {
            let id = value["id"].as_str().unwrap();
            let feature = recipe.features.iter().find(|f| f.id == id).unwrap();
            *value = serde_json::to_value(&feature.expr).map_err(engine_error)?;
            return visit(value, recipe, sources, matches, anchor, row_index);
        }
        match value {
            serde_json::Value::Object(map) => {
                for child in map.values_mut() {
                    visit(child, recipe, sources, matches, anchor, row_index)?;
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    visit(child, recipe, sources, matches, anchor, row_index)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(&mut value, recipe, sources, matches, anchor, row_index)?;
    serde_json::from_value(value).map_err(engine_error)
}
