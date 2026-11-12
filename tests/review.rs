//! Ticket #20: supplier/date-qualified invoice review with evidence, through the
//! public validate/explain/run seam over real JSON, CSV and XLSX files.
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use invoice::*;
use iresiphi::{
    ExecutionContext,
    model::*,
    validation::{validate, validate_bound},
};
use serde_json::{Value as Json, json};
use std::sync::atomic::AtomicBool;

fn check<'a>(
    report: &'a iresiphi::validation::Report,
    id: &str,
) -> &'a iresiphi::validation::Check {
    report
        .checks
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no check {id} in {report:?}"))
}
fn static_code(recipe: &Recipe) -> String {
    let report = validate(recipe);
    report
        .checks
        .iter()
        .find_map(|c| c.diagnostic.as_ref().map(|d| d.code.clone()))
        .unwrap_or_else(|| "valid".into())
}

// --- Acceptance totals ------------------------------------------------------

#[test]
fn report_keeps_comparable_totals_distinct_from_the_full_population() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let combined = &report.manifest["totals"]["combined"]["AUD"];
    assert_eq!(
        combined["rows"],
        json!({"population":3,"resolved":2,"unresolved":1})
    );
    let billed = &combined["columns"]["billed"];
    assert_eq!(billed["population"], "180.00");
    assert_eq!(billed["comparable"], "160.00");
    assert_eq!(billed["unresolved"], "20.00");
    assert_eq!(billed["null_rows"], 0);
    let expected = &combined["columns"]["expected"];
    assert_eq!(expected["comparable"], "145.00");
    // A population total over a column with unresolved nulls is not claimed.
    assert_eq!(expected["population"], Json::Null);
    assert_eq!(expected["null_rows"], 1);
    assert_eq!(combined["columns"]["variance"]["comparable"], "15.00");
    let a = &report.manifest["totals"]["by_output"]["a_review"]["AUD"];
    assert_eq!(a["columns"]["billed"]["population"], "130.00");
    assert_eq!(a["columns"]["billed"]["comparable"], "110.00");
    // Never labelled fully reconciled.
    assert_eq!(report.manifest["reconciliation"]["status"], "partial");
    assert_eq!(report.manifest["reconciliation"]["resolved_rows"], 2);
    assert_eq!(report.manifest["reconciliation"]["unresolved_rows"], 1);
    assert!(
        !report
            .manifest
            .to_string()
            .contains("fully_reconciled\":true")
    );
}

#[test]
fn unresolved_lines_carry_a_status_and_a_reason_naming_the_lookup() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let report = run(&recipe, &bindings, &policy);
    let lines = &report.tables["a_review"];
    assert_eq!(text(&lines[0]["_status"]), "matched");
    assert_eq!(lines[0]["_reason"], Value::Null);
    let unknown = &lines[1];
    assert_eq!(text(&unknown["_status"]), "unmatched");
    assert_eq!(unknown["expected"], Value::Null);
    assert_eq!(unknown["variance"], Value::Null);
    let reason = text(&unknown["_reason"]);
    // The dependent rate lookup is blocked by the missing translation, and says so.
    assert!(reason.contains("a_rate"), "{reason}");
    assert!(reason.contains("a_mapping"), "{reason}");
    assert!(reason.contains("unmatched"), "{reason}");
    // No source values leak into the reason text.
    assert!(!reason.contains("unknown"), "{reason}");
}

// --- Lookup selection ---------------------------------------------------------

#[test]
fn overlapping_qualified_rates_are_ambiguous_never_latest_wins() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "rates.csv",
        "id,supplier,item,rate,start,end\nr1,A,shared,50,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\nr3,A,shared,60,2026-06-01,\\N\n",
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let line = &report.tables["a_review"][0];
    assert_eq!(text(&line["_status"]), "ambiguous");
    assert_eq!(line["expected"], Value::Null);
    assert_eq!(line["variance"], Value::Null);
    assert_eq!(text(&line["billed"]), "110.00");
    let reason = text(&line["_reason"]);
    assert!(
        reason.contains("a_rate") && reason.contains("2 qualified candidates"),
        "{reason}"
    );
    // Still exactly one primary row per extracted line.
    assert_eq!(report.tables["a_review"].len(), 2);
    assert_eq!(text(&report.tables["b_review"][0]["expected"]), "45.00");
}

