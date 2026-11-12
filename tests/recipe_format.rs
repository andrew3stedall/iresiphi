//! Decision #8/#9: portable recipe format, canonical identity, bindings separation
//! and static/bound validation coverage, through public seams only.
use iresiphi::{
    ExecutionContext,
    model::*,
    validation::{Report, explain, validate, validate_bound},
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn document() -> Json {
    json!({
        "format_version":"0.1",
        "name":"format",
        "sources":[
            {"id":"lines","identity":["supplier","id"],"grain":["supplier","id"],"projection":{"array":"","fields":{
                "id":{"path":"/id","data_type":{"kind":"string"}},
                "supplier":{"path":"/supplier","data_type":{"kind":"string"}},
                "quantity":{"path":"/quantity","data_type":{"kind":"decimal","precision":12,"scale":4}}
            }}},
            {"id":"rates","identity":["supplier"],"grain":["supplier"],"projection":{"fields":{
                "supplier":{"path":"supplier","data_type":{"kind":"string"}},
                "rate":{"path":"rate","data_type":{"kind":"decimal","precision":14,"scale":6}}
            }}}
        ],
        "relationships":[{"id":"rate","from":"lines","to":"rates","from_keys":["supplier"],"to_keys":["supplier"],"cardinality":"many_to_one"}],
        "features":[{"id":"charge","expr":{"op":"quantize","precision":18,"scale":2,"rounding":"half_even","expr":{"op":"multiply",
            "left":{"op":"field","source":"lines","field":"quantity"},
            "right":{"op":"field","source":"rates","field":"rate","path":["rate"]}}}}],
        "outputs":[{"id":"review","anchor":"lines","columns":[
            {"name":"id","expr":{"op":"field","source":"lines","field":"id"}},
            {"name":"charge","expr":{"op":"feature","id":"charge"}}
        ]}]
    })
}
fn load(doc: &Json) -> Result<Recipe> {
    Recipe::from_json(&doc.to_string())
}
fn load_code(doc: &Json) -> String {
    load(doc).expect_err("document must be rejected").code
}
fn first_error(report: &Report) -> Option<String> {
    let code = report
        .checks
        .iter()
        .find_map(|c| c.diagnostic.as_ref().map(|d| d.code.clone()));
    assert_eq!(report.valid, code.is_none(), "{report:?}");
    code
}
fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn format_version_must_be_exactly_0_1() {
    assert!(load(&document()).is_ok());
    for version in ["0.1.0", "0.10", "1.0", "0.2", "", " 0.1", "v0.1"] {
        let mut doc = document();
        doc["format_version"] = json!(version);
        assert_eq!(load_code(&doc), "UNSUPPORTED_VERSION", "{version:?}");
    }
    let mut doc = document();
    doc["format_version"] = json!(0.1);
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
    doc.as_object_mut().unwrap().remove("format_version");
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
}

#[test]
fn unknown_properties_fail_at_every_level() {
    let pointers = [
        "",
        "/sources/0",
        "/sources/0/projection",
        "/sources/0/projection/fields/id",
        "/sources/0/projection/fields/quantity/data_type",
        "/relationships/0",
        "/features/0",
        "/features/0/expr",
        "/features/0/expr/expr/left",
        "/outputs/0",
        "/outputs/0/columns/0",
    ];
    for pointer in pointers {
        let mut doc = document();
        doc.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extension".into(), json!(true));
        assert_eq!(load_code(&doc), "INVALID_RECIPE", "{pointer}");
    }
    let mut doc = document();
    doc["outputs"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"x","expr":{"op":"literal","value":{"kind":"string","value":"x","encoding":"utf8"}}}));
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
    let mut doc = document();
    doc["relationships"][0]["cardinality"] = json!("many_to_many");
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
    // Annotations are the reserved nonsemantic extension point.
    let mut doc = document();
    doc["annotations"] = json!({"owner":{"team":"finance"},"note":"anything"});
    assert!(load(&doc).is_ok());
}

#[test]
fn numeric_literals_must_be_tagged_strings() {
    let mut doc = document();
    doc["outputs"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"x","expr":{"op":"literal","value":{"kind":"decimal","value":1.5,"precision":3,"scale":2}}}));
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
    let mut doc = document();
    doc["outputs"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"x","expr":{"op":"literal","value":{"kind":"int64","value":7}}}));
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
}

