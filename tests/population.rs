//! Ticket #28: explicit output population (recipient source crossed with a finite
//! period set), aggregation of finer records onto it, the completeness
//! declaration, unmatched-record reporting and row-limit accounting, through the
//! public validate/explain/run seam. Contract: docs/spec/slice-2.md.
#[path = "support/customer_month.rs"]
mod customer_month;
#[path = "support/invoice.rs"]
mod invoice;
#[path = "support/workbook.rs"]
#[allow(dead_code)]
mod workbook_support;
use customer_month::*;
use iresiphi::{ExecutionContext, explain};
use serde_json::{Value as Json, json};
use std::sync::atomic::AtomicBool;

#[test]
fn six_members_carry_sales_and_counts_with_declared_complete_activity() {
    let f = sales_fixture();
    assert_eq!(static_code(&f.doc), "valid");
    let report = f.succeeded();
    assert_eq!(report.status, "succeeded");
    let table = &report.tables["customer_sales"];
    assert_eq!(table.len(), 6, "customer by month");
    for (customer, january, february) in [
        ("A", "100.00", "0.00"),
        ("B", "200.00", "120.00"),
        ("C", "50.00", "0.00"),
    ] {
        assert_eq!(cell(table, customer, "2026-01", "sales"), Some(january));
        assert_eq!(cell(table, customer, "2026-02", "sales"), Some(february));
    }
    for (customer, january, february) in [("A", "2", "0"), ("B", "1", "1"), ("C", "1", "0")] {
        assert_eq!(
            cell(table, customer, "2026-01", "transactions"),
            Some(january)
        );
        assert_eq!(
            cell(table, customer, "2026-02", "transactions"),
            Some(february)
        );
    }
    for row in table {
        assert_eq!(value_text(&row["_status"]), Some("matched"));
    }
    let totals = &report.manifest["totals"]["by_output"]["customer_sales"]["AUD"];
    assert_eq!(totals["columns"]["sales"]["population"], "470.00");
    assert_eq!(totals["columns"]["transactions"]["population"], "5");
    assert_eq!(totals["rows"]["population"], 6);
    assert_eq!(report.evidence.len(), 6, "one evidence record per member");
    assert_eq!(
        report.detail_evidence.len(),
        5,
        "every transaction joined a member"
    );
    let population = &report.manifest["populations"]["customer_sales"];
    assert_eq!(population["population"], "population:customer_month");
    assert_eq!(population["recipients"], 3);
    assert_eq!(population["periods"], json!(["2026-01", "2026-02"]));
    assert_eq!(population["members"], 6);
    assert_eq!(population["activity"]["sales"]["matched_records"], 5);
    assert_eq!(population["activity"]["sales"]["unmatched_records"], 0);
}

#[test]
fn members_without_activity_are_null_with_zero_count_unless_declared_complete() {
    let mut f = sales_fixture();
    f.output().as_object_mut().unwrap().remove("complete");
    let report = f.succeeded();
    let table = &report.tables["customer_sales"];
    assert_eq!(
        cell(table, "A", "2026-02", "sales"),
        None,
        "unobserved is not zero"
    );
    assert_eq!(cell(table, "A", "2026-02", "transactions"), Some("0"));
    assert_eq!(cell(table, "C", "2026-02", "sales"), None);
    assert_eq!(cell(table, "B", "2026-02", "sales"), Some("120.00"));
    let rule = &report.manifest["populations"]["customer_sales"]["activity"]["sales"];
    assert_eq!(rule["empty_member_rule"], "observed_only");
    assert_eq!(rule["empty_members"], 2);
    assert!(
        rule["statement"].as_str().unwrap().contains("null"),
        "{rule}"
    );
    // A total over a column with a null member is not presented as complete.
    let totals = &report.manifest["totals"]["by_output"]["customer_sales"]["AUD"]["columns"];
    assert_eq!(totals["sales"]["population"], Json::Null);
    assert_eq!(totals["sales"]["null_rows"], 2);
}

