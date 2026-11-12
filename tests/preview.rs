//! Ticket #24: workload phase measurements and reader diagnostics that count every
//! affected record, through the public run seam over real JSON and CSV files.
#[path = "support/run_fixture.rs"]
mod run_fixture;
use iresiphi::{ExecutionContext, RunReport};
use run_fixture::*;
use std::sync::atomic::AtomicBool;

/// Phases every run reports, in microseconds. `delivery` is added only to the
/// report of a committed bundle run.
const PHASES: [&str; 8] = [
    "static_validation",
    "source_access",
    "parse",
    "conversion",
    "data_validation",
    "planning",
    "execution",
    "evidence",
];

fn run(f: &Fixture) -> RunReport {
    ExecutionContext::default().run(&f.recipe, &f.bindings, &f.policy, &AtomicBool::new(false))
}

#[test]
fn manifest_records_every_workload_phase_in_microseconds() {
    let f = fixture();
    let report = run(&f);
    assert!(
        report.status.starts_with("succeeded"),
        "{:?}",
        report.diagnostics
    );
    let phases = &report.manifest["phases_us"];
    for phase in PHASES {
        assert!(phases[phase].is_u64(), "{phase} missing from {phases}");
    }
    let total = phases["total"].as_u64().expect("total");
    let sum: u64 = PHASES.iter().map(|p| phases[*p].as_u64().unwrap()).sum();
    assert!(sum <= total, "phases {sum}us exceed total {total}us");
    assert!(
        phases.get("delivery").is_none(),
        "a memory run delivers nothing"
    );
}

#[test]
fn bundle_runs_report_delivery_time_after_commit_only() {
    let f = fixture();
    let report = ExecutionContext::default().run_bundle(
        &f.recipe,
        &f.bindings,
        &f.policy,
        &f.destination(),
        &AtomicBool::new(false),
    );
    assert!(
        report.status.starts_with("succeeded"),
        "{:?}",
        report.diagnostics
    );
    assert!(report.manifest["phases_us"]["delivery"].is_u64());
    // The committed manifest cannot time its own publication.
    let committed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.destination().join("manifest.json")).unwrap())
            .unwrap();
    for phase in PHASES {
        assert!(committed["phases_us"][phase].is_u64(), "{phase}");
    }
    assert!(committed["phases_us"].get("delivery").is_none());
}

#[test]
fn csv_type_errors_report_the_exact_count_and_record_samples() {
    let f = fixture();
    std::fs::write(
        f.root().join("rates.csv"),
        "id,code,rate,start,end\nr1,A,x,2026-01-01,\\N\nr2,B,2.25,2026-01-01,2027-01-01\nr3,C,1.2.3,2026-01-01,\\N\n",
    )
    .unwrap();
    let report = run(&f);
    assert_failed_without_results(&report, "failed", "INVALID_DECIMAL");
    let d = &report.diagnostics[0];
    assert_eq!(d.subject.as_deref(), Some("source:rates"));
    assert_eq!(d.affected_count, Some(2), "{d:?}");
    assert_eq!(d.count_status, "exact");
    let locations: Vec<_> = d.samples.iter().map(|s| s.location.as_str()).collect();
    assert_eq!(locations, ["record:2", "record:4"]);
    assert!(!d.samples_truncated);
    assert!(d.values.is_none(), "values stay out unless opted in");
}

#[test]
fn json_type_errors_report_the_exact_count_and_pointer_samples() {
    let f = fixture();
    std::fs::write(
        f.root().join("lines.json"),
        r#"[{"id":"1","code":"A","date":"2026-02-01","qty":"two","note":""},
            {"id":"2","code":"B","date":"2026-03-01","qty":"1","note":""},
            {"id":"3","code":"Z","date":"2026-03-01","qty":"four","note":""}]"#,
    )
    .unwrap();
    let report = run(&f);
    assert_eq!(report.status, "failed");
    let d = &report.diagnostics[0];
    assert_eq!(d.subject.as_deref(), Some("source:lines"));
    assert_eq!(d.affected_count, Some(2), "{d:?}");
    assert_eq!(d.count_status, "exact");
    let locations: Vec<_> = d.samples.iter().map(|s| s.location.as_str()).collect();
    assert_eq!(locations, ["/0", "/2"]);
}

