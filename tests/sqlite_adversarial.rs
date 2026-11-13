//! Adversarial checks of slice 3's placement claim (docs/spec/slice-3.md): under
//! source placement, output rows, values, evidence records and output counts are
//! the same as under local placement, and anything that cannot be pushed with the
//! same semantics stays local (or is PLACEMENT_INFEASIBLE under require_source).
//!
//! Regression coverage for placement safety and its disclosed validation scope.
use iresiphi::{ExecutionContext, RunReport, model::*, validation::validate_bound};
use serde_json::{Value as Json, json};
use std::{path::Path, sync::atomic::AtomicBool};

const ORDERS_DDL: &str = "CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER, flag INTEGER)";
/// Orders 1 and 3 are open; 2 and 4 are closed. Flag is a nullable Boolean.
const ORDERS: &str = "INSERT INTO orders VALUES
    (1,'A','2026-01-05','60.00',1,1),
    (2,'A','2026-01-20','40.00',0,NULL),
    (3,'B','2026-01-11','200.00',1,0),
    (4,'C','2026-02-08','120.50',0,1)";
const CUSTOMERS: &str = "CustNo,SalesArea\nA,North\nB,North\nC,South\n";

fn column(path: &str, kind: &str, nullable: bool) -> Json {
    json!({"path":path,"data_type":{"kind":kind},"nullable":nullable})
}
fn field(name: &str) -> Json {
    json!({"op":"field","source":"orders","field":name})
}
fn literal(kind: &str, value: Json) -> Json {
    json!({"op":"literal","value":{"kind":kind,"value":value}})
}
fn op(name: &str, left: Json, right: Json) -> Json {
    json!({"op":name,"left":left,"right":right})
}
fn not(expr: Json) -> Json {
    json!({"op":"not","expr":expr})
}
fn is_null(expr: Json) -> Json {
    json!({"op":"is_null","expr":expr})
}
fn named(name: &str, expr: Json) -> Json {
    json!({"name":name,"expr":expr})
}

fn orders_source() -> Json {
    json!({"id":"orders","identity":["order_no"],"grain":["order_no"],"projection":{"fields":{
        "order_no":column("order_no","int64",false),
        "cust":column("cust","string",true),
        "booked":column("booked","date",true),
        "net":{"path":"net","data_type":{"kind":"decimal","precision":18,"scale":2}},
        "open":column("open","boolean",false),
        "flag":column("flag","boolean",true)}}})
}

