//! Run a saved recipe with separately supplied bindings and policy.
use iresiphi::{Bindings, ExecutionContext, Policy, Recipe};
use std::{path::Path, sync::atomic::AtomicBool};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: run RECIPE BINDINGS POLICY NEW_BUNDLE".into());
    }
    let recipe = Recipe::from_json(&std::fs::read_to_string(&args[1])?)?;
    let bindings: Bindings = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let policy: Policy = serde_json::from_str(&std::fs::read_to_string(&args[3])?)?;
    let report = ExecutionContext::default().run_bundle(
        &recipe,
        &bindings,
        &policy,
        Path::new(&args[4]),
        &AtomicBool::new(false),
    );
    println!("{}", serde_json::to_string_pretty(&report.manifest)?);
    if report.status == "failed" || report.status == "cancelled" {
        return Err(format!("{:?}", report.diagnostics).into());
    }
    Ok(())
}
