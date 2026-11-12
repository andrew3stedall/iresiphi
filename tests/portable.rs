use iresiphi::{
    eval::eval,
    model::*,
    validation::{explain, validate},
};
fn recipe() -> Recipe {
    serde_json::from_str(r#"{"format_version":"0.1","name":"test","sources":[{"id":"lines","identity":["id"],"grain":["id"],"projection":{"fields":{"id":{"path":"/id","data_type":{"kind":"string"}}}}}],"outputs":[{"id":"review","anchor":"lines","columns":[{"name":"id","expr":{"op":"field","source":"lines","field":"id"}}]}]}"#).unwrap()
}
#[test]
fn portable_recipe_hash_ignores_annotations_but_not_meaning() {
    let r = recipe();
    assert!(validate(&r).valid);
    let reopened = Recipe::from_json(&r.to_json().unwrap()).unwrap();
    assert_eq!(r.hash().unwrap(), reopened.hash().unwrap());
    let mut annotated = r.clone();
    annotated
        .annotations
        .insert("note".into(), serde_json::json!("anything"));
    assert_eq!(r.hash().unwrap(), annotated.hash().unwrap());
    annotated.name = "different".into();
    assert_ne!(r.hash().unwrap(), annotated.hash().unwrap());
    assert!(explain(&r).get("nodes").is_some());
}
#[test]
fn unknown_fields_versions_cycles_and_decimal_inference_fail() {
    assert!(
        Recipe::from_json(
            r#"{"format_version":"0.1","name":"x","sources":[],"outputs":[],"secret":"x"}"#
        )
        .is_err()
    );
    let mut r = recipe();
    r.format_version = "2".into();
    assert!(!validate(&r).valid);
    r = recipe();
    r.features = vec![Feature {
        id: "cycle".into(),
        expr: Expr::Feature { id: "cycle".into() },
    }];
    assert!(!validate(&r).valid);
    r = recipe();
    r.features = vec![Feature {
        id: "overflow".into(),
        expr: Expr::Multiply {
            left: Box::new(Expr::Literal {
                value: Value::Decimal {
                    value: "1".into(),
                    precision: 38,
                    scale: 0,
                },
            }),
            right: Box::new(Expr::Literal {
                value: Value::Decimal {
                    value: "1".into(),
                    precision: 1,
                    scale: 0,
                },
            }),
        },
    }];
    assert!(!validate(&r).valid);
}
#[test]
fn scalar_null_logic_and_integer_overflow_are_public() {
    let r = recipe();
    let mut no_fields =
        |_: &str, _: &str, _: &[String]| -> Result<Value> { panic!("no source access") };
    let null = Expr::Literal { value: Value::Null };
    let false_ = Expr::Literal {
        value: Value::Boolean { value: false },
    };
    assert_eq!(
        eval(
            &Expr::And {
                left: Box::new(null.clone()),
                right: Box::new(false_)
            },
            &r,
            &mut no_fields
        )
        .unwrap(),
        Value::Boolean { value: false }
    );
    assert_eq!(
        eval(
            &Expr::Equal {
                left: Box::new(null.clone()),
                right: Box::new(null)
            },
            &r,
            &mut no_fields
        )
        .unwrap(),
        Value::Null
    );
    let max = Expr::Literal {
        value: Value::Int64 {
            value: i64::MAX.to_string(),
        },
    };
    let one = Expr::Literal {
        value: Value::Int64 { value: "1".into() },
    };
    assert!(
        eval(
            &Expr::Add {
                left: Box::new(max),
                right: Box::new(one)
            },
            &r,
            &mut no_fields
        )
        .is_err()
    );
}
#[test]
fn bundled_definitions_are_self_contained_and_tamper_evident() {
    let mut r = recipe();
    r.bundle_definition("lines", Some("example catalog revision 1".into()))
        .unwrap();
    let saved = r.to_json().unwrap();
    assert!(Recipe::from_json(&saved).is_ok());
    r.bundled_definitions.get_mut("lines").unwrap().content_hash = "wrong".into();
    assert!(!validate(&r).valid);
}
#[test]
fn decimal_quantization_is_explicit_and_negative_ties_are_even() {
    let r = recipe();
    let literal = |value: &str| Expr::Literal {
        value: Value::Decimal {
            value: value.into(),
            precision: 6,
            scale: 3,
        },
    };
    for (value, expected) in [
        ("1.005", "1.00"),
        ("1.015", "1.02"),
        ("-1.005", "-1.00"),
        ("-1.015", "-1.02"),
    ] {
        let expr = Expr::Quantize {
            expr: Box::new(literal(value)),
            precision: 5,
            scale: 2,
            rounding: iresiphi::scalar::Rounding::HalfEven,
        };
        let actual = eval(&expr, &r, &mut |_, _, _| panic!("no fields")).unwrap();
        assert_eq!(
            actual,
            Value::Decimal {
                value: expected.into(),
                precision: 5,
                scale: 2
            }
        );
    }
}
#[test]
fn nonlocal_fields_require_connected_to_one_paths() {
    let mut r = recipe();
    let mut other = r.sources[0].clone();
    other.id = "other".into();
    r.sources.push(other);
    r.outputs[0].columns[0].expr = Expr::Field {
        source: "other".into(),
        field: "id".into(),
        path: vec![],
    };
    assert!(!validate(&r).valid);
    r.relationships.push(Relationship {
        id: "lookup".into(),
        from: "lines".into(),
        to: "other".into(),
        from_keys: vec!["id".into()],
        to_keys: vec!["id".into()],
        cardinality: Cardinality::ManyToOne,
        validity: None,
        strict: false,
        period: None,
    });
    r.outputs[0].columns[0].expr = Expr::Field {
        source: "other".into(),
        field: "id".into(),
        path: vec!["lookup".into()],
    };
    assert!(validate(&r).valid);
    r.relationships[0].cardinality = Cardinality::OneToMany;
    assert!(!validate(&r).valid);
}