#[test]
fn duplicate_rate_records_with_equal_values_are_not_deduplicated() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "rates.csv",
        "id,supplier,item,rate,start,end\nr1,A,shared,50,2026-01-01,2030-01-01\nr1b,A,shared,50,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\n",
    );
    let report = run(&recipe, &bindings, &policy);
    let line = &report.tables["a_review"][0];
    assert_eq!(text(&line["_status"]), "ambiguous");
    assert_eq!(line["expected"], Value::Null);
}

#[test]
fn validity_is_inclusive_lower_exclusive_upper_with_open_upper_bound() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    // A starts on the invoice date (inclusive) and is open-ended; B ends on it (exclusive).
    write(
        root.path(),
        "rates.csv",
        "id,supplier,item,rate,start,end\nr1,A,shared,50,2026-11-12,\\N\nr2,B,shared,45,2026-01-01,2026-11-12\n",
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    assert_eq!(text(&report.tables["a_review"][0]["expected"]), "100.00");
    let b = &report.tables["b_review"][0];
    assert_eq!(b["expected"], Value::Null);
    assert_eq!(text(&b["_status"]), "unmatched");
    let reason = text(&b["_reason"]);
    assert!(
        reason.contains("b_rate") && reason.contains("none valid"),
        "{reason}"
    );
}

#[test]
fn invalid_and_empty_intervals_fail_the_run_and_full_validation() {
    for (label, row) in [
        ("inverted", "r3,A,other,1,2026-05-01,2026-04-01"),
        ("empty", "r3,A,other,1,2026-05-01,2026-05-01"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (recipe, bindings, policy) = fixture(root.path());
        // The invalid row is not even a candidate for any line: the whole rate
        // population is validated, not only the rows a lookup happens to touch.
        write(root.path(), "rates.csv", &format!("{RATES}{row}\n"));
        let report = run(&recipe, &bindings, &policy);
        assert_eq!(report.status, "failed", "{label}");
        assert!(report.tables.is_empty(), "{label}");
        let d = diagnostic(&report, "INVALID_INTERVAL");
        assert_eq!(d.subject.as_deref(), Some("source:rates"), "{label}");
        assert_eq!(d.check.as_deref(), Some("data.rate_validity"), "{label}");
        assert_eq!(d.affected_count, Some(1), "{label}");
        assert_eq!(d.samples[0].location, "record:4", "{label}");
        let full = validate_bound(&recipe, &bindings, &policy, "full");
        assert!(!full.valid, "{label}");
        assert_eq!(
            check(&full, "data.rate_validity").status,
            "failed",
            "{label}"
        );
    }
}

#[test]
fn null_key_components_never_match_and_keys_are_not_coerced() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    source_mut(&mut recipe, "a")
        .projection
        .fields
        .get_mut("raw")
        .unwrap()
        .nullable = true;
    source_mut(&mut recipe, "mapping")
        .projection
        .fields
        .get_mut("raw")
        .unwrap()
        .nullable = true;
    // Line 1's item differs only by case; line 2's item is null. Mapping m3 has a
    // null raw value too: null = null is unknown, never a match.
    write(
        root.path(),
        "a.json",
        r#"{"supplier":"A","invoice":"A1","date":"2026-11-12","currency":"AUD","lines":[{"id":"1","item":"Service A","quantity":"2","billed":"110"},{"id":"2","item":null,"quantity":"1","billed":"20"}]}"#,
    );
    write(
        root.path(),
        "mapping.csv",
        &format!("{MAPPING}m3,A,\\N,shared\n"),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    let lines = &report.tables["a_review"];
    assert_eq!(text(&lines[0]["_status"]), "unmatched");
    assert_eq!(lines[0]["expected"], Value::Null);
    assert_eq!(text(&lines[1]["_status"]), "unmatched");
    assert!(
        text(&lines[1]["_reason"]).contains("null key"),
        "{:?}",
        lines[1]
    );
    // Cross-type keys are rejected statically rather than coerced.
    let mut mistyped = recipe.clone();
    source_mut(&mut mistyped, "mapping")
        .projection
        .fields
        .get_mut("raw")
        .unwrap()
        .data_type = Type::Int64;
    assert_eq!(static_code(&mistyped), "TYPE_MISMATCH");
}

#[test]
fn strict_cardinality_fails_the_run_while_default_quarantines_per_anchor() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "rates.csv",
        &format!("{RATES}r3,A,shared,60,2026-06-01,\\N\n"),
    );
    let default = run(&recipe, &bindings, &policy);
    assert_eq!(default.status, "succeeded_with_unresolved");
    let warning = diagnostic(&default, "LOOKUP_AMBIGUOUS");
    assert_eq!(warning.severity, "warning");
    assert_eq!(warning.affected_outputs, ["a_review"]);
    assert_eq!(warning.affected_count, Some(1));
    relationship_mut(&mut recipe, "a_rate").strict = true;
    let strict = run(&recipe, &bindings, &policy);
    assert_eq!(strict.status, "failed");
    assert!(strict.tables.is_empty());
    assert!(strict.evidence.is_empty());
    let d = diagnostic(&strict, "CARDINALITY_VIOLATION");
    assert_eq!(d.severity, "error");
    assert_eq!(d.subject.as_deref(), Some("relationship:a_rate"));
    assert_eq!(d.affected_count, Some(1));
    assert_eq!(d.samples[0].location, "/lines/0");
}

