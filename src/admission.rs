//! Run admission, resource accounting and bundle delivery (decision 13).
//!
//! The default profile is one active run per execution context, two execution
//! partitions, batches of at most `batch_rows`, and pull-based producers. Adapters
//! produce batches only when pulled and engine repartitioning is disabled, so no
//! producer queues more than [`QUEUED_BATCHES`] batches. These budgets are admission
//! choices, not a process RSS ceiling: the manifest reports tracked engine
//! reservations separately from measured peak RSS.
use crate::{
    cancel::Cancellation,
    engine::{RunReport, failed},
    model::*,
};
use datafusion::execution::memory_pool::{
    GreedyMemoryPool, MemoryPool, PeakRecordingPool, TrackConsumersPool,
};
use serde_json::json;
use std::{collections::BTreeMap, num::NonZeroUsize, sync::Arc, sync::atomic::AtomicBool};

pub(crate) const PARTITIONS: usize = 2;
pub(crate) const QUEUED_BATCHES: usize = 2;
/// Engine worker threads carry this name so their shutdown is observable.
pub(crate) const WORKER_THREAD: &str = "iresiphi-exec";

impl<T: Cancellation + ?Sized> Cancellation for &T {
    fn flag(&self) -> &AtomicBool {
        (**self).flag()
    }
    fn entered(&self, phase: crate::cancel::RunPhase) {
        (**self).entered(phase)
    }
}

pub(crate) fn engine_batch_rows(policy: &Policy) -> usize {
    policy.limits.batch_rows.max(1)
}
pub(crate) fn run_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
fn admission(policy: &Policy) -> serde_json::Value {
    json!({"active_runs_per_context":1,"partitions":PARTITIONS,"batch_rows":engine_batch_rows(policy),
        "queued_batches_per_producer":QUEUED_BATCHES,"producer_model":"pull","spill":false})
}
/// The report for an attempt refused because the context already has an active run.
pub(crate) fn busy(policy: &Policy) -> RunReport {
    let mut report = failed(
        run_id(),
        Error::new(
            "RUN_BUSY",
            "admission",
            "execution context already has an active run",
        ),
        std::time::Instant::now(),
    );
    report.manifest["limits"] = json!(policy.limits);
    report.manifest["admission"] = admission(policy);
    report
}

/// Peak resident set size of this process, where the platform reports it.
fn peak_rss() -> serde_json::Value {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                let kib = line.strip_prefix("VmHWM:")?.trim().strip_suffix("kB")?;
                kib.trim().parse::<u64>().ok()
            })
        })
        .map(|kib| {
            json!({"bytes":kib * 1024,"source":"/proc/self/status VmHWM",
                "scope":"process lifetime high-water mark; includes allocations outside run accounting"})
        })
        .unwrap_or_else(|| json!({"status":"unknown","reason":"peak RSS is not measurable here"}))
}

