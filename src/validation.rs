use crate::model::*;
use serde::{Deserialize, Serialize, de};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    pub status: String,
    pub coverage: String,
    pub diagnostic: Option<Error>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub valid: bool,
    pub checks: Vec<Check>,
}
fn error(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, "validation", message)
}
fn iso_date_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}
pub fn value_type(v: &Value) -> Result<Option<Type>> {
    Ok(match v {
        Value::Null => None,
        Value::Boolean { .. } => Some(Type::Boolean),
        Value::String { .. } => Some(Type::String),
        Value::Int64 { value } => {
            value
                .parse::<i64>()
                .map_err(|_| error("INVALID_VALUE", "invalid Int64"))?;
            Some(Type::Int64)
        }
        Value::Decimal {
            value,
            precision,
            scale,
        } => {
            crate::scalar::Decimal::parse(value, *precision, *scale)
                .map_err(|e| error("INVALID_VALUE", e.to_string()))?;
            Some(Type::Decimal {
                precision: *precision,
                scale: *scale,
            })
        }
        Value::Date { value } => {
            if !iso_date_shape(value) {
                return Err(error("INVALID_VALUE", "date must be YYYY-MM-DD"));
            }
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|_| error("INVALID_VALUE", "invalid calendar date"))?;
            Some(Type::Date)
        }
    })
}
/// Validate a tagged value and return its canonical transport form
/// (`+007.5` at scale 2 becomes `7.50`; `-0` becomes `0`).
pub fn canonical_value(v: &Value) -> Result<Value> {
    value_type(v)?;
    Ok(match v {
        Value::Int64 { value } => Value::Int64 {
            value: value.parse::<i64>().expect("validated").to_string(),
        },
        Value::Decimal {
            value,
            precision,
            scale,
        } => Value::Decimal {
            value: crate::scalar::Decimal::parse(value, *precision, *scale)
                .expect("validated")
                .to_string(),
            precision: *precision,
            scale: *scale,
        },
        other => other.clone(),
    })
}
fn check_type(t: &Type) -> Result<()> {
    if let Type::Decimal { precision, scale } = t {
        if *precision == 0 || *precision > 38 || scale > precision {
            return Err(error(
                "INVALID_TYPE",
                "decimal requires 1 <= precision <= 38 and 0 <= scale <= precision",
            ));
        }
    }
    Ok(())
}
fn source<'a>(r: &'a Recipe, id: &str) -> Result<&'a Source> {
    r.sources
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| error("MISSING_REFERENCE", format!("source {id}")))
}
fn field<'a>(r: &'a Recipe, s: &str, f: &str) -> Result<&'a Field> {
    source(r, s)?
        .projection
        .fields
        .get(f)
        .ok_or_else(|| error("MISSING_REFERENCE", format!("field {s}.{f}")))
}
fn relationship<'a>(r: &'a Recipe, id: &str) -> Result<&'a Relationship> {
    r.relationships
        .iter()
        .find(|x| x.id == id)
        .ok_or_else(|| error("MISSING_REFERENCE", format!("relationship {id}")))
}
fn key_field<'a>(r: &'a Recipe, source_id: &str, key: &str) -> Result<&'a Field> {
    if let Ok(value) = field(r, source_id, key) {
        return Ok(value);
    }
    let mut parts = key.split('.').peekable();
    let mut current = source_id;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return field(r, current, part);
        }
        let rel = relationship(r, part)?;
        if rel.from != current || rel.cardinality == Cardinality::OneToMany {
            return Err(error(
                "INVALID_PATH",
                "lookup key must follow named to-one path",
            ));
        }
        current = &rel.to;
    }
    Err(error("INVALID_PATH", key))
}
fn compatible(a: Option<Type>, b: Option<Type>) -> Result<Option<Type>> {
    match (a, b) {
        (None, b) => Ok(b),
        (a, None) => Ok(a),
        (Some(a), Some(b)) if a == b => Ok(Some(a)),
        _ => Err(error("TYPE_MISMATCH", "explicit cast required")),
    }
}
fn boolean(t: Option<Type>) -> Result<()> {
    if t.is_some() && t != Some(Type::Boolean) {
        return Err(error("TYPE_MISMATCH", "Boolean required"));
    }
    Ok(())
}
/// Aggregate functions supported by format 0.1. Everything else, including
/// average, distinct count and allocation, is an explicit unsupported operation.
const AGGREGATES: [&str; 5] = ["count_records", "count_non_null", "sum", "min", "max"];
fn unsupported_aggregate(function: &str) -> Error {
    error(
        "UNSUPPORTED_OPERATION",
        format!("aggregate function {function} is not supported in format 0.1"),
    )
}
pub fn expression_type(e: &Expr, r: &Recipe) -> Result<Option<Type>> {
    infer(e, r, &mut BTreeSet::new())
}
fn infer(e: &Expr, r: &Recipe, visiting: &mut BTreeSet<String>) -> Result<Option<Type>> {
    use Expr::*;
    match e {
        Literal { value } => value_type(value),
        Field {
            source: s,
            field: f,
            path,
        } => {
            let mut end = None;
            for id in path {
                let rel = relationship(r, id)?;
                if end.is_some_and(|x| x != &rel.from) {
                    return Err(error("INVALID_PATH", "disconnected relationship path"));
                }
                if rel.cardinality == Cardinality::OneToMany {
                    return Err(error("GRAIN_VIOLATION", "scalar detail requires aggregate"));
                }
                end = Some(&rel.to);
            }
            if end.is_some_and(|x| x != s) {
                return Err(error("INVALID_PATH", "path does not reach source"));
            }
            Ok(Some(field(r, s, f)?.data_type.clone()))
        }
        Feature { id } => {
            if !visiting.insert(id.clone()) {
                return Err(error("CYCLIC_DEPENDENCY", id));
            }
            let feature = r
                .features
                .iter()
                .find(|f| f.id == *id)
                .ok_or_else(|| error("MISSING_REFERENCE", format!("feature {id}")))?;
            let result = infer(&feature.expr, r, visiting);
            visiting.remove(id);
            result
        }
        Divide { .. } => Err(error(
            "UNSUPPORTED_OPERATION",
            "division is deferred in format 0.1 and rejected statically, including division by zero",
        )),
        Add { left, right } | Subtract { left, right } | Multiply { left, right } => {
            let a = infer(left, r, visiting)?;
            let b = infer(right, r, visiting)?;
            match (a, b) {
                (None, None) => Ok(None),
                (None, Some(t)) | (Some(t), None)
                    if matches!(t, Type::Int64 | Type::Decimal { .. }) =>
                {
                    Ok(Some(t))
                }
                (Some(Type::Int64), Some(Type::Int64)) => Ok(Some(Type::Int64)),
                (
                    Some(Type::Decimal {
                        precision: p,
                        scale: s,
                    }),
                    Some(Type::Decimal {
                        precision: q,
                        scale: t,
                    }),
                ) => {
                    let (precision, scale) = if matches!(e, Multiply { .. }) {
                        (p + q, s + t)
                    } else {
                        (
                            p.saturating_sub(s).max(q.saturating_sub(t)) + s.max(t) + 1,
                            s.max(t),
                        )
                    };
                    if precision > 38 {
                        return Err(error(
                            "PRECISION_OVERFLOW",
                            format!(
                                "inferred decimal({precision},{scale}) exceeds precision 38; quantize or narrow inputs explicitly"
                            ),
                        ));
                    }
                    let ty = Type::Decimal { precision, scale };
                    check_type(&ty)?;
                    Ok(Some(ty))
                }
                _ => Err(error(
                    "TYPE_MISMATCH",
                    "arithmetic requires same numeric domains",
                )),
            }
        }
        Equal { left, right } | Less { left, right } | LessEqual { left, right } => {
            let a = infer(left, r, visiting)?;
            let b = infer(right, r, visiting)?;
            match (&a, &b) {
                (Some(Type::Decimal { .. }), Some(Type::Decimal { .. })) => {}
                _ => {
                    compatible(a, b)?;
                }
            }
            Ok(Some(Type::Boolean))
        }
        And { left, right } | Or { left, right } => {
            boolean(infer(left, r, visiting)?)?;
            boolean(infer(right, r, visiting)?)?;
            Ok(Some(Type::Boolean))
        }
        Not { expr } => {
            boolean(infer(expr, r, visiting)?)?;
            Ok(Some(Type::Boolean))
        }
        IsNull { expr } => {
            infer(expr, r, visiting)?;
            Ok(Some(Type::Boolean))
        }
        ValidOn {
            date,
            valid_from,
            valid_to,
        } => {
            for part in [date, valid_from, valid_to] {
                if !matches!(infer(part, r, visiting)?, None | Some(Type::Date)) {
                    return Err(error("TYPE_MISMATCH", "validity predicate requires dates"));
                }
            }
            Ok(Some(Type::Boolean))
        }
        Coalesce { values } => {
            if values.is_empty() {
                return Err(error("INVALID_EXPRESSION", "coalesce requires values"));
            }
            let mut ty = None;
            for v in values {
                ty = compatible(ty, infer(v, r, visiting)?)?;
            }
            Ok(ty)
        }
        Case {
            condition,
            then,
            otherwise,
        } => {
            boolean(infer(condition, r, visiting)?)?;
            compatible(infer(then, r, visiting)?, infer(otherwise, r, visiting)?)
        }
        Cast { expr, data_type } => {
            let source = infer(expr, r, visiting)?;
            check_type(data_type)?;
            let allowed = match (&source, data_type) {
                (None, _) => true,
                (Some(a), b) if a == b => true,
                (_, Type::String) => true,
                (Some(Type::String), _) => true,
                (Some(Type::Int64), Type::Decimal { .. })
                | (Some(Type::Decimal { .. }), Type::Decimal { .. })
                | (Some(Type::Decimal { .. }), Type::Int64) => true,
                _ => false,
            };
            if !allowed {
                return Err(error("INVALID_CAST", "unsupported explicit conversion"));
            }
            Ok(Some(data_type.clone()))
        }
        Quantize {
            expr,
            precision,
            scale,
            ..
        } => {
            if !matches!(infer(expr, r, visiting)?, None | Some(Type::Decimal { .. })) {
                return Err(error("TYPE_MISMATCH", "quantize requires decimal"));
            }
            let t = Type::Decimal {
                precision: *precision,
                scale: *scale,
            };
            check_type(&t)?;
            Ok(Some(t))
        }
        Aggregate {
            relationship: relationship_id,
            function,
            field: f,
            filter,
        } => {
            let rel = relationship(r, relationship_id)?;
            if let Some(p) = filter {
                boolean(infer(p, r, visiting)?)?;
            }
            let aggregate_field = || {
                field(
                    r,
                    &rel.to,
                    f.as_deref()
                        .ok_or_else(|| error("MISSING_REFERENCE", "aggregate field"))?,
                )
            };
            match function.as_str() {
                "count_records" => Ok(Some(Type::Int64)),
                "count_non_null" => {
                    aggregate_field()?;
                    Ok(Some(Type::Int64))
                }
                "sum" | "min" | "max" => {
                    let t = aggregate_field()?.data_type.clone();
                    if function == "sum" {
                        match t {
                            Type::Decimal { scale, .. } => Ok(Some(Type::Decimal {
                                precision: 38,
                                scale,
                            })),
                            Type::Int64 => Ok(Some(Type::Int64)),
                            _ => Err(error("TYPE_MISMATCH", "sum requires numeric input")),
                        }
                    } else {
                        Ok(Some(t))
                    }
                }
                other => Err(unsupported_aggregate(other)),
            }
        }
    }
}
fn check_output_paths(
    e: &Expr,
    r: &Recipe,
    anchor: &str,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    match e {
        Expr::Field { source, path, .. } => {
            if source != anchor && path.is_empty() {
                return Err(error("INVALID_PATH", "nonlocal field requires named path"));
            }
            if let Some(first) = path.first() {
                if relationship(r, first)?.from != anchor {
                    return Err(error("INVALID_PATH", "path must start at output anchor"));
                }
            }
        }
        Expr::Feature { id } => {
            if seen.insert(id.clone()) {
                if let Some(f) = r.features.iter().find(|f| f.id == *id) {
                    check_output_paths(&f.expr, r, anchor, seen)?;
                }
            }
        }
        Expr::Aggregate {
            relationship: id, ..
        } => {
            if relationship(r, id)?.from != anchor {
                return Err(error("INVALID_PATH", "aggregate must begin at anchor"));
            }
        }
        _ => {
            for child in children(e) {
                check_output_paths(child, r, anchor, seen)?;
            }
        }
    }
    Ok(())
}
fn children(e: &Expr) -> Vec<&Expr> {
    use Expr::*;
    match e {
        Add { left, right }
        | Subtract { left, right }
        | Multiply { left, right }
        | Divide { left, right }
        | Equal { left, right }
        | Less { left, right }
        | LessEqual { left, right }
        | And { left, right }
        | Or { left, right } => vec![left, right],
        Not { expr } | IsNull { expr } | Cast { expr, .. } | Quantize { expr, .. } => vec![expr],
        ValidOn {
            date,
            valid_from,
            valid_to,
        } => vec![date, valid_from, valid_to],
        Coalesce { values } => values.iter().collect(),
        Case {
            condition,
            then,
            otherwise,
        } => vec![condition, then, otherwise],
        Aggregate {
            filter: Some(f), ..
        } => vec![f],
        _ => vec![],
    }
}
fn foreign_field(e: &Expr, r: &Recipe, anchor: &str, seen: &mut BTreeSet<String>) -> bool {
    match e {
        Expr::Field { source, .. } => source != anchor,
        Expr::Feature { id } => {
            seen.insert(id.clone())
                && r.features
                    .iter()
                    .find(|f| f.id == *id)
                    .is_some_and(|f| foreign_field(&f.expr, r, anchor, seen))
        }
        _ => children(e)
            .into_iter()
            .any(|child| foreign_field(child, r, anchor, seen)),
    }
}

