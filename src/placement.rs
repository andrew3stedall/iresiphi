//! Source-side placement (docs/spec/slice-3.md, "Contract: placement").
//!
//! Source operations include an output's anchor eligibility predicate,
//! as a filter on the anchor source's scan. This module decides, from the recipe,
//! the bindings and the policy alone, whether a predicate may be pushed. It opens
//! nothing. The decision is negotiated against the exact predicate, so a source
//! that can filter still keeps a predicate it cannot evaluate with the recipe's
//! semantics in the local executor.
use crate::model::*;
use serde_json::json;
use std::collections::BTreeMap;

/// A result whose error is the reason a predicate stays in the local executor.
type Reasoned<T> = std::result::Result<T, String>;

/// A literal bound into a pushed predicate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SqlValue {
    Integer(i64),
    Text(String),
}

/// An eligibility predicate that runs at its anchor source.
#[derive(Debug, Clone)]
pub struct Pushed {
    /// Stage ID of the predicate, as explain names it.
    pub stage: String,
    /// SQL condition over the source's columns, with `?` placeholders.
    pub sql: String,
    pub parameters: Vec<SqlValue>,
}

/// The placement of every output eligibility under one policy.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Pushed predicates by source ID.
    pub pushed: BTreeMap<String, Pushed>,
    /// One record per output eligibility, for explain and the manifest.
    pub decisions: Vec<serde_json::Value>,
    pub reductions: BTreeMap<String, String>,
    pub candidates: Vec<(String, String)>,
    /// Key transfers by target source ID (docs/spec/slice-5.md). The target's
    /// condition is built at run time from the delivered from records.
    pub transfers: BTreeMap<String, Transfer>,
    /// One record per planned transfer, for explain.
    pub transfer_records: Vec<serde_json::Value>,
}

/// A many-to-one lookup whose SQLite target is reduced to keys the executor
/// transfers from the delivered records of a source elsewhere.
#[derive(Debug, Clone)]
pub struct Transfer {
    /// Stage ID of the reduced target scan: `relationship:<id>/target`.
    pub stage: String,
    /// Transfer ID: `relationship:<id>/keys`.
    pub id: String,
    pub from: String,
    pub from_keys: Vec<String>,
    /// Target key operands, stripped of affinity and collation.
    pub target_keys: Vec<String>,
}

/// SQLite's default bound-parameter ceiling; a transfer never exceeds it
/// whatever `limits.transfer_keys` says.
pub const TRANSFER_PARAMETERS: usize = 32_766;

impl Transfer {
    /// Distinct keys of `rows` with no null component, in key order.
    pub(crate) fn keys(&self, rows: &[Row]) -> Result<std::collections::BTreeSet<Vec<SqlValue>>> {
        let mut keys = std::collections::BTreeSet::new();
        'rows: for row in rows {
            let mut key = Vec::with_capacity(self.from_keys.len());
            for field in &self.from_keys {
                key.push(match row.fields.get(field) {
                    Some(Value::Int64 { value }) => {
                        SqlValue::Integer(value.parse().map_err(|_| {
                            Error::new("INVALID_VALUE", "source", "an int64 key is out of range")
                        })?)
                    }
                    Some(Value::String { value } | Value::Date { value }) => {
                        SqlValue::Text(value.clone())
                    }
                    Some(Value::Boolean { value }) => SqlValue::Integer((*value).into()),
                    // Null keys match nothing. Decimal keys are never planned.
                    _ => continue 'rows,
                });
            }
            keys.insert(key);
        }
        Ok(keys)
    }

    /// The target condition selecting exactly the records whose key is in `keys`.
    pub(crate) fn pushed(&self, keys: std::collections::BTreeSet<Vec<SqlValue>>) -> Pushed {
        let sql = if keys.is_empty() {
            "0".to_string()
        } else if self.target_keys.len() == 1 {
            format!(
                "({} IN ({}))",
                self.target_keys[0],
                vec!["?"; keys.len()].join(", ")
            )
        } else {
            let tuple = format!("({})", vec!["?"; self.target_keys.len()].join(", "));
            format!(
                "(({}) IN (VALUES {}))",
                self.target_keys.join(", "),
                vec![tuple; keys.len()].join(", ")
            )
        };
        Pushed {
            stage: self.stage.clone(),
            sql,
            parameters: keys.into_iter().flatten().collect(),
        }
    }
}

