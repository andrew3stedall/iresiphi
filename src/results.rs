//! Where a run's rows go as they are produced (docs/spec/slice-7.md, phase 2).
//!
//! A memory run keeps them ([`Retained`]); a bundle run writes them to its staging
//! directory and keeps nothing ([`Bundled`]). [`Results`] sits in front of either and
//! counts what passes: rows per table, evidence records per kind, and the exact
//! serialized size the memory result has, so budgets and the manifest are the same
//! whichever destination a run has.
use crate::{
    admission::serialized_size,
    engine::Table,
    model::{Error, Policy, Result, Value},
    sink::Staged,
};
use serde_json::json;
use std::collections::BTreeMap;

/// The evidence tables, in the order of the memory result's evidence fields.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Record {
    Primary,
    Candidate,
    Detail,
    Allocation,
}
impl Record {
    pub(crate) const ALL: [Record; 4] = [
        Record::Primary,
        Record::Candidate,
        Record::Detail,
        Record::Allocation,
    ];
    pub(crate) fn table(self) -> &'static str {
        match self {
            Record::Primary => "_evidence",
            Record::Candidate => "_candidate_evidence",
            Record::Detail => "_detail_evidence",
            Record::Allocation => "_allocation_evidence",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

/// A destination for output rows and evidence records.
pub(crate) trait Rows {
    /// Whether rows are kept in memory, and so count against `limits.retained_bytes`.
    fn retains(&self) -> bool;
    /// Output production begins. A bundle creates its staging here.
    fn start(&mut self) -> Result<()>;
    /// An output table begins; it exists even if no row follows.
    fn begin(&mut self, table: &str) -> Result<()>;
    fn row(&mut self, table: &str, row: BTreeMap<String, Value>) -> Result<()>;
    fn record(&mut self, kind: Record, record: serde_json::Value) -> Result<()>;
}

/// Rows kept in memory for the caller: what `collect()` and `ExecutionContext::run`
/// return.
#[derive(Default)]
pub(crate) struct Retained {
    pub(crate) tables: BTreeMap<String, Table>,
    pub(crate) records: [Vec<serde_json::Value>; 4],
}
impl Rows for Retained {
    fn retains(&self) -> bool {
        true
    }
    fn start(&mut self) -> Result<()> {
        Ok(())
    }
    fn begin(&mut self, table: &str) -> Result<()> {
        self.tables.entry(table.to_owned()).or_default();
        Ok(())
    }
    fn row(&mut self, table: &str, row: BTreeMap<String, Value>) -> Result<()> {
        self.tables.entry(table.to_owned()).or_default().push(row);
        Ok(())
    }
    fn record(&mut self, kind: Record, record: serde_json::Value) -> Result<()> {
        self.records[kind.index()].push(record);
        Ok(())
    }
}

/// The schema of an evidence table: one JSON record per row.
pub(crate) fn record_schema() -> serde_json::Value {
    json!({"columns":[{"name":"record_json","type":{"kind":"string"}}]})
}

/// Rows written straight to a bundle's staging directory and not kept.
pub(crate) struct Bundled<'a> {
    pub(crate) staged: Staged<'a>,
    schema: serde_json::Value,
}
impl<'a> Bundled<'a> {
    pub(crate) fn new(staged: Staged<'a>, schema: serde_json::Value) -> Self {
        Self { staged, schema }
    }
}
impl Rows for Bundled<'_> {
    fn retains(&self) -> bool {
        false
    }
    fn start(&mut self) -> Result<()> {
        self.staged.start()?;
        // Primary, candidate and detail evidence tables are always in a bundle;
        // allocation evidence only when a record exists.
        for kind in [Record::Primary, Record::Candidate, Record::Detail] {
            self.staged
                .begin(kind.table(), vec!["record_json".into()])?;
        }
        Ok(())
    }
    fn begin(&mut self, table: &str) -> Result<()> {
        let columns = crate::sink::schema_columns(&self.schema, table)?;
        self.staged.begin(table, columns)
    }
    fn row(&mut self, table: &str, row: BTreeMap<String, Value>) -> Result<()> {
        self.staged.row(table, &row)
    }
    fn record(&mut self, kind: Record, record: serde_json::Value) -> Result<()> {
        self.staged
            .begin(kind.table(), vec!["record_json".into()])?;
        let row = BTreeMap::from([(
            "record_json".to_string(),
            Value::String {
                value: record.to_string(),
            },
        )]);
        self.staged.row(kind.table(), &row)
    }
}

