//! Deferred source access and explicit execution through the public interface.
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{ExecutionContext, Pipeline};
use std::sync::atomic::AtomicBool;

#[test]
fn planning_does_not_open_sources_and_execution_observes_current_data() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = invoice::fixture(root.path());
    let path = bindings["a"].path.clone();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let pipeline = Pipeline::new(recipe.clone(), bindings.clone(), policy.clone());
    assert_eq!(pipeline.explain()["bound"]["source_access"], "not-run");
    let context = ExecutionContext::default();
    let cancel = AtomicBool::new(false);
    let missing = pipeline.collect(&context, &cancel);
    assert_eq!(missing.status, "failed");
    std::fs::write(&path, bytes).unwrap();
    let collected = pipeline.collect(&context, &cancel);
    let direct = context.run(&recipe, &bindings, &policy, &cancel);
    assert_eq!(collected.status, direct.status);
    assert_eq!(
        serde_json::to_value(collected.tables).unwrap(),
        serde_json::to_value(direct.tables).unwrap()
    );
    let bundled = pipeline.run_bundle(&context, &root.path().join("bundle"), &cancel);
    assert_eq!(bundled.status, direct.status);
    assert!(root.path().join("bundle/manifest.json").is_file());
}

#[test]
fn streamed_joins_preserve_rows_and_evidence_across_batch_sizes() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = invoice::fixture(root.path());
    policy.evidence = iresiphi::model::Evidence::Full;
    let context = ExecutionContext::default();
    let cancel = AtomicBool::new(false);
    let expected = context.run(&recipe, &bindings, &policy, &cancel);
    assert_ne!(expected.status, "failed", "{:?}", expected.diagnostics);
    policy.limits.batch_rows = 1;
    let actual = Pipeline::new(recipe, bindings, policy).collect(&context, &cancel);
    assert_eq!(actual.status, expected.status);
    assert_eq!(
        serde_json::to_value(actual.tables).unwrap(),
        serde_json::to_value(expected.tables).unwrap()
    );
    assert_eq!(actual.evidence, expected.evidence);
    assert_eq!(actual.candidate_evidence, expected.candidate_evidence);
    assert_eq!(actual.detail_evidence, expected.detail_evidence);
    assert_eq!(
        actual.manifest["retained_result_bytes"],
        expected.manifest["retained_result_bytes"]
    );
}
