//! Slice 5: a lookup target in SQLite reduced to keys the executor transfers
//! from a source elsewhere, through the public validate/explain/run seam.
//! Contract: docs/spec/slice-5.md.
use iresiphi::{
    ExecutionContext, RunReport,
    model::*,
    validation::{explain_bound, validate_bound},
};
use serde_json::{Value as Json, json};
use std::{path::Path, sync::atomic::AtomicBool};

const ORDERS_CSV: &str = "order_no,cust,booked,net,open\n1,A,2026-01-05,60.00,true\n2,A,2026-01-20,40.00,false\n3,B,2026-01-11,200.00,true\n4,C,2026-02-08,120.50,true\n5,Z,2026-02-09,10.00,true\n6,B,2025-12-31,75.25,true\n7,D,2025-11-02,30.00,true\n";
const ORDERS_DDL: &str =
    "CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER)";
const ORDERS: &str = "INSERT INTO orders VALUES
    (1,'A','2026-01-05','60.00',1),
    (2,'A','2026-01-20','40.00',0),
    (3,'B','2026-01-11','200.00',1),
    (4,'C','2026-02-08','120.50',1),
    (5,'Z','2026-02-09','10.00',1),
    (6,'B','2025-12-31','75.25',1),
    (7,'D','2025-11-02','30.00',1)";
const CUSTOMERS_DDL: &str = "CREATE TABLE customers (CustNo TEXT, SalesArea TEXT, Since TEXT)";
const CUSTOMERS: &str = "INSERT INTO customers VALUES
    ('A','North','2020-01-01'),
    ('B','North',NULL),
    ('C','South','2021-06-30'),
    ('D','West','2022-02-02'),
    ('E','East','2023-03-03')";

fn column(path: &str, kind: &str) -> Json {
    json!({"path":path,"data_type":{"kind":kind}})
}
fn nullable(path: &str, kind: &str) -> Json {
    json!({"path":path,"data_type":{"kind":kind},"nullable":true})
}
fn field(name: &str) -> Json {
    json!({"op":"field","source":"orders","field":name})
}
fn named(name: &str, expr: Json) -> Json {
    json!({"name":name,"expr":expr})
}

fn recipe_doc() -> Json {
    json!({
        "format_version":"0.1",
        "name":"open orders",
        "sources":[
            {"id":"orders","identity":["order_no"],"grain":["order_no"],"projection":{"fields":{
                "order_no":column("order_no","int64"),
                "cust":nullable("cust","string"),
                "booked":nullable("booked","date"),
                "net":{"path":"net","data_type":{"kind":"decimal","precision":18,"scale":2}},
                "open":column("open","boolean")}}},
            {"id":"customers","identity":["CustNo"],"grain":["CustNo"],"projection":{"fields":{
                "CustNo":column("CustNo","string"),
                "SalesArea":column("SalesArea","string"),
                "Since":nullable("Since","date")}}},
        ],
        "relationships":[{"id":"customer","from":"orders","to":"customers",
            "from_keys":["cust"],"to_keys":["CustNo"],"cardinality":"many_to_one"}],
        "outputs":[{"id":"open_orders","anchor":"orders",
            "eligibility":{"op":"and","left":field("open"),"right":
                {"op":"less_equal","left":{"op":"literal","value":{"kind":"date","value":"2026-01-01"}},"right":field("booked")}},
            "columns":[
                named("order_no", field("order_no")),
                named("currency", json!({"op":"literal","value":{"kind":"string","value":"AUD"}})),
                named("net", field("net")),
                named("area", json!({"op":"field","source":"customers","field":"SalesArea","path":["customer"]})),
            ]}],
    })
}

struct Fixture {
    dir: tempfile::TempDir,
    doc: Json,
    bindings: Bindings,
    policy: Policy,
}

fn database(path: &Path, statements: &[&str]) {
    let conn = rusqlite::Connection::open(path).unwrap();
    for statement in statements {
        conn.execute_batch(statement).unwrap();
    }
}

