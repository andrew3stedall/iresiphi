//! Bounded runs (#22, decision 13): every default budget and caller override is
//! recorded in the manifest and enforced through the public run seam, admission is
//! one run per context, and tracked reservations are reported apart from measured RSS.
#[path = "support/run_fixture.rs"]
mod run_fixture;
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{CancellationToken, ExecutionContext, RunPhase, model::*};
use run_fixture::*;
use serde_json::json;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

const MIB: usize = 1 << 20;

fn go() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn default_profile_matches_the_decision_and_is_recorded_in_every_manifest() {
    let limits = Limits::default();
    let expected = json!({
        "engine_bytes": 256 * MIB, "file_bytes": 64 * MIB, "input_bytes": 256 * MIB,
        "xlsx_bytes": 256 * MIB, "worksheet_cells": 1_000_000, "field_bytes": MIB,
        "json_depth": 64, "anchor_rows": 100_000, "component_rows": 1_000_000,
        "lookup_rows": 100_000, "candidate_pairs": 1_000_000, "output_bytes": 256 * MIB,
        "retained_bytes": 256 * MIB, "batch_rows": 8192, "batch_bytes": 16 * MIB
    });
    assert_eq!(serde_json::to_value(&limits).unwrap(), expected);

    let f = fixture();
    let ok = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    assert_eq!(
        ok.status, "succeeded_with_unresolved",
        "{:?}",
        ok.diagnostics
    );
    assert_eq!(ok.manifest["limits"], expected);
    let admission = &ok.manifest["admission"];
    assert_eq!(admission["active_runs_per_context"], 1);
    assert_eq!(admission["partitions"], 2);
    assert_eq!(admission["batch_rows"], 8192);
    assert_eq!(admission["queued_batches_per_producer"], 2);
    assert_eq!(admission["spill"], false);

    // A failed run records the same admission choices it was refused under.
    let mut tiny = f.policy.clone();
    tiny.limits.anchor_rows = 2;
    let failed = ExecutionContext::default().run(&f.recipe, &f.bindings, &tiny, &go());
    assert_failed_without_results(&failed, "failed", "RESOURCE_LIMIT");
    assert_eq!(failed.manifest["limits"]["anchor_rows"], 2);
    assert_eq!(failed.manifest["admission"]["partitions"], 2);
    assert_eq!(failed.manifest["checks"]["data"], "incomplete");
}

#[test]
fn raised_limits_are_admission_choices_recorded_verbatim() {
    let mut f = fixture();
    f.policy.limits.anchor_rows = 250_000;
    f.policy.limits.engine_bytes = 512 * MIB;
    f.policy.limits.candidate_pairs = 2_000_000;
    let report = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    assert_eq!(report.status, "succeeded_with_unresolved");
    assert_eq!(
        report.manifest["limits"],
        serde_json::to_value(&f.policy.limits).unwrap()
    );
    assert_eq!(
        report.manifest["engine_peak_reservation"]["limit_bytes"],
        512 * MIB
    );
}

/// One case per budget: the smallest violating override fails the run with
/// RESOURCE_LIMIT, no partial tables, an incomplete manifest and the override recorded.
#[test]
fn each_budget_overflow_fails_the_run_with_resource_limit() {
    let base = fixture();
    let sizes = |name: &str| std::fs::metadata(base.root().join(name)).unwrap().len() as usize;
    let total = sizes("lines.json") + sizes("rates.csv") + sizes("components.csv");
    let cases: Vec<(&str, usize)> = vec![
        ("file_bytes", sizes("lines.json") - 1),
        ("input_bytes", total - 1),
        ("field_bytes", 4),
        ("json_depth", 1),
        ("anchor_rows", 2),
        ("component_rows", 2),
        ("lookup_rows", 1),
        ("candidate_pairs", 3),
        ("batch_bytes", 64),
        ("engine_bytes", 1),
    ];
    for (name, value) in cases {
        let mut policy = base.policy.clone();
        let mut limits = serde_json::to_value(&policy.limits).unwrap();
        limits[name] = json!(value);
        policy.limits = serde_json::from_value(limits).unwrap();
        let report = ExecutionContext::default().run(&base.recipe, &base.bindings, &policy, &go());
        assert_eq!(report.status, "failed", "{name}: {:?}", report.diagnostics);
        assert_eq!(
            report.diagnostics[0].code, "RESOURCE_LIMIT",
            "{name}: {:?}",
            report.diagnostics
        );
        assert!(
            report.tables.is_empty() && report.evidence.is_empty(),
            "{name}"
        );
        assert_eq!(report.manifest["complete"], false, "{name}");
        assert_eq!(report.manifest["checks"]["data"], "incomplete", "{name}");
        assert_eq!(report.manifest["limits"][name], value, "{name}");
    }
}

