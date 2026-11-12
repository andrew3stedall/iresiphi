//! Ticket #29: allocation as a distinct declared operation over an explicit
//! population, with exact shares, residual policies, per-partition conservation,
//! the edge-case diagnostics, filter timing and evidence, through the public
//! validate/explain/run seam. Contract: docs/spec/slice-2.md.
#[path = "support/customer_month.rs"]
mod customer_month;
use customer_month::*;
use iresiphi::{CancellationToken, ExecutionContext, explain};
use serde_json::{Value as Json, json};
use std::sync::atomic::AtomicBool;

fn allocated<'a>(table: &'a iresiphi::engine::Table, customer: &str, month: &str) -> Option<&'a str> {
    cell(table, customer, month, "allocated_target")
}

fn partition<'a>(report: &'a iresiphi::RunReport, territory: &str, period: &str) -> &'a Json {
    report
        .allocation_evidence
        .iter()
        .find(|p| {
            p["partition"]["Territory"] == territory && p["partition"]["Period"] == period
        })
        .unwrap_or_else(|| panic!("no partition {territory}/{period}"))
}

/// The evidence record of one member's allocation.
fn recipient<'a>(report: &'a iresiphi::RunReport, customer: &str, month: &str) -> &'a Json {
    let record = report
        .evidence
        .iter()
        .find(|r| {
            r["member"]["CustNo"] == customer && r["member"]["month"] == month
        })
        .unwrap_or_else(|| panic!("no evidence for {customer}/{month}"));
    &record["allocations"]["allocated_target"]
}

