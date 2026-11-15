//! Slice 7 phase 3: streamed anchors (docs/spec/slice-7.md).
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{ExecutionContext, model::*};
use serde_json::json;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

/// Files of a bundle other than its manifest, which holds timings and run IDs.
fn bundle_files(dir: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name() != "manifest.json")
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

/// An anchor read only as an anchor keeps its key records; lookup targets and
/// detail sources are held. Results do not depend on the batch size.
#[test]
fn anchors_stream_and_results_do_not_depend_on_the_batch_size() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = invoice::fixture(root.path());
    let context = ExecutionContext::default();
    let cancel = AtomicBool::new(false);
    let mut bundles = vec![];
    let mut reports = vec![];
    for batch_rows in [8192, 2, 1] {
        let mut policy = policy.clone();
        policy.limits.batch_rows = batch_rows;
        let destination = root.path().join(format!("bundle-{batch_rows}"));
        let report = context.run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
        assert_ne!(report.status, "failed", "{:?}", report.diagnostics);
        let sources = &report.manifest["sources"];
        for anchor in ["a", "b"] {
            let streamed = &sources[anchor]["streamed"];
            assert_eq!(
                streamed["kept_fields"],
                json!(["date", "id", "invoice", "raw", "supplier"]),
                "{anchor}: identity, undotted from keys and the validity date"
            );
            assert_eq!(streamed["reads"], 2, "{anchor}");
            assert_eq!(streamed["batch_rows"], batch_rows, "{anchor}");
        }
        for held in ["enrichment", "mapping", "rates", "components"] {
            assert!(sources[held].get("streamed").is_none(), "{held} is held");
        }
        bundles.push(bundle_files(&destination));
        reports.push(report);
    }
    for (report, files) in reports.iter().zip(&bundles).skip(1) {
        assert_eq!(*files, bundles[0]);
        assert_eq!(report.status, reports[0].status);
        assert_eq!(
            serde_json::to_value(&report.diagnostics).unwrap(),
            serde_json::to_value(&reports[0].diagnostics).unwrap()
        );
        for key in [
            "counts",
            "totals",
            "evidence",
            "resolved_rows",
            "unresolved_rows",
            "retained_result_bytes",
            "check_results",
        ] {
            assert_eq!(report.manifest[key], reports[0].manifest[key], "{key}");
        }
    }
    // A memory run streams its anchors too and returns the same tables.
    let collected = context.run(&recipe, &bindings, &policy, &cancel);
    let tables = iresiphi::sink::read_bundle(&root.path().join("bundle-8192")).unwrap();
    for output in ["a_review", "b_review"] {
        assert_eq!(tables[output], collected.tables[output], "{output}");
    }
}

/// One anchor of eight records, each carrying a wide field no lookup reads.
fn wide_anchor() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let records: Vec<_> = (0..8)
        .map(|i| json!({"id": i.to_string(), "text": "w".repeat(2048)}))
        .collect();
    let input = dir.path().join("lines.json");
    std::fs::write(&input, serde_json::to_vec(&records).unwrap()).unwrap();
    let string = |p: &str| json!({"path":p,"data_type":{"kind":"string"}});
    let recipe: Recipe = serde_json::from_value(json!({
        "format_version":"0.1", "name":"wide anchor",
        "sources":[{"id":"lines","identity":["id"],"grain":["id"],
            "projection":{"array":"","fields":{"id":string("/id"),"text":string("/text")}}}],
        "outputs":[{"id":"review","anchor":"lines","columns":[
            {"name":"id","expr":{"op":"field","source":"lines","field":"id","path":[]}}
        ]}]
    }))
    .unwrap();
    let bindings = BTreeMap::from([(
        "lines".to_string(),
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
        evidence: Default::default(),
    };
    (dir, recipe, bindings, policy)
}

/// `limits.retained_bytes` counts the key records a streamed anchor keeps, not
/// its decoded records. Holding the same source exceeds the same limit.
#[test]
fn retained_bytes_count_the_key_records_of_a_streamed_anchor() {
    let (dir, recipe, bindings, mut policy) = wide_anchor();
    policy.limits.retained_bytes = 8192;
    let cancel = AtomicBool::new(false);
    let held = iresiphi::adapters::read_sources(&recipe, &bindings, &policy, &cancel);
    let error = held.expect_err("eight wide records exceed 8 KiB when held");
    assert_eq!(error.code, "RESOURCE_LIMIT");

    let destination = dir.path().join("bundle");
    let report =
        ExecutionContext::default().run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    assert_eq!(
        report.manifest["sources"]["lines"]["streamed"]["kept_fields"],
        json!(["id"])
    );
    assert_eq!(report.manifest["counts"]["review"]["output_rows"], 8);
    let tables = iresiphi::sink::read_bundle(&destination).unwrap();
    assert_eq!(tables["review"].len(), 8);
}

/// Every record of a streamed anchor is still checked on its first read: a
/// duplicated identity fails before any output is produced.
#[test]
fn a_streamed_anchor_is_checked_in_full_before_output() {
    let (dir, recipe, bindings, policy) = wide_anchor();
    let records = json!([{"id":"1","text":"a"},{"id":"2","text":"b"},{"id":"1","text":"c"}]);
    std::fs::write(&bindings["lines"].path, records.to_string()).unwrap();
    let destination = dir.path().join("bundle");
    let report = ExecutionContext::default().run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(report.status, "failed");
    let error = &report.diagnostics[0];
    assert_eq!(error.code, "DUPLICATE_IDENTITY");
    assert_eq!(error.affected_count, Some(2));
    assert_eq!(error.samples.len(), 2);
    assert_eq!(error.samples[0].location, "/0");
    assert_eq!(error.samples[1].location, "/2");
    assert!(!destination.exists());
    assert!(
        report.manifest.get("delivery").is_none(),
        "no staging began"
    );
}
