//! Slice 7 phases 4 and 5: relationship targets over `limits.target_bytes`,
//! lookup targets and one-to-many details alike, hold their key records and
//! read the records a batch of anchor lines reaches (docs/spec/slice-7.md).
#[path = "support/customer_month.rs"]
mod customer_month;
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{ExecutionContext, RunReport, model::*};
use serde_json::{Value as Json, json};
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

/// Spilled targets and details give the same tables, evidence and diagnostics
/// as held ones, whatever the batch size, in memory and bundle runs.
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
            // A one-to-many detail keeps its identity and the keys its parents match.
            ("components", json!(["id", "invoice", "line", "supplier"])),
        ] {
            let spill = &sources[id]["spilled"];
            assert_eq!(spill["store"], "temporary_file", "{id}");
            assert_eq!(spill["kept_fields"], kept, "{id}");
            assert_eq!(spill["records"], sources[id]["rows"], "{id}");
            assert_eq!(spill["target_bytes"], 0, "{id}");
            assert!(spill["spilled_bytes"].as_u64().unwrap() > 0, "{id}");
        }
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

// --- Phase 5: one-to-many details and anchored targets -------------------------

fn named(name: &str, expr: Json) -> Json {
    json!({"name": name, "expr": expr})
}
fn field(source: &str, name: &str) -> Json {
    json!({"op": "field", "source": source, "field": name})
}
/// A per-line aggregate over a one-to-many detail relationship.
fn aggregate(relationship: &str, function: &str, input: Option<&str>) -> Json {
    json!({"op": "aggregate", "relationship": relationship, "function": function, "field": input})
}

fn local_policy(dir: &Path) -> Policy {
    Policy {
        allowed_read_roots: vec![dir.into()],
        allowed_output_roots: vec![dir.into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Placement::Local,
        transfers: Transfers::None,
        evidence: Evidence::Full,
    }
}

const LEDGER_LINES: &str = "id,currency,month,billed\nL1,AUD,2026-01,10\nL2,AUD,2026-01,10\nL3,AUD,2026-03,110\nL4,USD,2026-03,5\n";
const LEDGER_COMPONENTS: &str =
    "id,line,amount\nk1,L1,1\nk2,L1,2\nk3,L1,3\nk4,L2,4\nk5,L3,50\nk6,L3,\\N\nk7,L9,30\n";
const LEDGER_PARTS: &str =
    "id,component,note\np1,k1,first\np2,k1,second\np3,k5,third\np4,k9,orphan\n";

/// Lines with a one-to-many component detail, whose components have a
/// one-to-many part detail. Lines are only an anchor, so they stream. k6 has
/// no amount; k7 and p4 have no parent.
fn ledger() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let mut bindings = Bindings::new();
    for (id, content) in [
        ("lines", LEDGER_LINES),
        ("components", LEDGER_COMPONENTS),
        ("parts", LEDGER_PARTS),
    ] {
        let path = dir.path().join(format!("{id}.csv"));
        std::fs::write(&path, content).unwrap();
        bindings.insert(
            id.into(),
            Binding {
                path,
                adapter: "csv".into(),
                settings: BTreeMap::new(),
            },
        );
    }
    let string = |p: &str| json!({"path": p, "data_type": {"kind": "string"}});
    let money = json!({"kind": "decimal", "precision": 18, "scale": 2});
    let recipe: Recipe = serde_json::from_value(json!({
        "format_version": "0.1", "name": "ledger",
        "sources": [
            {"id": "lines", "identity": ["id"], "grain": ["id"], "projection": {"fields": {
                "id": string("id"), "currency": string("currency"), "month": string("month"),
                "billed": {"path": "billed", "data_type": money}}}},
            {"id": "components", "identity": ["id"], "grain": ["id"], "projection": {"fields": {
                "id": string("id"), "line": string("line"),
                "amount": {"path": "amount", "data_type": money, "nullable": true}}}},
            {"id": "parts", "identity": ["id"], "grain": ["id"], "projection": {"fields": {
                "id": string("id"), "component": string("component"), "note": string("note")}}}],
        "relationships": [
            {"id": "line_components", "from": "lines", "to": "components", "from_keys": ["id"],
                "to_keys": ["line"], "cardinality": "one_to_many"},
            {"id": "component_parts", "from": "components", "to": "parts", "from_keys": ["id"],
                "to_keys": ["component"], "cardinality": "one_to_many"}],
        "outputs": [
            {"id": "review", "anchor": "lines", "columns": [
                named("id", field("lines", "id")),
                named("currency", field("lines", "currency")),
                named("billed", field("lines", "billed")),
                named("components", aggregate("line_components", "count_records", None)),
                named("component_total", aggregate("line_components", "sum", Some("amount")))]},
            {"id": "by_month", "anchor": "lines", "group_by": ["currency", "month"],
                "columns": [
                    named("currency", field("lines", "currency")),
                    named("month", field("lines", "month")),
                    named("component_total", aggregate("line_components", "sum", Some("amount")))],
                "aggregates": [
                    {"name": "billed", "function": "sum", "expr": field("lines", "billed")},
                    {"name": "component_amount", "function": "sum",
                        "expr": field("by_month", "component_total")}]},
            {"id": "components_detail", "anchor": "components", "detail": true, "columns": [
                named("id", field("components", "id")),
                named("amount", field("components", "amount")),
                named("parts", aggregate("component_parts", "count_records", None))]},
            {"id": "parts_detail", "anchor": "parts", "detail": true, "columns": [
                named("id", field("parts", "id")),
                named("note", field("parts", "note"))]}]
    }))
    .unwrap();
    let policy = local_policy(dir.path());
    (dir, recipe, bindings, policy)
}