fn binding(path: &Path, adapter: &str, settings: Json) -> Binding {
    Binding {
        path: path.into(),
        adapter: adapter.into(),
        settings: serde_json::from_value(settings).unwrap(),
    }
}

/// Orders in a CSV file, customers in a SQLite table built by `customers`.
fn fixture_with(customers: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let orders = dir.path().join("orders.csv");
    std::fs::write(&orders, ORDERS_CSV).unwrap();
    let crm = dir.path().join("crm.db");
    database(&crm, customers);
    let mut bindings = Bindings::new();
    bindings.insert("orders".into(), binding(&orders, "csv", json!({})));
    bindings.insert(
        "customers".into(),
        binding(&crm, "sqlite", json!({"table":"customers"})),
    );
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Placement::Local,
        transfers: Transfers::None,
    };
    Fixture {
        dir,
        doc: recipe_doc(),
        bindings,
        policy,
    }
}

fn fixture() -> Fixture {
    fixture_with(&[CUSTOMERS_DDL, CUSTOMERS])
}

impl Fixture {
    fn recipe(&self) -> Recipe {
        serde_json::from_value(self.doc.clone()).unwrap()
    }
    fn placed(mut self, placement: Placement) -> Self {
        self.policy.placement = placement;
        self
    }
    fn keys(mut self) -> Self {
        self.policy.transfers = Transfers::Keys;
        self
    }
    /// Orders in a SQLite table of their own file, built by `statements`.
    fn orders_in_sqlite(mut self, statements: &[&str]) -> Self {
        let path = self.dir.path().join("orders.db");
        database(&path, statements);
        self.bindings.insert(
            "orders".into(),
            binding(&path, "sqlite", json!({"table":"orders"})),
        );
        self
    }
    /// The same files under another policy. Binding identities hash the path,
    /// so runs are comparable only within one directory; `self` must outlive
    /// the copy, which owns an unused directory of its own.
    fn under(&self, placement: Placement, transfers: Transfers) -> Fixture {
        let mut policy = self.policy.clone();
        policy.placement = placement;
        policy.transfers = transfers;
        Fixture {
            dir: tempfile::tempdir().unwrap(),
            doc: self.doc.clone(),
            bindings: self.bindings.clone(),
            policy,
        }
    }
    fn run(&self) -> RunReport {
        ExecutionContext::default().run(
            &self.recipe(),
            &self.bindings,
            &self.policy,
            &AtomicBool::new(false),
        )
    }
    fn succeeded(&self) -> RunReport {
        let report = self.run();
        assert!(
            report.status.starts_with("succeeded"),
            "{} {:?}",
            report.status,
            report.diagnostics
        );
        report
    }
    fn failure(&self) -> Error {
        let report = self.run();
        assert_eq!(report.status, "failed", "{:?}", report.diagnostics);
        assert!(report.tables.is_empty(), "failed runs publish no tables");
        report.diagnostics[0].clone()
    }
    fn explain(&self) -> Json {
        explain_bound(&self.recipe(), &self.bindings, &self.policy)
    }
}

fn text(v: &Value) -> Option<&str> {
    match v {
        Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Int64 { value }
        | Value::Date { value } => Some(value),
        Value::Null => None,
        other => panic!("expected scalar text, got {other:?}"),
    }
}

/// `(order_no, area, status)` of every output row, in output order.
fn rows(report: &RunReport) -> Vec<(String, Option<String>, String)> {
    report.tables["open_orders"]
        .iter()
        .map(|r| {
            (
                text(&r["order_no"]).unwrap().to_string(),
                text(&r["area"]).map(str::to_string),
                text(&r["_status"]).unwrap().to_string(),
            )
        })
        .collect()
}

