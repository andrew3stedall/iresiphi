//! Slice 7 phase 4: lookup targets over `limits.target_bytes` hold their key
//! records and read the records a batch of anchor lines reaches
//! (docs/spec/slice-7.md).
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{ExecutionContext, RunReport, model::*};
use serde_json::json;
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

/// Files of a bundle other than its manifest, which holds timings and run IDs.
fn bundle_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
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

/// Everything a run reports other than timings, run IDs and source reads.
fn same_results(report: &RunReport, expected: &RunReport) {
    assert_eq!(report.status, expected.status);
    assert_eq!(report.tables, expected.tables);
    assert_eq!(report.evidence, expected.evidence);
    assert_eq!(report.candidate_evidence, expected.candidate_evidence);
    assert_eq!(report.detail_evidence, expected.detail_evidence);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&expected.diagnostics).unwrap()
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
        assert_eq!(report.manifest[key], expected.manifest[key], "{key}");
    }
}

/// Spilled targets give the same tables, evidence and diagnostics as held ones,
/// whatever the batch size, in memory and bundle runs.
#[test]
fn spilled_targets_do_not_change_results() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = invoice::fixture(root.path());
    let context = ExecutionContext::default();
    let cancel = AtomicBool::new(false);
    let held = context.run(&recipe, &bindings, &policy, &cancel);
    assert_ne!(held.status, "failed", "{:?}", held.diagnostics);
    for id in ["enrichment", "mapping", "rates", "components"] {
        assert!(
            held.manifest["sources"][id].get("spilled").is_none(),
            "{id} is held under the default budget"
        );
    }
    let destination = root.path().join("bundle-held");
    let report = context.run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
    assert_ne!(report.status, "failed", "{:?}", report.diagnostics);
    let held_files = bundle_files(&destination);

    for batch_rows in [8192, 2, 1] {
        let mut policy = policy.clone();
        policy.limits.target_bytes = 0;
        policy.limits.batch_rows = batch_rows;
        let spilled = context.run(&recipe, &bindings, &policy, &cancel);
        same_results(&spilled, &held);
        let sources = &spilled.manifest["sources"];
        for (id, kept) in [
            ("enrichment", json!(["id", "supplier"])),
            // `item` is read by the dotted key `a_mapping.item` of the rate lookups.
            ("mapping", json!(["id", "item", "raw", "supplier"])),
            ("rates", json!(["end", "id", "item", "start", "supplier"])),
        ] {
            let spill = &sources[id]["spilled"];
            assert_eq!(spill["store"], "temporary_file", "{id}");
            assert_eq!(spill["kept_fields"], kept, "{id}");
            assert_eq!(spill["records"], sources[id]["rows"], "{id}");
            assert_eq!(spill["target_bytes"], 0, "{id}");
            assert!(spill["spilled_bytes"].as_u64().unwrap() > 0, "{id}");
        }
        assert!(
            sources["components"].get("spilled").is_none(),
            "a one-to-many detail is held"
        );
        for anchor in ["a", "b"] {
            assert!(sources[anchor].get("spilled").is_none(), "{anchor}");
        }

        let destination = root.path().join(format!("bundle-{batch_rows}"));
        let report = context.run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
        assert_ne!(report.status, "failed", "{:?}", report.diagnostics);
        assert_eq!(bundle_files(&destination), held_files, "{batch_rows}");
    }
}

/// Under the default evidence level only unresolved lines read their
/// candidates; results still match a held run.
#[test]
fn spilled_targets_match_under_the_default_evidence_level() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = invoice::fixture(root.path());
    policy.evidence = Default::default();
    let held = invoice::run(&recipe, &bindings, &policy);
    assert_ne!(held.status, "failed", "{:?}", held.diagnostics);
    policy.limits.target_bytes = 0;
    policy.limits.batch_rows = 1;
    same_results(&invoice::run(&recipe, &bindings, &policy), &held);
}

