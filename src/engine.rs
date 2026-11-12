use crate::model::*;
use datafusion::{
    arrow::{
        array::{StringArray, UInt64Array},
        datatypes::{DataType, Field as ArrowField, Schema},
        record_batch::RecordBatch,
    },
    common::JoinType,
    execution::{
        context::{SessionConfig, SessionContext},
        disk_manager::{DiskManagerBuilder, DiskManagerMode},
        runtime_env::RuntimeEnvBuilder,
    },
    prelude::col,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub type Table = Vec<BTreeMap<String, Value>>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub status: String,
    pub tables: BTreeMap<String, Table>,
    pub evidence: Vec<serde_json::Value>,
    pub diagnostics: Vec<Error>,
    pub manifest: serde_json::Value,
}
#[derive(Default)]
pub struct ExecutionContext {
    busy: AtomicBool,
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl ExecutionContext {
    pub fn run(
        &self,
        recipe: &Recipe,
        bindings: &Bindings,
        policy: &Policy,
        cancel: &AtomicBool,
    ) -> RunReport {
        let id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let started = Instant::now();
        if self.busy.swap(true, Ordering::AcqRel) {
            return failed(
                id,
                Error::new("RUN_BUSY", "admission", "execution context already active"),
                started,
            );
        }
        let _busy = Busy(&self.busy);
        let report = crate::validation::validate(recipe);
        if !report.valid {
            return failed(
                id,
                Error::new(
                    "INVALID_RECIPE",
                    "validation",
                    serde_json::to_string(&report).unwrap_or_default(),
                ),
                started,
            );
        }
        match execute(recipe, bindings, policy, cancel) {
            Ok((tables, evidence, unresolved, counts)) => {
                let manifest = serde_json::json!({"run_id":id,"format_version":"0.1","package_version":env!("CARGO_PKG_VERSION"),"recipe_hash":recipe.hash().ok(),"policy":policy,"source_rows":counts,"elapsed_ms":started.elapsed().as_millis(),"peak_rss":null,"engine_peak_reservation":null,"checks":{"static":"passed","data":"passed"},"unresolved_rows":unresolved,"complete":true});
                RunReport {
                    run_id: id,
                    status: if unresolved > 0 {
                        "succeeded_with_unresolved"
                    } else {
                        "succeeded"
                    }
                    .into(),
                    tables,
                    evidence,
                    diagnostics: vec![],
                    manifest,
                }
            }
            Err(error) => failed(id, error, started),
        }
    }
}
fn failed(id: String, error: Error, started: Instant) -> RunReport {
    let status = if error.code == "CANCELLED" {
        "cancelled"
    } else {
        "failed"
    };
    RunReport {
        run_id: id.clone(),
        status: status.into(),
        tables: BTreeMap::new(),
        evidence: vec![],
        diagnostics: vec![error],
        manifest: serde_json::json!({"run_id":id,"complete":false,"checks":{"data":"incomplete"},"elapsed_ms":started.elapsed().as_millis()}),
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::new("CANCELLED", "execution", "run cancelled"))
    } else {
        Ok(())
    }
}
fn engine_error(e: impl std::fmt::Display) -> Error {
    Error::new("EXECUTION_ERROR", "execution", e.to_string())
}
fn key(row: &Row, keys: &[String]) -> Option<String> {
    let values: Vec<_> = keys
        .iter()
        .map(|k| row.fields.get(k).unwrap_or(&Value::Null))
        .collect();
    if values.iter().any(|v| matches!(v, Value::Null)) {
        None
    } else {
        Some(serde_json::to_string(&values).unwrap())
    }
}
fn batch(
    prefix: &str,
    rows: &[Row],
    keys: &[String],
    dates: [Option<&str>; 3],
) -> Result<RecordBatch> {
    let name = |n: &str| format!("{prefix}{n}");
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new(name("key"), DataType::Utf8, true),
        ArrowField::new(name("idx"), DataType::UInt64, false),
        ArrowField::new(name("date"), DataType::Utf8, true),
        ArrowField::new(name("start"), DataType::Utf8, true),
        ArrowField::new(name("end"), DataType::Utf8, true),
    ]));
    let mut arrays: Vec<Arc<dyn datafusion::arrow::array::Array>> = vec![
        Arc::new(StringArray::from(
            rows.iter().map(|r| key(r, keys)).collect::<Vec<_>>(),
        )),
        Arc::new(UInt64Array::from_iter_values(0..rows.len() as u64)),
    ];
    for field in dates {
        arrays.push(Arc::new(StringArray::from(
            rows.iter()
                .map(|r| {
                    field.and_then(|f| match r.fields.get(f) {
                        Some(Value::Date { value }) => Some(value.clone()),
                        _ => None,
                    })
                })
                .collect::<Vec<_>>(),
        )));
    }
    RecordBatch::try_new(schema, arrays).map_err(engine_error)
}

