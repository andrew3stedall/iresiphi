//! Save, reopen, rebind, validate, explain and run a recipe.
//!
//! ```text
//! cargo run --example run -- RECIPE BINDINGS POLICY WORK_DIR [SOURCE=PATH ...]
//! ```
//!
//! 1. Reopen the saved recipe RECIPE, save it again as WORK_DIR/saved.iresiphi,
//!    reopen that copy and check that the semantic hash is unchanged.
//! 2. Validate the reopened recipe without bindings (static checks), then with
//!    BINDINGS and POLICY (access and source schemas; no data checks).
//! 3. Explain it with the bindings: logical nodes, lineage, reads and budgets.
//! 4. Run it into a new bundle, WORK_DIR/bundle-1.
//! 5. If SOURCE=PATH pairs are given, rebind those sources to new files, keeping
//!    their adapter and settings, and run the unchanged recipe again into
//!    WORK_DIR/bundle-2. The saved recipe is not edited; only bindings change.
//!
//! WORK_DIR must not exist yet and must lie under an allowed output root of
//! POLICY. Each bundle is published atomically and never overwritten.
use iresiphi::{
    Bindings, ExecutionContext, Policy, Recipe, RunReport,
    validation::{explain_bound, validate, validate_bound},
};
use std::{path::Path, sync::atomic::AtomicBool};

type Failure = Box<dyn std::error::Error>;

fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        return Err("usage: run RECIPE BINDINGS POLICY WORK_DIR [SOURCE=PATH ...]".into());
    }
    let bindings: Bindings = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let policy: Policy = serde_json::from_str(&std::fs::read_to_string(&args[3])?)?;
    let work = Path::new(&args[4]);
    std::fs::create_dir(work)?;

    // 1. Save and reopen. A saved recipe holds no paths, data or secrets.
    let opened = Recipe::from_json(&std::fs::read_to_string(&args[1])?)?;
    let saved = work.join("saved.iresiphi");
    std::fs::write(&saved, opened.to_json()?)?;
    let recipe = Recipe::from_json(&std::fs::read_to_string(&saved)?)?;
    assert_eq!(
        recipe.hash()?,
        opened.hash()?,
        "reopening preserves meaning"
    );
    println!(
        "saved and reopened {} ({})",
        saved.display(),
        recipe.hash()?
    );

    // 2. Validate: static, then bound (access and schemas only).
    for (label, report) in [
        ("static", validate(&recipe)),
        ("bound", validate_bound(&recipe, &bindings, &policy, "none")),
    ] {
        let failed: Vec<_> = report
            .checks
            .iter()
            .filter(|c| c.status == "failed")
            .collect();
        println!(
            "validate ({label}): valid={} checks={} failed={}",
            report.valid,
            report.checks.len(),
            failed.len()
        );
        if !report.valid {
            return Err(format!("{label} validation failed: {:?}", failed).into());
        }
    }

    // 3. Explain with bindings. Nothing is read; estimates stay unknown.
    let explanation = explain_bound(&recipe, &bindings, &policy);
    println!(
        "explain: {} nodes, feasible={}",
        explanation["nodes"].as_array().map_or(0, Vec::len),
        explanation["bound"]["feasible"]
    );

    // 4. Run into a new bundle.
    let context = ExecutionContext::default();
    let first = run(
        &context,
        &recipe,
        &bindings,
        &policy,
        &work.join("bundle-1"),
    )?;

    // 5. Rebind the unchanged recipe and run again.
    let mut rebound = bindings.clone();
    for pair in &args[5..] {
        let (source, path) = pair
            .split_once('=')
            .ok_or("rebinding arguments are SOURCE=PATH")?;
        rebound
            .get_mut(source)
            .ok_or_else(|| format!("no binding for source {source}"))?
            .path = path.into();
    }
    if args.len() > 5 {
        let second = run(&context, &recipe, &rebound, &policy, &work.join("bundle-2"))?;
        println!(
            "rebound run: recipe hash unchanged={}",
            second.manifest["recipe_hash"] == first.manifest["recipe_hash"]
        );
    }
    Ok(())
}

fn run(
    context: &ExecutionContext,
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    bundle: &Path,
) -> Result<RunReport, Failure> {
    let report = context.run_bundle(recipe, bindings, policy, bundle, &AtomicBool::new(false));
    if report.status == "failed" || report.status == "cancelled" {
        return Err(format!("{}: {:?}", report.status, report.diagnostics).into());
    }
    let m = &report.manifest;
    println!(
        "run {}: {} into {}; {} resolved and {} unresolved lines; reconciliation {}",
        report.run_id,
        report.status,
        bundle.display(),
        m["resolved_rows"],
        m["unresolved_rows"],
        m["reconciliation"]["status"]
    );
    // Combined totals per currency and column: the full population, the
    // comparable (resolved) lines and the unresolved remainder.
    if let Some(currencies) = m["totals"]["combined"].as_object() {
        for (currency, totals) in currencies {
            for (column, t) in totals["columns"].as_object().into_iter().flatten() {
                println!(
                    "  {currency} {column}: full {}, comparable {}, unresolved {}",
                    t["population"], t["comparable"], t["unresolved"]
                );
            }
        }
    }
    for d in &report.diagnostics {
        println!(
            "  {} {} {}: {} records",
            d.severity,
            d.code,
            d.subject.as_deref().unwrap_or("-"),
            d.affected_count.map_or("unknown".into(), |n| n.to_string())
        );
    }
    Ok(report)
}
