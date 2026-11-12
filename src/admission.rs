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
pub(crate) fn output_schema(recipe: &Recipe) -> serde_json::Map<String, serde_json::Value> {
    let mut schema = serde_json::Map::new();
    for output in &recipe.outputs {
        let mut columns: Vec<_> = output
            .columns
            .iter()
            .filter(|c| output.aggregates.is_empty() || output.group_by.contains(&c.name))
            .map(|c| json!({"name":c.name,"type":column_type(&c.expr, recipe)}))
            .collect();
        for aggregate in &output.aggregates {
            columns.push(
                json!({"name":aggregate.name,"type":reduced_type(&aggregate.function, &aggregate.expr, recipe)}),
            );
        }
        columns.push(json!({"name":"_status","type":{"kind":"string"}}));
        schema.insert(output.id.clone(), json!({"columns":columns}));
    }
    schema
}
fn reduced_type(function: &str, expr: &Expr, recipe: &Recipe) -> Option<Type> {
    let ty = crate::validation::expression_type(expr, recipe)
        .ok()
        .flatten();
    if function.starts_with("count_") {
        Some(Type::Int64)
    } else if let (true, Some(Type::Decimal { scale, .. })) = (function == "sum", &ty) {
        Some(Type::Decimal {
            precision: 38,
            scale: *scale,
        })
    } else {
        ty
    }
}
fn column_type(expr: &Expr, recipe: &Recipe) -> Option<Type> {
    crate::validation::expression_type(expr, recipe)
        .ok()
        .flatten()
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
        if report.status != "failed" && report.status != "cancelled" {
            let mut counter = Counter(0);
            if serde_json::to_writer(&mut counter, &(&report.tables, &report.evidence)).is_ok() {
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

/// Publish a successful run as one directory bundle; on failure the report keeps
/// no results and records the delivery outcome and cleanup evidence.
pub(crate) fn deliver(
    report: &mut RunReport,
    recipe: &Recipe,
    policy: &Policy,
    destination: &std::path::Path,
    control: &RunControl<'_>,
) {
    let mut schema = output_schema(recipe);
    let evidence_table = report
        .evidence
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
    report.tables.insert("_evidence".into(), evidence_table);
    schema.insert(
        "_evidence".into(),
        json!({"columns":[{"name":"record_json","type":{"kind":"string"}}]}),
    );
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
            report.manifest["complete"] = json!(false);
            report.manifest["delivery"]["status"] = json!(status);
            report.manifest["delivery"]["committed"] = json!(false);
            report.manifest["cleanup"] = json!(failure.cleanup);
            report.diagnostics.insert(0, failure.error);
        }
    }
}