/// Count and size of the items of one JSON array.
#[derive(Default, Clone, Copy)]
struct Items {
    count: usize,
    bytes: usize,
}
impl Items {
    /// Serialized size of the array: brackets, items and separating commas.
    fn array(self) -> usize {
        2 + self.bytes + self.count.saturating_sub(1)
    }
}

/// The front of a [`Rows`] destination: forwards every row and record and counts
/// them, failing the run as soon as a declared bound is exceeded.
pub(crate) struct Results<'r> {
    rows: &'r mut dyn Rows,
    tables: BTreeMap<String, Items>,
    records: [Items; 4],
    /// Serialized bytes of every item so far; the exact total adds only structure.
    items: usize,
    limit: usize,
    candidate_pairs: usize,
}
impl<'r> Results<'r> {
    pub(crate) fn new(rows: &'r mut dyn Rows, policy: &Policy) -> Self {
        let limit = if rows.retains() {
            policy.limits.output_bytes.min(policy.limits.retained_bytes)
        } else {
            policy.limits.output_bytes
        };
        Self {
            rows,
            tables: BTreeMap::new(),
            records: [Items::default(); 4],
            items: 0,
            limit,
            candidate_pairs: policy.limits.candidate_pairs,
        }
    }
    pub(crate) fn start(&mut self) -> Result<()> {
        self.rows.start()
    }
    pub(crate) fn begin(&mut self, table: &str) -> Result<()> {
        self.tables.entry(table.to_owned()).or_default();
        self.rows.begin(table)
    }
    fn add(&mut self, bytes: usize) -> Result<()> {
        self.items += bytes;
        if self.items > self.limit {
            return Err(self.over(format!("more than {}", self.limit)));
        }
        Ok(())
    }
    fn over(&self, bytes: String) -> Error {
        // The memory result is retained only if it fits; this is a delivery budget.
        let mut error = crate::engine::evidence_budget(format!(
            "result and evidence tables ({bytes} bytes) exceed the output/retained byte budget"
        ));
        error.phase = "delivery".into();
        error
    }
    pub(crate) fn row(&mut self, table: &str, row: BTreeMap<String, Value>) -> Result<()> {
        let bytes = serialized_size(&row)
            .map_err(|e| Error::new("INTERNAL_ERROR", "execution", e.to_string()))?;
        self.add(bytes)?;
        let items = self.tables.entry(table.to_owned()).or_default();
        items.count += 1;
        items.bytes += bytes;
        self.rows.row(table, row)
    }
    pub(crate) fn record(&mut self, kind: Record, record: serde_json::Value) -> Result<()> {
        let bytes = serialized_size(&record)
            .map_err(|e| Error::new("INTERNAL_ERROR", "execution", e.to_string()))?;
        self.add(bytes)?;
        let items = &mut self.records[kind.index()];
        items.count += 1;
        items.bytes += bytes;
        let pairs = self.count(Record::Candidate) + self.count(Record::Detail);
        if pairs > self.candidate_pairs {
            return Err(crate::engine::evidence_budget(format!(
                "{pairs} candidate and detail evidence records exceed limits.candidate_pairs ({})",
                self.candidate_pairs
            )));
        }
        self.rows.record(kind, record)
    }
    pub(crate) fn count(&self, kind: Record) -> usize {
        self.records[kind.index()].count
    }
    pub(crate) fn rows_in(&self, table: &str) -> usize {
        self.tables.get(table).map_or(0, |items| items.count)
    }
    /// Exact serialized size of the memory result: the tables and the primary,
    /// candidate and detail evidence as one JSON array, plus allocation evidence
    /// when there is any.
    pub(crate) fn bytes(&self) -> usize {
        let tables = 2
            + self.tables.len().saturating_sub(1)
            + self
                .tables
                .iter()
                .map(|(name, items)| {
                    serialized_size(name).unwrap_or(name.len() + 2) + 1 + items.array()
                })
                .sum::<usize>();
        let mut bytes = 2
            + 3
            + tables
            + Record::ALL[..3]
                .iter()
                .map(|kind| self.records[kind.index()].array())
                .sum::<usize>();
        let allocation = self.records[Record::Allocation.index()];
        if allocation.count > 0 {
            bytes += allocation.array();
        }
        bytes
    }
    /// The exact size, once every row has passed; fails if it exceeds the budget.
    pub(crate) fn finish(&self) -> Result<usize> {
        let bytes = self.bytes();
        if bytes > self.limit {
            return Err(self.over(bytes.to_string()));
        }
        Ok(bytes)
    }
}