#[test]
fn one_to_one_lookup_sharing_a_target_is_a_cardinality_violation() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    relationship_mut(&mut recipe, "a_mapping").cardinality = Cardinality::OneToOne;
    write(
        root.path(),
        "a.json",
        &A_JSON.replace(r#""item":"unknown""#, r#""item":"service A""#),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    for line in &report.tables["a_review"] {
        assert_eq!(text(&line["_status"]), "ambiguous", "{line:?}");
        assert!(text(&line["_reason"]).contains("one_to_one"), "{line:?}");
        assert_eq!(line["expected"], Value::Null);
    }
    relationship_mut(&mut recipe, "a_mapping").strict = true;
    let strict = run(&recipe, &bindings, &policy);
    assert_eq!(strict.status, "failed");
    assert_eq!(
        diagnostic(&strict, "CARDINALITY_VIOLATION").affected_count,
        Some(2)
    );
}

// --- Named paths and grain ------------------------------------------------------

#[test]
fn competing_paths_require_an_explicit_named_choice() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    // A second, currently equivalent route from invoice lines to the enrichment sheet.
    let mut second = recipe
        .relationships
        .iter()
        .find(|r| r.id == "a_enrichment")
        .unwrap()
        .clone();
    second.id = "a_enrichment_by_line".into();
    recipe.relationships.push(second);
    let column = |path: Vec<&str>| Column {
        name: "chosen".into(),
        expr: Expr::Field {
            source: "enrichment".into(),
            field: "amount".into(),
            path: path.into_iter().map(str::to_owned).collect(),
        },
    };
    let mut omitted = recipe.clone();
    omitted.outputs[0].columns.push(column(vec![]));
    assert_eq!(static_code(&omitted), "INVALID_PATH");
    let failed = run(&omitted, &bindings, &policy);
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.diagnostics[0].code, "INVALID_PATH");
    let mut unknown = recipe.clone();
    unknown.outputs[0]
        .columns
        .push(column(vec!["no_such_route"]));
    assert_eq!(static_code(&unknown), "MISSING_REFERENCE");
    let mut disconnected = recipe.clone();
    disconnected.outputs[0]
        .columns
        .push(column(vec!["a_mapping", "b_enrichment"]));
    assert_eq!(static_code(&disconnected), "INVALID_PATH");
    recipe.outputs[0]
        .columns
        .push(column(vec!["a_enrichment_by_line"]));
    let report = run(&recipe, &bindings, &policy);
    assert!(
        report.status.starts_with("succeeded"),
        "{:?}",
        report.diagnostics
    );
    assert_eq!(text(&report.tables["a_review"][0]["chosen"]), "7.00");
}

#[test]
fn scalar_component_selection_at_line_grain_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    recipe.outputs[0].columns.push(Column {
        name: "component".into(),
        expr: Expr::Field {
            source: "components".into(),
            field: "amount".into(),
            path: vec!["a_components".into()],
        },
    });
    assert_eq!(static_code(&recipe), "GRAIN_VIOLATION");
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(report.status, "failed");
    assert_eq!(report.diagnostics[0].code, "GRAIN_VIOLATION");
}

