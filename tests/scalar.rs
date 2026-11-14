//! Decision #6: exact scalar language, exercised only through public seams
//! (`eval`, `validate`, `ExecutionContext::run`).
use iresiphi::{
    ExecutionContext,
    eval::eval,
    model::*,
    scalar::Rounding,
    validation::{Report, validate},
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn recipe() -> Recipe {
    serde_json::from_str(r#"{"format_version":"0.1","name":"scalar","sources":[{"id":"lines","identity":["id"],"grain":["id"],"projection":{"fields":{"id":{"path":"/id","data_type":{"kind":"string"}}}}}],"outputs":[{"id":"review","anchor":"lines","columns":[{"name":"id","expr":{"op":"field","source":"lines","field":"id"}}]}]}"#).unwrap()
}
fn b(e: Expr) -> Box<Expr> {
    Box::new(e)
}
fn lit(value: Value) -> Expr {
    Expr::Literal { value }
}
fn dec(value: &str, precision: u8, scale: u8) -> Expr {
    lit(dv(value, precision, scale))
}
fn dv(value: &str, precision: u8, scale: u8) -> Value {
    Value::Decimal {
        value: value.into(),
        precision,
        scale,
    }
}
fn int(value: &str) -> Expr {
    lit(iv(value))
}
fn iv(value: &str) -> Value {
    Value::Int64 {
        value: value.into(),
    }
}
fn s(value: &str) -> Expr {
    lit(sv(value))
}
fn sv(value: &str) -> Value {
    Value::String {
        value: value.into(),
    }
}
fn date(value: &str) -> Expr {
    lit(Value::Date {
        value: value.into(),
    })
}
fn boolean(value: Option<bool>) -> Expr {
    lit(bool_value(value))
}
fn bool_value(value: Option<bool>) -> Value {
    value
        .map(|value| Value::Boolean { value })
        .unwrap_or(Value::Null)
}
fn null() -> Expr {
    lit(Value::Null)
}
fn add(l: Expr, r: Expr) -> Expr {
    Expr::Add {
        left: b(l),
        right: b(r),
    }
}
fn sub(l: Expr, r: Expr) -> Expr {
    Expr::Subtract {
        left: b(l),
        right: b(r),
    }
}
fn mul(l: Expr, r: Expr) -> Expr {
    Expr::Multiply {
        left: b(l),
        right: b(r),
    }
}
fn eq(l: Expr, r: Expr) -> Expr {
    Expr::Equal {
        left: b(l),
        right: b(r),
    }
}
fn lt(l: Expr, r: Expr) -> Expr {
    Expr::Less {
        left: b(l),
        right: b(r),
    }
}
fn le(l: Expr, r: Expr) -> Expr {
    Expr::LessEqual {
        left: b(l),
        right: b(r),
    }
}
fn quantize(e: Expr, precision: u8, scale: u8, rounding: Rounding) -> Expr {
    Expr::Quantize {
        expr: b(e),
        precision,
        scale,
        rounding,
    }
}
fn cast(e: Expr, data_type: Type) -> Expr {
    Expr::Cast {
        expr: b(e),
        data_type,
    }
}
fn decimal_type(precision: u8, scale: u8) -> Type {
    Type::Decimal { precision, scale }
}
fn run(e: &Expr) -> Result<Value> {
    eval(e, &recipe(), &mut |_, _, _| panic!("no source access"))
}
fn ok(e: &Expr) -> Value {
    run(e).unwrap_or_else(|error| panic!("{error:?}"))
}
fn code(e: &Expr) -> String {
    run(e).expect_err("expression must fail").code
}
/// Place an expression in a feature and return the first static diagnostic code.
fn static_code(e: Expr) -> Option<String> {
    let mut r = recipe();
    r.features.push(Feature {
        id: "probe".into(),
        expr: e,
    });
    first_error(&validate(&r))
}
fn first_error(report: &Report) -> Option<String> {
    let code = report
        .checks
        .iter()
        .find_map(|c| c.diagnostic.as_ref().map(|d| d.code.clone()));
    assert_eq!(report.valid, code.is_none(), "{report:?}");
    code
}

#[test]
fn decision6_invoice_examples_multiply_exactly_then_quantize_half_even() {
    // quantity decimal(12,4) * rate decimal(14,6) is exact decimal(26,10).
    let product = mul(dec("2", 12, 4), dec("50", 14, 6));
    assert_eq!(ok(&product), dv("100.0000000000", 26, 10));
    let expected = quantize(product, 18, 2, Rounding::HalfEven);
    assert_eq!(ok(&expected), dv("100.00", 18, 2));
    // billed decimal(18,2) - expected decimal(18,2) is decimal(19,2).
    assert_eq!(ok(&sub(dec("110", 18, 2), expected)), dv("10.00", 19, 2));
    let expected = quantize(
        mul(dec("1", 12, 4), dec("45", 14, 6)),
        18,
        2,
        Rounding::HalfEven,
    );
    assert_eq!(ok(&expected), dv("45.00", 18, 2));
    assert_eq!(ok(&sub(dec("50", 18, 2), expected)), dv("5.00", 19, 2));
}

#[test]
fn half_even_ties_round_to_even_in_both_signs() {
    for (input, expected) in [
        ("1.005", "1.00"),
        ("1.015", "1.02"),
        ("-1.005", "-1.00"),
        ("-1.015", "-1.02"),
        ("1.0051", "1.01"),
        ("-1.0049", "-1.00"),
        ("2.500", "2.50"),
    ] {
        let e = quantize(dec(input, 8, 4), 6, 2, Rounding::HalfEven);
        assert_eq!(ok(&e), dv(expected, 6, 2), "{input}");
    }
    for (input, expected) in [("2.5", "2"), ("3.5", "4"), ("-2.5", "-2"), ("-3.5", "-4")] {
        let e = quantize(dec(input, 3, 1), 3, 0, Rounding::HalfEven);
        assert_eq!(ok(&e), dv(expected, 3, 0), "{input}");
    }
}

#[test]
fn toward_zero_truncates_in_both_signs() {
    for (input, expected) in [
        ("1.019", "1.01"),
        ("-1.019", "-1.01"),
        ("-1.999", "-1.99"),
        ("0.009", "0.00"),
        ("-0.009", "0.00"),
    ] {
        let e = quantize(dec(input, 6, 3), 5, 2, Rounding::TowardZero);
        assert_eq!(ok(&e), dv(expected, 5, 2), "{input}");
    }
}

#[test]
fn quantization_at_38_digit_extremes_never_saturates_or_wraps() {
    let nines37 = "9".repeat(37);
    let tie = format!("{nines37}.5");
    let ten37 = format!("1{}", "0".repeat(37));
    for (input, expected) in [
        (tie.clone(), ten37.clone()),
        (format!("-{tie}"), format!("-{ten37}")),
    ] {
        let e = quantize(dec(&input, 38, 1), 38, 0, Rounding::HalfEven);
        assert_eq!(ok(&e), dv(&expected, 38, 0));
        // Rounding up to 10^37 needs 38 digits: narrowing to 37 must fail.
        let narrow = quantize(dec(&input, 38, 1), 37, 0, Rounding::HalfEven);
        assert_eq!(code(&narrow), "ARITHMETIC_OVERFLOW");
    }
    let truncated = quantize(dec(&tie, 38, 1), 37, 0, Rounding::TowardZero);
    assert_eq!(ok(&truncated), dv(&nines37, 37, 0));
    let max38 = "9".repeat(38);
    assert_eq!(
        ok(&quantize(dec(&max38, 38, 0), 38, 0, Rounding::HalfEven)),
        dv(&max38, 38, 0)
    );
    // Increasing scale of a full-precision value overflows rather than saturating.
    assert_eq!(
        code(&quantize(dec(&max38, 38, 0), 38, 1, Rounding::HalfEven)),
        "ARITHMETIC_OVERFLOW"
    );
    let smallest = format!("0.{}1", "0".repeat(36));
    assert_eq!(
        ok(&quantize(
            dec(&smallest, 38, 37),
            38,
            38,
            Rounding::HalfEven
        )),
        dv(&format!("0.{}10", "0".repeat(36)), 38, 38)
    );
}

#[test]
fn decimal_add_subtract_multiply_infer_exact_result_types() {
    // scale=max(2,4)=4, precision=max(3,3)+4+1=8.
    assert_eq!(
        ok(&add(dec("999.99", 5, 2), dec("999.9999", 7, 4))),
        dv("1999.9899", 8, 4)
    );
    assert_eq!(
        ok(&sub(dec("-999.99", 5, 2), dec("999.9999", 7, 4))),
        dv("-1999.9899", 8, 4)
    );
    // Binary floating point would give 0.30000000000000004.
    assert_eq!(
        ok(&add(dec("0.1", 1, 1), dec("0.2", 1, 1))),
        dv("0.3", 2, 1)
    );
    let nines37 = "9".repeat(37);
    assert_eq!(
        ok(&add(dec(&nines37, 37, 0), dec(&nines37, 37, 0))),
        dv(&format!("1{}8", "9".repeat(36)), 38, 0)
    );
    let nines19 = "9".repeat(19);
    assert_eq!(
        ok(&mul(
            dec(&nines19, 19, 0),
            dec(&format!("-{nines19}"), 19, 0)
        )),
        dv("-99999999999999999980000000000000000001", 38, 0)
    );
    assert_eq!(
        ok(&mul(dec("-1.5", 2, 1), dec("0.25", 3, 2))),
        dv("-0.375", 5, 3)
    );
}

#[test]
fn inferred_precision_beyond_38_is_rejected_statically() {
    for e in [
        add(dec("1", 38, 0), dec("1", 1, 0)),
        sub(dec("1", 38, 2), dec("1", 38, 2)),
        mul(dec("1", 20, 0), dec("1", 19, 0)),
        mul(dec("1", 38, 0), dec("1", 1, 0)),
    ] {
        assert_eq!(
            static_code(e.clone()).as_deref(),
            Some("PRECISION_OVERFLOW")
        );
        assert_eq!(code(&e), "PRECISION_OVERFLOW");
    }
    assert_eq!(static_code(mul(dec("1", 19, 0), dec("1", 19, 0))), None);
    // max(32-0, 10-5) + 5 + 1 = 38 is the widest accepted sum.
    assert_eq!(static_code(add(dec("1", 32, 0), dec("1", 10, 5))), None);
    assert_eq!(
        static_code(add(dec("1", 33, 0), dec("1", 10, 5))).as_deref(),
        Some("PRECISION_OVERFLOW")
    );
}

#[test]
fn decimal_type_bounds_are_1_to_38_with_scale_at_most_precision() {
    for (precision, scale) in [(0, 0), (39, 0), (2, 3), (39, 39)] {
        let t = decimal_type(precision, scale);
        assert_eq!(
            static_code(cast(s("1"), t)).as_deref(),
            Some("INVALID_TYPE"),
            "cast({precision},{scale})"
        );
        assert_eq!(
            static_code(quantize(
                dec("1", 5, 0),
                precision,
                scale,
                Rounding::HalfEven
            ))
            .as_deref(),
            Some("INVALID_TYPE")
        );
        assert_eq!(
            static_code(dec("0", precision, scale)).as_deref(),
            Some("INVALID_VALUE")
        );
        let mut r = recipe();
        r.sources[0].projection.fields.insert(
            "amount".into(),
            Field {
                path: "/amount".into(),
                data_type: decimal_type(precision, scale),
                nullable: true,
                header: false,
            },
        );
        assert_eq!(first_error(&validate(&r)).as_deref(), Some("INVALID_TYPE"));
    }
    let tiny = format!("0.{}", "9".repeat(38));
    assert_eq!(ok(&dec(&tiny, 38, 38)), dv(&tiny, 38, 38));
    assert_eq!(ok(&dec("7", 1, 0)), dv("7", 1, 0));
    assert_eq!(static_code(cast(s("1"), decimal_type(38, 0))), None);
}

#[test]
fn decimal_literals_reject_nan_infinity_exponents_and_excess_digits() {
    for text in [
        "NaN",
        "nan",
        "inf",
        "Infinity",
        "-Infinity",
        "1e3",
        "1E-2",
        " 1",
        "1 ",
        "",
        ".5",
        "1.",
        "1,000",
        "0x10",
    ] {
        assert_eq!(code(&dec(text, 10, 2)), "INVALID_VALUE", "{text:?}");
        assert_eq!(
            code(&cast(s(text), decimal_type(10, 2))),
            "INVALID_CAST",
            "{text:?}"
        );
    }
    for text in [
        "NaN",
        "inf",
        "1.0",
        "9223372036854775808",
        "-9223372036854775809",
        "",
    ] {
        assert_eq!(code(&int(text)), "INVALID_VALUE", "{text:?}");
    }
    // Excess fractional digits require explicit quantization, never silent rounding.
    assert_eq!(code(&dec("1.005", 5, 2)), "INVALID_VALUE");
    assert_eq!(code(&dec("1000", 5, 2)), "INVALID_VALUE");
}

#[test]
fn literals_evaluate_to_canonical_tagged_text() {
    assert_eq!(ok(&dec("1.5", 3, 2)), dv("1.50", 3, 2));
    assert_eq!(ok(&dec("+007.5", 5, 2)), dv("7.50", 5, 2));
    assert_eq!(ok(&dec("-0.00", 5, 2)), dv("0.00", 5, 2));
    assert_eq!(ok(&int("+42")), iv("42"));
    assert_eq!(ok(&int("-0")), iv("0"));
}

#[test]
fn int64_arithmetic_is_checked_and_overflow_fails() {
    let max = i64::MAX.to_string();
    let min = i64::MIN.to_string();
    assert_eq!(ok(&mul(int("6"), int("-7"))), iv("-42"));
    assert_eq!(ok(&sub(int(&max), int(&max))), iv("0"));
    for e in [
        add(int(&max), int("1")),
        sub(int(&min), int("1")),
        mul(int(&max), int("2")),
        mul(int(&min), int("-1")),
    ] {
        assert_eq!(code(&e), "ARITHMETIC_OVERFLOW");
    }
    assert_eq!(
        code(&cast(dec(&"9".repeat(20), 20, 0), Type::Int64)),
        "ARITHMETIC_OVERFLOW"
    );
}

#[test]
fn three_valued_logic_and_null_comparisons() {
    let values = [Some(true), Some(false), None];
    for l in values {
        for r in values {
            let and = match (l, r) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            };
            let or = match (l, r) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            };
            let e = Expr::And {
                left: b(boolean(l)),
                right: b(boolean(r)),
            };
            assert_eq!(ok(&e), bool_value(and), "{l:?} AND {r:?}");
            let e = Expr::Or {
                left: b(boolean(l)),
                right: b(boolean(r)),
            };
            assert_eq!(ok(&e), bool_value(or), "{l:?} OR {r:?}");
        }
        assert_eq!(
            ok(&Expr::Not {
                expr: b(boolean(l))
            }),
            bool_value(l.map(|x| !x))
        );
    }
    // `null = null` is unknown, as is any comparison with null.
    assert_eq!(ok(&eq(null(), null())), Value::Null);
    assert_eq!(ok(&eq(dec("1", 1, 0), null())), Value::Null);
    assert_eq!(ok(&lt(null(), int("1"))), Value::Null);
    assert_eq!(ok(&le(date("2026-11-12"), null())), Value::Null);
    assert_eq!(
        ok(&Expr::IsNull { expr: b(null()) }),
        bool_value(Some(true))
    );
    assert_eq!(
        static_code(Expr::And {
            left: b(int("1")),
            right: b(boolean(Some(true)))
        })
        .as_deref(),
        Some("TYPE_MISMATCH")
    );
}

