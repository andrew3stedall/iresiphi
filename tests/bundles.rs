use iresiphi::{ExecutionContext, model::*};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn fixture() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.json");
    std::fs::write(&input, br#"[{"id":"a"}]"#).unwrap();
    let recipe: Recipe = serde_json::from_value(serde_json::json!({
        "format_version":"0.1", "name":"bundle test",
        "sources":[{"id":"lines","identity":["id"],"grain":["id"],"projection":{"array":"","fields":{"id":{"path":"/id","data_type":{"kind":"string"}}}}}],
        "outputs":[{"id":"review","anchor":"lines","columns":[
            {"name":"empty","expr":{"op":"literal","value":{"kind":"string","value":""}}},
            {"name":"null","expr":{"op":"literal","value":{"kind":"null"}}},
            {"name":"slash","expr":{"op":"literal","value":{"kind":"string","value":"\\N"}}},
            {"name":"quoted","expr":{"op":"literal","value":{"kind":"string","value":"a,\"b\"\nc"}}}
        ]}]
    })).unwrap();
    let bindings = BTreeMap::from([(
        "lines".into(),
        Binding {
            path: input,
            adapter: "json".into(),
            settings: BTreeMap::new(),
        },
    )]);
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
        transfers: Default::default(),
        evidence: iresiphi::model::Evidence::Full,
    };
    (dir, recipe, bindings, policy)
}
#[test]
fn bundle_roundtrips_csv_null_empty_backslash_and_quoting_and_refuses_retry() {
    let (dir, recipe, bindings, policy) = fixture();
    let destination = dir.path().join("bundle");
    let context = ExecutionContext::default();
    let run = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(run.status, "succeeded", "{:?}", run.diagnostics);
    let schema: serde_json::Value =
        serde_json::from_slice(&std::fs::read(destination.join("schema.json")).unwrap()).unwrap();
    let filename = schema["csv"]["files"]["review"].as_str().unwrap();
    let mut reader = csv::Reader::from_path(destination.join(filename)).unwrap();
    let headers = reader.headers().unwrap().clone();
    let record = reader.records().next().unwrap().unwrap();
    let values: BTreeMap<_, _> = headers.iter().zip(record.iter()).collect();
    assert_eq!(values["empty"], "");
    assert_eq!(values["null"], "\\N");
    assert_eq!(values["slash"], "\\\\N");
    assert_eq!(values["quoted"], "a,\"b\"\nc");
    let before = std::fs::read(destination.join("manifest.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&before).unwrap()["complete"],
        true
    );
    let retry = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(retry.status, "failed");
    assert_eq!(retry.diagnostics[0].code, "DESTINATION_EXISTS");
    assert_eq!(
        std::fs::read(destination.join("manifest.json")).unwrap(),
        before
    );
}
#[test]
fn cancelled_and_over_budget_runs_publish_nothing_and_leave_no_staging() {
    let (dir, recipe, bindings, mut policy) = fixture();
    let context = ExecutionContext::default();
    let destination = dir.path().join("bundle");
    let cancelled = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(true),
    );
    assert_eq!(cancelled.status, "cancelled");
    assert!(!destination.exists());
    policy.limits.output_bytes = 1;
    let failed = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(failed.status, "failed");
    assert!(!destination.exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".iresiphi-")
    }));
}
#[test]
fn bundle_rejects_symlink_parent_and_outside_roots() {
    let (dir, recipe, bindings, policy) = fixture();
    let other = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(other.path(), dir.path().join("link")).unwrap();
    let context = ExecutionContext::default();
    for destination in [dir.path().join("link/bundle"), other.path().join("bundle")] {
        let report = context.run_bundle(
            &recipe,
            &bindings,
            &policy,
            &destination,
            &AtomicBool::new(false),
        );
        assert_eq!(report.status, "failed");
        assert!(!destination.exists());
    }
}

/// Records which run phases were entered.
#[derive(Default)]
struct Phases {
    flag: AtomicBool,
    seen: std::sync::Mutex<Vec<iresiphi::RunPhase>>,
}
impl iresiphi::Cancellation for Phases {
    fn flag(&self) -> &AtomicBool {
        &self.flag
    }
    fn entered(&self, phase: iresiphi::RunPhase) {
        self.seen.lock().unwrap().push(phase);
    }
}