/// A target spills only once its records pass the budget: a budget above the
/// target's serialized size keeps it held.
#[test]
fn a_target_within_the_budget_is_held() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = invoice::fixture(root.path());
    policy.limits.target_bytes = 1 << 20;
    let report = invoice::run(&recipe, &bindings, &policy);
    assert_ne!(report.status, "failed", "{:?}", report.diagnostics);
    for id in ["enrichment", "mapping", "rates"] {
        assert!(
            report.manifest["sources"][id].get("spilled").is_none(),
            "{id}"
        );
    }
}

/// Eight anchor lines, each matching one target record that carries a wide
/// field only an output column reads. With `held_anchor`, another source looks
/// up the lines, so the anchor is held rather than streamed.
fn wide_target(held_anchor: bool) -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let lines: Vec<_> = (0..8).map(|i| json!({"id": i.to_string()})).collect();
    let notes: Vec<_> = (0..8)
        .map(|i| json!({"id": i.to_string(), "text": format!("{i}{}", "w".repeat(2048))}))
        .collect();
    std::fs::write(
        dir.path().join("lines.json"),
        serde_json::to_vec(&lines).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("notes.json"),
        serde_json::to_vec(&notes).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("refs.json"), r#"[{"id":"r","line":"3"}]"#).unwrap();
    let string = |p: &str| json!({"path":p,"data_type":{"kind":"string"}});
    let mut doc = json!({
        "format_version":"0.1", "name":"wide target",
        "sources":[
            {"id":"lines","identity":["id"],"grain":["id"],
                "projection":{"array":"","fields":{"id":string("/id")}}},
            {"id":"notes","identity":["id"],"grain":["id"],
                "projection":{"array":"","fields":{"id":string("/id"),"text":string("/text")}}}],
        "relationships":[{"id":"note","from":"lines","to":"notes","from_keys":["id"],
            "to_keys":["id"],"cardinality":"many_to_one"}],
        "outputs":[{"id":"review","anchor":"lines","columns":[
            {"name":"id","expr":{"op":"field","source":"lines","field":"id","path":[]}},
            {"name":"text","expr":{"op":"field","source":"notes","field":"text","path":["note"]}}
        ]}]
    });
    if held_anchor {
        doc["sources"].as_array_mut().unwrap().push(json!({"id":"refs","identity":["id"],
            "grain":["id"],"projection":{"array":"","fields":{"id":string("/id"),"line":string("/line")}}}));
        doc["relationships"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"ref","from":"refs",
            "to":"lines","from_keys":["line"],"to_keys":["id"],"cardinality":"many_to_one"}));
    }
    let recipe: Recipe = serde_json::from_value(doc).unwrap();
    let bindings = ["lines", "notes", "refs"]
        .into_iter()
        .map(|id| {
            (
                id.to_string(),
                Binding {
                    path: dir.path().join(format!("{id}.json")),
                    adapter: "json".into(),
                    settings: BTreeMap::new(),
                },
            )
        })
        .collect();
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

/// A target too large to hold under `limits.retained_bytes` runs once it
/// spills: only its key records are retained, and each line still reads the
/// wide field of the record it matched.
#[test]
fn a_target_larger_than_the_retained_budget_runs_when_spilled() {
    for held_anchor in [false, true] {
        let (dir, recipe, bindings, mut policy) = wide_target(held_anchor);
        policy.limits.retained_bytes = 8192;
        let cancel = AtomicBool::new(false);
        let context = ExecutionContext::default();
        let held = context.run(&recipe, &bindings, &policy, &cancel);
        assert_eq!(held.status, "failed");
        assert_eq!(held.diagnostics[0].code, "RESOURCE_LIMIT");

        policy.limits.target_bytes = 4096;
        policy.limits.batch_rows = 3;
        let destination = dir.path().join("bundle");
        let report = context.run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
        assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
        let sources = &report.manifest["sources"];
        assert_eq!(sources["lines"].get("streamed").is_none(), held_anchor);
        let spill = &sources["notes"]["spilled"];
        assert_eq!(spill["kept_fields"], json!(["id"]));
        assert_eq!(spill["records"], 8);
        let tables = iresiphi::sink::read_bundle(&destination).unwrap();
        let rows = &tables["review"];
        assert_eq!(rows.len(), 8);
        for row in rows {
            let (Some(Value::String { value: id }), Some(Value::String { value: text })) =
                (row.get("id"), row.get("text"))
            else {
                panic!("expected id and text in {row:?}");
            };
            assert_eq!(*text, format!("{id}{}", "w".repeat(2048)));
        }
    }
}