#[test]
fn the_customer_month_fixture_allocates_regional_targets_by_share() {
    let f = fixture();
    assert_eq!(static_code(&f.doc), "valid");
    let report = f.succeeded();
    assert_eq!(report.status, "succeeded");
    let table = &report.tables["customer_targets"];
    assert_eq!(table.len(), 6);
    for (customer, january, february) in [
        ("A", "225.00", "200.00"),
        ("B", "675.00", "600.00"),
        ("C", "300.00", "400.00"),
    ] {
        assert_eq!(allocated(table, customer, "2026-01"), Some(january));
        assert_eq!(allocated(table, customer, "2026-02"), Some(february));
    }
    // Sales are unchanged by adding the allocation.
    assert_eq!(cell(table, "B", "2026-02", "sales"), Some("120.00"));
    assert_eq!(cell(table, "A", "2026-02", "sales"), Some("0.00"));

    // Broadcast context repeats the regional goal on every member.
    for (customer, january, february) in [
        ("A", "900.00", "800.00"),
        ("B", "900.00", "800.00"),
        ("C", "300.00", "400.00"),
    ] {
        assert_eq!(cell(table, customer, "2026-01", "target_context"), Some(january));
        assert_eq!(cell(table, customer, "2026-02", "target_context"), Some(february));
    }

    let totals = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"];
    let columns = &totals["columns"];
    assert_eq!(columns["sales"]["population"], "470.00");
    // The six-row broadcast sum (4100) is never reported as a target total.
    assert!(columns.get("target_context").is_none(), "{columns}");
    assert!(!report.manifest["totals"].to_string().contains("4100"));
    assert_eq!(
        totals["excluded"]["target_context"]["transition"],
        "broadcast"
    );
    // The allocated total sits beside the source total it conserves.
    let target = &columns["allocated_target"];
    assert_eq!(target["population"], "2400.00");
    assert_eq!(target["allocation"]["source_total"], "2400.00");
    assert_eq!(target["allocation"]["assigned_total"], "2400.00");
    assert_eq!(target["allocation"]["residual_total"], "0.00");
    assert_eq!(target["allocation"]["conserved"], true);

    // Conservation per partition, in the manifest and in evidence.
    let manifest = &report.manifest["allocations"]["customer_targets"]["allocated_target"];
    assert_eq!(manifest["check"]["id"], "allocation.conservation");
    assert_eq!(manifest["check"]["status"], "passed");
    assert_eq!(manifest["partitions"].as_array().unwrap().len(), 4);
    assert!(
        report.manifest["check_results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "allocation.conservation" && c["status"] == "passed")
    );
    assert_eq!(report.allocation_evidence.len(), 4);
    let north = partition(&report, "North", "2026-01");
    assert_eq!(north["quantity"], "900.00");
    assert_eq!(north["recipient_count"], 2);
    assert_eq!(north["weight_sum"], "4");
    assert_eq!(north["assigned_sum"], "900.00");
    assert_eq!(north["residual"], "0.00");
    assert_eq!(north["policy"], "largest_remainder");
    assert_eq!(north["conserved"], true);

    let a = recipient(&report, "A", "2026-01");
    assert_eq!(a["weight"], "1");
    assert_eq!(a["exact_share"], "225");
    assert_eq!(a["exact_share_terminates"], true);
    assert_eq!(a["assigned"], "225.00");
    assert_eq!(a["status"], "allocated");
}

#[test]
fn schema_marks_allocated_and_broadcast_columns() {
    let report = fixture().succeeded();
    let columns = report.manifest["schema"]["customer_targets"]["columns"]
        .as_array()
        .unwrap()
        .clone();
    let column = |name: &str| columns.iter().find(|c| c["name"] == name).unwrap().clone();
    let allocated = column("allocated_target");
    assert_eq!(allocated["role"], "allocation");
    assert_eq!(allocated["ownership"], "allocated");
    assert_eq!(allocated["additive"], true);
    assert_eq!(allocated["partition"], json!(["Territory", "Period"]));
    assert_eq!(allocated["type"], decimal(18, 2));
    let origin = &allocated["origins"][0];
    assert_eq!(origin["source"], "source:targets");
    assert_eq!(origin["field"], "Goal");
    assert_eq!(origin["origin_grain"], json!(["Territory", "Period"]));
    assert_eq!(origin["transition"], "allocated");
    let context = column("target_context");
    assert_eq!(context["ownership"], "broadcast");
    assert_eq!(context["additive"], false);
    assert_eq!(context["origins"][0]["transition"], "broadcast");
    assert_eq!(column("sales")["additive"], true);
}

#[test]
fn selecting_a_after_allocation_keeps_its_share() {
    let mut f = fixture();
    f.output()["filter"] = equal(field("customer_targets", "CustNo"), text("A"));
    let report = f.succeeded();
    let table = &report.tables["customer_targets"];
    assert_eq!(table.len(), 2);
    assert_eq!(allocated(table, "A", "2026-01"), Some("225.00"));
    assert_eq!(allocated(table, "A", "2026-02"), Some("200.00"));
    // The partitions still conserve the whole source total.
    let column = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]["allocated_target"];
    assert_eq!(column["population"], "425.00");
    assert_eq!(column["allocation"]["source_total"], "2400.00");
    assert_eq!(column["allocation"]["assigned_total"], "2400.00");

    // The output filter may also read the allocated value itself.
    f.output()["filter"] = json!({"op":"less","left":field("customer_targets","allocated_target"),"right":{"op":"literal","value":{"kind":"decimal","value":"250","precision":3,"scale":0}}});
    let table = &f.succeeded().tables["customer_targets"];
    assert_eq!(table.len(), 2);
    assert_eq!(allocated(table, "A", "2026-01"), Some("225.00"));
}

#[test]
fn a_recipient_filter_runs_before_normalization_and_changes_shares() {
    let mut f = fixture();
    f.allocation()["recipient_filter"] = or(
        equal(field("customer_month", "CustNo"), text("A")),
        equal(field("customer_month", "SalesArea"), text("South")),
    );
    let report = f.succeeded();
    let table = &report.tables["customer_targets"];
    assert_eq!(table.len(), 6, "the filter changes recipients, not the population");
    assert_eq!(allocated(table, "A", "2026-01"), Some("900.00"));
    assert_eq!(allocated(table, "A", "2026-02"), Some("800.00"));
    assert_eq!(allocated(table, "B", "2026-01"), None);
    assert_eq!(allocated(table, "C", "2026-01"), Some("300.00"));
    assert_eq!(recipient(&report, "B", "2026-01")["status"], "not_recipient");
    assert_eq!(partition(&report, "North", "2026-01")["recipient_count"], 1);
    let column = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]["allocated_target"];
    assert_eq!(column["allocation"]["assigned_total"], "2400.00");
}