// ---------------------------------------------------------------------------
// Canonical JSON and content identity.

/// Rewrite tagged Int64/decimal literals to canonical text so equal values
/// have one serialization. Invalid literals are left for validation to reject.
fn normalize_literals(value: &mut Json) {
    match value {
        Json::Object(map) => {
            let small = |key: &str| {
                map.get(key)
                    .and_then(Json::as_u64)
                    .and_then(|n| u8::try_from(n).ok())
            };
            let literal = match (
                map.get("kind").and_then(Json::as_str),
                map.get("value").and_then(Json::as_str),
            ) {
                (Some("int64"), Some(text)) if map.len() == 2 => {
                    Some(Value::Int64 { value: text.into() })
                }
                (Some("decimal"), Some(text)) if map.len() == 4 => {
                    match (small("precision"), small("scale")) {
                        (Some(precision), Some(scale)) => Some(Value::Decimal {
                            value: text.into(),
                            precision,
                            scale,
                        }),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(Ok(canonical)) = literal.as_ref().map(canonical_value) {
                *value = serde_json::to_value(canonical).expect("values serialize");
                return;
            }
            for child in map.values_mut() {
                normalize_literals(child);
            }
        }
        Json::Array(values) => values.iter_mut().for_each(normalize_literals),
        _ => {}
    }
}
/// Recursively key-sorted, compact UTF-8 JSON with preserved array order.
pub fn canonical_json(value: &Json) -> String {
    fn sort(value: &Json) -> Json {
        match value {
            Json::Object(map) => {
                let ordered: BTreeMap<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), sort(v))).collect();
                serde_json::to_value(ordered).expect("JSON values serialize")
            }
            Json::Array(values) => Json::Array(values.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(value)).expect("JSON values serialize")
}
fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}
fn content_hash(value: &Json) -> String {
    let mut value = value.clone();
    normalize_literals(&mut value);
    sha256_hex(&canonical_json(&value))
}
fn declaration(r: &Recipe, id: &str) -> Option<Json> {
    r.sources
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| serde_json::to_value(s).ok())
        .or_else(|| {
            r.features
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| serde_json::to_value(f).ok())
        })
        .or_else(|| {
            r.relationships
                .iter()
                .find(|x| x.id == id)
                .and_then(|x| serde_json::to_value(x).ok())
        })
}