#[test]
fn internal_bundle_table_names_are_rejected_before_execution() {
    for name in [
        "_evidence",
        "_candidate_evidence",
        "_detail_evidence",
        "_reconciliation",
    ] {
        let mut doc = document();
        doc["outputs"][0]["id"] = json!(name);
        assert_eq!(load_code(&doc), "DUPLICATE_ID", "{name}");
    }
}

#[test]
fn duplicate_ids_fail_including_duplicate_object_keys() {
    let mut doc = document();
    doc["sources"][1]["id"] = json!("lines");
    assert_eq!(load_code(&doc), "DUPLICATE_ID");
    // Sources, relationships, features and outputs share one declaration namespace.
    let mut doc = document();
    doc["features"][0]["id"] = json!("rate");
    assert_eq!(load_code(&doc), "DUPLICATE_ID");
    let mut doc = document();
    doc["outputs"][0]["columns"][1]["name"] = json!("id");
    assert_eq!(load_code(&doc), "DUPLICATE_ID");
    let mut doc = document();
    doc["sources"][0]["id"] = json!("");
    assert_eq!(load_code(&doc), "DUPLICATE_ID");
    // A repeated JSON object key would silently replace a field declaration.
    let text = document().to_string().replacen(
        r#""id":{"data_type":{"kind":"string"},"path":"/id"}"#,
        r#""id":{"data_type":{"kind":"string"},"path":"/id"},"id":{"data_type":{"kind":"int64"},"path":"/other"}"#,
        1,
    );
    assert_ne!(text, document().to_string());
    assert_eq!(Recipe::from_json(&text).unwrap_err().code, "DUPLICATE_KEY");
    let text = document().to_string().replacen(
        r#""name":"format""#,
        r#""name":"format","name":"other""#,
        1,
    );
    assert_eq!(Recipe::from_json(&text).unwrap_err().code, "DUPLICATE_KEY");
}

#[test]
fn missing_references_fail() {
    let cases: Vec<(&str, Json)> = vec![
        ("/features/0/expr/expr/left/source", json!("nowhere")),
        ("/features/0/expr/expr/left/field", json!("missing")),
        ("/features/0/expr/expr/right/path", json!(["missing"])),
        ("/outputs/0/columns/1/expr/id", json!("missing")),
        ("/outputs/0/anchor", json!("missing")),
        ("/relationships/0/to", json!("missing")),
        ("/relationships/0/from_keys", json!(["missing"])),
        ("/sources/0/identity", json!(["missing"])),
    ];
    for (pointer, value) in cases {
        let mut doc = document();
        *doc.pointer_mut(pointer).unwrap() = value;
        assert_eq!(load_code(&doc), "MISSING_REFERENCE", "{pointer}");
    }
}

#[test]
fn unsupported_capabilities_fail_and_known_ones_pass() {
    let mut doc = document();
    doc["required_capabilities"] = json!(["local", "exact_decimal", "json", "csv"]);
    assert!(load(&doc).is_ok());
    for capability in ["remote_pushdown", "python_callback", "LOCAL", ""] {
        doc["required_capabilities"] = json!([capability]);
        assert_eq!(load_code(&doc), "UNSUPPORTED_CAPABILITY", "{capability:?}");
    }
}

