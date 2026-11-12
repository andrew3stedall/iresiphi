//! Tickets #28 and #29: the Rust half of the customer/month parity golden.
//!
//! The slice-2 acceptance fixture (docs/spec/slice-2.md) runs through the public
//! Rust API and is compared to the committed golden file
//! `tests/fixtures/customer_month.parity.json`. The Python suite
//! (`tests/python_acceptance.py`, `CustomerMonthParity`) builds the same recipe
//! with `iresiphi.build`, runs it through the Python bindings and compares
//! against the same file. Regenerate with
//! `IRESIPHI_BLESS=1 cargo test --test customer_month_parity`.
#[path = "support/customer_month.rs"]
mod customer_month;
use customer_month::*;
use iresiphi::RunReport;
use serde_json::{Value as Json, json};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/customer_month.parity.json"
);

fn sorted_rows(report: &RunReport) -> Json {
    let mut tables = serde_json::Map::new();
    for (name, rows) in &report.tables {
        let mut rows: Vec<Json> = rows.iter().map(|r| json!(r)).collect();
        rows.sort_by_key(|r| r.to_string());
        tables.insert(name.clone(), Json::Array(rows));
    }
    Json::Object(tables)
}

/// Remove source references, whose binding identity depends on file paths.
fn without_references(mut value: Json) -> Json {
    match &mut value {
        Json::Object(map) => {
            for key in ["quantity_reference", "unmatched_samples"] {
                map.remove(key);
            }
            for child in map.values_mut() {
                *child = without_references(child.take());
            }
        }
        Json::Array(items) => {
            for child in items {
                *child = without_references(child.take());
            }
        }
        _ => {}
    }
    value
}

fn expected(report: &RunReport) -> Json {
    let mut diagnostics: Vec<Json> = report
        .diagnostics
        .iter()
        .map(|d| json!({"code":d.code,"severity":d.severity,"subject":d.subject,"affected_count":d.affected_count}))
        .collect();
    diagnostics.sort_by_key(|d| d.to_string());
    json!({
        "status": report.status,
        "tables": sorted_rows(report),
        "schema": report.manifest["schema"],
        "totals": report.manifest["totals"],
        "reconciliation": report.manifest["reconciliation"],
        "populations": without_references(report.manifest["populations"].clone()),
        "allocations": report.manifest["allocations"],
        "allocation_evidence": without_references(json!(report.allocation_evidence)),
        "diagnostics": diagnostics,
        "evidence_counts": {
            "primary": report.evidence.len(),
            "candidate": report.candidate_evidence.len(),
            "detail": report.detail_evidence.len(),
            "allocation": report.allocation_evidence.len(),
        },
    })
}

fn outcome(report: &RunReport) -> Json {
    let primary = &report.diagnostics[0];
    json!({
        "status": report.status,
        "code": primary.code,
        "phase": primary.phase,
        "check": primary.check,
        "subject": primary.subject,
    })
}

fn parity() -> Json {
    let f = fixture();
    let recipe = f.recipe();
    let full = expected(&f.succeeded());

    // Selecting A after allocation keeps its shares.
    let mut selected = fixture();
    selected.output()["filter"] = equal(field("customer_targets", "CustNo"), text("A"));
    let selected = expected(&selected.succeeded());

    // Restricting North recipients to A before normalization assigns A the whole goal.
    let mut restricted = fixture();
    restricted.allocation()["recipient_filter"] = or(
        equal(field("customer_month", "CustNo"), text("A")),
        equal(field("customer_month", "SalesArea"), text("South")),
    );
    let restricted = expected(&restricted.succeeded());

    // A null weight fails the run unless the recipe declares zero.
    let mut null_weight = fixture();
    null_weight.doc["sources"][0]["projection"]["fields"]["Share"]["nullable"] = json!(true);
    null_weight.write(
        "customers",
        "CustNo,SalesArea,Share\nA,North,1\nB,North,\\N\nC,South,1\n",
    );
    let null_weight = outcome(&null_weight.run());

    // A missing regional target leaves C's February allocation null and unresolved.
    let missing = fixture();
    missing.write(
        "targets",
        "Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\n",
    );
    let missing = expected(&missing.succeeded());

    json!({
        "recipe_hash": recipe.hash().unwrap(),
        "canonical_json": recipe.canonical_json().unwrap(),
        "expected": full,
        "scenarios": {
            "selected_a": selected,
            "north_restricted_to_a": restricted,
            "missing_target": missing,
        },
        "errors": {
            "null_weight": null_weight,
        },
    })
}

#[test]
fn rust_matches_the_shared_customer_month_golden() {
    let actual = parity();
    // The acceptance figures are fixed independently of the golden file.
    let expected = &actual["expected"];
    assert_eq!(expected["status"], "succeeded");
    let totals = &expected["totals"]["by_output"]["customer_targets"]["AUD"]["columns"];
    assert_eq!(totals["sales"]["population"], "470.00");
    assert_eq!(totals["allocated_target"]["population"], "2400.00");
    assert_eq!(totals["allocated_target"]["allocation"]["source_total"], "2400.00");
    assert!(totals.get("target_context").is_none());
    assert_eq!(expected["tables"]["customer_targets"].as_array().unwrap().len(), 6);
    assert_eq!(actual["errors"]["null_weight"]["code"], "ALLOCATION_NULL_WEIGHT");
    let text = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    if std::env::var_os("IRESIPHI_BLESS").is_some() {
        std::fs::write(GOLDEN, &text).unwrap();
    }
    let golden: Json = serde_json::from_str(
        &std::fs::read_to_string(GOLDEN)
            .expect("golden missing: run with IRESIPHI_BLESS=1 to create it"),
    )
    .unwrap();
    assert_eq!(
        actual, golden,
        "Rust results drifted from the shared customer/month golden"
    );
}

#[test]
fn the_documented_example_is_the_fixture_recipe() {
    let example = iresiphi::Recipe::from_json(include_str!("../examples/customer_month.iresiphi"))
        .expect("the example recipe loads");
    assert_eq!(example.hash().unwrap(), fixture().recipe().hash().unwrap());
}