#[test]
fn type_error_samples_are_capped_while_the_count_stays_exact() {
    let f = fixture();
    let mut csv = String::from("id,code,rate,start,end\n");
    for i in 0..25 {
        csv.push_str(&format!("r{i},C{i},bad,2026-01-01,\\N\n"));
    }
    std::fs::write(f.root().join("rates.csv"), csv).unwrap();
    let report = run(&f);
    let d = &report.diagnostics[0];
    assert_eq!(d.affected_count, Some(25));
    assert_eq!(d.samples.len(), 20);
    assert!(d.samples_truncated);
}

#[test]
fn bundles_carry_a_typed_reconciliation_table() {
    use iresiphi::model::Value;
    let f = fixture();
    let report = ExecutionContext::default().run_bundle(
        &f.recipe,
        &f.bindings,
        &f.policy,
        &f.destination(),
        &AtomicBool::new(false),
    );
    assert_eq!(report.status, "succeeded_with_unresolved");
    let tables = iresiphi::sink::read_bundle(&f.destination()).unwrap();
    let rows = &tables["_reconciliation"];
    let text = |row: &std::collections::BTreeMap<String, Value>, key: &str| match &row[key] {
        Value::String { value } | Value::Int64 { value } => value.clone(),
        Value::Null => "null".into(),
        other => panic!("{key}: {other:?}"),
    };
    // Lines 1 and 2 resolve; line 3 has no rate. Each line's count column is -7.
    let count = rows
        .iter()
        .find(|r| text(r, "scope") == "combined" && text(r, "column") == "count")
        .expect("combined count row");
    assert_eq!(text(count, "currency"), "undeclared");
    assert_eq!(text(count, "population_rows"), "3");
    assert_eq!(text(count, "resolved_rows"), "2");
    assert_eq!(text(count, "unresolved_rows"), "1");
    assert_eq!(text(count, "population_total"), "-21");
    assert_eq!(text(count, "comparable_total"), "-14");
    assert_eq!(text(count, "unresolved_total"), "-7");
    // A column with a null on some line has no complete total.
    let expected = rows
        .iter()
        .find(|r| text(r, "scope") == "review" && text(r, "column") == "expected")
        .expect("per-output expected row");
    assert_eq!(text(expected, "population_total"), "null");
    assert_eq!(text(expected, "null_rows"), "1");
    // The same totals as the manifest, which stays authoritative.
    assert_eq!(
        report.manifest["totals"]["combined"]["undeclared"]["columns"]["count"]["comparable"],
        "-14"
    );
}

#[test]
fn runs_assert_output_population_and_identity_after_execution() {
    let f = fixture();
    let report = run(&f);
    let checks = report.manifest["check_results"].as_array().unwrap();
    let population = checks
        .iter()
        .find(|c| c["id"] == "output.population")
        .expect("population invariant check");
    assert_eq!(population["status"], "passed");
    assert!(
        population["coverage"]
            .as_str()
            .unwrap()
            .contains("3 anchor lines"),
        "{population}"
    );
    // The check precedes evidence, so a failed run lists it as not-run.
    std::fs::write(
        f.root().join("rates.csv"),
        "id,code,rate,start,end\nr1,A,x,2026-01-01,\\N\n",
    )
    .unwrap();
    let failed = run(&f);
    let checks = failed.manifest["check_results"].as_array().unwrap();
    let population = checks
        .iter()
        .find(|c| c["id"] == "output.population")
        .unwrap();
    assert_eq!(population["status"], "not-run");
}