#[test]
fn canonical_hash_is_sha256_of_sorted_compact_json_without_annotations() {
    let doc = json!({
        "name":"pinned","format_version":"0.1",
        "sources":[{"projection":{"fields":{"id":{"path":"/id","data_type":{"kind":"string"}}}},"grain":["id"],"identity":["id"],"id":"lines"}],
        "outputs":[{"id":"review","anchor":"lines","columns":[{"name":"id","expr":{"op":"field","source":"lines","field":"id"}}]}],
        "annotations":{"author":"someone"}
    });
    let canonical = concat!(
        r#"{"bundled_definitions":{},"features":[],"format_version":"0.1","name":"pinned","#,
        r#""outputs":[{"aggregates":[],"anchor":"lines","columns":[{"expr":{"field":"id","op":"field","path":[],"source":"lines"},"name":"id"}],"#,
        r#""detail":false,"eligibility":null,"filter":null,"group_by":[],"id":"review"}],"#,
        r#""relationships":[],"required_capabilities":[],"#,
        r#""sources":[{"grain":["id"],"id":"lines","identity":["id"],"projection":{"array":null,"fields":{"id":{"data_type":{"kind":"string"},"header":false,"nullable":false,"path":"/id"}}}}]}"#
    );
    let hash = load(&doc).unwrap().hash().unwrap();
    assert_eq!(hash, sha256(canonical));
    assert_eq!(hash.len(), 64);
    assert!(
        hash.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
}

#[test]
fn hash_ignores_key_order_whitespace_and_annotations_but_not_meaning() {
    let base = load(&document()).unwrap().hash().unwrap();
    let pretty = serde_json::to_string_pretty(&document()).unwrap();
    assert_eq!(Recipe::from_json(&pretty).unwrap().hash().unwrap(), base);
    let mut doc = document();
    doc["annotations"] = json!({"revision":"draft 7"});
    assert_eq!(load(&doc).unwrap().hash().unwrap(), base);
    // Omitted defaults and explicit defaults are the same content.
    let mut doc = document();
    doc["required_capabilities"] = json!([]);
    doc["outputs"][0]["filter"] = Json::Null;
    doc["sources"][0]["projection"]["fields"]["id"]["nullable"] = json!(false);
    assert_eq!(load(&doc).unwrap().hash().unwrap(), base);
    let changes: Vec<(&str, Json)> = vec![
        ("/name", json!("renamed")),
        ("/features/0/expr/rounding", json!("toward_zero")),
        ("/sources/0/identity", json!(["id", "supplier"])),
        ("/outputs/0/columns", {
            let mut columns = document()["outputs"][0]["columns"].clone();
            columns.as_array_mut().unwrap().reverse();
            columns
        }),
    ];
    for (pointer, value) in changes {
        let mut doc = document();
        *doc.pointer_mut(pointer).unwrap() = value;
        assert_ne!(load(&doc).unwrap().hash().unwrap(), base, "{pointer}");
    }
}

#[test]
fn save_reload_preserves_identity_and_is_stable() {
    let recipe = load(&document()).unwrap();
    let saved = recipe.to_json().unwrap();
    let reopened = Recipe::from_json(&saved).unwrap();
    assert_eq!(reopened.hash().unwrap(), recipe.hash().unwrap());
    assert_eq!(reopened.to_json().unwrap(), saved);
}

#[test]
fn saved_recipes_carry_no_bindings_paths_or_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("secret-input.csv");
    std::fs::write(&input, "id\na\n").unwrap();
    let recipe = Recipe::from_json(
        r#"{"format_version":"0.1","name":"secret free","sources":[{"id":"lines","identity":["id"],"grain":["id"],"projection":{"fields":{"id":{"path":"id","data_type":{"kind":"string"}}}}}],"outputs":[{"id":"review","anchor":"lines","columns":[{"name":"id","expr":{"op":"field","source":"lines","field":"id"}}]}]}"#,
    )
    .unwrap();
    let bindings = BTreeMap::from([(
        "lines".into(),
        Binding {
            path: input.clone(),
            adapter: "csv".into(),
            // A valid setting carrying a sentinel value: settings must never be persisted.
            settings: BTreeMap::from([("null_tokens".into(), json!(["hunter2"]))]),
        },
    )]);
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    };
    let before = recipe.to_json().unwrap();
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    let saved = recipe.to_json().unwrap();
    assert_eq!(saved, before);
    let root = dir.path().to_string_lossy();
    for needle in [root.as_ref(), "secret-input", "hunter2", "token", "adapter"] {
        assert!(!saved.contains(needle), "{needle}");
    }
    // Bindings are not a recipe property.
    let mut doc: Json = serde_json::from_str(&saved).unwrap();
    doc["bindings"] = json!({"lines":{"path":input,"adapter":"csv"}});
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
}

