//! Allocation of a coarse quantity onto explicit population members
//! (docs/spec/slice-2.md, "Contract: allocation").
//!
//! Shares are exact: `quantity × weight ÷ sum(weights in partition)` over
//! arbitrary-precision integers, then quantized to the declared scale. This is
//! the only division in format 0.1.
use crate::model::*;
use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

fn ten(power: u32) -> BigInt {
    BigInt::from(10).pow(power)
}

/// Decimal text of `coefficient × 10^-scale`.
pub(crate) fn render(coefficient: &BigInt, scale: u8) -> String {
    let digits = coefficient.abs().to_string();
    let sign = if coefficient.is_negative() { "-" } else { "" };
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let width = usize::from(scale) + 1;
    let padded = format!("{digits:0>width$}");
    let split = padded.len() - usize::from(scale);
    format!("{sign}{}.{}", &padded[..split], &padded[split..])
}

/// A numeric value as an integer coefficient and its scale.
pub(crate) fn coefficient(value: &Value) -> Option<(BigInt, u8)> {
    match value {
        Value::Int64 { value } => value.parse::<i64>().ok().map(|v| (BigInt::from(v), 0)),
        Value::Decimal {
            value,
            precision,
            scale,
        } => crate::scalar::Decimal::parse(value, *precision, *scale)
            .ok()
            .map(|d| (BigInt::from(d.coefficient), d.scale)),
        _ => None,
    }
}

/// Assigned coefficients at the result scale and the residual, both in units of
/// the last place.
pub(crate) struct Split {
    pub assigned: Vec<BigInt>,
    pub residual: BigInt,
}

/// Split `units` (the quantity at the result scale) by non-negative `weights`
/// whose sum is positive. `rank[i]` orders recipient `i` by ascending identity.
pub(crate) fn split(
    units: &BigInt,
    weights: &[BigInt],
    policy: ResidualPolicy,
    rank: &[usize],
) -> Split {
    let total: BigInt = weights.iter().sum();
    let negative = units.is_negative();
    let magnitude = units.abs();
    let mut assigned = Vec::with_capacity(weights.len());
    match policy {
        ResidualPolicy::LargestRemainder => {
            let mut remainders = Vec::with_capacity(weights.len());
            for w in weights {
                let n = &magnitude * w;
                assigned.push(&n / &total);
                remainders.push(n % &total);
            }
            let floor_sum: BigInt = assigned.iter().sum();
            let mut remaining = &magnitude - floor_sum;
            let mut order: Vec<usize> = (0..weights.len()).collect();
            order.sort_by(|&a, &b| {
                remainders[b]
                    .cmp(&remainders[a])
                    .then(rank[a].cmp(&rank[b]))
            });
            for i in order {
                if !remaining.is_positive() {
                    break;
                }
                assigned[i] += 1;
                remaining -= 1;
            }
        }
        ResidualPolicy::Report => {
            let two = BigInt::from(2);
            for w in weights {
                let n = &magnitude * w;
                let mut q = &n / &total;
                let twice = (n % &total) * &two;
                if twice > total || (twice == total && !(&q % &two).is_zero()) {
                    q += 1;
                }
                assigned.push(q);
            }
        }
    }
    if negative {
        for a in &mut assigned {
            *a = -&*a;
        }
    }
    let sum: BigInt = assigned.iter().sum();
    Split {
        residual: units - sum,
        assigned,
    }
}

fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    while !b.is_zero() {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}

