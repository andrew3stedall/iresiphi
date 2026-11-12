//! The slice-2 customer/month acceptance fixture (docs/spec/slice-2.md), with the
//! raw field names kept. Shared by population.rs, allocation.rs and the
//! customer/month parity golden.
#![allow(dead_code)]
use iresiphi::{ExecutionContext, RunReport, engine::Table, model::*};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

pub const CUSTOMERS: &str = "CustNo,SalesArea,Share\nA,North,1\nB,North,3\nC,South,1\n";
pub const TRANSACTIONS: &str = "receipt_id,buyer_ref,booked_at,gross\nt1,A,2026-01-05,60\nt2,A,2026-01-20,40\nt3,B,2026-01-11,200\nt4,C,2026-01-17,50\nt5,B,2026-02-08,120\n";
pub const TARGETS: &str = "Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\nSouth,2026-02,400\n";

pub fn string() -> Json {
    json!({"kind":"string"})
}
pub fn decimal(precision: u8, scale: u8) -> Json {
    json!({"kind":"decimal","precision":precision,"scale":scale})
}
pub fn column(path: &str, data_type: Json) -> Json {
    json!({"path":path,"data_type":data_type})
}
pub fn field(source: &str, name: &str) -> Json {
    json!({"op":"field","source":source,"field":name})
}
pub fn via(source: &str, name: &str, path: &[&str]) -> Json {
    json!({"op":"field","source":source,"field":name,"path":path})
}
pub fn text(value: &str) -> Json {
    json!({"op":"literal","value":{"kind":"string","value":value}})
}
pub fn equal(left: Json, right: Json) -> Json {
    json!({"op":"equal","left":left,"right":right})
}
pub fn or(left: Json, right: Json) -> Json {
    json!({"op":"or","left":left,"right":right})
}
pub fn not(expr: Json) -> Json {
    json!({"op":"not","expr":expr})
}
pub fn named(name: &str, expr: Json) -> Json {
    json!({"name":name,"expr":expr})
}

/// Customers, transactions and regional targets as declared sources.
pub fn sources() -> Json {
    json!([
        {"id":"customers","identity":["CustNo"],"grain":["CustNo"],"projection":{"fields":{
            "CustNo":column("CustNo",string()),"SalesArea":column("SalesArea",string()),
            "Share":column("Share",json!({"kind":"int64"}))}}},
        {"id":"transactions","identity":["receipt_id"],"grain":["receipt_id"],"projection":{"fields":{
            "receipt_id":column("receipt_id",string()),"buyer_ref":column("buyer_ref",string()),
            "booked_at":{"path":"booked_at","data_type":{"kind":"date"},"nullable":true},
            "gross":column("gross",decimal(18,2))}}},
        {"id":"targets","identity":["Territory","Period"],"grain":["Territory","Period"],"projection":{"fields":{
            "Territory":column("Territory",string()),"Period":column("Period",string()),
            "Goal":{"path":"Goal","data_type":decimal(18,2),"nullable":true}}}},
    ])
}

/// Every customer in each of two listed calendar months.
pub fn population() -> Json {
    json!({"id":"customer_month","recipients":"customers",
        "period":{"field":"month","calendar":"month","values":["2026-01","2026-02"]}})
}

/// Transactions join a member by buyer and by the calendar month of `booked_at`.
pub fn sales_relationship() -> Json {
    json!({"id":"sales","from":"customer_month","to":"transactions",
        "from_keys":["CustNo"],"to_keys":["buyer_ref"],"cardinality":"one_to_many",
        "period":{"key":"month","date":"booked_at","calendar":"month"}})
}

/// The regional target of a member's sales area and month.
pub fn target_relationship() -> Json {
    json!({"id":"region_target","from":"customer_month","to":"targets",
        "from_keys":["SalesArea","month"],"to_keys":["Territory","Period"],"cardinality":"many_to_one"})
}

pub fn sales_columns() -> Vec<Json> {
    vec![
        named("CustNo", field("customer_month", "CustNo")),
        named("month", field("customer_month", "month")),
        named("currency", text("AUD")),
        named(
            "sales",
            json!({"op":"aggregate","relationship":"sales","function":"sum","field":"gross"}),
        ),
        named(
            "transactions",
            json!({"op":"aggregate","relationship":"sales","function":"count_records"}),
        ),
    ]
}