#[test]
fn empty_string_is_not_null() {
    assert_eq!(
        ok(&Expr::IsNull { expr: b(s("")) }),
        bool_value(Some(false))
    );
    assert_eq!(
        ok(&Expr::Coalesce {
            values: vec![s(""), s("fallback")]
        }),
        sv("")
    );
    assert_eq!(ok(&eq(s(""), s(""))), bool_value(Some(true)));
    assert_eq!(ok(&eq(s(""), null())), Value::Null);
    assert_eq!(ok(&cast(s(""), Type::String)), sv(""));
}

#[test]
fn coalesce_is_explicit_ordered_and_typed() {
    let e = Expr::Coalesce {
        values: vec![null(), dec("2.50", 3, 2), dec("9", 3, 2)],
    };
    assert_eq!(ok(&e), dv("2.50", 3, 2));
    assert_eq!(
        ok(&Expr::Coalesce {
            values: vec![null(), null()]
        }),
        Value::Null
    );
    assert_eq!(
        static_code(Expr::Coalesce {
            values: vec![s("1"), int("1")]
        })
        .as_deref(),
        Some("TYPE_MISMATCH")
    );
    assert_eq!(
        static_code(Expr::Coalesce { values: vec![] }).as_deref(),
        Some("INVALID_EXPRESSION")
    );
}

