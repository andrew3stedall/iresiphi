use iresiphi::{adapters::read_sources, model::*};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn fixture(adapter: &str, contents: &[u8]) -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input");
    std::fs::write(&path, contents).unwrap();
    let recipe:Recipe=serde_json::from_value(serde_json::json!({"format_version":"0.1","name":"adapter","sources":[{"id":"lines","projection":{"array":"/lines","fields":{"id":{"path":"/id","data_type":{"kind":"string"}},"amount":{"path":"/amount","data_type":{"kind":"decimal","precision":20,"scale":2}},"supplier":{"path":"/supplier","header":true,"data_type":{"kind":"string"}}}},"identity":["id"],"grain":["id"]}],"outputs":[]})).unwrap();
    let bindings = BTreeMap::from([(
        "lines".into(),
        Binding {
            path,
            adapter: adapter.into(),
            settings: BTreeMap::new(),
        },
    )]);
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
        transfers: Default::default(),
    };
    (dir, recipe, bindings, policy)
}
#[test]
fn json_projects_header_and_preserves_exact_numeric_lexeme() {
    let (_dir, r, b, p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[{"id":"x","amount":9007199254740993.01}]}"#,
    );
    let rows = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    let row = &rows["lines"][0];
    assert_eq!(
        row.fields["amount"],
        Value::Decimal {
            value: "9007199254740993.01".into(),
            precision: 20,
            scale: 2
        }
    );
    assert_eq!(row.fields["supplier"], Value::String { value: "A".into() });
    assert_eq!(row.reference.location, "/lines/0");
    assert_eq!(row.raw["amount"].to_string(), "9007199254740993.01");
}
#[test]
fn duplicate_keys_are_rejected_even_outside_projection() {
    let (_dir, r, b, p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[],"unused":{"x":1,"x":2}}"#,
    );
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "DUPLICATE_JSON_KEY"
    );
}
#[test]
fn duplicate_identity_and_path_escape_fail() {
    let (_dir, r, mut b, p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[{"id":"x","amount":"1.00"},{"id":"x","amount":"2.00"}]}"#,
    );
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "DUPLICATE_IDENTITY"
    );
    b.get_mut("lines").unwrap().path = "/etc/passwd".into();
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "POLICY_DENIED"
    );
}
#[test]
fn cancellation_and_tiny_limits_fail_before_result() {
    let (_dir, r, b, mut p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[{"id":"x","amount":"1.00"}]}"#,
    );
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(true))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    p.limits.file_bytes = 1;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
    p.limits = Limits::default();
    p.limits.lookup_rows = 0;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
}
#[test]
fn csv_quoted_newline_empty_string_and_null_are_distinct() {
    let (_dir, mut r, b, p) = fixture(
        "csv",
        b"id,amount,supplier\nx,1.00,\"A\nB\"\ny,2.00,\nz,3.00,\\N\nw,4.00,\\\\N\n",
    );
    r.sources[0]
        .projection
        .fields
        .get_mut("supplier")
        .unwrap()
        .nullable = true;
    let rows = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    let rows = &rows["lines"];
    assert_eq!(
        rows[0].fields["supplier"],
        Value::String {
            value: "A\nB".into()
        }
    );
    assert_eq!(
        rows[1].fields["supplier"],
        Value::String { value: "".into() }
    );
    assert_eq!(rows[2].fields["supplier"], Value::Null);
    assert_eq!(
        rows[3].fields["supplier"],
        Value::String {
            value: "\\N".into()
        }
    );
}
#[test]
fn csv_ragged_records_and_invalid_dates_fail() {
    let (_dir, r, b, p) = fixture("csv", b"id,amount,supplier\nx,1.00\n");
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "INVALID_CSV"
    );
    let (_dir, mut r, b, p) = fixture("csv", b"id,amount,supplier\nx,1.00,2025-02-29\n");
    r.sources[0]
        .projection
        .fields
        .get_mut("supplier")
        .unwrap()
        .data_type = Type::Date;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "INVALID_VALUE"
    );
}
#[path = "support/workbook.rs"]
mod workbook_support;
use workbook_support::workbook_bytes as workbook;