#[test]
fn pruning_a_member_from_the_output_keeps_its_weight_in_the_denominator() {
    let mut f = fixture();
    f.output()["filter"] = not(equal(field("customer_targets", "CustNo"), text("B")));
    let report = f.succeeded();
    let table = &report.tables["customer_targets"];
    assert_eq!(table.len(), 4);
    assert_eq!(allocated(table, "A", "2026-01"), Some("225.00"), "not 900");
    assert_eq!(partition(&report, "North", "2026-01")["weight_sum"], "4");
    assert_eq!(partition(&report, "North", "2026-01")["recipient_count"], 2);
}

#[test]
fn explain_names_each_filter_stage_and_the_denominator() {
    let mut f = fixture();
    f.allocation()["recipient_filter"] = equal(field("customer_month", "SalesArea"), text("North"));
    f.allocation()["unallocated"] = json!("report");
    f.output()["filter"] = equal(field("customer_targets", "CustNo"), text("A"));
    let explanation = explain(&f.recipe());
    assert_eq!(explanation["validation"]["valid"], true, "{}", explanation["validation"]);
    let nodes = explanation["nodes"].as_array().unwrap();
    let node = |id: &str| {
        nodes
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("no node {id}"))
            .clone()
    };
    let allocation = node("output:customer_targets/allocation:allocated_target");
    assert_eq!(allocation["kind"], "allocation");
    assert_eq!(allocation["quantity"], "source:targets/field:Goal");
    assert_eq!(allocation["partition"], json!(["Territory", "Period"]));
    assert_eq!(allocation["via"], "relationship:region_target");
    assert_eq!(allocation["residual"], "largest_remainder");
    assert_eq!(allocation["null_weight"], "error");
    assert_eq!(allocation["unallocated"], "report");
    assert!(
        allocation["denominator"]
            .as_str()
            .unwrap()
            .contains("output filter"),
        "{allocation}"
    );
    assert_eq!(
        allocation["recipient_filter"]["applies"],
        "before_normalization"
    );
    let filter = node("output:customer_targets/filter");
    assert_eq!(filter["applies"], "after_allocation");
    assert!(filter["retains"].as_str().unwrap().contains("never changes shares"));
    let output = node("output:customer_targets");
    assert!(
        output["columns"]
            .as_array()
            .unwrap()
            .contains(&json!("output:customer_targets/allocation:allocated_target"))
    );
}

/// Three North customers of equal weight sharing one 2.00 target.
fn thirds(residual: &str) -> Fixture {
    let mut f = fixture();
    f.write(
        "customers",
        "CustNo,SalesArea,Share\nA,North,1\nB,North,1\nD,North,1\n",
    );
    f.write("targets", "Territory,Period,Goal\nNorth,2026-01,2\nNorth,2026-02,1\n");
    f.allocation()["residual"] = json!(residual);
    f
}

#[test]
fn largest_remainder_distributes_last_place_units_by_remainder_then_identity() {
    let report = thirds("largest_remainder").succeeded();
    let table = &report.tables["customer_targets"];
    // 2.00 / 3: each floors to 0.66; two units remain and the remainders tie,
    // so they go to A and B by ascending identity.
    assert_eq!(allocated(table, "A", "2026-01"), Some("0.67"));
    assert_eq!(allocated(table, "B", "2026-01"), Some("0.67"));
    assert_eq!(allocated(table, "D", "2026-01"), Some("0.66"));
    // 1.00 / 3: one unit remains and goes to A.
    assert_eq!(allocated(table, "A", "2026-02"), Some("0.34"));
    assert_eq!(allocated(table, "B", "2026-02"), Some("0.33"));
    assert_eq!(allocated(table, "D", "2026-02"), Some("0.33"));
    let january = partition(&report, "North", "2026-01");
    assert_eq!(january["assigned_sum"], "2.00");
    assert_eq!(january["residual"], "0.00");
    // A third does not terminate: no exact text, a flag instead.
    let share = recipient(&report, "A", "2026-01");
    assert_eq!(share["exact_share"], Json::Null);
    assert_eq!(share["exact_share_terminates"], false);
}