/// A spilled detail gives the same per-line aggregates, grouped measures,
/// detail outputs, parent references and orphan diagnostics as a held one,
/// whatever the batch size, including a detail whose parent is a spilled
/// detail and is itself read in batches by the detail output anchored at it.
#[test]
fn spilled_details_do_not_change_aggregates_or_detail_outputs() {
    let (dir, recipe, bindings, policy) = ledger();
    let context = ExecutionContext::default();
    let cancel = AtomicBool::new(false);
    let held = context.run(&recipe, &bindings, &policy, &cancel);
    assert!(
        held.status.starts_with("succeeded"),
        "{} {:?}",
        held.status,
        held.diagnostics
    );
    let row = |table: &str, id: &str| {
        held.tables[table]
            .iter()
            .find(|r| r.get("id").map(invoice::text) == Some(id))
            .unwrap_or_else(|| panic!("no {id} in {table}"))
            .clone()
    };
    let l1 = row("review", "L1");
    assert_eq!(invoice::text(&l1["components"]), "3");
    assert_eq!(invoice::text(&l1["component_total"]), "6.00");
    assert_eq!(invoice::text(&row("components_detail", "k1")["parts"]), "2");
    assert_eq!(held.tables["components_detail"].len(), 7);
    assert_eq!(held.tables["parts_detail"].len(), 4);
    assert_eq!(
        held.diagnostics
            .iter()
            .filter(|d| d.code == "ORPHAN_DETAIL")
            .count(),
        2,
        "k7 and p4: {:?}",
        held.diagnostics
    );
    for id in ["components", "parts"] {
        assert!(
            held.manifest["sources"][id].get("spilled").is_none(),
            "{id}"
        );
    }
    let destination = dir.path().join("bundle-held");
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
        assert!(
            sources["lines"].get("streamed").is_some(),
            "lines are only an anchor"
        );
        for (id, kept) in [
            // Components key their parts on `id`, which is also the identity.
            ("components", json!(["id", "line"])),
            ("parts", json!(["component", "id"])),
        ] {
            let spill = &sources[id]["spilled"];
            assert_eq!(spill["store"], "temporary_file", "{id}");
            assert_eq!(spill["kept_fields"], kept, "{id}");
            assert_eq!(spill["records"], sources[id]["rows"], "{id}");
        }
        let destination = dir.path().join(format!("bundle-{batch_rows}"));
        let report = context.run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
        assert_ne!(report.status, "failed", "{:?}", report.diagnostics);
        assert_eq!(bundle_files(&destination), held_files, "{batch_rows}");
    }

    // Under the default evidence level only unresolved lines record evidence.
    let mut policy = policy.clone();
    policy.evidence = Default::default();
    let held = context.run(&recipe, &bindings, &policy, &cancel);
    policy.limits.target_bytes = 0;
    policy.limits.batch_rows = 1;
    same_results(&context.run(&recipe, &bindings, &policy, &cancel), &held);
}

const ORDER_LINES: &str = "CREATE TABLE order_lines (LineNo TEXT, OrderNo TEXT, Qty INTEGER);
    INSERT INTO order_lines VALUES ('l1','1',2),('l2','1',3),('l3','3',1),('l4','9',5);";

