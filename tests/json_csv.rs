//! Public contract tests for the local JSON and CSV source adapters (#18).
use iresiphi::{
    ExecutionContext,
    adapters::{read_batches, read_sources},
    model::*,
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

fn go() -> AtomicBool {
    AtomicBool::new(false)
}
fn string(path: &str) -> Json {
    json!({"path":path,"data_type":{"kind":"string"}})
}
fn nullable(mut field: Json) -> Json {
    field["nullable"] = json!(true);
    field
}
fn header(mut field: Json) -> Json {
    field["header"] = json!(true);
    field
}
fn dec(path: &str, p: u8, s: u8) -> Json {
    json!({"path":path,"data_type":{"kind":"decimal","precision":p,"scale":s}})
}
fn date(path: &str) -> Json {
    json!({"path":path,"data_type":{"kind":"date"}})
}
fn int(path: &str) -> Json {
    json!({"path":path,"data_type":{"kind":"int64"}})
}
fn source(id: &str, array: Option<&str>, fields: Json, identity: &[&str]) -> Json {
    json!({"id":id,"identity":identity,"grain":identity,"projection":{"array":array,"fields":fields}})
}
fn recipe(sources: Vec<Json>, relationships: Vec<Json>, outputs: Vec<Json>) -> Recipe {
    serde_json::from_value(json!({"format_version":"0.1","name":"json-csv","sources":sources,"relationships":relationships,"outputs":outputs})).unwrap()
}
fn binding(path: PathBuf, adapter: &str) -> Binding {
    Binding {
        path,
        adapter: adapter.into(),
        settings: BTreeMap::new(),
    }
}
fn policy(root: &Path) -> Policy {
    Policy {
        allowed_read_roots: vec![root.into()],
        allowed_output_roots: vec![root.into()],
        limits: Limits::default(),
        diagnostic_values: false,
        placement: Default::default(),
    }
}

/// One source named `lines` read from one file.
struct One {
    dir: tempfile::TempDir,
    recipe: Recipe,
    bindings: Bindings,
    policy: Policy,
}
impl One {
    fn new(adapter: &str, contents: &[u8], array: Option<&str>, fields: Json) -> Self {
        Self::keyed(adapter, contents, array, fields, &["id"])
    }
    fn keyed(
        adapter: &str,
        contents: &[u8],
        array: Option<&str>,
        fields: Json,
        identity: &[&str],
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        std::fs::write(&path, contents).unwrap();
        let recipe = recipe(
            vec![source("lines", array, fields, identity)],
            vec![],
            vec![],
        );
        let bindings = BTreeMap::from([("lines".into(), binding(path, adapter))]);
        let policy = policy(dir.path());
        Self {
            dir,
            recipe,
            bindings,
            policy,
        }
    }
    fn setting(mut self, key: &str, value: Json) -> Self {
        self.bindings
            .get_mut("lines")
            .unwrap()
            .settings
            .insert(key.into(), value);
        self
    }
    fn read(&self) -> Result<Vec<Row>> {
        read_sources(&self.recipe, &self.bindings, &self.policy, &go())
            .map(|mut rows| rows.remove("lines").unwrap())
    }
    fn code(&self) -> String {
        match self.read() {
            Ok(rows) => panic!("expected failure, read {} rows", rows.len()),
            Err(e) => e.code,
        }
    }
}
fn s(value: &str) -> Value {
    Value::String {
        value: value.into(),
    }
}
fn d(value: &str, precision: u8, scale: u8) -> Value {
    Value::Decimal {
        value: value.into(),
        precision,
        scale,
    }
}

// ---------------------------------------------------------------- JSON projection

#[test]
fn json_header_fields_carry_onto_each_array_record_without_cartesian_product() {
    let one = One::new(
        "json",
        br#"{"supplier":"A","notes":[{"n":1},{"n":2},{"n":3}],"lines":[{"id":"1"},{"id":"2"}]}"#,
        Some("/lines"),
        json!({"id":string("/id"),"supplier":header(string("/supplier"))}),
    );
    let rows = one.read().unwrap();
    assert_eq!(rows.len(), 2, "only the declared array expands");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.fields["supplier"], s("A"));
        assert_eq!(row.fields["id"], s(&(i + 1).to_string()));
        assert_eq!(row.reference.location, format!("/lines/{i}"));
        assert_eq!(row.reference.source, "lines");
    }
    // Pointing a scalar field at an unrelated array never expands it.
    let one = One::new(
        "json",
        br#"{"supplier":"A","notes":[{"n":1}],"lines":[{"id":"1"}]}"#,
        Some("/lines"),
        json!({"id":string("/id"),"note":header(string("/notes"))}),
    );
    assert_eq!(one.code(), "INVALID_VALUE");
}