#[test]
fn candidate_pairs_are_capped_exactly_at_the_budget() {
    // Two qualified rate pairs plus three component pairs.
    let f = fixture();
    let mut policy = f.policy.clone();
    policy.limits.candidate_pairs = 5;
    let ok = ExecutionContext::default().run(&f.recipe, &f.bindings, &policy, &go());
    assert_eq!(
        ok.status, "succeeded_with_unresolved",
        "{:?}",
        ok.diagnostics
    );
    policy.limits.candidate_pairs = 4;
    let over = ExecutionContext::default().run(&f.recipe, &f.bindings, &policy, &go());
    assert_failed_without_results(&over, "failed", "RESOURCE_LIMIT");
}

#[test]
fn xlsx_decompressed_and_cell_budgets_fail_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sheet.xlsx");
    std::fs::write(
        &path,
        workbook_support::workbook_bytes(
            r#"<c r="B2" t="inlineStr"><is><t>1.00</t></is></c>"#,
            false,
        ),
    )
    .unwrap();
    let recipe = Recipe::from_json(&json!({
        "format_version":"0.1","name":"sheet",
        "sources":[{"id":"lines","identity":["id"],"grain":["id"],"projection":{"fields":{
            "id":{"path":"id","data_type":{"kind":"string"}},
            "amount":{"path":"amount","data_type":{"kind":"decimal","precision":12,"scale":2}}}}}],
        "outputs":[{"id":"out","anchor":"lines","columns":[
            {"name":"amount","expr":{"op":"field","source":"lines","field":"amount","path":[]}}]}]
    }).to_string()).unwrap();
    let bindings = BTreeMap::from([(
        "lines".to_owned(),
        Binding {
            path,
            adapter: "xlsx".into(),
            settings: BTreeMap::from([
                ("sheet".into(), json!("Review")),
                ("header_row".into(), json!(1)),
            ]),
        },
    )]);
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    };
    let ok = ExecutionContext::default().run(&recipe, &bindings, &policy, &go());
    assert_eq!(ok.status, "succeeded", "{:?}", ok.diagnostics);
    for (name, value) in [("xlsx_bytes", 20usize), ("worksheet_cells", 5)] {
        let mut tight = policy.clone();
        let mut limits = serde_json::to_value(&tight.limits).unwrap();
        limits[name] = json!(value);
        tight.limits = serde_json::from_value(limits).unwrap();
        let report = ExecutionContext::default().run(&recipe, &bindings, &tight, &go());
        assert_failed_without_results(&report, "failed", "RESOURCE_LIMIT");
        assert_eq!(report.manifest["limits"][name], value);
    }
}

/// `/proc` files report a size of zero, so only counting while reading can bound them.
#[test]
fn unknown_size_input_is_counted_while_reading_and_aborts() {
    let root = std::path::PathBuf::from(format!("/proc/{}", std::process::id()));
    let status = root.join("status");
    let metadata = std::fs::metadata(&status).unwrap();
    assert!(metadata.is_file() && metadata.len() == 0, "size is unknown");
    let actual = std::fs::read(&status).unwrap().len();
    assert!(actual > 256);
    let mut f = fixture();
    f.bindings.get_mut("lines").unwrap().path = status;
    f.policy.allowed_read_roots.push(root);
    f.policy.limits.file_bytes = 256;
    let report = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    assert_failed_without_results(&report, "failed", "RESOURCE_LIMIT");
    assert!(report.diagnostics[0].message.contains("file bytes"));
    // With room to read it, the same source gets past admission and fails as JSON.
    f.policy.limits.file_bytes = 64 * MIB;
    let report = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    assert_eq!(report.status, "failed");
    assert_ne!(report.diagnostics[0].code, "RESOURCE_LIMIT");
}

