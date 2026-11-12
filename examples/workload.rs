//! Measure one complete bundle run, for docs/measurements/workload-100k.md.
//!
//! ```text
//! cargo run --release --example workload -- RECIPE BINDINGS POLICY NEW_BUNDLE
//! ```
//!
//! Prints one JSON object: the process's resident memory before the run, the
//! wall time of reopening the recipe and of `run_bundle`, the process high-water
//! mark after delivery, the run status and diagnostics summary, and the full run
//! manifest (phase timings, counts, bytes, tracked reservations). Peak RSS is a
//! process-wide figure read from /proc/self/status; it is not a per-run bound.
//!
//! With `IRESIPHI_CANCEL_AFTER_MS=N` a second thread requests cancellation N ms
//! after the run starts, and the output adds how long the run took to return
//! after that request. Cancellation is cooperative, so this is an observation of
//! one run, not a latency bound.
use iresiphi::{Bindings, ExecutionContext, Policy, Recipe};
use serde_json::json;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// A `/proc/self/status` field in bytes, where the platform reports it.
fn status_bytes(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let kib = line.strip_prefix(field)?.trim().strip_suffix("kB")?;
        kib.trim().parse::<u64>().ok().map(|k| k * 1024)
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: workload RECIPE BINDINGS POLICY NEW_BUNDLE".into());
    }
    let opening = Instant::now();
    let recipe = Recipe::from_json(&std::fs::read_to_string(&args[1])?)?;
    let bindings: Bindings = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let policy: Policy = serde_json::from_str(&std::fs::read_to_string(&args[3])?)?;
    let open_us = opening.elapsed().as_micros() as u64;
    let rss_before = status_bytes("VmRSS:");
    let cancel_after = std::env::var("IRESIPHI_CANCEL_AFTER_MS")
        .ok()
        .map(|ms| ms.parse::<u64>())
        .transpose()?;
    let cancel = AtomicBool::new(false);
    let requested: Mutex<Option<Instant>> = Mutex::new(None);
    let finished = AtomicBool::new(false);
    let running = Instant::now();
    let (report, returned) = std::thread::scope(|scope| {
        if let Some(ms) = cancel_after {
            let deadline = running + Duration::from_millis(ms);
            let (finished, requested, cancel) = (&finished, &requested, &cancel);
            scope.spawn(move || {
                while Instant::now() < deadline && !finished.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                if !finished.load(Ordering::Acquire) {
                    *requested.lock().unwrap() = Some(Instant::now());
                    cancel.store(true, Ordering::Release);
                }
            });
        }
        let report = ExecutionContext::default().run_bundle(
            &recipe,
            &bindings,
            &policy,
            Path::new(&args[4]),
            &cancel,
        );
        let returned = Instant::now();
        finished.store(true, Ordering::Release);
        (report, returned)
    });
    let run_us = (returned - running).as_micros() as u64;
    let cancel_latency_us = requested
        .lock()
        .unwrap()
        .map(|at| (returned - at).as_micros() as u64);
    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| json!({"code":d.code,"severity":d.severity,"subject":d.subject,"affected_count":d.affected_count}))
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": report.status,
            "open_recipe_us": open_us,
            "run_bundle_wall_us": run_us,
            "cancel_after_ms": cancel_after,
            "cancel_to_return_us": cancel_latency_us,
            "rss_before_run_bytes": rss_before,
            "peak_rss_after_delivery_bytes": status_bytes("VmHWM:"),
            "diagnostics": diagnostics,
            "manifest": report.manifest,
        }))?
    );
    if report.status == "failed" || (report.status == "cancelled" && cancel_after.is_none()) {
        return Err("run did not succeed".into());
    }
    Ok(())
}
