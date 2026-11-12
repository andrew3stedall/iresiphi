//! Ticket #21: aggregate and broadcast measures without changing their ownership
//! (decision #7), through the public validate/explain/run seam over real CSV, JSON
//! and XLSX files.
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{
    ExecutionContext, RunReport,
    engine::Table,
    model::*,
    validation::{explain, validate},
};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

// --- Ledger fixture ------------------------------------------------------------
//
// Four invoice lines. L1 and L2 are both billed 10.00 but carry three and one
// observed components; L3 is billed 110.00 with three components; L4 (USD) has no
// components at all. Adjustments are a second, independent detail collection: L1
// has two all-null adjustments, L2 one adjustment of 7.00, L3 and L4 none.

const LINES: &str = "id,customer,currency,month,billed,quantity\nL1,C1,AUD,2026-01,10,1\nL2,C1,AUD,2026-01,10,2\nL3,C2,AUD,2026-03,110,1\nL4,C2,USD,2026-03,5,1\n";
const CUSTOMERS: &str = "id,budget\nC1,1000\nC2,500\n";
const COMPONENTS: &str =
    "id,line,amount\nk1,L1,1\nk2,L1,2\nk3,L1,3\nk4,L2,4\nk5,L3,50\nk6,L3,30\nk7,L3,30\n";
const ADJUSTMENTS: &str = "id,line,amount\nj1,L1,\\N\nj2,L1,\\N\nj3,L2,7\n";

fn string() -> Json {
    json!({"kind":"string"})
}
fn decimal(precision: u8, scale: u8) -> Json {
    json!({"kind":"decimal","precision":precision,"scale":scale})
}
fn column(path: &str, data_type: Json) -> Json {
    json!({"path":path,"data_type":data_type})
}
fn field(source: &str, name: &str) -> Json {
    json!({"op":"field","source":source,"field":name})
}
fn via(source: &str, name: &str, path: &[&str]) -> Json {
    json!({"op":"field","source":source,"field":name,"path":path})
}
fn dec(value: &str, precision: u8, scale: u8) -> Json {
    json!({"op":"literal","value":{"kind":"decimal","value":value,"precision":precision,"scale":scale}})
}
fn text_literal(value: &str) -> Json {
    json!({"op":"literal","value":{"kind":"string","value":value}})
}
/// A per-line aggregate over a one-to-many detail relationship.
fn detail(relationship: &str, function: &str, field: Option<&str>) -> Json {
    json!({"op":"aggregate","relationship":relationship,"function":function,"field":field})
}
/// A grouped aggregate declaration.
fn measure(name: &str, function: &str, expr: Json) -> Json {
    json!({"name":name,"function":function,"expr":expr})
}
fn named(name: &str, expr: Json) -> Json {
    json!({"name":name,"expr":expr})
}

struct Ledger {
    _root: tempfile::TempDir,
    doc: Json,
    bindings: Bindings,
    policy: Policy,
}