#[test]
fn case_treats_unknown_as_not_true_and_types_both_branches() {
    let case = |condition| Expr::Case {
        condition: b(condition),
        then: b(s("then")),
        otherwise: b(s("otherwise")),
    };
    assert_eq!(ok(&case(boolean(Some(true)))), sv("then"));
    assert_eq!(ok(&case(boolean(Some(false)))), sv("otherwise"));
    assert_eq!(ok(&case(boolean(None))), sv("otherwise"));
    assert_eq!(ok(&case(eq(null(), null()))), sv("otherwise"));
    // An unselected branch is still statically typed.
    let mixed = Expr::Case {
        condition: b(boolean(Some(true))),
        then: b(s("1")),
        otherwise: b(int("1")),
    };
    assert_eq!(code(&mixed), "TYPE_MISMATCH");
    assert_eq!(
        static_code(case(s("true"))).as_deref(),
        Some("TYPE_MISMATCH")
    );
}

#[test]
fn comparisons_require_one_domain_and_compare_decimals_by_value() {
    assert_eq!(
        ok(&eq(dec("1.0", 2, 1), dec("1.00", 3, 2))),
        bool_value(Some(true))
    );
    assert_eq!(
        ok(&lt(dec("-1.01", 3, 2), dec("-1", 1, 0))),
        bool_value(Some(true))
    );
    assert_eq!(ok(&le(int("5"), int("5"))), bool_value(Some(true)));
    assert_eq!(ok(&lt(s("a"), s("b"))), bool_value(Some(true)));
    assert_eq!(
        ok(&lt(date("2026-11-11"), date("2026-11-12"))),
        bool_value(Some(true))
    );
    assert_eq!(
        ok(&eq(date("2026-11-12"), date("2026-11-12"))),
        bool_value(Some(true))
    );
    assert_eq!(
        ok(&lt(date("2025-12-31"), date("2026-01-01"))),
        bool_value(Some(true))
    );
    for e in [
        eq(s("1"), int("1")),
        eq(s("1"), dec("1", 1, 0)),
        eq(int("1"), dec("1", 1, 0)),
        lt(date("2026-11-12"), s("2026-11-12")),
        eq(boolean(Some(true)), s("true")),
    ] {
        assert_eq!(static_code(e.clone()).as_deref(), Some("TYPE_MISMATCH"));
        assert_eq!(code(&e), "TYPE_MISMATCH");
    }
}