/// Everything a transfer must leave unchanged.
fn assert_same_results(a: &RunReport, b: &RunReport) {
    assert_eq!(a.status, b.status);
    assert_eq!(a.tables, b.tables);
    assert_eq!(
        serde_json::to_value(&a.evidence).unwrap(),
        serde_json::to_value(&b.evidence).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&a.candidate_evidence).unwrap(),
        serde_json::to_value(&b.candidate_evidence).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&a.diagnostics).unwrap(),
        serde_json::to_value(&b.diagnostics).unwrap()
    );
    assert_eq!(a.manifest["counts"], b.manifest["counts"]);
}

fn decision(explanation: &Json) -> Json {
    explanation["bound"]["placement"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["operation"] == "relationship:customer/target")
        .cloned()
        .unwrap_or(Json::Null)
}

#[test]
fn local_placement_reads_every_customer() {
    let report = fixture().succeeded();
    let expected: Vec<(String, Option<String>, String)> = [
        ("1", Some("North"), "matched"),
        ("3", Some("North"), "matched"),
        ("4", Some("South"), "matched"),
        ("5", None, "unmatched"),
    ]
    .into_iter()
    .map(|(o, a, s)| (o.into(), a.map(str::to_string), s.into()))
    .collect();
    assert_eq!(rows(&report), expected);
    let customers = &report.manifest["sources"]["customers"];
    assert_eq!(customers["rows"], 5);
    assert_eq!(customers["scanned_rows"], Json::Null);
    assert_eq!(customers["placement"], Json::Null);
}

#[test]
fn transferred_keys_reduce_the_target_and_change_no_result() {
    let f = fixture();
    let local = f.succeeded();
    for placement in [Placement::PreferSource, Placement::RequireSource] {
        let reduced = f.under(placement, Transfers::Keys).succeeded();
        assert_same_results(&reduced, &local);
        let customers = &reduced.manifest["sources"]["customers"];
        assert_eq!(customers["rows"], 5, "{customers}");
        // Every order is delivered from the CSV file: keys A, B, C, D and Z.
        assert_eq!(customers["scanned_rows"], 4, "{customers}");
        let placed = &customers["placement"];
        assert_eq!(placed["scan"], "source");
        assert_eq!(placed["operation"], "relationship:customer/target");
        assert_eq!(placed["reduction"], "transferred_keys");
        assert_eq!(placed["transfer"]["id"], "relationship:customer/keys");
        assert_eq!(placed["transfer"]["from"], "source:orders");
        assert_eq!(placed["transfer"]["keys"], 5);
        assert_eq!(placed["transfer"]["key_values"], 5);
        assert!(
            placed["checks"]["text_formats"]
                .as_str()
                .unwrap()
                .contains("scanned records only")
        );
        let orders = &reduced.manifest["sources"]["orders"];
        assert_eq!(orders["rows"], 7);
        assert_eq!(orders["placement"], Json::Null);
        // The manifest carries counts, never the transferred values.
        assert!(!placed["transfer"].to_string().contains("\"Z\""));
    }
}

#[test]
fn a_pushed_eligibility_narrows_the_transferred_keys() {
    let f = fixture().orders_in_sqlite(&[ORDERS_DDL, ORDERS]);
    let local = f.succeeded();
    let reduced = f
        .under(Placement::RequireSource, Transfers::Keys)
        .succeeded();
    assert_eq!(reduced.tables, local.tables);
    assert_eq!(reduced.manifest["counts"], local.manifest["counts"]);
    let orders = &reduced.manifest["sources"]["orders"];
    assert_eq!(orders["scanned_rows"], 4);
    assert_eq!(orders["shared_read"], Json::Null, "different files");
    let customers = &reduced.manifest["sources"]["customers"];
    // Eligible orders carry A, B, C and Z; three of them are customers.
    assert_eq!(customers["placement"]["transfer"]["keys"], 4);
    assert_eq!(customers["scanned_rows"], 3);
}