// ---------------------------------------------------------------------------
// Document loading: duplicate keys and deferred constructs.

const DUPLICATE_KEY: &str = "iresiphi duplicate object key";
/// Accepts any JSON while rejecting repeated object keys, which serde would
/// otherwise resolve silently (last value wins for maps).
struct UniqueKeys;
impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        d.deserialize_any(UniqueKeys)
    }
}
impl<'de> de::Visitor<'de> for UniqueKeys {
    type Value = UniqueKeys;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<Self, E> {
        Ok(self)
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<Self, E> {
        Ok(self)
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<Self, E> {
        Ok(self)
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<Self, E> {
        Ok(self)
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<Self, E> {
        Ok(self)
    }
    fn visit_unit<E>(self) -> std::result::Result<Self, E> {
        Ok(self)
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Self, A::Error> {
        while a.next_element::<UniqueKeys>()?.is_some() {}
        Ok(self)
    }
    fn visit_map<A: de::MapAccess<'de>>(self, mut a: A) -> std::result::Result<Self, A::Error> {
        let mut seen = BTreeSet::new();
        while let Some(key) = a.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("{DUPLICATE_KEY} {key:?}")));
            }
            a.next_value::<UniqueKeys>()?;
        }
        Ok(self)
    }
}
/// Operation names deliberately deferred by the preview. They fail with an
/// explicit diagnostic instead of a generic unknown-variant parse error.
const DEFERRED_OPERATIONS: [&str; 20] = [
    "window",
    "rank",
    "row_number",
    "lag",
    "lead",
    "udf",
    "python",
    "callback",
    "regex",
    "sql",
    "average",
    "avg",
    "mean",
    "distinct_count",
    "count_distinct",
    "allocate",
    "allocation",
    "fx",
    "tax",
    "tier",
];
const DEFERRED_TYPES: [&str; 7] = [
    "timestamp",
    "timestamp_tz",
    "time",
    "float",
    "float32",
    "float64",
    "double",
];
fn deferred(value: &Json, pointer: &str) -> Result<()> {
    match value {
        Json::Object(map) => {
            if let Some(op) = map.get("op").and_then(Json::as_str) {
                if DEFERRED_OPERATIONS.contains(&op) {
                    return Err(error(
                        "UNSUPPORTED_OPERATION",
                        format!("operation {op} at {pointer} is deferred in format 0.1"),
                    ));
                }
            }
            if let Some(kind) = map.get("kind").and_then(Json::as_str) {
                if DEFERRED_TYPES.contains(&kind) {
                    return Err(error(
                        "UNSUPPORTED_TYPE",
                        format!("type {kind} at {pointer} is deferred in format 0.1"),
                    ));
                }
            }
            for (key, child) in map {
                if pointer.is_empty() && key == "annotations" {
                    continue;
                }
                deferred(child, &format!("{pointer}/{key}"))?;
            }
        }
        Json::Array(values) => {
            for (i, child) in values.iter().enumerate() {
                deferred(child, &format!("{pointer}/{i}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Saved documents must not carry machine-specific absolute paths.
fn machine_path(text: &str) -> bool {
    let b = text.as_bytes();
    text.starts_with('/')
        || text.starts_with('\\')
        || text.starts_with('~')
        || text.to_ascii_lowercase().starts_with("file:")
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/'))
}
fn no_machine_paths(value: &Json, place: &str) -> Result<()> {
    match value {
        Json::String(text) if machine_path(text) => Err(error(
            "ABSOLUTE_PATH",
            format!("{place} contains an absolute machine path; bind paths at run time"),
        )),
        Json::Object(map) => map
            .iter()
            .try_for_each(|(k, v)| no_machine_paths(v, &format!("{place}/{k}"))),
        Json::Array(values) => values.iter().try_for_each(|v| no_machine_paths(v, place)),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Static validation stages.

type Stage = (&'static str, &'static str, fn(&Recipe) -> Result<()>);
const STAGES: [Stage; 8] = [
    (
        "format",
        "document format_version and nonsemantic annotations; no source data read",
        check_format,
    ),
    (
        "capabilities",
        "required capabilities against this build; no source data read",
        check_capabilities,
    ),
    (
        "identifiers",
        "declaration IDs and output column names; no source data read",
        check_identifiers,
    ),
    (
        "bundled_definitions",
        "bundled definition content hashes and provenance; no source data read",
        check_bundles,
    ),
    (
        "sources",
        "declared projections, field types, identity and grain; no source data read",
        check_sources,
    ),
    (
        "relationships",
        "relationship endpoints, key types, validity fields and cycles; no source data read",
        check_relationships,
    ),
    (
        "expressions",
        "feature, column, predicate and aggregate typing, references and supported operations; no source data read",
        check_expressions,
    ),
    (
        "outputs",
        "output anchors, named paths, grain and currency rules; no source data read",
        check_outputs,
    ),
];
fn check_format(r: &Recipe) -> Result<()> {
    if r.format_version != "0.1" {
        return Err(error(
            "UNSUPPORTED_VERSION",
            format!(
                "format_version {:?} is not accepted; this build reads exactly \"0.1\"",
                r.format_version
            ),
        ));
    }
    no_machine_paths(
        &serde_json::to_value(&r.annotations).expect("annotations serialize"),
        "annotations",
    )
}
fn check_capabilities(r: &Recipe) -> Result<()> {
    const SUPPORTED: [&str; 11] = [
        "local",
        "json",
        "csv",
        "xlsx",
        "exact_decimal",
        "date",
        "lookup",
        "aggregate",
        "broadcast",
        "memory",
        "bundle",
    ];
    for cap in &r.required_capabilities {
        if !SUPPORTED.contains(&cap.as_str()) {
            return Err(error("UNSUPPORTED_CAPABILITY", cap));
        }
    }
    Ok(())
}
fn check_identifiers(r: &Recipe) -> Result<()> {
    let mut ids = BTreeSet::new();
    for id in r
        .sources
        .iter()
        .map(|s| &s.id)
        .chain(r.relationships.iter().map(|s| &s.id))
        .chain(r.features.iter().map(|s| &s.id))
        .chain(r.outputs.iter().map(|s| &s.id))
    {
        if id.is_empty() || !ids.insert(id) {
            return Err(error("DUPLICATE_ID", format!("declaration ID {id:?}")));
        }
    }
    for out in &r.outputs {
        let mut names = BTreeSet::new();
        for name in out
            .columns
            .iter()
            .map(|c| &c.name)
            .chain(out.aggregates.iter().map(|a| &a.name))
        {
            if name.is_empty() || !names.insert(name) {
                return Err(error(
                    "DUPLICATE_ID",
                    format!("output {} column {name:?}", out.id),
                ));
            }
        }
    }
    Ok(())
}
fn check_bundles(r: &Recipe) -> Result<()> {
    for (id, definition) in &r.bundled_definitions {
        if definition.content_hash != content_hash(&definition.content) {
            return Err(error("CONTENT_HASH_MISMATCH", id));
        }
        if declaration(r, id).as_ref() != Some(&definition.content) {
            return Err(error("BUNDLED_DEFINITION_MISMATCH", id));
        }
        if let Some(provenance) = &definition.provenance {
            no_machine_paths(&json!(provenance), &format!("bundled_definitions/{id}"))?;
        }
    }
    Ok(())
}
fn check_sources(r: &Recipe) -> Result<()> {
    for s in &r.sources {
        if s.identity.is_empty() || s.grain.is_empty() {
            return Err(error(
                "INVALID_IDENTITY",
                "identity and grain must be explicit",
            ));
        }
        for f in s.projection.fields.values() {
            check_type(&f.data_type)?;
        }
        for key in s.identity.iter().chain(&s.grain) {
            field(r, &s.id, key)?;
        }
        for key in &s.identity {
            if field(r, &s.id, key)?.nullable {
                return Err(error("INVALID_IDENTITY", "identity cannot be nullable"));
            }
        }
    }
    Ok(())
}
fn check_relationships(r: &Recipe) -> Result<()> {
    for rel in &r.relationships {
        source(r, &rel.from)?;
        source(r, &rel.to)?;
        if rel.from_keys.is_empty() || rel.from_keys.len() != rel.to_keys.len() {
            return Err(error(
                "INVALID_RELATIONSHIP",
                "keys must be nonempty and same length",
            ));
        }
        for (a, b) in rel.from_keys.iter().zip(&rel.to_keys) {
            if key_field(r, &rel.from, a)?.data_type != field(r, &rel.to, b)?.data_type {
                return Err(error(
                    "TYPE_MISMATCH",
                    "relationship keys require equal types",
                ));
            }
        }
        if let Some(v) = &rel.validity {
            for (s, k) in [
                (&rel.from, &v.date),
                (&rel.to, &v.valid_from),
                (&rel.to, &v.valid_to),
            ] {
                if field(r, s, k)?.data_type != Type::Date {
                    return Err(error("TYPE_MISMATCH", "validity fields require Date"));
                }
            }
            if field(r, &rel.from, &v.date)?.nullable || field(r, &rel.to, &v.valid_from)?.nullable
            {
                return Err(error(
                    "INVALID_RELATIONSHIP",
                    "invoice date and valid_from are required",
                ));
            }
        }
    }
    fn visit_relationship<'a>(
        rel: &'a Relationship,
        r: &'a Recipe,
        active: &mut BTreeSet<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Result<()> {
        if done.contains(rel.id.as_str()) {
            return Ok(());
        }
        if !active.insert(&rel.id) {
            return Err(error("CYCLIC_DEPENDENCY", &rel.id));
        }
        for key in &rel.from_keys {
            if field(r, &rel.from, key).is_ok() {
                continue;
            }
            let parts: Vec<_> = key.split('.').collect();
            for id in parts.iter().take(parts.len().saturating_sub(1)) {
                visit_relationship(relationship(r, id)?, r, active, done)?;
            }
        }
        active.remove(rel.id.as_str());
        done.insert(&rel.id);
        Ok(())
    }
    let mut relationships_done = BTreeSet::new();
    for rel in &r.relationships {
        visit_relationship(rel, r, &mut BTreeSet::new(), &mut relationships_done)?;
    }
    fn visit<'a>(
        id: &'a str,
        r: &'a Recipe,
        active: &mut BTreeSet<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        if !active.insert(id) {
            return Err(error("CYCLIC_DEPENDENCY", id));
        }
        for rel in r.relationships.iter().filter(|x| x.from == id) {
            visit(&rel.to, r, active, done)?;
        }
        active.remove(id);
        done.insert(id);
        Ok(())
    }
    let mut done = BTreeSet::new();
    for s in &r.sources {
        visit(&s.id, r, &mut BTreeSet::new(), &mut done)?;
    }
    Ok(())
}
fn check_expressions(r: &Recipe) -> Result<()> {
    for f in &r.features {
        expression_type(&f.expr, r)?;
    }
    for out in &r.outputs {
        for c in &out.columns {
            expression_type(&c.expr, r)?;
        }
        for p in [&out.eligibility, &out.filter].into_iter().flatten() {
            boolean(expression_type(p, r)?)?;
        }
        for aggregate in &out.aggregates {
            if !AGGREGATES.contains(&aggregate.function.as_str()) {
                return Err(unsupported_aggregate(&aggregate.function));
            }
            expression_type(&aggregate.expr, r)?;
        }
    }
    Ok(())
}
fn check_outputs(r: &Recipe) -> Result<()> {
    for out in &r.outputs {
        source(r, &out.anchor)?;
        for c in &out.columns {
            check_output_paths(&c.expr, r, &out.anchor, &mut BTreeSet::new())?;
        }
        for p in [&out.eligibility, &out.filter].into_iter().flatten() {
            check_output_paths(p, r, &out.anchor, &mut BTreeSet::new())?;
        }
        for g in &out.group_by {
            if !out.columns.iter().any(|c| c.name == *g) {
                return Err(error("MISSING_REFERENCE", format!("group column {g}")));
            }
        }
        for aggregate in &out.aggregates {
            let ty = expression_type(&aggregate.expr, r)?;
            check_output_paths(&aggregate.expr, r, &out.anchor, &mut BTreeSet::new())?;
            if aggregate.function == "sum" {
                if !matches!(ty, None | Some(Type::Int64) | Some(Type::Decimal { .. })) {
                    return Err(error("TYPE_MISMATCH", "sum requires numeric input"));
                }
                if foreign_field(&aggregate.expr, r, &out.anchor, &mut BTreeSet::new()) {
                    return Err(error(
                        "GRAIN_VIOLATION",
                        "cannot sum broadcast measure at recipient grain",
                    ));
                }
            }
        }
        let keeps_currency = out.group_by.iter().any(|g| {
            out.columns.iter().any(|c| {
                c.name == *g
                    && matches!(&c.expr, Expr::Field { source, field, path }
                        if source == &out.anchor && field == "currency" && path.is_empty())
            })
        });
        if !out.aggregates.is_empty()
            && source(r, &out.anchor)?
                .projection
                .fields
                .contains_key("currency")
            && !keeps_currency
        {
            return Err(error("GRAIN_VIOLATION", "grouping must retain currency"));
        }
    }
    Ok(())
}
fn static_report(r: &Recipe) -> Vec<Check> {
    let mut failed = false;
    STAGES
        .iter()
        .map(|(id, coverage, stage)| {
            if failed {
                return Check {
                    id: (*id).into(),
                    status: "not-run".into(),
                    coverage: "none; an earlier static check failed".into(),
                    diagnostic: None,
                };
            }
            let result = stage(r);
            failed = result.is_err();
            Check {
                id: (*id).into(),
                status: if failed { "failed" } else { "passed" }.into(),
                coverage: (*coverage).into(),
                diagnostic: result.err(),
            }
        })
        .collect()
}
fn static_checks(r: &Recipe) -> Result<()> {
    for (_, _, stage) in STAGES {
        stage(r)?;
    }
    Ok(())
}
/// `validate(recipe)`: version, reference, type, path, grain and capability
/// validation without opening sources. It never implies data correctness.
pub fn validate(r: &Recipe) -> Report {
    let mut checks = static_report(r);
    checks.push(Check {
        id: "data".into(),
        status: "not-run".into(),
        coverage: "none; static validation reads no source data".into(),
        diagnostic: None,
    });
    Report {
        valid: checks.iter().all(|c| c.status != "failed"),
        checks,
    }
}

// ---------------------------------------------------------------------------
// Explain.

#[derive(Default)]
struct Lineage {
    depends_on: BTreeSet<String>,
    origins: BTreeSet<(String, String, Vec<String>)>,
    operations: BTreeSet<String>,
    grain_transitions: Vec<Json>,
    unsupported: Vec<Json>,
}
fn op_name(e: &Expr) -> String {
    serde_json::to_value(e)
        .ok()
        .and_then(|v| v.get("op").and_then(Json::as_str).map(str::to_owned))
        .unwrap_or_default()
}
fn trace(e: &Expr, r: &Recipe, direct: bool, seen: &mut BTreeSet<String>, out: &mut Lineage) {
    out.operations.insert(op_name(e));
    match e {
        Expr::Field {
            source,
            field,
            path,
        } => {
            if direct {
                out.depends_on
                    .insert(format!("source:{source}/field:{field}"));
                out.depends_on
                    .extend(path.iter().map(|p| format!("relationship:{p}")));
            }
            out.origins
                .insert((source.clone(), field.clone(), path.clone()));
        }
        Expr::Feature { id } => {
            if direct {
                out.depends_on.insert(format!("feature:{id}"));
            }
            if seen.insert(id.clone()) {
                if let Some(f) = r.features.iter().find(|f| f.id == *id) {
                    trace(&f.expr, r, false, seen, out);
                }
            }
        }
        Expr::Divide { .. } => out.unsupported.push(json!({
            "code":"UNSUPPORTED_OPERATION","operation":"divide",
            "reason":"division is deferred in format 0.1"
        })),
        Expr::Aggregate {
            relationship,
            function,
            field,
            ..
        } => {
            if direct {
                out.depends_on
                    .insert(format!("relationship:{relationship}"));
            }
            if !AGGREGATES.contains(&function.as_str()) {
                out.unsupported.push(json!({
                    "code":"UNSUPPORTED_OPERATION","operation":function,
                    "reason":"aggregate function is not supported in format 0.1"
                }));
            }
            if let Some(rel) = r.relationships.iter().find(|x| x.id == *relationship) {
                if let Some(f) = field {
                    out.origins
                        .insert((rel.to.clone(), f.clone(), vec![relationship.clone()]));
                }
                out.grain_transitions.push(json!({
                    "from":format!("source:{}",rel.to),"to":format!("source:{}",rel.from),
                    "via":format!("relationship:{relationship}"),"function":function
                }));
            }
        }
        _ => {}
    }
    for child in children(e) {
        trace(child, r, direct, seen, out);
    }
}
fn expression_node(id: String, kind: &str, e: &Expr, r: &Recipe, all: &mut Vec<Json>) -> Json {
    let mut lineage = Lineage::default();
    trace(e, r, true, &mut BTreeSet::new(), &mut lineage);
    for item in &lineage.unsupported {
        let mut item = item.clone();
        item["node"] = json!(id);
        all.push(item);
    }
    json!({
        "id": id,
        "kind": kind,
        "type": expression_type(e, r).ok().flatten(),
        "depends_on": lineage.depends_on,
        "operations": lineage.operations,
        "measure_origins": lineage.origins.iter().map(|(s,f,p)| json!({"source":format!("source:{s}"),"field":f,"path":p})).collect::<Vec<_>>(),
        "grain_transitions": lineage.grain_transitions,
        "unsupported": lineage.unsupported,
    })
}
fn sorted<'a, T>(items: &'a [T], id: impl Fn(&T) -> &str) -> Vec<&'a T> {
    let mut items: Vec<_> = items.iter().collect();
    items.sort_by(|a, b| id(a).cmp(id(b)));
    items
}
/// Logical explanation with stable declaration IDs. It never opens sources,
/// runs data checks or claims a cost optimum.
pub fn explain(r: &Recipe) -> Json {
    let mut nodes = vec![];
    let mut unsupported = vec![];
    for s in sorted(&r.sources, |s| &s.id) {
        nodes.push(json!({
            "id": format!("source:{}", s.id),
            "kind": "scan",
            "placement": "local",
            "fields": s.projection.fields.iter().map(|(name, f)| json!({"id":format!("source:{}/field:{name}",s.id),"path":f.path,"type":f.data_type,"nullable":f.nullable})).collect::<Vec<_>>(),
            "identity": s.identity,
            "grain": s.grain,
            "estimates": {"rows":null,"bytes":null,"status":"unknown"},
        }));
    }
    for rel in sorted(&r.relationships, |x| &x.id) {
        let mut depends_on =
            BTreeSet::from([format!("source:{}", rel.from), format!("source:{}", rel.to)]);
        for key in &rel.from_keys {
            if field(r, &rel.from, key).is_err() {
                let parts: Vec<_> = key.split('.').collect();
                for id in &parts[..parts.len().saturating_sub(1)] {
                    depends_on.insert(format!("relationship:{id}"));
                }
            }
        }
        nodes.push(json!({
            "id": format!("relationship:{}", rel.id),
            "kind": if rel.cardinality == Cardinality::OneToMany { "detail" } else { "checked_lookup" },
            "from": format!("source:{}", rel.from),
            "to": format!("source:{}", rel.to),
            "keys": rel.from_keys.iter().zip(&rel.to_keys).map(|(a,b)| json!({"from":a,"to":b})).collect::<Vec<_>>(),
            "cardinality": rel.cardinality,
            "validity": rel.validity.as_ref().map(|v| json!({"date":v.date,"valid_from":v.valid_from,"valid_to":v.valid_to,"predicate":"valid_from <= date < valid_to; null valid_to is open-ended"})),
            "strict": rel.strict,
            "depends_on": depends_on,
        }));
    }
    for f in sorted(&r.features, |f| &f.id) {
        nodes.push(expression_node(
            format!("feature:{}", f.id),
            "feature",
            &f.expr,
            r,
            &mut unsupported,
        ));
    }
    for out in sorted(&r.outputs, |o| &o.id) {
        let base = format!("output:{}", out.id);
        let anchor_grain = r
            .sources
            .iter()
            .find(|s| s.id == out.anchor)
            .map(|s| s.grain.clone())
            .unwrap_or_default();
        let mut stages = vec![];
        for (name, predicate) in [("eligibility", &out.eligibility), ("filter", &out.filter)] {
            if let Some(p) = predicate {
                let mut node = expression_node(
                    format!("{base}/{name}"),
                    "population_stage",
                    p,
                    r,
                    &mut unsupported,
                );
                node["retains"] =
                    json!("rows where the predicate is true; false and unknown are excluded");
                stages.push(node["id"].clone());
                nodes.push(node);
            }
        }
        let mut columns = vec![];
        for c in &out.columns {
            let node = expression_node(
                format!("{base}/column:{}", c.name),
                "column",
                &c.expr,
                r,
                &mut unsupported,
            );
            columns.push(node["id"].clone());
            nodes.push(node);
        }
        let mut transitions = vec![];
        for a in &out.aggregates {
            let mut node = expression_node(
                format!("{base}/aggregate:{}", a.name),
                "grouped_aggregate",
                &a.expr,
                r,
                &mut unsupported,
            );
            node["function"] = json!(a.function);
            if !AGGREGATES.contains(&a.function.as_str()) {
                let item = json!({"node":node["id"],"code":"UNSUPPORTED_OPERATION","operation":a.function,"reason":"aggregate function is not supported in format 0.1"});
                node["unsupported"]
                    .as_array_mut()
                    .unwrap()
                    .push(item.clone());
                unsupported.push(item);
            }
            columns.push(node["id"].clone());
            nodes.push(node);
        }
        if !out.aggregates.is_empty() {
            transitions.push(json!({"from":anchor_grain,"to":out.group_by,"operation":"group_by"}));
        }
        nodes.push(json!({
            "id": base,
            "kind": "output",
            "anchor": format!("source:{}", out.anchor),
            "anchor_grain": anchor_grain,
            "grain": if out.aggregates.is_empty() { json!(anchor_grain) } else { json!(out.group_by) },
            "grain_transitions": transitions,
            "population_stages": stages,
            "columns": columns,
            "detail": out.detail,
            "output_order": "unspecified",
        }));
    }
    json!({
        "recipe": {"name": r.name, "format_version": r.format_version, "hash": r.hash().ok()},
        "validation": validate(r),
        "placement": "local",
        "estimates": {"rows": null, "bytes": null, "status": "unknown"},
        "data_checks": "not-run",
        "output_order": "unspecified",
        "nodes": nodes,
        "unsupported": unsupported,
    })
}

// ---------------------------------------------------------------------------
// Portable document identity, persistence and bundling.

impl Recipe {
    /// Bundle one of this recipe's own declarations with its content hash.
    pub fn bundle_definition(&mut self, id: &str, provenance: Option<String>) -> Result<()> {
        let content = declaration(self, id).ok_or_else(|| error("MISSING_REFERENCE", id))?;
        self.bundled_definitions.insert(
            id.to_string(),
            BundledDefinition {
                content_hash: content_hash(&content),
                content,
                provenance,
            },
        );
        Ok(())
    }

    /// Copy a catalog declaration into this recipe and bundle it, so the saved
    /// document is self-contained. A name already used by different content
    /// requires an explicit `alias`; identical content is idempotent.
    pub fn bundle_from(&mut self, catalog: &Catalog, id: &str, alias: Option<&str>) -> Result<()> {
        let name = alias.unwrap_or(id);
        if name.is_empty() {
            return Err(error("DUPLICATE_ID", "alias must be nonempty"));
        }
        enum Item {
            Source(Source),
            Relationship(Relationship),
            Feature(Feature),
        }
        let item = if let Some(s) = catalog.sources.iter().find(|s| s.id == id) {
            let mut s = s.clone();
            s.id = name.into();
            Item::Source(s)
        } else if let Some(x) = catalog.relationships.iter().find(|x| x.id == id) {
            let mut x = x.clone();
            x.id = name.into();
            Item::Relationship(x)
        } else if let Some(f) = catalog.features.iter().find(|f| f.id == id) {
            let mut f = f.clone();
            f.id = name.into();
            Item::Feature(f)
        } else {
            return Err(error(
                "MISSING_REFERENCE",
                format!("catalog {} has no declaration {id}", catalog.name),
            ));
        };
        let content = match &item {
            Item::Source(x) => serde_json::to_value(x),
            Item::Relationship(x) => serde_json::to_value(x),
            Item::Feature(x) => serde_json::to_value(x),
        }
        .map_err(|e| error("INVALID_RECIPE", e.to_string()))?;
        let taken = declaration(self, name).is_some() || self.outputs.iter().any(|o| o.id == name);
        if taken {
            if declaration(self, name).as_ref() == Some(&content) {
                return Ok(());
            }
            return Err(error(
                "NAME_CONFLICT",
                format!(
                    "{name} is already declared with different content; bundle it under an explicit alias"
                ),
            ));
        }
        let provenance = format!(
            "catalog {}{} declaration {id}",
            catalog.name,
            catalog
                .revision
                .as_ref()
                .map(|r| format!(" revision {r}"))
                .unwrap_or_default()
        );
        no_machine_paths(&json!(provenance), "catalog provenance")?;
        match item {
            Item::Source(x) => self.sources.push(x),
            Item::Relationship(x) => self.relationships.push(x),
            Item::Feature(x) => self.features.push(x),
        }
        self.bundled_definitions.insert(
            name.to_string(),
            BundledDefinition {
                content_hash: content_hash(&content),
                content,
                provenance: Some(provenance),
            },
        );
        Ok(())
    }

    /// Load a `.iresiphi` document: exact UTF-8 JSON, no duplicate object keys,
    /// no unknown properties, and complete static validation before any binding.
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str::<UniqueKeys>(text).map_err(|e| {
            let message = e.to_string();
            if message.contains(DUPLICATE_KEY) {
                error("DUPLICATE_KEY", message)
            } else {
                error("INVALID_RECIPE", message)
            }
        })?;
        let value: Json =
            serde_json::from_str(text).map_err(|e| error("INVALID_RECIPE", e.to_string()))?;
        deferred(&value, "")?;
        let r: Self =
            serde_json::from_str(text).map_err(|e| error("INVALID_RECIPE", e.to_string()))?;
        static_checks(&r)?;
        Ok(r)
    }
    pub fn to_json(&self) -> Result<String> {
        static_checks(self)?;
        serde_json::to_string(self).map_err(|e| error("INVALID_RECIPE", e.to_string()))
    }
    /// Canonical semantic content: annotations removed, ID-keyed declaration
    /// lists ordered by ID (their order is not significant), capabilities as a
    /// sorted set, and numeric literals in canonical tagged text. All other
    /// arrays keep their order.
    pub fn canonical_json(&self) -> Result<String> {
        static_checks(self)?;
        let mut value =
            serde_json::to_value(self).map_err(|e| error("INVALID_RECIPE", e.to_string()))?;
        let root = value.as_object_mut().expect("recipe is an object");
        root.remove("annotations");
        for key in ["sources", "relationships", "features", "outputs"] {
            if let Some(Json::Array(items)) = root.get_mut(key) {
                items.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
            }
        }
        if let Some(Json::Array(items)) = root.get_mut("required_capabilities") {
            items.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            items.dedup();
        }
        normalize_literals(&mut value);
        Ok(canonical_json(&value))
    }
    /// SHA-256 (lowercase hex) of [`Recipe::canonical_json`].
    pub fn hash(&self) -> Result<String> {
        Ok(sha256_hex(&self.canonical_json()?))
    }
}

/// Lowercase hexadecimal encoding of a digest.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Bound validation and explain.

fn bound_feasibility(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    inspect: bool,
) -> Result<()> {
    if policy.allowed_read_roots.is_empty() {
        return Err(error("POLICY_DENIED", "no allowed read roots"));
    }
    for source in &recipe.sources {
        let binding = bindings
            .get(&source.id)
            .ok_or_else(|| error("MISSING_BINDING", &source.id))?;
        if !["json", "csv", "xlsx"].contains(&binding.adapter.as_str()) {
            return Err(error("UNSUPPORTED_ADAPTER", &binding.adapter));
        }
        if !binding.path.is_absolute()
            || binding
                .path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(error(
                "POLICY_DENIED",
                "bound source path must be absolute and contain no parent traversal",
            ));
        }
        policy
            .allowed_read_roots
            .iter()
            .find(|root| root.is_absolute() && binding.path.starts_with(root))
            .ok_or_else(|| error("POLICY_DENIED", "source outside allowed read roots"))?;
        if inspect {
            let mut current = std::path::PathBuf::new();
            for part in binding.path.components() {
                current.push(part.as_os_str());
                let metadata = std::fs::symlink_metadata(&current)
                    .map_err(|e| error("SOURCE_ACCESS", e.to_string()))?;
                if metadata.file_type().is_symlink() {
                    return Err(error("POLICY_DENIED", "source path contains symlink"));
                }
            }
            let metadata = std::fs::metadata(&binding.path)
                .map_err(|e| error("SOURCE_ACCESS", e.to_string()))?;
            if !metadata.is_file() {
                return Err(error("SOURCE_ACCESS", "source must be a file"));
            }
            if metadata.len() > policy.limits.file_bytes as u64 {
                return Err(error(
                    "RESOURCE_LIMIT",
                    "encoded source exceeds file budget",
                ));
            }
        }
    }
    Ok(())
}
/// Data-check categories reported by full bound validation, with the run
/// diagnostic codes that establish a failure in each category.
const DATA_CHECKS: [(&str, &[&str]); 4] = [
    (
        "data.types",
        &[
            "INVALID_VALUE",
            "INVALID_DECIMAL",
            "INVALID_DATE",
            "TYPE_MISMATCH",
            "NULL_REQUIRED",
            "UNKNOWN_PATH",
            "INVALID_JSON",
            "DUPLICATE_JSON_KEY",
            "INVALID_CSV",
            "INVALID_XLSX",
            "XLSX_QUANTIZATION_REQUIRED",
            "XLSX_CELL_ERROR",
            "FORMULA_CACHE_MISSING",
        ],
    ),
    ("data.identity", &["NULL_IDENTITY", "DUPLICATE_IDENTITY"]),
    ("data.relationships", &["CARDINALITY_VIOLATION"]),
    ("data.rate_validity", &["INVALID_INTERVAL"]),
];
/// `validate(recipe, bindings, policy)`. With `data_checks = "none"` (the
/// default) it adds binding/access feasibility without reading file contents;
/// `"full"` scans every selected source population. There is no sampled mode.
pub fn validate_bound(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    data_checks: &str,
) -> Report {
    let mut report = validate(recipe);
    if !report.valid {
        return report;
    }
    if !["none", "full"].contains(&data_checks) {
        report.valid = false;
        report.checks.push(Check {
            id: "data_mode".into(),
            status: "failed".into(),
            coverage: "requested mode".into(),
            diagnostic: Some(error(
                "UNSUPPORTED_OPERATION",
                "data_checks accepts none or full; there is no sampled mode",
            )),
        });
        return report;
    }
    let feasibility = bound_feasibility(recipe, bindings, policy, true);
    report.valid = feasibility.is_ok();
    report.checks.push(Check {
        id: "access".into(),
        status: if feasibility.is_ok() {
            "passed"
        } else {
            "failed"
        }
        .into(),
        coverage: "declared bindings and local file metadata; file contents not scanned".into(),
        diagnostic: feasibility.err(),
    });
    if !report.valid {
        return report;
    }
    if data_checks == "full" {
        let run = crate::engine::ExecutionContext::default().run(
            recipe,
            bindings,
            policy,
            &std::sync::atomic::AtomicBool::new(false),
        );
        let passed = run.status == "succeeded" || run.status == "succeeded_with_unresolved";
        let diagnostic = run.diagnostics.into_iter().next();
        let rows = &run.manifest["source_rows"];
        report.valid = passed;
        report.checks.retain(|c| c.id != "data");
        report.checks.push(Check {
            id: "data".into(),
            status: if passed { "passed" } else { "failed" }.into(),
            coverage: if passed {
                format!("complete selected materialization and required execution checks; source rows {rows}")
            } else {
                "incomplete; stopped at first failure".into()
            },
            diagnostic: diagnostic.clone(),
        });
        let failed_category = diagnostic.as_ref().and_then(|d| {
            DATA_CHECKS
                .iter()
                .find(|(_, codes)| codes.contains(&d.code.as_str()))
                .map(|(id, _)| *id)
        });
        for (id, _) in DATA_CHECKS {
            let (status, coverage, diagnostic) = if passed {
                (
                    "passed",
                    format!("full: every row of every selected source; source rows {rows}"),
                    None,
                )
            } else if failed_category == Some(id) {
                (
                    "failed",
                    "incomplete; run stopped at the first failure".into(),
                    diagnostic.clone(),
                )
            } else {
                (
                    "not-run",
                    "incomplete; run stopped before this check completed".into(),
                    None,
                )
            };
            report.checks.push(Check {
                id: id.into(),
                status: status.into(),
                coverage,
                diagnostic,
            });
        }
    } else {
        report.checks.push(Check {
            id: "source_schema".into(),
            status: "not-run".into(),
            coverage:
                "none; source schema probing without materialization is not available in this build"
                    .into(),
            diagnostic: None,
        });
    }
    report
}
/// Describe bound placement without opening source files or performing data checks.
pub fn explain_bound(recipe: &Recipe, bindings: &Bindings, policy: &Policy) -> Json {
    let mut explanation = explain(recipe);
    let feasibility = bound_feasibility(recipe, bindings, policy, false);
    explanation["bound"] = json!({
        "feasible": feasibility.is_ok(),
        "diagnostic": feasibility.err(),
        "budgets": policy.limits,
        "reads": recipe.sources.iter().map(|source| json!({
            "source": format!("source:{}", source.id),
            "adapter": bindings.get(&source.id).map(|b| &b.adapter),
            "location": "local",
            "rows": null,
            "bytes": null,
            "estimate_status": "unknown",
        })).collect::<Vec<_>>(),
        "transfers": [],
        "source_access": "not-run",
        "data_checks": "not-run",
    });
    explanation
}