#[test]
fn largest_remainder_prefers_the_largest_fractional_remainder() {
    let f = fixture();
    f.write(
        "customers",
        "CustNo,SalesArea,Share\nA,North,1\nB,North,2\nD,North,4\n",
    );
    f.write("targets", "Territory,Period,Goal\nNorth,2026-01,1\nNorth,2026-02,1\n");
    let table = &f.succeeded().tables["customer_targets"];
    // 1.00 × 1/7, 2/7, 4/7 = 0.1428…, 0.2857…, 0.5714…: floors 0.14, 0.28, 0.57
    // leave one unit, which goes to B (remainder 0.57…) ahead of A (0.28…).
    assert_eq!(allocated(table, "A", "2026-01"), Some("0.14"));
    assert_eq!(allocated(table, "B", "2026-01"), Some("0.29"));
    assert_eq!(allocated(table, "D", "2026-01"), Some("0.57"));
}

#[test]
fn an_exact_share_longer_than_38_digits_is_flagged_not_written() {
    let f = fixture();
    // Weights 1 and 2^62 - 1 sum to 2^62, so each exact share terminates only
    // after 62 decimal places.
    f.write(
        "customers",
        "CustNo,SalesArea,Share\nA,North,1\nB,North,4611686018427387903\nC,South,1\n",
    );
    f.write("targets", "Territory,Period,Goal\nNorth,2026-01,1\nSouth,2026-01,1\nNorth,2026-02,1\nSouth,2026-02,1\n");
    let report = f.succeeded();
    let share = recipient(&report, "A", "2026-01");
    assert_eq!(share["exact_share"], Json::Null);
    assert_eq!(share["exact_share_terminates"], false);
    assert_eq!(share["exact_share_status"], "exceeds_38_digits");
    assert_eq!(share["assigned"], "0.00");
    assert_eq!(recipient(&report, "B", "2026-01")["assigned"], "1.00");
    assert_eq!(recipient(&report, "C", "2026-01")["exact_share"], "1");
}

#[test]
fn report_policy_quantizes_half_even_and_reports_a_possibly_negative_residual() {
    let report = thirds("report").succeeded();
    let table = &report.tables["customer_targets"];
    for customer in ["A", "B", "D"] {
        assert_eq!(allocated(table, customer, "2026-01"), Some("0.67"));
        assert_eq!(allocated(table, customer, "2026-02"), Some("0.33"));
    }
    let january = partition(&report, "North", "2026-01");
    assert_eq!(january["assigned_sum"], "2.01");
    assert_eq!(january["residual"], "-0.01");
    assert_eq!(january["policy"], "report");
    assert_eq!(january["conserved"], true);
    assert_eq!(partition(&report, "North", "2026-02")["residual"], "0.01");
    let column = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]["allocated_target"];
    assert_eq!(column["allocation"]["source_total"], "3.00");
    assert_eq!(column["allocation"]["assigned_total"], "3.00");
    assert_eq!(column["allocation"]["residual_total"], "0.00");
}

#[test]
fn decimal_weights_and_negative_quantities_allocate_exactly() {
    let mut f = fixture();
    f.doc["sources"][0]["projection"]["fields"]["Share"]["data_type"] = decimal(6, 3);
    f.write(
        "customers",
        "CustNo,SalesArea,Share\nA,North,0.5\nB,North,1.5\nC,South,0.125\n",
    );
    f.write(
        "targets",
        "Territory,Period,Goal\nNorth,2026-01,-10\nSouth,2026-01,1\nNorth,2026-02,0\nSouth,2026-02,0.01\n",
    );
    let report = f.succeeded();
    let table = &report.tables["customer_targets"];
    assert_eq!(allocated(table, "A", "2026-01"), Some("-2.50"));
    assert_eq!(allocated(table, "B", "2026-01"), Some("-7.50"));
    assert_eq!(allocated(table, "C", "2026-01"), Some("1.00"));
    assert_eq!(allocated(table, "A", "2026-02"), Some("0.00"));
    assert_eq!(allocated(table, "C", "2026-02"), Some("0.01"));
    assert_eq!(recipient(&report, "B", "2026-01")["exact_share"], "-7.5");
    assert_eq!(partition(&report, "North", "2026-01")["weight_sum"], "2.000");
}

