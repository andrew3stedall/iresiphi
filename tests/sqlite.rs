//! Slice 3: a SQLite database source and source-side placement of an output's
//! anchor eligibility, through the public validate/explain/run seam.
//! Contract: docs/spec/slice-3.md.
use iresiphi::{
    ExecutionContext, RunReport,
    model::*,
    validation::{explain_bound, validate_bound},
};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

const ORDERS_DDL: &str =
    "CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER)";
const ORDERS: &str = "INSERT INTO orders VALUES
    (1,'A','2026-01-05','60.00',1),
    (2,'A','2026-01-20','40.00',0),
    (3,'B','2026-01-11','200.00',1),
    (4,'C','2026-02-08','120.50',1),
    (5,'Z','2026-02-09','10.00',1),
    (6,'B','2025-12-31','75.25',1)";
const ORDERS_CSV: &str = "order_no,cust,booked,net,open\n1,A,2026-01-05,60.00,true\n2,A,2026-01-20,40.00,false\n3,B,2026-01-11,200.00,true\n4,C,2026-02-08,120.50,true\n5,Z,2026-02-09,10.00,true\n6,B,2025-12-31,75.25,true\n";
const CUSTOMERS: &str = "CustNo,SalesArea\nA,North\nB,North\nC,South\n";

fn column(path: &str, kind: &str) -> Json {
    json!({"path":path,"data_type":{"kind":kind}})
}
fn field(name: &str) -> Json {
    json!({"op":"field","source":"orders","field":name})
}
fn literal(kind: &str, value: &str) -> Json {
    json!({"op":"literal","value":{"kind":kind,"value":value}})
}
fn op(name: &str, left: Json, right: Json) -> Json {
    json!({"op":name,"left":left,"right":right})
}
fn not(expr: Json) -> Json {
    json!({"op":"not","expr":expr})
}
fn named(name: &str, expr: Json) -> Json {
    json!({"name":name,"expr":expr})
}

/// Open orders booked from 2026-01-01.
fn open_from_january() -> Json {
    op(
        "and",
        field("open"),
        op("less_equal", literal("date", "2026-01-01"), field("booked")),
    )
}