#[test]
fn without_permission_nothing_is_transferred_and_explain_says_why() {
    let base = fixture();
    let local = base.succeeded();
    for placement in [Placement::PreferSource, Placement::RequireSource] {
        let f = base.under(placement, Transfers::None);
        let report = f.succeeded();
        assert_same_results(&report, &local);
        let customers = &report.manifest["sources"]["customers"];
        assert_eq!(customers["scanned_rows"], Json::Null);
        assert_eq!(customers["placement"], Json::Null);
        let explanation = f.explain();
        assert_eq!(explanation["bound"]["feasible"], true);
        assert_eq!(explanation["bound"]["transfers"], json!([]));
        assert_eq!(explanation["bound"]["placement"]["transfers"], "none");
        let d = decision(&explanation);
        assert_eq!(d["placement"], "local", "{d}");
        let reason = d["reason"].as_str().unwrap();
        assert!(reason.contains("permits no transfers"), "{reason}");
        assert!(reason.contains("transfers: keys"), "{reason}");
    }
}

#[test]
fn local_placement_transfers_nothing_even_when_permitted() {
    let f = fixture().keys();
    let report = f.succeeded();
    assert_eq!(
        report.manifest["sources"]["customers"]["scanned_rows"],
        Json::Null
    );
    let explanation = f.explain();
    assert_eq!(explanation["bound"]["transfers"], json!([]));
    assert_eq!(decision(&explanation)["placement"], "local");
}

