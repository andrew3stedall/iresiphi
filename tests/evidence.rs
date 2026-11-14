//! Slice 6: a run records evidence for lines that did not resolve unless the
//! policy asks for every line (docs/spec/slice-6.md).
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use invoice::*;
use iresiphi::{ExecutionContext, model::*};
use serde_json::{Value as Json, json};
use std::sync::atomic::AtomicBool;

fn check<'a>(manifest: &'a Json, id: &str) -> &'a Json {
    manifest["check_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no {id} check"))
}

#[test]
fn policy_records_problem_lines_unless_it_states_full() {
    let stated = |extra: Json| -> std::result::Result<Policy, serde_json::Error> {
        let mut policy = json!({"allowed_read_roots":[],"allowed_output_roots":[]});
        if let Json::Object(extra) = extra {
            policy.as_object_mut().unwrap().extend(extra);
        }
        serde_json::from_value(policy)
    };
    assert_eq!(stated(json!({})).unwrap().evidence, Evidence::Problems);
    assert_eq!(
        stated(json!({"evidence":"problems"})).unwrap().evidence,
        Evidence::Problems
    );
    assert_eq!(
        stated(json!({"evidence":"full"})).unwrap().evidence,
        Evidence::Full
    );
    assert!(stated(json!({"evidence":"none"})).is_err());
    assert!(stated(json!({"evidence":true})).is_err());
}

#[test]
fn problems_evidence_is_full_evidence_restricted_to_unresolved_lines() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = fixture(root.path());
    policy.evidence = Evidence::Full;
    let full = run(&recipe, &bindings, &policy);
    policy.evidence = Evidence::Problems;
    let problems = run(&recipe, &bindings, &policy);
    assert_eq!(
        problems.status, "succeeded_with_unresolved",
        "{:?}",
        problems.diagnostics
    );
    assert_eq!(full.status, problems.status);

    // Results, counts, totals and diagnostics do not depend on the level.
    assert_eq!(
        serde_json::to_value(&full.tables).unwrap(),
        serde_json::to_value(&problems.tables).unwrap()
    );
    for key in [
        "counts",
        "totals",
        "reconciliation",
        "resolved_rows",
        "unresolved_rows",
        "source_rows",
        "output_schema",
    ] {
        assert_eq!(full.manifest[key], problems.manifest[key], "{key}");
    }
    assert_eq!(
        serde_json::to_value(&full.diagnostics).unwrap(),
        serde_json::to_value(&problems.diagnostics).unwrap()
    );

    // The fixture has three lines; only supplier A's unmapped line is unresolved.
    assert_eq!(full.evidence.len(), 3);
    let unresolved: Vec<&Json> = full
        .evidence
        .iter()
        .filter(|r| r["status"] != "matched")
        .collect();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(problems.evidence.iter().collect::<Vec<_>>(), unresolved);
    let recorded: Vec<&Json> = unresolved.iter().map(|r| &r["source"]).collect();
    let of = |records: &'_ [Json], line: &str| -> Vec<Json> {
        records
            .iter()
            .filter(|r| recorded.contains(&&r[line]))
            .cloned()
            .collect()
    };
    assert_eq!(
        problems.candidate_evidence,
        of(&full.candidate_evidence, "anchor")
    );
    assert_eq!(
        problems.detail_evidence,
        of(&full.detail_evidence, "parent")
    );
    assert!(problems.candidate_evidence.len() < full.candidate_evidence.len());
    assert!(problems.detail_evidence.len() < full.detail_evidence.len());

    // The manifest states the level and what it covers.
    assert_eq!(problems.manifest["policy"]["evidence"], "problems");
    assert_eq!(problems.manifest["evidence"]["level"], "problems");
    assert_eq!(problems.manifest["evidence"]["primary_rows"], 1);
    assert_eq!(full.manifest["evidence"]["level"], "full");
    assert_eq!(full.manifest["evidence"]["primary_rows"], 3);
    let coverage = check(&problems.manifest, "evidence")["coverage"]
        .as_str()
        .unwrap();
    assert!(
        coverage.starts_with("problems only: 1 of 3 selected lines"),
        "{coverage}"
    );
    assert!(
        check(&full.manifest, "evidence")["coverage"]
            .as_str()
            .unwrap()
            .starts_with("complete: 3 primary")
    );
    let population = |report: &iresiphi::RunReport| {
        check(&report.manifest, "output.population")["coverage"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(population(&full).contains("output and evidence"));
    assert!(!population(&problems).contains("evidence"));
}

#[test]
fn a_run_in_which_every_line_resolves_records_no_line_evidence() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = fixture(root.path());
    let mut a: Json = serde_json::from_str(A_JSON).unwrap();
    a["lines"].as_array_mut().unwrap().truncate(1);
    write(root.path(), "a.json", &a.to_string());
    policy.evidence = Evidence::Problems;
    let destination = root.path().join("bundle");
    let report = ExecutionContext::default().run_bundle(
        &recipe,
        &bindings,
        &policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    assert_eq!(report.manifest["resolved_rows"], 2);
    assert_eq!(report.manifest["evidence"]["primary_rows"], 0);
    assert_eq!(report.manifest["evidence"]["candidate_rows"], 0);
    assert_eq!(report.manifest["evidence"]["detail_rows"], 0);
    let schema: Json =
        serde_json::from_slice(&std::fs::read(destination.join("schema.json")).unwrap()).unwrap();
    for table in ["_evidence", "_candidate_evidence", "_detail_evidence"] {
        let file = schema["csv"]["files"][table]
            .as_str()
            .unwrap_or_else(|| panic!("bundle has no {table} table"));
        let mut reader = csv::Reader::from_path(destination.join(file)).unwrap();
        assert_eq!(reader.records().count(), 0, "{table}");
    }
    let file = schema["csv"]["files"]["a_review"].as_str().unwrap();
    let mut reader = csv::Reader::from_path(destination.join(file)).unwrap();
    assert_eq!(reader.records().count(), 1);
}

#[test]
fn the_default_level_fits_a_budget_full_evidence_exceeds() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = fixture(root.path());
    policy.evidence = Evidence::Problems;
    let measured = run(&recipe, &bindings, &policy);
    assert_eq!(measured.status, "succeeded_with_unresolved");
    let bytes = serde_json::to_vec(&(
        &measured.tables,
        &measured.evidence,
        &measured.candidate_evidence,
        &measured.detail_evidence,
    ))
    .unwrap()
    .len();
    // Exactly what the default level serialized; full evidence needs more.
    policy.limits.output_bytes = bytes;
    assert_eq!(
        run(&recipe, &bindings, &policy).status,
        "succeeded_with_unresolved"
    );
    policy.evidence = Evidence::Full;
    let refused = run(&recipe, &bindings, &policy);
    assert_eq!(refused.status, "failed");
    assert_eq!(refused.diagnostics[0].code, "RESOURCE_LIMIT");
    assert!(refused.tables.is_empty());
}
