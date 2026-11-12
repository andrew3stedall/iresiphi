#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{ExecutionContext, model::*};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn string(path: &str) -> Json {
    json!({"path":path,"data_type":{"kind":"string"}})
}
fn dec(path: &str, p: u8, s: u8) -> Json {
    json!({"path":path,"data_type":{"kind":"decimal","precision":p,"scale":s}})
}
fn field(source: &str, name: &str, path: &[&str]) -> Json {
    json!({"op":"field","source":source,"field":name,"path":path})
}
fn source(id: &str, array: Option<&str>, fields: Json, identity: Vec<&str>) -> Json {
    json!({"id":id,"identity":identity,"grain":identity,"projection":{"array":array,"fields":fields}})
}
fn fixture(root: &std::path::Path) -> (Recipe, Bindings, Policy) {
    workbook_support::enrichment_workbook(&root.join("enrichment.xlsx"));
    std::fs::write(root.join("a.json"),r#"{"supplier":"A","invoice":"A1","date":"2026-11-12","currency":"AUD","lines":[{"id":"1","item":"service A","quantity":"2","billed":"110"},{"id":"2","item":"unknown","quantity":"1","billed":"20"}]}"#).unwrap();
    std::fs::write(root.join("b.json"),r#"{"vendor":"B","number":"B1","issued":"2026-11-12","currency":"AUD","details":[{"position":"1","description":"service B","units":"1","charge":"50"}]}"#).unwrap();
    std::fs::write(
        root.join("mapping.csv"),
        "id,supplier,raw,item\nm1,A,service A,shared\nm2,B,service B,shared\n",
    )
    .unwrap();
    std::fs::write(root.join("rates.csv"),"id,supplier,item,rate,start,end\nr1,A,shared,50,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\n").unwrap();
    std::fs::write(
        root.join("components.csv"),
        "id,supplier,invoice,line,amount\nc1,A,A1,1,1\nc2,A,A1,1,2\nc3,A,A1,1,3\nc4,B,B1,1,4\n",
    )
    .unwrap();
    let mut sources = vec![];
    let mut relationships = vec![];
    let mut outputs = vec![];
    for (id, array, supplier, invoice, date, line, item, quantity, billed) in [
        (
            "a",
            "/lines",
            "/supplier",
            "/invoice",
            "/date",
            "/id",
            "/item",
            "/quantity",
            "/billed",
        ),
        (
            "b",
            "/details",
            "/vendor",
            "/number",
            "/issued",
            "/position",
            "/description",
            "/units",
            "/charge",
        ),
    ] {
        let mut fields = json!({"supplier":string(supplier),"invoice":string(invoice),"date":{"path":date,"data_type":{"kind":"date"}},"currency":string("/currency"),"id":string(line),"raw":string(item),"quantity":dec(quantity,12,4),"billed":dec(billed,18,2)});
        for key in ["supplier", "invoice", "date", "currency"] {
            fields[key]["header"] = json!(true);
        }
        sources.push(source(
            id,
            Some(array),
            fields,
            vec!["supplier", "invoice", "id"],
        ));
        let mapping = format!("{id}_mapping");
        let rate = format!("{id}_rate");
        let detail = format!("{id}_components");
        let enrichment = format!("{id}_enrichment");
        relationships.push(json!({"id":enrichment,"from":id,"to":"enrichment","from_keys":["supplier","id"],"to_keys":["supplier","id"],"cardinality":"many_to_one"}));
        relationships.push(json!({"id":mapping,"from":id,"to":"mapping","from_keys":["supplier","raw"],"to_keys":["supplier","raw"],"cardinality":"many_to_one"}));
        relationships.push(json!({"id":rate,"from":id,"to":"rates","from_keys":["supplier",format!("{mapping}.item")],"to_keys":["supplier","item"],"cardinality":"many_to_one","validity":{"date":"date","valid_from":"start","valid_to":"end"}}));
        relationships.push(json!({"id":detail,"from":id,"to":"components","from_keys":["supplier","invoice","id"],"to_keys":["supplier","invoice","line"],"cardinality":"one_to_many"}));
        let expected = json!({"op":"quantize","expr":{"op":"multiply","left":field(id,"quantity",&[]),"right":field("rates","rate",&[&rate])},"precision":18,"scale":2,"rounding":"half_even"});
        outputs.push(json!({"id":format!("{id}_review"),"anchor":id,"columns":[{"name":"enriched","expr":field("enrichment","amount",&[&enrichment])},{"name":"id","expr":field(id,"id",&[])},{"name":"supplier","expr":field(id,"supplier",&[])},{"name":"billed","expr":field(id,"billed",&[])},{"name":"expected","expr":expected},{"name":"variance","expr":{"op":"subtract","left":field(id,"billed",&[]),"right":expected}}]}));
    }
    sources.push(source(
        "enrichment",
        None,
        json!({"id":string("id"),"supplier":string("supplier"),"amount":dec("amount",18,2)}),
        vec!["supplier", "id"],
    ));
    sources.push(source("mapping",None,json!({"id":string("id"),"supplier":string("supplier"),"raw":string("raw"),"item":string("item")}),vec!["id"]));
    sources.push(source("rates",None,json!({"id":string("id"),"supplier":string("supplier"),"item":string("item"),"rate":dec("rate",14,6),"start":{"path":"start","data_type":{"kind":"date"}},"end":{"path":"end","data_type":{"kind":"date"},"nullable":true}}),vec!["id"]));
    sources.push(source("components",None,json!({"id":string("id"),"supplier":string("supplier"),"invoice":string("invoice"),"line":string("line"),"amount":dec("amount",18,2)}),vec!["id"]));
    // Reversed declaration order proves that lookup dependencies determine execution order.
    relationships.reverse();
    let recipe=Recipe::from_json(&json!({"format_version":"0.1","name":"two suppliers","sources":sources,"relationships":relationships,"outputs":outputs}).to_string()).unwrap();
    let bindings = [
        ("a", "a.json", "json"),
        ("b", "b.json", "json"),
        ("mapping", "mapping.csv", "csv"),
        ("rates", "rates.csv", "csv"),
        ("components", "components.csv", "csv"),
        ("enrichment", "enrichment.xlsx", "xlsx"),
    ]
    .into_iter()
    .map(|(id, path, adapter)| {
        (
            id.into(),
            Binding {
                path: root.join(path),
                adapter: adapter.into(),
                settings: if adapter == "xlsx" {
                    BTreeMap::from([
                        ("sheet".into(), json!("Review")),
                        ("header_row".into(), json!(1)),
                    ])
                } else {
                    BTreeMap::new()
                },
            },
        )
    })
    .collect();
    let policy = Policy {
        allowed_read_roots: vec![root.to_owned()],
        allowed_output_roots: vec![root.to_owned()],
        limits: Limits::default(),
        diagnostic_values: false,
    };
    (recipe, bindings, policy)
}
fn text(v: &Value) -> &str {
    match v {
        Value::String { value } | Value::Decimal { value, .. } | Value::Int64 { value } => value,
        _ => panic!("expected scalar text"),
    }
}
#[test]
fn two_supplier_review_preserves_unresolved_lines_and_component_fanout() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let rows: Vec<_> = report.tables.values().flatten().collect();
    assert_eq!(rows.len(), 3);
    let a = rows
        .iter()
        .find(|r| text(&r["supplier"]) == "A" && text(&r["id"]) == "1")
        .unwrap();
    assert_eq!(text(&a["billed"]), "110.00");
    assert_eq!(text(&a["expected"]), "100.00");
    assert_eq!(text(&a["variance"]), "10.00");
    let b = rows.iter().find(|r| text(&r["supplier"]) == "B").unwrap();
    assert_eq!(text(&b["expected"]), "45.00");
    assert_eq!(text(&b["variance"]), "5.00");
    let unknown = rows
        .iter()
        .find(|r| text(&r["supplier"]) == "A" && text(&r["id"]) == "2")
        .unwrap();
    assert_eq!(unknown["expected"], Value::Null);
    assert_eq!(unknown["variance"], Value::Null);
    assert_eq!(text(&unknown["billed"]), "20.00");
    assert_eq!(report.evidence.len(), 3);
    let full: i64 = rows
        .iter()
        .map(|r| text(&r["billed"]).replace('.', "").parse::<i64>().unwrap())
        .sum();
    assert_eq!(full, 18000);
    let comparable: Vec<_> = rows
        .iter()
        .filter(|r| r["expected"] != Value::Null)
        .collect();
    for (column, expected) in [("billed", 16000), ("expected", 14500), ("variance", 1500)] {
        let cents: i64 = comparable
            .iter()
            .map(|r| text(&r[column]).replace('.', "").parse::<i64>().unwrap())
            .sum();
        assert_eq!(cents, expected);
    }
}
#[test]
fn duplicate_anchor_fails_without_publishing_partial_tables() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let path = root.path().join("a.json");
    let mut doc: Json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["lines"][1]["id"] = json!("1");
    std::fs::write(path, doc.to_string()).unwrap();
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(report.status, "failed");
    assert!(report.tables.is_empty());
    assert_eq!(report.diagnostics[0].code, "DUPLICATE_IDENTITY");
}
#[test]
fn rebind_saved_recipe_uses_new_rates_without_editing_invoices() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, mut bindings, policy) = fixture(root.path());
    let saved = recipe.to_json().unwrap();
    let reopened = Recipe::from_json(&saved).unwrap();
    let replacement = root.path().join("new-rates.csv");
    std::fs::write(&replacement,"id,supplier,item,rate,start,end\nr1,A,shared,49,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\n").unwrap();
    bindings.get_mut("rates").unwrap().path = replacement;
    let report =
        ExecutionContext::default().run(&reopened, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    assert_eq!(text(&report.tables["a_review"][0]["expected"]), "98.00");
    assert_eq!(recipe.hash().unwrap(), reopened.hash().unwrap());
}
#[test]
fn bound_validation_reports_coverage_without_implicitly_scanning() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    std::fs::write(root.path().join("a.json"), "not json").unwrap();
    let report = iresiphi::validation::validate_bound(&recipe, &bindings, &policy, "none");
    assert!(report.valid);
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.id == "data" && c.status == "not-run")
    );
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.id == "source_schema" && c.status == "not-run")
    );
    let report = iresiphi::validation::validate_bound(&recipe, &bindings, &policy, "full");
    assert!(!report.valid);
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.id == "data" && c.status == "failed")
    );
    let explained = iresiphi::validation::explain_bound(&recipe, &bindings, &policy);
    assert_eq!(explained["bound"]["source_access"], "not-run");
}
#[test]
fn overlapping_rates_remain_ambiguous_and_ending_interval_is_excluded() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    std::fs::write(root.path().join("rates.csv"),"id,supplier,item,rate,start,end\nr1,A,shared,50,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\nr3,A,shared,99,2026-11-12,2030-01-01\nr4,B,shared,99,2026-01-01,2026-11-12\n").unwrap();
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    assert_eq!(report.tables["a_review"][0]["expected"], Value::Null);
    assert!(text(&report.tables["a_review"][0]["_status"]).contains("ambiguous"));
    assert_eq!(text(&report.tables["b_review"][0]["expected"]), "45.00");
    assert_eq!(report.tables["a_review"].len(), 2);
}
#[test]
#[ignore = "ticket #21: grouped aggregation over an output is not implemented yet"]
fn currency_safe_grouping_retains_owned_totals_and_filters_after_sum() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    let out = &mut recipe.outputs[0];
    out.columns.push(Column {
        name: "currency".into(),
        expr: Expr::Field {
            source: "a".into(),
            field: "currency".into(),
            path: vec![],
        },
    });
    out.group_by = vec!["currency".into()];
    out.aggregates = vec![GroupAggregate {
        name: "total".into(),
        function: "sum".into(),
        expr: Expr::Field {
            source: "a".into(),
            field: "billed".into(),
            path: vec![],
        },
    }];
    out.filter = Some(Expr::Less {
        left: Box::new(Expr::Literal {
            value: Value::Decimal {
                value: "120.00".into(),
                precision: 18,
                scale: 2,
            },
        }),
        right: Box::new(Expr::Field {
            source: "a_review".into(),
            field: "total".into(),
            path: vec![],
        }),
    });
    assert!(
        iresiphi::validate(&recipe).valid,
        "{:?}",
        iresiphi::validate(&recipe)
    );
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert!(
        report.status.starts_with("succeeded"),
        "{:?}",
        report.diagnostics
    );
    assert_eq!(report.tables["a_review"].len(), 1);
    assert_eq!(text(&report.tables["a_review"][0]["total"]), "130.00");
}
#[test]
fn summing_broadcast_rates_or_discarding_currency_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, _, _) = fixture(root.path());
    recipe.outputs[0].aggregates = vec![GroupAggregate {
        name: "wrong".into(),
        function: "sum".into(),
        expr: Expr::Field {
            source: "rates".into(),
            field: "rate".into(),
            path: vec!["a_rate".into()],
        },
    }];
    assert!(!iresiphi::validate(&recipe).valid);
    recipe.outputs[0].aggregates[0].expr = Expr::Field {
        source: "a".into(),
        field: "billed".into(),
        path: vec![],
    };
    assert!(!iresiphi::validate(&recipe).valid);
}
#[test]
fn independent_details_have_separate_aggregates_with_empty_and_null_semantics() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, mut bindings, policy) = fixture(root.path());
    let mut other = recipe
        .sources
        .iter()
        .find(|s| s.id == "components")
        .unwrap()
        .clone();
    other.id = "other_components".into();
    other.projection.fields.get_mut("amount").unwrap().nullable = true;
    recipe.sources.push(other);
    std::fs::write(
        root.path().join("other.csv"),
        "id,supplier,invoice,line,amount\no1,A,A1,1,\\N\no2,A,A1,1,\\N\n",
    )
    .unwrap();
    bindings.insert(
        "other_components".into(),
        Binding {
            path: root.path().join("other.csv"),
            adapter: "csv".into(),
            settings: BTreeMap::new(),
        },
    );
    recipe.relationships.push(Relationship {
        id: "other_detail".into(),
        from: "a".into(),
        to: "other_components".into(),
        from_keys: vec!["supplier".into(), "invoice".into(), "id".into()],
        to_keys: vec!["supplier".into(), "invoice".into(), "line".into()],
        cardinality: Cardinality::OneToMany,
        validity: None,
        strict: false,
    });
    for (name, relationship, function, field) in [
        ("components_sum", "a_components", "sum", Some("amount")),
        ("components_count", "a_components", "count_records", None),
        ("other_sum", "other_detail", "sum", Some("amount")),
        (
            "other_nonnull",
            "other_detail",
            "count_non_null",
            Some("amount"),
        ),
        ("other_count", "other_detail", "count_records", None),
    ] {
        recipe.outputs[0].columns.push(Column {
            name: name.into(),
            expr: Expr::Aggregate {
                relationship: relationship.into(),
                function: function.into(),
                field: field.map(str::to_owned),
                filter: None,
            },
        });
    }
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let populated = &report.tables["a_review"][0];
    assert_eq!(text(&populated["components_sum"]), "6.00");
    assert_eq!(text(&populated["components_count"]), "3");
    assert_eq!(populated["other_sum"], Value::Null);
    assert_eq!(text(&populated["other_nonnull"]), "0");
    assert_eq!(text(&populated["other_count"]), "2");
    assert_eq!(text(&populated["billed"]), "110.00");
    let empty = &report.tables["a_review"][1];
    assert_eq!(empty["components_sum"], Value::Null);
    assert_eq!(text(&empty["components_count"]), "0");
    assert_eq!(empty["other_sum"], Value::Null);
}