#[test]
fn json_whole_document_projection_uses_the_root_pointer() {
    let one = One::new(
        "json",
        br#"{"id":"only","a/b":"slash","m~n":"tilde"}"#,
        None,
        json!({"id":string("/id"),"slash":string("/a~1b"),"tilde":string("/m~0n")}),
    );
    let rows = one.read().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reference.location, "");
    assert_eq!(rows[0].fields["slash"], s("slash"));
    assert_eq!(rows[0].fields["tilde"], s("tilde"));
}

#[test]
fn json_field_paths_must_be_json_pointers() {
    // A pointer typo must not silently become an all-null nullable column.
    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1","note":"x"}]}"#,
        Some("/lines"),
        json!({"id":string("/id"),"note":nullable(string("note"))}),
    );
    assert_eq!(one.code(), "INVALID_PROJECTION");
    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1"}]}"#,
        Some("lines"),
        json!({"id":string("/id")}),
    );
    assert_eq!(one.code(), "INVALID_PROJECTION");
}

#[test]
fn json_missing_path_is_null_only_when_nullable_and_scalar_traversal_is_malformed() {
    let fields = json!({"id":string("/id"),"code":nullable(string("/meta/code"))});
    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1"},{"id":"2","meta":{}},{"id":"3","meta":{"code":null}}]}"#,
        Some("/lines"),
        fields.clone(),
    );
    let rows = one.read().unwrap();
    assert!(rows.iter().all(|r| r.fields["code"] == Value::Null));

    // Present but structurally wrong: `meta` is text, so `/meta/code` is malformed, not missing.
    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1","meta":"text"}]}"#,
        Some("/lines"),
        fields.clone(),
    );
    assert_eq!(one.code(), "INVALID_VALUE");
    // An array element that is not a record is malformed, not a record of missing fields.
    let one = One::new(
        "json",
        br#"{"lines":[7]}"#,
        Some("/lines"),
        json!({"id":nullable(string("/id"))}),
    );
    assert_eq!(one.code(), "INVALID_VALUE");

    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1"}]}"#,
        Some("/lines"),
        json!({"id":string("/id"),"code":string("/code")}),
    );
    assert_eq!(one.code(), "NULL_REQUIRED");
}

#[test]
fn json_empty_string_is_not_null_and_mistyped_values_fail() {
    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1","note":""}]}"#,
        Some("/lines"),
        json!({"id":string("/id"),"note":nullable(string("/note"))}),
    );
    assert_eq!(one.read().unwrap()[0].fields["note"], s(""));
    for (field, value) in [
        (nullable(dec("/v", 10, 2)), json!("")),
        (nullable(string("/v")), json!(12)),
        (nullable(string("/v")), json!(true)),
        (nullable(dec("/v", 10, 2)), json!(true)),
        (nullable(date("/v")), json!(20261112)),
        (nullable(int("/v")), json!(true)),
        (nullable(string("/v")), json!({"nested":"x"})),
    ] {
        let doc = json!({"lines":[{"id":"1","v":value}]}).to_string();
        let one = One::new(
            "json",
            doc.as_bytes(),
            Some("/lines"),
            json!({"id":string("/id"),"v":field}),
        );
        assert!(
            ["INVALID_VALUE", "INVALID_DECIMAL"].contains(&one.code().as_str()),
            "{value} must not coerce"
        );
    }
}

#[test]
fn json_duplicate_keys_are_detected_after_escape_decoding() {
    for doc in [
        br#"{"lines":[{"id":"1","id":"2"}]}"#.as_slice(),
        br#"{"lines":[{"id":"1"}],"\u0061":1,"a":2}"#.as_slice(),
    ] {
        let one = One::new("json", doc, Some("/lines"), json!({"id":string("/id")}));
        assert_eq!(one.code(), "DUPLICATE_JSON_KEY");
    }
}