fn with_detail_output(recipe: &mut Recipe) {
    recipe.outputs.push(Output {
        id: "components_detail".into(),
        anchor: "components".into(),
        columns: vec![
            Column {
                name: "id".into(),
                expr: Expr::Field {
                    source: "components".into(),
                    field: "id".into(),
                    path: vec![],
                },
            },
            Column {
                name: "amount".into(),
                expr: Expr::Field {
                    source: "components".into(),
                    field: "amount".into(),
                    path: vec![],
                },
            },
        ],
        eligibility: None,
        filter: None,
        group_by: vec![],
        detail: true,
        aggregates: vec![],
        complete: vec![],
        allocations: vec![],
    });
}

#[test]
fn components_are_a_separate_keyed_detail_output_with_parent_references() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    with_detail_output(&mut recipe);
    write(
        root.path(),
        "components.csv",
        &format!("{COMPONENTS}c5,A,A1,9,5\n"),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    // Components never multiply primary rows or their amounts.
    assert_eq!(report.tables["a_review"].len(), 2);
    assert_eq!(text(&report.tables["a_review"][0]["billed"]), "110.00");
    assert_eq!(
        report.manifest["totals"]["combined"]["AUD"]["columns"]["billed"]["population"],
        "180.00"
    );
    let detail = &report.tables["components_detail"];
    assert_eq!(detail.len(), 5);
    let c1 = detail.iter().find(|r| text(&r["id"]) == "c1").unwrap();
    assert_eq!(text(&c1["_status"]), "matched");
    let parent: Json = serde_json::from_str(text(&c1["_parent"])).unwrap();
    assert_eq!(parent["relationship"], "a_components");
    assert_eq!(parent["source"], "a");
    assert_eq!(
        parent["identity"],
        json!({"supplier":"A","invoice":"A1","id":"1"})
    );
    let c4 = detail.iter().find(|r| text(&r["id"]) == "c4").unwrap();
    let parent: Json = serde_json::from_str(text(&c4["_parent"])).unwrap();
    assert_eq!(parent["relationship"], "b_components");
    // The orphan stays visible as detail without a parent; it is not a new line.
    let c5 = detail.iter().find(|r| text(&r["id"]) == "c5").unwrap();
    assert_eq!(c5["_parent"], Value::Null);
    assert_eq!(text(&c5["_status"]), "unmatched");
    assert_eq!(diagnostic(&report, "ORPHAN_DETAIL").affected_count, Some(1));
    // Detail outputs are not comparison lines and carry no totals.
    assert!(
        report.manifest["totals"]["by_output"]
            .get("components_detail")
            .is_none()
    );
    assert_eq!(
        report.manifest["counts"]["components_detail"]["output_rows"],
        5
    );
}

#[test]
fn detail_outputs_require_a_declared_one_to_many_parent() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, _, _) = fixture(root.path());
    with_detail_output(&mut recipe);
    assert_eq!(static_code(&recipe), "valid");
    recipe.outputs.last_mut().unwrap().anchor = "mapping".into();
    for column in &mut recipe.outputs.last_mut().unwrap().columns {
        if let Expr::Field { source, .. } = &mut column.expr {
            *source = "mapping".into();
        }
    }
    recipe.outputs.last_mut().unwrap().columns.pop();
    assert_eq!(static_code(&recipe), "INVALID_DETAIL");
}

// --- Evidence ---------------------------------------------------------------------

#[test]
fn primary_evidence_records_selected_references_and_original_values() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let report = run(&recipe, &bindings, &policy);
    let line = report
        .evidence
        .iter()
        .find(|e| e["output"] == "a_review" && e["source"]["location"] == "/lines/0")
        .unwrap();
    assert_eq!(line["original"]["billed"], "110");
    assert_eq!(line["status"], "matched");
    let rel = |id: &str| {
        line["relationships"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["relationship"] == id)
            .unwrap()
            .clone()
    };
    let rate = rel("a_rate");
    assert_eq!(rate["status"], "matched");
    assert_eq!(rate["candidate_count"], 1);
    assert_eq!(rate["selected"]["location"], "record:2");
    assert_eq!(rate["selected"]["source"], "rates");
    assert_eq!(rel("a_mapping")["selected"]["location"], "record:2");
    assert_eq!(rel("a_enrichment")["selected"]["location"], "Review!2");
    // Component parent links: three observed components beneath the line.
    assert_eq!(rel("a_components")["candidate_count"], 3);
    let unknown = report
        .evidence
        .iter()
        .find(|e| e["source"]["location"] == "/lines/1")
        .unwrap();
    assert_eq!(unknown["status"], "unmatched");
    assert_eq!(unknown["original"]["raw"], "unknown");
}