#[test]
fn missing_bindings_fail_only_at_bound_validation_and_run() {
    let recipe = load(&document()).unwrap();
    assert!(validate(&recipe).valid);
    let dir = tempfile::tempdir().unwrap();
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    };
    let bound = validate_bound(&recipe, &Bindings::new(), &policy, "none");
    assert_eq!(first_error(&bound).as_deref(), Some("MISSING_BINDING"));
    let run = ExecutionContext::default().run(
        &recipe,
        &Bindings::new(),
        &policy,
        &AtomicBool::new(false),
    );
    assert_eq!(run.status, "failed");
    assert_eq!(run.diagnostics[0].code, "MISSING_BINDING");
}

#[test]
fn deferred_aggregate_functions_are_explicitly_unsupported() {
    for function in ["average", "distinct_count", "allocate"] {
        let mut doc = document();
        doc["outputs"][0]["aggregates"] = json!([{"name":"agg","function":function,"expr":{"op":"field","source":"lines","field":"quantity"}}]);
        doc["outputs"][0]["group_by"] = json!(["id"]);
        assert_eq!(load_code(&doc), "UNSUPPORTED_OPERATION", "{function}");
        let mut doc = document();
        doc["outputs"][0]["columns"].as_array_mut().unwrap().push(
            json!({"name":"agg","expr":{"op":"aggregate","relationship":"rate","function":function,"field":"rate"}}),
        );
        assert_eq!(load_code(&doc), "UNSUPPORTED_OPERATION", "{function}");
    }
}

#[test]
fn static_validation_reports_named_checks_and_no_data_coverage() {
    let report = validate(&load(&document()).unwrap());
    assert!(report.valid);
    let ids: Vec<_> = report.checks.iter().map(|c| c.id.as_str()).collect();
    for id in [
        "format",
        "capabilities",
        "identifiers",
        "bundled_definitions",
        "sources",
        "relationships",
        "expressions",
        "outputs",
        "data",
    ] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    for check in &report.checks {
        assert!(["passed", "failed", "not-run"].contains(&check.status.as_str()));
        assert!(!check.coverage.is_empty());
        if check.id == "data" {
            assert_eq!(check.status, "not-run");
        } else {
            assert_eq!(check.status, "passed", "{check:?}");
        }
    }
    // An early failure leaves dependent checks explicitly not run.
    let mut recipe = load(&document()).unwrap();
    recipe.format_version = "9".into();
    let report = validate(&recipe);
    let status = |id: &str| {
        report
            .checks
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.status.clone())
            .unwrap()
    };
    assert_eq!(status("format"), "failed");
    assert_eq!(status("expressions"), "not-run");
    assert_eq!(status("data"), "not-run");
    assert_eq!(first_error(&report).as_deref(), Some("UNSUPPORTED_VERSION"));
}

#[test]
fn explain_uses_stable_declaration_ids() {
    let recipe = load(&document()).unwrap();
    let explained = explain(&recipe);
    let ids: Vec<_> = explained["nodes"]
        .as_array()
        .expect("nodes list")
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    for id in [
        "source:lines",
        "source:rates",
        "relationship:rate",
        "feature:charge",
        "output:review",
        "output:review/column:id",
        "output:review/column:charge",
    ] {
        assert!(ids.contains(&id.to_string()), "{id} in {ids:?}");
    }
    // Reloading or reordering declarations does not change explained identity.
    let mut doc = document();
    doc["sources"].as_array_mut().unwrap().reverse();
    assert_eq!(
        explain(&load(&doc).unwrap())["nodes"]
            .as_array()
            .unwrap()
            .len(),
        ids.len()
    );
    assert_eq!(
        explain(&recipe),
        explain(&Recipe::from_json(&recipe.to_json().unwrap()).unwrap())
    );
}