#[test]
fn json_numeric_literals_parse_exactly_or_fail() {
    let one = One::new(
        "json",
        br#"{"lines":[{"id":"1","v":0.1},{"id":"2","v":1E-2},{"id":"3","v":-0.50}]}"#,
        Some("/lines"),
        json!({"id":string("/id"),"v":dec("/v",10,2)}),
    );
    let rows = one.read().unwrap();
    assert_eq!(rows[0].fields["v"], d("0.10", 10, 2));
    assert_eq!(rows[0].raw["v"].to_string(), "0.1");
    assert_eq!(rows[1].fields["v"], d("0.01", 10, 2));
    assert_eq!(rows[2].fields["v"], d("-0.50", 10, 2));
    // Excess scale and precision fail rather than round; floats are never an intermediate.
    for literal in ["1.005", "123456789.01", "1e400", "1e-3"] {
        let doc = format!(r#"{{"lines":[{{"id":"1","v":{literal}}}]}}"#);
        let one = One::new(
            "json",
            doc.as_bytes(),
            Some("/lines"),
            json!({"id":string("/id"),"v":dec("/v",10,2)}),
        );
        assert_eq!(one.code(), "INVALID_DECIMAL", "{literal}");
    }
    for literal in ["9223372036854775808", "1.0", "1e3"] {
        let doc = format!(r#"{{"lines":[{{"id":"1","v":{literal}}}]}}"#);
        let one = One::new(
            "json",
            doc.as_bytes(),
            Some("/lines"),
            json!({"id":string("/id"),"v":int("/v")}),
        );
        assert_eq!(one.code(), "INVALID_VALUE", "{literal}");
    }
}

#[test]
fn nan_and_infinity_are_rejected_not_null() {
    for doc in [
        br#"{"lines":[{"id":"1","v":NaN}]}"#.as_slice(),
        br#"{"lines":[{"id":"1","v":Infinity}]}"#.as_slice(),
    ] {
        let one = One::new(
            "json",
            doc,
            Some("/lines"),
            json!({"id":string("/id"),"v":nullable(dec("/v",10,2))}),
        );
        assert_eq!(one.code(), "INVALID_JSON");
    }
    for text in ["NaN", "nan", "Infinity", "-Infinity", "inf", "-inf"] {
        let doc = json!({"lines":[{"id":"1","v":text}]}).to_string();
        let one = One::new(
            "json",
            doc.as_bytes(),
            Some("/lines"),
            json!({"id":string("/id"),"v":nullable(dec("/v",10,2))}),
        );
        assert_eq!(one.code(), "INVALID_DECIMAL", "JSON {text}");
        let csv = format!("id,v\n1,{text}\n");
        let one = One::new(
            "csv",
            csv.as_bytes(),
            None,
            json!({"id":string("id"),"v":nullable(dec("v",10,2))}),
        );
        assert_eq!(one.code(), "INVALID_DECIMAL", "CSV {text}");
    }
}

// ------------------------------------------------------- references and fingerprints

#[test]
fn source_references_fingerprint_the_bytes_read_and_name_a_safe_binding_identity() {
    let bytes = br#"{"lines":[{"id":"1"},{"id":"2"}]}"#;
    let one = One::new("json", bytes, Some("/lines"), json!({"id":string("/id")}));
    let rows = one.read().unwrap();
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(rows.iter().all(|r| r.reference.fingerprint == digest));
    let identity = rows[0].reference.binding.clone();
    assert!(!identity.is_empty());
    assert_ne!(
        identity, "lines",
        "binding identity is more than the source id"
    );
    let path = one.dir.path().to_string_lossy().into_owned();
    assert!(
        !identity.contains(&path),
        "binding identity must not leak machine paths"
    );
    // Stable for the same binding...
    assert_eq!(one.read().unwrap()[0].reference.binding, identity);
    // ...and distinct when the same bytes are rebound from elsewhere.
    let copy = one.dir.path().join("copy");
    std::fs::write(&copy, bytes).unwrap();
    let mut rebound = One::new("json", bytes, Some("/lines"), json!({"id":string("/id")}));
    rebound.policy = one.policy.clone();
    rebound.bindings.get_mut("lines").unwrap().path = copy;
    let rows = rebound.read().unwrap();
    assert_eq!(rows[0].reference.fingerprint, digest);
    assert_ne!(rows[0].reference.binding, identity);
}

// ----------------------------------------------------------------------- CSV dialect

#[test]
fn csv_rfc4180_quoting_keeps_embedded_crlf_quotes_and_delimiters() {
    let one = One::new(
        "csv",
        b"id,note\r\n1,\"say \"\"hi\"\", ok\r\nnext\"\r\n2,plain\r\n",
        None,
        json!({"id":string("id"),"note":string("note")}),
    );
    let rows = one.read().unwrap();
    assert_eq!(rows[0].fields["note"], s("say \"hi\", ok\r\nnext"));
    assert_eq!(rows[0].reference.location, "record:2");
    assert_eq!(rows[1].fields["note"], s("plain"));
    assert_eq!(rows[1].reference.location, "record:3");
}

#[test]
fn csv_malformed_quoting_fails() {
    for csv in [
        b"id,note\n1,\"unterminated\n".as_slice(),
        b"id,note\n1,stray\"quote\n".as_slice(),
        b"id,note\n1,\"closed\"trailing\n".as_slice(),
    ] {
        let one = One::new(
            "csv",
            csv,
            None,
            json!({"id":string("id"),"note":string("note")}),
        );
        assert_eq!(
            one.code(),
            "INVALID_CSV",
            "{}",
            String::from_utf8_lossy(csv)
        );
    }
}

#[test]
fn csv_ragged_and_blank_records_fail_instead_of_being_skipped() {
    for csv in [
        b"id,note\n1,a\n\n2,b\n".as_slice(),
        b"id,note\n1,a,extra\n".as_slice(),
        b"id,note\n1\n".as_slice(),
        b"".as_slice(),
    ] {
        let one = One::new(
            "csv",
            csv,
            None,
            json!({"id":string("id"),"note":string("note")}),
        );
        assert_eq!(
            one.code(),
            "INVALID_CSV",
            "{:?}",
            String::from_utf8_lossy(csv)
        );
    }
}

#[test]
fn csv_dialect_and_schema_are_explicit() {
    let fields = json!({"id":string("id"),"note":string("note")});
    let one = One::new("csv", b"id;note\n1;'a;b'\n", None, fields.clone())
        .setting("delimiter", json!(";"))
        .setting("quote", json!("'"));
    assert_eq!(one.read().unwrap()[0].fields["note"], s("a;b"));
    // A misspelt setting must not silently fall back to the default dialect.
    let one =
        One::new("csv", b"id;note\n1;a\n", None, fields.clone()).setting("delimeter", json!(";"));
    assert_eq!(one.code(), "INVALID_BINDING");
    // An empty null token would erase the difference between empty text and null.
    let one =
        One::new("csv", b"id,note\n1,\n", None, fields.clone()).setting("null_tokens", json!([""]));
    assert_eq!(one.code(), "INVALID_BINDING");
    let one =
        One::new("csv", b"id,note\n1,a\n", None, fields.clone()).setting("delimiter", json!("\""));
    assert_eq!(one.code(), "INVALID_BINDING");
    let one = One::new(
        "json",
        br#"{"id":"1","note":"a"}"#,
        None,
        json!({"id":string("/id"),"note":string("/note")}),
    )
    .setting("delimiter", json!(";"));
    assert_eq!(one.code(), "INVALID_BINDING");
    assert_eq!(
        One::new("csv", b"id,note,note\n1,a,b\n", None, fields.clone()).code(),
        "INVALID_CSV"
    );
    assert_eq!(
        One::new("csv", b"id,text\n1,a\n", None, fields.clone()).code(),
        "INVALID_PROJECTION"
    );
    assert_eq!(
        One::new("csv", b"id,note\n1,\xff\n", None, fields.clone()).code(),
        "INVALID_CSV"
    );
    // Extra columns are allowed; the declared schema is what gets projected.
    let rows = One::new("csv", b"extra,id,note\nz,1,a\n", None, fields)
        .read()
        .unwrap();
    assert_eq!(rows[0].fields.len(), 2);
}

#[test]
fn csv_null_token_and_backslash_escaping_follow_the_bundle_contract() {
    let one = One::keyed(
        "csv",
        b"n,note\n1,\\N\n2,\"\\N\"\n3,\\\\N\n4,\\\\x\n5,\\x\n6,\n",
        None,
        json!({"n":string("n"),"note":nullable(string("note"))}),
        &["n"],
    );
    let rows = one.read().unwrap();
    let notes: Vec<_> = rows.iter().map(|r| r.fields["note"].clone()).collect();
    assert_eq!(
        notes,
        vec![
            Value::Null,
            Value::Null,
            s("\\N"),
            s("\\x"),
            s("\\x"),
            s("")
        ]
    );
    assert_eq!(
        rows[2].raw["note"],
        json!("\\\\N"),
        "raw keeps the observed text"
    );
    let one = One::new(
        "csv",
        b"id,note\n1,\\N\n",
        None,
        json!({"id":string("id"),"note":string("note")}),
    );
    assert_eq!(one.code(), "NULL_REQUIRED");
}

#[test]
fn dates_accept_only_iso_calendar_text() {
    let ok = One::new(
        "csv",
        b"id,day\n1,2026-11-12\n",
        None,
        json!({"id":string("id"),"day":date("day")}),
    );
    assert_eq!(
        ok.read().unwrap()[0].fields["day"],
        Value::Date {
            value: "2026-11-12".into()
        }
    );
    for text in [
        "2026/11/12",
        "12/11/2026",
        "2026-1-2",
        "20261112",
        "2026-11-12T00:00:00",
        " 2026-11-12",
        "2025-02-29",
        "",
    ] {
        let csv = format!("id,day\n1,{text}\n");
        let one = One::new(
            "csv",
            csv.as_bytes(),
            None,
            json!({"id":string("id"),"day":nullable(date("day"))}),
        );
        assert_eq!(one.code(), "INVALID_VALUE", "{text:?}");
    }
}

// --------------------------------------------------------------------------- identity

#[test]
fn null_or_duplicate_identity_fails_before_publication() {
    let one = One::new(
        "csv",
        b"id,note\n\\N,a\n",
        None,
        json!({"id":nullable(string("id")),"note":string("note")}),
    );
    assert_eq!(one.code(), "NULL_IDENTITY");
    let one = One::keyed(
        "csv",
        b"supplier,id\nA,1\nB,1\nA,1\n",
        None,
        json!({"supplier":string("supplier"),"id":string("id")}),
        &["supplier", "id"],
    );
    assert_eq!(one.code(), "DUPLICATE_IDENTITY");
    // Empty text is a real key value, distinct from null.
    let one = One::new("csv", b"id\n\"\"\n", None, json!({"id":string("id")}));
    assert_eq!(one.read().unwrap()[0].fields["id"], s(""));
}

// ------------------------------------------------------------------------ read roots

#[test]
fn read_roots_refuse_escapes_before_access() {
    let one = One::new("json", br#"{"id":"1"}"#, None, json!({"id":string("/id")}));
    let root = one.dir.path();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), br#"{"id":"1"}"#).unwrap();
    std::fs::create_dir(root.join("real")).unwrap();
    std::fs::write(root.join("real/input"), br#"{"id":"1"}"#).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("away")).unwrap();
    for path in [
        outside.path().join("secret"),
        outside.path().join("does-not-exist"),
        root.join("real/../input"),
        root.join("linked/input"),
        root.join("away/secret"),
    ] {
        let mut bindings = one.bindings.clone();
        bindings.get_mut("lines").unwrap().path = path.clone();
        assert_eq!(
            read_sources(&one.recipe, &bindings, &one.policy, &go())
                .unwrap_err()
                .code,
            "POLICY_DENIED",
            "{}",
            path.display()
        );
    }
    let mut bindings = one.bindings.clone();
    bindings.get_mut("lines").unwrap().path = root.join("real/input");
    assert!(read_sources(&one.recipe, &bindings, &one.policy, &go()).is_ok());
}

#[test]
fn non_regular_files_are_refused_without_blocking() {
    let one = One::new("csv", b"id\n1\n", None, json!({"id":string("id")}));
    let fifo = one.dir.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let mut bindings = one.bindings.clone();
    bindings.get_mut("lines").unwrap().path = fifo;
    assert_eq!(
        read_sources(&one.recipe, &bindings, &one.policy, &go())
            .unwrap_err()
            .code,
        "SOURCE_ACCESS"
    );
}

// ---------------------------------------------------------------------------- limits

fn lines_and_components(root: &Path, lines: &str, components: &str) -> (Recipe, Bindings) {
    std::fs::write(root.join("lines.json"), lines).unwrap();
    std::fs::write(root.join("components.csv"), components).unwrap();
    let recipe = recipe(
        vec![
            source(
                "lines",
                Some("/lines"),
                json!({"supplier":header(string("/supplier")),"invoice":header(string("/invoice")),"id":string("/id"),"billed":dec("/billed",18,2)}),
                &["supplier", "invoice", "id"],
            ),
            source(
                "components",
                None,
                json!({"id":string("id"),"supplier":string("supplier"),"invoice":string("invoice"),"line":string("line"),"amount":dec("amount",18,2)}),
                &["id"],
            ),
        ],
        vec![
            json!({"id":"parts","from":"lines","to":"components","from_keys":["supplier","invoice","id"],"to_keys":["supplier","invoice","line"],"cardinality":"one_to_many"}),
        ],
        vec![
            json!({"id":"review","anchor":"lines","columns":[{"name":"id","expr":{"op":"field","source":"lines","field":"id","path":[]}},{"name":"billed","expr":{"op":"field","source":"lines","field":"billed","path":[]}}]}),
        ],
    );
    let bindings = BTreeMap::from([
        ("lines".into(), binding(root.join("lines.json"), "json")),
        (
            "components".into(),
            binding(root.join("components.csv"), "csv"),
        ),
    ]);
    (recipe, bindings)
}
const LINES: &str = r#"{"supplier":"A","invoice":"A1","lines":[{"id":"1","billed":110},{"id":"2","billed":"20.00"}]}"#;
const COMPONENTS: &str = "id,supplier,invoice,line,amount\nc1,A,A1,1,1\nc2,A,A1,1,2\nc3,A,A1,1,3\n";

#[test]
fn row_limits_distinguish_invoice_lines_components_and_lookups() {
    let dir = tempfile::tempdir().unwrap();
    let (recipe, bindings) = lines_and_components(dir.path(), LINES, COMPONENTS);
    let mut policy = policy(dir.path());
    policy.limits.lookup_rows = 1;
    let rows = read_sources(&recipe, &bindings, &policy, &go()).unwrap();
    assert_eq!((rows["lines"].len(), rows["components"].len()), (2, 3));
    policy.limits.anchor_rows = 1;
    assert_eq!(
        read_sources(&recipe, &bindings, &policy, &go())
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
    policy.limits = Limits::default();
    policy.limits.component_rows = 2;
    assert_eq!(
        read_sources(&recipe, &bindings, &policy, &go())
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
}

#[test]
fn aggregate_encoded_input_is_counted_across_sources() {
    let dir = tempfile::tempdir().unwrap();
    let (recipe, bindings) = lines_and_components(dir.path(), LINES, COMPONENTS);
    let mut policy = policy(dir.path());
    policy.limits.file_bytes = LINES.len().max(COMPONENTS.len());
    policy.limits.input_bytes = LINES.len() + COMPONENTS.len();
    assert!(read_sources(&recipe, &bindings, &policy, &go()).is_ok());
    policy.limits.input_bytes -= 1;
    assert_eq!(
        read_sources(&recipe, &bindings, &policy, &go())
            .unwrap_err()
            .code,
        "RESOURCE_LIMIT"
    );
}

#[test]
fn field_token_and_depth_limits_apply_to_json_and_csv() {
    let mut one = One::new(
        "csv",
        b"id,note\n1,\"ab\ncd\"\n",
        None,
        json!({"id":string("id"),"note":string("note")}),
    );
    one.policy.limits.field_bytes = 4;
    assert_eq!(one.code(), "RESOURCE_LIMIT");
    one.policy.limits.field_bytes = 5;
    assert!(one.read().is_ok());
    // Unprojected columns are still bounded while reading.
    let mut one = One::new(
        "csv",
        b"id,unused\n1,abcdefgh\n",
        None,
        json!({"id":string("id")}),
    );
    one.policy.limits.field_bytes = 4;
    assert_eq!(one.code(), "RESOURCE_LIMIT");

    let mut one = One::new(
        "json",
        br#"{"id":"1","unused":123456789}"#,
        None,
        json!({"id":string("/id")}),
    );
    one.policy.limits.field_bytes = 8;
    assert_eq!(one.code(), "RESOURCE_LIMIT");
    let nested = format!(
        r#"{{"id":"1","deep":{}{}}}"#,
        "[".repeat(64),
        "]".repeat(64)
    );
    let one = One::new("json", nested.as_bytes(), None, json!({"id":string("/id")}));
    assert_eq!(one.code(), "RESOURCE_LIMIT", "default depth is 64");
    let nested = format!(
        r#"{{"id":"1","deep":{}{}}}"#,
        "[".repeat(63),
        "]".repeat(63)
    );
    let one = One::new("json", nested.as_bytes(), None, json!({"id":string("/id")}));
    assert!(one.read().is_ok());
}

#[test]
fn batch_byte_limit_rejects_a_single_oversized_record() {
    let mut one = One::new(
        "csv",
        b"id,note\n1,short\n2,a much longer note than the others\n",
        None,
        json!({"id":string("id"),"note":string("note")}),
    );
    let rows = one.read().unwrap();
    let small = serde_json::to_vec(&rows[0]).unwrap().len();
    one.policy.limits.batch_bytes = small;
    assert_eq!(one.code(), "RESOURCE_LIMIT");
}

#[test]
fn json_and_csv_deliver_the_same_typed_batch_contract() {
    let fields = json!({"id":string("id"),"amount":dec("amount",20,2),"day":date("day"),"n":nullable(int("n"))});
    let mut csv = One::new(
        "csv",
        b"id,amount,day,n\n1,1.10,2026-01-01,\\N\n2,2,2026-01-02,7\n3,3.30,2026-01-03,8\n4,4.40,2026-01-04,9\n5,5.50,2026-01-05,10\n",
        None,
        fields,
    );
    let mut json_fields = json!({"id":string("/id"),"amount":dec("/amount",20,2),"day":date("/day"),"n":nullable(int("/n"))});
    let doc = json!({"lines":(1..=5).map(|i|json!({"id":i.to_string(),"amount":format!("{i}.{i}0"),"day":format!("2026-01-0{i}"),"n":if i==1{Json::Null}else{json!(i+5)}})).collect::<Vec<_>>()}).to_string();
    let mut json = One::new("json", doc.as_bytes(), Some("/lines"), json_fields.take());
    let mut schemas = vec![];
    for one in [&mut json, &mut csv] {
        one.policy.limits.batch_rows = 2;
        let cancel = go();
        let mut sources = read_batches(&one.recipe, &one.bindings, &one.policy, &cancel).unwrap();
        let batches: Vec<_> = sources
            .remove("lines")
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            batches
                .iter()
                .map(|b| b.batch.num_rows())
                .collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
        assert!(
            batches
                .iter()
                .all(|b| b.observations.len() == b.batch.num_rows())
        );
        schemas.push(batches[0].batch.schema());
    }
    assert_eq!(schemas[0], schemas[1]);
    let schema = &schemas[0];
    use datafusion::arrow::datatypes::DataType;
    assert_eq!(
        schema.field_with_name("amount").unwrap().data_type(),
        &DataType::Decimal128(20, 2)
    );
    assert_eq!(
        schema.field_with_name("day").unwrap().data_type(),
        &DataType::Date32
    );
    assert!(schema.field_with_name("n").unwrap().is_nullable());
    assert!(!schema.field_with_name("id").unwrap().is_nullable());
}

// ---------------------------------------------------------------------- cancellation

#[test]
fn cancellation_between_batches_stops_delivery() {
    let mut one = One::new("csv", b"id\n1\n2\n3\n4\n", None, json!({"id":string("id")}));
    one.policy.limits.batch_rows = 2;
    let cancel = go();
    let mut sources = read_batches(&one.recipe, &one.bindings, &one.policy, &cancel).unwrap();
    let mut lines = sources.remove("lines").unwrap();
    assert_eq!(lines.next().unwrap().unwrap().batch.num_rows(), 2);
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(lines.next().unwrap().err().unwrap().code, "CANCELLED");
    assert!(lines.next().is_none());
}

// ---------------------------------------------------------------- public run seam

#[test]
fn public_run_retains_original_projected_values_and_references_per_line() {
    let dir = tempfile::tempdir().unwrap();
    let (recipe, bindings) = lines_and_components(dir.path(), LINES, COMPONENTS);
    let report = ExecutionContext::default().run(&recipe, &bindings, &policy(dir.path()), &go());
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    assert_eq!(
        report.tables["review"].len(),
        2,
        "components never add lines"
    );
    let digest: String = Sha256::digest(LINES.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let evidence: Vec<_> = report
        .evidence
        .iter()
        .filter(|e| e["output"] == "review")
        .collect();
    assert_eq!(evidence.len(), 2);
    let first = evidence
        .iter()
        .find(|e| e["source"]["location"] == "/lines/0")
        .unwrap();
    assert_eq!(first["source"]["fingerprint"], json!(digest));
    assert_eq!(first["source"]["source"], "lines");
    // The exact observed lexeme survives beside the typed 110.00.
    assert_eq!(first["original"]["billed"].to_string(), "110");
    assert_eq!(first["original"]["supplier"], "A");
    let second = evidence
        .iter()
        .find(|e| e["source"]["location"] == "/lines/1")
        .unwrap();
    assert_eq!(second["original"]["billed"], "20.00");
    let parts = &first["relationships"][0];
    assert_eq!(parts["candidate_count"], 3);
    let mut observed: Vec<_> = parts["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["reference"]["location"].as_str().unwrap().to_owned(),
                c["raw"]["amount"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    observed.sort();
    assert_eq!(
        observed,
        [("record:2", "1"), ("record:3", "2"), ("record:4", "3")]
            .map(|(l, a)| (l.to_owned(), a.to_owned()))
    );
}

#[test]
fn orphan_detail_is_reported_in_diagnostics_not_turned_into_lines() {
    let dir = tempfile::tempdir().unwrap();
    let components = format!("{COMPONENTS}c4,A,A1,9,4\nc5,B,A1,1,5\n");
    let (recipe, bindings) = lines_and_components(dir.path(), LINES, &components);
    let report = ExecutionContext::default().run(&recipe, &bindings, &policy(dir.path()), &go());
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    assert_eq!(report.tables["review"].len(), 2);
    let orphan = report
        .diagnostics
        .iter()
        .find(|d| d.code == "ORPHAN_DETAIL")
        .expect("orphan diagnostic");
    let context: Json = serde_json::from_str(orphan.context.as_deref().unwrap()).unwrap();
    assert_eq!(context["source"], "components");
    assert_eq!(context["count"], 2);
    assert_eq!(context["severity"], "warning");
    let locations: Vec<_> = context["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["location"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(locations, vec!["record:5", "record:6"]);
    assert!(
        !orphan.message.contains("c4"),
        "values are omitted by default"
    );
}

#[test]
fn public_run_fails_without_tables_on_source_contract_violations() {
    for (lines, components, code) in [
        (
            r#"{"supplier":"A","invoice":"A1","lines":[{"id":"1","billed":1,"id":"2"}]}"#,
            COMPONENTS,
            "DUPLICATE_JSON_KEY",
        ),
        (
            r#"{"supplier":"A","invoice":"A1","lines":[{"id":"1","billed":1},{"id":"1","billed":2}]}"#,
            COMPONENTS,
            "DUPLICATE_IDENTITY",
        ),
        (
            LINES,
            "id,supplier,invoice,line,amount\nc1,A,A1,1\n",
            "INVALID_CSV",
        ),
        (
            r#"{"supplier":"A","invoice":"A1","lines":[{"id":"1","billed":1.001}]}"#,
            COMPONENTS,
            "INVALID_DECIMAL",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (recipe, bindings) = lines_and_components(dir.path(), lines, components);
        let report =
            ExecutionContext::default().run(&recipe, &bindings, &policy(dir.path()), &go());
        assert_eq!(report.status, "failed");
        assert!(report.tables.is_empty() && report.evidence.is_empty());
        assert_eq!(report.diagnostics[0].code, code);
        assert_eq!(report.manifest["complete"], false);
    }
    let dir = tempfile::tempdir().unwrap();
    let (recipe, bindings) = lines_and_components(dir.path(), LINES, COMPONENTS);
    let report = ExecutionContext::default().run(
        &recipe,
        &bindings,
        &policy(dir.path()),
        &AtomicBool::new(true),
    );
    assert_eq!(report.status, "cancelled");
    assert!(report.tables.is_empty());
}