/// True when `adapter` can evaluate a negotiated filter during its scan.
pub(crate) fn filter_pushdown(adapter: &str) -> bool {
    adapter == "sqlite"
}

/// Quote an SQL identifier.
pub(crate) fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The column a declared field reads, as the SQLite reader resolves it.
pub(crate) fn column(field: &Field) -> &str {
    &field.path
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Text,
    Boolean,
}

struct Translator<'a> {
    source: &'a Source,
    parameters: Vec<SqlValue>,
}

impl Translator<'_> {
    /// A comparable operand: an anchor field or a literal.
    fn operand(&mut self, expr: &Expr) -> Reasoned<(String, Kind)> {
        match expr {
            Expr::Field {
                source,
                field,
                path,
            } => {
                if *source != self.source.id || !path.is_empty() {
                    return Err(format!(
                        "the predicate reads {source}.{field} through a relationship; only fields of the anchor source can be evaluated at the source"
                    ));
                }
                let declared = self
                    .source
                    .projection
                    .fields
                    .get(field)
                    .ok_or_else(|| format!("unknown field {field}"))?;
                let kind = kind(&declared.data_type)?;
                // Unary plus removes the column's declared affinity and COLLATE
                // BINARY its declared collation, so SQLite compares the stored
                // values themselves: integers numerically and text bytewise.
                Ok((
                    format!("+{} COLLATE BINARY", quoted(column(declared))),
                    kind,
                ))
            }
            Expr::Literal { value } => {
                let (parameter, kind) = match value {
                    Value::Int64 { value } => (
                        SqlValue::Integer(
                            value
                                .parse()
                                .map_err(|_| "an int64 literal is out of range".to_string())?,
                        ),
                        Kind::Integer,
                    ),
                    Value::String { value } | Value::Date { value } => {
                        (SqlValue::Text(value.clone()), Kind::Text)
                    }
                    Value::Boolean { value } => (SqlValue::Integer((*value).into()), Kind::Boolean),
                    Value::Decimal { .. } => return Err(decimal_reason()),
                    Value::Null => {
                        return Err("the predicate compares with a null literal".into());
                    }
                };
                if self.parameters.len() >= 500 {
                    return Err("the predicate exceeds 500 bound parameters".into());
                }
                self.parameters.push(parameter);
                Ok(("?".into(), kind))
            }
            _ => Err(
                "the predicate compares a computed value; only fields and literals can be compared at the source"
                    .into(),
            ),
        }
    }

    /// A condition whose SQL truth value is the predicate's: true, false or unknown.
    fn condition(&mut self, expr: &Expr, depth: usize) -> Reasoned<String> {
        if depth > crate::validation::EXPRESSION_DEPTH {
            return Err("the predicate is nested too deeply to push".into());
        }
        match expr {
            Expr::Equal { left, right }
            | Expr::Less { left, right }
            | Expr::LessEqual { left, right } => {
                let (a, left_kind) = self.operand(left)?;
                let (b, right_kind) = self.operand(right)?;
                if left_kind != right_kind {
                    return Err("the predicate compares values of different types".into());
                }
                let op = match expr {
                    Expr::Equal { .. } => "=",
                    Expr::Less { .. } => "<",
                    _ => "<=",
                };
                Ok(format!("({a} {op} {b})"))
            }
            Expr::And { left, right } | Expr::Or { left, right } => {
                let a = self.condition(left, depth + 1)?;
                let b = self.condition(right, depth + 1)?;
                let op = if matches!(expr, Expr::And { .. }) {
                    "AND"
                } else {
                    "OR"
                };
                Ok(format!("({a} {op} {b})"))
            }
            Expr::Not { expr } => Ok(format!("(NOT {})", self.condition(expr, depth + 1)?)),
            Expr::IsNull { expr } => match expr.as_ref() {
                Expr::Field { .. } => {
                    let (operand, _) = self.operand(expr)?;
                    Ok(format!("({operand} IS NULL)"))
                }
                inner => Ok(format!("({} IS NULL)", self.condition(inner, depth + 1)?)),
            },
            // A Boolean field or literal used as a truth value.
            Expr::Field { .. } | Expr::Literal { .. } => {
                let (operand, kind) = self.operand(expr)?;
                if kind != Kind::Boolean {
                    return Err("the predicate uses a value that is not Boolean as a condition".into());
                }
                Ok(format!("({operand} = 1)"))
            }
            Expr::Feature { .. } => Err(
                "the predicate reads a named feature; only fields and literals can be evaluated at the source"
                    .into(),
            ),
            Expr::Aggregate { .. } => {
                Err("the predicate reads an aggregate, which runs in the local executor".into())
            }
            _ => Err(
                "the predicate uses an operation the source cannot evaluate with the recipe's semantics"
                    .into(),
            ),
        }
    }
}

