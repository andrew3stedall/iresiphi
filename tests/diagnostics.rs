//! Decision #9 diagnostic records and bound validation coverage (carried from #17).
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
mod workbook_support;
use invoice::*;
use iresiphi::{
    model::*,
    validation::{validate, validate_bound},
};
use serde_json::json;

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
fn divide() -> Expr {
    Expr::Divide {
        left: Box::new(Expr::Field {
            source: "a".into(),
            field: "billed".into(),
            path: vec![],
        }),
        right: Box::new(Expr::Field {
            source: "a".into(),
            field: "quantity".into(),
            path: vec![],
        }),
    }
}

#[test]
fn run_with_an_unsupported_operation_fails_with_that_operations_code() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, bindings, policy) = fixture(root.path());
    recipe.outputs[0].columns.push(Column {
        name: "unit".into(),
        expr: divide(),
    });
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(report.status, "failed");
    let d = &report.diagnostics[0];
    assert_eq!(d.code, "UNSUPPORTED_OPERATION");
    assert_eq!(d.phase, "validation");
    assert_eq!(d.check.as_deref(), Some("expressions"));
    assert_eq!(d.severity, "error");
    assert!(d.remediation.as_deref().is_some_and(|r| !r.is_empty()));
    assert!(report.tables.is_empty());
    assert_eq!(report.manifest["complete"], false);
}

#[test]
fn static_validation_diagnostics_name_their_check_and_remediation() {
    let root = tempfile::tempdir().unwrap();
    let (mut recipe, _, _) = fixture(root.path());
    recipe.outputs[0].columns.push(Column {
        name: "unit".into(),
        expr: divide(),
    });
    let report = validate(&recipe);
    let d = check(&report, "expressions").diagnostic.as_ref().unwrap();
    assert_eq!(d.code, "UNSUPPORTED_OPERATION");
    assert_eq!(d.check.as_deref(), Some("expressions"));
    assert_eq!(d.severity, "error");
    assert!(d.remediation.is_some());
}

#[test]
fn lookup_diagnostics_are_complete_bounded_records_without_values() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "rates.csv",
        &format!("{RATES}r3,A,shared,60,2026-06-01,\\N\n"),
    );
    let report = run(&recipe, &bindings, &policy);
    let d = diagnostic(&report, "LOOKUP_AMBIGUOUS");
    assert_eq!(d.severity, "warning");
    assert_eq!(d.phase, "lookup");
    assert_eq!(d.subject.as_deref(), Some("relationship:a_rate"));
    assert_eq!(d.check.as_deref(), Some("lookup.cardinality"));
    assert_eq!(d.affected_outputs, ["a_review"]);
    assert_eq!(d.affected_count, Some(1));
    assert_eq!(d.count_status, "exact");
    assert_eq!(d.samples.len(), 1);
    assert_eq!(d.samples[0].location, "/lines/0");
    assert_eq!(d.samples[0].source, "a");
    assert!(!d.samples_truncated);
    assert!(d.remediation.as_deref().is_some_and(|r| !r.is_empty()));
    assert!(d.values.is_none());
    assert!(!serde_json::to_string(d).unwrap().contains("shared"));
    let unmatched = diagnostic(&report, "LOOKUP_UNMATCHED");
    assert_eq!(unmatched.subject.as_deref(), Some("relationship:a_mapping"));
    assert_eq!(unmatched.check.as_deref(), Some("lookup.coverage"));
    assert_eq!(unmatched.affected_count, Some(1));
    assert_eq!(unmatched.samples[0].location, "/lines/1");
}

#[test]
fn diagnostic_values_are_opt_in_and_the_opt_in_is_recorded() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, mut policy) = fixture(root.path());
    policy.diagnostic_values = true;
    let report = run(&recipe, &bindings, &policy);
    let d = diagnostic(&report, "LOOKUP_UNMATCHED");
    let values = d.values.as_ref().expect("values were requested");
    assert_eq!(values[0], json!({"supplier":"A","raw":"unknown"}));
    assert_eq!(report.manifest["diagnostic_values"], true);
}

#[test]
fn samples_are_capped_at_twenty_with_explicit_truncation_and_exact_count() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    let lines: Vec<_> = (1..=25)
        .map(|i| json!({"id":i.to_string(),"item":"unknown","quantity":"1","billed":"1"}))
        .collect();
    write(
        root.path(),
        "a.json",
        &json!({"supplier":"A","invoice":"A1","date":"2026-11-12","currency":"AUD","lines":lines})
            .to_string(),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    assert_eq!(report.tables["a_review"].len(), 25);
    let d = report
        .diagnostics
        .iter()
        .find(|d| {
            d.code == "LOOKUP_UNMATCHED" && d.subject.as_deref() == Some("relationship:a_mapping")
        })
        .unwrap();
    assert_eq!(d.affected_count, Some(25));
    assert_eq!(d.count_status, "exact");
    assert_eq!(d.samples.len(), 20);
    assert!(d.samples_truncated);
}

