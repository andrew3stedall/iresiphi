//! The slice-2 customer/month example: sales aggregated onto an explicit
//! customer × month population, regional targets broadcast as context, and the
//! same targets allocated by each customer's share.
//!
//! ```text
//! cargo run --example customer_month -- WORK_DIR
//! ```
//!
//! WORK_DIR must not exist yet. The program writes the three CSV inputs there,
//! loads `examples/customer_month.iresiphi`, validates and runs it to memory, and
//! prints the rows, the totals and the conservation figures of each partition.
use iresiphi::{Binding, Bindings, ExecutionContext, Limits, Policy, Recipe, model::Value};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

const RECIPE: &str = include_str!("customer_month.iresiphi");
const INPUTS: [(&str, &str); 3] = [
    (
        "customers",
        "CustNo,SalesArea,Share\nA,North,1\nB,North,3\nC,South,1\n",
    ),
    (
        "transactions",
        "receipt_id,buyer_ref,booked_at,gross\nt1,A,2026-01-05,60\nt2,A,2026-01-20,40\nt3,B,2026-01-11,200\nt4,C,2026-01-17,50\nt5,B,2026-02-08,120\n",
    ),
    (
        "targets",
        "Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\nSouth,2026-02,400\n",
    ),
];

fn text(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Int64 { value }
        | Value::Date { value } => value.clone(),
        Value::Boolean { value } => value.to_string(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let work = std::env::args()
        .nth(1)
        .ok_or("usage: customer_month WORK_DIR")?;
    let work = Path::new(&work);
    std::fs::create_dir(work)?;
    let work = work.canonicalize()?;
    let mut bindings = Bindings::new();
    for (id, content) in INPUTS {
        let path = work.join(format!("{id}.csv"));
        std::fs::write(&path, content)?;
        bindings.insert(
            id.into(),
            Binding {
                path,
                adapter: "csv".into(),
                settings: BTreeMap::new(),
            },
        );
    }
    let policy = Policy {
        allowed_read_roots: vec![work.clone()],
        allowed_output_roots: vec![work.clone()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    };
    let recipe = Recipe::from_json(RECIPE)?;
    let report = iresiphi::validate(&recipe);
    println!("validate: valid={} hash={}", report.valid, recipe.hash()?);
    let run = ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    println!("run: {}", run.status);
    let mut rows = run.tables["customer_targets"].clone();
    rows.sort_by_key(|r| (text(&r["month"]), text(&r["CustNo"])));
    println!("CustNo month    sales  transactions target_context allocated_target");
    for row in &rows {
        println!(
            "{:<6} {} {:>7} {:>13} {:>14} {:>16}",
            text(&row["CustNo"]),
            text(&row["month"]),
            text(&row["sales"]),
            text(&row["transactions"]),
            text(&row["target_context"]),
            text(&row["allocated_target"]),
        );
    }
    let totals = &run.manifest["totals"]["by_output"]["customer_targets"]["AUD"];
    println!("total sales: {}", totals["columns"]["sales"]["population"]);
    println!(
        "allocated_target: assigned {} of source total {}, residual {}",
        totals["columns"]["allocated_target"]["allocation"]["assigned_total"],
        totals["columns"]["allocated_target"]["allocation"]["source_total"],
        totals["columns"]["allocated_target"]["allocation"]["residual_total"],
    );
    println!(
        "excluded from totals: {}",
        totals["excluded"]
            .as_object()
            .map(|e| e.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    );
    for partition in &run.allocation_evidence {
        println!(
            "partition {}/{}: quantity {} over {} recipients (weight sum {}), assigned {}, residual {}",
            partition["partition"]["Territory"].as_str().unwrap_or("?"),
            partition["partition"]["Period"].as_str().unwrap_or("?"),
            partition["quantity"],
            partition["recipient_count"],
            partition["weight_sum"],
            partition["assigned_sum"],
            partition["residual"],
        );
    }
    Ok(())
}