// --- Edge cases (contract clause 6) ---------------------------------------------

#[test]
fn a_null_weight_is_an_error_unless_declared_zero() {
    let mut f = fixture();
    f.doc["sources"][0]["projection"]["fields"]["Share"]["nullable"] = json!(true);
    f.write("customers", "CustNo,SalesArea,Share\nA,North,1\nB,North,\\N\nC,South,1\n");
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "ALLOCATION_NULL_WEIGHT");
    assert_eq!(error.affected_count, Some(2));
    assert_eq!(error.subject.as_deref(), Some("output:customer_targets/allocation:allocated_target"));

    f.allocation()["null_weight"] = json!("zero");
    let report = f.succeeded();
    let table = &report.tables["customer_targets"];
    assert_eq!(allocated(table, "A", "2026-01"), Some("900.00"));
    assert_eq!(allocated(table, "B", "2026-01"), Some("0.00"));
    assert_eq!(recipient(&report, "B", "2026-01")["weight"], Json::Null);
    assert_eq!(recipient(&report, "B", "2026-01")["weight_rule"], "declared_zero");
    assert_eq!(partition(&report, "North", "2026-01")["recipient_count"], 2);
}

#[test]
fn a_negative_weight_is_always_an_error() {
    let mut f = fixture();
    f.write("customers", "CustNo,SalesArea,Share\nA,North,1\nB,North,-3\nC,South,1\n");
    assert_eq!(failure(&f.run()).code, "ALLOCATION_NEGATIVE_WEIGHT");
    f.allocation()["null_weight"] = json!("zero");
    f.allocation()["unallocated"] = json!("report");
    assert_eq!(failure(&f.run()).code, "ALLOCATION_NEGATIVE_WEIGHT");
}

#[test]
fn a_zero_weight_partition_is_an_error_unless_declared_unallocated() {
    let mut f = fixture();
    f.write("customers", "CustNo,SalesArea,Share\nA,North,0\nB,North,0\nC,South,1\n");
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "ALLOCATION_ZERO_WEIGHT");
    assert_eq!(error.affected_count, Some(2), "two North partitions");

    f.allocation()["unallocated"] = json!("report");
    let report = f.succeeded();
    let table = &report.tables["customer_targets"];
    assert_eq!(allocated(table, "A", "2026-01"), None);
    assert_eq!(allocated(table, "B", "2026-02"), None);
    assert_eq!(allocated(table, "C", "2026-01"), Some("300.00"));
    let north = partition(&report, "North", "2026-01");
    assert_eq!(north["status"], "unallocated");
    assert_eq!(north["assigned_sum"], "0.00");
    assert_eq!(north["residual"], "900.00");
    assert_eq!(north["conserved"], true);
    assert_eq!(recipient(&report, "A", "2026-01")["status"], "unallocated");
    let column = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]["allocated_target"];
    assert_eq!(column["allocation"]["source_total"], "2400.00");
    assert_eq!(column["allocation"]["assigned_total"], "700.00");
    assert_eq!(column["allocation"]["residual_total"], "1700.00");
}

#[test]
fn a_partition_without_recipients_is_an_error_unless_declared_unallocated() {
    let mut f = fixture();
    f.write("targets", &format!("{TARGETS}East,2026-01,50\n"));
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "ALLOCATION_NO_RECIPIENTS");
    assert_eq!(error.affected_count, Some(1));
    assert_eq!(error.samples.len(), 1);

    f.allocation()["unallocated"] = json!("report");
    let report = f.succeeded();
    let east = partition(&report, "East", "2026-01");
    assert_eq!(east["recipient_count"], 0);
    assert_eq!(east["residual"], "50.00");
    assert_eq!(east["status"], "unallocated");
    let column = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]["allocated_target"];
    assert_eq!(column["allocation"]["source_total"], "2450.00");
    assert_eq!(column["allocation"]["residual_total"], "50.00");

    // A recipient filter that removes every recipient of a partition empties it too.
    let mut f = fixture();
    f.allocation()["recipient_filter"] = equal(field("customer_month", "SalesArea"), text("North"));
    assert_eq!(failure(&f.run()).code, "ALLOCATION_NO_RECIPIENTS");
}