/// The fixture with one wide column, so results are much larger than the source.
fn wide() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let (dir, mut recipe, bindings, policy) = fixture();
    let mut declaration = serde_json::to_value(&recipe).unwrap();
    declaration["outputs"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name":"wide","expr":{"op":"literal","value":{"kind":"string","value":"w".repeat(4096)}}}));
    recipe = serde_json::from_value(declaration).unwrap();
    (dir, recipe, bindings, policy)
}

/// Slice 7 phase 2: a bundle run writes rows as it produces them and keeps none, so
/// `limits.retained_bytes` does not bound its results; `limits.output_bytes` does.
#[test]
fn bundle_runs_retain_no_results() {
    let (dir, recipe, bindings, mut policy) = wide();
    let context = ExecutionContext::default();
    let memory = context.run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(memory.status, "succeeded", "{:?}", memory.diagnostics);
    let bytes = memory.manifest["retained_result_bytes"].as_u64().unwrap() as usize;
    assert!(bytes > 4096, "{bytes}");

    policy.limits.retained_bytes = bytes - 1;
    let refused = context.run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(refused.status, "failed");
    assert_eq!(refused.diagnostics[0].code, "RESOURCE_LIMIT");
    assert_eq!(refused.diagnostics[0].phase, "delivery");

    let destination = dir.path().join("bundle");
    let bundled = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(bundled.status, "succeeded", "{:?}", bundled.diagnostics);
    assert!(bundled.tables.is_empty() && bundled.evidence.is_empty());
    // Counts, totals and the measured result size match the memory run exactly.
    for key in [
        "retained_result_bytes",
        "counts",
        "totals",
        "evidence",
        "resolved_rows",
    ] {
        assert_eq!(bundled.manifest[key], memory.manifest[key], "{key}");
    }
    let tables = iresiphi::sink::read_bundle(&destination).unwrap();
    assert_eq!(tables["review"], memory.tables["review"]);
    assert_eq!(tables["_evidence"].len(), memory.evidence.len());

    policy.limits.output_bytes = bytes - 1;
    let over = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &dir.path().join("over"),
        &AtomicBool::new(false),
    );
    assert_eq!(over.status, "failed");
    assert_eq!(over.diagnostics[0].code, "RESOURCE_LIMIT");
    assert_eq!(over.manifest["delivery"]["committed"], false);
    assert!(!dir.path().join("over").exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".iresiphi-")
    }));
}

/// The destination is approved before any source is read: an existing destination
/// fails the run without opening a source, even one that does not exist.
#[test]
fn an_existing_destination_is_refused_before_any_source_is_read() {
    let (dir, recipe, mut bindings, policy) = fixture();
    bindings.get_mut("lines").unwrap().path = dir.path().join("missing.json");
    let destination = dir.path().join("bundle");
    std::fs::create_dir(&destination).unwrap();
    let phases = Phases::default();
    let report =
        ExecutionContext::default().run_bundle(&recipe, &bindings, &policy, &destination, &phases);
    assert_eq!(report.status, "failed");
    assert_eq!(report.diagnostics[0].code, "DESTINATION_EXISTS");
    assert_eq!(report.diagnostics[0].phase, "delivery");
    assert_eq!(report.manifest["complete"], false);
    assert_eq!(report.manifest["delivery"]["committed"], false);
    assert!(
        phases.seen.lock().unwrap().is_empty(),
        "no phase was entered"
    );
}

/// A run that fails after output started removes its staged rows and says so.
#[test]
fn a_run_failing_while_writing_removes_its_staging() {
    let (dir, recipe, bindings, mut policy) = wide();
    policy.limits.output_bytes = 4096;
    let phases = Phases::default();
    let destination = dir.path().join("bundle");
    let report =
        ExecutionContext::default().run_bundle(&recipe, &bindings, &policy, &destination, &phases);
    assert_eq!(report.status, "failed");
    assert_eq!(report.diagnostics[0].code, "RESOURCE_LIMIT");
    assert_eq!(
        *phases.seen.lock().unwrap(),
        vec![iresiphi::RunPhase::Parse, iresiphi::RunPhase::Write]
    );
    assert_eq!(report.manifest["delivery"]["status"], "failed");
    assert!(report.manifest["cleanup"]["staging"].is_string());
    assert_eq!(
        report.manifest["cleanup"]["leftovers"],
        serde_json::json!([])
    );
    assert!(!destination.exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".iresiphi-")
    }));
}