/// Regional targets allocated to members by `Share`, largest remainder at scale 2.
pub fn allocation() -> Json {
    json!({"name":"allocated_target",
        "quantity":{"source":"targets","field":"Goal"},
        "partition":["Territory","Period"],
        "via":"region_target",
        "weight":field("customer_month","Share"),
        "precision":18,"scale":2,
        "residual":"largest_remainder"})
}

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub doc: Json,
    pub bindings: Bindings,
    pub policy: Policy,
}

fn files(dir: &std::path::Path) -> Bindings {
    let mut bindings = Bindings::new();
    for (id, content) in [
        ("customers", CUSTOMERS),
        ("transactions", TRANSACTIONS),
        ("targets", TARGETS),
    ] {
        let path = dir.join(format!("{id}.csv"));
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
    bindings
}

fn new(doc: Json) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let bindings = files(dir.path());
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
    };
    Fixture {
        dir,
        doc,
        bindings,
        policy,
    }
}

/// Ticket #28: the sales half. Activity is declared complete.
pub fn sales_fixture() -> Fixture {
    new(json!({
        "format_version":"0.1",
        "name":"customer month sales",
        "required_capabilities":["population"],
        "sources":sources(),
        "populations":[population()],
        "relationships":[sales_relationship()],
        "outputs":[{"id":"customer_sales","anchor":"customer_month","complete":["sales"],
            "columns":sales_columns()}],
    }))
}

/// Ticket #29: the full fixture with broadcast target context and allocated targets.
pub fn fixture() -> Fixture {
    let mut columns = sales_columns();
    columns.push(named(
        "target_context",
        via("targets", "Goal", &["region_target"]),
    ));
    new(json!({
        "format_version":"0.1",
        "name":"customer month targets",
        "required_capabilities":["population","allocation"],
        "sources":sources(),
        "populations":[population()],
        "relationships":[sales_relationship(),target_relationship()],
        "outputs":[{"id":"customer_targets","anchor":"customer_month","complete":["sales"],
            "columns":columns,"allocations":[allocation()]}],
    }))
}

impl Fixture {
    pub fn output(&mut self) -> &mut Json {
        &mut self.doc["outputs"][0]
    }
    pub fn allocation(&mut self) -> &mut Json {
        &mut self.doc["outputs"][0]["allocations"][0]
    }
    pub fn write(&self, id: &str, content: &str) {
        std::fs::write(&self.bindings[id].path, content).unwrap();
    }
    /// The recipe as declared, without static validation, so invalid documents
    /// reach `validate` and `run`.
    pub fn recipe(&self) -> Recipe {
        serde_json::from_value(self.doc.clone()).unwrap()
    }
    pub fn run(&self) -> RunReport {
        ExecutionContext::default().run(
            &self.recipe(),
            &self.bindings,
            &self.policy,
            &AtomicBool::new(false),
        )
    }
    pub fn succeeded(&self) -> RunReport {
        let report = self.run();
        assert!(
            report.status.starts_with("succeeded"),
            "{} {:?}",
            report.status,
            report.diagnostics
        );
        report
    }
}

/// The first failing static check's code, or `valid`.
pub fn static_code(doc: &Json) -> String {
    let recipe: Recipe = match serde_json::from_value(doc.clone()) {
        Ok(recipe) => recipe,
        Err(_) => return "INVALID_RECIPE".into(),
    };
    iresiphi::validate(&recipe)
        .checks
        .iter()
        .find_map(|c| c.diagnostic.as_ref().map(|d| d.code.clone()))
        .unwrap_or_else(|| "valid".into())
}

pub fn value_text(v: &Value) -> Option<&str> {
    match v {
        Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Int64 { value }
        | Value::Date { value } => Some(value),
        Value::Null => None,
        other => panic!("expected scalar text, got {other:?}"),
    }
}

/// `column` of the row for customer `customer` in `month`.
pub fn cell<'a>(table: &'a Table, customer: &str, month: &str, column: &str) -> Option<&'a str> {
    let row = table
        .iter()
        .find(|r| {
            value_text(&r["CustNo"]) == Some(customer) && value_text(&r["month"]) == Some(month)
        })
        .unwrap_or_else(|| panic!("no row {customer}/{month} in {table:?}"));
    value_text(&row[column])
}

pub fn failure(report: &RunReport) -> &Error {
    assert_eq!(report.status, "failed", "{:?}", report.diagnostics);
    assert!(report.tables.is_empty(), "failed runs publish no tables");
    &report.diagnostics[0]
}