#[test]
fn a_missing_target_leaves_the_recipient_null_and_unresolved_never_zero() {
    let f = fixture();
    f.write(
        "targets",
        "Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\nSouth,2026-02,\\N\n",
    );
    let report = f.succeeded();
    assert_eq!(report.status, "succeeded_with_unresolved");
    let table = &report.tables["customer_targets"];
    assert_eq!(allocated(table, "C", "2026-02"), None);
    assert_eq!(cell(table, "C", "2026-02", "_status"), Some("unmatched"));
    assert_eq!(cell(table, "A", "2026-02", "_status"), Some("matched"));
    assert_eq!(recipient(&report, "C", "2026-02")["status"], "missing_quantity");
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "ALLOCATION_MISSING_QUANTITY")
        .unwrap();
    assert_eq!(diagnostic.severity, "warning");
    assert_eq!(diagnostic.affected_count, Some(1));

    // No target record at all: the via lookup is unmatched.
    f.write(
        "targets",
        "Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\n",
    );
    let report = f.succeeded();
    assert_eq!(report.status, "succeeded_with_unresolved");
    let table = &report.tables["customer_targets"];
    assert_eq!(allocated(table, "C", "2026-02"), None);
    assert_eq!(cell(table, "C", "2026-02", "_status"), Some("unmatched"));
    assert!(
        value_text(&table.iter().find(|r| value_text(&r["CustNo"]) == Some("C") && value_text(&r["month"]) == Some("2026-02")).unwrap()["_reason"])
            .unwrap()
            .contains("allocated_target")
    );
    let column = &report.manifest["totals"]["by_output"]["customer_targets"]["AUD"]["columns"]["allocated_target"];
    assert_eq!(column["population"], Json::Null, "a null member leaves no complete total");
    assert_eq!(column["allocation"]["source_total"], "2000.00");
    assert_eq!(column["allocation"]["assigned_total"], "2000.00");
}

#[test]
fn duplicate_partition_keys_are_an_identity_violation() {
    let mut f = fixture();
    f.write("targets", &format!("{TARGETS}North,2026-01,5\n"));
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "DUPLICATE_IDENTITY");
    assert_eq!(error.subject.as_deref(), Some("source:targets"));

    // Also when the source's identity is a separate key and only the partition repeats.
    f.doc["sources"][2]["identity"] = json!(["Goal_id"]);
    f.doc["sources"][2]["projection"]["fields"]["Goal_id"] = column("Goal_id", string());
    f.write(
        "targets",
        "Goal_id,Territory,Period,Goal\ng1,North,2026-01,900\ng2,South,2026-01,300\ng3,North,2026-02,800\ng4,South,2026-02,400\ng5,North,2026-01,5\n",
    );
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "DUPLICATE_IDENTITY");
    assert_eq!(error.check.as_deref(), Some("data.identity"));
    assert!(error.message.contains("partition"), "{}", error.message);
}

#[test]
fn mixed_currency_between_quantity_and_recipient_is_rejected() {
    let mut f = fixture();
    f.doc["sources"][0]["projection"]["fields"]["currency"] = column("currency", string());
    f.doc["sources"][2]["projection"]["fields"]["currency"] = column("currency", string());
    f.output()["columns"][2] = named("currency", field("customer_month", "currency"));
    f.write(
        "customers",
        "CustNo,SalesArea,Share,currency\nA,North,1,AUD\nB,North,3,AUD\nC,South,1,AUD\n",
    );
    f.write(
        "targets",
        "Territory,Period,Goal,currency\nNorth,2026-01,900,AUD\nSouth,2026-01,300,USD\nNorth,2026-02,800,AUD\nSouth,2026-02,400,AUD\n",
    );
    assert_eq!(static_code(&f.doc), "valid");
    let report = f.run();
    let error = failure(&report);
    assert_eq!(error.code, "MIXED_CURRENCY");
    assert_eq!(error.affected_count, Some(1));

    // A currency-carrying quantity needs a currency-carrying recipient.
    let mut f = fixture();
    f.doc["sources"][2]["projection"]["fields"]["currency"] = column("currency", string());
    assert_eq!(static_code(&f.doc), "GRAIN_VIOLATION");
}