/// Orders anchored output with the customer's area looked up through `cardinality`.
fn recipe_doc(eligibility: Json, cardinality: &str) -> Json {
    json!({
        "format_version":"0.1",
        "name":"open orders",
        "sources":[
            orders_source(),
            {"id":"customers","identity":["CustNo"],"grain":["CustNo"],"projection":{"fields":{
                "CustNo":column("CustNo","string",false),"SalesArea":column("SalesArea","string",false)}}},
        ],
        "relationships":[{"id":"order_customer","from":"orders","to":"customers",
            "from_keys":["cust"],"to_keys":["CustNo"],"cardinality":cardinality}],
        "outputs":[{"id":"open_orders","anchor":"orders","eligibility":eligibility,"columns":[
            named("order_no", field("order_no")),
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

fn fixture_doc(doc: Json, statements: &[&str]) -> Fixture {
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
        doc,
        bindings,
        policy,
    }
}

fn fixture(eligibility: Json, statements: &[&str]) -> Fixture {
    fixture_doc(recipe_doc(eligibility, "many_to_one"), statements)
}

impl Fixture {
    fn recipe(&self) -> Recipe {
        serde_json::from_value(self.doc.clone()).unwrap()
    }
    fn run_with(&mut self, placement: Placement) -> RunReport {
        self.policy.placement = placement;
        ExecutionContext::default().run(
            &self.recipe(),
            &self.bindings,
            &self.policy,
            &AtomicBool::new(false),
        )
    }
    fn pushed(&self) -> bool {
        let mut policy = self.policy.clone();
        policy.placement = Placement::PreferSource;
        iresiphi::validation::explain_bound(&self.recipe(), &self.bindings, &policy)["bound"]["placement"]
            ["operations"][0]["placement"]
            == "source"
    }
}

fn codes(report: &RunReport) -> Vec<String> {
    report.diagnostics.iter().map(|d| d.code.clone()).collect()
}

/// The parts of a run the placement claim covers.
fn outcome(report: &RunReport) -> Json {
    json!({
        "status": report.status,
        "tables": report.tables,
        "evidence": report.evidence,
        "candidate_evidence": report.candidate_evidence,
        "detail_evidence": report.detail_evidence,
        "counts": report.manifest["counts"],
        "errors": report.diagnostics.iter().filter(|d| d.severity == "error")
            .map(|d| &d.code).collect::<Vec<_>>(),
    })
}

/// Run the same files under local and source placement and compare.
fn assert_same_outcome(f: &mut Fixture, placement: Placement) -> (RunReport, RunReport) {
    let local = f.run_with(Placement::Local);
    let source = f.run_with(placement);
    assert_eq!(
        outcome(&source),
        outcome(&local),
        "local {:?}\nsource {:?}",
        local.diagnostics,
        source.diagnostics
    );
    (local, source)
}

// ---------------------------------------------------------------------------
// Regressions for defects found in the merged implementation.
// ---------------------------------------------------------------------------

/// A one_to_one lookup marks an anchor ambiguous when another anchor matches the
/// same target. Under local placement the closed order 2 shares customer A with
/// open order 1, so order 1 is ambiguous with a null area. Under source placement
/// order 2 is never scanned, so order 1 is matched with area North.
#[test]
fn a_one_to_one_lookup_from_the_anchor_keeps_its_filter_local() {
    let mut f = fixture_doc(
        recipe_doc(field("open"), "one_to_one"),
        &[ORDERS_DDL, ORDERS],
    );
    let local = f.run_with(Placement::Local);
    assert_eq!(
        local.tables["open_orders"][0]["_status"],
        Value::String {
            value: "ambiguous".into()
        }
    );
    // Regression: order 1 is "matched" with area North under source placement.
    assert_same_outcome(&mut f, Placement::PreferSource);
    assert!(
        !f.pushed(),
        "a one_to_one lookup needs every anchor record to detect a shared target"
    );
}

/// Detail records of a one_to_many relationship from the anchor that match no
/// scanned anchor are reported as orphans. Under source placement the lines of the
/// closed order 2 match no scanned order, so the run warns ORPHAN_DETAIL for
/// records that have a parent; under local placement it does not.
#[test]
fn components_of_dropped_anchors_are_not_reported_as_orphans() {
    let mut doc = recipe_doc(field("open"), "many_to_one");
    doc["sources"].as_array_mut().unwrap().push(json!(
        {"id":"lines","identity":["line"],"grain":["line"],"projection":{"fields":{
            "line":column("line","string",false),"order_no":column("order_no","int64",false)}}}
    ));
    doc["relationships"].as_array_mut().unwrap().push(json!(
        {"id":"order_lines","from":"orders","to":"lines","from_keys":["order_no"],
         "to_keys":["order_no"],"cardinality":"one_to_many"}
    ));
    doc["outputs"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .push(named(
            "lines",
            json!({"op":"aggregate","relationship":"order_lines","function":"count_records"}),
        ));
    let mut f = fixture_doc(doc, &[ORDERS_DDL, ORDERS]);
    let lines = f.dir.path().join("lines.csv");
    std::fs::write(&lines, "line,order_no\nL1,1\nL2,2\nL3,3\nL4,4\n").unwrap();
    f.bindings
        .insert("lines".into(), binding(&lines, "csv", json!({})));
    assert!(!f.pushed(), "detail parent checks need every anchor");
    let (local, source) = assert_same_outcome(&mut f, Placement::PreferSource);
    assert!(!codes(&local).contains(&"ORPHAN_DETAIL".to_string()));
    assert_eq!(codes(&source), codes(&local), "{:?}", source.diagnostics);
}

/// Invalid UTF-8 fails the local reader. Source placement checks UTF-8 only
/// for scanned text, as disclosed in the contract and validation coverage.
#[test]
fn utf8_checks_cover_scanned_text_as_disclosed() {
    let mut f = fixture(
        field("open"),
        &[
            ORDERS_DDL,
            ORDERS,
            "INSERT INTO orders VALUES (5, CAST(x'ff' AS TEXT), '2026-01-05', '1.00', 0, NULL)",
        ],
    );
    let local = f.run_with(Placement::Local);
    assert_eq!(local.status, "failed");
    assert_eq!(local.diagnostics[0].code, "INVALID_VALUE");
    let source = f.run_with(Placement::PreferSource);
    assert!(
        source.status.starts_with("succeeded"),
        "{:?}",
        source.diagnostics
    );
    assert_eq!(source.tables["open_orders"].len(), 2);
    assert!(
        serde_json::to_string(&source.manifest["check_results"])
            .unwrap()
            .contains("UTF-8")
    );
    let validation = validate_bound(&f.recipe(), &f.bindings, &f.policy, "full");
    assert!(validation.valid);
    assert!(
        validation
            .checks
            .iter()
            .any(|c| c.coverage.contains("UTF-8"))
    );
}

/// A text value larger than `limits.field_bytes` fails the local read with
/// RESOURCE_LIMIT. Under source placement a dropped record holding it is never
/// measured in the original implementation. The source must check every TEXT value.
#[test]
fn an_oversized_text_value_in_a_dropped_record_fails_under_either_placement() {
    let big = format!(
        "INSERT INTO orders VALUES (5, '{}', '2026-01-05', '1.00', 0, NULL)",
        "x".repeat(200)
    );
    let mut f = fixture(field("open"), &[ORDERS_DDL, ORDERS, &big]);
    f.policy.limits.field_bytes = 100;
    let local = f.run_with(Placement::Local);
    assert_eq!(local.diagnostics[0].code, "RESOURCE_LIMIT");
    assert_same_outcome(&mut f, Placement::PreferSource);
}

/// The source-side Boolean check `+"col" NOT IN (0, 1)` keeps the column's
/// declared collation. A collation the database names but the connection lacks
/// fails that query with INVALID_SQLITE, while the local read, which compares
/// nothing, succeeds.
#[test]
fn an_unknown_collation_on_a_boolean_column_does_not_fail_the_source_checks() {
    let mut f = fixture(
        field("open"),
        &[
            ORDERS_DDL,
            ORDERS,
            "PRAGMA writable_schema = ON",
            "UPDATE sqlite_schema SET sql = 'CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER COLLATE hostile, flag INTEGER)' WHERE name = 'orders'",
        ],
    );
    let local = f.run_with(Placement::Local);
    assert!(
        local.status.starts_with("succeeded"),
        "{:?}",
        local.diagnostics
    );
    assert_same_outcome(&mut f, Placement::PreferSource);
}

/// A predicate with more literals than SQLite allows bound parameters (32766)
/// is accepted by the planner and then fails to prepare: INVALID_SQLITE under
/// source placement in the original implementation, success under local placement.
#[test]
fn a_predicate_with_more_literals_than_sqlite_parameters_stays_local() {
    fn any_of(values: std::ops::Range<i64>) -> Json {
        if values.end - values.start == 1 {
            return op(
                "equal",
                field("order_no"),
                literal("int64", json!(values.start.to_string())),
            );
        }
        let middle = values.start + (values.end - values.start) / 2;
        op(
            "or",
            any_of(values.start..middle),
            any_of(middle..values.end),
        )
    }
    let mut f = fixture(any_of(-32_800..3), &[ORDERS_DDL, ORDERS]);
    let local = f.run_with(Placement::Local);
    assert!(
        local.status.starts_with("succeeded"),
        "{:?}",
        local.diagnostics
    );
    assert_eq!(local.tables["open_orders"].len(), 2);
    assert_same_outcome(&mut f, Placement::PreferSource);
}

/// Bound validation with full data checks runs the run, so under source placement
/// it checks text formats of scanned records only, yet still reports "full: every
/// row of every selected source". The same files fail full validation locally.
#[test]
fn full_bound_validation_does_not_claim_every_row_under_source_placement() {
    let mut f = fixture(
        field("open"),
        &[
            ORDERS_DDL,
            ORDERS,
            "INSERT INTO orders VALUES (5,'A','05/01/2026','1.00',0,NULL)",
        ],
    );
    let local = validate_bound(&f.recipe(), &f.bindings, &f.policy, "full");
    assert!(!local.valid, "the closed order's date is not ISO");
    f.policy.placement = Placement::PreferSource;
    let source = validate_bound(&f.recipe(), &f.bindings, &f.policy, "full");
    let claims_every_row = source
        .checks
        .iter()
        .any(|c| c.coverage.contains("every row of every selected source"));
    assert!(
        !source.valid || !claims_every_row,
        "full validation passed and claimed every row: {:?}",
        source.checks
    );
}

/// Schema inspection opens the database but does not check its text encoding,
/// so bound validation calls a UTF-16 database valid under require_source and the
/// run then fails with PLACEMENT_INFEASIBLE (contract "placement" clauses 1, 4).
#[test]
fn bound_validation_reports_an_infeasible_encoding_under_require_source() {
    let mut f = fixture(
        field("open"),
        &["PRAGMA encoding = 'UTF-16le'", ORDERS_DDL, ORDERS],
    );
    f.policy.placement = Placement::RequireSource;
    let run = f.run_with(Placement::RequireSource);
    assert_eq!(run.diagnostics[0].code, "PLACEMENT_INFEASIBLE");
    let report = validate_bound(&f.recipe(), &f.bindings, &f.policy, "none");
    assert!(
        !report.valid,
        "validation said valid for a run that fails: {:?}",
        report.checks
    );
}

/// A generated column named `rowid` does not appear in `pragma_table_info`, so
/// the reader takes `rowid` to be the real rowid. The query then reads, orders
/// and references records by the generated value (contract "SQLite source" 4).
#[test]
fn a_generated_column_named_rowid_does_not_replace_the_record_reference() {
    let mut f = fixture(
        field("open"),
        &[
            "CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, open INTEGER, flag INTEGER, rowid INTEGER GENERATED ALWAYS AS (100 - order_no) VIRTUAL)",
            ORDERS,
        ],
    );
    let report = f.run_with(Placement::Local);
    assert!(
        report.status.starts_with("succeeded"),
        "{:?}",
        report.diagnostics
    );
    // Regression: the records are referenced and ordered by the generated value
    // (rowid:97 for order 3, then rowid:99 for order 1).
    let evidence = serde_json::to_string(&report.evidence).unwrap();
    assert!(evidence.contains("\"rowid:1\""), "{evidence}");
    assert!(!evidence.contains("rowid:99"), "{evidence}");
}

/// Before the iterative preflight, about 40 nested `not` could overflow a
/// 2 MiB debug thread stack and abort a Python or Node host process.
/// Deep expressions must now return a validation failure safely.
/// Run in a child process so the abort does not take the test binary down.
#[test]
fn a_deeply_nested_recipe_fails_validation_without_aborting_the_process() {
    const CHILD: &str = "IRESIPHI_ADVERSARIAL_DEPTH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let mut deep = field("open");
        for _ in 0..60 {
            deep = not(deep);
        }
        let recipe: Recipe =
            big_stack(move || serde_json::from_value(recipe_doc(deep, "many_to_one")).unwrap());
        // 2 MiB is the default stack of a thread spawned by Rust.
        let valid = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || iresiphi::validate(&recipe).valid)
            .unwrap()
            .join()
            .unwrap();
        assert!(
            !valid,
            "deep expression must be rejected before recursive validation"
        );
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_deeply_nested_recipe_fails_validation_without_aborting_the_process",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "validation aborted the process: {:?}\n{}",
        child.status,
        String::from_utf8_lossy(&child.stderr)
    );
}

/// Run `f` on a thread with a large stack, so a deep predicate reaches the
/// placement code instead of overflowing in static validation (see above).
fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Coverage: attacks the implementation withstands.
// ---------------------------------------------------------------------------

/// A small deterministic generator of eligibility predicates over the pushable
/// grammar, with nulls in every column type.
struct Predicates(u64);
impl Predicates {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
    fn operand(&mut self, kind: &str) -> Json {
        let choose = self.next(3);
        match kind {
            "int64" if choose == 0 => field("order_no"),
            "int64" => literal(
                "int64",
                json!(
                    ["-9223372036854775808", "0", "1", "3", "9223372036854775807"]
                        [self.next(5) as usize]
                ),
            ),
            "string" if choose == 0 => field("cust"),
            "string" => literal(
                "string",
                json!(["", "A", "a", "B", "é", "A\u{0}", "Z", "\u{10000}"][self.next(8) as usize]),
            ),
            "date" if choose == 0 => field("booked"),
            "date" => literal(
                "date",
                json!(
                    ["2025-12-31", "2026-01-05", "2026-01-20", "2026-02-08"][self.next(4) as usize]
                ),
            ),
            _ if choose == 0 => field(["open", "flag"][self.next(2) as usize]),
            _ if choose == 1 => field("flag"),
            _ => literal("boolean", json!(self.next(2) == 1)),
        }
    }
    fn condition(&mut self, depth: u32) -> Json {
        let leaf = depth == 0 || self.next(3) == 0;
        if leaf {
            let kind = ["int64", "string", "date", "boolean"][self.next(4) as usize];
            return match self.next(5) {
                0 if kind == "boolean" => self.operand("boolean"),
                1 => is_null(field(
                    ["order_no", "cust", "booked", "open", "flag"][self.next(5) as usize],
                )),
                _ => op(
                    ["equal", "less", "less_equal"][self.next(3) as usize],
                    self.operand(kind),
                    self.operand(kind),
                ),
            };
        }
        match self.next(4) {
            0 => not(self.condition(depth - 1)),
            1 => is_null(self.condition(depth - 1)),
            2 => op("and", self.condition(depth - 1), self.condition(depth - 1)),
            _ => op("or", self.condition(depth - 1), self.condition(depth - 1)),
        }
    }
}

/// Stored values chosen to separate SQLite's semantics from the local executor's:
/// int64 bounds, embedded NUL, text that orders differently under NOCASE or a
/// numeric affinity, astral characters, and nulls in every nullable column.
const EDGE_DDL: &str = "CREATE TABLE orders (order_no INTEGER PRIMARY KEY, cust TEXT COLLATE NOCASE, booked NUMERIC COLLATE RTRIM, net TEXT, open INTEGER, flag NUMERIC)";
const EDGE_ROWS: &str = "INSERT INTO orders VALUES
    (-9223372036854775808,'a','2026-01-05','1.00',1,1),
    (9223372036854775807,'A',NULL,'1.00',0,0),
    (0,'A'||char(0),'2026-01-20','1.00',1,NULL),
    (1,'A'||char(0)||'b','2025-12-31','1.00',0,1),
    (3,NULL,'2026-02-08','1.00',1,0),
    (4,'é','2026-01-05','1.00',0,NULL),
    (5,'',NULL,'1.00',1,1),
    (6,char(65536),'2026-02-08','1.00',1,0),
    (7,'Z ','2026-01-20','1.00',0,1),
    (8,'10','2026-01-05','1.00',1,NULL)";

#[test]
fn generated_predicates_agree_under_local_and_source_placement() {
    let mut generate = Predicates(0x5eed);
    let mut f = fixture(field("open"), &[EDGE_DDL, EDGE_ROWS]);
    let mut pushed = 0;
    for _ in 0..120 {
        let predicate = generate.condition(3);
        f.doc = recipe_doc(predicate.clone(), "many_to_one");
        if !f.pushed() {
            continue;
        }
        pushed += 1;
        let local = f.run_with(Placement::Local);
        let source = f.run_with(Placement::RequireSource);
        assert!(
            local.status.starts_with("succeeded"),
            "{predicate} {:?}",
            local.diagnostics
        );
        assert_eq!(outcome(&source), outcome(&local), "{predicate}");
        let read = &source.manifest["sources"]["orders"];
        assert_eq!(read["placement"]["scan"], "source", "{predicate}");
        assert_eq!(
            read["scanned_rows"].as_u64().unwrap() as usize,
            local.tables["open_orders"].len(),
            "{predicate}: SQLite kept exactly the eligible records"
        );
    }
    assert!(pushed > 80, "only {pushed} predicates were pushed");
}

/// Quotes, spaces, `?` and keywords in table and column names are quoted, not
/// interpreted, in the pushed SQL and the source-side checks.
#[test]
fn hostile_identifiers_are_quoted_in_the_pushed_sql() {
    let mut doc = recipe_doc(
        op(
            "and",
            field("open"),
            op("less", literal("string", json!("A")), field("cust")),
        ),
        "many_to_one",
    );
    let fields = &mut doc["sources"][0]["projection"]["fields"];
    fields["order_no"]["path"] = json!("order\"no");
    fields["cust"]["path"] = json!("cust?");
    fields["open"]["path"] = json!("select");
    let mut f = fixture_doc(
        doc,
        &[
            "CREATE TABLE \"or\"\"ders\" (\"order\"\"no\" INTEGER, \"cust?\" TEXT, booked TEXT, net TEXT, \"select\" INTEGER, flag INTEGER)",
            "INSERT INTO \"or\"\"ders\" VALUES (1,'A','2026-01-05','1.00',1,1),(2,'B','2026-01-05','1.00',1,1),(3,'C','2026-01-05','1.00',0,1)",
            "CREATE VIEW orders AS SELECT 1",
            "CREATE TRIGGER t AFTER INSERT ON \"or\"\"ders\" BEGIN DELETE FROM \"or\"\"ders\"; END",
        ],
    );
    let path = f.bindings["orders"].path.clone();
    f.bindings.insert(
        "orders".into(),
        binding(&path, "sqlite", json!({"table":"or\"ders"})),
    );
    let (_, source) = assert_same_outcome(&mut f, Placement::RequireSource);
    assert_eq!(source.tables["open_orders"].len(), 1);
    assert_eq!(source.manifest["sources"]["orders"]["scanned_rows"], 1);
}

/// A storage-class failure in a record the predicate drops makes the source read
/// the whole table, so the diagnostic is the one local placement gives.
#[test]
fn storage_class_failures_in_dropped_records_fall_back_to_the_local_diagnostic() {
    for (bad, code) in [
        ("(5,'A','2026-01-05','1.00',0,2)", "INVALID_VALUE"), // Boolean 2
        ("(5,'A','2026-01-05','1.00',0,'1')", "INVALID_VALUE"), // Boolean as text
        ("(5,'A','2026-01-05','1.00',0,1.0)", "INVALID_VALUE"), // Boolean as REAL
        ("(5,'A','2026-01-05',1.5,0,1)", "INVALID_VALUE"),    // decimal as REAL
        ("(5,x'41','2026-01-05','1.00',0,1)", "INVALID_VALUE"), // string as BLOB
        ("(5,'A',20260105,'1.00',0,1)", "INVALID_VALUE"),     // date as INTEGER
        ("(5,'A','2026-01-05','1.00',NULL,1)", "NULL_REQUIRED"),
        ("(1,'A','2026-01-05','1.00',0,1)", "DUPLICATE_IDENTITY"),
        ("('5','A','2026-01-05','1.00',0,1)", "INVALID_VALUE"), // int64 as text
    ] {
        // Untyped columns keep each value's own storage class.
        let untyped = "CREATE TABLE orders (order_no, cust, booked, net, open, flag)";
        let insert = format!("INSERT INTO orders VALUES (1,'A','2026-01-05','1.00',1,1),{bad}");
        let mut f = fixture(field("open"), &[untyped, &insert]);
        let (local, source) = assert_same_outcome(&mut f, Placement::RequireSource);
        assert_eq!(local.diagnostics[0].code, code, "{bad}");
        assert_eq!(
            local.diagnostics[0].affected_count, source.diagnostics[0].affected_count,
            "{bad}"
        );
    }
}

/// Text identities that differ only by case or trailing space are distinct to the
/// local executor; a NOCASE or RTRIM column must not make SQLite call them
/// duplicates, or the predicate would be pushed for nothing or not at all.
#[test]
fn identity_uniqueness_at_the_source_is_bytewise() {
    let mut doc = recipe_doc(field("open"), "many_to_one");
    doc["sources"][0]["identity"] = json!(["cust"]);
    doc["sources"][0]["grain"] = json!(["cust"]);
    doc["sources"][0]["projection"]["fields"]["cust"]["nullable"] = json!(false);
    let mut f = fixture_doc(
        doc,
        &[
            "CREATE TABLE orders (order_no INTEGER, cust TEXT COLLATE NOCASE, booked TEXT, net TEXT, open INTEGER, flag INTEGER)",
            "INSERT INTO orders VALUES (1,'A','2026-01-05','1.00',1,1),(2,'a','2026-01-05','1.00',0,1),(3,'A ','2026-01-05','1.00',1,1),(4,'A'||char(0),'2026-01-05','1.00',1,1)",
        ],
    );
    let (_, source) = assert_same_outcome(&mut f, Placement::RequireSource);
    assert_eq!(
        source.manifest["sources"]["orders"]["placement"]["scan"],
        "source"
    );
    assert_eq!(source.manifest["sources"]["orders"]["scanned_rows"], 3);
}

/// Every other use of the anchor source keeps the predicate local.
#[test]
fn other_uses_of_the_anchor_keep_its_filter_local() {
    // A lookup into the anchor from another output's anchor: notes on closed
    // order 2 need it although order 2 is not eligible for open_orders.
    let mut doc = recipe_doc(field("open"), "many_to_one");
    doc["sources"].as_array_mut().unwrap().push(json!(
        {"id":"notes","identity":["note"],"grain":["note"],"projection":{"fields":{
            "note":column("note","string",false),"order_no":column("order_no","int64",false)}}}
    ));
    doc["relationships"].as_array_mut().unwrap().push(json!(
        {"id":"note_order","from":"notes","to":"orders","from_keys":["order_no"],
         "to_keys":["order_no"],"cardinality":"many_to_one"}
    ));
    doc["outputs"].as_array_mut().unwrap().push(json!(
        {"id":"order_notes","anchor":"notes","columns":[named("net",
            json!({"op":"field","source":"orders","field":"net","path":["note_order"]}))]}
    ));
    let mut f = fixture_doc(doc, &[ORDERS_DDL, ORDERS]);
    let notes = f.dir.path().join("notes.csv");
    std::fs::write(&notes, "note,order_no\nN1,1\nN2,2\n").unwrap();
    f.bindings
        .insert("notes".into(), binding(&notes, "csv", json!({})));
    assert!(!f.pushed());
    let (local, _) = assert_same_outcome(&mut f, Placement::PreferSource);
    assert!(
        local.status.starts_with("succeeded"),
        "{:?}",
        local.diagnostics
    );
    assert_eq!(local.tables["order_notes"].len(), 2);
    let mut policy = f.policy.clone();
    policy.placement = Placement::RequireSource;
    let report =
        ExecutionContext::default().run(&f.recipe(), &f.bindings, &policy, &AtomicBool::new(false));
    assert_eq!(report.diagnostics[0].code, "PLACEMENT_INFEASIBLE");
}

/// Predicates the planner must keep local: a decimal anywhere, a computed operand,
/// a path, a null literal and nesting beyond its depth bound.
#[test]
fn predicates_without_identical_semantics_stay_local() {
    big_stack(predicates_without_identical_semantics_stay_local_body);
}
fn predicates_without_identical_semantics_stay_local_body() {
    let decimal =
        json!({"op":"literal","value":{"kind":"decimal","value":"50.00","precision":18,"scale":2}});
    let mut deep = field("open");
    for _ in 0..70 {
        deep = not(deep);
    }
    for predicate in [
        op("less", decimal.clone(), field("net")),
        is_null(field("net")),
        op(
            "equal",
            field("cust"),
            json!({"op":"literal","value":{"kind":"null"}}),
        ),
        op(
            "equal",
            json!({"op":"cast","expr":field("order_no"),"data_type":{"kind":"string"}}),
            literal("string", json!("1")),
        ),
        op(
            "equal",
            json!({"op":"field","source":"customers","field":"SalesArea","path":["order_customer"]}),
            literal("string", json!("North")),
        ),
        json!({"op":"coalesce","values":[field("flag"), field("open")]}),
        deep,
    ] {
        let mut f = fixture(predicate.clone(), &[ORDERS_DDL, ORDERS]);
        assert!(!f.pushed(), "{predicate}");
        assert_same_outcome(&mut f, Placement::PreferSource);
    }
}

/// The deepest predicate the planner pushes still parses in SQLite.
#[test]
fn the_deepest_pushed_predicate_parses_in_sqlite() {
    big_stack(the_deepest_pushed_predicate_parses_in_sqlite_body);
}
fn the_deepest_pushed_predicate_parses_in_sqlite_body() {
    let mut deep = op("less", literal("string", json!("A")), field("cust"));
    // Seventeen unary wrappers plus comparison operands reach depth 18.
    for _ in 0..17 {
        deep = not(deep);
    }
    let mut f = fixture(deep, &[ORDERS_DDL, ORDERS]);
    assert!(f.pushed());
    let (_, source) = assert_same_outcome(&mut f, Placement::RequireSource);
    assert_eq!(
        source.manifest["sources"]["orders"]["placement"]["scan"],
        "source"
    );
}

#[test]
fn leading_slash_is_part_of_a_sqlite_column_name() {
    let mut doc = recipe_doc(field("open"), "many_to_one");
    doc["sources"][0]["projection"]["fields"]["open"]["path"] = json!("/open");
    let mut f = fixture_doc(
        doc,
        &[
            "CREATE TABLE orders (order_no INTEGER, cust TEXT, booked TEXT, net TEXT, \"/open\" INTEGER, flag INTEGER)",
            ORDERS,
        ],
    );
    let (_, source) = assert_same_outcome(&mut f, Placement::RequireSource);
    assert_eq!(source.tables["open_orders"].len(), 2);
    assert_eq!(source.manifest["sources"]["orders"]["scanned_rows"], 2);
}

#[test]
fn infeasible_explain_preserves_the_required_policy_on_each_decision() {
    let predicate = op("less", literal("decimal", json!("50.00")), field("net"));
    // A well-formed decimal literal, so this checks placement rather than syntax.
    let mut doc = recipe_doc(predicate, "many_to_one");
    doc["outputs"][0]["eligibility"]["left"]["value"] =
        json!({"kind":"decimal","value":"50.00","precision":18,"scale":2});
    let mut f = fixture_doc(doc, &[ORDERS_DDL, ORDERS]);
    f.policy.placement = Placement::RequireSource;
    let explained = iresiphi::validation::explain_bound(&f.recipe(), &f.bindings, &f.policy);
    assert_eq!(explained["bound"]["feasible"], false);
    assert_eq!(
        explained["bound"]["diagnostic"]["code"],
        "PLACEMENT_INFEASIBLE"
    );
    assert_eq!(
        explained["bound"]["placement"]["operations"][0]["policy"],
        "require_source"
    );
    assert_eq!(f.policy.placement, Placement::RequireSource);
}

#[test]
fn the_static_depth_boundary_is_safe_on_a_two_mib_stack() {
    let recipe = big_stack(|| {
        let mut expr = field("open");
        for _ in 0..18 {
            expr = not(expr);
        }
        serde_json::from_value::<Recipe>(recipe_doc(expr, "many_to_one")).unwrap()
    });
    let report = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || iresiphi::validate(&recipe))
        .unwrap()
        .join()
        .unwrap();
    assert!(report.valid, "{:?}", report.checks);
}

#[test]
fn feature_references_cannot_bypass_the_static_depth_guard() {
    let mut doc = recipe_doc(json!({"op":"feature","id":"f0"}), "many_to_one");
    doc["features"] =
        json!((0..24).map(|i| json!({"id":format!("f{i}"),"expr":
        if i == 23 {field("open")} else {json!({"op":"feature","id":format!("f{}",i+1)})}
    })).collect::<Vec<_>>());
    let recipe: Recipe = serde_json::from_value(doc).unwrap();
    let report = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || iresiphi::validate(&recipe))
        .unwrap()
        .join()
        .unwrap();
    assert!(!report.valid);
    assert_eq!(
        report.checks[0].diagnostic.as_ref().unwrap().code,
        "EXPRESSION_DEPTH"
    );
}