#[test]
fn explain_lists_the_transfer_without_opening_a_source() {
    let f = fixture().placed(Placement::PreferSource).keys();
    for b in f.bindings.values() {
        std::fs::remove_file(&b.path).unwrap();
    }
    let bound = &f.explain()["bound"];
    assert_eq!(bound["feasible"], true, "{bound}");
    assert_eq!(bound["placement"]["transfers"], "keys");
    let transfers = bound["transfers"].as_array().unwrap();
    assert_eq!(transfers.len(), 1, "{bound}");
    let t = &transfers[0];
    assert_eq!(t["transfer"], "relationship:customer/keys");
    assert_eq!(t["from"], "source:orders");
    assert_eq!(t["to"], "source:customers");
    assert_eq!(t["operation"], "relationship:customer/target");
    assert_eq!(t["fields"], json!(["cust"]));
    assert_eq!(t["limit"]["transfer_keys"], 10_000);
    assert!(
        t["mechanism"]
            .as_str()
            .unwrap()
            .contains("nothing is written")
    );
    let d = decision(&f.explain());
    assert_eq!(d["placement"], "source");
    assert_eq!(d["reduction"], "transferred_keys");
    assert_eq!(d["transfer"], "relationship:customer/keys");
    let read = bound["reads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["source"] == "source:customers")
        .unwrap();
    assert_eq!(
        read["scan"],
        "reduced at the source to keys transferred from `orders`"
    );
    assert!(
        read["capabilities"]["pushdown_operations"]
            .as_array()
            .unwrap()
            .contains(&json!("relationship_target_key_transfer"))
    );
    assert_eq!(read["capabilities"]["transfers_accepted"], json!(["keys"]));
}

#[test]
fn a_transfer_above_the_key_limit_stays_local_or_is_infeasible() {
    let base = fixture();
    let local = base.succeeded();
    let mut f = base.under(Placement::PreferSource, Transfers::Keys);
    f.policy.limits.transfer_keys = 4;
    let report = f.succeeded();
    assert_same_results(&report, &local);
    let placed = &report.manifest["sources"]["customers"]["placement"];
    assert_eq!(placed["scan"], "local", "{placed}");
    assert!(
        placed["reason"]
            .as_str()
            .unwrap()
            .contains("limits.transfer_keys (4)")
    );
    assert_eq!(placed["transfer"], Json::Null);
    assert_eq!(report.manifest["sources"]["customers"]["scanned_rows"], 5);

    f.policy.limits.transfer_keys = 5;
    assert_eq!(
        f.succeeded().manifest["sources"]["customers"]["placement"]["scan"],
        "source"
    );

    f.policy.limits.transfer_keys = 4;
    f.policy.placement = Placement::RequireSource;
    let error = f.failure();
    assert_eq!(error.code, "PLACEMENT_INFEASIBLE");
    assert_eq!(error.phase, "source");
    assert_eq!(
        error.subject.as_deref(),
        Some("relationship:customer/target")
    );
    assert!(
        error.message.contains("limits.transfer_keys"),
        "{}",
        error.message
    );
    // The values themselves never appear in the diagnostic.
    assert!(!serde_json::to_string(&error).unwrap().contains("North"));
}

#[test]
fn a_target_with_another_use_is_never_reduced() {
    let mut f = fixture().placed(Placement::PreferSource).keys();
    f.doc["outputs"].as_array_mut().unwrap().push(json!({
    "id":"customer_list","anchor":"customers","columns":[
        named("cust", json!({"op":"field","source":"customers","field":"CustNo"})),
        named("currency", json!({"op":"literal","value":{"kind":"string","value":"AUD"}})),
    ]}));
    let report = f.succeeded();
    assert_eq!(report.tables["customer_list"].len(), 5);
    assert_eq!(
        report.manifest["sources"]["customers"]["scanned_rows"],
        Json::Null
    );
    let d = decision(&f.explain());
    assert_eq!(d["placement"], "local");
    assert!(d["reason"].as_str().unwrap().contains("another use"));

    // Required and permitted, but impossible: refused before any source opens.
    f.policy.placement = Placement::RequireSource;
    for b in f.bindings.values() {
        std::fs::remove_file(&b.path).unwrap();
    }
    let error = f.failure();
    assert_eq!(error.code, "PLACEMENT_INFEASIBLE");
    assert_eq!(error.phase, "planning");
    assert_eq!(
        error.subject.as_deref(),
        Some("relationship:customer/target")
    );
    let bound = &f.explain()["bound"];
    assert_eq!(bound["feasible"], false);
    assert_eq!(bound["diagnostic"]["code"], "PLACEMENT_INFEASIBLE");
    assert_eq!(bound["transfers"], json!([]));
}

#[test]
fn an_unscanned_invalid_date_is_a_disclosed_difference() {
    let bad = "UPDATE customers SET Since = '2023-02-30' WHERE CustNo = 'E'";
    let local = fixture_with(&[CUSTOMERS_DDL, CUSTOMERS, bad]).failure();
    assert!(
        ["INVALID_VALUE", "INVALID_DATE"].contains(&local.code.as_str()),
        "{local:?}"
    );
    assert_eq!(local.samples[0].location, "rowid:5");
    let reduced = fixture_with(&[CUSTOMERS_DDL, CUSTOMERS, bad])
        .placed(Placement::PreferSource)
        .keys()
        .succeeded();
    let checks = serde_json::to_string(&reduced.manifest["check_results"]).unwrap();
    assert!(checks.contains("scanned records only"), "{checks}");
}

#[test]
fn a_failed_source_check_reads_the_target_locally_with_the_local_diagnostic() {
    // A BLOB sales area on an unreferenced customer: the whole-table check
    // fails, so the scan is local and the diagnostic is the local one.
    let bad = "UPDATE customers SET SalesArea = x'00' WHERE CustNo = 'E'";
    let f = fixture_with(&[CUSTOMERS_DDL, CUSTOMERS, bad]);
    let local = f.failure();
    for placement in [Placement::PreferSource, Placement::RequireSource] {
        let reduced = f.under(placement, Transfers::Keys).failure();
        assert_eq!(reduced.code, local.code);
        assert_eq!(
            serde_json::to_value(&reduced.samples).unwrap(),
            serde_json::to_value(&local.samples).unwrap()
        );
    }
}

#[test]
fn duplicate_target_keys_stay_ambiguous_and_null_keys_match_nothing() {
    // A second A and a null key; customers are identified by a separate Id.
    let customers = [
        "CREATE TABLE customers (CustNo TEXT, SalesArea TEXT, Since TEXT, Id INTEGER)",
        "INSERT INTO customers VALUES ('A','North',NULL,1), ('B','North',NULL,2), ('C','South',NULL,3),
            ('D','West',NULL,4), ('E','East',NULL,5), ('A','Central',NULL,6), (NULL,'Nowhere',NULL,7)",
    ];
    let orders = [
        ORDERS_DDL,
        ORDERS,
        "INSERT INTO orders VALUES (8,NULL,'2026-03-01','1.00',1), (9,'a','2026-03-02','2.00',1), (10,'','2026-03-03','3.00',1)",
    ];
    let mut f = fixture_with(&customers).orders_in_sqlite(&orders);
    f.doc["sources"][1]["identity"] = json!(["Id"]);
    f.doc["sources"][1]["grain"] = json!(["Id"]);
    f.doc["sources"][1]["projection"]["fields"]["Id"] = column("Id", "int64");
    f.doc["sources"][1]["projection"]["fields"]["CustNo"] = nullable("CustNo", "string");
    let build = |placement: Placement, transfers: Transfers| f.under(placement, transfers);
    let local = build(Placement::Local, Transfers::None).succeeded();
    // Compare under the same pushed eligibility, with and without the transfer.
    let filtered = build(Placement::PreferSource, Transfers::None).succeeded();
    let reduced = build(Placement::PreferSource, Transfers::Keys).succeeded();
    assert_same_results(&reduced, &filtered);
    assert_eq!(reduced.tables, local.tables);
    let by_order: std::collections::BTreeMap<_, _> = rows(&reduced)
        .into_iter()
        .map(|(o, a, s)| (o, (a, s)))
        .collect();
    assert_eq!(by_order["1"].1, "ambiguous");
    assert_eq!(by_order["8"], (None, "unmatched".to_string()));
    assert_eq!(
        by_order["9"],
        (None, "unmatched".to_string()),
        "'a' is not 'A'"
    );
    assert_eq!(by_order["10"], (None, "unmatched".to_string()));
    let customers = &reduced.manifest["sources"]["customers"];
    // Both A records, B and C cross; D, E and the null-keyed record do not.
    assert_eq!(customers["scanned_rows"], 4, "{customers}");
    assert_eq!(customers["rows"], 7);
}

#[test]
fn an_empty_key_set_scans_no_target_record() {
    let orders = [
        ORDERS_DDL,
        "INSERT INTO orders VALUES (1,NULL,'2026-01-05','60.00',1)",
    ];
    let f = fixture().orders_in_sqlite(&orders);
    let local = f.succeeded();
    let reduced = f
        .under(Placement::RequireSource, Transfers::Keys)
        .succeeded();
    assert_eq!(reduced.tables, local.tables);
    let customers = &reduced.manifest["sources"]["customers"];
    assert_eq!(customers["scanned_rows"], 0);
    assert_eq!(customers["placement"]["transfer"]["keys"], 0);
}

/// Orders keyed by (region INTEGER, code TEXT, active BOOLEAN) against a
/// target with NOCASE and NUMERIC columns: the transfer compares stored values.
#[test]
fn compound_keys_compare_stored_values_without_affinity_or_collation() {
    let dir = tempfile::tempdir().unwrap();
    let orders = dir.path().join("orders.csv");
    std::fs::write(
        &orders,
        "id,region,code,active\n1,1,ab,true\n2,1,AB,true\n3,2,ab,false\n4,3,zz,true\n5,01,ab,true\n",
    )
    .unwrap();
    let db = dir.path().join("ref.db");
    database(
        &db,
        &[
            "CREATE TABLE codes (region NUMERIC, code TEXT COLLATE NOCASE, active INTEGER, label TEXT)",
            "INSERT INTO codes VALUES (1,'ab',1,'one-ab'), (1,'AB',0,'one-AB-off'), (2,'ab',0,'two-ab'), (2,'ab',1,'two-ab-on'), (9,'q',1,'unused')",
        ],
    );
    let doc = json!({
        "format_version":"0.1","name":"codes",
        "sources":[
            {"id":"orders","identity":["id"],"grain":["id"],"projection":{"fields":{
                "id":column("id","int64"),"region":column("region","int64"),
                "code":column("code","string"),"active":column("active","boolean")}}},
            {"id":"codes","identity":["label"],"grain":["label"],"projection":{"fields":{
                "region":column("region","int64"),"code":column("code","string"),
                "active":column("active","boolean"),"label":column("label","string")}}},
        ],
        "relationships":[{"id":"code","from":"orders","to":"codes",
            "from_keys":["region","code","active"],"to_keys":["region","code","active"],
            "cardinality":"many_to_one"}],
        "outputs":[{"id":"labelled","anchor":"orders","columns":[
            named("id", field("id")),
            named("currency", json!({"op":"literal","value":{"kind":"string","value":"AUD"}})),
            named("label", json!({"op":"field","source":"codes","field":"label","path":["code"]})),
        ]}],
    });
    let recipe: Recipe = serde_json::from_value(doc).unwrap();
    let mut bindings = Bindings::new();
    bindings.insert("orders".into(), binding(&orders, "csv", json!({})));
    bindings.insert(
        "codes".into(),
        binding(&db, "sqlite", json!({"table":"codes"})),
    );
    let mut policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Placement::Local,
        transfers: Transfers::None,
    };
    let run = |policy: &Policy| {
        let report =
            ExecutionContext::default().run(&recipe, &bindings, policy, &AtomicBool::new(false));
        assert!(
            report.status.starts_with("succeeded"),
            "{:?}",
            report.diagnostics
        );
        report
    };
    let local = run(&policy);
    policy.placement = Placement::RequireSource;
    policy.transfers = Transfers::Keys;
    let reduced = run(&policy);
    assert_same_results(&reduced, &local);
    let labels: Vec<_> = reduced.tables["labelled"]
        .iter()
        .map(|r| text(&r["label"]).map(str::to_string))
        .collect();
    assert_eq!(
        labels,
        vec![
            Some("one-ab".to_string()),
            None,
            Some("two-ab".into()),
            None,
            Some("one-ab".into())
        ]
    );
    let codes = &reduced.manifest["sources"]["codes"];
    assert_eq!(codes["rows"], 5);
    assert_eq!(codes["scanned_rows"], 2, "{codes}");
    // Keys (1,ab,true) twice, (1,AB,true), (2,ab,false), (3,zz,true): four distinct.
    assert_eq!(codes["placement"]["transfer"]["keys"], 4);
    assert_eq!(codes["placement"]["transfer"]["key_values"], 12);
}

