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
    /// One record per partition of each allocation: quantity, recipient count,
    /// weight sum, assigned sum, residual and policy. Absent when nothing allocates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allocation_evidence: Vec<serde_json::Value>,
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
        let validating = Instant::now();
        let report = crate::validation::validate(recipe);
        progress.time("static_validation", validating);
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
        let expanded = crate::validation::expand(recipe);
        match execute(recipe, &expanded, bindings, policy, cancel, &mut progress) {
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
                    allocation_evidence: run.allocation_evidence,
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
        allocation_evidence: vec![],
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
const RUN_CHECKS: [&str; 11] = [
    "static",
    "access",
    "data.types",
    "data.identity",
    "data.rate_validity",
    "data.relationships",
    "lookup.cardinality",
    "lookup.coverage",
    "detail.parent",
    "output.population",
    "evidence",
];
/// What a run has established so far; retained on failure for the manifest.
#[derive(Default)]
struct Progress {
    checks: Vec<(String, String)>,
    reads: BTreeMap<String, crate::adapters::SourceRead>,
    nodes: Vec<String>,
    timings: BTreeMap<&'static str, u128>,
    /// Workload phases in microseconds: static and data validation, source access,
    /// parsing, Arrow conversion, planning, execution and evidence.
    phases: BTreeMap<&'static str, u64>,
}
impl Progress {
    /// Add the time since `started` to `phase`.
    fn time(&mut self, phase: &'static str, started: Instant) {
        *self.phases.entry(phase).or_default() += crate::adapters::micros(started);
    }
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
        // Checks only some recipes need (populations, allocations), in run order.
        for (id, coverage) in self
            .checks
            .iter()
            .filter(|(c, _)| !RUN_CHECKS.contains(&c.as_str()))
        {
            results.push(serde_json::json!({"id":id,"status":"passed","coverage":coverage}));
        }
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
    allocation_evidence: Vec<serde_json::Value>,
    /// Per population output: members, periods, activity coverage and the
    /// empty-member rule.
    populations: BTreeMap<String, serde_json::Value>,
    /// Per output and allocation: policy, conservation check and partitions.
    allocations: BTreeMap<String, serde_json::Value>,
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
    declared: &Recipe,
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &RunControl<'_>,
    progress: &mut Progress,
) -> Result<Execution> {
    check(cancel)?;
    cancel.entered(RunPhase::Parse);
    let phase = Instant::now();
    let (mut sources, reads) =
        crate::adapters::read_sources_observed(declared, bindings, policy, cancel)?;
    for read in reads.values() {
        for (phase, us) in [
            ("source_access", read.access_us),
            ("parse", read.parse_us),
            ("conversion", read.conversion_us),
        ] {
            *progress.phases.entry(phase).or_default() += us;
        }
    }
    progress.reads = reads;
    let validating = Instant::now();
    let rows_read: usize = sources.values().map(Vec::len).sum();
    // Checks over anchor records cover what was scanned; say so when a source
    // evaluated an eligibility predicate and kept the other records.
    let pushed_sources = progress
        .reads
        .values()
        .filter(|read| {
            read.scanned_rows.is_some()
                && read
                    .placement
                    .as_ref()
                    .is_some_and(|p| p["scan"] == "source")
        })
        .count();
    let text_note = if pushed_sources == 0 {
        String::new()
    } else {
        format!(
            "; {pushed_sources} sources evaluated an eligibility predicate or relationship target reduction, so text checks (date text, decimal text, and valid UTF-8) cover their scanned records only"
        )
    };
    let anchor_filters = progress
        .reads
        .values()
        .filter(|read| {
            read.placement.as_ref().is_some_and(|p| {
                p["scan"] == "source"
                    && p["operation"]
                        .as_str()
                        .is_some_and(|stage| stage.starts_with("output:"))
            })
        })
        .count();
    let lookup_note = if anchor_filters == 0 {
        String::new()
    } else {
        format!(
            "; {anchor_filters} sources evaluated an eligibility predicate, so lookups cover their scanned records only"
        )
    };
    progress.pass(
        "access",
        format!(
            "{} bound sources opened under the read policy",
            sources.len()
        ),
    );
    progress.pass(
        "data.types",
        format!("full: every projected field of every record read; {rows_read} source records{text_note}"),
    );
    for source in &declared.sources {
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
    check_partitions(recipe, &sources)?;
    progress.pass(
        "data.identity",
        format!("full: declared identity of every record; {rows_read} source records"),
    );
    progress.time("data_validation", validating);
    progress.timings.insert("read", phase.elapsed().as_millis());
    let population_info = materialize_populations(recipe, &mut sources, policy, cancel, progress)?;
    let phase = Instant::now();
    let validating = Instant::now();
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
    progress.time("data_validation", validating);
    let planning = Instant::now();
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
    progress.time("planning", planning);
    let executing = Instant::now();
    for rel in ordered {
        check(cancel)?;
        progress.nodes.push(format!("relationship:{}", rel.id));
        let subject = format!("relationship:{}", rel.id);
        // A period-derived key joins on one more component: the member's period
        // against the calendar period of the target's date.
        let extended;
        let period_rows;
        let (rel, right): (&Relationship, &Vec<Row>) = match &rel.period {
            Some(pm) => {
                extended = Relationship {
                    from_keys: [rel.from_keys.clone(), vec![pm.key.clone()]].concat(),
                    to_keys: [rel.to_keys.clone(), vec![DERIVED_PERIOD.into()]].concat(),
                    ..rel.clone()
                };
                period_rows = with_periods(&sources[&rel.to], &pm.date);
                (&extended, &period_rows)
            }
            None => (rel, &sources[&rel.to]),
        };
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
        format!("full: declared cardinality of every lookup over every anchor record; no strict lookup was violated{lookup_note}"),
    );
    progress.pass(
        "lookup.cardinality",
        format!("full: qualified candidates counted before selection for every anchor record; {quarantined} anchor records quarantined as ambiguous"),
    );
    progress.pass(
        "lookup.coverage",
        format!("full: every anchor record of every lookup; {unmatched_anchors} anchor records unmatched{lookup_note}"),
    );
    diagnostics.extend(orphan_details(recipe, &sources, &matches));
    progress.pass(
        "detail.parent",
        "full: every detail record checked for a parent line",
    );
    if !recipe.populations.is_empty() {
        progress.pass(
            "population.coverage",
            "full: every finer record of every population activity joined one member or was counted as unmatched; unmatched records never widen a population",
        );
    }
    progress
        .timings
        .insert("lookups", phase.elapsed().as_millis());
    let phase = Instant::now();
    let mut tables = BTreeMap::new();
    let mut evidence = vec![];
    let mut candidate_evidence = vec![];
    let mut detail_evidence = vec![];
    let mut counts = serde_json::Map::new();
    let mut allocation_evidence = vec![];
    let mut allocation_summaries: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut population_reports: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut populations: Vec<(&Output, Vec<(String, bool)>)> = vec![];
    // Distinct anchor lines of primary outputs, resolved only if resolved in every
    // output that selects them; a line read by several outputs is counted once.
    let mut lines: BTreeMap<(&str, usize), bool> = BTreeMap::new();
    let mut evidence_time = std::time::Duration::ZERO;
    let mut asserted_lines = 0usize;
    for output in &recipe.outputs {
        progress.nodes.push(format!("output:{}", output.id));
        let parents = output
            .detail
            .then(|| detail_parents(recipe, &sources, &matches, &output.anchor));
        let anchor_rows = &sources[&output.anchor];
        // Under a pushed eligibility the source holds more records than were scanned.
        let anchor_source_rows = progress
            .reads
            .get(&output.anchor)
            .map_or(anchor_rows.len(), |read| read.rows);
        let grouped = !output.aggregates.is_empty();
        let mut table = vec![];
        let mut population = vec![];
        let mut members: Vec<Member> = vec![];
        let mut eligible = 0usize;
        let mut statuses: BTreeMap<&'static str, usize> = BTreeMap::new();
        let evidence_before = evidence.len();
        let mut selected: Vec<usize> = vec![];
        // Allocation runs over every member before any row is selected, so the
        // output filter never changes a denominator.
        let mut allocations = vec![];
        for a in &output.allocations {
            check(cancel)?;
            let (members, weight_reasons) =
                allocation_members(recipe, &sources, &matches, &outcomes, output, a)?;
            let mut outcome = crate::allocation::allocate(
                output,
                a,
                &sources[&a.quantity.source],
                &members,
                policy,
            )?;
            diagnostics.append(&mut outcome.warnings);
            allocation_evidence.extend(outcome.partitions.iter().cloned());
            allocation_summaries
                .entry(output.id.clone())
                .or_insert_with(|| serde_json::json!({}))[a.name.as_str()] =
                outcome.summary.clone();
            allocations.push((a, outcome, weight_reasons));
        }
        let member_identity = crate::validation::population_output(recipe, output)
            .then(|| {
                recipe
                    .sources
                    .iter()
                    .find(|s| s.id == output.anchor)
                    .map(|s| s.identity.clone())
            })
            .flatten();
        for (row_index, row) in anchor_rows.iter().enumerate() {
            check(cancel)?;
            let mut reasons: Vec<Outcome> = vec![];
            let resolve = |reasons: &mut Vec<Outcome>,
                           source: &str,
                           field: &str,
                           path: &[String]|
             -> Result<Value> {
                resolve_path(
                    recipe,
                    &sources,
                    &matches,
                    &outcomes,
                    (&output.anchor, row_index),
                    reasons,
                    (source, field, path),
                )
            };
            // Selection predicates read lookups but do not redefine the line's status.
            let select = |predicate: &Expr, values: &BTreeMap<String, Value>| -> Result<bool> {
                let bound = bind_line(
                    predicate, recipe, &sources, &matches, output, row_index, values,
                )?;
                Ok(matches!(
                    crate::eval::eval(&bound, recipe, &mut |s: &str, f: &str, p: &[String]| {
                        resolve(&mut vec![], s, f, p)
                    })?,
                    Value::Boolean { value: true }
                ))
            };
            if let Some(predicate) = &output.eligibility {
                if !matches!(
                    crate::eval::eval(predicate, recipe, &mut |s: &str, f: &str, p: &[String]| {
                        resolve(&mut reasons, s, f, p)
                    })?,
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
                            &output.complete,
                        )?,
                        recipe,
                        &mut |s: &str, f: &str, p: &[String]| resolve(&mut reasons, s, f, p),
                    )
                    .map_err(|e| {
                        if e.code == "MIXED_CURRENCY" {
                            let mut error = e
                                .with_subject(format!("expression:{}.{}", output.id, column.name))
                                .with_affected(1, [&row.reference]);
                            // Execution stops here; this sample is not a census
                            // of expressions on records not evaluated yet.
                            error.count_status = "unknown".into();
                            error
                        } else {
                            e
                        }
                    })?,
                );
            }
            // Measure inputs at line grain; the lookups they read are part of the
            // line's status.
            let mut inputs = Vec::with_capacity(output.aggregates.len());
            for aggregate in &output.aggregates {
                let bound = bind_line(
                    &aggregate.expr,
                    recipe,
                    &sources,
                    &matches,
                    output,
                    row_index,
                    &values,
                )?;
                inputs.push(Some(crate::eval::eval(
                    &bound,
                    recipe,
                    &mut |s: &str, f: &str, p: &[String]| resolve(&mut reasons, s, f, p),
                )?));
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
            for (a, outcome, weight_reasons) in &allocations {
                let assignment = &outcome.members[row_index];
                values.insert(a.name.clone(), assignment.value.clone());
                reasons.extend(weight_reasons[row_index].iter().cloned());
                if let Some(reason) = &assignment.unresolved {
                    reasons.push(Outcome {
                        status: "unmatched",
                        reason: Some(reason.clone()),
                        inherited: false,
                    });
                }
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
            if grouped {
                // Aggregate input predicates apply before grouping, per measure.
                for (input, aggregate) in inputs.iter_mut().zip(&output.aggregates) {
                    if let Some(predicate) = &aggregate.filter {
                        if !select(predicate, &values)? {
                            *input = None;
                        }
                    }
                }
            } else if let Some(predicate) = &output.filter {
                // Final selection of lines.
                if !select(predicate, &values)? {
                    continue;
                }
            }
            *statuses.entry(status).or_default() += 1;
            if !output.detail {
                let resolved = lines
                    .entry((output.anchor.as_str(), row_index))
                    .or_insert(true);
                *resolved &= status == "matched";
            }
            // A population output keeps its totals by its own currency column.
            let currency = match if member_identity.is_some() {
                values.get("currency")
            } else {
                row.fields.get("currency")
            } {
                Some(Value::String { value }) => value.clone(),
                _ => "undeclared".into(),
            };
            population.push((currency, status == "matched"));
            let recording = Instant::now();
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
            if let Some(identity) = &member_identity {
                record["member"] = key_values(row, identity);
                if !allocations.is_empty() {
                    record["allocations"] = allocations
                        .iter()
                        .map(|(a, outcome, _)| {
                            (a.name.clone(), outcome.members[row_index].evidence.clone())
                        })
                        .collect::<serde_json::Map<_, _>>()
                        .into();
                }
            }
            if grouped {
                let keys: BTreeMap<String, Value> = output
                    .group_by
                    .iter()
                    .map(|k| (k.clone(), values.get(k).cloned().unwrap_or(Value::Null)))
                    .collect();
                record["group"] = keys
                    .iter()
                    .map(|(k, v)| (k.clone(), crate::adapters::plain_value(v)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                record["aggregate_inputs"] = output
                    .aggregates
                    .iter()
                    .zip(&inputs)
                    .map(|(a, input)| (a.name.clone(), input.is_some().into()))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                members.push(Member {
                    key: serde_json::to_string(&keys.values().collect::<Vec<_>>())
                        .map_err(engine_error)?,
                    keys,
                    matched: status == "matched",
                    inputs,
                    reference: row.reference.clone(),
                });
            } else {
                table.push(values);
            }
            evidence.push(record);
            selected.push(row_index);
            evidence_time += recording.elapsed();
        }
        check_population(
            output,
            anchor_rows,
            &selected,
            eligible,
            evidence.len() - evidence_before,
            if grouped { members.len() } else { table.len() },
        )?;
        asserted_lines += selected.len();

        let tally = |s: &str| statuses.get(s).copied().unwrap_or(0);
        let count = if grouped {
            let grouping = group(recipe, output, &members, policy)?;
            table = grouping.rows;
            serde_json::json!({
                "source_rows": anchor_source_rows,
                "eligible_rows": eligible,
                "output_rows": table.len(),
                "group_rows": grouping.groups,
                "resolved_rows": tally("matched"),
                "unresolved_rows": members.len() - tally("matched"),
                "aggregate_null_inputs": grouping.null_inputs,
            })
        } else {
            let output_rows = table.len();
            let mut count = serde_json::json!({"source_rows":anchor_source_rows,"eligible_rows":eligible,"output_rows":output_rows});
            if output.detail {
                count["parented_rows"] = tally("matched").into();
                count["orphan_rows"] = tally("unmatched").into();
                count["ambiguous_parent_rows"] = tally("ambiguous").into();
            } else {
                count["resolved_rows"] = tally("matched").into();
                count["unresolved_rows"] = (output_rows - tally("matched")).into();
                populations.push((output, population));
            }
            count
        };
        counts.insert(output.id.clone(), count);
        if let Some(p) = crate::validation::population(recipe, &output.anchor) {
            let mut report = population_info[&p.id].clone();
            report["activity"] = population_activity(recipe, output, &sources, &matches);
            population_reports.insert(output.id.clone(), report);
        }
        tables.insert(output.id.clone(), table);
    }
    progress.pass(
        "output.population",
        format!(
            "full: {asserted_lines} anchor lines across {} outputs each appear once in their output and evidence; no line was added, dropped or repeated by lookups or detail",
            recipe.outputs.len()
        ),
    );
    let resolved = lines.values().filter(|r| **r).count();
    let unresolved = lines.len() - resolved;
    if !allocation_summaries.is_empty() {
        let partitions: usize = allocation_evidence.len();
        progress.pass(
            "allocation.conservation",
            format!("full: {partitions} allocation partitions; assigned sum plus residual equals the source quantity exactly in each"),
        );
    }
    let totals = review_totals(recipe, &tables, &populations, &allocation_summaries)?;
    let budgeting = Instant::now();
    let evidence_rows = candidate_evidence.len() + detail_evidence.len();
    if evidence_rows > policy.limits.candidate_pairs {
        return Err(evidence_budget(format!(
            "{evidence_rows} candidate and detail evidence records exceed limits.candidate_pairs ({})",
            policy.limits.candidate_pairs
        )));
    }
    let mut bytes =
        serde_json::to_vec(&(&tables, &evidence, &candidate_evidence, &detail_evidence))
            .map_err(engine_error)?
            .len();
    // Allocation evidence counts only when present, so other runs keep their exact size.
    if !allocation_evidence.is_empty() {
        bytes += serde_json::to_vec(&allocation_evidence)
            .map_err(engine_error)?
            .len();
    }
    if bytes > policy.limits.output_bytes || bytes > policy.limits.retained_bytes {
        // The memory result is retained only if it fits; this is a delivery budget.
        let mut error = evidence_budget(format!(
            "result and evidence tables ({bytes} bytes) exceed the output/retained byte budget"
        ));
        error.phase = "delivery".into();
        return Err(error);
    }
    evidence_time += budgeting.elapsed();
    let evidence_us = u64::try_from(evidence_time.as_micros()).unwrap_or(u64::MAX);
    *progress.phases.entry("evidence").or_default() += evidence_us;
    *progress.phases.entry("execution").or_default() +=
        crate::adapters::micros(executing).saturating_sub(evidence_us);
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
        allocation_evidence,
        populations: population_reports,
        allocations: allocation_summaries,
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
    allocations: &BTreeMap<String, serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut by_output = serde_json::Map::new();
    let mut combined: BTreeMap<String, CurrencyTotals> = BTreeMap::new();
    let mut combined_kinds: BTreeMap<String, Option<Numeric>> = BTreeMap::new();
    for (output, population) in populations {
        // Population outputs total only what is additive at member grain: owned
        // measures and allocated columns. Broadcast context is excluded, and
        // population outputs stay out of the combined review totals.
        let members = crate::validation::population_output(recipe, output);
        let mut excluded = serde_json::Map::new();
        let mut kinds = BTreeMap::new();
        for column in &output.columns {
            let kind = match crate::validation::expression_type(&column.expr, recipe) {
                Ok(Some(Type::Int64)) => Numeric::Int,
                Ok(Some(Type::Decimal { scale, .. })) => Numeric::Decimal(scale),
                _ => continue,
            };
            if members {
                if crate::validation::additive(recipe, output, column) {
                    kinds.insert(column.name.clone(), kind);
                } else {
                    let broadcast =
                        matches!(&column.expr, Expr::Field { path, .. } if !path.is_empty());
                    excluded.insert(
                        column.name.clone(),
                        serde_json::json!({
                            "transition": if broadcast { "broadcast" } else { "not_owned" },
                            "reason": "context repeated on each member; summing it would count one coarse value once per member",
                        }),
                    );
                }
                continue;
            }
            kinds.insert(column.name.clone(), kind);
            let entry = combined_kinds
                .entry(column.name.clone())
                .or_insert(Some(kind));
            if *entry != Some(kind) {
                *entry = None;
            }
        }
        if members {
            for a in &output.allocations {
                kinds.insert(a.name.clone(), Numeric::Decimal(a.scale));
            }
        }
        let mut per_currency: BTreeMap<String, CurrencyTotals> = BTreeMap::new();
        for (row, (currency, matched)) in tables[&output.id].iter().zip(population) {
            let mut targets = vec![per_currency.entry(currency.clone()).or_default()];
            if !members {
                targets.push(combined.entry(currency.clone()).or_default());
            }
            for totals in targets {
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
            let mut json = currency_json(totals, &kinds)?;
            if members {
                json["excluded"] = excluded.clone().into();
                for a in &output.allocations {
                    let summary = &allocations[&output.id][a.name.as_str()];
                    // A quantity that carries a currency is totalled per currency.
                    let figures = if crate::validation::currency_amount(
                        recipe,
                        &a.quantity.source,
                        &a.quantity.field,
                    ) {
                        &summary["by_currency"][currency.as_str()]
                    } else {
                        summary
                    };
                    json["columns"][a.name.as_str()]["allocation"] = serde_json::json!({
                        "source": summary["quantity"],
                        "source_total": figures["source_total"],
                        "assigned_total": figures["assigned_total"],
                        "residual_total": figures["residual_total"],
                        "conserved": figures["conserved"],
                        "scope": "every partition of the quantity source, including members the output filter removed",
                    });
                }
            }
            currencies.insert(currency.clone(), json);
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
                    Some(("output", id)) => vec![id.split('/').next().unwrap_or(id).to_owned()],
                    Some(("population", id)) => recipe
                        .outputs
                        .iter()
                        .filter(|o| o.anchor == id)
                        .map(|o| o.id.clone())
                        .collect(),
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
    let mut phases: serde_json::Map<_, _> = progress
        .phases
        .iter()
        .map(|(k, v)| ((*k).to_string(), serde_json::json!(v)))
        .collect();
    phases.insert(
        "total".into(),
        serde_json::json!(crate::adapters::micros(started)),
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
        "phases_us": phases,
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
        "output_schema": crate::validation::output_schema(recipe),
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
    // Slice-2 sections appear only for recipes that declare them.
    if !run.populations.is_empty() {
        manifest["populations"] = serde_json::json!(run.populations);
    }
    if !run.allocations.is_empty() {
        manifest["allocations"] = serde_json::json!(run.allocations);
        manifest["evidence"]["allocation_rows"] = run.allocation_evidence.len().into();
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

/// Population and identity invariants of one output, asserted after execution
/// (decision #11): every selected anchor line appears exactly once, in both the
/// output (or its group members) and the evidence, and a line-grain output without
/// a final filter keeps every eligible line.
fn check_population(
    output: &Output,
    anchor_rows: &[Row],
    selected: &[usize],
    eligible: usize,
    evidence_rows: usize,
    output_lines: usize,
) -> Result<()> {
    let mut locations = std::collections::HashSet::with_capacity(selected.len());
    let repeated = selected
        .iter()
        .filter(|&&i| !locations.insert(anchor_rows[i].reference.location.as_str()))
        .count();
    let kept_all =
        output.filter.is_some() || !output.aggregates.is_empty() || selected.len() == eligible;
    if repeated == 0
        && evidence_rows == selected.len()
        && output_lines == selected.len()
        && eligible <= anchor_rows.len()
        && kept_all
    {
        return Ok(());
    }
    Err(Error::new(
        "POPULATION_INVARIANT",
        "execution",
        format!(
            "output {} broke its population: {} anchor records, {eligible} eligible, {} selected, {output_lines} output lines, {evidence_rows} evidence records, {repeated} repeated",
            output.id,
            anchor_rows.len(),
            selected.len()
        ),
    )
    .with_subject(format!("output:{}", output.id))
    .with_check("output.population"))
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
            // Finer records of a population's activity that match no member.
            let activity = recipe.relationships.iter().any(|r| {
                r.to == detail
                    && r.cardinality == Cardinality::OneToMany
                    && crate::validation::population(recipe, &r.from).is_some()
            });
            let (code, phase, check, message) = if activity {
                (
                    "POPULATION_UNMATCHED",
                    "population",
                    "population.coverage",
                    format!(
                        "{} {detail} records match no member of the explicit population; they are counted here and never widen it",
                        orphans.len()
                    ),
                )
            } else {
                (
                    "ORPHAN_DETAIL",
                    "source",
                    "detail.parent",
                    format!(
                        "{} {detail} detail records have no parent line; they are not output lines",
                        orphans.len()
                    ),
                )
            };
            let mut diagnostic = Error::new(code, phase, message)
                .warning()
                .with_subject(format!("source:{detail}"))
                .with_check(check)
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
/// One eligible anchor line of a grouped output.
struct Member {
    /// Canonical text of the group key values.
    key: String,
    keys: BTreeMap<String, Value>,
    matched: bool,
    /// Per measure: the line's input value, or `None` when the aggregate input
    /// predicate excluded the line from that measure.
    inputs: Vec<Option<Value>>,
    reference: SourceReference,
}
struct Grouping {
    rows: Table,
    groups: usize,
    null_inputs: BTreeMap<String, usize>,
}
/// Reduce eligible lines to one row per present group (decision #7). Groups are
/// never manufactured; measure input predicates have already been applied; the
/// final predicate selects group rows afterwards and changes no measure.
fn group(
    recipe: &Recipe,
    output: &Output,
    members: &[Member],
    policy: &Policy,
) -> Result<Grouping> {
    let mut groups: BTreeMap<&str, Vec<&Member>> = BTreeMap::new();
    for member in members {
        groups.entry(member.key.as_str()).or_default().push(member);
    }
    let mut null_inputs = BTreeMap::new();
    let mut rows = vec![];
    for lines in groups.values() {
        let mut result = lines[0].keys.clone();
        let mut skipped_nulls = false;
        for (i, aggregate) in output.aggregates.iter().enumerate() {
            let values: Vec<Value> = lines.iter().filter_map(|m| m.inputs[i].clone()).collect();
            if ["sum", "min", "max"].contains(&aggregate.function.as_str()) {
                let nulls = values.iter().filter(|v| **v == Value::Null).count();
                if nulls > 0 {
                    skipped_nulls = true;
                    *null_inputs.entry(aggregate.name.clone()).or_default() += nulls;
                }
            }
            if aggregate.context {
                check_context(output, aggregate, lines, i, &values, policy)?;
            }
            result.insert(
                aggregate.name.clone(),
                reduce(&values, &aggregate.function)?,
            );
        }
        // A measure that skipped null inputs is never labelled complete.
        let status = if lines.iter().any(|m| !m.matched) {
            "unresolved"
        } else if skipped_nulls {
            "incomplete"
        } else {
            "matched"
        };
        result.insert(
            "_status".into(),
            Value::String {
                value: status.into(),
            },
        );
        if let Some(predicate) = &output.filter {
            let bound =
                crate::validation::rewrite_output_refs(predicate, &output.id, &mut |name| {
                    Ok(Expr::Literal {
                        value: result.get(name).cloned().unwrap_or(Value::Null),
                    })
                })?;
            let keep = crate::eval::eval(&bound, recipe, &mut |source, _, _| {
                Err(Error::new(
                    "GRAIN_VIOLATION",
                    "execution",
                    format!("{source} has no single value after grouping"),
                ))
            })?;
            if keep != (Value::Boolean { value: true }) {
                continue;
            }
        }
        rows.push(result);
    }
    Ok(Grouping {
        rows,
        groups: groups.len(),
        null_inputs,
    })
}
/// A named context reduction is valid only when every non-null value of the
/// broadcast context in the group is equal.
fn check_context(
    output: &Output,
    aggregate: &GroupAggregate,
    lines: &[&Member],
    index: usize,
    values: &[Value],
    policy: &Policy,
) -> Result<()> {
    let present: Vec<&Value> = values.iter().filter(|v| **v != Value::Null).collect();
    let mut distinct: Vec<&Value> = vec![];
    for value in &present {
        if !distinct.iter().any(|d| {
            crate::eval::compare(d, value).ok().flatten() == Some(std::cmp::Ordering::Equal)
        }) {
            distinct.push(value);
        }
    }
    if distinct.len() <= 1 {
        return Ok(());
    }
    let affected: Vec<&Member> = lines
        .iter()
        .copied()
        .filter(|m| m.inputs[index].is_some())
        .collect();
    Err(Error::new(
        "CONTEXT_NOT_CONSTANT",
        "aggregate",
        format!(
            "context reduction {} of output {} found {} different values in one group of {} lines; broadcast context is not additive and has no single value here",
            aggregate.name,
            output.id,
            distinct.len(),
            affected.len()
        ),
    )
    .with_subject(format!("output:{}/aggregate:{}", output.id, aggregate.name))
    .with_check("aggregate.context")
    .with_outputs(vec![output.id.clone()])
    .with_affected(affected.len(), affected.iter().map(|m| &m.reference))
    .with_values(
        policy.diagnostic_values,
        distinct
            .iter()
            .take(Error::SAMPLE_CAP)
            .map(|v| crate::adapters::plain_value(v))
            .collect(),
    ))
}
/// Bind an output's own column references to this line's values, then resolve
/// per-line detail aggregates and features.
fn bind_line(
    expr: &Expr,
    recipe: &Recipe,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
    output: &Output,
    row_index: usize,
    values: &BTreeMap<String, Value>,
) -> Result<Expr> {
    let bound = crate::validation::rewrite_output_refs(expr, &output.id, &mut |name| {
        Ok(output
            .columns
            .iter()
            .find(|column| column.name == name)
            .map(|column| column.expr.clone())
            .unwrap_or_else(|| Expr::Literal {
                value: values.get(name).cloned().unwrap_or(Value::Null),
            }))
    })?;
    prepare(
        &bound,
        recipe,
        sources,
        matches,
        &output.anchor,
        row_index,
        &output.complete,
    )
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
    complete: &[String],
) -> Result<Expr> {
    let mut value = serde_json::to_value(expr).map_err(engine_error)?;
    fn visit(
        value: &mut serde_json::Value,
        recipe: &Recipe,
        sources: &BTreeMap<String, Vec<Row>>,
        matches: &Matches,
        anchor: &str,
        row_index: usize,
        complete: &[String],
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
                // Observations that contribute an amount, by currency where the
                // detail declares one; the anchor's own currency joins the set.
                let monetary = ["sum", "min", "max"].contains(&function.as_str())
                    && recipe
                        .sources
                        .iter()
                        .find(|s| s.id == rel.to)
                        .is_some_and(|s| s.projection.fields.contains_key("currency"));
                let mut currencies: Vec<Value> = vec![];
                if monetary
                    && recipe
                        .sources
                        .iter()
                        .find(|s| s.id == anchor)
                        .is_some_and(|s| s.projection.fields.contains_key("currency"))
                {
                    currencies.push(
                        sources[anchor][row_index]
                            .fields
                            .get("currency")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
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
                    let value = field
                        .as_ref()
                        .and_then(|f| row.fields.get(f))
                        .cloned()
                        .unwrap_or(Value::Null);
                    if monetary && value != Value::Null {
                        let currency = row.fields.get("currency").cloned().unwrap_or(Value::Null);
                        if !currencies.contains(&currency) {
                            currencies.push(currency);
                        }
                    }
                    values.push(value);
                }
                if currencies.len() > 1 {
                    return Err(Error::new(
                        "MIXED_CURRENCY",
                        "aggregate",
                        format!(
                            "{function} over {relationship} would combine amounts in {} currencies for one {anchor} record; amounts in different currencies are never summed or compared",
                            currencies.len()
                        ),
                    )
                    .with_subject(format!("relationship:{relationship}"))
                    .with_check("aggregate.currency")
                    .with_affected(1, [&sources[anchor][row_index].reference]));
                }
                // Declared-complete activity: a member with no records has
                // observed nothing, so its sum is zero rather than unknown.
                let reduced =
                    if values.is_empty() && function == "sum" && complete.contains(&relationship) {
                        zero_sum(recipe, rel, field.as_deref())
                    } else {
                        reduce(&values, &function)?
                    };
                *value =
                    serde_json::to_value(Expr::Literal { value: reduced }).map_err(engine_error)?;
                return Ok(());
            }
        }
        if value.get("op").and_then(|x| x.as_str()) == Some("feature") {
            let id = value["id"].as_str().unwrap();
            let feature = recipe.features.iter().find(|f| f.id == id).unwrap();
            *value = serde_json::to_value(&feature.expr).map_err(engine_error)?;
            return visit(value, recipe, sources, matches, anchor, row_index, complete);
        }
        match value {
            serde_json::Value::Object(map) => {
                for child in map.values_mut() {
                    visit(child, recipe, sources, matches, anchor, row_index, complete)?;
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    visit(child, recipe, sources, matches, anchor, row_index, complete)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(
        &mut value, recipe, sources, matches, anchor, row_index, complete,
    )?;
    serde_json::from_value(value).map_err(engine_error)
}

// ---------------------------------------------------------------------------
// Slice 2: explicit populations and allocation (docs/spec/slice-2.md).

/// Field name under which a target record carries its derived period. It cannot
/// collide with a projected field: projection names never contain U+001F.
const DERIVED_PERIOD: &str = "\u{1f}period";

/// Target records with their calendar month (`YYYY-MM`) added under
/// [`DERIVED_PERIOD`]. Dates are business-local: the month is read from the date
/// as recorded, with no time zone conversion. A null date has no period.
fn with_periods(rows: &[Row], date: &str) -> Vec<Row> {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            let period = match row.fields.get(date) {
                Some(Value::Date { value }) => Value::String {
                    value: value[..7].to_string(),
                },
                _ => Value::Null,
            };
            row.fields.insert(DERIVED_PERIOD.into(), period);
            row
        })
        .collect()
}

/// Allocation partitions are the quantity source's grain keys. Like any identity
/// they must be present and unique, whatever the source's declared identity.
fn check_partitions(recipe: &Recipe, sources: &BTreeMap<String, Vec<Row>>) -> Result<()> {
    for output in &recipe.outputs {
        for a in &output.allocations {
            let rows = &sources[&a.quantity.source];
            let mut seen = std::collections::HashSet::new();
            let (mut nulls, mut repeated) = (vec![], vec![]);
            for row in rows {
                match key(row, &a.partition) {
                    None => nulls.push(&row.reference),
                    Some(k) => {
                        if !seen.insert(k) {
                            repeated.push(&row.reference);
                        }
                    }
                }
            }
            for (code, references, what) in [
                ("NULL_IDENTITY", nulls, "have a null partition key"),
                (
                    "DUPLICATE_IDENTITY",
                    repeated,
                    "repeat the partition keys of an earlier record",
                ),
            ] {
                if !references.is_empty() {
                    return Err(Error::new(
                        code,
                        "data",
                        format!(
                            "{} {} records {what} {:?} of allocation {}; each partition must be exactly one record",
                            references.len(),
                            a.quantity.source,
                            a.partition,
                            a.name
                        ),
                    )
                    .with_subject(format!("source:{}", a.quantity.source))
                    .with_check("data.identity")
                    .with_affected(references.len(), references));
                }
            }
        }
    }
    Ok(())
}

/// Build every declared population: recipient records that pass the filter,
/// crossed with the declared periods. The member count is checked against
/// `limits.anchor_rows` before any member is created.
fn materialize_populations(
    recipe: &Recipe,
    sources: &mut BTreeMap<String, Vec<Row>>,
    policy: &Policy,
    cancel: &RunControl<'_>,
    progress: &mut Progress,
) -> Result<BTreeMap<String, serde_json::Value>> {
    let mut info = BTreeMap::new();
    let mut total = 0usize;
    for p in &recipe.populations {
        check(cancel)?;
        let subject = format!("population:{}", p.id);
        let recipients = &sources[&p.recipients];
        let mut eligible = vec![];
        for row in recipients {
            let keep = match &p.filter {
                None => true,
                Some(filter) => matches!(
                    crate::eval::eval(
                        filter,
                        recipe,
                        &mut |source: &str, field: &str, path: &[String]| {
                            if source != p.recipients || !path.is_empty() {
                                return Err(Error::new(
                                    "INVALID_PATH",
                                    "execution",
                                    "a population filter reads recipient fields only",
                                ));
                            }
                            Ok(row.fields.get(field).cloned().unwrap_or(Value::Null))
                        }
                    )?,
                    Value::Boolean { value: true }
                ),
            };
            if keep {
                eligible.push(row);
            }
        }
        let (periods, period_source) = match (&p.period.values, &p.period.from) {
            (Some(values), _) => (values.clone(), "listed".to_string()),
            (_, Some(from)) => {
                let mut distinct = std::collections::BTreeSet::new();
                let mut invalid = vec![];
                for row in &sources[&from.source] {
                    match row.fields.get(&from.field) {
                        Some(Value::String { value })
                            if crate::validation::calendar_month(value) =>
                        {
                            distinct.insert(value.clone());
                        }
                        _ => invalid.push(&row.reference),
                    }
                }
                if !invalid.is_empty() {
                    return Err(Error::new(
                        "INVALID_PERIOD",
                        "data",
                        format!(
                            "{} {} records hold a {} that is not a calendar month YYYY-MM; the period set of population {} is never guessed",
                            invalid.len(),
                            from.source,
                            from.field,
                            p.id
                        ),
                    )
                    .with_subject(format!("source:{}", from.source))
                    .with_check("population.periods")
                    .with_affected(invalid.len(), invalid));
                }
                (
                    distinct.into_iter().collect::<Vec<_>>(),
                    format!("source:{}/field:{}", from.source, from.field),
                )
            }
            _ => {
                return Err(Error::new(
                    "INVALID_POPULATION",
                    "execution",
                    format!("population {} declares no period set", p.id),
                ));
            }
        };
        let members = eligible
            .len()
            .checked_mul(periods.len())
            .filter(|n| *n <= policy.limits.anchor_rows)
            .ok_or_else(|| {
                Error::new(
                    "RESOURCE_LIMIT",
                    "execution",
                    format!(
                        "population {} has {} recipients × {} periods = {} members, more than limits.anchor_rows ({}); counted before any member was created",
                        p.id,
                        eligible.len(),
                        periods.len(),
                        eligible.len().saturating_mul(periods.len()),
                        policy.limits.anchor_rows
                    ),
                )
                .with_subject(subject.clone())
                .with_check("budget")
            })?;
        let mut rows = Vec::with_capacity(members);
        for recipient in &eligible {
            check(cancel)?;
            for period in &periods {
                let mut fields = recipient.fields.clone();
                fields.insert(
                    p.period.field.clone(),
                    Value::String {
                        value: period.clone(),
                    },
                );
                rows.push(Row {
                    fields,
                    reference: SourceReference {
                        source: p.id.clone(),
                        binding: recipient.reference.binding.clone(),
                        fingerprint: recipient.reference.fingerprint.clone(),
                        location: format!(
                            "{} {}={period}",
                            recipient.reference.location, p.period.field
                        ),
                    },
                    raw: BTreeMap::from([
                        (
                            "recipient".to_string(),
                            serde_json::json!(recipient.reference),
                        ),
                        (
                            "recipient_original".to_string(),
                            serde_json::json!(recipient.raw),
                        ),
                        (p.period.field.clone(), serde_json::json!(period)),
                    ]),
                });
            }
        }
        total += members;
        info.insert(
            p.id.clone(),
            serde_json::json!({
                "population": subject,
                "recipients_source": format!("source:{}", p.recipients),
                "recipient_records": recipients.len(),
                "recipient_filter": p.filter.is_some(),
                "recipients": eligible.len(),
                "period_field": p.period.field,
                "periods": periods,
                "period_source": period_source,
                "members": members,
                "limit": {"name": "anchor_rows", "value": policy.limits.anchor_rows,
                    "counted": "before materialization"},
                "inference": "none: the declared recipients crossed with the declared periods",
            }),
        );
        sources.insert(p.id.clone(), rows);
        progress.nodes.push(subject);
    }
    if !recipe.populations.is_empty() {
        progress.pass(
            "population.members",
            format!(
                "full: {total} members across {} populations, each population counted against limits.anchor_rows before materialization; no member inferred from activity",
                recipe.populations.len()
            ),
        );
    }
    Ok(info)
}

/// Value of `source.field` reached from anchor record `row` along the named
/// to-one `path`. An unresolved lookup yields null and records its outcome.
fn resolve_path(
    recipe: &Recipe,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
    outcomes: &Outcomes,
    (anchor, row): (&str, usize),
    reasons: &mut Vec<Outcome>,
    (source, field, path): (&str, &str, &[String]),
) -> Result<Value> {
    let mut current = anchor;
    let mut position = row;
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
}

/// Every member of `output`'s population as allocation `a` sees it: recipient
/// filter result, weight, partition and currency. Also returns, per member, the
/// lookups its weight read that did not resolve; they become part of a
/// recipient's row status.
fn allocation_members<'a>(
    recipe: &Recipe,
    sources: &'a BTreeMap<String, Vec<Row>>,
    matches: &Matches,
    outcomes: &Outcomes,
    output: &Output,
    a: &Allocation,
) -> Result<(Vec<crate::allocation::Member<'a>>, Vec<Vec<Outcome>>)> {
    let rows = &sources[&output.anchor];
    let identity = recipe
        .sources
        .iter()
        .find(|s| s.id == output.anchor)
        .map(|s| s.identity.clone())
        .unwrap_or_default();
    let currency = recipe
        .sources
        .iter()
        .find(|s| s.id == output.anchor)
        .is_some_and(|s| s.projection.fields.contains_key("currency"));
    let mut members = Vec::with_capacity(rows.len());
    let mut weight_reasons = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let evaluate = |expr: &Expr, reasons: &mut Vec<Outcome>| -> Result<Value> {
            let prepared = prepare(
                expr,
                recipe,
                sources,
                matches,
                &output.anchor,
                index,
                &output.complete,
            )?;
            crate::eval::eval(&prepared, recipe, &mut |s: &str, f: &str, p: &[String]| {
                resolve_path(
                    recipe,
                    sources,
                    matches,
                    outcomes,
                    (&output.anchor, index),
                    reasons,
                    (s, f, p),
                )
            })
        };
        // Recipient filter: before normalization; false and unknown exclude.
        let recipient = match &a.recipient_filter {
            None => true,
            Some(filter) => evaluate(filter, &mut vec![])? == (Value::Boolean { value: true }),
        };
        let mut reasons = vec![];
        let weight = if recipient {
            evaluate(&a.weight, &mut reasons)?
        } else {
            Value::Null
        };
        reasons.retain(|o| o.status != "matched");
        let outcome = &outcomes[&a.via][index];
        let partition = (outcome.status == "matched" && matches[&a.via][index].len() == 1)
            .then(|| matches[&a.via][index][0]);
        members.push(crate::allocation::Member {
            reference: &row.reference,
            identity: identity
                .iter()
                .map(|k| row.fields.get(k).cloned().unwrap_or(Value::Null))
                .collect(),
            recipient,
            weight,
            partition,
            via: partition.is_none().then(|| {
                outcome
                    .reason
                    .clone()
                    .unwrap_or_else(|| format!("{} {}", a.via, outcome.status))
            }),
            currency: currency.then(|| row.fields.get("currency").cloned().unwrap_or(Value::Null)),
        });
        weight_reasons.push(if recipient { reasons } else { vec![] });
    }
    Ok((members, weight_reasons))
}

/// Coverage of each activity relationship of a population output: matched and
/// unmatched finer records (unmatched ones never widen the population) and the
/// rule applied to members without records.
fn population_activity(
    recipe: &Recipe,
    output: &Output,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
) -> serde_json::Value {
    let mut activity = serde_json::Map::new();
    for rel in crate::validation::activity(recipe, output) {
        let records = &sources[&rel.to];
        let mut attached = vec![false; records.len()];
        for &child in matches[&rel.id].iter().flatten() {
            attached[child] = true;
        }
        let unmatched: Vec<&SourceReference> = attached
            .iter()
            .enumerate()
            .filter(|(_, a)| !**a)
            .map(|(i, _)| &records[i].reference)
            .collect();
        let empty = matches[&rel.id].iter().filter(|m| m.is_empty()).count();
        let (rule, statement) = crate::validation::completeness_rule(recipe, output, rel);
        activity.insert(
            rel.id.clone(),
            serde_json::json!({
                "relationship": format!("relationship:{}", rel.id),
                "source": format!("source:{}", rel.to),
                "period": rel.period.as_ref().map(|pm| serde_json::json!({"key": pm.key, "date": pm.date, "derive": "calendar_month"})),
                "records": records.len(),
                "matched_records": records.len() - unmatched.len(),
                "unmatched_records": unmatched.len(),
                "unmatched_samples": unmatched.iter().take(Error::SAMPLE_CAP).collect::<Vec<_>>(),
                "unmatched_samples_truncated": unmatched.len() > Error::SAMPLE_CAP,
                "empty_members": empty,
                "empty_member_rule": rule,
                "statement": statement,
            }),
        );
    }
    activity.into()
}

/// The sum of no records of a declared-complete activity, typed as the sum of
/// `field` would be.
fn zero_sum(recipe: &Recipe, rel: &Relationship, field: Option<&str>) -> Value {
    let data_type = recipe
        .sources
        .iter()
        .find(|s| s.id == rel.to)
        .and_then(|s| field.and_then(|f| s.projection.fields.get(f)))
        .map(|f| f.data_type.clone());
    match data_type {
        Some(Type::Decimal { scale, .. }) => Value::Decimal {
            value: crate::scalar::Decimal {
                coefficient: 0,
                precision: 38,
                scale,
            }
            .to_string(),
            precision: 38,
            scale,
        },
        _ => Value::Int64 { value: "0".into() },
    }
}