type Matches = BTreeMap<String, Vec<Vec<usize>>>;
fn qualified(rel: &Relationship, left: &Row, right: &Row) -> Result<bool> {
    if let Some(v) = &rel.validity {
        let date = |r: &Row, k: &str| match r.fields.get(k) {
            Some(Value::Date { value }) => Ok(value.clone()),
            _ => Err(Error::new(
                "INVALID_DATE",
                "data",
                format!("required date {k}"),
            )),
        };
        let d = date(left, &v.date)?;
        let start = date(right, &v.valid_from)?;
        let end = match right.fields.get(&v.valid_to) {
            Some(Value::Null) => None,
            _ => Some(date(right, &v.valid_to)?),
        };
        if end.as_ref().is_some_and(|e| e <= &start) {
            return Err(Error::new(
                "INVALID_INTERVAL",
                "data",
                "rate interval must have start < end",
            ));
        }
        Ok(d >= start && end.as_ref().is_none_or(|e| &d < e))
    } else {
        Ok(true)
    }
}
fn execute(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<(
    BTreeMap<String, Table>,
    Vec<serde_json::Value>,
    usize,
    BTreeMap<String, usize>,
)> {
    check(cancel)?;
    let sources = crate::adapters::read_sources(recipe, bindings, policy, cancel)?;
    for source in &recipe.sources {
        let rows = &sources[&source.id];
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            let k = key(row, &source.identity)
                .ok_or_else(|| Error::new("NULL_IDENTITY", "data", source.id.clone()))?;
            if !seen.insert(k) {
                return Err(Error::new("DUPLICATE_IDENTITY", "data", source.id.clone()));
            }
        }
    }
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_limit(policy.limits.engine_bytes, 1.0)
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()
        .map_err(engine_error)?;
    let ctx = SessionContext::new_with_config_rt(
        SessionConfig::new()
            .with_target_partitions(2)
            .with_batch_size(policy.limits.batch_rows.min(8192).max(1)),
        runtime,
    );
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(engine_error)?;
    let mut matches = Matches::new();
    let mut total = 0;
    let mut ordered = vec![];
    let mut pending: Vec<_> = recipe.relationships.iter().collect();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|rel| {
            let ready = rel
                .from_keys
                .iter()
                .filter_map(|k| k.split_once('.').map(|(id, _)| id))
                .all(|id| ordered.iter().any(|r: &&Relationship| r.id == id));
            if ready {
                ordered.push(*rel);
                false
            } else {
                true
            }
        });
        if pending.len() == before {
            return Err(Error::new(
                "DEPENDENCY_CYCLE",
                "validation",
                "relationship key dependencies",
            ));
        }
    }
    for (index, rel) in ordered.into_iter().enumerate() {
        check(cancel)?;
        let mut effective_left = sources[&rel.from].clone();
        for key_name in &rel.from_keys {
            if let Some((dependency, field)) = key_name.split_once('.') {
                let prior = recipe
                    .relationships
                    .iter()
                    .find(|r| r.id == dependency)
                    .ok_or_else(|| Error::new("UNKNOWN_PATH", "execution", dependency))?;
                for (i, row) in effective_left.iter_mut().enumerate() {
                    let candidates = &matches[dependency][i];
                    let value = if candidates.len() == 1 {
                        sources[&prior.to][candidates[0]]
                            .fields
                            .get(field)
                            .cloned()
                            .unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    };
                    row.fields.insert(key_name.clone(), value);
                }
            }
        }
        let left = &effective_left;
        let right = &sources[&rel.to];
        if let Some(validity) = &rel.validity {
            for row in right {
                let start = match row.fields.get(&validity.valid_from) {
                    Some(Value::Date { value }) => value,
                    _ => {
                        return Err(Error::new(
                            "INVALID_DATE",
                            "data",
                            "valid_from must be a date",
                        ));
                    }
                };
                match row.fields.get(&validity.valid_to) {
                    Some(Value::Null) => {}
                    Some(Value::Date { value }) if value > start => {}
                    _ => {
                        return Err(Error::new(
                            "INVALID_INTERVAL",
                            "data",
                            "validity interval must be increasing or open-ended",
                        ));
                    }
                }
            }
        }

        let mut index_by_key: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, row) in right.iter().enumerate() {
            if let Some(k) = key(row, &rel.to_keys) {
                index_by_key.entry(k).or_default().push(i);
            }
        }
        for row in left {
            if let Some(candidates) = key(row, &rel.from_keys).and_then(|k| index_by_key.get(&k)) {
                for candidate in candidates {
                    if qualified(rel, row, &right[*candidate])? {
                        total += 1;
                        if total > policy.limits.candidate_pairs {
                            return Err(Error::new(
                                "RESOURCE_LIMIT",
                                "execution",
                                "qualified lookup candidate budget exceeded",
                            ));
                        }
                    }
                }
            }
        }
        // Candidate pairs come from an engine equi-join; ISO dates compare correctly as text.
        let left_frame = ctx
            .read_batch(batch(
                "l_",
                left,
                &rel.from_keys,
                [rel.validity.as_ref().map(|v| v.date.as_str()), None, None],
            )?)
            .map_err(engine_error)?;
        let right_frame = ctx
            .read_batch(batch(
                "r_",
                right,
                &rel.to_keys,
                [
                    None,
                    rel.validity.as_ref().map(|v| v.valid_from.as_str()),
                    rel.validity.as_ref().map(|v| v.valid_to.as_str()),
                ],
            )?)
            .map_err(engine_error)?;
        let temporal = rel.validity.as_ref().map(|_| {
            col("l_date")
                .gt_eq(col("r_start"))
                .and(col("r_end").is_null().or(col("l_date").lt(col("r_end"))))
        });
        let batches: Vec<RecordBatch> = rt.block_on(async {
            let frame=left_frame.join(right_frame,JoinType::Inner,&["l_key"],&["r_key"],temporal)?.select_columns(&["l_idx","r_idx"])?;
            let work=frame.collect();tokio::pin!(work);
            loop {tokio::select! {result=&mut work => break result, _=tokio::time::sleep(std::time::Duration::from_millis(50)) => {if cancel.load(Ordering::Acquire){break Err(datafusion::error::DataFusionError::Execution("run cancelled".into()));}}}}
        }).map_err(|e|if cancel.load(Ordering::Acquire){Error::new("CANCELLED","execution","run cancelled")}else{engine_error(e)})?;
        let _ = index;
        let mut found = vec![vec![]; left.len()];
        for b in batches {
            check(cancel)?;
            let li = b.column(0).as_any().downcast_ref::<UInt64Array>().unwrap();
            let ri = b.column(1).as_any().downcast_ref::<UInt64Array>().unwrap();
            for n in 0..b.num_rows() {
                let l = li.value(n) as usize;
                let r = ri.value(n) as usize;
                if qualified(rel, &left[l], &right[r])? {
                    found[l].push(r);
                }
            }
        }
        if rel.strict
            && rel.cardinality != Cardinality::OneToMany
            && found.iter().any(|v| v.len() > 1)
        {
            return Err(Error::new("CARDINALITY_VIOLATION", "data", rel.id.clone()));
        }
        matches.insert(rel.id.clone(), found);
    }
    let mut tables = BTreeMap::new();
    let mut evidence = vec![];
    let mut unresolved = 0;
    for output in &recipe.outputs {
        let mut table = vec![];
        for (row_index, row) in sources[&output.anchor].iter().enumerate() {
            check(cancel)?;
            let mut reasons = vec![];
            let mut resolver = |source: &str, field: &str, path: &[String]| -> Result<Value> {
                let mut current = output.anchor.as_str();
                let mut position = row_index;
                for rel_id in path {
                    let rel = recipe
                        .relationships
                        .iter()
                        .find(|r| &r.id == rel_id)
                        .ok_or_else(|| Error::new("UNKNOWN_PATH", "execution", rel_id.clone()))?;
                    if rel.from != current {
                        return Err(Error::new("INVALID_PATH", "execution", rel.id.clone()));
                    }
                    let candidates = &matches[&rel.id][position];
                    if candidates.len() != 1 {
                        reasons.push(
                            if candidates.is_empty() {
                                "unmatched"
                            } else {
                                "ambiguous"
                            }
                            .to_string(),
                        );
                        return Ok(Value::Null);
                    }
                    position = candidates[0];
                    current = &rel.to;
                }
                if current != source {
                    return Err(Error::new("INVALID_PATH", "execution", source.to_string()));
                }
                Ok(sources[current][position]
                    .fields
                    .get(field)
                    .cloned()
                    .unwrap_or(Value::Null))
            };
            if let Some(predicate) = &output.eligibility {
                if !matches!(
                    crate::eval::eval(predicate, recipe, &mut resolver)?,
                    Value::Boolean { value: true }
                ) {
                    continue;
                }
            }
            let mut values = BTreeMap::new();
            for column in &output.columns {
                values.insert(
                    column.name.clone(),
                    crate::eval::eval(
                        &prepare(
                            &column.expr,
                            recipe,
                            &sources,
                            &matches,
                            &output.anchor,
                            row_index,
                        )?,
                        recipe,
                        &mut resolver,
                    )?,
                );
            }
            for aggregate in &output.aggregates {
                values.insert(
                    format!("__aggregate_{}", aggregate.name),
                    crate::eval::eval(
                        &prepare(
                            &aggregate.expr,
                            recipe,
                            &sources,
                            &matches,
                            &output.anchor,
                            row_index,
                        )?,
                        recipe,
                        &mut resolver,
                    )?,
                );
            }
            if let Some(predicate) = &output.filter {
                if !matches!(
                    crate::eval::eval(predicate, recipe, &mut resolver)?,
                    Value::Boolean { value: true }
                ) {
                    continue;
                }
            }
            let is_unresolved = !reasons.is_empty();
            if is_unresolved {
                unresolved += 1;
            }
            values.insert(
                "_status".into(),
                Value::String {
                    value: if is_unresolved {
                        reasons.join(",")
                    } else {
                        "matched".into()
                    },
                },
            );
            evidence.push(serde_json::json!({"output":output.id,"source":row.reference,"original":row.raw,"relationships":recipe.relationships.iter().filter(|r|r.from==output.anchor).map(|r|serde_json::json!({"relationship":r.id,"candidate_count":matches[&r.id][row_index].len(),"candidates":matches[&r.id][row_index].iter().map(|i|&sources[&r.to][*i]).collect::<Vec<_>>()})).collect::<Vec<_>>()}));
            table.push(values);
        }
        if !output.aggregates.is_empty() {
            let mut groups: BTreeMap<String, Vec<BTreeMap<String, Value>>> = BTreeMap::new();
            for row in table {
                let key = serde_json::to_string(
                    &output
                        .group_by
                        .iter()
                        .map(|k| row.get(k).cloned().unwrap_or(Value::Null))
                        .collect::<Vec<_>>(),
                )
                .map_err(engine_error)?;
                groups.entry(key).or_default().push(row);
            }
            table = vec![];
            for rows in groups.values() {
                let mut result = BTreeMap::new();
                for key in &output.group_by {
                    result.insert(key.clone(), rows[0][key].clone());
                }
                for aggregate in &output.aggregates {
                    let values = rows
                        .iter()
                        .map(|r| r[&format!("__aggregate_{}", aggregate.name)].clone())
                        .collect::<Vec<_>>();
                    result.insert(
                        aggregate.name.clone(),
                        reduce(&values, &aggregate.function)?,
                    );
                }
                result.insert(
                    "_status".into(),
                    Value::String {
                        value: if rows.iter().any(|r| {
                            r["_status"]
                                != (Value::String {
                                    value: "matched".into(),
                                })
                        }) {
                            "unresolved"
                        } else {
                            "matched"
                        }
                        .into(),
                    },
                );
                table.push(result);
            }
        }
        tables.insert(output.id.clone(), table);
    }
    let bytes = serde_json::to_vec(&(&tables, &evidence))
        .map_err(engine_error)?
        .len();
    if bytes > policy.limits.output_bytes || bytes > policy.limits.retained_bytes {
        return Err(Error::new(
            "RESOURCE_LIMIT",
            "delivery",
            "result/evidence budget exceeded",
        ));
    }
    Ok((
        tables,
        evidence,
        unresolved,
        sources.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
    ))
}
impl ExecutionContext {
    pub fn run_bundle(
        &self,
        recipe: &Recipe,
        bindings: &Bindings,
        policy: &Policy,
        destination: &std::path::Path,
        cancel: &AtomicBool,
    ) -> RunReport {
        let mut report = self.run(recipe, bindings, policy, cancel);
        if report.status == "failed" || report.status == "cancelled" {
            return report;
        }
        let mut schema = serde_json::Map::new();
        for output in &recipe.outputs {
            let mut columns: Vec<_> = output.columns.iter().filter(|c|output.aggregates.is_empty()||output.group_by.contains(&c.name)).map(|c|serde_json::json!({"name":c.name,"type":crate::validation::expression_type(&c.expr,recipe).ok().flatten()})).collect();
            for aggregate in &output.aggregates {
                let mut ty = crate::validation::expression_type(&aggregate.expr, recipe)
                    .ok()
                    .flatten();
                if aggregate.function.starts_with("count_") {
                    ty = Some(Type::Int64);
                } else if aggregate.function == "sum" {
                    if let Some(Type::Decimal { scale, .. }) = ty {
                        ty = Some(Type::Decimal {
                            precision: 38,
                            scale,
                        });
                    }
                }
                columns.push(serde_json::json!({"name":aggregate.name,"type":ty}));
            }
            columns.push(serde_json::json!({"name":"_status","type":{"kind":"string"}}));
            schema.insert(output.id.clone(), serde_json::json!({"columns":columns}));
        }
        let evidence_table = report
            .evidence
            .iter()
            .map(|record| {
                BTreeMap::from([(
                    "record_json".into(),
                    Value::String {
                        value: record.to_string(),
                    },
                )])
            })
            .collect();
        report.tables.insert("_evidence".into(), evidence_table);
        schema.insert(
            "_evidence".into(),
            serde_json::json!({"columns":[{"name":"record_json","type":{"kind":"string"}}]}),
        );
        match crate::sink::publish_bundle(
            destination,
            policy,
            &report.tables,
            &serde_json::Value::Object(schema),
            &report.manifest,
            cancel,
        ) {
            Ok(bytes) => {
                report.manifest["bytes_written"] = serde_json::json!(bytes);
                report.manifest["bundle"] = serde_json::json!(destination);
            }
            Err(error) => {
                report.status = if error.code == "CANCELLED" {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                report.tables.clear();
                report.evidence.clear();
                report.manifest["complete"] = serde_json::json!(false);
                report.diagnostics.push(error);
            }
        }
        report
    }
}
fn reduce(values: &[Value], function: &str) -> Result<Value> {
    if function == "count_records" {
        return Ok(Value::Int64 {
            value: values.len().to_string(),
        });
    }
    let values: Vec<_> = values.iter().filter(|v| **v != Value::Null).collect();
    if function == "count_non_null" {
        return Ok(Value::Int64 {
            value: values.len().to_string(),
        });
    }
    let Some(first) = values.first() else {
        return Ok(Value::Null);
    };
    match function {
        "min" | "max" => {
            let mut chosen = (*first).clone();
            for value in values.into_iter().skip(1) {
                let cmp = crate::eval::compare(value, &chosen)?;
                if (function == "min" && cmp == Some(std::cmp::Ordering::Less))
                    || (function == "max" && cmp == Some(std::cmp::Ordering::Greater))
                {
                    chosen = value.clone();
                }
            }
            Ok(chosen)
        }
        "sum" => match first {
            Value::Int64 { .. } => {
                let mut sum = 0i64;
                for value in values {
                    if let Value::Int64 { value } = value {
                        sum = sum
                            .checked_add(value.parse().map_err(engine_error)?)
                            .ok_or_else(|| {
                                Error::new("ARITHMETIC_OVERFLOW", "aggregate", "Int64 sum")
                            })?;
                    } else {
                        return Err(Error::new("TYPE_MISMATCH", "aggregate", "mixed sum types"));
                    }
                }
                Ok(Value::Int64 {
                    value: sum.to_string(),
                })
            }
            Value::Decimal { scale, .. } => {
                let scale = *scale;
                let mut sum = num_bigint::BigInt::from(0);
                for value in values {
                    if let Value::Decimal {
                        value,
                        precision,
                        scale: s,
                    } = value
                    {
                        if *s != scale {
                            return Err(Error::new(
                                "TYPE_MISMATCH",
                                "aggregate",
                                "mixed sum scales",
                            ));
                        }
                        let d = crate::scalar::Decimal::parse(value, *precision, *s)
                            .map_err(engine_error)?;
                        sum += d.coefficient;
                    } else {
                        return Err(Error::new("TYPE_MISMATCH", "aggregate", "mixed sum types"));
                    }
                }
                use num_traits::ToPrimitive;
                let coefficient = sum
                    .to_i128()
                    .ok_or_else(|| Error::new("ARITHMETIC_OVERFLOW", "aggregate", "decimal sum"))?;
                if coefficient.unsigned_abs() >= 10u128.pow(38) {
                    return Err(Error::new(
                        "ARITHMETIC_OVERFLOW",
                        "aggregate",
                        "decimal sum precision",
                    ));
                }
                let d = crate::scalar::Decimal {
                    coefficient,
                    precision: 38,
                    scale,
                };
                Ok(Value::Decimal {
                    value: d.to_string(),
                    precision: 38,
                    scale,
                })
            }
            _ => Err(Error::new(
                "TYPE_MISMATCH",
                "aggregate",
                "sum requires numeric values",
            )),
        },
        _ => Err(Error::new(
            "UNSUPPORTED_OPERATION",
            "aggregate",
            function.to_string(),
        )),
    }
}
fn prepare(
    expr: &Expr,
    recipe: &Recipe,
    sources: &BTreeMap<String, Vec<Row>>,
    matches: &Matches,
    anchor: &str,
    row_index: usize,
) -> Result<Expr> {
    let mut value = serde_json::to_value(expr).map_err(engine_error)?;
    fn visit(
        value: &mut serde_json::Value,
        recipe: &Recipe,
        sources: &BTreeMap<String, Vec<Row>>,
        matches: &Matches,
        anchor: &str,
        row_index: usize,
    ) -> Result<()> {
        if value.get("op").and_then(|x| x.as_str()) == Some("aggregate") {
            let expr: Expr = serde_json::from_value(value.clone()).map_err(engine_error)?;
            if let Expr::Aggregate {
                relationship,
                function,
                field,
                filter,
            } = expr
            {
                let rel = recipe
                    .relationships
                    .iter()
                    .find(|r| r.id == relationship)
                    .unwrap();
                if rel.from != anchor {
                    return Err(Error::new(
                        "INVALID_PATH",
                        "aggregate",
                        "aggregate must start at anchor",
                    ));
                }
                let mut values = vec![];
                for candidate in &matches[&relationship][row_index] {
                    let row = &sources[&rel.to][*candidate];
                    if let Some(p) = &filter {
                        let v = crate::eval::eval(p, recipe, &mut |source, f, path| {
                            if source != rel.to || !path.is_empty() {
                                return Err(Error::new(
                                    "INVALID_PATH",
                                    "aggregate",
                                    "aggregate filter must refer to detail source",
                                ));
                            }
                            Ok(row.fields.get(f).cloned().unwrap_or(Value::Null))
                        })?;
                        if v != (Value::Boolean { value: true }) {
                            continue;
                        }
                    }
                    values.push(
                        field
                            .as_ref()
                            .and_then(|f| row.fields.get(f))
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                *value = serde_json::to_value(Expr::Literal {
                    value: reduce(&values, &function)?,
                })
                .map_err(engine_error)?;
                return Ok(());
            }
        }
        if value.get("op").and_then(|x| x.as_str()) == Some("feature") {
            let id = value["id"].as_str().unwrap();
            let feature = recipe.features.iter().find(|f| f.id == id).unwrap();
            *value = serde_json::to_value(&feature.expr).map_err(engine_error)?;
            return visit(value, recipe, sources, matches, anchor, row_index);
        }
        match value {
            serde_json::Value::Object(map) => {
                for child in map.values_mut() {
                    visit(child, recipe, sources, matches, anchor, row_index)?;
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    visit(child, recipe, sources, matches, anchor, row_index)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(&mut value, recipe, sources, matches, anchor, row_index)?;
    serde_json::from_value(value).map_err(engine_error)
}
