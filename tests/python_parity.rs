//! Ticket #23: the Rust half of Rust/Python public parity.
//!
//! The two-supplier fixture's recipe hash, expected results and tiny error cases
//! are computed through the public Rust API and compared to the committed
//! golden file `tests/fixtures/two_suppliers.parity.json`. The Python suite
//! (`tests/python_acceptance.py`) builds the same recipe with
//! `iresiphi.build`, runs it through the Python bindings and compares against
//! the same file. Regenerate with `IRESIPHI_BLESS=1 cargo test --test python_parity`.
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use invoice::*;
use iresiphi::{RunReport, model::*};
use serde_json::{Value as Json, json};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/two_suppliers.parity.json"
);

fn outcome(report: &RunReport) -> Json {
    let primary = report
        .diagnostics
        .iter()
        .find(|d| d.severity == "error")
        .or_else(|| report.diagnostics.first());
    json!({
        "status": report.status,
        "code": primary.map(|d| d.code.clone()),
        "phase": primary.map(|d| d.phase.clone()),
        "check": primary.and_then(|d| d.check.clone()),
        "subject": primary.and_then(|d| d.subject.clone()),
    })
}

fn sorted_rows(report: &RunReport) -> Json {
    let mut tables = serde_json::Map::new();
    for (name, rows) in &report.tables {
        let mut rows: Vec<Json> = rows.iter().map(|r| json!(r)).collect();
        rows.sort_by_key(|r| r.to_string());
        tables.insert(name.clone(), Json::Array(rows));
    }
    Json::Object(tables)
}

fn parity() -> Json {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let report = run(&recipe, &bindings, &policy);
    let mut warnings: Vec<Json> = report
        .diagnostics
        .iter()
        .map(|d| json!({"code":d.code,"severity":d.severity,"subject":d.subject,"affected_count":d.affected_count}))
        .collect();
    warnings.sort_by_key(|w| w.to_string());
    let expected = json!({
        "status": report.status,
        "tables": sorted_rows(&report),
        "schema": report.manifest["schema"],
        "totals": report.manifest["totals"],
        "reconciliation": report.manifest["reconciliation"],
        "diagnostics": warnings,
        "evidence_counts": {
            "primary": report.evidence.len(),
            "candidate": report.candidate_evidence.len(),
            "detail": report.detail_evidence.len(),
        },
    });

    // Missing binding: the run fails at access with a structured record.
    let mut missing = bindings.clone();
    missing.remove("rates");
    let missing_binding = outcome(&run(&recipe, &missing, &policy));

    // Duplicate lookup identity: data check failure.
    write(
        root.path(),
        "mapping.csv",
        "id,supplier,raw,item\nm1,A,service A,shared\nm1,B,service B,shared\n",
    );
    let duplicate_identity = outcome(&run(&recipe, &bindings, &policy));

    // Division is deferred and rejected statically, before any binding.
    let mut document: Json = serde_json::from_str(&recipe.to_json().unwrap()).unwrap();
    let outputs = document["outputs"].as_array_mut().unwrap();
    let a_review = outputs.iter_mut().find(|o| o["id"] == "a_review").unwrap();
    let columns = a_review["columns"].as_array_mut().unwrap();
    let variance = columns
        .iter_mut()
        .find(|c| c["name"] == "variance")
        .unwrap();
    variance["expr"] =
        json!({"op":"divide","left":field("a","billed",&[]),"right":field("a","quantity",&[])});
    let division = match Recipe::from_json(&document.to_string()) {
        Err(error) => json!({"stage":"load","code":error.code,"phase":error.phase}),
        Ok(divided) => {
            let report = iresiphi::validate(&divided);
            let diagnostic = report
                .checks
                .iter()
                .find_map(|c| c.diagnostic.clone())
                .expect("division is rejected");
            json!({"stage":"validate","valid":report.valid,"code":diagnostic.code,"phase":diagnostic.phase})
        }
    };

    json!({
        "recipe_hash": recipe.hash().unwrap(),
        "canonical_json": recipe.canonical_json().unwrap(),
        "expected": expected,
        "errors": {
            "missing_binding": missing_binding,
            "duplicate_identity": duplicate_identity,
            "division": division,
        },
    })
}

#[test]
fn rust_matches_the_shared_python_parity_golden() {
    let actual = parity();
    // The acceptance totals are fixed independently of the golden file.
    let combined = &actual["expected"]["totals"]["combined"]["AUD"]["columns"];
    assert_eq!(actual["expected"]["status"], "succeeded_with_unresolved");
    assert_eq!(combined["billed"]["population"], "180.00");
    assert_eq!(combined["billed"]["comparable"], "160.00");
    assert_eq!(combined["billed"]["unresolved"], "20.00");
    assert_eq!(combined["expected"]["comparable"], "145.00");
    assert_eq!(combined["variance"]["comparable"], "15.00");
    let text = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    if std::env::var_os("IRESIPHI_BLESS").is_some() {
        std::fs::create_dir_all(std::path::Path::new(GOLDEN).parent().unwrap()).unwrap();
        std::fs::write(GOLDEN, &text).unwrap();
    }
    let golden: Json = serde_json::from_str(
        &std::fs::read_to_string(GOLDEN)
            .expect("golden missing: run with IRESIPHI_BLESS=1 to create it"),
    )
    .unwrap();
    assert_eq!(
        actual, golden,
        "Rust results drifted from the shared parity golden"
    );
}
