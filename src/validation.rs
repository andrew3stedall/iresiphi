use crate::model::*;
use serde::Serialize;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

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
            let d = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|_| error("INVALID_VALUE", "invalid calendar date"))?;
            if d.format("%Y-%m-%d").to_string() != *value || value.len() != 10 {
                return Err(error("INVALID_VALUE", "date must be YYYY-MM-DD"));
            }
            Some(Type::Date)
        }
    })
}
fn check_type(t: &Type) -> Result<()> {
    if let Type::Decimal { precision, scale } = t {
        if *precision == 0 || *precision > 38 || scale > precision {
            return Err(error(
                "INVALID_TYPE",
                "decimal precision/scale out of bounds",
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
        let rel = r
            .relationships
            .iter()
            .find(|x| x.id == part)
            .ok_or_else(|| error("MISSING_REFERENCE", part))?;
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
                let rel = r
                    .relationships
                    .iter()
                    .find(|x| x.id == *id)
                    .ok_or_else(|| error("MISSING_REFERENCE", format!("relationship {id}")))?;
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
                .ok_or_else(|| error("MISSING_REFERENCE", id))?;
            let result = infer(&feature.expr, r, visiting);
            visiting.remove(id);
            result
        }
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
                        ((p - s).max(q - t) + s.max(t) + 1, s.max(t))
                    };
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
            relationship,
            function,
            field: f,
            filter,
        } => {
            let rel = r
                .relationships
                .iter()
                .find(|x| x.id == *relationship)
                .ok_or_else(|| error("MISSING_REFERENCE", relationship))?;
            if let Some(p) = filter {
                boolean(infer(p, r, visiting)?)?;
            }
            match function.as_str() {
                "count_records" => Ok(Some(Type::Int64)),
                "count_non_null" => {
                    field(
                        r,
                        &rel.to,
                        f.as_deref()
                            .ok_or_else(|| error("MISSING_REFERENCE", "aggregate field"))?,
                    )?;
                    Ok(Some(Type::Int64))
                }
                "sum" | "min" | "max" => {
                    let t = field(
                        r,
                        &rel.to,
                        f.as_deref()
                            .ok_or_else(|| error("MISSING_REFERENCE", "aggregate field"))?,
                    )?
                    .data_type
                    .clone();
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
                _ => Err(error("UNSUPPORTED_OPERATION", function)),
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
                let rel = r
                    .relationships
                    .iter()
                    .find(|x| x.id == *first)
                    .ok_or_else(|| error("MISSING_REFERENCE", first))?;
                if rel.from != anchor {
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
        Expr::Aggregate { relationship, .. } => {
            let rel = r
                .relationships
                .iter()
                .find(|x| x.id == *relationship)
                .ok_or_else(|| error("MISSING_REFERENCE", relationship))?;
            if rel.from != anchor {
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
        | Equal { left, right }
        | Less { left, right }
        | LessEqual { left, right }
        | And { left, right }
        | Or { left, right } => vec![left, right],
        Not { expr } | IsNull { expr } | Cast { expr, .. } | Quantize { expr, .. } => vec![expr],
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
fn canonical(value: &Json) -> Json {
    match value {
        Json::Object(map) => {
            let ordered: std::collections::BTreeMap<_, _> =
                map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect();
            serde_json::to_value(ordered).expect("JSON values serialize")
        }
        Json::Array(values) => Json::Array(values.iter().map(canonical).collect()),
        other => other.clone(),
    }
}
fn content_hash(value: &Json) -> String {
    hex(&Sha256::digest(
        serde_json::to_vec(&canonical(value)).expect("JSON values serialize"),
    ))
}
fn static_checks(r: &Recipe) -> Result<()> {
    if r.format_version != "0.1" {
        return Err(error("UNSUPPORTED_VERSION", &r.format_version));
    }
    for (id, definition) in &r.bundled_definitions {
        if definition.content_hash != content_hash(&definition.content) {
            return Err(error("CONTENT_HASH_MISMATCH", id));
        }
        let declared = r
            .sources
            .iter()
            .find(|s| s.id == *id)
            .and_then(|s| serde_json::to_value(s).ok())
            .or_else(|| {
                r.features
                    .iter()
                    .find(|f| f.id == *id)
                    .and_then(|f| serde_json::to_value(f).ok())
            })
            .or_else(|| {
                r.relationships
                    .iter()
                    .find(|x| x.id == *id)
                    .and_then(|x| serde_json::to_value(x).ok())
            });
        if declared.as_ref() != Some(&definition.content) {
            return Err(error("BUNDLED_DEFINITION_MISMATCH", id));
        }
    }
    let supported = [
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
        if !supported.contains(&cap.as_str()) {
            return Err(error("UNSUPPORTED_CAPABILITY", cap));
        }
    }
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
            return Err(error("DUPLICATE_ID", id));
        }
    }
    for s in &r.sources {
        if s.identity.is_empty() || s.grain.is_empty() {
            return Err(error(
                "INVALID_IDENTITY",
                "identity and grain must be explicit",
            ));
        }
        for key in s.identity.iter().chain(&s.grain) {
            field(r, &s.id, key)?;
        }
        for key in &s.identity {
            if field(r, &s.id, key)?.nullable {
                return Err(error("INVALID_IDENTITY", "identity cannot be nullable"));
            }
        }
        for f in s.projection.fields.values() {
            check_type(&f.data_type)?;
        }
    }
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
                let dependency = r
                    .relationships
                    .iter()
                    .find(|x| x.id == *id)
                    .ok_or_else(|| error("MISSING_REFERENCE", *id))?;
                visit_relationship(dependency, r, active, done)?;
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
    for f in &r.features {
        expression_type(&f.expr, r)?;
    }
    for out in &r.outputs {
        source(r, &out.anchor)?;
        let mut names = BTreeSet::new();
        for c in &out.columns {
            if !names.insert(&c.name) {
                return Err(error("DUPLICATE_ID", &c.name));
            }
            expression_type(&c.expr, r)?;
            check_output_paths(&c.expr, r, &out.anchor, &mut BTreeSet::new())?;
        }
        for p in [&out.eligibility, &out.filter].into_iter().flatten() {
            boolean(expression_type(p, r)?)?;
            check_output_paths(p, r, &out.anchor, &mut BTreeSet::new())?;
        }
        for g in &out.group_by {
            if !out.columns.iter().any(|c| c.name == *g) {
                return Err(error("MISSING_REFERENCE", format!("group column {g}")));
            }
        }
        for aggregate in &out.aggregates {
            if !names.insert(&aggregate.name) {
                return Err(error("DUPLICATE_ID", &aggregate.name));
            }
            let ty = expression_type(&aggregate.expr, r)?;
            check_output_paths(&aggregate.expr, r, &out.anchor, &mut BTreeSet::new())?;
            match aggregate.function.as_str() {
                "count_records" | "count_non_null" | "min" | "max" => {}
                "sum" => {
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
                other => return Err(error("UNSUPPORTED_OPERATION", other)),
            }
        }
        if !out.aggregates.is_empty() && source(r,&out.anchor)?.projection.fields.contains_key("currency") && !out.group_by.iter().any(|g|out.columns.iter().any(|c|c.name==*g&&matches!(&c.expr,Expr::Field{source,field,path}if source==&out.anchor&&field=="currency"&&path.is_empty()))){return Err(error("GRAIN_VIOLATION","grouping must retain currency"));}
    }
    Ok(())
}
pub fn validate(r: &Recipe) -> Report {
    let result = static_checks(r);
    Report {
        valid: result.is_ok(),
        checks: vec![
            Check {
                id: "static".into(),
                status: if result.is_ok() { "passed" } else { "failed" }.into(),
                coverage: "entire recipe; no source data read".into(),
                diagnostic: result.err(),
            },
            Check {
                id: "data".into(),
                status: "not-run".into(),
                coverage: "none".into(),
                diagnostic: None,
            },
        ],
    }
}
pub fn explain(r: &Recipe) -> Json {
    json!({"validation":validate(r),"placement":"local","estimates":{"rows":null,"bytes":null},"nodes":{"sources":r.sources,"relationships":r.relationships,"features":r.features,"outputs":r.outputs},"data_checks":"not-run","output_order":"unspecified"})
}
impl Recipe {
    pub fn bundle_definition(&mut self, id: &str, provenance: Option<String>) -> Result<()> {
        let content = self
            .sources
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| serde_json::to_value(s).ok())
            .or_else(|| {
                self.features
                    .iter()
                    .find(|f| f.id == id)
                    .and_then(|f| serde_json::to_value(f).ok())
            })
            .or_else(|| {
                self.relationships
                    .iter()
                    .find(|x| x.id == id)
                    .and_then(|x| serde_json::to_value(x).ok())
            })
            .ok_or_else(|| error("MISSING_REFERENCE", id))?;
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

    pub fn from_json(text: &str) -> Result<Self> {
        let r: Self =
            serde_json::from_str(text).map_err(|e| error("INVALID_RECIPE", e.to_string()))?;
        static_checks(&r)?;
        Ok(r)
    }
    pub fn to_json(&self) -> Result<String> {
        static_checks(self)?;
        serde_json::to_string(self).map_err(|e| error("INVALID_RECIPE", e.to_string()))
    }
    pub fn hash(&self) -> Result<String> {
        static_checks(self)?;
        let mut value =
            serde_json::to_value(self).map_err(|e| error("INVALID_RECIPE", e.to_string()))?;
        value.as_object_mut().unwrap().remove("annotations");
        Ok(content_hash(&value))
    }
}

/// Lowercase hexadecimal encoding of a digest.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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
        let root = policy
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
            let _ = root;
        }
    }
    Ok(())
}
/// Bound validation defaults to feasibility checks when `data_checks` is `none`.
/// The report explicitly distinguishes those checks from full materialization.
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
                "data_checks accepts none or full",
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
        report.valid = passed;
        report.checks.retain(|c| c.id != "data");
        report.checks.push(Check {
            id: "data".into(),
            status: if passed { "passed" } else { "failed" }.into(),
            coverage: if passed {
                "complete selected materialization and required execution checks"
            } else {
                "incomplete; stopped at first failure"
            }
            .into(),
            diagnostic: run.diagnostics.into_iter().next(),
        });
    } else {
        report.checks.push(Check {
            id: "source_schema".into(),
            status: "not-run".into(),
            coverage: "none; full adapter materialization required".into(),
            diagnostic: None,
        });
    }
    report
}
/// Describe bound placement without opening source files or performing data checks.
pub fn explain_bound(recipe: &Recipe, bindings: &Bindings, policy: &Policy) -> Json {
    let mut explanation = explain(recipe);
    let feasibility = bound_feasibility(recipe, bindings, policy, false);
    explanation["bound"] = json!({"feasible":feasibility.is_ok(),"diagnostic":feasibility.err(),"budgets":policy.limits,"reads":recipe.sources.iter().map(|source|json!({"source":source.id,"adapter":bindings.get(&source.id).map(|b|&b.adapter),"location":"local","rows":null,"bytes":null})).collect::<Vec<_>>(),"transfers":[],"source_access":"not-run"});
    explanation
}