#[test]
fn candidate_and_detail_evidence_are_separate_tables() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "rates.csv",
        &format!("{RATES}r3,A,shared,60,2026-06-01,\\N\n"),
    );
    let report = run(&recipe, &bindings, &policy);
    let line = report
        .evidence
        .iter()
        .find(|e| e["source"]["location"] == "/lines/0")
        .unwrap();
    let rate = line["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["relationship"] == "a_rate")
        .unwrap();
    assert_eq!(rate["status"], "ambiguous");
    assert_eq!(rate["candidate_count"], 2);
    assert_eq!(rate["selected"], Json::Null);
    let mut refs: Vec<_> = rate["ambiguous_references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["location"].as_str().unwrap().to_owned())
        .collect();
    refs.sort();
    assert_eq!(refs, ["record:2", "record:4"]);
    let candidates: Vec<_> = report
        .candidate_evidence
        .iter()
        .filter(|c| c["relationship"] == "a_rate" && c["anchor"]["location"] == "/lines/0")
        .collect();
    assert_eq!(candidates.len(), 2);
    for c in &candidates {
        assert_eq!(c["output"], "a_review");
        assert_eq!(c["status"], "ambiguous");
        assert_eq!(c["selected"], false);
        assert_eq!(c["candidate_count"], 2);
    }
    let r3 = candidates
        .iter()
        .find(|c| c["candidate"]["location"] == "record:4")
        .unwrap();
    assert_eq!(r3["original"]["rate"], "60");
    // B's unique rate is a selected candidate.
    assert!(
        report
            .candidate_evidence
            .iter()
            .any(|c| c["relationship"] == "b_rate"
                && c["selected"] == true
                && c["original"]["rate"] == "45")
    );
    // Detail evidence links each component to its parent line with original values.
    let details: Vec<_> = report
        .detail_evidence
        .iter()
        .filter(|d| d["relationship"] == "a_components")
        .collect();
    assert_eq!(details.len(), 3);
    for d in &details {
        assert_eq!(d["parent"]["location"], "/lines/0");
        assert_eq!(d["output"], "a_review");
        assert!(
            d["detail"]["location"]
                .as_str()
                .unwrap()
                .starts_with("record:")
        );
    }
    assert!(details.iter().any(|d| d["original"]["amount"] == "3"));
    assert_eq!(
        report.manifest["evidence"]["candidate_rows"],
        report.candidate_evidence.len()
    );
    assert_eq!(report.manifest["evidence"]["detail_rows"], 4);
    assert_eq!(report.manifest["evidence"]["complete"], true);
}

#[test]
fn evidence_budget_fails_rather_than_truncating() {
    for squeeze in ["pairs", "bytes"] {
        let root = tempfile::tempdir().unwrap();
        let (recipe, bindings, mut policy) = fixture(root.path());
        match squeeze {
            "pairs" => policy.limits.candidate_pairs = 5,
            _ => policy.limits.output_bytes = 2_000,
        }
        let report = run(&recipe, &bindings, &policy);
        assert_eq!(report.status, "failed", "{squeeze}");
        assert!(report.tables.is_empty(), "{squeeze}");
        assert!(report.evidence.is_empty(), "{squeeze}");
        assert!(report.candidate_evidence.is_empty(), "{squeeze}");
        assert!(report.detail_evidence.is_empty(), "{squeeze}");
        let d = diagnostic(&report, "RESOURCE_LIMIT");
        assert_eq!(d.check.as_deref(), Some("budget.evidence"), "{squeeze}");
        assert_eq!(report.manifest["complete"], false, "{squeeze}");
    }
}

// --- Validation and re-checking ----------------------------------------------------

#[test]
fn full_validation_is_evidence_and_runs_recheck_changed_inputs() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let full = validate_bound(&recipe, &bindings, &policy, "full");
    assert!(full.valid, "{full:?}");
    for id in [
        "data.types",
        "data.identity",
        "data.relationships",
        "data.rate_validity",
    ] {
        assert_eq!(check(&full, id).status, "passed", "{id}");
    }
    let first = run(&recipe, &bindings, &policy);
    // Inputs change after validation; the run checks its own read materialization.
    write(
        root.path(),
        "rates.csv",
        &format!("{RATES}r3,A,x,1,2026-05-01,2026-04-01\n"),
    );
    let changed = run(&recipe, &bindings, &policy);
    assert_eq!(changed.status, "failed");
    assert_eq!(changed.diagnostics[0].code, "INVALID_INTERVAL");
    write(root.path(), "rates.csv", &RATES.replace(",50,", ",49,"));
    let rebound = run(&recipe, &bindings, &policy);
    assert_eq!(text(&rebound.tables["a_review"][0]["expected"]), "98.00");
    assert_ne!(
        first.manifest["sources"]["rates"]["fingerprint"],
        rebound.manifest["sources"]["rates"]["fingerprint"]
    );
    assert_eq!(
        first.manifest["sources"]["a"]["fingerprint"],
        rebound.manifest["sources"]["a"]["fingerprint"]
    );
}