#[test]
fn no_implicit_string_to_number_conversion() {
    for e in [
        add(s("1"), int("1")),
        mul(s("2"), dec("1", 1, 0)),
        sub(int("1"), dec("1", 1, 0)),
        add(date("2026-11-12"), int("1")),
        add(boolean(Some(true)), int("1")),
        add(s("1"), s("1")),
    ] {
        assert_eq!(static_code(e).as_deref(), Some("TYPE_MISMATCH"));
    }
    // Explicit conversion is allowed and exact.
    assert_eq!(
        ok(&add(cast(s("1.25"), decimal_type(3, 2)), dec("1", 1, 0))),
        dv("2.25", 4, 2)
    );
}

#[test]
fn explicit_casts_are_exact_or_fail() {
    assert_eq!(ok(&cast(int("5"), decimal_type(3, 2))), dv("5.00", 3, 2));
    assert_eq!(
        code(&cast(int("1000"), decimal_type(3, 2))),
        "ARITHMETIC_OVERFLOW"
    );
    assert_eq!(code(&cast(s("1000"), decimal_type(3, 2))), "INVALID_CAST");
    assert_eq!(ok(&cast(dec("5.00", 3, 2), Type::Int64)), iv("5"));
    assert_eq!(code(&cast(dec("5.50", 3, 2), Type::Int64)), "INVALID_CAST");
    assert_eq!(
        ok(&cast(dec("123.40", 5, 2), decimal_type(4, 1))),
        dv("123.4", 4, 1)
    );
    assert_eq!(
        code(&cast(dec("123.45", 5, 2), decimal_type(4, 1))),
        "INVALID_CAST"
    );
    assert_eq!(
        code(&cast(dec("123.40", 5, 2), decimal_type(3, 1))),
        "ARITHMETIC_OVERFLOW"
    );
    assert_eq!(ok(&cast(dec("1.5", 3, 2), Type::String)), sv("1.50"));
    assert_eq!(ok(&cast(int("-7"), Type::String)), sv("-7"));
    assert_eq!(ok(&cast(boolean(Some(false)), Type::String)), sv("false"));
    assert_eq!(
        ok(&cast(date("2026-11-12"), Type::String)),
        sv("2026-11-12")
    );
    assert_eq!(ok(&cast(s("42"), Type::Int64)), iv("42"));
    assert_eq!(code(&cast(s("4.2"), Type::Int64)), "INVALID_CAST");
    assert_eq!(
        ok(&cast(s("2026-11-12"), Type::Date)),
        Value::Date {
            value: "2026-11-12".into()
        }
    );
    assert_eq!(code(&cast(s("12/11/2026"), Type::Date)), "INVALID_CAST");
    assert_eq!(ok(&cast(s("true"), Type::Boolean)), bool_value(Some(true)));
    assert_eq!(code(&cast(s("TRUE"), Type::Boolean)), "INVALID_CAST");
    assert_eq!(ok(&cast(null(), decimal_type(5, 2))), Value::Null);
    for e in [
        cast(boolean(Some(true)), Type::Int64),
        cast(date("2026-11-12"), decimal_type(10, 0)),
        cast(int("1"), Type::Date),
        cast(dec("1", 1, 0), Type::Boolean),
    ] {
        assert_eq!(static_code(e).as_deref(), Some("INVALID_CAST"));
    }
}

