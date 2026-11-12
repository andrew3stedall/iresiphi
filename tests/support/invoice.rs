//! The two-supplier invoice review fixture (decision #3) shared by the review and
//! diagnostics suites. Real JSON, CSV and XLSX files are written below `root`.
#![allow(dead_code)]
use iresiphi::{ExecutionContext, RunReport, model::*};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

fn string(path: &str) -> Json {
    json!({"path":path,"data_type":{"kind":"string"}})
}
fn dec(path: &str, p: u8, s: u8) -> Json {
    json!({"path":path,"data_type":{"kind":"decimal","precision":p,"scale":s}})
}
pub fn field(source: &str, name: &str, path: &[&str]) -> Json {
    json!({"op":"field","source":source,"field":name,"path":path})
}
fn source(id: &str, array: Option<&str>, fields: Json, identity: Vec<&str>) -> Json {
    json!({"id":id,"identity":identity,"grain":identity,"projection":{"array":array,"fields":fields}})
}

pub const A_JSON: &str = r#"{"supplier":"A","invoice":"A1","date":"2026-11-12","currency":"AUD","lines":[{"id":"1","item":"service A","quantity":"2","billed":"110"},{"id":"2","item":"unknown","quantity":"1","billed":"20"}]}"#;
pub const B_JSON: &str = r#"{"vendor":"B","number":"B1","issued":"2026-11-12","currency":"AUD","details":[{"position":"1","description":"service B","units":"1","charge":"50"}]}"#;
pub const MAPPING: &str = "id,supplier,raw,item\nm1,A,service A,shared\nm2,B,service B,shared\n";
pub const RATES: &str = "id,supplier,item,rate,start,end\nr1,A,shared,50,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\n";
pub const COMPONENTS: &str =
    "id,supplier,invoice,line,amount\nc1,A,A1,1,1\nc2,A,A1,1,2\nc3,A,A1,1,3\nc4,B,B1,1,4\n";

pub fn write(root: &Path, name: &str, content: &str) {
    std::fs::write(root.join(name), content).unwrap();
}

/// Recipe, bindings and policy for the acceptance fixture.
pub fn fixture(root: &Path) -> (Recipe, Bindings, Policy) {
    crate::workbook_support::enrichment_workbook(&root.join("enrichment.xlsx"));
    write(root, "a.json", A_JSON);
    write(root, "b.json", B_JSON);
    write(root, "mapping.csv", MAPPING);
    write(root, "rates.csv", RATES);
    write(root, "components.csv", COMPONENTS);
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
    let recipe = Recipe::from_json(
        &json!({"format_version":"0.1","name":"two suppliers","sources":sources,"relationships":relationships,"outputs":outputs})
            .to_string(),
    )
    .unwrap();
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
        placement: Default::default(),
    };
    (recipe, bindings, policy)
}

pub fn run(recipe: &Recipe, bindings: &Bindings, policy: &Policy) -> RunReport {
    ExecutionContext::default().run(recipe, bindings, policy, &AtomicBool::new(false))
}

pub fn text(v: &Value) -> &str {
    match v {
        Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Int64 { value }
        | Value::Date { value } => value,
        other => panic!("expected scalar text, got {other:?}"),
    }
}

pub fn relationship_mut<'a>(recipe: &'a mut Recipe, id: &str) -> &'a mut Relationship {
    recipe
        .relationships
        .iter_mut()
        .find(|r| r.id == id)
        .unwrap()
}

pub fn source_mut<'a>(recipe: &'a mut Recipe, id: &str) -> &'a mut Source {
    recipe.sources.iter_mut().find(|s| s.id == id).unwrap()
}

pub fn diagnostic<'a>(report: &'a RunReport, code: &str) -> &'a Error {
    report
        .diagnostics
        .iter()
        .find(|d| d.code == code)
        .unwrap_or_else(|| panic!("no {code} diagnostic in {:?}", report.diagnostics))
}