// --- Run statuses -------------------------------------------------------------------

#[test]
fn run_statuses_distinguish_complete_unresolved_empty_and_cancelled() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "mapping.csv",
        &format!("{MAPPING}m3,A,unknown,shared\n"),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    assert_eq!(report.manifest["reconciliation"]["status"], "complete");
    assert_eq!(
        report.manifest["totals"]["combined"]["AUD"]["columns"]["expected"]["population"],
        "195.00"
    );
    // An empty selected population succeeds with zero counts and no completeness claim.
    for output in &mut recipe.outputs {
        output.eligibility = Some(Expr::Literal {
            value: Value::Boolean { value: false },
        });
    }
    let empty = run(&recipe, &bindings, &policy);
    assert_eq!(empty.status, "succeeded", "{:?}", empty.diagnostics);
    assert!(empty.tables.values().all(|t| t.is_empty()));
    assert_eq!(empty.manifest["reconciliation"]["status"], "empty");
    assert_eq!(empty.manifest["counts"]["a_review"]["source_rows"], 2);
    assert_eq!(empty.manifest["counts"]["a_review"]["eligible_rows"], 0);
    assert_eq!(empty.manifest["counts"]["a_review"]["output_rows"], 0);
    let cancelled =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(true));
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.tables.is_empty());
}

// --- Manifest -------------------------------------------------------------------------

#[test]
fn manifest_records_decision9_run_evidence() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let report = run(&recipe, &bindings, &policy);
    let m = &report.manifest;
    assert_eq!(m["recipe_hash"], json!(recipe.hash().unwrap()));
    assert_eq!(m["format_version"], "0.1");
    assert!(m["package_version"].is_string());
    assert_eq!(m["policy"]["diagnostic_values"], false);
    assert_eq!(m["data_checks"], "full");
    // Safe binding identities and fingerprints of the bytes actually read.
    let a = &m["sources"]["a"];
    assert!(a["binding"].as_str().unwrap().starts_with("binding:"));
    assert_eq!(a["adapter"], "json");
    assert_eq!(a["rows"], 2);
    assert_eq!(a["bytes_read"], A_JSON.len());
    use sha2::Digest;
    let digest: String = sha2::Sha256::digest(A_JSON.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(a["fingerprint"], json!(digest));
    let sources = m["sources"].to_string();
    assert!(
        !sources.contains(root.path().to_str().unwrap()),
        "{sources}"
    );
    assert_eq!(m["sources"].as_object().unwrap().len(), 6);
    let total: u64 = m["sources"]
        .as_object()
        .unwrap()
        .values()
        .map(|s| s["bytes_read"].as_u64().unwrap())
        .sum();
    assert_eq!(m["bytes_read"], total);
    // Executed nodes use stable declaration IDs.
    let nodes: Vec<_> = m["executed_nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    for node in [
        "source:a",
        "source:rates",
        "relationship:a_mapping",
        "relationship:a_rate",
        "output:a_review",
        "output:b_review",
    ] {
        assert!(nodes.contains(&node), "{node} in {nodes:?}");
    }
    assert!(
        nodes.iter().position(|n| *n == "relationship:a_mapping")
            < nodes.iter().position(|n| *n == "relationship:a_rate")
    );
    assert_eq!(
        m["counts"]["a_review"],
        json!({"source_rows":2,"eligible_rows":2,"output_rows":2,"resolved_rows":1,"unresolved_rows":1})
    );
    assert_eq!(m["resolved_rows"], 2);
    assert_eq!(m["unresolved_rows"], 1);
    // Memory runs cannot know bytes written; #22 measures the engine reservation
    // and, where the platform allows, peak RSS — only unmeasured values stay unknown.
    let unknown = m["unknown_measurements"].as_array().unwrap();
    assert!(unknown.contains(&json!("bytes_written")));
    assert!(!unknown.contains(&json!("engine_peak_reservation")));
    assert!(m["engine_peak_reservation"]["peak_bytes"].is_number());
    assert_eq!(
        unknown.contains(&json!("peak_rss")),
        m["peak_rss"].get("bytes").is_none()
    );
    let checks = m["check_results"].as_array().unwrap();
    for id in [
        "static",
        "data.types",
        "data.identity",
        "data.relationships",
        "data.rate_validity",
        "lookup.cardinality",
        "evidence",
    ] {
        let c = checks
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("{id}"));
        assert_eq!(c["status"], "passed", "{id}");
        assert!(c["coverage"].is_string(), "{id}");
    }
    for phase in ["read", "lookups", "outputs", "total"] {
        assert!(m["timings_ms"][phase].is_u64(), "{phase}");
    }
    assert_eq!(m["complete"], true);
}