#[test]
fn manifest_and_explain_state_that_completeness_is_declared_not_observed() {
    let f = sales_fixture();
    let report = f.succeeded();
    let rule = &report.manifest["populations"]["customer_sales"]["activity"]["sales"];
    assert_eq!(rule["empty_member_rule"], "declared_complete");
    assert_eq!(rule["empty_members"], 2);
    let statement = rule["statement"].as_str().unwrap();
    assert!(statement.contains("declared assumption"), "{statement}");
    assert!(statement.contains("not an observation"), "{statement}");

    let explanation = explain(&f.recipe());
    assert_eq!(explanation["validation"]["valid"], true);
    let nodes = explanation["nodes"].as_array().unwrap();
    let output = nodes
        .iter()
        .find(|n| n["id"] == "output:customer_sales")
        .unwrap();
    assert_eq!(output["population"], "population:customer_month");
    let completeness = &output["completeness"]["sales"];
    assert_eq!(completeness["rule"], "declared_complete");
    assert!(
        completeness["statement"]
            .as_str()
            .unwrap()
            .contains("declared assumption, not an observation")
    );
    let population = nodes
        .iter()
        .find(|n| n["id"] == "population:customer_month")
        .expect("population node");
    assert_eq!(population["kind"], "population");
    assert_eq!(population["recipients"], "source:customers");
    assert_eq!(
        population["period"]["values"],
        json!(["2026-01", "2026-02"])
    );
    assert!(
        population["inference"]
            .as_str()
            .unwrap()
            .contains("never inferred"),
        "{population}"
    );
    // Finer records reach members by declared keys and the declared derivation.
    let sales = nodes
        .iter()
        .find(|n| n["id"] == "relationship:sales")
        .unwrap();
    assert_eq!(sales["period"]["derive"], "calendar_month");
    assert_eq!(sales["period"]["date"], "booked_at");
    // No synthetic scan of the population.
    assert!(!nodes.iter().any(|n| n["id"] == "source:customer_month"));
}

#[test]
fn unmatched_finer_records_are_counted_with_samples_and_never_widen_the_population() {
    let f = sales_fixture();
    f.write(
        "transactions",
        &format!("{TRANSACTIONS}t6,D,2026-01-09,10\nt7,A,2026-03-02,30\nt8,B,\\N,5\n"),
    );
    let report = f.succeeded();
    let table = &report.tables["customer_sales"];
    assert_eq!(table.len(), 6, "no member for D or March");
    assert_eq!(cell(table, "A", "2026-01", "sales"), Some("100.00"));
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "POPULATION_UNMATCHED")
        .expect("unmatched finer records are reported");
    assert_eq!(diagnostic.severity, "warning");
    assert_eq!(diagnostic.affected_count, Some(3));
    assert_eq!(diagnostic.count_status, "exact");
    assert_eq!(diagnostic.samples.len(), 3);
    assert_eq!(diagnostic.subject.as_deref(), Some("source:transactions"));
    assert_eq!(
        diagnostic.affected_outputs,
        vec!["customer_sales".to_string()]
    );
    let activity = &report.manifest["populations"]["customer_sales"]["activity"]["sales"];
    assert_eq!(activity["unmatched_records"], 3);
    assert_eq!(activity["matched_records"], 5);
    assert_eq!(activity["unmatched_samples"].as_array().unwrap().len(), 3);
}

#[test]
fn calendar_month_derivation_uses_business_local_dates_without_conversion() {
    let f = sales_fixture();
    f.write(
        "transactions",
        "receipt_id,buyer_ref,booked_at,gross\nt1,A,2026-01-31,7\nt2,A,2026-02-01,11\n",
    );
    let table = &f.succeeded().tables["customer_sales"];
    assert_eq!(cell(table, "A", "2026-01", "sales"), Some("7.00"));
    assert_eq!(cell(table, "A", "2026-02", "sales"), Some("11.00"));
}

#[test]
fn the_period_set_can_be_the_distinct_values_of_a_named_source_field() {
    let mut f = sales_fixture();
    f.doc["populations"][0]["period"] =
        json!({"field":"month","calendar":"month","from":{"source":"targets","field":"Period"}});
    let report = f.succeeded();
    assert_eq!(report.tables["customer_sales"].len(), 6);
    let population = &report.manifest["populations"]["customer_sales"];
    assert_eq!(population["periods"], json!(["2026-01", "2026-02"]));
    assert_eq!(population["period_source"], "source:targets/field:Period");

    // A value that is not a calendar month fails the run; nothing is guessed.
    f.write("targets", &format!("{TARGETS}East,2026-1,5\n"));
    let report = f.run();
    assert_eq!(failure(&report).code, "INVALID_PERIOD");
}

#[test]
fn a_recipient_filter_subsets_the_recipient_source() {
    let mut f = sales_fixture();
    f.doc["populations"][0]["filter"] = equal(field("customers", "SalesArea"), text("North"));
    let report = f.succeeded();
    let table = &report.tables["customer_sales"];
    assert_eq!(table.len(), 4);
    assert!(table.iter().all(|r| value_text(&r["CustNo"]) != Some("C")));
    let population = &report.manifest["populations"]["customer_sales"];
    assert_eq!(population["recipients"], 2);
    assert_eq!(population["members"], 4);
    // C's transaction now matches no member and is reported, not added.
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "POPULATION_UNMATCHED")
        .unwrap();
    assert_eq!(diagnostic.affected_count, Some(1));
}