/// The orders also have a one-to-many detail of order lines in the customers'
/// database: each order counts its lines and sums their quantity, and a detail
/// output is anchored at the lines. Line l4 has no order.
fn orders_with_lines() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let (dir, mut recipe, mut bindings, policy) = orders_and_customers();
    let crm = dir.path().join("crm.db");
    rusqlite::Connection::open(&crm)
        .unwrap()
        .execute_batch(ORDER_LINES)
        .unwrap();
    let column = |p: &str| json!({"path": p, "data_type": {"kind": "string"}});
    recipe.sources.push(
        serde_json::from_value(json!({"id": "order_lines", "identity": ["LineNo"],
            "grain": ["LineNo"], "projection": {"fields": {
                "LineNo": column("LineNo"), "OrderNo": column("OrderNo"),
                "Qty": {"path": "Qty", "data_type": {"kind": "int64"}}}}}))
        .unwrap(),
    );
    recipe.relationships.push(
        serde_json::from_value(json!({"id": "items", "from": "orders", "to": "order_lines",
            "from_keys": ["order_no"], "to_keys": ["OrderNo"], "cardinality": "one_to_many"}))
        .unwrap(),
    );
    for (name, expr) in [
        ("items", aggregate("items", "count_records", None)),
        ("qty", aggregate("items", "sum", Some("Qty"))),
    ] {
        recipe.outputs[0]
            .columns
            .push(serde_json::from_value(named(name, expr)).unwrap());
    }
    recipe.outputs.push(
        serde_json::from_value(json!({"id": "lines_out", "anchor": "order_lines",
            "detail": true, "columns": [
                named("line", field("order_lines", "LineNo")),
                named("qty", field("order_lines", "Qty"))]}))
        .unwrap(),
    );
    bindings.insert(
        "order_lines".into(),
        Binding {
            path: crm,
            adapter: "sqlite".into(),
            settings: BTreeMap::from([("table".into(), json!("order_lines"))]),
        },
    );
    (dir, recipe, bindings, policy)
}

/// A SQLite detail over the budget is queried again at its snapshot by rowid
/// when placement allows work at the source, and spilled to a temporary file
/// under `local`. Both match the held run, including the detail output
/// anchored at it.
#[test]
fn a_sqlite_detail_is_queried_at_its_source_unless_placement_is_local() {
    let (_dir, recipe, bindings, policy) = orders_with_lines();
    let cancel = AtomicBool::new(false);
    let context = ExecutionContext::default();
    let held = context.run(&recipe, &bindings, &policy, &cancel);
    assert_ne!(held.status, "failed", "{:?}", held.diagnostics);
    assert_eq!(held.tables["lines_out"].len(), 4);
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
            let spill = &report.manifest["sources"]["order_lines"]["spilled"];
            assert_eq!(spill["store"], store, "{placement:?}");
            assert_eq!(
                spill["kept_fields"],
                json!(["LineNo", "OrderNo"]),
                "{placement:?}"
            );
            assert_eq!(spill["records"], 4, "{placement:?}");
        }
    }
}

/// Eight lines, each with one note that carries a wide field only a detail
/// output reads.
fn wide_detail() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let lines: Vec<Json> = (0..8).map(|i| json!({"id": i.to_string()})).collect();
    let notes: Vec<Json> = (0..8)
        .map(|i| {
            json!({"id": format!("n{i}"), "line": i.to_string(),
                "text": format!("{i}{}", "w".repeat(2048))})
        })
        .collect();
    let mut bindings = Bindings::new();
    for (id, records) in [("lines", lines), ("notes", notes)] {
        let path = dir.path().join(format!("{id}.json"));
        std::fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
        bindings.insert(
            id.into(),
            Binding {
                path,
                adapter: "json".into(),
                settings: BTreeMap::new(),
            },
        );
    }
    let string = |p: &str| json!({"path": p, "data_type": {"kind": "string"}});
    let recipe: Recipe = serde_json::from_value(json!({
        "format_version": "0.1", "name": "wide detail",
        "sources": [
            {"id": "lines", "identity": ["id"], "grain": ["id"],
                "projection": {"array": "", "fields": {"id": string("/id")}}},
            {"id": "notes", "identity": ["id"], "grain": ["id"],
                "projection": {"array": "", "fields": {
                    "id": string("/id"), "line": string("/line"), "text": string("/text")}}}],
        "relationships": [{"id": "line_notes", "from": "lines", "to": "notes",
            "from_keys": ["id"], "to_keys": ["line"], "cardinality": "one_to_many"}],
        "outputs": [
            {"id": "review", "anchor": "lines", "columns": [
                named("id", field("lines", "id")),
                named("notes", aggregate("line_notes", "count_records", None))]},
            {"id": "notes_out", "anchor": "notes", "detail": true, "columns": [
                named("id", field("notes", "id")),
                named("text", field("notes", "text"))]}]
    }))
    .unwrap();
    let mut policy = local_policy(dir.path());
    policy.evidence = Default::default();
    (dir, recipe, bindings, policy)
}