#[test]
fn failed_runs_mark_incomplete_checks_and_counts_explicitly() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "rates.csv",
        &format!("{RATES}r3,A,x,1,2026-05-01,2026-04-01\n"),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(report.status, "failed");
    let m = &report.manifest;
    assert_eq!(m["complete"], false);
    assert_eq!(m["recipe_hash"], json!(recipe.hash().unwrap()));
    assert_eq!(m["counts"], "incomplete");
    assert_eq!(m["totals"], "incomplete");
    let checks = m["check_results"].as_array().unwrap();
    let rate = checks
        .iter()
        .find(|c| c["id"] == "data.rate_validity")
        .unwrap();
    assert_eq!(rate["status"], "failed");
    let evidence = checks.iter().find(|c| c["id"] == "evidence").unwrap();
    assert_eq!(evidence["status"], "not-run");
    assert!(m["policy"].is_object());
}

#[test]
fn engine_status_columns_are_reserved() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, _, _) = fixture(root.path());
    for name in ["_status", "_reason", "_parent"] {
        let mut clashing = recipe.clone();
        clashing.outputs[0].columns[0].name = name.into();
        assert_eq!(static_code(&clashing), "DUPLICATE_ID", "{name}");
    }
}

#[test]
fn detail_records_matched_by_several_parent_lines_are_flagged_not_duplicated() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    // Keying components on supplier+invoice only lets every A1 component match both A1 lines.
    let rel = relationship_mut(&mut recipe, "a_components");
    rel.from_keys = vec!["supplier".into(), "invoice".into()];
    rel.to_keys = vec!["supplier".into(), "invoice".into()];
    with_detail_output(&mut recipe);
    let report = run(&recipe, &bindings, &policy);
    assert!(
        report.status.starts_with("succeeded"),
        "{:?}",
        report.diagnostics
    );
    let d = diagnostic(&report, "DETAIL_MULTIPLE_PARENTS");
    assert_eq!(d.severity, "warning");
    assert_eq!(d.subject.as_deref(), Some("relationship:a_components"));
    assert_eq!(d.check.as_deref(), Some("detail.parent"));
    assert_eq!(d.affected_count, Some(2));
    // Primary rows and billed totals are unchanged; shared detail has no single parent.
    assert_eq!(report.tables["a_review"].len(), 2);
    assert_eq!(
        report.manifest["totals"]["combined"]["AUD"]["columns"]["billed"]["population"],
        "180.00"
    );
    let detail = &report.tables["components_detail"];
    assert_eq!(detail.len(), 4);
    let c1 = detail.iter().find(|r| text(&r["id"]) == "c1").unwrap();
    assert_eq!(text(&c1["_status"]), "ambiguous");
    assert_eq!(c1["_parent"], Value::Null);
    relationship_mut(&mut recipe, "a_components").strict = true;
    let strict = run(&recipe, &bindings, &policy);
    assert_eq!(strict.status, "failed");
    assert_eq!(
        diagnostic(&strict, "CARDINALITY_VIOLATION").affected_count,
        Some(2)
    );
}