#[test]
fn identity_failures_name_source_check_count_and_samples() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "a.json",
        &A_JSON.replace(r#""id":"2""#, r#""id":"1""#),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(report.status, "failed");
    assert!(report.tables.is_empty());
    let d = &report.diagnostics[0];
    assert_eq!(d.code, "DUPLICATE_IDENTITY");
    assert_eq!(d.severity, "error");
    assert_eq!(d.subject.as_deref(), Some("source:a"));
    assert_eq!(d.check.as_deref(), Some("data.identity"));
    assert_eq!(d.affected_outputs, ["a_review"]);
    assert_eq!(d.affected_count, Some(2));
    let mut locations: Vec<_> = d.samples.iter().map(|s| s.location.as_str()).collect();
    locations.sort();
    assert_eq!(locations, ["/lines/0", "/lines/1"]);
    assert!(d.values.is_none());
    assert!(d.remediation.is_some());
}

#[test]
fn adapter_type_failures_name_their_source_and_check() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, bindings, policy) = fixture(root.path());
    write(
        root.path(),
        "a.json",
        &A_JSON.replace(r#""quantity":"2""#, r#""quantity":"two""#),
    );
    let report = run(&recipe, &bindings, &policy);
    assert_eq!(report.status, "failed");
    let d = &report.diagnostics[0];
    assert_eq!(d.subject.as_deref(), Some("source:a"));
    assert_eq!(d.check.as_deref(), Some("data.types"));
    assert_eq!(d.severity, "error");
    assert_eq!(d.affected_outputs, ["a_review"]);
    assert!(d.remediation.is_some());
    assert!(!d.message.contains("two"));
}

// --- Bound validation with data_checks = "none" ---------------------------------

#[test]
fn bound_validation_inspects_source_schemas_without_scanning_data() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, mut bindings, policy) = fixture(root.path());
    let none = validate_bound(&recipe, &bindings, &policy, "none");
    assert!(none.valid, "{none:?}");
    let schema = check(&none, "source_schema");
    assert_eq!(schema.status, "passed");
    assert!(schema.coverage.contains("headers"), "{}", schema.coverage);
    assert_eq!(check(&none, "data").status, "not-run");
    // A malformed data value is not a schema problem: none-mode does not scan for it.
    write(
        root.path(),
        "a.json",
        &A_JSON.replace(r#""quantity":"2""#, r#""quantity":"two""#),
    );
    assert!(validate_bound(&recipe, &bindings, &policy, "none").valid);
    assert!(!validate_bound(&recipe, &bindings, &policy, "full").valid);
    write(root.path(), "a.json", A_JSON);
    let failing = |bindings: &Bindings| {
        let report = validate_bound(&recipe, bindings, &policy, "none");
        assert!(!report.valid, "{report:?}");
        let check = check(&report, "source_schema");
        assert_eq!(check.status, "failed");
        check.diagnostic.clone().unwrap()
    };
    // Missing CSV header.
    write(
        root.path(),
        "rates.csv",
        &RATES.replacen("rate", "price", 1),
    );
    let d = failing(&bindings);
    assert_eq!(d.code, "INVALID_PROJECTION");
    assert_eq!(d.subject.as_deref(), Some("source:rates"));
    write(root.path(), "rates.csv", RATES);
    // Missing JSON array pointer.
    write(root.path(), "b.json", &B_JSON.replace("details", "items"));
    let d = failing(&bindings);
    assert_eq!(d.code, "INVALID_PROJECTION");
    assert_eq!(d.subject.as_deref(), Some("source:b"));
    // Missing required JSON header pointer.
    write(root.path(), "b.json", &B_JSON.replace("currency", "money"));
    assert_eq!(failing(&bindings).code, "INVALID_PROJECTION");
    write(root.path(), "b.json", B_JSON);
    // Missing XLSX sheet.
    bindings
        .get_mut("enrichment")
        .unwrap()
        .settings
        .insert("sheet".into(), json!("Elsewhere"));
    let d = failing(&bindings);
    assert_eq!(d.code, "INVALID_BINDING");
    assert_eq!(d.subject.as_deref(), Some("source:enrichment"));
    // Missing XLSX header column.
    bindings
        .get_mut("enrichment")
        .unwrap()
        .settings
        .insert("sheet".into(), json!("Review"));
    let mut renamed = recipe.clone();
    source_mut(&mut renamed, "enrichment")
        .projection
        .fields
        .get_mut("amount")
        .unwrap()
        .path = "total".into();
    let report = validate_bound(&renamed, &bindings, &policy, "none");
    assert_eq!(check(&report, "source_schema").status, "failed");
    assert_eq!(
        check(&report, "source_schema")
            .diagnostic
            .as_ref()
            .unwrap()
            .code,
        "INVALID_PROJECTION"
    );
}
