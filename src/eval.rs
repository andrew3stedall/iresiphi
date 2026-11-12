use crate::{
    model::*,
    scalar::{Decimal, Rounding},
    validation::{canonical_value, expression_type, value_type},
};
use num_bigint::BigInt;
use std::cmp::Ordering;
fn fail(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, "execution", message)
}
fn decimal(v: &Value) -> Result<Decimal> {
    if let Value::Decimal {
        value,
        precision,
        scale,
    } = v
    {
        Decimal::parse(value, *precision, *scale).map_err(|e| fail("INVALID_VALUE", e.to_string()))
    } else {
        Err(fail("TYPE_MISMATCH", "decimal required"))
    }
}
fn dv(d: Decimal) -> Value {
    Value::Decimal {
        value: d.to_string(),
        precision: d.precision,
        scale: d.scale,
    }
}
fn integer(v: &Value) -> Result<i64> {
    if let Value::Int64 { value } = v {
        value.parse().map_err(|_| fail("INVALID_VALUE", "Int64"))
    } else {
        Err(fail("TYPE_MISMATCH", "Int64 required"))
    }
}
fn truth(v: &Value) -> Result<Option<bool>> {
    match v {
        Value::Null => Ok(None),
        Value::Boolean { value } => Ok(Some(*value)),
        _ => Err(fail("TYPE_MISMATCH", "Boolean required")),
    }
}
fn bv(b: Option<bool>) -> Value {
    b.map(|value| Value::Boolean { value })
        .unwrap_or(Value::Null)
}
pub fn compare(a: &Value, b: &Value) -> Result<Option<Ordering>> {
    if matches!(a, Value::Null) || matches!(b, Value::Null) {
        return Ok(None);
    }
    Ok(Some(match (a, b) {
        (Value::Decimal { .. }, Value::Decimal { .. }) => {
            let x = decimal(a)?;
            let y = decimal(b)?;
            let s = x.scale.max(y.scale);
            let left = BigInt::from(x.coefficient) * BigInt::from(10).pow((s - x.scale).into());
            let right = BigInt::from(y.coefficient) * BigInt::from(10).pow((s - y.scale).into());
            left.cmp(&right)
        }
        (Value::Int64 { .. }, Value::Int64 { .. }) => integer(a)?.cmp(&integer(b)?),
        (Value::String { value: a }, Value::String { value: b })
        | (Value::Date { value: a }, Value::Date { value: b }) => a.cmp(b),
        (Value::Boolean { value: a }, Value::Boolean { value: b }) => a.cmp(b),
        _ => {
            return Err(fail(
                "TYPE_MISMATCH",
                "comparison requires compatible types",
            ));
        }
    }))
}
pub fn cast(value: Value, target: &Type) -> Result<Value> {
    if value == Value::Null {
        return Ok(value);
    }
    let value = canonical_value(&value)?;
    let result = match target {
        Type::String => Value::String {
            value: match value {
                Value::String { value }
                | Value::Date { value }
                | Value::Int64 { value }
                | Value::Decimal { value, .. } => value,
                Value::Boolean { value } => value.to_string(),
                Value::Null => unreachable!(),
            },
        },
        Type::Int64 => {
            let n = match &value {
                Value::Int64 { .. } => integer(&value)?,
                Value::String { value } => value
                    .parse::<i64>()
                    .map_err(|_| fail("INVALID_CAST", "invalid Int64 text"))?,
                Value::Decimal { .. } => {
                    let d = decimal(&value)?;
                    let divisor = 10i128.pow(d.scale.into());
                    if d.coefficient % divisor != 0 {
                        return Err(fail("INVALID_CAST", "inexact integer cast"));
                    }
                    i64::try_from(d.coefficient / divisor)
                        .map_err(|_| fail("ARITHMETIC_OVERFLOW", "integer narrowing"))?
                }
                _ => return Err(fail("INVALID_CAST", "cannot convert value to Int64")),
            };
            Value::Int64 {
                value: n.to_string(),
            }
        }
        Type::Decimal { precision, scale } => {
            let text = match &value {
                Value::String { value } => value.clone(),
                // Integer text is always well formed, so failure is narrowing overflow.
                Value::Int64 { value } => {
                    return Ok(dv(Decimal::parse(value, *precision, *scale)
                        .map_err(|e| fail("ARITHMETIC_OVERFLOW", e.to_string()))?));
                }
                Value::Decimal { .. } => {
                    let d = decimal(&value)?;
                    let out = d
                        .quantize(*precision, *scale, Rounding::TowardZero)
                        .map_err(|e| fail("ARITHMETIC_OVERFLOW", e.to_string()))?;
                    if compare(&value, &dv(out))? != Some(Ordering::Equal) {
                        return Err(fail(
                            "INVALID_CAST",
                            "inexact decimal cast requires quantize",
                        ));
                    }
                    return Ok(dv(out));
                }
                _ => return Err(fail("INVALID_CAST", "cannot convert value to decimal")),
            };
            dv(Decimal::parse(&text, *precision, *scale)
                .map_err(|e| fail("INVALID_CAST", e.to_string()))?)
        }
        Type::Date => match value {
            Value::Date { value } | Value::String { value } => Value::Date { value },
            _ => return Err(fail("INVALID_CAST", "date requires explicit ISO text")),
        },
        Type::Boolean => match value {
            Value::Boolean { value } => Value::Boolean { value },
            Value::String { value } if value == "true" || value == "false" => Value::Boolean {
                value: value == "true",
            },
            _ => return Err(fail("INVALID_CAST", "invalid Boolean cast")),
        },
    };
    value_type(&result).map_err(|e| fail("INVALID_CAST", e.message))?;
    Ok(result)
}
/// Evaluate checked scalar expressions. The executor resolves explicit field paths;
/// aggregates are evaluated at their declared population boundary by that executor.
pub fn eval<F>(expr: &Expr, recipe: &Recipe, resolver: &mut F) -> Result<Value>
where
    F: FnMut(&str, &str, &[String]) -> Result<Value>,
{
    // Inference first ensures an unselected case branch cannot hide unsupported semantics.
    expression_type(expr, recipe)?;
    evaluate(expr, recipe, resolver, 0)
}
fn evaluate<F>(expr: &Expr, r: &Recipe, resolver: &mut F, depth: usize) -> Result<Value>
where
    F: FnMut(&str, &str, &[String]) -> Result<Value>,
{
    if depth > 256 {
        return Err(fail("RESOURCE_LIMIT", "expression depth exceeds 256"));
    }
    let mut next = |e: &Expr| evaluate(e, r, resolver, depth + 1);
    use Expr::*;
    match expr {
        Literal { value } => canonical_value(value),
        Field {
            source,
            field,
            path,
        } => resolver(source, field, path),
        Feature { id } => {
            let f = r
                .features
                .iter()
                .find(|f| f.id == *id)
                .ok_or_else(|| fail("MISSING_REFERENCE", id))?;
            next(&f.expr)
        }
        Add { left, right } | Subtract { left, right } | Multiply { left, right } => {
            let a = next(left)?;
            let b = next(right)?;
            if a == Value::Null || b == Value::Null {
                return Ok(Value::Null);
            }
            match (&a, &b) {
                (Value::Int64 { .. }, Value::Int64 { .. }) => {
                    let a = integer(&a)?;
                    let b = integer(&b)?;
                    let n = match expr {
                        Add { .. } => a.checked_add(b),
                        Subtract { .. } => a.checked_sub(b),
                        _ => a.checked_mul(b),
                    }
                    .ok_or_else(|| fail("ARITHMETIC_OVERFLOW", "Int64 arithmetic"))?;
                    Ok(Value::Int64 {
                        value: n.to_string(),
                    })
                }
                (Value::Decimal { .. }, Value::Decimal { .. }) => {
                    let a = decimal(&a)?;
                    let b = decimal(&b)?;
                    let Some(Type::Decimal { precision, scale }) = expression_type(expr, r)? else {
                        return Err(fail("TYPE_MISMATCH", "decimal arithmetic"));
                    };
                    let result = match expr {
                        Add { .. } => a.checked_add(b, precision, scale),
                        Subtract { .. } => a.checked_sub(b, precision, scale),
                        _ => a.checked_mul(b, precision, scale),
                    }
                    .map_err(|e| fail("ARITHMETIC_OVERFLOW", e.to_string()))?;
                    Ok(dv(result))
                }
                _ => Err(fail("TYPE_MISMATCH", "numeric arithmetic")),
            }
        }
        Equal { left, right } | Less { left, right } | LessEqual { left, right } => {
            let a = next(left)?;
            let b = next(right)?;
            let order = compare(&a, &b)?;
            Ok(bv(order.map(|x| match expr {
                Equal { .. } => x == Ordering::Equal,
                Less { .. } => x == Ordering::Less,
                _ => x != Ordering::Greater,
            })))
        }
        And { left, right } | Or { left, right } => {
            let a = truth(&next(left)?)?;
            let b = truth(&next(right)?)?;
            Ok(bv(if matches!(expr, And { .. }) {
                match (a, b) {
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                }
            } else {
                match (a, b) {
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    (Some(false), Some(false)) => Some(false),
                    _ => None,
                }
            }))
        }
        Not { expr } => Ok(bv(truth(&next(expr)?)?.map(|x| !x))),
        IsNull { expr } => Ok(Value::Boolean {
            value: next(expr)? == Value::Null,
        }),
        ValidOn {
            date,
            valid_from,
            valid_to,
        } => {
            let date = next(date)?;
            let start = next(valid_from)?;
            let end = next(valid_to)?;
            let (Value::Date { value: date }, Value::Date { value: start }) = (&date, &start)
            else {
                // A missing date or lower bound leaves validity unknown.
                return Ok(Value::Null);
            };
            match &end {
                Value::Null => Ok(bv(Some(start <= date))),
                Value::Date { value: end } if end <= start => Err(fail(
                    "INVALID_INTERVAL",
                    "validity interval requires valid_from < valid_to",
                )),
                // ISO YYYY-MM-DD text orders chronologically.
                Value::Date { value: end } => Ok(bv(Some(start <= date && date < end))),
                _ => Err(fail("TYPE_MISMATCH", "valid_to must be a date")),
            }
        }
        Divide { .. } => Err(fail(
            "UNSUPPORTED_OPERATION",
            "division is deferred in format 0.1",
        )),
        Coalesce { values } => {
            for e in values {
                let value = next(e)?;
                if value != Value::Null {
                    return Ok(value);
                }
            }
            Ok(Value::Null)
        }
        Case {
            condition,
            then,
            otherwise,
        } => {
            if truth(&next(condition)?)? == Some(true) {
                next(then)
            } else {
                next(otherwise)
            }
        }
        Cast { expr, data_type } => cast(next(expr)?, data_type),
        Quantize {
            expr,
            precision,
            scale,
            rounding,
        } => {
            let value = next(expr)?;
            if value == Value::Null {
                return Ok(value);
            }
            Ok(dv(decimal(&value)?
                .quantize(*precision, *scale, *rounding)
                .map_err(|e| fail("ARITHMETIC_OVERFLOW", e.to_string()))?))
        }
        Aggregate { .. } => Err(fail(
            "UNSUPPORTED_OPERATION",
            "aggregate requires executor population context",
        )),
    }
}