/// The exact share `units × weight ÷ (total × 10^scale)` as decimal text when it
/// terminates within 38 digits; otherwise `None` and the reason.
pub(crate) fn exact_share(
    units: &BigInt,
    weight: &BigInt,
    total: &BigInt,
    scale: u8,
) -> (Option<String>, &'static str) {
    let numerator = units * weight;
    if numerator.is_zero() {
        return (Some("0".into()), "terminates");
    }
    let denominator = total * ten(u32::from(scale));
    let g = gcd(numerator.abs(), denominator.clone());
    let numerator = numerator / &g;
    let mut rest = denominator / &g;
    let (mut twos, mut fives) = (0u32, 0u32);
    let (two, five) = (BigInt::from(2), BigInt::from(5));
    while (&rest % &two).is_zero() {
        rest /= &two;
        twos += 1;
    }
    while (&rest % &five).is_zero() {
        rest /= &five;
        fives += 1;
    }
    if !rest.is_one() {
        return (None, "non_terminating");
    }
    let places = twos.max(fives);
    let denominator = two.pow(twos) * five.pow(fives);
    let digits = numerator * ten(places) / denominator;
    if places > 38 || digits.abs().to_string().len() > 38 {
        return (None, "exceeds_38_digits");
    }
    (Some(render(&digits, places as u8)), "terminates")
}

/// One member of the population as the allocation sees it.
pub(crate) struct Member<'a> {
    pub reference: &'a SourceReference,
    /// Identity values, for ascending tie-breaks.
    pub identity: Vec<Value>,
    pub recipient: bool,
    pub weight: Value,
    /// Index of the quantity record the via relationship matched.
    pub partition: Option<usize>,
    /// Why there is no partition: the via relationship's status and reason.
    pub via: Option<String>,
    pub currency: Option<Value>,
}

/// What one member receives.
pub(crate) struct Assignment {
    pub value: Value,
    /// Reason the row is unresolved (a recipient whose partition has no quantity).
    pub unresolved: Option<String>,
    pub evidence: Json,
}

pub(crate) struct Outcome {
    pub members: Vec<Assignment>,
    pub partitions: Vec<Json>,
    pub summary: Json,
    pub warnings: Vec<Error>,
}

fn compare_identity(a: &[Value], b: &[Value]) -> std::cmp::Ordering {
    for (x, y) in a.iter().zip(b) {
        let order = crate::eval::compare(x, y)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                serde_json::to_string(x)
                    .unwrap_or_default()
                    .cmp(&serde_json::to_string(y).unwrap_or_default())
            });
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

fn plain(row: &Row, keys: &[String]) -> Json {
    Json::Object(
        keys.iter()
            .map(|k| {
                (
                    k.clone(),
                    crate::adapters::plain_value(row.fields.get(k).unwrap_or(&Value::Null)),
                )
            })
            .collect(),
    )
}