/// A detail too large to hold under `limits.retained_bytes` runs once it
/// spills: only its key records are retained, and the detail output anchored
/// at it still reads every wide field.
#[test]
fn a_detail_larger_than_the_retained_budget_runs_when_spilled() {
    let (dir, recipe, bindings, mut policy) = wide_detail();
    policy.limits.retained_bytes = 8192;
    let cancel = AtomicBool::new(false);
    let context = ExecutionContext::default();
    let held = context.run_bundle(
        &recipe,
        &bindings,
        &policy,
        &dir.path().join("held"),
        &cancel,
    );
    assert_eq!(held.status, "failed");
    assert_eq!(held.diagnostics[0].code, "RESOURCE_LIMIT");

    policy.limits.target_bytes = 4096;
    policy.limits.batch_rows = 3;
    let destination = dir.path().join("bundle");
    let report = context.run_bundle(&recipe, &bindings, &policy, &destination, &cancel);
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    let spill = &report.manifest["sources"]["notes"]["spilled"];
    assert_eq!(spill["kept_fields"], json!(["id", "line"]));
    assert_eq!(spill["records"], 8);
    let tables = iresiphi::sink::read_bundle(&destination).unwrap();
    let notes = &tables["notes_out"];
    assert_eq!(notes.len(), 8);
    for row in notes {
        let (Some(id), Some(text)) = (row.get("id"), row.get("text")) else {
            panic!("expected id and text in {row:?}");
        };
        let id = invoice::text(id);
        assert_eq!(
            invoice::text(text),
            format!("{}{}", &id[1..], "w".repeat(2048))
        );
    }
    assert_eq!(tables["review"].len(), 8);
    for row in &tables["review"] {
        assert_eq!(invoice::text(&row["notes"]), "1", "{row:?}");
    }
}

/// A lookup target that an output also anchors at spills as well: that output
/// reads its own records in batches, and another output still reaches them,
/// and the targets beyond them, through lookups.
#[test]
fn an_anchored_lookup_target_spills_and_matches() {
    let (_dir, mut recipe, bindings, mut policy) = wide_target(true);
    recipe.outputs.push(
        serde_json::from_value(json!({"id": "refs_out", "anchor": "refs", "columns": [
            named("id", field("refs", "id")),
            named("text", json!({"op": "field", "source": "notes", "field": "text",
                "path": ["ref", "note"]}))]}))
        .unwrap(),
    );
    policy.evidence = Evidence::Full;
    let cancel = AtomicBool::new(false);
    let context = ExecutionContext::default();
    let held = context.run(&recipe, &bindings, &policy, &cancel);
    assert_eq!(held.status, "succeeded", "{:?}", held.diagnostics);
    assert_eq!(
        invoice::text(&held.tables["refs_out"][0]["text"]),
        format!("3{}", "w".repeat(2048))
    );
    for batch_rows in [8192, 2, 1] {
        let mut policy = policy.clone();
        policy.limits.target_bytes = 0;
        policy.limits.batch_rows = batch_rows;
        let report = context.run(&recipe, &bindings, &policy, &cancel);
        same_results(&report, &held);
        let sources = &report.manifest["sources"];
        for id in ["lines", "notes"] {
            assert_eq!(sources[id]["spilled"]["kept_fields"], json!(["id"]), "{id}");
        }
        assert!(sources["refs"].get("streamed").is_some());
    }
}

/// Sources a population or an allocation reads stay held whatever the budget:
/// members are built and allocated outside anchor batches.
#[test]
fn population_and_allocation_sources_stay_held() {
    let fixture = customer_month::fixture();
    let held = fixture.succeeded();
    let mut policy = fixture.policy.clone();
    policy.limits.target_bytes = 0;
    policy.limits.batch_rows = 1;
    let report = ExecutionContext::default().run(
        &fixture.recipe(),
        &fixture.bindings,
        &policy,
        &AtomicBool::new(false),
    );
    same_results(&report, &held);
    assert_eq!(report.allocation_evidence, held.allocation_evidence);
    for id in ["customers", "transactions", "targets"] {
        assert!(
            report.manifest["sources"][id].get("spilled").is_none(),
            "{id}"
        );
    }
}