/// Memory results become successful only when their serialized form fits both the
/// serialized-output and the retained-buffer budgets; the boundary is exact.
#[test]
fn memory_results_must_fit_serialized_and_retained_budgets_before_success() {
    let f = fixture();
    let reference = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    let bytes = result_bytes(&reference);
    assert_eq!(reference.manifest["retained_result_bytes"], bytes);
    for name in ["output_bytes", "retained_bytes"] {
        let mut policy = f.policy.clone();
        let mut limits = serde_json::to_value(&policy.limits).unwrap();
        limits[name] = json!(bytes);
        policy.limits = serde_json::from_value(limits.clone()).unwrap();
        let fits = ExecutionContext::default().run(&f.recipe, &f.bindings, &policy, &go());
        assert_eq!(fits.status, "succeeded_with_unresolved", "{name}");
        limits[name] = json!(bytes - 1);
        policy.limits = serde_json::from_value(limits).unwrap();
        let over = ExecutionContext::default().run(&f.recipe, &f.bindings, &policy, &go());
        assert_failed_without_results(&over, "failed", "RESOURCE_LIMIT");
        assert_eq!(over.diagnostics[0].phase, "delivery", "{name}");
    }
}

/// The engine pool is a hard, non-spilling reservation budget; the manifest
/// separates its tracked peak from measured process RSS.
#[test]
fn engine_reservations_are_tracked_separately_from_measured_rss() {
    let f = fixture();
    let report = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    let engine = &report.manifest["engine_peak_reservation"];
    assert_eq!(engine["limit_bytes"], 256 * MIB);
    assert!(engine["peak_bytes"].as_u64().unwrap() > 0, "{engine}");
    assert_eq!(engine["spill"], false);
    let rss = &report.manifest["peak_rss"];
    match rss.get("bytes") {
        Some(bytes) => {
            assert!(bytes.as_u64().unwrap() > 0);
            assert_eq!(rss["source"], "/proc/self/status VmHWM");
        }
        None => assert_eq!(rss["status"], "unknown"),
    }

    let mut tight = f.policy.clone();
    tight.limits.engine_bytes = 64;
    let failed = ExecutionContext::default().run(&f.recipe, &f.bindings, &tight, &go());
    assert_failed_without_results(&failed, "failed", "RESOURCE_LIMIT");
    assert_eq!(
        failed.manifest["engine_peak_reservation"]["limit_bytes"],
        64
    );
}

/// A second run on the same context is refused while the first is active —
/// including while a bundle is still being written — and admitted afterwards.
#[test]
fn one_active_run_per_context_returns_run_busy() {
    let f = fixture();
    let context = ExecutionContext::default();
    let seen = std::sync::Mutex::new(Vec::new());
    let probe = hook(|phase, _| {
        if matches!(phase, RunPhase::Parse | RunPhase::Join | RunPhase::Write) {
            let nested = context.run(&f.recipe, &f.bindings, &f.policy, &go());
            assert_eq!(nested.diagnostics[0].phase, "admission");
            assert_eq!(nested.manifest["complete"], false);
            seen.lock()
                .unwrap()
                .push((phase, nested.status, nested.diagnostics[0].code.clone()));
        }
    });
    let report = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &f.destination(), &probe);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let seen = seen.into_inner().unwrap();
    assert!(seen.iter().any(|(p, ..)| *p == RunPhase::Parse));
    assert!(seen.iter().any(|(p, ..)| *p == RunPhase::Write));
    assert!(
        seen.iter()
            .all(|(_, status, code)| status == "failed" && code == "RUN_BUSY"),
        "{seen:?}"
    );
    // A separate context is independent, and this one is free again afterwards.
    let other = ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &go());
    assert_eq!(other.status, "succeeded_with_unresolved");
    let again = context.run(&f.recipe, &f.bindings, &f.policy, &CancellationToken::new());
    assert_eq!(again.status, "succeeded_with_unresolved");
}