#[test]
fn thousands_of_keys_transfer_within_the_default_limit() {
    let dir = tempfile::tempdir().unwrap();
    let orders = dir.path().join("orders.csv");
    let mut csv = String::from("id,item\n");
    for i in 0..3000 {
        csv.push_str(&format!("{i},{}\n", i * 2));
    }
    std::fs::write(&orders, csv).unwrap();
    let db = dir.path().join("items.db");
    database(
        &db,
        &[
            "CREATE TABLE items (item INTEGER, name TEXT)",
            "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<8999) INSERT INTO items SELECT x, 'item-' || x FROM n",
        ],
    );
    let doc = json!({
        "format_version":"0.1","name":"items",
        "sources":[
            {"id":"orders","identity":["id"],"grain":["id"],"projection":{"fields":{
                "id":column("id","int64"),"item":column("item","int64")}}},
            {"id":"items","identity":["item"],"grain":["item"],"projection":{"fields":{
                "item":column("item","int64"),"name":column("name","string")}}},
        ],
        "relationships":[{"id":"item","from":"orders","to":"items",
            "from_keys":["item"],"to_keys":["item"],"cardinality":"many_to_one"}],
        "outputs":[{"id":"named","anchor":"orders","columns":[
            named("id", field("id")),
            named("currency", json!({"op":"literal","value":{"kind":"string","value":"AUD"}})),
            named("name", json!({"op":"field","source":"items","field":"name","path":["item"]})),
        ]}],
    });
    let recipe: Recipe = serde_json::from_value(doc).unwrap();
    let mut bindings = Bindings::new();
    bindings.insert("orders".into(), binding(&orders, "csv", json!({})));
    bindings.insert(
        "items".into(),
        binding(&db, "sqlite", json!({"table":"items"})),
    );
    let mut policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Placement::Local,
        transfers: Transfers::None,
    };
    // The lookup row limit refuses the full table but admits the reduced scan.
    policy.limits.lookup_rows = 4000;
    let refused =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(refused.diagnostics[0].code, "RESOURCE_LIMIT");
    policy.placement = Placement::RequireSource;
    policy.transfers = Transfers::Keys;
    let reduced =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert!(
        reduced.status.starts_with("succeeded"),
        "{:?}",
        reduced.diagnostics
    );
    assert_eq!(reduced.tables["named"].len(), 3000);
    assert_eq!(
        text(&reduced.tables["named"][2999]["name"]),
        Some("item-5998")
    );
    let items = &reduced.manifest["sources"]["items"];
    assert_eq!(items["rows"], 9000);
    assert_eq!(items["scanned_rows"], 3000);
    assert_eq!(items["placement"]["transfer"]["keys"], 3000);
}

