//! Public XLSX adapter contract (#19): declared sheets, explicit numeric/date/cache
//! policies, original cell evidence and bounded package admission. Every case writes a
//! real `.xlsx` package to disk and reads it through the public adapter or run seam.
#[path = "support/workbook.rs"]
mod workbook_support;
use iresiphi::{ExecutionContext, adapters::read_sources, model::*};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use workbook_support::{package, set_part, sheet_xml, workbook_parts};

type Parts = Vec<(String, String)>;

fn inline(reference: &str, text: &str) -> String {
    format!(r#"<c r="{reference}" t="inlineStr"><is><t>{text}</t></is></c>"#)
}
fn header() -> String {
    format!(
        r#"<row r="1">{}{}{}</row>"#,
        inline("A1", "id"),
        inline("B1", "amount"),
        inline("C1", "supplier")
    )
}
/// Header row plus one data row whose B2 cell is the supplied XML.
fn one_row(b2: &str) -> String {
    format!(
        r#"{}<row r="2">{}{b2}{}</row>"#,
        header(),
        inline("A2", "x"),
        inline("C2", "A")
    )
}
fn standard(b2: &str) -> Parts {
    workbook_parts(&sheet_xml(Some("A1:C2"), &one_row(b2)), false)
}
fn standard_1904(b2: &str) -> Parts {
    workbook_parts(&sheet_xml(Some("A1:C2"), &one_row(b2)), true)
}

struct Case {
    _dir: tempfile::TempDir,
    recipe: Recipe,
    bindings: Bindings,
    policy: Policy,
}
impl Case {
    fn new(parts: &Parts) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.xlsx");
        std::fs::write(&path, package(parts)).unwrap();
        let recipe: Recipe = serde_json::from_value(json!({"format_version":"0.1","name":"xlsx","sources":[{"id":"lines","projection":{"fields":{"id":{"path":"id","data_type":{"kind":"string"}},"amount":{"path":"amount","data_type":{"kind":"decimal","precision":20,"scale":2}},"supplier":{"path":"supplier","data_type":{"kind":"string"}}}},"identity":["id"],"grain":["id"]}],"outputs":[]})).unwrap();
        let bindings = BTreeMap::from([(
            "lines".into(),
            Binding {
                path,
                adapter: "xlsx".into(),
                settings: BTreeMap::from([
                    ("sheet".into(), json!("Review")),
                    ("header_row".into(), json!(1)),
                ]),
            },
        )]);
        let policy = Policy {
            allowed_read_roots: vec![dir.path().into()],
            allowed_output_roots: vec![dir.path().into()],
            limits: Limits::default(),
            diagnostic_values: false,
        };
        Case {
            _dir: dir,
            recipe,
            bindings,
            policy,
        }
    }
    fn field(&mut self, name: &str) -> &mut Field {
        self.recipe.sources[0]
            .projection
            .fields
            .get_mut(name)
            .unwrap()
    }
    fn amount_type(mut self, data_type: Type) -> Self {
        self.field("amount").data_type = data_type;
        self
    }
    fn nullable_amount(mut self) -> Self {
        self.field("amount").nullable = true;
        self
    }
    fn setting(mut self, key: &str, value: serde_json::Value) -> Self {
        self.bindings
            .get_mut("lines")
            .unwrap()
            .settings
            .insert(key.into(), value);
        self
    }
    fn half_even(self) -> Self {
        self.setting("decimal_quantization", json!({"amount":"half_even"}))
    }
    fn excel_dates(self) -> Self {
        self.amount_type(Type::Date)
            .setting("excel_dates", json!(["amount"]))
    }
    fn read(&self) -> Result<Vec<Row>> {
        read_sources(
            &self.recipe,
            &self.bindings,
            &self.policy,
            &AtomicBool::new(false),
        )
        .map(|mut sources| sources.remove("lines").unwrap())
    }
    fn amount(&self) -> Value {
        let rows = self.read().unwrap_or_else(|e| panic!("{e:?}"));
        rows[0].fields["amount"].clone()
    }
    fn code(&self) -> String {
        match self.read() {
            Ok(rows) => panic!("expected an adapter error, read {:?}", rows[0].fields),
            Err(e) => e.code,
        }
    }
}
fn decimal(value: &str) -> Value {
    Value::Decimal {
        value: value.into(),
        precision: 20,
        scale: 2,
    }
}
fn date(value: &str) -> Value {
    Value::Date {
        value: value.into(),
    }
}
fn number(b2: &str) -> String {
    format!(r#"<c r="B2"><v>{b2}</v></c>"#)
}

// ---------------------------------------------------------------------------
// Public run seam

#[test]
fn public_run_enriches_invoice_lines_with_original_cell_evidence() {
    let root = tempfile::tempdir().unwrap();
    workbook_support::enrichment_workbook(&root.path().join("enrichment.xlsx"));
    std::fs::write(
        root.path().join("invoice.json"),
        r#"{"supplier":"A","lines":[{"id":"1"},{"id":"2"}]}"#,
    )
    .unwrap();
    let field = |source: &str, name: &str, path: &[&str]| json!({"op":"field","source":source,"field":name,"path":path});
    let recipe = Recipe::from_json(&json!({"format_version":"0.1","name":"xlsx enrichment","sources":[
        {"id":"invoice","identity":["supplier","id"],"grain":["supplier","id"],"projection":{"array":"/lines","fields":{"supplier":{"path":"/supplier","header":true,"data_type":{"kind":"string"}},"id":{"path":"/id","data_type":{"kind":"string"}}}}},
        {"id":"enrichment","identity":["supplier","id"],"grain":["supplier","id"],"projection":{"fields":{"id":{"path":"id","data_type":{"kind":"string"}},"supplier":{"path":"supplier","data_type":{"kind":"string"}},"amount":{"path":"amount","data_type":{"kind":"decimal","precision":18,"scale":2}}}}}],
        "relationships":[{"id":"enrich","from":"invoice","to":"enrichment","from_keys":["supplier","id"],"to_keys":["supplier","id"],"cardinality":"many_to_one"}],
        "outputs":[{"id":"review","anchor":"invoice","columns":[{"name":"id","expr":field("invoice","id",&[])},{"name":"enriched","expr":field("enrichment","amount",&["enrich"])}]}]}).to_string()).unwrap();
    let bindings = BTreeMap::from([
        (
            "invoice".to_string(),
            Binding {
                path: root.path().join("invoice.json"),
                adapter: "json".into(),
                settings: BTreeMap::new(),
            },
        ),
        (
            "enrichment".to_string(),
            Binding {
                path: root.path().join("enrichment.xlsx"),
                adapter: "xlsx".into(),
                settings: BTreeMap::from([
                    ("sheet".into(), json!("Review")),
                    ("header_row".into(), json!(1)),
                ]),
            },
        ),
    ]);
    let policy = Policy {
        allowed_read_roots: vec![root.path().into()],
        allowed_output_roots: vec![root.path().into()],
        limits: Limits::default(),
        diagnostic_values: false,
    };
    let report =
        ExecutionContext::default().run(&recipe, &bindings, &policy, &AtomicBool::new(false));
    assert_eq!(report.status, "succeeded", "{:?}", report.diagnostics);
    let enriched: BTreeMap<_, _> = report.tables["review"]
        .iter()
        .map(|r| {
            (
                serde_json::to_string(&r["id"]).unwrap(),
                r["enriched"].clone(),
            )
        })
        .collect();
    assert_eq!(
        enriched[&serde_json::to_string(&Value::String { value: "1".into() }).unwrap()],
        Value::Decimal {
            value: "7.00".into(),
            precision: 18,
            scale: 2
        }
    );
    assert_eq!(
        enriched[&serde_json::to_string(&Value::String { value: "2".into() }).unwrap()],
        Value::Decimal {
            value: "9.00".into(),
            precision: 18,
            scale: 2
        }
    );
    let line_one = report
        .evidence
        .iter()
        .find(|e| e["original"]["id"] == json!("1"))
        .expect("evidence for invoice line 1");
    let candidate = &line_one["relationships"][0]["candidates"][0];
    assert_eq!(line_one["relationships"][0]["candidate_count"], 1);
    assert_eq!(candidate["reference"]["location"], "Review!2");
    assert_eq!(candidate["raw"]["amount"]["cell"], "Review!B2");
    assert_eq!(candidate["raw"]["amount"]["observed"], "7.00");
    assert_eq!(candidate["raw"]["amount"]["cell_type"], "string");
}

// ---------------------------------------------------------------------------
// Declared sheet, header row and typed columns

#[test]
fn header_row_is_declared_not_inferred() {
    let rows = format!(
        r#"<row r="1">{}</row><row r="2">{}{}{}</row><row r="3">{}{}{}</row>"#,
        inline("A1", "Invoice enrichment export"),
        inline("A2", "id"),
        inline("B2", "amount"),
        inline("C2", "supplier"),
        inline("A3", "x"),
        inline("B3", "1.25"),
        inline("C3", "A"),
    );
    let case = Case::new(&workbook_parts(&sheet_xml(Some("A1:C3"), &rows), false))
        .setting("header_row", json!(2));
    let read = case.read().unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].fields["amount"], decimal("1.25"));
    assert_eq!(read[0].reference.location, "Review!3");
    assert_eq!(read[0].raw["amount"]["cell"], "Review!B3");
}