/// Byte count of a serialization without materializing it.
struct Counter(usize);
impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Typed schema shared by memory results and bundle tables.
/// Typed output columns shared by memory results, the manifest and `schema.json`,
/// including measure ownership and grain metadata from validation.
pub(crate) fn output_schema(recipe: &Recipe) -> serde_json::Map<String, serde_json::Value> {
    match crate::validation::output_schema(recipe) {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

/// Per-run cancellation and resource tracking passed through the executor.
pub(crate) struct RunControl<'a> {
    cancel: &'a dyn Cancellation,
    pool: Arc<PeakRecordingPool>,
    limit: usize,
}
impl<'a> RunControl<'a> {
    pub(crate) fn new(cancel: &'a dyn Cancellation, policy: &Policy) -> Self {
        let tracked: Arc<dyn MemoryPool> = Arc::new(TrackConsumersPool::new(
            GreedyMemoryPool::new(policy.limits.engine_bytes),
            NonZeroUsize::new(5).unwrap(),
        ));
        Self {
            cancel,
            pool: Arc::new(PeakRecordingPool::new(tracked)),
            limit: policy.limits.engine_bytes,
        }
    }
    pub(crate) fn cancel(&self) -> &dyn Cancellation {
        self.cancel
    }
    pub(crate) fn entered(&self, phase: crate::cancel::RunPhase) {
        self.cancel.entered(phase)
    }
    /// The run's engine pool: a hard reservation budget with no spill.
    pub(crate) fn memory_pool(&self) -> Arc<dyn MemoryPool> {
        self.pool.clone()
    }
    /// Record admission choices, budgets, tracked reservations and measured RSS.
    pub(crate) fn record(&self, report: &mut RunReport, recipe: &Recipe, policy: &Policy) {
        let manifest = &mut report.manifest;
        manifest["limits"] = json!(policy.limits);
        manifest["admission"] = admission(policy);
        manifest["engine_peak_reservation"] = json!({"peak_bytes":self.pool.peak_reserved(),
            "limit_bytes":self.limit,"spill":false,
            "scope":"tracked DataFusion pool reservations; some engine allocations are untracked"});
        manifest["peak_rss"] = peak_rss();
        let rss_measured = manifest["peak_rss"].get("bytes").is_some();
        if let Some(unknown) = manifest["unknown_measurements"].as_array_mut() {
            unknown
                .retain(|m| m != "engine_peak_reservation" && (m != "peak_rss" || !rss_measured));
        }
        if report.status != "failed" && report.status != "cancelled" {
            let mut counter = Counter(0);
            if serde_json::to_writer(
                &mut counter,
                &(
                    &report.tables,
                    &report.evidence,
                    &report.candidate_evidence,
                    &report.detail_evidence,
                ),
            )
            .is_ok()
            {
                manifest["retained_result_bytes"] = json!(counter.0);
            }
            manifest["schema"] = serde_json::Value::Object(output_schema(recipe));
        }
    }
}
impl std::ops::Deref for RunControl<'_> {
    type Target = AtomicBool;
    fn deref(&self) -> &AtomicBool {
        self.cancel.flag()
    }
}

const RECONCILIATION_COUNTS: [&str; 4] = [
    "population_rows",
    "resolved_rows",
    "unresolved_rows",
    "null_rows",
];
const RECONCILIATION_TOTALS: [&str; 3] =
    ["population_total", "comparable_total", "unresolved_total"];

/// Typed columns of the bundle's reconciliation table. Totals are exact decimal or
/// integer text, because one table holds columns of different scales.
fn reconciliation_schema() -> serde_json::Value {
    let mut columns = vec![
        json!({"name":"scope","type":{"kind":"string"}}),
        json!({"name":"currency","type":{"kind":"string"}}),
        json!({"name":"column","type":{"kind":"string"}}),
    ];
    columns.extend(
        RECONCILIATION_COUNTS
            .iter()
            .map(|n| json!({"name":n,"type":{"kind":"int64"}})),
    );
    columns.extend(
        RECONCILIATION_TOTALS
            .iter()
            .map(|n| json!({"name":n,"type":{"kind":"string"},"nullable":true})),
    );
    json!({"columns": columns})
}

/// One row per scope (an output, or `combined`), currency and numeric column: the
/// manifest's full-population, comparable and unresolved totals as a table. A null
/// total means some line's value was null, so no complete total exists.
fn reconciliation(manifest: &serde_json::Value) -> crate::engine::Table {
    let mut rows = vec![];
    let mut scopes: Vec<(&str, &serde_json::Value)> = vec![];
    if let Some(outputs) = manifest["totals"]["by_output"].as_object() {
        scopes.extend(outputs.iter().map(|(id, totals)| (id.as_str(), totals)));
    }
    scopes.push(("combined", &manifest["totals"]["combined"]));
    let text = |value: &serde_json::Value| match value {
        serde_json::Value::String(text) => Value::String {
            value: text.clone(),
        },
        _ => Value::Null,
    };
    let int = |value: &serde_json::Value| Value::Int64 {
        value: value.as_u64().unwrap_or(0).to_string(),
    };
    for (scope, currencies) in scopes {
        let Some(currencies) = currencies.as_object() else {
            continue;
        };
        for (currency, totals) in currencies {
            let Some(columns) = totals["columns"].as_object() else {
                continue;
            };
            for (column, sums) in columns {
                let mut row = BTreeMap::from([
                    (
                        "scope".to_string(),
                        Value::String {
                            value: scope.into(),
                        },
                    ),
                    (
                        "currency".to_string(),
                        Value::String {
                            value: currency.clone(),
                        },
                    ),
                    (
                        "column".to_string(),
                        Value::String {
                            value: column.clone(),
                        },
                    ),
                ]);
                let counts = &totals["rows"];
                row.insert("population_rows".into(), int(&counts["population"]));
                row.insert("resolved_rows".into(), int(&counts["resolved"]));
                row.insert("unresolved_rows".into(), int(&counts["unresolved"]));
                row.insert("null_rows".into(), int(&sums["null_rows"]));
                row.insert("population_total".into(), text(&sums["population"]));
                row.insert("comparable_total".into(), text(&sums["comparable"]));
                row.insert("unresolved_total".into(), text(&sums["unresolved"]));
                rows.push(row);
            }
        }
    }
    rows
}

