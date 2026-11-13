//! Source-side placement (docs/spec/slice-3.md, "Contract: placement").
//!
//! One operation can run at a source: an output's anchor eligibility predicate,
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
                    return Err(Error::new(
                        "PLACEMENT_INFEASIBLE",
                        "planning",
                        format!(
                            "policy placement require_source cannot be met for {stage}: {reason}"
                        ),
                    )
                    .with_subject(stage)
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
    Ok(plan)
}