#[test]
fn dates_accept_only_iso_calendar_dates() {
    for valid in ["2026-11-12", "2024-02-29", "0001-01-01", "9999-12-31"] {
        assert_eq!(
            ok(&date(valid)),
            Value::Date {
                value: valid.into()
            }
        );
    }
    for invalid in [
        "2026-02-29",
        "2026-13-01",
        "2026-1-05",
        "26-11-12",
        "12/11/2026",
        "2026/11/12",
        "2026-11-12T00:00:00",
        "2026-11-12Z",
        " 2026-11-12",
        "+2026-11-12",
        "-001-01-01",
        "20261112",
        "",
    ] {
        assert_eq!(code(&date(invalid)), "INVALID_VALUE", "{invalid:?}");
    }
}

#[test]
fn null_operands_propagate_through_typed_arithmetic() {
    assert_eq!(ok(&add(null(), dec("1.00", 3, 2))), Value::Null);
    assert_eq!(ok(&mul(int("2"), null())), Value::Null);
    assert_eq!(ok(&quantize(null(), 5, 2, Rounding::HalfEven)), Value::Null);
    assert_eq!(ok(&cast(null(), Type::Int64)), Value::Null);
}

fn json_run(
    rows: &str,
    fields: serde_json::Value,
    columns: serde_json::Value,
    filter: Option<serde_json::Value>,
) -> iresiphi::RunReport {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("lines.json");
    std::fs::write(&input, rows).unwrap();
    let mut output = serde_json::json!({"id":"review","anchor":"lines","columns":columns});
    if let Some(filter) = filter {
        output["filter"] = filter;
    }
    let recipe = Recipe::from_json(
        &serde_json::json!({"format_version":"0.1","name":"run","sources":[{"id":"lines","identity":["id"],"grain":["id"],"projection":{"array":"","fields":fields}}],"outputs":[output]})
            .to_string(),
    )
    .unwrap();
    let bindings = BTreeMap::from([(
        "lines".into(),
        Binding {
            path: input,
            adapter: "json".into(),
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
        evidence: iresiphi::model::Evidence::Full,
    };
    ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false))
}

#[test]
fn filters_retain_only_true_rows() {
    let fields = serde_json::json!({
        "id":{"path":"/id","data_type":{"kind":"string"}},
        "amount":{"path":"/amount","data_type":{"kind":"decimal","precision":5,"scale":2},"nullable":true}
    });
    let threshold = serde_json::json!({"op":"literal","value":{"kind":"decimal","value":"10","precision":5,"scale":2}});
    let report = json_run(
        r#"[{"id":"high","amount":"20"},{"id":"unknown","amount":null},{"id":"low","amount":"5"}]"#,
        fields,
        serde_json::json!([{"name":"id","expr":{"op":"field","source":"lines","field":"id"}}]),
        Some(
            serde_json::json!({"op":"less","left":threshold,"right":{"op":"field","source":"lines","field":"amount"}}),
        ),
    );
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    let ids: Vec<_> = report.tables["review"]
        .iter()
        .map(|row| row["id"].clone())
        .collect();
    assert_eq!(ids, vec![sv("high")]);
}

#[test]
fn integer_overflow_fails_the_run_without_publishing_rows() {
    let fields = serde_json::json!({
        "id":{"path":"/id","data_type":{"kind":"string"}},
        "n":{"path":"/n","data_type":{"kind":"int64"}}
    });
    let columns = serde_json::json!([{"name":"next","expr":{"op":"add","left":{"op":"field","source":"lines","field":"n"},"right":{"op":"literal","value":{"kind":"int64","value":"1"}}}}]);
    let report = json_run(
        &format!(r#"[{{"id":"a","n":"1"}},{{"id":"b","n":"{}"}}]"#, i64::MAX),
        fields,
        columns,
        None,
    );
    assert_eq!(report.status, "failed");
    assert_eq!(report.diagnostics[0].code, "ARITHMETIC_OVERFLOW");
    assert!(report.tables.is_empty());
}

fn valid_on(d: Expr, from: Expr, to: Expr) -> Expr {
    Expr::ValidOn {
        date: b(d),
        valid_from: b(from),
        valid_to: b(to),
    }
}

#[test]
fn division_is_rejected_statically_including_division_by_zero() {
    for e in [
        Expr::Divide {
            left: b(dec("1", 1, 0)),
            right: b(dec("3", 1, 0)),
        },
        Expr::Divide {
            left: b(int("1")),
            right: b(int("0")),
        },
        Expr::Divide {
            left: b(dec("1.00", 3, 2)),
            right: b(dec("0", 1, 0)),
        },
    ] {
        assert_eq!(
            static_code(e.clone()).as_deref(),
            Some("UNSUPPORTED_OPERATION")
        );
        assert_eq!(code(&e), "UNSUPPORTED_OPERATION");
    }
    // Even an unselected case branch cannot hide a division.
    let hidden = Expr::Case {
        condition: b(boolean(Some(false))),
        then: b(Expr::Divide {
            left: b(int("1")),
            right: b(int("0")),
        }),
        otherwise: b(int("1")),
    };
    assert_eq!(code(&hidden), "UNSUPPORTED_OPERATION");
}

#[test]
fn rate_validity_is_inclusive_lower_exclusive_upper_and_open_ended() {
    let on = |d: &str, from: &str, to: Option<&str>| {
        ok(&valid_on(
            date(d),
            date(from),
            to.map(date).unwrap_or_else(null),
        ))
    };
    // A rate ending 2026-11-12 does not apply on that date.
    assert_eq!(
        on("2026-11-12", "2026-01-01", Some("2026-11-12")),
        bool_value(Some(false))
    );
    assert_eq!(
        on("2026-11-11", "2026-01-01", Some("2026-11-12")),
        bool_value(Some(true))
    );
    assert_eq!(
        on("2026-11-12", "2026-11-12", Some("2026-11-13")),
        bool_value(Some(true))
    );
    assert_eq!(
        on("2026-11-11", "2026-11-12", Some("2026-11-13")),
        bool_value(Some(false))
    );
    // A null upper bound is the explicit open-ended interval.
    assert_eq!(on("9999-12-31", "2026-01-01", None), bool_value(Some(true)));
    assert_eq!(
        on("2025-12-31", "2026-01-01", None),
        bool_value(Some(false))
    );
    // An unknown date or lower bound is unknown, not false.
    assert_eq!(
        ok(&valid_on(null(), date("2026-01-01"), null())),
        Value::Null
    );
    assert_eq!(
        ok(&valid_on(date("2026-11-12"), null(), date("2027-01-01"))),
        Value::Null
    );
    assert_eq!(
        code(&valid_on(
            date("2026-11-12"),
            date("2026-11-12"),
            date("2026-11-12")
        )),
        "INVALID_INTERVAL"
    );
    assert_eq!(
        static_code(valid_on(s("2026-11-12"), date("2026-01-01"), null())).as_deref(),
        Some("TYPE_MISMATCH")
    );
}