/// Publish a successful run as one directory bundle; on failure the report keeps
/// no results and records the delivery outcome and cleanup evidence.
pub(crate) fn deliver(
    report: &mut RunReport,
    recipe: &Recipe,
    policy: &Policy,
    destination: &std::path::Path,
    control: &RunControl<'_>,
) {
    let delivering = std::time::Instant::now();
    let mut schema = output_schema(recipe);
    for (name, records) in [
        ("_evidence", &report.evidence),
        ("_candidate_evidence", &report.candidate_evidence),
        ("_detail_evidence", &report.detail_evidence),
    ] {
        let evidence_table = records
            .iter()
            .map(|record| {
                BTreeMap::from([(
                    "record_json".into(),
                    Value::String {
                        value: record.to_string(),
                    },
                )])
            })
            .collect();
        report.tables.insert(name.into(), evidence_table);
        schema.insert(
            name.into(),
            json!({"columns":[{"name":"record_json","type":{"kind":"string"}}]}),
        );
    }
    report
        .tables
        .insert("_reconciliation".into(), reconciliation(&report.manifest));
    schema.insert("_reconciliation".into(), reconciliation_schema());
    // A bundle cannot record its own size: bytes_written is added to the run
    // report only after commit, so it is neither present nor "unknown" inside.
    if let Some(manifest) = report.manifest.as_object_mut() {
        manifest.remove("bytes_written");
    }
    if let Some(unknown) = report.manifest["unknown_measurements"].as_array_mut() {
        unknown.retain(|m| m != "bytes_written");
    }
    report.manifest["delivery"] = json!({"kind":"directory_bundle","encoding":"csv",
        "schema":"schema.json","status":"committed","committed":true});
    match crate::sink::publish_bundle(
        destination,
        policy,
        &report.tables,
        &serde_json::Value::Object(schema),
        &report.manifest,
        control.cancel(),
    ) {
        Ok(published) => {
            // Like bytes_written, publication time is known only after commit.
            report.manifest["phases_us"]["delivery"] = json!(crate::adapters::micros(delivering));
            report.manifest["bytes_written"] = json!(published.bytes);
            report.manifest["bundle"] = json!(destination);
            report.manifest["cleanup"] = json!(published.cleanup);
            if control.load(std::sync::atomic::Ordering::Acquire) {
                report.manifest["post_commit"] = json!({"cancel_requested":true,
                    "effect":"none; the bundle was already committed"});
            }
        }
        Err(failure) => {
            let status = if failure.error.code == "CANCELLED" {
                "cancelled"
            } else {
                "failed"
            };
            report.status = status.into();
            report.tables.clear();
            report.evidence.clear();
            report.candidate_evidence.clear();
            report.detail_evidence.clear();
            report.manifest["complete"] = json!(false);
            report.manifest["delivery"]["status"] = json!(status);
            report.manifest["delivery"]["committed"] = json!(false);
            report.manifest["cleanup"] = json!(failure.cleanup);
            report.diagnostics.insert(0, failure.error);
        }
    }
}