#[test]
fn sheet_and_header_row_must_be_explicit_and_exist() {
    let parts = standard(&inline("B2", "1.00"));
    let mut case = Case::new(&parts);
    case.bindings
        .get_mut("lines")
        .unwrap()
        .settings
        .remove("sheet");
    assert_eq!(case.code(), "INVALID_BINDING");
    let mut case = Case::new(&parts);
    case.bindings
        .get_mut("lines")
        .unwrap()
        .settings
        .remove("header_row");
    assert_eq!(case.code(), "INVALID_BINDING");
    assert_eq!(
        Case::new(&parts).setting("header_row", json!(0)).code(),
        "INVALID_BINDING"
    );
    assert_eq!(
        Case::new(&parts).setting("header_row", json!("1")).code(),
        "INVALID_BINDING"
    );
    assert_eq!(
        Case::new(&parts).setting("sheet", json!("Missing")).code(),
        "INVALID_BINDING"
    );
}

#[test]
fn unknown_binding_settings_are_rejected() {
    let case = Case::new(&standard(&inline("B2", "1.00")))
        .setting("decimal_quantisation", json!({"amount":"half_even"}));
    assert_eq!(case.code(), "INVALID_BINDING");
}

#[test]
fn missing_declared_columns_and_inexact_headers_fail() {
    let rows = format!(
        r#"<row r="1">{}{}{}</row><row r="2">{}{}{}</row>"#,
        inline("A1", "id"),
        inline("B1", "Amount"),
        inline("C1", "supplier"),
        inline("A2", "x"),
        inline("B2", "1.00"),
        inline("C2", "A"),
    );
    let case = Case::new(&workbook_parts(&sheet_xml(Some("A1:C2"), &rows), false));
    assert_eq!(case.code(), "INVALID_PROJECTION");
    let rows = rows.replace("<t>Amount</t>", r#"<t xml:space="preserve">amount </t>"#);
    let case = Case::new(&workbook_parts(&sheet_xml(Some("A1:C2"), &rows), false));
    assert_eq!(case.code(), "INVALID_PROJECTION");
}

#[test]
fn duplicate_header_names_fail() {
    let rows = format!(
        r#"<row r="1">{}{}{}{}</row><row r="2">{}{}{}{}</row>"#,
        inline("A1", "id"),
        inline("B1", "amount"),
        inline("C1", "supplier"),
        inline("D1", "amount"),
        inline("A2", "x"),
        inline("B2", "1.00"),
        inline("C2", "A"),
        inline("D2", "2.00"),
    );
    let case = Case::new(&workbook_parts(&sheet_xml(Some("A1:D2"), &rows), false));
    assert_eq!(case.code(), "INVALID_XLSX");
}

#[test]
fn cell_types_are_not_coerced_to_other_declared_types() {
    // A numeric cell is not a string, and a Boolean cell is not a string.
    let case = Case::new(&standard(&number("12"))).amount_type(Type::String);
    assert_eq!(case.code(), "INVALID_VALUE");
    let case = Case::new(&standard(r#"<c r="B2" t="b"><v>1</v></c>"#)).amount_type(Type::String);
    assert_eq!(case.code(), "INVALID_VALUE");
    // A Boolean cell is not a decimal.
    let case = Case::new(&standard(r#"<c r="B2" t="b"><v>1</v></c>"#)).half_even();
    assert_eq!(case.code(), "INVALID_VALUE");
}

#[test]
fn shared_strings_inline_strings_and_booleans_are_read() {
    let mut parts = workbook_parts(
        &sheet_xml(
            Some("A1:C3"),
            &format!(
                r#"{}<row r="2">{}<c r="B2" t="s"><v>0</v></c>{}</row><row r="3">{}<c r="B3" t="s"><v>1</v></c>{}</row>"#,
                header(),
                inline("A2", "x"),
                r#"<c r="C2" t="b"><v>1</v></c>"#,
                inline("A3", "y"),
                r#"<c r="C3" t="b"><v>0</v></c>"#,
            ),
        ),
        false,
    );
    set_part(
        &mut parts,
        "xl/sharedStrings.xml",
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>9007199254740993.01</t></si><si><r><t>12.</t></r><r><t>50</t></r></si></sst>"#,
    );
    set_part(
        &mut parts,
        "xl/_rels/workbook.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#,
    );
    let mut case = Case::new(&parts);
    case.field("supplier").data_type = Type::Boolean;
    let rows = case.read().unwrap();
    assert_eq!(rows[0].fields["amount"], decimal("9007199254740993.01"));
    assert_eq!(rows[1].fields["amount"], decimal("12.50"));
    assert_eq!(rows[0].fields["supplier"], Value::Boolean { value: true });
    assert_eq!(rows[1].fields["supplier"], Value::Boolean { value: false });
    assert_eq!(rows[0].raw["amount"]["cell_type"], "string");
    assert_eq!(rows[0].raw["supplier"]["cell_type"], "boolean");
}

#[test]
fn integer_cells_must_identify_exactly_one_int64() {
    let case = Case::new(&standard(&number("42"))).amount_type(Type::Int64);
    assert_eq!(case.amount(), Value::Int64 { value: "42".into() });
    let case = Case::new(&standard(&number("9007199254740991"))).amount_type(Type::Int64);
    assert_eq!(
        case.amount(),
        Value::Int64 {
            value: "9007199254740991".into()
        }
    );
    // 2^53 + 1 is stored as 2^53, so the stored double no longer identifies one integer.
    for stored in ["42.5", "9007199254740993", "-9007199254740992"] {
        let case = Case::new(&standard(&number(stored))).amount_type(Type::Int64);
        assert_eq!(case.code(), "INVALID_VALUE", "{stored}");
    }
}

// ---------------------------------------------------------------------------
// Decimals: text exact; numeric cells need an explicit quantization policy

#[test]
fn text_decimals_stay_exact_and_are_never_rounded() {
    let case = Case::new(&standard(&inline("B2", "9007199254740993.01")));
    assert_eq!(case.amount(), decimal("9007199254740993.01"));
    // Excess text scale is malformed for the declared type, even with a numeric policy.
    let case = Case::new(&standard(&inline("B2", "1.005"))).half_even();
    assert_eq!(case.code(), "INVALID_DECIMAL");
}

#[test]
fn numeric_cells_require_an_explicit_policy_even_when_integral() {
    let case = Case::new(&standard(&number("7")));
    assert_eq!(case.code(), "XLSX_QUANTIZATION_REQUIRED");
    assert_eq!(case.half_even().amount(), decimal("7.00"));
}

#[test]
fn numeric_cells_use_shortest_roundtrip_decimal_then_declared_rounding() {
    // 2.675 is stored as 2.67499999999999982236431605997495353221893310546875; the
    // documented conversion uses its shortest round-trip text "2.675" before rounding.
    for (stored, rounding, expected) in [
        ("2.675", "half_even", "2.68"),
        ("2.665", "half_even", "2.66"),
        ("2.675", "toward_zero", "2.67"),
        ("0.30000000000000004", "half_even", "0.30"),
        ("-2.675", "half_even", "-2.68"),
        ("-2.675", "toward_zero", "-2.67"),
        ("1E+21", "half_even", "1000000000000000000000.00"),
    ] {
        let mut case = Case::new(&standard(&number(stored)))
            .setting("decimal_quantization", json!({ "amount": rounding }));
        if expected.len() > 20 {
            case.field("amount").data_type = Type::Decimal {
                precision: 38,
                scale: 2,
            };
            let rows = case.read().unwrap();
            assert_eq!(
                rows[0].fields["amount"],
                Value::Decimal {
                    value: expected.into(),
                    precision: 38,
                    scale: 2
                }
            );
            continue;
        }
        assert_eq!(case.amount(), decimal(expected), "{stored} {rounding}");
    }
}

#[test]
fn numeric_quantization_is_checked_against_declared_precision() {
    let case = Case::new(&standard(&number("1E+30"))).half_even();
    assert_eq!(case.code(), "INVALID_DECIMAL");
    // Rounding up across a precision boundary also overflows rather than saturating.
    let mut case = Case::new(&standard(&number("9.995"))).half_even();
    case.field("amount").data_type = Type::Decimal {
        precision: 3,
        scale: 2,
    };
    assert_eq!(case.code(), "INVALID_DECIMAL");
}

#[test]
fn numeric_evidence_records_observed_representation_policy_and_cell() {
    let case = Case::new(&standard(&number("2.675"))).half_even();
    let rows = case.read().unwrap();
    let raw = &rows[0].raw["amount"];
    assert_eq!(raw["cell"], "Review!B2");
    assert_eq!(raw["cell_type"], "number");
    assert_eq!(raw["observed"], "2.675");
    assert_eq!(raw["quantization"], "half_even");
    assert_eq!(raw["conversion"], "shortest_roundtrip_decimal");
    assert_eq!(rows[0].reference.location, "Review!2");
    assert_eq!(rows[0].reference.fingerprint.len(), 64);
}

#[test]
fn quantization_policy_is_validated_even_without_numeric_cells() {
    let text = standard(&inline("B2", "1.00"));
    let case = Case::new(&text).setting("decimal_quantization", json!({"amount":"half_up"}));
    assert_eq!(case.code(), "INVALID_BINDING");
    let case = Case::new(&text).setting("decimal_quantization", json!({"undeclared":"half_even"}));
    assert_eq!(case.code(), "INVALID_BINDING");
    let case = Case::new(&text).setting("decimal_quantization", json!({"supplier":"half_even"}));
    assert_eq!(case.code(), "INVALID_BINDING");
    let case = Case::new(&text).setting("decimal_quantization", json!("half_even"));
    assert_eq!(case.code(), "INVALID_BINDING");
}

#[test]
fn nonfinite_numeric_cells_are_rejected_not_null() {
    for stored in ["NaN", "inf", "1e400"] {
        let case = Case::new(&standard(&number(stored)))
            .half_even()
            .nullable_amount();
        assert!(case.read().is_err(), "{stored} must be rejected");
    }
}

// ---------------------------------------------------------------------------
// Formula caches

#[test]
fn formula_cached_values_are_used_and_never_evaluated() {
    let case = Case::new(&standard(r#"<c r="B2"><f>1+1</f><v>7</v></c>"#)).half_even();
    let rows = case.read().unwrap();
    assert_eq!(rows[0].fields["amount"], decimal("7.00"));
    assert_eq!(rows[0].raw["amount"]["formula"], "1+1");
    let case = Case::new(&standard(r#"<c r="B2" t="str"><f>A2</f><v>3.50</v></c>"#));
    assert_eq!(case.amount(), decimal("3.50"));
}

#[test]
fn missing_formula_caches_fail() {
    for cell in [
        r#"<c r="B2"><f>1+1</f></c>"#,
        r#"<c r="B2"><f>1+1</f><v></v></c>"#,
        r#"<c r="B2"><f>1+1</f><v/></c>"#,
        r#"<c r="B2" t="str"><f>A2</f><v/></c>"#,
        r#"<c r="B2"><f t="shared" si="0"/></c>"#,
    ] {
        let case = Case::new(&standard(cell)).half_even().nullable_amount();
        assert_eq!(case.code(), "FORMULA_CACHE_MISSING", "{cell}");
    }
}

#[test]
fn caches_declared_stale_by_the_workbook_are_uncertain() {
    let mut parts = standard(r#"<c r="B2"><f>1+1</f><v>0</v></c>"#);
    let workbook = parts
        .iter()
        .find(|(n, _)| n == "xl/workbook.xml")
        .unwrap()
        .1
        .replace("</sheets>", r#"</sheets><calcPr fullCalcOnLoad="1"/>"#);
    set_part(&mut parts, "xl/workbook.xml", workbook);
    let case = Case::new(&parts).half_even();
    assert_eq!(case.code(), "FORMULA_CACHE_UNCERTAIN");
    // The same workbook without formulas in projected cells is still readable.
    let mut parts = standard(&inline("B2", "1.00"));
    let workbook = parts
        .iter()
        .find(|(n, _)| n == "xl/workbook.xml")
        .unwrap()
        .1
        .replace("</sheets>", r#"</sheets><calcPr fullCalcOnLoad="1"/>"#);
    set_part(&mut parts, "xl/workbook.xml", workbook);
    assert_eq!(Case::new(&parts).amount(), decimal("1.00"));
}

#[test]
fn cached_error_values_fail() {
    let case = Case::new(&standard(r#"<c r="B2" t="e"><f>1/0</f><v>#DIV/0!</v></c>"#))
        .half_even()
        .nullable_amount();
    assert_eq!(case.code(), "XLSX_CELL_ERROR");
}

// ---------------------------------------------------------------------------
// Dates

#[test]
fn both_epochs_map_declared_serials_to_gregorian_dates() {
    for (epoch1904, serial, expected) in [
        (false, "1", "1900-01-01"),
        (false, "59", "1900-02-28"),
        (false, "61", "1900-03-01"),
        (false, "45000", "2023-03-15"),
        (false, "2958465", "9999-12-31"),
        (true, "0", "1904-01-01"),
        (true, "59", "1904-02-29"),
        (true, "43538", "2023-03-15"),
    ] {
        let parts = if epoch1904 {
            standard_1904(&number(serial))
        } else {
            standard(&number(serial))
        };
        let case = Case::new(&parts).excel_dates();
        let rows = case.read().unwrap();
        assert_eq!(rows[0].fields["amount"], date(expected), "{serial}");
        assert_eq!(
            rows[0].raw["amount"]["date_system"],
            if epoch1904 { "1904" } else { "1900" }
        );
    }
}

#[test]
fn fictitious_fractional_and_out_of_range_serials_fail() {
    for (epoch1904, serial) in [
        (false, "60"),
        (false, "0"),
        (false, "-1"),
        (false, "45000.5"),
        (false, "2958466"),
        (true, "-1"),
        (true, "0.25"),
        (true, "2957004"),
    ] {
        let parts = if epoch1904 {
            standard_1904(&number(serial))
        } else {
            standard(&number(serial))
        };
        let case = Case::new(&parts).excel_dates();
        assert_eq!(case.code(), "INVALID_DATE", "{epoch1904} {serial}");
    }
}

#[test]
fn numeric_dates_require_an_explicit_binding_declaration() {
    let case = Case::new(&standard(&number("45000"))).amount_type(Type::Date);
    assert_eq!(case.code(), "INVALID_BINDING");
}

fn date_styled(b2: &str) -> Parts {
    let mut parts = standard(b2);
    set_part(
        &mut parts,
        "xl/styles.xml",
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/></cellXfs></styleSheet>"#,
    );
    set_part(
        &mut parts,
        "xl/_rels/workbook.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
    );
    parts
}

#[test]
fn date_formatted_numeric_cells_still_require_declaration() {
    let styled = date_styled(r#"<c r="B2" s="1"><v>45000</v></c>"#);
    let case = Case::new(&styled).amount_type(Type::Date);
    assert_eq!(case.code(), "INVALID_BINDING");
    let case = Case::new(&styled).excel_dates();
    let rows = case.read().unwrap();
    assert_eq!(rows[0].fields["amount"], date("2023-03-15"));
    assert_eq!(rows[0].raw["amount"]["cell_type"], "date_serial");
    let fractional = date_styled(r#"<c r="B2" s="1"><v>45000.75</v></c>"#);
    assert_eq!(Case::new(&fractional).excel_dates().code(), "INVALID_DATE");
}

#[test]
fn excel_date_declarations_must_name_declared_date_fields() {
    let parts = standard(&number("45000"));
    let case = Case::new(&parts)
        .amount_type(Type::Date)
        .setting("excel_dates", json!(["undeclared"]));
    assert_eq!(case.code(), "INVALID_BINDING");
    let case = Case::new(&parts)
        .amount_type(Type::Date)
        .setting("excel_dates", json!(["amount", "supplier"]));
    assert_eq!(case.code(), "INVALID_BINDING");
    let case = Case::new(&parts)
        .amount_type(Type::Date)
        .setting("excel_dates", json!("amount"));
    assert_eq!(case.code(), "INVALID_BINDING");
}

#[test]
fn iso_dates_are_explicit_and_not_locale_guessed() {
    let case = Case::new(&standard(&inline("B2", "2026-11-12"))).amount_type(Type::Date);
    assert_eq!(case.amount(), date("2026-11-12"));
    let case =
        Case::new(&standard(r#"<c r="B2" t="d"><v>2026-11-12</v></c>"#)).amount_type(Type::Date);
    assert_eq!(case.amount(), date("2026-11-12"));
    for cell in [
        inline("B2", "12/11/2026"),
        inline("B2", "2026-02-29"),
        inline("B2", "2026-1-2"),
        r#"<c r="B2" t="d"><v>2026-11-12T00:00:00</v></c>"#.to_owned(),
    ] {
        let case = Case::new(&standard(&cell)).amount_type(Type::Date);
        assert_eq!(case.code(), "INVALID_VALUE", "{cell}");
    }
}

/// The date system comes from the workbook part the package actually designates,
/// not from a conventional path.
#[test]
fn date_system_follows_the_relocated_workbook_part() {
    let sheet = sheet_xml(Some("A1:C2"), &one_row(&number("0")));
    let parts: Parts = vec![
        ("[Content_Types].xml".into(), r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/wb/book.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/wb/sheets/s1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.into()),
        ("_rels/.rels".into(), r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="wb/book.xml"/></Relationships>"#.into()),
        ("wb/book.xml".into(), r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="1"/><sheets><sheet name="Review" sheetId="1" r:id="rId1"/></sheets></workbook>"#.into()),
        ("wb/_rels/book.xml.rels".into(), r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="sheets/s1.xml"/></Relationships>"#.into()),
        ("wb/sheets/s1.xml".into(), sheet),
    ];
    let case = Case::new(&parts).excel_dates();
    let rows = case.read().unwrap();
    assert_eq!(rows[0].fields["amount"], date("1904-01-01"));
    assert_eq!(rows[0].raw["amount"]["date_system"], "1904");
}

// ---------------------------------------------------------------------------
// Blank cells versus empty strings

#[test]
fn blank_cells_are_null_only_for_nullable_fields() {
    let rows = format!(
        r#"{}<row r="2">{}{}</row>"#,
        header(),
        inline("A2", "x"),
        inline("C2", "A")
    );
    let parts = workbook_parts(&sheet_xml(Some("A1:C2"), &rows), false);
    assert_eq!(Case::new(&parts).code(), "NULL_REQUIRED");
    let case = Case::new(&parts).nullable_amount();
    let read = case.read().unwrap();
    assert_eq!(read[0].fields["amount"], Value::Null);
    assert_eq!(read[0].raw["amount"]["cell_type"], "blank");
    // A present-but-empty styled numeric cell is blank too.
    let case = Case::new(&standard(r#"<c r="B2" s="0"/>"#)).nullable_amount();
    assert_eq!(case.amount(), Value::Null);
}

#[test]
fn empty_strings_are_not_blank() {
    let mut parts = standard(&inline("B2", ""));
    let case = Case::new(&parts)
        .amount_type(Type::String)
        .nullable_amount();
    let rows = case.read().unwrap();
    assert_eq!(rows[0].fields["amount"], Value::String { value: "".into() });
    assert_eq!(rows[0].raw["amount"]["cell_type"], "string");
    // A shared empty string is also an empty string, not null.
    let sheet = sheet_xml(Some("A1:C2"), &one_row(r#"<c r="B2" t="s"><v>0</v></c>"#));
    set_part(&mut parts, "xl/worksheets/sheet1.xml", sheet);
    set_part(
        &mut parts,
        "xl/sharedStrings.xml",
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><t></t></si></sst>"#,
    );
    let case = Case::new(&parts)
        .amount_type(Type::String)
        .nullable_amount();
    assert_eq!(case.amount(), Value::String { value: "".into() });
    // An empty string is malformed for a decimal, never silently null.
    let case = Case::new(&standard(&inline("B2", ""))).nullable_amount();
    assert!(case.read().is_err());
}

// ---------------------------------------------------------------------------
// Package admission: ZIP preflight, cell accounting, field bytes, cancellation

#[test]
fn actual_decompressed_bytes_are_bounded_in_aggregate() {
    let mut parts = standard(&inline("B2", "1.00"));
    // Highly compressible parts: the archive is small, the expansion is not.
    set_part(&mut parts, "xl/media/padding-1.bin", "0".repeat(600 << 10));
    set_part(&mut parts, "xl/media/padding-2.bin", "0".repeat(600 << 10));
    let mut case = Case::new(&parts);
    let encoded = std::fs::metadata(&case.bindings["lines"].path)
        .unwrap()
        .len();
    assert!(encoded < 32 << 10, "fixture must be a compression bomb");
    case.policy.limits.xlsx_bytes = 1 << 20;
    assert_eq!(case.code(), "RESOURCE_LIMIT");
    case.policy.limits.xlsx_bytes = 2 << 20;
    assert_eq!(case.amount(), decimal("1.00"));
}

#[test]
fn decoded_cells_per_worksheet_are_bounded_without_a_dimension() {
    let parts = workbook_parts(&sheet_xml(None, &one_row(&inline("B2", "1.00"))), false);
    let mut case = Case::new(&parts);
    case.policy.limits.worksheet_cells = 5;
    assert_eq!(case.code(), "RESOURCE_LIMIT");
    case.policy.limits.worksheet_cells = 6;
    assert_eq!(case.amount(), decimal("1.00"));
}

#[test]
fn dimension_and_sparse_range_bombs_are_rejected_before_allocation() {
    let case = Case::new(&workbook_parts(
        &sheet_xml(Some("A1:XFD1048576"), &one_row(&inline("B2", "1.00"))),
        false,
    ));
    assert_eq!(case.code(), "RESOURCE_LIMIT");
    // Two populated cells far apart would make the reader allocate their bounding box.
    let rows = format!(
        r#"{}<row r="2">{}{}{}</row><row r="1048576"><c r="XFD1048576" t="inlineStr"><is><t>far</t></is></c></row>"#,
        header(),
        inline("A2", "x"),
        inline("B2", "1.00"),
        inline("C2", "A")
    );
    let case = Case::new(&workbook_parts(&sheet_xml(None, &rows), false));
    assert_eq!(case.code(), "RESOURCE_LIMIT");
}

/// Worksheets are identified by content and package relationships, so relocating
/// a sheet to an unconventional part name cannot bypass cell accounting.
#[test]
fn relocated_worksheet_parts_are_still_accounted() {
    for target in ["grid/data.xml", "worksheets/sheet1.bin"] {
        let mut parts = standard(&inline("B2", "1.00"));
        let sheet = parts
            .iter()
            .position(|(n, _)| n == "xl/worksheets/sheet1.xml")
            .unwrap();
        let (_, content) = parts.remove(sheet);
        parts.push((format!("xl/{target}"), content));
        set_part(
            &mut parts,
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="{target}"/></Relationships>"#
            ),
        );
        let mut case = Case::new(&parts);
        assert_eq!(case.amount(), decimal("1.00"), "{target} must be readable");
        case.policy.limits.worksheet_cells = 5;
        assert_eq!(case.code(), "RESOURCE_LIMIT", "{target}");
    }
}

#[test]
fn projected_field_bytes_are_bounded() {
    let mut case = Case::new(&standard(&inline("B2", "1.00"))).amount_type(Type::String);
    case.policy.limits.field_bytes = 9;
    let long = Case::new(&standard(&inline("B2", "0123456789"))).amount_type(Type::String);
    assert_eq!(
        case.amount(),
        Value::String {
            value: "1.00".into()
        }
    );
    let mut long = long;
    long.policy.limits.field_bytes = 9;
    assert_eq!(long.code(), "RESOURCE_LIMIT");
}

#[test]
fn unprojected_field_bytes_are_bounded_across_entities_and_cdata() {
    for note in [
        "aaaaaaaa&amp;aaaaaaaa&amp;aaaaaaaa".to_owned(),
        "<![CDATA[aaaaaaaaaaaaaaaaaaaaaaaa]]>".to_owned(),
    ] {
        let rows = format!(
            r#"<row r="1">{}{}{}{}</row><row r="2">{}{}{}<c r="D2" t="inlineStr"><is><t>{note}</t></is></c></row>"#,
            inline("A1", "id"),
            inline("B1", "amount"),
            inline("C1", "supplier"),
            inline("D1", "note"),
            inline("A2", "x"),
            inline("B2", "1.00"),
            inline("C2", "A"),
        );
        let mut case = Case::new(&workbook_parts(&sheet_xml(Some("A1:D2"), &rows), false));
        assert_eq!(case.amount(), decimal("1.00"));
        case.policy.limits.field_bytes = 16;
        assert_eq!(case.code(), "RESOURCE_LIMIT", "{note}");
    }
}

#[test]
fn xml_document_types_are_forbidden() {
    let sheet = format!(
        r#"<!DOCTYPE worksheet [<!ENTITY x "1.00">]>{}"#,
        sheet_xml(Some("A1:C2"), &one_row(&inline("B2", "1.00")))
    );
    let case = Case::new(&workbook_parts(&sheet, false));
    assert_eq!(case.code(), "INVALID_XLSX");
}

#[test]
fn malformed_packages_fail_with_stable_codes() {
    let dir = tempfile::tempdir().unwrap();
    let mut case = Case::new(&standard(&inline("B2", "1.00")));
    let path = dir.path().join("not-a-zip.xlsx");
    std::fs::write(&path, b"PK\x03\x04 not really a zip").unwrap();
    case.bindings.get_mut("lines").unwrap().path = path;
    case.policy.allowed_read_roots = vec![dir.path().into()];
    assert_eq!(case.code(), "INVALID_XLSX");
    let mut parts = standard(&inline("B2", "1.00"));
    set_part(
        &mut parts,
        "xl/worksheets/sheet1.xml",
        "<worksheet><sheetData>",
    );
    assert_eq!(Case::new(&parts).code(), "INVALID_XLSX");
}

#[test]
fn workbook_outside_allowed_roots_is_refused_before_access() {
    let mut case = Case::new(&standard(&inline("B2", "1.00")));
    let elsewhere = tempfile::tempdir().unwrap();
    case.policy.allowed_read_roots = vec![elsewhere.path().into()];
    assert_eq!(case.code(), "POLICY_DENIED");
}

#[test]
fn cancellation_stops_an_in_progress_workbook_read() {
    let mut rows = header();
    for row in 2..=40_000 {
        rows.push_str(&format!(
            r#"<row r="{row}">{}{}{}</row>"#,
            inline(&format!("A{row}"), &row.to_string()),
            inline(&format!("B{row}"), "1.00"),
            inline(&format!("C{row}"), "A"),
        ));
    }
    let case = Case::new(&workbook_parts(&sheet_xml(None, &rows), false));
    let started = std::time::Instant::now();
    let full = case.read().unwrap();
    let uncancelled = started.elapsed();
    assert_eq!(full.len(), 39_999);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let delay = uncancelled / 10;
    let setter = std::thread::spawn(move || {
        std::thread::sleep(delay);
        flag.store(true, Ordering::Relaxed);
    });
    let result = read_sources(&case.recipe, &case.bindings, &case.policy, &cancel);
    setter.join().unwrap();
    assert_eq!(result.unwrap_err().code, "CANCELLED");
}