fn catalog() -> Catalog {
    let doc = document();
    Catalog {
        name: "shared-rates".into(),
        revision: Some("7".into()),
        sources: vec![serde_json::from_value(doc["sources"][1].clone()).unwrap()],
        relationships: vec![serde_json::from_value(doc["relationships"][0].clone()).unwrap()],
        features: vec![serde_json::from_value(doc["features"][0].clone()).unwrap()],
    }
}
fn without_rates() -> Recipe {
    let mut doc = document();
    doc["sources"].as_array_mut().unwrap().truncate(1);
    doc["relationships"] = json!([]);
    doc["features"] = json!([]);
    doc["outputs"][0]["columns"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    load(&doc).unwrap()
}

#[test]
fn catalog_reuse_bundles_self_contained_hashed_definitions() {
    let mut recipe = without_rates();
    let catalog = catalog();
    recipe.bundle_from(&catalog, "rates", None).unwrap();
    recipe.bundle_from(&catalog, "rate", None).unwrap();
    recipe.bundle_from(&catalog, "charge", None).unwrap();
    // Re-bundling identical content is idempotent.
    recipe.bundle_from(&catalog, "rates", None).unwrap();
    assert!(validate(&recipe).valid, "{:?}", validate(&recipe));
    let saved = recipe.to_json().unwrap();
    let reopened = Recipe::from_json(&saved).unwrap();
    assert_eq!(reopened.hash().unwrap(), recipe.hash().unwrap());
    let doc: Json = serde_json::from_str(&saved).unwrap();
    for id in ["rates", "rate", "charge"] {
        let bundled = &doc["bundled_definitions"][id];
        assert_eq!(
            bundled["content_hash"],
            json!(sha256(&serde_json::to_string(&bundled["content"]).unwrap()))
        );
        assert!(
            bundled["provenance"]
                .as_str()
                .unwrap()
                .contains("shared-rates")
        );
    }
    // Equivalent to declaring the same content inline, apart from bundling metadata.
    let mut inline = load(&document()).unwrap();
    inline.bundled_definitions = recipe.bundled_definitions.clone();
    inline.outputs[0].columns.truncate(1);
    assert_eq!(inline.hash().unwrap(), recipe.hash().unwrap());
}

#[test]
fn conflicting_bundled_names_require_explicit_alias() {
    let mut recipe = without_rates();
    let mut other = catalog();
    other.sources[0]
        .projection
        .fields
        .get_mut("rate")
        .unwrap()
        .path = "price".into();
    recipe.bundle_from(&catalog(), "rates", None).unwrap();
    assert_eq!(
        recipe.bundle_from(&other, "rates", None).unwrap_err().code,
        "NAME_CONFLICT"
    );
    recipe
        .bundle_from(&other, "rates", Some("rates_v2"))
        .unwrap();
    assert!(recipe.sources.iter().any(|s| s.id == "rates_v2"));
    assert!(validate(&recipe).valid);
    assert_eq!(
        recipe
            .bundle_from(&catalog(), "missing", None)
            .unwrap_err()
            .code,
        "MISSING_REFERENCE"
    );
}

#[test]
fn bundled_definitions_are_tamper_evident() {
    let mut recipe = without_rates();
    recipe.bundle_from(&catalog(), "rates", None).unwrap();
    let mut tampered = recipe.clone();
    tampered
        .bundled_definitions
        .get_mut("rates")
        .unwrap()
        .content_hash = "0".repeat(64);
    assert_eq!(
        first_error(&validate(&tampered)).as_deref(),
        Some("CONTENT_HASH_MISMATCH")
    );
    let mut edited = recipe.clone();
    edited.sources[1].grain = vec!["rate".into()];
    assert_eq!(
        first_error(&validate(&edited)).as_deref(),
        Some("BUNDLED_DEFINITION_MISMATCH")
    );
}

#[test]
fn absolute_machine_paths_are_not_persisted() {
    for path in [
        "/home/me/rates.csv",
        "C:\\data\\rates.csv",
        "\\\\server\\share",
        "file:///tmp/x",
        "~/rates.csv",
    ] {
        let mut doc = document();
        doc["annotations"] = json!({"source_file": path});
        assert_eq!(load_code(&doc), "ABSOLUTE_PATH", "{path}");
        let mut recipe = without_rates();
        recipe.bundle_from(&catalog(), "rates", None).unwrap();
        recipe
            .bundled_definitions
            .get_mut("rates")
            .unwrap()
            .provenance = Some(path.into());
        assert_eq!(
            recipe.to_json().unwrap_err().code,
            "ABSOLUTE_PATH",
            "{path}"
        );
    }
    // JSON pointer projection paths are recipe semantics, not machine paths.
    assert!(load(&document()).is_ok());
}

#[test]
fn deferred_operations_and_types_fail_explicitly() {
    for op in ["divide", "window", "udf", "python", "regex", "average"] {
        let mut doc = document();
        doc["features"][0]["expr"] = json!({"op":op,"left":{"op":"literal","value":{"kind":"int64","value":"1"}},"right":{"op":"literal","value":{"kind":"int64","value":"0"}}});
        assert_eq!(load_code(&doc), "UNSUPPORTED_OPERATION", "{op}");
    }
    for kind in ["timestamp", "float64"] {
        let mut doc = document();
        doc["sources"][0]["projection"]["fields"]["quantity"]["data_type"] = json!({"kind":kind});
        assert_eq!(load_code(&doc), "UNSUPPORTED_TYPE", "{kind}");
    }
    let mut doc = document();
    doc["features"][0]["expr"] = json!({"op":"frobnicate"});
    assert_eq!(load_code(&doc), "INVALID_RECIPE");
}

#[test]
fn explain_reports_unsupported_operations_with_their_node_ids() {
    let mut recipe = load(&document()).unwrap();
    recipe.features.push(Feature {
        id: "ratio".into(),
        expr: Expr::Divide {
            left: Box::new(Expr::Field {
                source: "lines".into(),
                field: "quantity".into(),
                path: vec![],
            }),
            right: Box::new(Expr::Literal {
                value: Value::Decimal {
                    value: "0".into(),
                    precision: 1,
                    scale: 0,
                },
            }),
        },
    });
    recipe.outputs[0].aggregates.push(GroupAggregate {
        name: "mean".into(),
        function: "average".into(),
        expr: Expr::Field {
            source: "lines".into(),
            field: "quantity".into(),
            path: vec![],
        },
        filter: None,
        context: false,
    });
    recipe.outputs[0].group_by = vec!["id".into()];
    assert!(!validate(&recipe).valid);
    let explained = explain(&recipe);
    let unsupported: Vec<_> = explained["unsupported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            assert_eq!(u["code"], "UNSUPPORTED_OPERATION");
            (
                u["node"].as_str().unwrap().to_string(),
                u["operation"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(
        unsupported.contains(&("feature:ratio".into(), "divide".into())),
        "{unsupported:?}"
    );
    assert!(
        unsupported.contains(&("output:review/aggregate:mean".into(), "average".into())),
        "{unsupported:?}"
    );
    assert_eq!(explained["data_checks"], "not-run");
}

#[test]
fn explain_exposes_dependencies_stages_grain_and_measure_origins() {
    let mut doc = document();
    doc["outputs"][0]["filter"] =
        json!({"op":"is_null","expr":{"op":"field","source":"lines","field":"id"}});
    let explained = explain(&load(&doc).unwrap());
    let node = |id: &str| {
        explained["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("{id}"))
            .clone()
    };
    let charge = node("feature:charge");
    assert_eq!(
        charge["type"],
        json!({"kind":"decimal","precision":18,"scale":2})
    );
    let deps: Vec<_> = charge["depends_on"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert!(deps.contains(&"source:lines/field:quantity"));
    assert!(deps.contains(&"source:rates/field:rate"));
    assert!(deps.contains(&"relationship:rate"));
    // Columns referencing a feature trace measure origins through it.
    let column = node("output:review/column:charge");
    assert_eq!(column["depends_on"], json!(["feature:charge"]));
    assert_eq!(column["measure_origins"].as_array().unwrap().len(), 2);
    let output = node("output:review");
    assert_eq!(output["grain"], json!(["supplier", "id"]));
    assert_eq!(output["population_stages"], json!(["output:review/filter"]));
    assert_eq!(output["output_order"], "unspecified");
    assert_eq!(node("relationship:rate")["kind"], "checked_lookup");
}

fn bound_fixture() -> (tempfile::TempDir, Recipe, Bindings, Policy) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lines.json"),
        r#"[{"id":"1","supplier":"A","quantity":"2"},{"id":"2","supplier":"B","quantity":"1"}]"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("rates.csv"), "supplier,rate\nA,50\nB,45\n").unwrap();
    let bindings = BTreeMap::from([
        (
            "lines".to_string(),
            Binding {
                path: dir.path().join("lines.json"),
                adapter: "json".into(),
                settings: BTreeMap::new(),
            },
        ),
        (
            "rates".to_string(),
            Binding {
                path: dir.path().join("rates.csv"),
                adapter: "csv".into(),
                settings: BTreeMap::new(),
            },
        ),
    ]);
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    };
    (dir, load(&document()).unwrap(), bindings, policy)
}
fn check<'a>(report: &'a Report, id: &str) -> &'a iresiphi::validation::Check {
    report
        .checks
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("{id} in {report:?}"))
}

#[test]
fn schema_only_json_checks_escaped_headers_and_nested_array_pointers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested.json");
    let mut doc = json!({
        "format_version":"0.1", "name":"nested-schema",
        "sources":[{"id":"lines", "identity":["supplier","id"],
            "grain":["supplier","id"], "projection":{
                "array":"/payload/0/a~1b", "fields":{
                    "supplier":{"path":"/head~1er/~0name", "header":true, "data_type":{"kind":"string"}},
                    "id":{"path":"/id", "data_type":{"kind":"string"}}
                }}}],
        "outputs":[{"id":"review", "anchor":"lines", "columns":[
            {"name":"id", "expr":{"op":"field", "source":"lines", "field":"id"}}
        ]}]
    });
    let bindings: Bindings =
        serde_json::from_value(json!({"lines":{"path":path, "adapter":"json"}})).unwrap();
    let policy = Policy {
        allowed_read_roots: vec![dir.path().into()],
        allowed_output_roots: vec![dir.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    };
    std::fs::write(
        &path,
        r#"{"head/er":{"~name":"A"},"payload":[{"a/b":[{"id":"1"},{"id":"2"}]}]}"#,
    )
    .unwrap();
    let recipe = load(&doc).unwrap();
    assert!(validate_bound(&recipe, &bindings, &policy, "none").valid);
    assert!(validate_bound(&recipe, &bindings, &policy, "full").valid);
    doc["sources"][0]["projection"]["array"] = json!("/payload/00/a~1b");
    assert_eq!(
        first_error(&validate_bound(
            &load(&doc).unwrap(),
            &bindings,
            &policy,
            "none"
        ))
        .as_deref(),
        Some("INVALID_VALUE")
    );
    std::fs::write(&path, r#"{"head/er":null,"payload":[{"a/b":[]}]}"#).unwrap();
    assert!(!validate_bound(&recipe, &bindings, &policy, "none").valid);
    std::fs::write(&path, r#"{"head/er":"scalar","payload":[{"a/b":[]}]}"#).unwrap();
    assert_eq!(
        first_error(&validate_bound(&recipe, &bindings, &policy, "none")).as_deref(),
        Some("INVALID_VALUE")
    );
}

#[test]
fn bound_validation_distinguishes_none_and_full_coverage() {
    let (dir, recipe, bindings, policy) = bound_fixture();
    let none = validate_bound(&recipe, &bindings, &policy, "none");
    assert!(none.valid, "{none:?}");
    assert_eq!(check(&none, "access").status, "passed");
    assert_eq!(check(&none, "data").status, "not-run");
    assert!(none.checks.iter().all(|c| !c.id.starts_with("data.")));
    let explanation = iresiphi::validation::explain_bound(&recipe, &bindings, &policy);
    for read in explanation["bound"]["reads"].as_array().unwrap() {
        assert_eq!(read["capabilities"]["location"], "local");
        assert_eq!(read["capabilities"]["typed_read"], true);
        assert_eq!(read["capabilities"]["pushdown"], false);
        assert_eq!(read["capabilities"]["streaming"], false);
    }
    let full = validate_bound(&recipe, &bindings, &policy, "full");
    assert!(full.valid, "{full:?}");
    for id in [
        "data",
        "data.types",
        "data.identity",
        "data.relationships",
        "data.rate_validity",
    ] {
        assert_eq!(check(&full, id).status, "passed", "{id}");
        assert!(check(&full, id).coverage.contains("rows"), "{id}");
    }
    // Static validity does not imply data correctness: a duplicate identity
    // passes static and none-mode validation but fails the full scan.
    std::fs::write(
        dir.path().join("lines.json"),
        r#"[{"id":"1","supplier":"A","quantity":"2"},{"id":"1","supplier":"A","quantity":"3"}]"#,
    )
    .unwrap();
    assert!(validate(&recipe).valid);
    assert!(validate_bound(&recipe, &bindings, &policy, "none").valid);
    let full = validate_bound(&recipe, &bindings, &policy, "full");
    assert!(!full.valid);
    assert_eq!(check(&full, "data.identity").status, "failed");
    assert_eq!(check(&full, "data.rate_validity").status, "not-run");
    assert_eq!(
        validate_bound(&recipe, &bindings, &policy, "sample")
            .checks
            .last()
            .unwrap()
            .diagnostic
            .as_ref()
            .unwrap()
            .code,
        "UNSUPPORTED_OPERATION"
    );
}

#[test]
fn policy_blocked_sources_are_infeasible_and_runs_refuse_before_access() {
    let (_dir, recipe, mut bindings, policy) = bound_fixture();
    let outside = tempfile::tempdir().unwrap();
    // The file does not exist: a refusal must come from policy, not from access.
    bindings.get_mut("rates").unwrap().path = outside.path().join("rates.csv");
    let report = validate_bound(&recipe, &bindings, &policy, "none");
    assert_eq!(first_error(&report).as_deref(), Some("POLICY_DENIED"));
    let explained = iresiphi::validation::explain_bound(&recipe, &bindings, &policy);
    assert_eq!(explained["bound"]["feasible"], false);
    assert_eq!(explained["bound"]["diagnostic"]["code"], "POLICY_DENIED");
    assert_eq!(explained["bound"]["source_access"], "not-run");
    assert!(explained["bound"]["budgets"].is_object());
    assert!(
        explained["bound"]["reads"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["rows"].is_null() && r["estimate_status"] == "unknown")
    );
    let run = ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(run.status, "failed");
    assert_eq!(run.diagnostics[0].code, "POLICY_DENIED");
}

#[test]
fn rebinding_the_unchanged_recipe_to_a_second_input_set() {
    let (dir, recipe, mut bindings, policy) = bound_fixture();
    let first =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(first.status, "succeeded", "{:?}", first.diagnostics);
    std::fs::write(
        dir.path().join("rates2.csv"),
        "supplier,rate\nA,49.5\nB,45\n",
    )
    .unwrap();
    bindings.get_mut("rates").unwrap().path = dir.path().join("rates2.csv");
    let reopened = Recipe::from_json(&recipe.to_json().unwrap()).unwrap();
    let second =
        ExecutionContext::default().run(&reopened, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(second.status, "succeeded", "{:?}", second.diagnostics);
    let charge = |r: &iresiphi::RunReport| {
        let mut rows: Vec<_> = r.tables["review"]
            .iter()
            .map(|row| (row["id"].clone(), row["charge"].clone()))
            .collect();
        rows.sort_by_key(|(id, _)| format!("{id:?}"));
        rows
    };
    let dec = |v: &str| Value::Decimal {
        value: v.into(),
        precision: 18,
        scale: 2,
    };
    assert_eq!(charge(&first)[0].1, dec("100.00"));
    assert_eq!(charge(&second)[0].1, dec("99.00"));
    assert_eq!(charge(&second)[1].1, dec("45.00"));
    assert_eq!(
        first.manifest["recipe_hash"],
        second.manifest["recipe_hash"]
    );
}

#[test]
fn hash_is_independent_of_declaration_order_and_literal_spelling() {
    let base = load(&document()).unwrap().hash().unwrap();
    // Sources, relationships, features and outputs are keyed by ID; their
    // list order carries no meaning.
    let mut doc = document();
    doc["sources"].as_array_mut().unwrap().reverse();
    assert_eq!(load(&doc).unwrap().hash().unwrap(), base);
    let literal = |value: &str| {
        let mut doc = document();
        doc["outputs"][0]["columns"].as_array_mut().unwrap().push(
            json!({"name":"x","expr":{"op":"literal","value":{"kind":"decimal","value":value,"precision":5,"scale":2}}}),
        );
        load(&doc).unwrap().hash().unwrap()
    };
    assert_eq!(literal("1.5"), literal("1.50"));
    assert_eq!(literal("+001.50"), literal("1.50"));
    assert_ne!(literal("1.51"), literal("1.50"));
    let capabilities = |caps: Json| {
        let mut doc = document();
        doc["required_capabilities"] = caps;
        load(&doc).unwrap().hash().unwrap()
    };
    assert_eq!(
        capabilities(json!(["json", "local"])),
        capabilities(json!(["local", "json"]))
    );
}