fn xlsx_fixture(cell: &str, epoch1904: bool) -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let (d, r, mut b, p) = fixture("xlsx", &workbook(cell, epoch1904));
    b.get_mut("lines").unwrap().settings = BTreeMap::from([
        ("sheet".into(), serde_json::json!("Review")),
        ("header_row".into(), serde_json::json!(1)),
    ]);
    (d, r, b, p)
}
#[test]
fn xlsx_text_decimal_and_source_cell_survive() {
    let (_d, r, b, p) = xlsx_fixture(
        r#"<c r="B2" t="inlineStr"><is><t>9007199254740993.01</t></is></c>"#,
        false,
    );
    let result = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    let row = &result["lines"][0];
    assert_eq!(
        row.fields["amount"],
        Value::Decimal {
            value: "9007199254740993.01".into(),
            precision: 20,
            scale: 2
        }
    );
    assert_eq!(row.raw["amount"]["cell"], "Review!B2");
}
#[test]
fn xlsx_numeric_quantization_is_explicit_and_half_even() {
    let (_d, r, mut b, p) = xlsx_fixture(r#"<c r="B2"><v>1.015</v></c>"#, false);
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "XLSX_QUANTIZATION_REQUIRED"
    );
    b.get_mut("lines").unwrap().settings.insert(
        "decimal_quantization".into(),
        serde_json::json!({"amount":"half_even"}),
    );
    let result = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result["lines"][0].fields["amount"],
        Value::Decimal {
            value: "1.02".into(),
            precision: 20,
            scale: 2
        }
    );
}
#[test]
fn xlsx_formula_cache_is_required_and_not_evaluated() {
    let (_d, r, b, p) = xlsx_fixture(r#"<c r="B2"><f>1+1</f></c>"#, false);
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "FORMULA_CACHE_MISSING"
    );
    let (_d, r, mut b, p) = xlsx_fixture(r#"<c r="B2"><f>1+1</f><v>7</v></c>"#, false);
    b.get_mut("lines").unwrap().settings.insert(
        "decimal_quantization".into(),
        serde_json::json!({"amount":"half_even"}),
    );
    let result = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result["lines"][0].fields["amount"],
        Value::Decimal {
            value: "7.00".into(),
            precision: 20,
            scale: 2
        }
    );
}
#[test]
fn xlsx_epochs_and_invalid_serials() {
    for (epoch, serial, expected) in [(false, "61", "1900-03-01"), (true, "0", "1904-01-01")] {
        let (_d, mut r, mut b, p) =
            xlsx_fixture(&format!(r#"<c r="B2"><v>{serial}</v></c>"#), epoch);
        r.sources[0]
            .projection
            .fields
            .get_mut("amount")
            .unwrap()
            .data_type = Type::Date;
        b.get_mut("lines")
            .unwrap()
            .settings
            .insert("excel_dates".into(), serde_json::json!(["amount"]));
        let result = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            result["lines"][0].fields["amount"],
            Value::Date {
                value: expected.into()
            }
        );
    }
    for serial in ["60", "61.5"] {
        let (_d, mut r, mut b, p) =
            xlsx_fixture(&format!(r#"<c r="B2"><v>{serial}</v></c>"#), false);
        r.sources[0]
            .projection
            .fields
            .get_mut("amount")
            .unwrap()
            .data_type = Type::Date;
        b.get_mut("lines")
            .unwrap()
            .settings
            .insert("excel_dates".into(), serde_json::json!(["amount"]));
        assert_eq!(
            read_sources(&r, &b, &p, &AtomicBool::new(false))
                .unwrap_err()
                .code,
            "INVALID_DATE"
        );
    }
}
#[test]
fn xlsx_decompression_and_range_limits_fail() {
    let (_d, r, b, mut p) =
        xlsx_fixture(r#"<c r="B2" t="inlineStr"><is><t>1.00</t></is></c>"#, false);
    p.limits.xlsx_bytes = 20;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
    p.limits = Limits::default();
    p.limits.worksheet_cells = 5;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
}
#[test]
fn symlink_escape_is_rejected_even_from_inside_root() {
    let (dir, r, mut b, p) = fixture("json", b"{}");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();
    b.get_mut("lines").unwrap().path = link;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "POLICY_DENIED"
    );
}
#[test]
fn json_null_identity_depth_and_token_limits_fail() {
    let (_dir, r, b, p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[{"id":null,"amount":"1.00"}]}"#,
    );
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "NULL_REQUIRED"
    );
    let (_dir, r, b, mut p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[{"id":"x","amount":"1.00"}]}"#,
    );
    p.limits.json_depth = 2;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
    p.limits = Limits::default();
    p.limits.field_bytes = 3;
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
}
#[test]
fn json_exponent_is_exact_and_fingerprint_tracks_read_bytes() {
    let (dir, r, b, p) = fixture(
        "json",
        br#"{"supplier":"A","lines":[{"id":"x","amount":1.01e2}]}"#,
    );
    let first = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        first["lines"][0].fields["amount"],
        Value::Decimal {
            value: "101.00".into(),
            precision: 20,
            scale: 2
        }
    );
    std::fs::write(
        dir.path().join("input"),
        br#"{"supplier":"A","lines":[{"id":"x","amount":1.02e2}]}"#,
    )
    .unwrap();
    let second = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    assert_ne!(
        first["lines"][0].reference.fingerprint,
        second["lines"][0].reference.fingerprint
    );
}
#[test]
fn xlsx_empty_cached_string_is_not_missing_cache_or_null() {
    let (_dir, mut r, b, p) = xlsx_fixture(r#"<c r="B2" t="str"><f>""</f><v></v></c>"#, false);
    r.sources[0]
        .projection
        .fields
        .get_mut("amount")
        .unwrap()
        .data_type = Type::String;
    let rows = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        rows["lines"][0].fields["amount"],
        Value::String { value: "".into() }
    );
}
#[test]
fn xlsx_negative_ties_and_tiny_numbers_follow_declared_rounding() {
    for (number, rounding, expected) in [
        ("-1.015", "half_even", "-1.02"),
        ("-1.015", "toward_zero", "-1.01"),
        ("1e-100", "half_even", "0.00"),
    ] {
        let (_dir, r, mut b, p) = xlsx_fixture(&format!(r#"<c r="B2"><v>{number}</v></c>"#), false);
        b.get_mut("lines").unwrap().settings.insert(
            "decimal_quantization".into(),
            serde_json::json!({"amount":rounding}),
        );
        let rows = read_sources(&r, &b, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            rows["lines"][0].fields["amount"],
            Value::Decimal {
                value: expected.into(),
                precision: 20,
                scale: 2
            }
        );
    }
}
#[test]
fn xlsx_error_cells_fail() {
    let (_dir, r, b, p) = xlsx_fixture(r#"<c r="B2" t="e"><v>#DIV/0!</v></c>"#, false);
    assert_eq!(
        read_sources(&r, &b, &p, &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "XLSX_CELL_ERROR"
    );
}