fn decimal_reason() -> String {
    "the predicate compares a decimal; sqlite has no exact decimal comparison, so the comparison stays in the local executor".into()
}

fn kind(data_type: &Type) -> Reasoned<Kind> {
    match data_type {
        Type::Int64 => Ok(Kind::Integer),
        Type::String | Type::Date => Ok(Kind::Text),
        Type::Boolean => Ok(Kind::Boolean),
        Type::Decimal { .. } => Err(decimal_reason()),
    }
}

/// Another use of `source` that needs the rows a pushed filter would drop.
fn other_use(recipe: &Recipe, source: &str, output: &str) -> Option<String> {
    if let Some(o) = recipe
        .outputs
        .iter()
        .find(|o| o.anchor == source && o.id != output)
    {
        return Some(format!("output {} also anchors at it", o.id));
    }
    if let Some(r) = recipe.relationships.iter().find(|r| r.to == source) {
        return Some(format!("relationship {} reads it as a target", r.id));
    }
    if let Some(r) = recipe
        .relationships
        .iter()
        .find(|r| r.from == source && r.cardinality != Cardinality::ManyToOne)
    {
        return Some(format!(
            "relationship {} needs every anchor record for cross-anchor ambiguity or detail parent checks",
            r.id
        ));
    }
    // A detail output reads its parent back through the parent's relationship.
    for r in recipe.relationships.iter().filter(|r| r.from == source) {
        if let Some(o) = recipe
            .outputs
            .iter()
            .find(|o| o.anchor == r.to && o.id != output)
        {
            return Some(format!(
                "output {} reads it through relationship {}",
                o.id, r.id
            ));
        }
    }
    for p in &recipe.populations {
        if p.recipients == source {
            return Some(format!("population {} takes its recipients from it", p.id));
        }
        if matches!(&p.period, PeriodSet { from: Some(from), .. } if from.source == source) {
            return Some(format!("population {} takes its periods from it", p.id));
        }
    }
    recipe
        .outputs
        .iter()
        .flat_map(|o| o.allocations.iter().map(move |a| (o, a)))
        .find(|(_, a)| a.quantity.source == source)
        .map(|(o, a)| format!("allocation {}.{} takes its quantity from it", o.id, a.name))
}