/// Allocate `allocation` of `output` over `members`. `quantities` are the
/// records of the quantity source, each one partition.
pub(crate) fn allocate(
    output: &Output,
    allocation: &Allocation,
    quantities: &[Row],
    members: &[Member<'_>],
    policy: &Policy,
) -> Result<Outcome> {
    let subject = format!("output:{}/allocation:{}", output.id, allocation.name);
    let fail = |code: &str, check: &str, message: String, affected: Vec<&SourceReference>| {
        Error::new(code, "allocation", message)
            .with_subject(subject.clone())
            .with_check(check)
            .with_outputs(vec![output.id.clone()])
            .with_affected(affected.len(), affected)
    };
    let scale = allocation.scale;
    let null_zero = allocation.null_weight == Some(NullWeight::Zero);
    let report_unallocated = allocation.unallocated == Some(Unallocated::Report);

    // Weights: negative is always an error; null is an error unless declared zero.
    let mut weights: Vec<Option<(BigInt, u8)>> = Vec::with_capacity(members.len());
    let (mut negative, mut null) = (vec![], vec![]);
    for m in members {
        if !m.recipient {
            weights.push(None);
            continue;
        }
        let weight = match &m.weight {
            Value::Null => {
                null.push(m.reference);
                Some((BigInt::zero(), 0))
            }
            other => {
                let w = coefficient(other).ok_or_else(|| {
                    Error::new("TYPE_MISMATCH", "allocation", "weight must be numeric")
                        .with_subject(subject.clone())
                })?;
                if w.0.is_negative() {
                    negative.push(m.reference);
                }
                Some(w)
            }
        };
        weights.push(weight);
    }
    if !negative.is_empty() {
        return Err(fail(
            "ALLOCATION_NEGATIVE_WEIGHT",
            "allocation.weights",
            format!(
                "{} recipients of allocation {} have a negative weight; weights must be zero or positive",
                negative.len(),
                allocation.name
            ),
            negative,
        ));
    }
    if !null.is_empty() && !null_zero {
        return Err(fail(
            "ALLOCATION_NULL_WEIGHT",
            "allocation.weights",
            format!(
                "{} recipients of allocation {} have a null weight; declare null_weight \"zero\" to treat them as weight 0",
                null.len(),
                allocation.name
            ),
            null,
        ));
    }
    let weight_scale = weights.iter().flatten().map(|w| w.1).max().unwrap_or(0);
    let scaled = |w: &(BigInt, u8)| &w.0 * ten(u32::from(weight_scale - w.1));

    // Currency: a recipient that carries one must match its partition's quantity.
    let mut mixed = vec![];
    for m in members.iter().filter(|m| m.recipient) {
        if let (Some(p), Some(currency)) = (m.partition, &m.currency) {
            if let Some(q) = quantities[p].fields.get("currency") {
                if q != currency {
                    mixed.push(m.reference);
                }
            }
        }
    }
    if !mixed.is_empty() {
        return Err(fail(
            "MIXED_CURRENCY",
            "allocation.currency",
            format!(
                "{} recipients of allocation {} carry a currency different from their partition's quantity; amounts in different currencies are never allocated across",
                mixed.len(),
                allocation.name
            ),
            mixed,
        ));
    }

    // Recipients of each partition, in member order.
    let mut by_partition: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, m) in members.iter().enumerate() {
        if let (true, Some(p)) = (m.recipient, m.partition) {
            by_partition.entry(p).or_default().push(i);
        }
    }
    let mut assigned: Vec<Option<BigInt>> = vec![None; members.len()];
    let mut shares: Vec<Option<(Option<String>, &'static str)>> = vec![None; members.len()];
    let mut member_status: Vec<&'static str> = members
        .iter()
        .map(|m| {
            if m.recipient {
                "allocated"
            } else {
                "not_recipient"
            }
        })
        .collect();
    let mut partitions = vec![];
    let (mut zero_weight, mut no_recipients) = (vec![], vec![]);
    let (mut source_total, mut assigned_total, mut residual_total) =
        (BigInt::zero(), BigInt::zero(), BigInt::zero());
    // Totals per quantity currency; amounts in different currencies are never added.
    let mut by_currency: BTreeMap<String, [BigInt; 3]> = BTreeMap::new();
    let currency_of = |row: &Row| match row.fields.get("currency") {
        Some(Value::String { value }) => value.clone(),
        _ => "undeclared".to_string(),
    };
    for (p, row) in quantities.iter().enumerate() {
        let recipients = by_partition.get(&p).cloned().unwrap_or_default();
        let weight_sum: BigInt = recipients
            .iter()
            .filter_map(|&i| weights[i].as_ref().map(scaled))
            .sum();
        let mut record = json!({
            "output": output.id,
            "allocation": allocation.name,
            "partition": plain(row, &allocation.partition),
            "quantity_reference": row.reference,
            "recipient_count": recipients.len(),
            "weight_sum": render(&weight_sum, weight_scale),
            "policy": allocation.residual,
        });
        let quantity = row
            .fields
            .get(&allocation.quantity.field)
            .and_then(coefficient);
        let Some((q, q_scale)) = quantity else {
            for &i in &recipients {
                member_status[i] = "missing_quantity";
            }
            record["quantity"] = Json::Null;
            record["assigned_sum"] = Json::Null;
            record["residual"] = Json::Null;
            record["status"] = json!("no_quantity");
            record["conserved"] = Json::Null;
            partitions.push(record);
            continue;
        };
        let units = q * ten(u32::from(scale - q_scale));
        source_total += &units;
        let currency_totals = by_currency.entry(currency_of(row)).or_default();
        currency_totals[0] += &units;
        record["quantity"] = json!(render(&units, scale));
        if recipients.is_empty() || weight_sum.is_zero() {
            if recipients.is_empty() {
                no_recipients.push(&row.reference);
            } else {
                zero_weight.push(&row.reference);
            }
            for &i in &recipients {
                member_status[i] = "unallocated";
            }
            currency_totals[2] += &units;
            residual_total += &units;
            record["assigned_sum"] = json!(render(&BigInt::zero(), scale));
            record["residual"] = json!(render(&units, scale));
            record["status"] = json!("unallocated");
            record["reason"] = json!(if recipients.is_empty() {
                "no_recipients"
            } else {
                "zero_weight"
            });
            record["conserved"] = json!(true);
            partitions.push(record);
            continue;
        }
        let partition_weights: Vec<BigInt> = recipients
            .iter()
            .map(|&i| weights[i].as_ref().map(scaled).unwrap_or_default())
            .collect();
        let mut by_identity: Vec<usize> = (0..recipients.len()).collect();
        by_identity.sort_by(|&a, &b| {
            compare_identity(
                &members[recipients[a]].identity,
                &members[recipients[b]].identity,
            )
        });
        let mut rank = vec![0; recipients.len()];
        for (position, &i) in by_identity.iter().enumerate() {
            rank[i] = position;
        }
        let result = split(&units, &partition_weights, allocation.residual, &rank);
        let sum: BigInt = result.assigned.iter().sum();
        // Conservation: assigned sum plus residual equals the quantity exactly.
        let conserved = &sum + &result.residual == units
            && (allocation.residual == ResidualPolicy::Report || result.residual.is_zero());
        if !conserved {
            return Err(fail(
                "ALLOCATION_CONSERVATION",
                "allocation.conservation",
                format!(
                    "allocation {} does not conserve its quantity in one partition: assigned {} plus residual {} differs from {}",
                    allocation.name,
                    render(&sum, scale),
                    render(&result.residual, scale),
                    render(&units, scale)
                ),
                vec![&row.reference],
            ));
        }
        let limit = ten(u32::from(allocation.precision));
        for (k, &i) in recipients.iter().enumerate() {
            if result.assigned[k].abs() >= limit {
                return Err(Error::new(
                    "ARITHMETIC_OVERFLOW",
                    "allocation",
                    format!(
                        "an amount assigned by allocation {} exceeds decimal precision {}",
                        allocation.name, allocation.precision
                    ),
                )
                .with_subject(subject.clone())
                .with_affected(1, [members[i].reference]));
            }
            assigned[i] = Some(result.assigned[k].clone());
            shares[i] = Some(exact_share(
                &units,
                &partition_weights[k],
                &weight_sum,
                scale,
            ));
        }
        assigned_total += &sum;
        residual_total += &result.residual;
        currency_totals[1] += &sum;
        currency_totals[2] += &result.residual;
        record["assigned_sum"] = json!(render(&sum, scale));
        record["residual"] = json!(render(&result.residual, scale));
        record["status"] = json!("allocated");
        record["conserved"] = json!(true);
        partitions.push(record);
    }
    for (code, references, what) in [
        (
            "ALLOCATION_NO_RECIPIENTS",
            no_recipients,
            "have no recipients",
        ),
        (
            "ALLOCATION_ZERO_WEIGHT",
            zero_weight,
            "have weights that sum to zero",
        ),
    ] {
        if !references.is_empty() && !report_unallocated {
            return Err(fail(
                code,
                "allocation.partitions",
                format!(
                    "{} partitions of allocation {} {what}; declare unallocated \"report\" to report their quantity as residual",
                    references.len(),
                    allocation.name
                ),
                references,
            ));
        }
    }
    // Recipients without a partition (the via lookup did not match) have no quantity.
    for (i, m) in members.iter().enumerate() {
        if m.recipient && m.partition.is_none() {
            member_status[i] = "missing_quantity";
        }
    }
    let missing: Vec<&SourceReference> = members
        .iter()
        .zip(&member_status)
        .filter(|(_, s)| **s == "missing_quantity")
        .map(|(m, _)| m.reference)
        .collect();
    let mut warnings = vec![];
    if !missing.is_empty() {
        warnings.push(
            Error::new(
                "ALLOCATION_MISSING_QUANTITY",
                "allocation",
                format!(
                    "{} recipients of allocation {} have no source quantity; each keeps a null allocation and is unresolved, never zero",
                    missing.len(),
                    allocation.name
                ),
            )
            .warning()
            .with_subject(subject.clone())
            .with_check("allocation.coverage")
            .with_outputs(vec![output.id.clone()])
            .with_affected(missing.len(), missing),
        );
    }
    let assignments = members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let status = member_status[i];
            let value = assigned[i]
                .as_ref()
                .map(|c| Value::Decimal {
                    value: render(c, scale),
                    precision: allocation.precision,
                    scale,
                })
                .unwrap_or(Value::Null);
            let unresolved = (status == "missing_quantity").then(|| {
                format!(
                    "{} unmatched: no source quantity in {} ({})",
                    allocation.name,
                    allocation.quantity.source,
                    m.via
                        .clone()
                        .unwrap_or_else(|| format!("{} is null", allocation.quantity.field))
                )
            });
            let (share, share_status) = match &shares[i] {
                Some((text, status)) => (json!(text), json!(status)),
                None => (Json::Null, Json::Null),
            };
            let evidence = json!({
                "status": status,
                "recipient": m.recipient,
                "partition": m.partition.map(|p| plain(&quantities[p], &allocation.partition)),
                "weight": if m.recipient { crate::adapters::plain_value(&m.weight) } else { Json::Null },
                "weight_rule": if m.recipient && m.weight == Value::Null { "declared_zero" } else { "declared" },
                "exact_share": share,
                "exact_share_terminates": shares[i].as_ref().map(|(text, _)| text.is_some()),
                "exact_share_status": share_status,
                "assigned": crate::adapters::plain_value(&value),
            });
            Assignment {
                value,
                unresolved,
                evidence,
            }
        })
        .collect();
    // Overall totals exist only when every partition is in one currency.
    let single = by_currency.len() <= 1;
    let summary = json!({
        "quantity": format!("source:{}/field:{}", allocation.quantity.source, allocation.quantity.field),
        "partition": allocation.partition,
        "via": format!("relationship:{}", allocation.via),
        "policy": allocation.residual,
        "precision": allocation.precision,
        "scale": scale,
        "null_weight": allocation.null_weight.unwrap_or(NullWeight::Error),
        "unallocated": allocation.unallocated.unwrap_or(Unallocated::Error),
        "recipient_filter": allocation.recipient_filter.is_some(),
        "recipients": members.iter().filter(|m| m.recipient).count(),
        "missing_quantity_recipients": member_status.iter().filter(|s| **s == "missing_quantity").count(),
        "source_total": single.then(|| render(&source_total, scale)),
        "assigned_total": single.then(|| render(&assigned_total, scale)),
        "residual_total": single.then(|| render(&residual_total, scale)),
        "conserved": &assigned_total + &residual_total == source_total,
        "by_currency": by_currency.iter().map(|(currency, [source, assigned, residual])| {
            (currency.clone(), json!({
                "source_total": render(source, scale),
                "assigned_total": render(assigned, scale),
                "residual_total": render(residual, scale),
                "conserved": assigned + residual == *source,
            }))
        }).collect::<serde_json::Map<_, _>>(),
        "check": {
            "id": "allocation.conservation",
            "status": "passed",
            "partitions": partitions.len(),
            "statement": "For every partition, assigned sum plus reported residual equals the source quantity exactly.",
        },
        "partitions": partitions.iter().map(|p| {
            let mut p = p.clone();
            if let Some(map) = p.as_object_mut() {
                map.remove("output");
                map.remove("allocation");
                map.remove("quantity_reference");
            }
            p
        }).collect::<Vec<_>>(),
    });
    let _ = policy;
    if [&source_total, &assigned_total, &residual_total]
        .iter()
        .any(|t| t.abs() >= ten(38))
    {
        return Err(Error::new(
            "ARITHMETIC_OVERFLOW",
            "allocation",
            format!(
                "totals of allocation {} exceed decimal precision 38",
                allocation.name
            ),
        )
        .with_subject(subject));
    }
    Ok(Outcome {
        members: assignments,
        partitions,
        summary,
        warnings,
    })
}