#[test]
fn a_utf16_target_is_local_when_preferred_and_infeasible_when_required() {
    let utf16 = ["PRAGMA encoding = 'UTF-16le'", CUSTOMERS_DDL, CUSTOMERS];
    let base = fixture_with(&utf16);
    let local = base.succeeded();
    let f = base.under(Placement::PreferSource, Transfers::Keys);
    let report = f.succeeded();
    assert_same_results(&report, &local);
    let placed = &report.manifest["sources"]["customers"]["placement"];
    assert_eq!(placed["scan"], "local", "{placed}");
    assert_eq!(placed["transfer"], Json::Null);

    let f = f.placed(Placement::RequireSource);
    let error = f.failure();
    assert_eq!(error.code, "PLACEMENT_INFEASIBLE");
    assert_eq!(
        error.subject.as_deref(),
        Some("relationship:customer/target")
    );
    // Bound validation reports the same infeasibility once it opens the snapshot.
    let report = validate_bound(&f.recipe(), &f.bindings, &f.policy, "none");
    assert!(!report.valid);
    let diagnostic = report
        .checks
        .iter()
        .find_map(|c| c.diagnostic.as_ref())
        .unwrap();
    assert_eq!(diagnostic.code, "PLACEMENT_INFEASIBLE");
}

#[test]
fn full_bound_validation_reports_the_runs_coverage() {
    let f = fixture().placed(Placement::PreferSource).keys();
    let report = validate_bound(&f.recipe(), &f.bindings, &f.policy, "full");
    assert!(report.valid, "{:?}", report.checks);
    let types = report.checks.iter().find(|c| c.id == "data.types").unwrap();
    assert!(
        types.coverage.contains("scanned records only"),
        "{}",
        types.coverage
    );
}

#[test]
fn transfers_are_policy_and_never_change_the_recipe() {
    let f = fixture();
    let policy: Policy = serde_json::from_value(json!({
        "allowed_read_roots":[f.dir.path()],"allowed_output_roots":[],
        "placement":"prefer_source","transfers":"keys",
        "limits":{"transfer_keys":7}}))
    .unwrap();
    assert_eq!(policy.transfers, Transfers::Keys);
    assert_eq!(policy.limits.transfer_keys, 7);
    let default: Policy =
        serde_json::from_value(json!({"allowed_read_roots":[],"allowed_output_roots":[]})).unwrap();
    assert_eq!(default.transfers, Transfers::None);
    assert!(
        serde_json::from_value::<Policy>(json!({
            "allowed_read_roots":[],"allowed_output_roots":[],"transfers":"rows"}))
        .is_err()
    );
    assert!(!f.recipe().canonical_json().unwrap().contains("transfers"));
}