const ORDERS_CSV: &str = "order_no,cust\n1,A\n2,A\n3,B\n4,C\n5,Z\n";
const CUSTOMERS: &str = "CREATE TABLE customers (CustNo TEXT, SalesArea TEXT, Note TEXT);
    INSERT INTO customers VALUES ('A','North','first'),('B','North',NULL),('C','South','third'),('E','East','fifth');";

/// Orders in a CSV file look up customers in a SQLite table.
fn orders_and_customers() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let orders = dir.path().join("orders.csv");
    std::fs::write(&orders, ORDERS_CSV).unwrap();
    let crm = dir.path().join("crm.db");
    rusqlite::Connection::open(&crm)
        .unwrap()
        .execute_batch(CUSTOMERS)
        .unwrap();
    let column = |p: &str| json!({"path":p,"data_type":{"kind":"string"}});
    let recipe: Recipe = serde_json::from_value(json!({
        "format_version":"0.1", "name":"orders",
        "sources":[
            {"id":"orders","identity":["order_no"],"grain":["order_no"],"projection":{"fields":{
                "order_no":column("order_no"),"cust":column("cust")}}},
            {"id":"customers","identity":["CustNo"],"grain":["CustNo"],"projection":{"fields":{
                "CustNo":column("CustNo"),"SalesArea":column("SalesArea"),
                "Note":{"path":"Note","data_type":{"kind":"string"},"nullable":true}}}}],
        "relationships":[{"id":"customer","from":"orders","to":"customers",
            "from_keys":["cust"],"to_keys":["CustNo"],"cardinality":"many_to_one"}],
        "outputs":[{"id":"orders_out","anchor":"orders","columns":[
            {"name":"order_no","expr":{"op":"field","source":"orders","field":"order_no","path":[]}},
            {"name":"area","expr":{"op":"field","source":"customers","field":"SalesArea","path":["customer"]}},
            {"name":"note","expr":{"op":"field","source":"customers","field":"Note","path":["customer"]}}
        ]}]
    }))
    .unwrap();
    let mut bindings = Bindings::new();
    bindings.insert(
        "orders".into(),
        Binding {
            path: orders,
            adapter: "csv".into(),
            settings: BTreeMap::new(),
        },
    );
    bindings.insert(
        "customers".into(),
        Binding {
            path: crm,
            adapter: "sqlite".into(),
            settings: BTreeMap::from([("table".into(), json!("customers"))]),
        },
    );
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Placement::Local,
        transfers: Transfers::None,
        evidence: Evidence::Full,
    };
    (dir, recipe, bindings, policy)
}

/// A SQLite target over the budget is queried again at its snapshot by rowid
/// when placement allows work at the source, and spilled to a temporary file
/// under `local`. Both match the held run.
#[test]
fn a_sqlite_target_is_queried_at_its_source_unless_placement_is_local() {
    let (_dir, recipe, bindings, policy) = orders_and_customers();
    let cancel = AtomicBool::new(false);
    let context = ExecutionContext::default();
    let held = context.run(&recipe, &bindings, &policy, &cancel);
    assert_ne!(held.status, "failed", "{:?}", held.diagnostics);
    for (placement, store) in [
        (Placement::Local, "temporary_file"),
        (Placement::PreferSource, "source"),
    ] {
        for batch_rows in [8192, 2] {
            let mut policy = policy.clone();
            policy.placement = placement;
            policy.limits.target_bytes = 0;
            policy.limits.batch_rows = batch_rows;
            let report = context.run(&recipe, &bindings, &policy, &cancel);
            same_results(&report, &held);
            let spill = &report.manifest["sources"]["customers"]["spilled"];
            assert_eq!(spill["store"], store, "{placement:?}");
            assert_eq!(spill["kept_fields"], json!(["CustNo"]), "{placement:?}");
            assert_eq!(spill["records"], 4, "{placement:?}");
        }
    }
}