/// Decide the placement of every output eligibility. Fails with
/// `PLACEMENT_INFEASIBLE` when the policy requires source placement of a
/// predicate that cannot run at its source; the policy is never overridden.
pub fn plan(recipe: &Recipe, bindings: &Bindings, policy: &Policy) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut eligibility_error = None;
    for output in &recipe.outputs {
        let Some(predicate) = &output.eligibility else {
            continue;
        };
        let stage = format!("output:{}/eligibility", output.id);
        let source = recipe.sources.iter().find(|s| s.id == output.anchor);
        let adapter = bindings.get(&output.anchor).map(|b| b.adapter.as_str());
        let capable = adapter.is_some_and(filter_pushdown);
        let mut decision = json!({
            "operation": stage,
            "source": format!("source:{}", output.anchor),
            "policy": policy.placement,
        });
        let outcome: std::result::Result<Pushed, String> = match (source, capable) {
            _ if policy.placement == Placement::Local => {
                Err("the policy places every operation in the local executor".into())
            }
            (Some(source), true) => match other_use(recipe, &source.id, &output.id) {
                Some(used) => Err(format!(
                    "source {} has another use that needs every record: {used}",
                    source.id
                )),
                None if source.identity.iter().any(|id| {
                    matches!(
                        source.projection.fields.get(id).map(|f| &f.data_type),
                        Some(Type::Decimal { .. })
                    )
                }) =>
                {
                    Err(format!(
                        "source {} has a decimal identity, whose uniqueness sqlite cannot check exactly",
                        source.id
                    ))
                }
                None => {
                    let mut translator = Translator {
                        source,
                        parameters: vec![],
                    };
                    translator.condition(predicate, 0).map(|sql| Pushed {
                        stage: stage.clone(),
                        sql,
                        parameters: translator.parameters,
                    })
                }
            },
            (Some(_), false) => Err(format!(
                "the {} adapter reads whole sources and declares no filter pushdown",
                adapter.unwrap_or("bound")
            )),
            (None, _) => Err(
                "the output anchors at a population, which is built in the local executor".into(),
            ),
        };
        match outcome {
            Ok(pushed) => {
                decision["placement"] = json!("source");
                decision["coverage"] = json!(
                    "identity, nullability, field_bytes and storage classes are checked at the source for every record; text checks (date text, decimal text, and valid UTF-8) and lookups are checked for scanned records only"
                );
                plan.pushed.insert(output.anchor.clone(), pushed);
            }
            Err(reason) => {
                // Requiring source placement only binds sources that can filter.
                if policy.placement == Placement::RequireSource && capable {
                    eligibility_error = Some(Error::new(
                        "PLACEMENT_INFEASIBLE",
                        "planning",
                        format!(
                            "policy placement require_source cannot be met for {stage}: {reason}"
                        ),
                    )
                    .with_subject(stage.clone())
                    .with_outputs(vec![output.id.clone()])
                    .with_remediation(
                        "Use placement prefer_source to run this predicate in the local executor, or local to place every operation there; the policy is not overridden.",
                    ));
                }
                decision["placement"] = json!("local");
                decision["reason"] = json!(reason);
            }
        }
        plan.decisions.push(decision);
    }
    for r in &recipe.relationships {
        let (Some(a), Some(b)) = (bindings.get(&r.from), bindings.get(&r.to)) else {
            continue;
        };
        if r.from != r.to && b.adapter == "sqlite" && !colocated(a, b) {
            plan_transfer(&mut plan, recipe, r, policy)?;
            continue;
        }
        if r.from == r.to || !colocated(a, b) {
            continue;
        }
        plan.candidates.push((r.from.clone(), r.to.clone()));
        let stage = format!("relationship:{}/target", r.id);
        let (Some(from), Some(to)) = (
            recipe.sources.iter().find(|s| s.id == r.from),
            recipe.sources.iter().find(|s| s.id == r.to),
        ) else {
            return Err(Error::new(
                "INVALID_RELATIONSHIP",
                "planning",
                "reduction endpoints must be declared sources",
            ));
        };
        if r.to_keys
            .iter()
            .any(|k| !to.projection.fields.contains_key(k))
        {
            return Err(Error::new(
                "INVALID_RELATIONSHIP",
                "planning",
                "target keys must be declared fields",
            ));
        }
        let reason = if policy.placement == Placement::Local {
            Some("the policy places every operation in the local executor".to_string())
        } else {
            reduction_reason(recipe, r, from, to)
        };
        if let Some(reason) = reason {
            if policy.placement == Placement::RequireSource {
                return Err(Error::new(
                    "PLACEMENT_INFEASIBLE",
                    "planning",
                    format!("policy placement require_source cannot be met for {stage}: {reason}"),
                )
                .with_subject(stage)
                .with_remediation(
                    "Use placement prefer_source or local; the policy is not overridden.",
                ));
            }
            plan.decisions.push(json!({"operation":stage,"source":format!("source:{}",r.to),"policy":policy.placement,"placement":"local","reason":reason}));
        } else {
            // IN materializes the from keys once instead of rescanning them for
            // each target. Unary plus and BINARY retain exact key semantics.
            let from_keys = r
                .from_keys
                .iter()
                .map(|f| {
                    format!(
                        "+iresiphi_from.{} COLLATE BINARY",
                        quoted(column(&from.projection.fields[f]))
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let target_keys = r
                .to_keys
                .iter()
                .map(|t| {
                    format!(
                        "+iresiphi_target.{} COLLATE BINARY",
                        quoted(column(&to.projection.fields[t]))
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let filter = plan.pushed.get(&r.from);
            let sql = format!(
                "({target_keys}) IN (SELECT {from_keys} FROM (SELECT * FROM {}{}) AS iresiphi_from)",
                quoted(a.settings["table"].as_str().unwrap_or("")),
                filter
                    .map(|p| format!(" WHERE {}", p.sql))
                    .unwrap_or_default()
            );
            let parameters = filter.map(|p| p.parameters.clone()).unwrap_or_default();
            plan.pushed.insert(
                r.to.clone(),
                Pushed {
                    stage: stage.clone(),
                    sql,
                    parameters,
                },
            );
            plan.reductions.insert(r.to.clone(), r.from.clone());
            plan.decisions.push(json!({"operation":stage,"source":format!("source:{}",r.to),"policy":policy.placement,"placement":"source","coverage":"whole-table source checks; text formats and valid UTF-8 checked for scanned records only"}));
        }
    }
    if let Some(error) = eligibility_error {
        return Err(error);
    }
    Ok(plan)
}

/// Why a many-to-one target cannot be reduced to the keys of its from records,
/// from the recipe alone. Shared by same-database reduction and key transfer.
fn reduction_reason(
    recipe: &Recipe,
    r: &Relationship,
    from: &Source,
    to: &Source,
) -> Option<String> {
    let decimal = |source: &Source, field: &String| {
        matches!(
            source.projection.fields[field].data_type,
            Type::Decimal { .. }
        )
    };
    if r.cardinality != Cardinality::ManyToOne || r.validity.is_some() || r.period.is_some() {
        Some("only many_to_one without validity or period qualification can be reduced".into())
    } else if r
        .from_keys
        .iter()
        .any(|k| !from.projection.fields.contains_key(k))
    {
        Some("every from key must be a field of the from source itself".into())
    } else if recipe.outputs.iter().any(|o| o.anchor == r.to)
        || recipe
            .relationships
            .iter()
            .any(|other| (other.to == r.to && other.id != r.id) || other.from == r.to)
        || recipe.populations.iter().any(|p| {
            p.recipients == r.to || p.period.from.as_ref().is_some_and(|f| f.source == r.to)
        })
        || recipe
            .outputs
            .iter()
            .flat_map(|o| &o.allocations)
            .any(|a| a.quantity.source == r.to)
    {
        Some("the target source has another use that needs every record".into())
    } else if r.from_keys.iter().any(|k| decimal(from, k))
        || r.to_keys.iter().any(|k| decimal(to, k))
    {
        Some("decimal keys cannot be reduced with exact executor equality".into())
    } else if to.identity.iter().any(|k| decimal(to, k)) {
        Some(
            "the target has a decimal identity whose uniqueness sqlite cannot check exactly".into(),
        )
    } else {
        None
    }
}

/// Plan the key transfer of one relationship whose target is a SQLite table
/// and whose from source is elsewhere (docs/spec/slice-5.md).
fn plan_transfer(
    plan: &mut Plan,
    recipe: &Recipe,
    r: &Relationship,
    policy: &Policy,
) -> Result<()> {
    let (Some(from), Some(to)) = (
        recipe.sources.iter().find(|s| s.id == r.from),
        recipe.sources.iter().find(|s| s.id == r.to),
    ) else {
        return Ok(());
    };
    if r.to_keys
        .iter()
        .any(|k| !to.projection.fields.contains_key(k))
    {
        return Err(Error::new(
            "INVALID_RELATIONSHIP",
            "planning",
            "target keys must be declared fields",
        ));
    }
    let stage = format!("relationship:{}/target", r.id);
    let id = format!("relationship:{}/keys", r.id);
    let mut decision = json!({
        "operation": stage,
        "source": format!("source:{}", r.to),
        "policy": policy.placement,
        "transfers": policy.transfers,
    });
    let reason = if policy.placement == Placement::Local {
        Some("the policy places every operation in the local executor".to_string())
    } else if policy.transfers == Transfers::None {
        Some(format!(
            "the policy permits no transfers; reducing this target needs the keys of {} sent to its source (transfers: keys)",
            r.from
        ))
    } else {
        reduction_reason(recipe, r, from, to)
    };
    if let Some(reason) = reason {
        // Requiring source placement binds a transfer only when the policy
        // also permits it: neither policy setting overrides the other.
        if policy.placement == Placement::RequireSource && policy.transfers == Transfers::Keys {
            return Err(Error::new(
                "PLACEMENT_INFEASIBLE",
                "planning",
                format!("policy placement require_source cannot be met for {stage}: {reason}"),
            )
            .with_subject(stage)
            .with_remediation(
                "Use placement prefer_source or local, or transfers none; the policy is not overridden.",
            ));
        }
        decision["placement"] = json!("local");
        decision["reason"] = json!(reason);
        plan.decisions.push(decision);
        return Ok(());
    }
    decision["placement"] = json!("source");
    decision["reduction"] = json!("transferred_keys");
    decision["transfer"] = json!(id);
    decision["coverage"] = json!(
        "whole-table source checks; text formats and valid UTF-8 checked for scanned records only"
    );
    plan.decisions.push(decision);
    plan.transfer_records.push(json!({
        "transfer": id,
        "from": format!("source:{}", r.from),
        "to": format!("source:{}", r.to),
        "operation": stage,
        "content": "distinct key values, with no null component, of the from records delivered to the executor",
        "fields": r.from_keys,
        "mechanism": "bound query parameters on the target's in-memory snapshot; nothing is written or staged",
        "limit": {"transfer_keys": policy.limits.transfer_keys.min(TRANSFER_PARAMETERS)},
        "rows": null,
        "estimate_status": "unknown",
    }));
    plan.transfers.insert(
        r.to.clone(),
        Transfer {
            stage,
            id,
            from: r.from.clone(),
            from_keys: r.from_keys.clone(),
            target_keys: r
                .to_keys
                .iter()
                .map(|t| {
                    format!(
                        "+iresiphi_target.{} COLLATE BINARY",
                        quoted(column(&to.projection.fields[t]))
                    )
                })
                .collect(),
        },
    );
    Ok(())
}

/// Declared spellings, deliberately without Path component equality.
pub(crate) fn colocated(a: &Binding, b: &Binding) -> bool {
    a.adapter == "sqlite"
        && b.adapter == "sqlite"
        && a.path.as_os_str().as_encoded_bytes() == b.path.as_os_str().as_encoded_bytes()
}