#[test]
fn population_size_counts_toward_anchor_rows_before_materialization() {
    let mut f = sales_fixture();
    f.policy.limits.anchor_rows = 5;
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "RESOURCE_LIMIT");
    assert_eq!(error.subject.as_deref(), Some("population:customer_month"));
    assert!(error.message.contains("6"), "{}", error.message);
    f.policy.limits.anchor_rows = 6;
    assert_eq!(f.succeeded().tables["customer_sales"].len(), 6);
}

#[test]
fn bundles_carry_population_outputs_and_their_schema() {
    let f = sales_fixture();
    let destination = f.dir.path().join("bundle");
    let report = ExecutionContext::default().run_bundle(
        &f.recipe(),
        &f.bindings,
        &f.policy,
        &destination,
        &AtomicBool::new(false),
    );
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    let tables = iresiphi::sink::read_bundle(&destination).unwrap();
    assert_eq!(tables["customer_sales"].len(), 6);
    let schema = &report.manifest["schema"]["customer_sales"];
    assert_eq!(schema["anchor"], "population:customer_month");
    assert_eq!(schema["grain"], json!(["CustNo", "month"]));
}

#[test]
fn populations_are_declared_explicitly_and_checked_statically() {
    let base = sales_fixture().doc;
    let variant = |change: &dyn Fn(&mut Json)| {
        let mut doc = base.clone();
        change(&mut doc);
        static_code(&doc)
    };
    assert_eq!(
        variant(&|d| d["populations"][0]["period"]
            .as_object_mut()
            .unwrap()
            .remove("values")
            .map(|_| ())
            .unwrap()),
        "INVALID_POPULATION",
        "a period set must be listed or named"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["period"]["from"] =
            json!({"source":"targets","field":"Period"})),
        "INVALID_POPULATION",
        "listed and sourced at once"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["period"]["values"] = json!(["2026-13"])),
        "INVALID_PERIOD"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["period"]["values"] = json!(["2026-01", "2026-01"])),
        "INVALID_POPULATION"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["period"]["field"] = json!("Share")),
        "INVALID_POPULATION",
        "the period field may not shadow a recipient field"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["recipients"] = json!("nowhere")),
        "MISSING_REFERENCE"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["id"] = json!("customers")),
        "DUPLICATE_ID"
    );
    assert_eq!(
        variant(
            &|d| d["populations"][0]["period"] = json!({"field":"month","calendar":"month","from":{"source":"transactions","field":"booked_at"}})
        ),
        "TYPE_MISMATCH",
        "a sourced period set reads calendar-month text"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["filter"] =
            equal(field("transactions", "buyer_ref"), text("A"))),
        "INVALID_PATH",
        "the recipient filter reads recipient fields only"
    );
    assert_eq!(
        variant(&|d| d["required_capabilities"] = json!([])),
        "UNDECLARED_CAPABILITY"
    );
    assert_eq!(
        variant(&|d| d["populations"][0]["period"]["calendar"] = json!("week")),
        "INVALID_RECIPE"
    );
    assert_eq!(
        variant(&|d| d["relationships"][0]["period"]["date"] = json!("gross")),
        "TYPE_MISMATCH"
    );
    assert_eq!(
        variant(&|d| d["relationships"][0]["period"]["key"] = json!("CustNo")),
        "INVALID_RELATIONSHIP",
        "the period key is the population's period field"
    );
    assert_eq!(
        variant(&|d| {
            d["relationships"][0]["from"] = json!("customers");
            d["relationships"][0]["period"]["key"] = json!("SalesArea");
        }),
        "INVALID_RELATIONSHIP",
        "period derivation joins finer records to a population"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["complete"] = json!(["nothing"])),
        "MISSING_REFERENCE"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["eligibility"] =
            equal(field("customer_month", "CustNo"), text("A"))),
        "INVALID_POPULATION",
        "recipient subsets belong to the population declaration"
    );
    assert_eq!(
        variant(&|d| {
            d["outputs"][0]["anchor"] = json!("customers");
            d["outputs"][0]["columns"] = json!([named("CustNo", field("customers", "CustNo"))]);
        }),
        "INVALID_POPULATION",
        "completeness is declared for population activity only"
    );
}

#[test]
fn recipes_without_slice_two_declarations_keep_their_canonical_form() {
    let root = tempfile::tempdir().unwrap();
    let (recipe, _, _) = invoice::fixture(root.path());
    let canonical = recipe.canonical_json().unwrap();
    for key in ["populations", "\"period\"", "\"complete\"", "allocations"] {
        assert!(!canonical.contains(key), "{key} leaked into {canonical}");
    }
    let saved = recipe.to_json().unwrap();
    assert!(!saved.contains("populations"), "{saved}");
}
