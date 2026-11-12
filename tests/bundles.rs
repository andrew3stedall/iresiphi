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