fn recipe_doc(eligibility: Json) -> Json {
    json!({
        "format_version":"0.1",
        "name":"open orders",
        "sources":[
            {"id":"orders","identity":["order_no"],"grain":["order_no"],"projection":{"fields":{
                "order_no":column("order_no","int64"),
                "cust":{"path":"cust","data_type":{"kind":"string"},"nullable":true},
                "booked":{"path":"booked","data_type":{"kind":"date"},"nullable":true},
                "net":{"path":"net","data_type":{"kind":"decimal","precision":18,"scale":2}},
                "open":column("open","boolean")}}},
            {"id":"customers","identity":["CustNo"],"grain":["CustNo"],"projection":{"fields":{
                "CustNo":column("CustNo","string"),"SalesArea":column("SalesArea","string")}}},
        ],
        "relationships":[{"id":"order_customer","from":"orders","to":"customers",
            "from_keys":["cust"],"to_keys":["CustNo"],"cardinality":"many_to_one"}],
        "outputs":[{"id":"open_orders","anchor":"orders","eligibility":eligibility,"columns":[
            named("order_no", field("order_no")),
            named("currency", literal("string", "AUD")),
            named("net", field("net")),
            named("area", json!({"op":"field","source":"customers","field":"SalesArea","path":["order_customer"]})),
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

/// Orders in a SQLite table built by `statements`, customers in a CSV file.
fn fixture_with(eligibility: Json, statements: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let orders = dir.path().join("sales.db");
    database(&orders, statements);
    let customers = dir.path().join("customers.csv");
    std::fs::write(&customers, CUSTOMERS).unwrap();
    let mut bindings = Bindings::new();
    bindings.insert(
        "orders".into(),
        binding(&orders, "sqlite", json!({"table":"orders"})),
    );
    bindings.insert("customers".into(), binding(&customers, "csv", json!({})));
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Placement::Local,
    };
    Fixture {
        dir,
        doc: recipe_doc(eligibility),
        bindings,
        policy,
    }
}

fn fixture() -> Fixture {
    fixture_with(open_from_january(), &[ORDERS_DDL, ORDERS])
}

impl Fixture {
    fn recipe(&self) -> Recipe {
        serde_json::from_value(self.doc.clone()).unwrap()
    }
    fn placed(mut self, placement: Placement) -> Self {
        self.policy.placement = placement;
        self
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

/// `(order_no, net, area, status)` of every output row, in output order.
fn rows(report: &RunReport) -> Vec<(String, String, Option<String>, String)> {
    report.tables["open_orders"]
        .iter()
        .map(|r| {
            (
                text(&r["order_no"]).unwrap().to_string(),
                text(&r["net"]).unwrap().to_string(),
                text(&r["area"]).map(str::to_string),
                text(&r["_status"]).unwrap().to_string(),
            )
        })
        .collect()
}

fn unmatched_warnings(report: &RunReport) -> u64 {
    report
        .diagnostics
        .iter()
        .filter(|d| d.code == "LOOKUP_UNMATCHED")
        .map(|d| d.affected_count.unwrap_or(0))
        .sum()
}

#[test]
fn a_sqlite_table_joins_a_csv_lookup_under_local_placement() {
    let f = fixture();
    assert!(iresiphi::validate(&f.recipe()).valid);
    let report = f.succeeded();
    let expected: Vec<(String, String, Option<String>, String)> = [
        ("1", "60.00", Some("North"), "matched"),
        ("3", "200.00", Some("North"), "matched"),
        ("4", "120.50", Some("South"), "matched"),
        ("5", "10.00", None, "unmatched"),
    ]
    .into_iter()
    .map(|(o, n, a, s)| (o.into(), n.into(), a.map(str::to_string), s.into()))
    .collect();
    assert_eq!(rows(&report), expected);
    let counts = &report.manifest["counts"]["open_orders"];
    assert_eq!(counts["source_rows"], 6, "{}", report.manifest);
    assert_eq!(counts["eligible_rows"], 4);
    assert_eq!(counts["output_rows"], 4);
    let read = &report.manifest["sources"]["orders"];
    assert_eq!(read["adapter"], "sqlite");
    assert_eq!(read["rows"], 6);
    assert_eq!(read["scanned_rows"], Json::Null, "nothing was planned");
    assert_eq!(read["placement"], Json::Null);
    assert_eq!(read["capabilities"]["pushdown"], true);
    assert_eq!(read["capabilities"]["snapshot"], "bytes_read");
    assert_eq!(
        report.manifest["sources"]["customers"]["capabilities"]["pushdown"],
        false
    );
    // References name the record by rowid, and the raw evidence is the stored value.
    let evidence = serde_json::to_string(&report.evidence).unwrap();
    assert!(evidence.contains("rowid:5"), "{evidence}");
}

#[test]
fn the_same_recipe_reads_orders_from_csv_with_the_same_result() {
    let sqlite = fixture().succeeded();
    let mut f = fixture();
    let path = f.dir.path().join("orders.csv");
    std::fs::write(&path, ORDERS_CSV).unwrap();
    f.bindings
        .insert("orders".into(), binding(&path, "csv", json!({})));
    let csv = f.succeeded();
    assert_eq!(rows(&sqlite), rows(&csv));
    assert_eq!(
        sqlite.manifest["counts"]["open_orders"],
        csv.manifest["counts"]["open_orders"]
    );
}

#[test]
fn a_pushed_eligibility_scans_fewer_records_and_changes_no_result() {
    let local = fixture().succeeded();
    for placement in [Placement::PreferSource, Placement::RequireSource] {
        let source = fixture().placed(placement).succeeded();
        assert_eq!(rows(&source), rows(&local));
        assert_eq!(source.tables, local.tables);
        assert_eq!(
            source.manifest["counts"]["open_orders"], local.manifest["counts"]["open_orders"],
            "source_rows still counts the table"
        );
        assert_eq!(source.evidence.len(), local.evidence.len());
        assert_eq!(
            source.candidate_evidence.len(),
            local.candidate_evidence.len()
        );
        let read = &source.manifest["sources"]["orders"];
        assert_eq!(read["rows"], 6);
        assert_eq!(read["scanned_rows"], 4);
        assert_eq!(read["placement"]["scan"], "source");
        assert_eq!(
            read["placement"]["operation"],
            "output:open_orders/eligibility"
        );
        assert_eq!(
            read["placement"]["checks"]["lookups"],
            "scanned records only"
        );
        assert_eq!(
            read["bytes_read"],
            local.manifest["sources"]["orders"]["bytes_read"]
        );
        assert_eq!(
            read["fingerprint"],
            local.manifest["sources"]["orders"]["fingerprint"]
        );
    }
}

#[test]
fn lookup_checks_cover_scanned_records_under_source_placement_and_say_so() {
    // Order 7 is closed and its customer is unknown: ineligible, but unmatched.
    let extra = "INSERT INTO orders VALUES (7,'Q','2026-03-01','5.00',0)";
    let local = fixture_with(open_from_january(), &[ORDERS_DDL, ORDERS, extra]).succeeded();
    let source = fixture_with(open_from_january(), &[ORDERS_DDL, ORDERS, extra])
        .placed(Placement::PreferSource)
        .succeeded();
    assert_eq!(source.tables, local.tables);
    assert_eq!(unmatched_warnings(&local), 2, "{:?}", local.diagnostics);
    assert_eq!(unmatched_warnings(&source), 1, "{:?}", source.diagnostics);
    let checks = serde_json::to_string(&source.manifest["check_results"]).unwrap();
    assert!(
        checks.contains("scanned records only"),
        "coverage is disclosed: {checks}"
    );
    let local_checks = serde_json::to_string(&local.manifest["check_results"]).unwrap();
    assert!(!local_checks.contains("scanned records only"));
}

#[test]
fn explain_names_each_placement_without_opening_a_source() {
    let mut f = fixture().placed(Placement::PreferSource);
    // The database does not need to exist for a bound explanation.
    std::fs::remove_file(&f.bindings["orders"].path).unwrap();
    let bound = &f.explain()["bound"];
    assert_eq!(bound["feasible"], true, "{bound}");
    assert_eq!(bound["transfers"], json!([]));
    assert_eq!(bound["placement"]["policy"], "prefer_source");
    let operation = &bound["placement"]["operations"][0];
    assert_eq!(operation["operation"], "output:open_orders/eligibility");
    assert_eq!(operation["source"], "source:orders");
    assert_eq!(operation["placement"], "source");
    let orders = bound["reads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["source"] == "source:orders")
        .unwrap();
    assert_eq!(orders["scan"], "filtered at the source");
    assert_eq!(
        orders["capabilities"]["pushdown_operations"][0],
        "anchor_eligibility_filter"
    );

    f.policy.placement = Placement::Local;
    let operation = &f.explain()["bound"]["placement"]["operations"][0];
    assert_eq!(operation["placement"], "local");
    assert!(
        operation["reason"].as_str().unwrap().contains("policy"),
        "{operation}"
    );
}

#[test]
fn a_decimal_comparison_is_never_pushed() {
    let over_fifty = op(
        "less",
        json!({"op":"literal","value":{"kind":"decimal","value":"50.00","precision":18,"scale":2}}),
        field("net"),
    );
    // Preferring the source keeps the comparison local and says why.
    let f = fixture_with(over_fifty.clone(), &[ORDERS_DDL, ORDERS]).placed(Placement::PreferSource);
    let operation = f.explain()["bound"]["placement"]["operations"][0].clone();
    assert_eq!(operation["placement"], "local");
    assert!(
        operation["reason"].as_str().unwrap().contains("decimal"),
        "{operation}"
    );
    let report = f.succeeded();
    // "120.50" < "50.00" bytewise; the exact comparison keeps it.
    let kept: Vec<_> = rows(&report).into_iter().map(|r| r.0).collect();
    assert_eq!(kept, ["1", "3", "4", "6"]);
    assert_eq!(
        report.manifest["sources"]["orders"]["scanned_rows"],
        Json::Null
    );

    // Requiring the source refuses before any source is opened.
    let f = fixture_with(over_fifty, &[ORDERS_DDL, ORDERS]).placed(Placement::RequireSource);
    std::fs::remove_file(&f.bindings["orders"].path).unwrap();
    let error = f.failure();
    assert_eq!(error.code, "PLACEMENT_INFEASIBLE");
    assert_eq!(
        error.subject.as_deref(),
        Some("output:open_orders/eligibility")
    );
    assert!(error.message.contains("decimal"), "{}", error.message);
    assert!(
        error
            .remediation
            .as_deref()
            .unwrap()
            .contains("prefer_source"),
        "the alternative is named"
    );
    let validation = validate_bound(&f.recipe(), &f.bindings, &f.policy, "none");
    assert!(!validation.valid);
    assert!(validation.checks.iter().any(|c| {
        c.diagnostic
            .as_ref()
            .is_some_and(|d| d.code == "PLACEMENT_INFEASIBLE")
    }));
    let bound = &f.explain()["bound"];
    assert_eq!(bound["feasible"], false);
    assert_eq!(bound["diagnostic"]["code"], "PLACEMENT_INFEASIBLE");
    assert_eq!(bound["placement"]["operations"][0]["placement"], "local");
}

#[test]
fn another_use_of_the_source_keeps_its_filter_local() {
    let mut f = fixture().placed(Placement::PreferSource);
    let mut second = f.doc["outputs"][0].clone();
    second["id"] = json!("all_orders");
    second.as_object_mut().unwrap().remove("eligibility");
    f.doc["outputs"].as_array_mut().unwrap().push(second);
    let operation = f.explain()["bound"]["placement"]["operations"][0].clone();
    assert_eq!(operation["placement"], "local");
    assert!(
        operation["reason"].as_str().unwrap().contains("all_orders"),
        "{operation}"
    );
    let report = f.succeeded();
    assert_eq!(report.tables["all_orders"].len(), 6);
    assert_eq!(report.tables["open_orders"].len(), 4);
    f.policy.placement = Placement::RequireSource;
    assert_eq!(f.failure().code, "PLACEMENT_INFEASIBLE");
}

#[test]
fn a_file_source_is_not_bound_by_require_source() {
    let mut f = fixture().placed(Placement::RequireSource);
    let path = f.dir.path().join("orders.csv");
    std::fs::write(&path, ORDERS_CSV).unwrap();
    f.bindings
        .insert("orders".into(), binding(&path, "csv", json!({})));
    let operation = f.explain()["bound"]["placement"]["operations"][0].clone();
    assert_eq!(operation["placement"], "local");
    assert!(operation["reason"].as_str().unwrap().contains("csv"));
    assert_eq!(f.succeeded().tables["open_orders"].len(), 4);
}

#[test]
fn pushed_predicates_agree_with_the_local_executor() {
    // Nulls, a NOCASE collation and a NUMERIC affinity must not change a result.
    let ddl = "CREATE TABLE orders (order_no INTEGER PRIMARY KEY, cust TEXT COLLATE NOCASE, booked NUMERIC, net TEXT, open INTEGER)";
    let data = "INSERT INTO orders VALUES
        (1,'A','2026-01-05','60.00',1),
        (2,'a','2026-01-20','40.00',0),
        (3,'B',NULL,'200.00',1),
        (4,NULL,'2026-02-08','120.50',1),
        (5,'é','2026-02-09','10.00',0),
        (-6,'10','2025-12-31','75.25',1),
        (7,'9','2026-01-01','1.00',0)";
    let text = |v: &str| literal("string", v);
    let int = |v: &str| literal("int64", v);
    let date = |v: &str| literal("date", v);
    let predicates = vec![
        field("open"),
        not(field("open")),
        op("equal", field("cust"), text("a")),
        op("equal", field("cust"), text("A")),
        not(op("equal", field("cust"), text("A"))),
        op("less", field("cust"), text("B")),
        op("less_equal", text("a"), field("cust")),
        op("less", field("cust"), text("9")),
        op("less", field("order_no"), int("3")),
        op("less_equal", int("-6"), field("order_no")),
        op("equal", field("order_no"), int("9223372036854775807")),
        op("less", field("booked"), date("2026-01-20")),
        op("less_equal", date("2026-01-01"), field("booked")),
        json!({"op":"is_null","expr":field("booked")}),
        not(json!({"op":"is_null","expr":field("cust")})),
        json!({"op":"is_null","expr":op("equal", field("cust"), text("A"))}),
        op(
            "or",
            op("equal", field("cust"), text("B")),
            op("less", field("booked"), date("2026-01-01")),
        ),
        op(
            "and",
            not(field("open")),
            op("less", text("A"), field("cust")),
        ),
        op(
            "or",
            field("open"),
            json!({"op":"is_null","expr":field("cust")}),
        ),
        op("equal", field("open"), literal_bool(false)),
        op("less", field("order_no"), field("order_no")),
        op("equal", field("cust"), field("cust")),
    ];
    for predicate in predicates {
        let local = fixture_with(predicate.clone(), &[ddl, data]).succeeded();
        let f = fixture_with(predicate.clone(), &[ddl, data]).placed(Placement::RequireSource);
        let source = f.succeeded();
        assert_eq!(source.tables, local.tables, "{predicate}");
        assert_eq!(
            source.manifest["counts"], local.manifest["counts"],
            "{predicate}"
        );
        let read = &source.manifest["sources"]["orders"];
        assert_eq!(read["placement"]["scan"], "source", "{predicate}");
        assert_eq!(
            read["scanned_rows"].as_u64().unwrap() as usize,
            local.tables["open_orders"].len(),
            "{predicate}"
        );
    }
}

fn literal_bool(value: bool) -> Json {
    json!({"op":"literal","value":{"kind":"boolean","value":value}})
}

#[test]
fn values_convert_by_storage_class_and_never_through_a_float() {
    // An INTEGER is an exact decimal; text is parsed as decimal text.
    let f = fixture_with(
        field("open"),
        &[
            ORDERS_DDL,
            "INSERT INTO orders VALUES (1,'A','2026-01-05',60,1),(2,'B','2026-01-06','0.10',1)",
        ],
    );
    let nets: Vec<_> = rows(&f.succeeded()).into_iter().map(|r| r.1).collect();
    assert_eq!(nets, ["60.00", "0.10"]);

    for (bad, code) in [
        ("(1,'A','2026-01-05',60.5,1)", "INVALID_VALUE"), // REAL decimal
        ("(1,'A','2026-01-05',x'3630',1)", "INVALID_VALUE"), // BLOB
        ("('1','A','2026-01-05','60.00',1)", "INVALID_VALUE"), // TEXT in an int64 field
        ("(1,7,'2026-01-05','60.00',1)", "INVALID_VALUE"), // INTEGER in a string field
        ("(1,'A','2026-01-05','60.00',2)", "INVALID_VALUE"), // Boolean other than 0 or 1
        ("(1,'A','05/01/2026','60.00',1)", "INVALID_VALUE"), // not an ISO date
        ("(1,'A','2026-01-05','sixty',1)", "INVALID_DECIMAL"),
        ("(1,'A','2026-01-05',NULL,1)", "NULL_REQUIRED"),
    ] {
        for placement in [Placement::Local, Placement::PreferSource] {
            let insert = format!(
                "INSERT INTO orders VALUES {bad},(8,'A','2026-01-05','1.00',1),(9,'A','2026-01-05','1.00',1)"
            );
            // Untyped columns have no affinity, so each value keeps its storage class.
            let untyped = "CREATE TABLE orders (order_no, cust, booked, net, open)";
            let error = fixture_with(field("open"), &[untyped, &insert])
                .placed(placement)
                .failure();
            assert_eq!(error.code, code, "{bad} {placement:?}");
            assert_eq!(error.affected_count, Some(1), "{bad}");
            assert_eq!(error.samples[0].location, "rowid:1", "{bad}");
            assert_eq!(error.subject.as_deref(), Some("source:orders"));
        }
    }
}

#[test]
fn identity_is_checked_for_every_record_under_either_placement() {
    for (data, code) in [
        (
            "INSERT INTO orders VALUES (1,'A','2026-01-05','1.00',1),(1,'B','2026-01-05','1.00',0)",
            "DUPLICATE_IDENTITY",
        ),
        (
            "INSERT INTO orders VALUES (1,'A','2026-01-05','1.00',1),(NULL,'B','2026-01-05','1.00',0)",
            "NULL_REQUIRED",
        ),
    ] {
        for placement in [
            Placement::Local,
            Placement::PreferSource,
            Placement::RequireSource,
        ] {
            let error = fixture_with(field("open"), &[ORDERS_DDL, data])
                .placed(placement)
                .failure();
            assert_eq!(error.code, code, "{placement:?}");
        }
    }
}

#[test]
fn row_limits_apply_to_scanned_records() {
    let mut f = fixture();
    f.policy.limits.anchor_rows = 4;
    assert_eq!(
        f.failure().code,
        "RESOURCE_LIMIT",
        "six records exceed four"
    );
    let mut f = fixture().placed(Placement::PreferSource);
    f.policy.limits.anchor_rows = 4;
    assert_eq!(f.succeeded().tables["open_orders"].len(), 4);
}

#[test]
fn a_binding_names_one_ordinary_table_with_the_declared_columns() {
    let cases: Vec<(Vec<&str>, Json, &str)> = vec![
        (
            vec![ORDERS_DDL],
            json!({"table":"missing"}),
            "INVALID_BINDING",
        ),
        (
            vec![ORDERS_DDL],
            json!({"table":"ORDERS"}),
            "INVALID_BINDING",
        ),
        (vec![ORDERS_DDL], json!({}), "INVALID_BINDING"),
        (
            vec![ORDERS_DDL],
            json!({"table":"orders","query":"select 1"}),
            "INVALID_BINDING",
        ),
        (
            vec![ORDERS_DDL, "CREATE VIEW v AS SELECT * FROM orders"],
            json!({"table":"v"}),
            "INVALID_BINDING",
        ),
        (
            vec![
                "CREATE TABLE orders (order_no INTEGER PRIMARY KEY, cust TEXT, booked TEXT, net TEXT, open INTEGER) WITHOUT ROWID",
            ],
            json!({"table":"orders"}),
            "INVALID_BINDING",
        ),
        (
            vec!["CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT)"],
            json!({"table":"orders"}),
            "INVALID_PROJECTION",
        ),
    ];
    for (statements, settings, code) in cases {
        let mut f = fixture_with(open_from_january(), &statements);
        let path = f.bindings["orders"].path.clone();
        f.bindings
            .insert("orders".into(), binding(&path, "sqlite", settings.clone()));
        assert_eq!(f.failure().code, code, "{settings}");
        // Schema inspection finds the same problem without reading a record.
        let report = validate_bound(&f.recipe(), &f.bindings, &f.policy, "none");
        let diagnostic = report
            .checks
            .iter()
            .find_map(|c| c.diagnostic.as_ref())
            .unwrap_or_else(|| panic!("{settings} validated"));
        assert_eq!(diagnostic.code, code, "{settings}");
    }
    let f = fixture();
    assert!(validate_bound(&f.recipe(), &f.bindings, &f.policy, "none").valid);
    assert!(validate_bound(&f.recipe(), &f.bindings, &f.policy, "full").valid);
}

#[test]
fn a_file_that_is_not_a_database_is_rejected() {
    let f = fixture();
    std::fs::write(&f.bindings["orders"].path, "order_no,cust\n1,A\n").unwrap();
    assert_eq!(f.failure().code, "INVALID_SQLITE");
    std::fs::write(&f.bindings["orders"].path, "").unwrap();
    assert_eq!(f.failure().code, "INVALID_SQLITE");
}

#[test]
fn the_read_policy_applies_to_a_database_file() {
    let mut f = fixture();
    let elsewhere = tempfile::tempdir().unwrap();
    let path = elsewhere.path().join("sales.db");
    database(&path, &[ORDERS_DDL, ORDERS]);
    f.bindings.insert(
        "orders".into(),
        binding(&path, "sqlite", json!({"table":"orders"})),
    );
    assert_eq!(f.failure().code, "POLICY_DENIED");
}

#[test]
fn placement_is_policy_and_defaults_to_local() {
    let policy: Policy = serde_json::from_value(json!({
        "allowed_read_roots":["/data"],"allowed_output_roots":[]
    }))
    .unwrap();
    assert_eq!(policy.placement, Placement::Local);
    let policy: Policy = serde_json::from_value(json!({
        "allowed_read_roots":["/data"],"allowed_output_roots":[],"placement":"prefer_source"
    }))
    .unwrap();
    assert_eq!(policy.placement, Placement::PreferSource);
    assert!(
        serde_json::from_value::<Policy>(json!({
            "allowed_read_roots":["/data"],"allowed_output_roots":[],"placement":"remote"
        }))
        .is_err()
    );
    let _ = BTreeMap::<String, String>::new();
}

/// Requests cancellation from another thread `after` the run enters parsing.
struct CancelDuringParse {
    flag: std::sync::Arc<AtomicBool>,
    after: std::time::Duration,
}
impl iresiphi::Cancellation for CancelDuringParse {
    fn flag(&self) -> &AtomicBool {
        &self.flag
    }
    fn entered(&self, phase: iresiphi::RunPhase) {
        if phase == iresiphi::RunPhase::Parse {
            let (flag, after) = (self.flag.clone(), self.after);
            std::thread::spawn(move || {
                std::thread::sleep(after);
                flag.store(true, std::sync::atomic::Ordering::Release);
            });
        }
    }
}

#[test]
fn a_run_cancelled_during_a_sqlite_scan_reports_cancelled() {
    // Enough records that the scan, not the file read, takes the run's time.
    const RECORDS: usize = 200_000;
    let many = format!(
        "WITH RECURSIVE n(i) AS (SELECT 7 UNION ALL SELECT i + 1 FROM n WHERE i < {})
         INSERT INTO orders SELECT i, 'A', '2026-01-05', '1.00', 1 FROM n",
        RECORDS + 6
    );
    for placement in [Placement::Local, Placement::RequireSource] {
        let mut f =
            fixture_with(open_from_january(), &[ORDERS_DDL, ORDERS, &many]).placed(placement);
        f.policy.limits.anchor_rows = RECORDS + 10;
        f.policy.limits.retained_bytes = 1 << 30;
        f.policy.limits.output_bytes = 1 << 30;
        let cancel = CancelDuringParse {
            flag: Default::default(),
            after: std::time::Duration::from_millis(200),
        };
        let started = std::time::Instant::now();
        let report = ExecutionContext::default().run(&f.recipe(), &f.bindings, &f.policy, &cancel);
        let latency = started.elapsed();
        assert_eq!(
            report.status, "cancelled",
            "{placement:?} {:?}",
            report.diagnostics
        );
        assert!(
            report.tables.is_empty(),
            "a cancelled run publishes nothing"
        );
        let error = &report.diagnostics[0];
        assert_eq!(error.code, "CANCELLED", "{placement:?}");
        assert_eq!(error.phase, "source", "{placement:?}");
        assert_eq!(
            error.subject.as_deref(),
            Some("source:orders"),
            "{placement:?}"
        );
        assert_eq!(report.manifest["checks"]["data"], "incomplete");
        eprintln!("cancel during {placement:?} sqlite scan: run returned after {latency:?}");
    }
}

/// The fixture's orders in a database whose text encoding is UTF-16le. Customer
/// "Ā" (U+0100) sorts after "B" in UTF-8 but before it in UTF-16le bytes.
fn utf16_fixture(eligibility: Json) -> Fixture {
    fixture_with(
        eligibility,
        &[
            "PRAGMA encoding = 'UTF-16le'",
            ORDERS_DDL,
            ORDERS,
            "INSERT INTO orders VALUES (7,'Ā','2026-03-01','5.00',1)",
        ],
    )
}

#[test]
fn a_utf16_database_is_read_locally_with_the_same_result() {
    let encoding: String =
        rusqlite::Connection::open(utf16_fixture(field("open")).bindings["orders"].path.clone())
            .unwrap()
            .query_row("PRAGMA encoding", [], |r| r.get(0))
            .unwrap();
    assert_eq!(encoding, "UTF-16le");
    // Local placement decodes UTF-16 text exactly.
    let local = utf16_fixture(open_from_january()).succeeded();
    let utf8 = fixture_with(
        open_from_january(),
        &[
            ORDERS_DDL,
            ORDERS,
            "INSERT INTO orders VALUES (7,'Ā','2026-03-01','5.00',1)",
        ],
    )
    .succeeded();
    assert_eq!(local.tables, utf8.tables);
    assert_eq!(rows(&local)[4].0, "7");
    assert_eq!(local.manifest["sources"]["orders"]["placement"], Json::Null);

    // A text comparison whose bytewise order differs between the encodings.
    let after_b = op("less", literal("string", "B"), field("cust"));
    let mut f = utf16_fixture(after_b.clone());
    let local = f.succeeded();
    let kept: Vec<_> = rows(&local).into_iter().map(|r| r.0).collect();
    assert_eq!(kept, ["4", "5", "7"], "C, Z and Ā sort after B");

    // Preferring the source reads the whole table locally and says why.
    f.policy.placement = Placement::PreferSource;
    assert_eq!(
        f.explain()["bound"]["placement"]["operations"][0]["placement"],
        "source",
        "explain opens no source, so it cannot see the encoding"
    );
    let source = f.succeeded();
    assert_eq!(source.tables, local.tables);
    assert_eq!(source.manifest["counts"], local.manifest["counts"]);
    assert_eq!(source.evidence, local.evidence);
    let read = &source.manifest["sources"]["orders"];
    assert_eq!(read["placement"]["scan"], "local", "{read}");
    assert!(
        read["placement"]["reason"]
            .as_str()
            .unwrap()
            .contains("UTF-16le"),
        "{read}"
    );
    assert_eq!(read["rows"], 7);
    assert_eq!(read["scanned_rows"], 7, "every record was read");

    // Requiring the source cannot be met.
    let error = utf16_fixture(after_b)
        .placed(Placement::RequireSource)
        .failure();
    assert_eq!(error.code, "PLACEMENT_INFEASIBLE", "{error:?}");
    assert_eq!(
        error.subject.as_deref(),
        Some("output:open_orders/eligibility")
    );
    assert!(error.message.contains("UTF-16le"), "{}", error.message);
    assert!(
        error
            .remediation
            .as_deref()
            .unwrap()
            .contains("prefer_source"),
        "the alternative is named"
    );
}