fn ledger() -> Ledger {
    let root = tempfile::tempdir().unwrap();
    let files = [
        ("lines", LINES),
        ("customers", CUSTOMERS),
        ("components", COMPONENTS),
        ("adjustments", ADJUSTMENTS),
    ];
    let mut bindings = Bindings::new();
    for (id, content) in files {
        let path = root.path().join(format!("{id}.csv"));
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
    let detail_fields = json!({"id":column("id",string()),"line":column("line",string()),"amount":{"path":"amount","data_type":decimal(18,2),"nullable":true}});
    let doc = json!({
        "format_version":"0.1",
        "name":"ledger",
        "sources":[
            {"id":"lines","identity":["id"],"grain":["id"],"projection":{"fields":{
                "id":column("id",string()),"customer":column("customer",string()),
                "currency":column("currency",string()),"month":column("month",string()),
                "billed":column("billed",decimal(18,2)),"quantity":column("quantity",decimal(12,4))}}},
            {"id":"customers","identity":["id"],"grain":["id"],"projection":{"fields":{
                "id":column("id",string()),"budget":column("budget",decimal(18,2))}}},
            {"id":"components","identity":["id"],"grain":["id"],"projection":{"fields":detail_fields}},
            {"id":"adjustments","identity":["id"],"grain":["id"],"projection":{"fields":detail_fields}},
        ],
        "relationships":[
            {"id":"customer","from":"lines","to":"customers","from_keys":["customer"],"to_keys":["id"],"cardinality":"many_to_one"},
            {"id":"line_components","from":"lines","to":"components","from_keys":["id"],"to_keys":["line"],"cardinality":"one_to_many"},
            {"id":"line_adjustments","from":"lines","to":"adjustments","from_keys":["id"],"to_keys":["line"],"cardinality":"one_to_many"},
        ],
        "outputs":[{"id":"review","anchor":"lines","columns":[
            named("id",field("lines","id")),
            named("customer",field("lines","customer")),
            named("currency",field("lines","currency")),
            named("month",field("lines","month")),
            named("billed",field("lines","billed")),
        ]}],
    });
    let policy = Policy {
        allowed_read_roots: vec![root.path().into()],
        allowed_output_roots: vec![root.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
    };
    Ledger {
        _root: root,
        doc,
        bindings,
        policy,
    }
}

impl Ledger {
    fn output(&mut self) -> &mut Json {
        &mut self.doc["outputs"][0]
    }
    fn add_column(&mut self, name: &str, expr: Json) {
        self.output()["columns"]
            .as_array_mut()
            .unwrap()
            .push(named(name, expr));
    }
    fn group(&mut self, keys: &[&str], measures: Vec<Json>) {
        self.output()["group_by"] = json!(keys);
        self.output()["aggregates"] = json!(measures);
    }
    fn write(&self, id: &str, content: &str) {
        std::fs::write(&self.bindings[id].path, content).unwrap();
    }
    /// The recipe as declared, without static validation (so invalid documents
    /// reach `validate` and `run`).
    fn recipe(&self) -> Recipe {
        serde_json::from_value(self.doc.clone()).unwrap()
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
}

fn static_code(recipe: &Recipe) -> String {
    validate(recipe)
        .checks
        .iter()
        .find_map(|c| c.diagnostic.as_ref().map(|d| d.code.clone()))
        .unwrap_or_else(|| "valid".into())
}
fn text(v: &Value) -> &str {
    match v {
        Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Int64 { value }
        | Value::Date { value } => value,
        other => panic!("expected scalar text, got {other:?}"),
    }
}
/// The row of `table` whose `key` columns hold `values`.
fn row<'a>(table: &'a Table, keys: &[(&str, &str)]) -> &'a BTreeMap<String, Value> {
    table
        .iter()
        .find(|r| keys.iter().all(|(k, v)| r.get(*k).map(text) == Some(*v)))
        .unwrap_or_else(|| panic!("no row {keys:?} in {table:?}"))
}
fn failure(report: &RunReport) -> &str {
    assert_eq!(report.status, "failed", "{:?}", report.diagnostics);
    assert!(report.tables.is_empty(), "failed runs publish no tables");
    &report.diagnostics[0].code
}

// --- Supported aggregates at explicit grouping keys ------------------------------

#[test]
fn grouped_counts_sums_and_extremes_execute_at_declared_keys() {
    let mut l = ledger();
    let billed = field("lines", "billed");
    l.group(
        &["currency"],
        vec![
            measure("lines", "count_records", billed.clone()),
            measure("billed_lines", "count_non_null", billed.clone()),
            measure("total", "sum", billed.clone()),
            measure("lowest", "min", billed.clone()),
            measure("highest", "max", billed),
        ],
    );
    assert_eq!(static_code(&l.recipe()), "valid");
    let report = l.succeeded();
    let table = &report.tables["review"];
    assert_eq!(table.len(), 2, "one row per present currency: {table:?}");
    let aud = row(table, &[("currency", "AUD")]);
    assert_eq!(text(&aud["lines"]), "3");
    assert_eq!(text(&aud["billed_lines"]), "3");
    assert_eq!(text(&aud["lowest"]), "10.00");
    assert_eq!(text(&aud["highest"]), "110.00");
    // Decimal sum keeps the input scale and accumulates at precision 38.
    assert_eq!(
        aud["total"],
        Value::Decimal {
            value: "130.00".into(),
            precision: 38,
            scale: 2
        }
    );
    let usd = row(table, &[("currency", "USD")]);
    assert_eq!(text(&usd["total"]), "5.00");
    assert_eq!(text(&usd["lines"]), "1");
    // Grouped rows carry exactly the keys, the measures and a status.
    let mut columns: Vec<_> = aud.keys().map(String::as_str).collect();
    columns.sort_unstable();
    assert_eq!(
        columns,
        [
            "_status",
            "billed_lines",
            "currency",
            "highest",
            "lines",
            "lowest",
            "total"
        ]
    );
}

#[test]
fn equal_billed_lines_with_unequal_component_counts_neither_fan_out_nor_deduplicate() {
    let mut l = ledger();
    l.add_column(
        "components",
        detail("line_components", "count_records", None),
    );
    l.add_column(
        "component_total",
        detail("line_components", "sum", Some("amount")),
    );
    let out = |name: &str| field("review", name);
    l.group(
        &["currency", "month"],
        vec![
            measure("billed_sum", "sum", field("lines", "billed")),
            measure("lines", "count_records", field("lines", "id")),
            measure("component_count", "sum", out("components")),
            measure("component_amount", "sum", out("component_total")),
        ],
    );
    let report = l.succeeded();
    let table = &report.tables["review"];
    // L1 and L2 are distinct lines each billed 10.00: 20.00, never 10.00 (DISTINCT)
    // and never 40.00 (one copy per component).
    let january = row(table, &[("currency", "AUD"), ("month", "2026-01")]);
    assert_eq!(text(&january["billed_sum"]), "20.00");
    assert_eq!(text(&january["lines"]), "2");
    assert_eq!(text(&january["component_count"]), "4");
    assert_eq!(text(&january["component_amount"]), "10.00");
    // Three components beneath the billed-110 line leave billed at 110.00; their
    // own total is a separate measure, not presumed equal to the billed charge.
    let march = row(table, &[("currency", "AUD"), ("month", "2026-03")]);
    assert_eq!(text(&march["billed_sum"]), "110.00");
    assert_eq!(text(&march["lines"]), "1");
    assert_eq!(text(&march["component_count"]), "3");
    assert_eq!(text(&march["component_amount"]), "110.00");
    // Absent groups are never manufactured: no February row, no USD January row.
    assert_eq!(table.len(), 3, "{table:?}");
    assert!(!table.iter().any(|r| text(&r["month"]) == "2026-02"));
}

#[test]
fn independent_detail_collections_aggregate_separately_before_combining() {
    let mut l = ledger();
    for (name, relationship, function, field) in [
        ("components", "line_components", "count_records", None),
        ("component_total", "line_components", "sum", Some("amount")),
        ("adjustments", "line_adjustments", "count_records", None),
        (
            "adjustment_values",
            "line_adjustments",
            "count_non_null",
            Some("amount"),
        ),
        (
            "adjustment_total",
            "line_adjustments",
            "sum",
            Some("amount"),
        ),
    ] {
        l.add_column(name, detail(relationship, function, field));
    }
    let report = l.succeeded();
    let table = &report.tables["review"];
    assert_eq!(table.len(), 4, "one row per line; details never add lines");
    let l1 = row(table, &[("id", "L1")]);
    // A join of L1's three components with its two adjustments would give six rows
    // and a component total of 12.00; each collection is reduced on its own.
    assert_eq!(text(&l1["components"]), "3");
    assert_eq!(text(&l1["component_total"]), "6.00");
    assert_eq!(text(&l1["adjustments"]), "2");
    assert_eq!(text(&l1["billed"]), "10.00");
    let l2 = row(table, &[("id", "L2")]);
    assert_eq!(text(&l2["components"]), "1");
    assert_eq!(text(&l2["adjustments"]), "1");
    assert_eq!(text(&l2["adjustment_total"]), "7.00");
    assert_eq!(text(&l2["adjustment_values"]), "1");

    // The same holds at a coarser grain: per-line measures are summed per group.
    let out = |name: &str| field("review", name);
    l.group(
        &["currency"],
        vec![
            measure("component_count", "sum", out("components")),
            measure("adjustment_count", "sum", out("adjustments")),
            measure("billed_sum", "sum", field("lines", "billed")),
        ],
    );
    let report = l.succeeded();
    let aud = row(&report.tables["review"], &[("currency", "AUD")]);
    assert_eq!(text(&aud["component_count"]), "7");
    assert_eq!(text(&aud["adjustment_count"]), "3");
    assert_eq!(text(&aud["billed_sum"]), "130.00");
}

// --- Empty and null inputs -------------------------------------------------------

#[test]
fn empty_detail_sets_and_all_null_observations_follow_decision7() {
    let mut l = ledger();
    for function in ["count_records", "count_non_null", "sum", "min", "max"] {
        for (prefix, relationship) in [
            ("component", "line_components"),
            ("adjustment", "line_adjustments"),
        ] {
            l.add_column(
                &format!("{prefix}_{function}"),
                detail(relationship, function, Some("amount")),
            );
        }
    }
    let report = l.succeeded();
    let table = &report.tables["review"];
    // L4 exists with zero related observations: counts are zero, the rest null.
    let l4 = row(table, &[("id", "L4")]);
    assert_eq!(text(&l4["component_count_records"]), "0");
    assert_eq!(text(&l4["component_count_non_null"]), "0");
    for function in ["sum", "min", "max"] {
        assert_eq!(
            l4[&format!("component_{function}")],
            Value::Null,
            "{function}"
        );
    }
    // L1's two adjustments are all null: sum/min/max are null, count_non_null 0.
    let l1 = row(table, &[("id", "L1")]);
    assert_eq!(text(&l1["adjustment_count_records"]), "2");
    assert_eq!(text(&l1["adjustment_count_non_null"]), "0");
    for function in ["sum", "min", "max"] {
        assert_eq!(
            l1[&format!("adjustment_{function}")],
            Value::Null,
            "{function}"
        );
    }
    assert_eq!(text(&l1["component_min"]), "1.00");
    assert_eq!(text(&l1["component_max"]), "3.00");

    // At group grain: a group whose inputs are all null has null sum/min/max and a
    // zero non-null count; a sum that skipped a null input is never labelled complete.
    let out = field("review", "adjustment_sum");
    l.group(
        &["currency", "month"],
        vec![
            measure("adjusted", "sum", out.clone()),
            measure("lowest", "min", out.clone()),
            measure("highest", "max", out.clone()),
            measure("observed", "count_non_null", out.clone()),
            measure("lines", "count_records", out),
        ],
    );
    let report = l.succeeded();
    let table = &report.tables["review"];
    let january = row(table, &[("currency", "AUD"), ("month", "2026-01")]);
    assert_eq!(text(&january["adjusted"]), "7.00");
    assert_eq!(text(&january["observed"]), "1");
    assert_eq!(text(&january["lines"]), "2");
    assert_eq!(text(&january["_status"]), "incomplete");
    let march = row(table, &[("currency", "AUD"), ("month", "2026-03")]);
    for name in ["adjusted", "lowest", "highest"] {
        assert_eq!(march[name], Value::Null, "{name}");
    }
    assert_eq!(text(&march["observed"]), "0");
    assert_eq!(text(&march["lines"]), "1");
    assert_ne!(text(&march["_status"]), "matched");
    assert!(
        report.manifest["counts"]["review"]["aggregate_null_inputs"]["adjusted"]
            .as_u64()
            .unwrap()
            >= 2
    );
}

#[test]
fn empty_anchor_population_succeeds_with_zero_counts_and_no_manufactured_groups() {
    let mut l = ledger();
    l.group(
        &["currency"],
        vec![
            measure("lines", "count_records", field("lines", "id")),
            measure("total", "sum", field("lines", "billed")),
        ],
    );
    l.output()["eligibility"] = json!({"op":"literal","value":{"kind":"boolean","value":false}});
    let report = l.run();
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    assert!(
        report.tables["review"].is_empty(),
        "no group is manufactured"
    );
    let counts = &report.manifest["counts"]["review"];
    assert_eq!(counts["source_rows"], 4);
    assert_eq!(counts["eligible_rows"], 0);
    assert_eq!(counts["output_rows"], 0);
    assert_eq!(counts["group_rows"], 0);
    assert_eq!(counts["resolved_rows"], 0);
    assert_eq!(counts["unresolved_rows"], 0);
    assert_eq!(report.manifest["reconciliation"]["status"], "empty");
}

// --- Input versus final filters --------------------------------------------------

#[test]
fn aggregate_input_predicates_apply_before_grouping_and_final_filters_after() {
    let mut l = ledger();
    let billed = field("lines", "billed");
    let large = json!({"op":"less_equal","left":dec("50",18,2),"right":billed});
    let mut big = measure("large", "sum", billed.clone());
    big["filter"] = large.clone();
    let mut big_lines = measure("large_lines", "count_records", billed.clone());
    big_lines["filter"] = large.clone();
    l.group(
        &["currency"],
        vec![
            measure("total", "sum", billed.clone()),
            measure("lines", "count_records", billed.clone()),
            big,
            big_lines,
        ],
    );
    // Input predicates exclude observations from one measure only; the group and
    // its other measures keep the whole population.
    let report = l.succeeded();
    let table = &report.tables["review"];
    let aud = row(table, &[("currency", "AUD")]);
    assert_eq!(text(&aud["total"]), "130.00");
    assert_eq!(text(&aud["lines"]), "3");
    assert_eq!(text(&aud["large"]), "110.00");
    assert_eq!(text(&aud["large_lines"]), "1");
    // An existing group whose observations are all excluded keeps its row: count 0,
    // sum null.
    let usd = row(table, &[("currency", "USD")]);
    assert_eq!(usd["large"], Value::Null);
    assert_eq!(text(&usd["large_lines"]), "0");
    assert_eq!(text(&usd["total"]), "5.00");

    // The final predicate selects group rows after grouping and changes no sum.
    l.output()["filter"] =
        json!({"op":"less","left":dec("100",18,2),"right":field("review","total")});
    let report = l.succeeded();
    let table = &report.tables["review"];
    assert_eq!(table.len(), 1);
    assert_eq!(text(&table[0]["currency"]), "AUD");
    assert_eq!(text(&table[0]["total"]), "130.00");
    assert_eq!(report.manifest["counts"]["review"]["group_rows"], 2);
    assert_eq!(report.manifest["counts"]["review"]["output_rows"], 1);

    // Population eligibility, by contrast, changes the population before grouping.
    l.output()["filter"] = Json::Null;
    l.output()["eligibility"] = large;
    let report = l.succeeded();
    let table = &report.tables["review"];
    assert_eq!(table.len(), 1, "USD has no eligible line, so no group");
    assert_eq!(text(&table[0]["total"]), "110.00");

    // A final filter over a line field after grouping has no single value.
    l.output()["eligibility"] = Json::Null;
    l.output()["filter"] =
        json!({"op":"less","left":dec("100",18,2),"right":field("lines","billed")});
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");
}

#[test]
fn detail_input_predicates_apply_before_reduction() {
    let mut l = ledger();
    let mut large = detail("line_components", "sum", Some("amount"));
    large["filter"] =
        json!({"op":"less_equal","left":dec("30",18,2),"right":field("components","amount")});
    l.add_column("large_components", large.clone());
    large["function"] = json!("count_records");
    l.add_column("large_component_count", large);
    let report = l.succeeded();
    let table = &report.tables["review"];
    let l3 = row(table, &[("id", "L3")]);
    assert_eq!(text(&l3["large_components"]), "110.00");
    assert_eq!(text(&l3["large_component_count"]), "3");
    let l1 = row(table, &[("id", "L1")]);
    assert_eq!(l1["large_components"], Value::Null);
    assert_eq!(text(&l1["large_component_count"]), "0");
    // Detail predicates read the detail record only; line fields need a column.
    let mut wrong = detail("line_components", "sum", Some("amount"));
    wrong["filter"] =
        json!({"op":"less_equal","left":dec("30",18,2),"right":field("lines","billed")});
    l.add_column("wrong", wrong);
    assert_eq!(static_code(&l.recipe()), "INVALID_PATH");
}

#[test]
fn ungrouped_final_filters_select_lines_by_their_own_columns() {
    let mut l = ledger();
    l.add_column(
        "components",
        detail("line_components", "count_records", None),
    );
    l.output()["filter"] = json!({"op":"less","left":{"op":"literal","value":{"kind":"int64","value":"1"}},"right":field("review","components")});
    let report = l.succeeded();
    let ids: Vec<_> = report.tables["review"]
        .iter()
        .map(|r| text(&r["id"]).to_owned())
        .collect();
    assert_eq!(ids, ["L1", "L3"]);
    assert_eq!(report.manifest["counts"]["review"]["output_rows"], 2);
    assert_eq!(report.manifest["counts"]["review"]["eligible_rows"], 4);
}

// --- Currency --------------------------------------------------------------------

#[test]
fn grouping_must_retain_currency_and_mixed_currency_detail_sums_fail() {
    let mut l = ledger();
    for function in ["sum", "min", "max", "count_records"] {
        l.group(
            &["month"],
            vec![measure("m", function, field("lines", "billed"))],
        );
        assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION", "{function}");
        let report = l.run();
        assert_eq!(failure(&report), "GRAIN_VIOLATION");
    }
    l.group(
        &["currency", "month"],
        vec![measure("total", "sum", field("lines", "billed"))],
    );
    assert_eq!(static_code(&l.recipe()), "valid");

    // Component amounts in a currency other than their line's cannot be summed into it.
    let mut l = ledger();
    l.doc["sources"][2]["projection"]["fields"]["currency"] = column("currency", string());
    l.write(
        "components",
        "id,line,amount,currency\nk1,L1,1,AUD\nk2,L1,2,USD\nk4,L2,4,AUD\n",
    );
    l.add_column(
        "component_count",
        detail("line_components", "count_records", None),
    );
    assert_eq!(
        text(&l.succeeded().tables["review"][0]["component_count"]),
        "2"
    );
    l.add_column(
        "component_total",
        detail("line_components", "sum", Some("amount")),
    );
    let report = l.run();
    assert_eq!(failure(&report), "MIXED_CURRENCY");
    assert_eq!(
        report.diagnostics[0].subject.as_deref(),
        Some("relationship:line_components")
    );
    // Same-currency components sum normally.
    l.write(
        "components",
        "id,line,amount,currency\nk1,L1,1,AUD\nk2,L1,2,AUD\nk4,L2,4,AUD\n",
    );
    let report = l.succeeded();
    assert_eq!(
        text(&row(&report.tables["review"], &[("id", "L1")])["component_total"]),
        "3.00"
    );
}

// --- Decimal precision and overflow ----------------------------------------------

#[test]
fn decimal_sum_overflow_and_integer_overflow_fail_the_run() {
    let mut l = ledger();
    let nines = "9".repeat(36);
    l.doc["sources"][0]["projection"]["fields"]["billed"]["data_type"] = decimal(38, 2);
    l.write(
        "lines",
        &format!("id,customer,currency,month,billed,quantity\nL1,C1,AUD,2026-01,{nines},1\nL2,C1,AUD,2026-01,{nines},1\n"),
    );
    l.group(
        &["currency"],
        vec![measure("total", "sum", field("lines", "billed"))],
    );
    assert_eq!(failure(&l.run()), "ARITHMETIC_OVERFLOW");
    // A single value at the limit is representable and keeps its scale.
    l.write(
        "lines",
        &format!("id,customer,currency,month,billed,quantity\nL1,C1,AUD,2026-01,{nines},1\n"),
    );
    let report = l.succeeded();
    assert_eq!(
        text(&report.tables["review"][0]["total"]),
        format!("{nines}.00")
    );

    // Checked Int64 accumulation: count-derived integers overflow loudly.
    let mut l = ledger();
    l.doc["sources"][0]["projection"]["fields"]["units"] = column("units", json!({"kind":"int64"}));
    l.write(
        "lines",
        "id,customer,currency,month,billed,quantity,units\nL1,C1,AUD,2026-01,1,1,9223372036854775807\nL2,C1,AUD,2026-01,1,1,1\n",
    );
    l.group(
        &["currency"],
        vec![measure("units", "sum", field("lines", "units"))],
    );
    assert_eq!(failure(&l.run()), "ARITHMETIC_OVERFLOW");
}

#[test]
fn arithmetic_beyond_precision_38_needs_an_explicit_checked_narrowing_cast() {
    let mut l = ledger();
    let total = detail("line_components", "sum", Some("amount"));
    l.add_column(
        "gross",
        json!({"op":"add","left":total.clone(),"right":field("lines","billed")}),
    );
    // decimal(38,2) + decimal(18,2) would need precision 39.
    assert_eq!(static_code(&l.recipe()), "PRECISION_OVERFLOW");
    let narrowed = json!({"op":"cast","expr":total.clone(),"data_type":decimal(20,2)});
    l.output()["columns"].as_array_mut().unwrap().pop();
    l.add_column(
        "gross",
        json!({"op":"add","left":narrowed,"right":field("lines","billed")}),
    );
    let report = l.succeeded();
    assert_eq!(
        text(&row(&report.tables["review"], &[("id", "L1")])["gross"]),
        "16.00"
    );
    // The narrowing cast is checked: a component total beyond decimal(3,2) fails.
    l.add_column(
        "tight",
        json!({"op":"cast","expr":total,"data_type":decimal(3,2)}),
    );
    assert_eq!(failure(&l.run()), "ARITHMETIC_OVERFLOW");

    // The same rule applies to grouped measures in final predicates.
    let mut l = ledger();
    l.group(
        &["currency"],
        vec![measure("total", "sum", field("lines", "billed"))],
    );
    let plus = |left: Json| json!({"op":"less","left":dec("100",18,2),"right":{"op":"add","left":left,"right":dec("1",18,2)}});
    l.output()["filter"] = plus(field("review", "total"));
    assert_eq!(static_code(&l.recipe()), "PRECISION_OVERFLOW");
    l.output()["filter"] =
        plus(json!({"op":"cast","expr":field("review","total"),"data_type":decimal(20,2)}));
    let report = l.succeeded();
    assert_eq!(report.tables["review"].len(), 1);
}

// --- Broadcast -------------------------------------------------------------------

#[test]
fn broadcast_context_cannot_be_reaggregated_at_recipient_grain() {
    let mut l = ledger();
    let budget = via("customers", "budget", &["customer"]);
    // As context on each line, a validated to-one broadcast is allowed.
    l.add_column("budget", budget.clone());
    let report = l.succeeded();
    assert_eq!(
        text(&row(&report.tables["review"], &[("id", "L1")])["budget"]),
        "1000.00"
    );

    // A customer budget repeated on lines is not one budget per line.
    for (function, expr) in [
        ("sum", budget.clone()),
        ("sum", field("review", "budget")),
        (
            "sum",
            json!({"op":"add","left":field("lines","billed"),"right":budget.clone()}),
        ),
        (
            "sum",
            json!({"op":"coalesce","values":[budget.clone(),dec("0",18,2)]}),
        ),
        ("min", budget.clone()),
        ("max", budget.clone()),
    ] {
        l.group(
            &["currency"],
            vec![measure("wrong", function, expr.clone())],
        );
        assert_eq!(
            static_code(&l.recipe()),
            "GRAIN_VIOLATION",
            "{function} {expr}"
        );
        assert_eq!(failure(&l.run()), "GRAIN_VIOLATION");
    }
    // A recipient-owned measure scaled by broadcast context is owned by the line.
    l.group(
        &["currency"],
        vec![
            measure(
                "scaled",
                "sum",
                json!({"op":"multiply","left":field("lines","quantity"),"right":budget.clone()}),
            ),
            measure("lines_with_budget", "count_non_null", budget),
        ],
    );
    assert_eq!(static_code(&l.recipe()), "valid");
    let report = l.succeeded();
    let aud = row(&report.tables["review"], &[("currency", "AUD")]);
    assert_eq!(text(&aud["scaled"]), "3500.000000");
    assert_eq!(text(&aud["lines_with_budget"]), "3");

    // A per-line aggregate over a to-one lookup would reduce its candidates.
    let mut l = ledger();
    l.add_column("budgets", detail("customer", "sum", Some("budget")));
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");
}

#[test]
fn named_context_reduction_requires_equal_values_in_every_group() {
    let mut l = ledger();
    let budget = via("customers", "budget", &["customer"]);
    let mut context = measure("budget", "max", budget.clone());
    context["context"] = json!(true);
    l.group(
        &["customer", "currency"],
        vec![
            context.clone(),
            measure("billed_sum", "sum", field("lines", "billed")),
        ],
    );
    assert_eq!(static_code(&l.recipe()), "valid");
    let report = l.succeeded();
    let table = &report.tables["review"];
    assert_eq!(table.len(), 3);
    let c1 = row(table, &[("customer", "C1"), ("currency", "AUD")]);
    assert_eq!(text(&c1["budget"]), "1000.00");
    assert_eq!(text(&c1["billed_sum"]), "20.00");
    assert_eq!(
        text(&row(table, &[("customer", "C2"), ("currency", "USD")])["budget"]),
        "500.00"
    );

    // Grouped by currency alone, AUD mixes two customers' budgets: not one context.
    l.group(&["currency"], vec![context.clone()]);
    let report = l.run();
    assert_eq!(failure(&report), "CONTEXT_NOT_CONSTANT");
    let diagnostic = &report.diagnostics[0];
    assert_eq!(
        diagnostic.subject.as_deref(),
        Some("output:review/aggregate:budget")
    );
    assert_eq!(diagnostic.affected_count, Some(3));
    assert!(diagnostic.values.is_none(), "values are omitted by default");

    // A context reduction names min or max only.
    let mut summed = context;
    summed["function"] = json!("sum");
    l.group(&["customer", "currency"], vec![summed]);
    assert_eq!(static_code(&l.recipe()), "UNSUPPORTED_OPERATION");
}

#[test]
fn nested_component_scalars_stay_invalid_without_explicit_aggregation() {
    let component = via("components", "amount", &["line_components"]);
    let mut l = ledger();
    l.add_column("component", component.clone());
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");

    let mut l = ledger();
    l.group(&["currency"], vec![measure("c", "sum", component.clone())]);
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");

    let mut l = ledger();
    let mut filtered = measure("c", "sum", field("lines", "billed"));
    filtered["filter"] = json!({"op":"is_null","expr":component.clone()});
    l.group(&["currency"], vec![filtered]);
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");

    let mut l = ledger();
    l.output()["filter"] = json!({"op":"is_null","expr":component});
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");
}

#[test]
fn duplicate_detail_identity_fails_instead_of_deduplicating_amounts() {
    let l = ledger();
    l.write("components", "id,line,amount\nk1,L1,1\nk1,L1,1\n");
    let report = l.run();
    assert_eq!(failure(&report), "DUPLICATE_IDENTITY");
    assert_eq!(
        report.diagnostics[0].subject.as_deref(),
        Some("source:components")
    );
}

// --- Unsupported operations ------------------------------------------------------

#[test]
fn deferred_aggregations_and_allocation_fail_validation_without_inventing_shares() {
    for function in [
        "average",
        "avg",
        "mean",
        "distinct_count",
        "count_distinct",
        "window_sum",
        "running_sum",
        "last",
        "first",
        "median",
        "custom",
        "allocate",
        "allocation",
    ] {
        let mut l = ledger();
        l.group(
            &["currency"],
            vec![measure("m", function, field("lines", "billed"))],
        );
        assert_eq!(
            static_code(&l.recipe()),
            "UNSUPPORTED_OPERATION",
            "{function}"
        );
        let report = l.run();
        assert_eq!(failure(&report), "UNSUPPORTED_OPERATION", "{function}");

        let mut l = ledger();
        l.add_column("m", detail("line_components", function, Some("amount")));
        assert_eq!(
            static_code(&l.recipe()),
            "UNSUPPORTED_OPERATION",
            "{function}"
        );
        assert_eq!(failure(&l.run()), "UNSUPPORTED_OPERATION", "{function}");
    }
    // An allocation of the invoice total across components by weight is deferred:
    // the document fails to load instead of producing invented shares.
    for op in ["allocate", "allocation", "window"] {
        let mut l = ledger();
        l.add_column(
            "share",
            json!({"op":op,"measure":field("lines","billed"),"relationship":"line_components","weights":"amount"}),
        );
        let error = Recipe::from_json(&l.doc.to_string()).unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_OPERATION", "{op}");
    }
}

// --- Explain and schema ----------------------------------------------------------

#[test]
fn explain_and_output_schema_expose_grain_transitions_and_measure_origins() {
    let mut l = ledger();
    l.add_column("budget", via("customers", "budget", &["customer"]));
    let explained = explain(&l.recipe());
    let node = |explained: &Json, id: &str| {
        explained["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("{id}"))
            .clone()
    };
    // A broadcast column names the to-one path and the grain it originates at.
    let budget = node(&explained, "output:review/column:budget");
    let origin = &budget["measure_origins"][0];
    assert_eq!(origin["source"], "source:customers");
    assert_eq!(origin["path"], json!(["customer"]));
    assert_eq!(origin["origin_grain"], json!(["id"]));
    assert_eq!(origin["transition"], "broadcast");
    assert!(
        budget["grain_transitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["operation"] == "broadcast"
                && t["from"] == "source:customers"
                && t["to"] == "source:lines"),
        "{budget}"
    );
    let schema = &explained["output_schema"]["review"];
    let budget_column = schema["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "budget")
        .unwrap();
    assert_eq!(budget_column["ownership"], "broadcast");
    assert_eq!(budget_column["origins"][0]["origin_grain"], json!(["id"]));
    assert_eq!(
        node(&explained, "output:review/column:billed")["measure_origins"][0]["transition"],
        "owned"
    );

    // Grouped: grain, stage order, measure origins and aggregate input predicates.
    let billed = field("lines", "billed");
    let mut large = measure("large", "sum", billed.clone());
    large["filter"] = json!({"op":"less_equal","left":dec("50",18,2),"right":billed.clone()});
    l.group(&["currency"], vec![measure("total", "sum", billed), large]);
    l.output()["filter"] =
        json!({"op":"less","left":dec("100",18,2),"right":field("review","total")});
    let explained = explain(&l.recipe());
    assert_eq!(
        explained["validation"]["valid"], true,
        "{}",
        explained["validation"]
    );
    let output = node(&explained, "output:review");
    assert_eq!(output["grain"], json!(["currency"]));
    assert_eq!(output["anchor_grain"], json!(["id"]));
    let filter = node(&explained, "output:review/filter");
    assert_eq!(filter["applies"], "after_grouping");
    let aggregate = node(&explained, "output:review/aggregate:large");
    assert_eq!(aggregate["input_filter"]["applies"], "before_grouping");
    assert_eq!(
        aggregate["measure_origins"][0]["origin_grain"],
        json!(["id"])
    );
    let schema = &explained["output_schema"]["review"];
    assert_eq!(schema["grain"], json!(["currency"]));
    let names: Vec<_> = schema["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["currency", "total", "large", "_status"]);
    let total = &schema["columns"][1];
    assert_eq!(total["type"], decimal(38, 2));
    assert_eq!(total["role"], "aggregate");
    assert_eq!(total["function"], "sum");
    assert_eq!(total["origins"][0]["source"], "source:lines");
    assert_eq!(total["origins"][0]["origin_grain"], json!(["id"]));
    // Unpublished line columns are inputs, not output columns.
    assert!(
        schema["line_inputs"]
            .as_array()
            .unwrap()
            .contains(&json!("budget"))
    );
    // Runs carry the same schema in their manifest.
    let report = l.succeeded();
    assert_eq!(report.manifest["output_schema"]["review"], *schema);
}

// --- Invoice comparison summary --------------------------------------------------

#[test]
fn comparison_summary_selects_resolved_lines_explicitly_and_never_claims_completeness() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = invoice::fixture(root.path());
    for id in ["a", "b"] {
        let summary = format!("{id}_summary");
        let out = |name: &str| field(&summary, name);
        let resolved = json!({"op":"equal","left":out("_status"),"right":text_literal("matched")});
        let unresolved = json!({"op":"not","expr":resolved.clone()});
        let filtered = |name: &str, function: &str, expr: Json, filter: &Json| {
            let mut m = measure(name, function, expr);
            m["filter"] = filter.clone();
            m
        };
        let rate = format!("{id}_rate");
        let expected = json!({"op":"quantize","expr":{"op":"multiply","left":field(id,"quantity"),"right":via("rates","rate",&[rate.as_str()])},"precision":18,"scale":2,"rounding":"half_even"});
        let output = json!({
            "id":summary,"anchor":id,
            "columns":[
                named("currency",field(id,"currency")),
                named("billed",field(id,"billed")),
                named("expected",expected.clone()),
                named("variance",json!({"op":"subtract","left":field(id,"billed"),"right":expected})),
                named("components",detail(&format!("{id}_components"),"sum",Some("amount"))),
            ],
            "group_by":["currency"],
            "aggregates":[
                measure("lines","count_records",out("billed")),
                filtered("resolved_lines","count_records",out("billed"),&resolved),
                filtered("unresolved_lines","count_records",out("billed"),&unresolved),
                measure("billed_total","sum",out("billed")),
                filtered("billed_comparable","sum",out("billed"),&resolved),
                filtered("expected_comparable","sum",out("expected"),&resolved),
                filtered("variance_comparable","sum",out("variance"),&resolved),
                filtered("billed_unresolved","sum",out("billed"),&unresolved),
                measure("expected_ignoring_nulls","sum",out("expected")),
                measure("component_total","sum",out("components")),
            ],
        });
        recipe.outputs.push(serde_json::from_value(output).unwrap());
    }
    assert_eq!(static_code(&recipe), "valid");
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let a = &report.tables["a_summary"][0];
    let b = &report.tables["b_summary"][0];
    let cents = |name: &str| -> i64 {
        [a, b]
            .iter()
            .map(|r| match &r[name] {
                Value::Null => 0,
                v => text(v).replace('.', "").parse::<i64>().unwrap(),
            })
            .sum()
    };
    assert_eq!(cents("billed_total"), 18000);
    assert_eq!(cents("billed_comparable"), 16000);
    assert_eq!(cents("expected_comparable"), 14500);
    assert_eq!(cents("variance_comparable"), 1500);
    assert_eq!(cents("billed_unresolved"), 2000);
    assert_eq!(cents("lines"), 3);
    assert_eq!(cents("resolved_lines"), 2);
    assert_eq!(cents("unresolved_lines"), 1);
    assert_eq!(text(&a["billed_comparable"]), "110.00");
    assert_eq!(text(&b["variance_comparable"]), "5.00");
    // A sum that skipped the unresolved line's null expected value is never labelled
    // complete: its group carries the unresolved status.
    assert_eq!(text(&a["expected_ignoring_nulls"]), "100.00");
    assert_eq!(text(&a["_status"]), "unresolved");
    assert_eq!(text(&b["_status"]), "matched");
    assert_eq!(b["billed_unresolved"], Value::Null);
    // Observed component totals are their own measure, unrelated to billed.
    assert_eq!(text(&a["component_total"]), "6.00");
    assert_eq!(text(&b["component_total"]), "4.00");
    // The run-level summary is unchanged by the extra grouped outputs: lines are
    // counted once, not once per output that reads them.
    let manifest = &report.manifest;
    assert_eq!(manifest["reconciliation"]["status"], "partial");
    assert_eq!(manifest["resolved_rows"], 2);
    assert_eq!(manifest["unresolved_rows"], 1);
    let combined = &manifest["totals"]["combined"]["AUD"];
    assert_eq!(combined["columns"]["billed"]["population"], "180.00");
    assert_eq!(combined["columns"]["billed"]["comparable"], "160.00");
    assert_eq!(combined["columns"]["expected"]["population"], Json::Null);
    let counts = &manifest["counts"]["a_summary"];
    assert_eq!(counts["eligible_rows"], 2);
    assert_eq!(counts["resolved_rows"], 1);
    assert_eq!(counts["unresolved_rows"], 1);
    assert_eq!(counts["output_rows"], 1);
    // Line evidence names the group each line contributed to.
    let evidence: Vec<_> = report
        .evidence
        .iter()
        .filter(|e| e["output"] == "a_summary")
        .collect();
    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0]["group"]["currency"], "AUD");
    assert_eq!(
        evidence
            .iter()
            .filter(|e| e["aggregate_inputs"]["billed_comparable"] == true)
            .count(),
        1
    );
}

// --- Currency at line grain and without an anchor currency (ticket #24) ----------

#[test]
fn line_grain_comparison_of_amounts_in_different_currencies_fails() {
    let mut l = ledger();
    l.doc["sources"][1]["projection"]["fields"]["currency"] = column("currency", string());
    l.write("customers", "id,budget,currency\nC1,1000,AUD\nC2,500,USD\n");
    l.add_column(
        "headroom",
        json!({"op":"subtract","left":via("customers","budget",&["customer"]),"right":field("lines","billed")}),
    );
    // L3 is billed in AUD against C2's USD budget; L4 (USD) against USD is fine.
    let report = l.run();
    assert_eq!(failure(&report), "MIXED_CURRENCY");
    let d = &report.diagnostics[0];
    assert_eq!(d.subject.as_deref(), Some("expression:review.headroom"));
    assert_eq!(d.affected_count, Some(1));
    assert_eq!(d.samples[0].location, "record:4");
    // Matching currencies compare normally.
    l.write("lines", &LINES.replace("L3,C2,AUD", "L3,C2,USD"));
    let report = l.succeeded();
    assert_eq!(
        text(&row(&report.tables["review"], &[("id", "L1")])["headroom"]),
        "990.00"
    );
}

#[test]
fn currency_checks_cover_predicates_and_measure_inputs_without_combining_independent_comparisons() {
    let setup = || {
        let mut l = ledger();
        l.doc["sources"][1]["projection"]["fields"]["currency"] = column("currency", string());
        l.write("customers", "id,budget,currency\nC1,1000,AUD\nC2,500,USD\n");
        l
    };
    let mixed = json!({"op":"less","left":via("customers","budget",&["customer"]),"right":field("lines","billed")});
    for slot in ["eligibility", "filter"] {
        let mut l = setup();
        l.output()[slot] = mixed.clone();
        assert_eq!(failure(&l.run()), "MIXED_CURRENCY", "{slot}");
    }
    let mut l = setup();
    l.group(&["currency"], vec![measure("mixed", "sum", json!({"op":"subtract","left":via("customers","budget",&["customer"]),"right":field("lines","billed")}))]);
    assert!(matches!(
        failure(&l.run()),
        "MIXED_CURRENCY" | "GRAIN_VIOLATION"
    ));
    let mut l = setup();
    let mut filtered = measure("filtered", "sum", field("lines", "billed"));
    filtered["filter"] = mixed;
    l.group(&["currency"], vec![filtered]);
    assert_eq!(failure(&l.run()), "MIXED_CURRENCY");
    let mut l = setup();
    l.add_column("independent", json!({"op":"and",
        "left":{"op":"equal","left":field("lines","billed"),"right":field("lines","billed")},
        "right":{"op":"equal","left":via("customers","budget",&["customer"]),"right":via("customers","budget",&["customer"])}}));
    let report = l.succeeded();
    assert!(
        report.tables["review"]
            .iter()
            .all(|r| r["independent"] == Value::Boolean { value: true })
    );
    let mut l = setup();
    l.add_column("selected", json!({"op":"case",
        "condition":{"op":"literal","value":{"kind":"boolean","value":true}},
        "then":{"op":"add","left":field("lines","billed"),"right":field("lines","billed")},
        "otherwise":{"op":"add","left":via("customers","budget",&["customer"]),"right":via("customers","budget",&["customer"])}}));
    l.succeeded();
}

#[test]
fn grouped_amounts_from_a_currency_source_need_an_anchor_currency() {
    let mut l = ledger();
    l.doc["sources"][0]["projection"]["fields"]
        .as_object_mut()
        .unwrap()
        .remove("currency");
    l.output()["columns"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["name"] != "currency");
    l.group(
        &["customer"],
        vec![measure(
            "components",
            "sum",
            detail("line_components", "sum", Some("amount")),
        )],
    );
    // Without currency anywhere the grouped sum is accepted.
    assert_eq!(static_code(&l.recipe()), "valid");
    // Components that carry a currency cannot be summed across lines whose own
    // currency is undeclared: nothing would keep USD and AUD apart.
    l.doc["sources"][2]["projection"]["fields"]["currency"] = column("currency", string());
    l.write(
        "components",
        "id,line,amount,currency\nk1,L1,1,AUD\nk4,L2,4,USD\n",
    );
    assert_eq!(static_code(&l.recipe()), "GRAIN_VIOLATION");
    assert_eq!(failure(&l.run()), "GRAIN_VIOLATION");
}