#[test]
fn allocation_is_declared_explicitly_and_checked_statically() {
    let base = fixture().doc;
    let variant = |change: &dyn Fn(&mut Json)| {
        let mut doc = base.clone();
        change(&mut doc);
        static_code(&doc)
    };
    assert_eq!(
        variant(&|d| d["required_capabilities"] = json!(["population"])),
        "UNDECLARED_CAPABILITY"
    );
    assert_eq!(
        variant(&|d| {
            d["outputs"][0]["allocations"][0]
                .as_object_mut()
                .unwrap()
                .remove("residual");
        }),
        "INVALID_RECIPE",
        "a residual policy must be declared"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["residual"] = json!("nearest")),
        "INVALID_RECIPE"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["partition"] = json!(["Territory"])),
        "INVALID_ALLOCATION",
        "the partition is the quantity source's grain"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["quantity"] =
            json!({"source":"transactions","field":"gross"})),
        "INVALID_ALLOCATION",
        "the via relationship reaches the quantity source"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["via"] = json!("sales")),
        "INVALID_ALLOCATION"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["quantity"]["field"] = json!("Territory")),
        "TYPE_MISMATCH"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["scale"] = json!(1)),
        "INVALID_ALLOCATION",
        "a result scale below the quantity's cannot conserve it"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["weight"] = field("customer_month", "SalesArea")),
        "TYPE_MISMATCH"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["recipient_filter"] = field("customer_month", "Share")),
        "TYPE_MISMATCH"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["name"] = json!("sales")),
        "DUPLICATE_ID"
    );
    assert_eq!(
        variant(&|d| d["outputs"][0]["allocations"][0]["weight"] =
            json!({"op":"divide","left":field("customer_month","Share"),"right":field("customer_month","Share")})),
        "UNSUPPORTED_OPERATION",
        "general division stays rejected"
    );
    // Allocation needs an explicit population; a line output cannot allocate.
    assert_eq!(
        variant(&|d| {
            d["outputs"][0]["anchor"] = json!("customers");
            d["outputs"][0].as_object_mut().unwrap().remove("complete");
            d["outputs"][0]["columns"] = json!([named("CustNo", field("customers", "CustNo"))]);
        }),
        "INVALID_ALLOCATION"
    );
    // A population output keeps one row per member; it is not regrouped, so a
    // broadcast goal cannot be summed back into something that looks allocated.
    assert_eq!(
        variant(&|d| {
            d["outputs"][0]["group_by"] = json!(["currency"]);
            d["outputs"][0]["aggregates"] =
                json!([{"name":"goal","function":"sum","expr":via("targets","Goal",&["region_target"])}]);
        }),
        "INVALID_POPULATION"
    );
}

// --- Budgets, cancellation and delivery (clause 11) ------------------------------

#[test]
fn bundles_carry_allocation_evidence_and_cancellation_applies_unchanged() {
    let f = fixture();
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
    assert_eq!(tables["customer_targets"].len(), 6);
    assert_eq!(tables["_allocation_evidence"].len(), 4);
    let manifest: Json =
        serde_json::from_slice(&std::fs::read(destination.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest["allocations"]["customer_targets"]["allocated_target"]["check"]["status"],
        "passed"
    );

    let token = CancellationToken::new();
    token.cancel();
    let report = ExecutionContext::default().run(&f.recipe(), &f.bindings, &f.policy, &token);
    assert_eq!(report.status, "cancelled");
    assert!(report.allocation_evidence.is_empty());
}

#[test]
fn allocated_values_count_toward_the_output_budget() {
    let mut f = fixture();
    f.policy.limits.output_bytes = 2_000;
    assert_eq!(failure(&f.run()).code, "RESOURCE_LIMIT");
}
