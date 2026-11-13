//! SQLite source reader (docs/spec/slice-3.md, "Contract: SQLite source").
//!
//! The database file is read once under the read policy and byte limits, like
//! every other source, and the copy is queried read-only in memory. The
//! fingerprint therefore identifies exactly the data the queries saw.
//!
//! Values are converted by their own storage class and never through a float.
//! A pushed eligibility predicate (`crate::placement`) is evaluated by SQLite
//! during the scan, after source-side checks over every record of the table.
use super::*;
use crate::placement::{Pushed, SqlValue, column, quoted};
use rusqlite::{Connection, config::DbConfig, types::ValueRef};

/// What one scan of a SQLite table produced.
pub(super) struct Scan {
    pub rows: Vec<Row>,
    /// Records in the table, whether or not they were scanned.
    pub total_rows: usize,
    /// Placement record for the manifest, when a predicate was planned for this source.
    pub placement: Option<serde_json::Value>,
}

fn sql_error(e: rusqlite::Error) -> Error {
    if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        err("CANCELLED", "SQLite query cancelled")
    } else {
        err("INVALID_SQLITE", e.to_string())
    }
}

/// Keep the borrowed cancellation flag alive until SQLite's callback is removed.
/// The connection executes synchronously and is never shared across threads.
struct QueryCancellation<'a> {
    conn: &'a Connection,
    _cancel: &'a AtomicBool,
}
impl<'a> QueryCancellation<'a> {
    fn install(conn: &'a Connection, cancel: &'a AtomicBool) -> Self {
        unsafe extern "C" fn cancelled(data: *mut std::ffi::c_void) -> std::ffi::c_int {
            // SAFETY: install borrows the flag for the guard's lifetime; Drop
            // removes the callback before either borrow can end.
            unsafe { (&*data.cast::<AtomicBool>()).load(Ordering::Relaxed) as _ }
        }
        // SAFETY: the callback does not unwind, and the flag remains borrowed
        // until the callback is removed in Drop. These guards are not nested.
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                conn.handle(),
                1000,
                Some(cancelled),
                std::ptr::from_ref(cancel).cast_mut().cast(),
            );
        }
        Self {
            conn,
            _cancel: cancel,
        }
    }
}
impl Drop for QueryCancellation<'_> {
    fn drop(&mut self) {
        // SAFETY: the connection is still borrowed and no query outlives this guard.
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                self.conn.handle(),
                0,
                None,
                std::ptr::null_mut(),
            );
        }
    }
}

/// Open the bytes read as a read-only in-memory database.
pub(super) fn open(bytes: &[u8]) -> Result<Connection> {
    if bytes.is_empty() {
        return Err(err("INVALID_SQLITE", "the database file is empty"));
    }
    let mut conn = Connection::open_in_memory().map_err(sql_error)?;
    conn.deserialize_read_exact(rusqlite::MAIN_DB, bytes, bytes.len(), true)
        .map_err(sql_error)?;
    // The file is untrusted input: its schema may not run functions or virtual
    // tables with side effects.
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(sql_error)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)
        .map_err(sql_error)?;
    Ok(conn)
}

/// The bound table, its rowid alias and the column of each declared field.
struct Table<'a> {
    name: String,
    rowid: &'static str,
    columns: Vec<(&'a String, &'a Field, &'a str)>,
}

fn table<'a>(conn: &Connection, source: &'a Source, binding: &Binding) -> Result<Table<'a>> {
    let name = match binding.settings.get("table") {
        Some(serde_json::Value::String(name)) if !name.is_empty() => name.clone(),
        _ => {
            return Err(err(
                "INVALID_BINDING",
                "a sqlite binding names one table in settings.table",
            ));
        }
    };
    if source.projection.array.is_some() || source.projection.fields.values().any(|f| f.header) {
        return Err(err(
            "INVALID_PROJECTION",
            "a sqlite projection reads table columns; it declares no array and no header fields",
        ));
    }
    let mut found = conn
        .prepare("SELECT type, wr FROM pragma_table_list WHERE schema = 'main' AND name = ?1")
        .map_err(sql_error)?;
    let kind: Option<(String, i64)> = found
        .query_map([&name], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(sql_error)?
        .next()
        .transpose()
        .map_err(sql_error)?;
    match kind {
        None => {
            return Err(err(
                "INVALID_BINDING",
                format!("declared SQLite table {name} does not exist"),
            ));
        }
        Some((kind, without_rowid)) if kind != "table" || without_rowid != 0 => {
            return Err(err(
                "INVALID_BINDING",
                format!(
                    "{name} is not an ordinary rowid table; views, virtual tables and WITHOUT ROWID tables are not supported"
                ),
            ));
        }
        Some(_) => {}
    }
    let mut info = conn
        .prepare("SELECT name FROM pragma_table_xinfo(?1)")
        .map_err(sql_error)?;
    let present: BTreeSet<String> = info
        .query_map([&name], |r| r.get(0))
        .map_err(sql_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql_error)?;
    let columns = source
        .projection
        .fields
        .iter()
        .map(|(field, declared)| {
            let column = column(declared);
            if present.contains(column) {
                Ok((field, declared, column))
            } else {
                Err(err(
                    "INVALID_PROJECTION",
                    format!("declared SQLite column {column} is not present in the source"),
                ))
            }
        })
        .collect::<Result<_>>()?;
    let rowid = ["rowid", "_rowid_", "oid"]
        .into_iter()
        .find(|alias| !present.iter().any(|c| c.eq_ignore_ascii_case(alias)))
        .ok_or_else(|| {
            err(
                "INVALID_BINDING",
                format!("{name} hides every rowid alias behind a column"),
            )
        })?;
    Ok(Table {
        name,
        rowid,
        columns,
    })
}

/// Bound validation's schema inspection: the table and each declared column
/// exist. No record is read.
pub(super) fn inspect(
    source: &Source,
    binding: &Binding,
    bytes: &[u8],
    required: Option<&Pushed>,
) -> Result<()> {
    let conn = open(bytes)?;
    inspect_connection(source, binding, &conn, required)
}
pub(super) fn inspect_connection(
    source: &Source,
    binding: &Binding,
    conn: &Connection,
    required: Option<&Pushed>,
) -> Result<()> {
    table(conn, source, binding)?;
    if let Some(pushed) = required {
        let encoding: String = conn
            .query_row("PRAGMA encoding", [], |r| r.get(0))
            .map_err(sql_error)?;
        if encoding != "UTF-8" {
            return Err(Error::new("PLACEMENT_INFEASIBLE", "source", format!("policy placement require_source cannot be met for {}: the database stores text as {encoding}", pushed.stage)).with_subject(pushed.stage.clone()).with_remediation("Use placement prefer_source or local; the policy is not overridden."));
        }
    }
    Ok(())
}

/// The SQL storage classes a declared type accepts, as `typeof` names them.
fn storage_classes(field: &Field) -> Vec<&'static str> {
    let mut classes = match field.data_type {
        Type::Int64 | Type::Boolean => vec!["integer"],
        Type::String | Type::Date => vec!["text"],
        Type::Decimal { .. } => vec!["text", "integer"],
    };
    if field.nullable {
        classes.push("null");
    }
    classes
}

/// Checks a pushed scan would otherwise skip for the records it does not read:
/// identity presence and uniqueness, nullability and storage classes, over
/// every record of the table. Returns why the scan must stay local, if it must.
#[derive(Clone)]
pub(super) enum Fallback {
    Encoding(String),
    Data(String),
}
impl Fallback {
    pub(super) fn is_encoding(&self) -> bool {
        matches!(self, Self::Encoding(_))
    }
    pub(super) fn reason(&self) -> &str {
        match self {
            Self::Encoding(reason) | Self::Data(reason) => reason,
        }
    }
}
impl std::fmt::Display for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

fn source_checks(
    conn: &Connection,
    source: &Source,
    table: &Table<'_>,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<Option<Fallback>> {
    let encoding: String = conn
        .query_row("PRAGMA encoding", [], |r| r.get(0))
        .map_err(sql_error)?;
    if encoding != "UTF-8" {
        return Ok(Some(Fallback::Encoding(format!(
            "the database stores text as {encoding}; bytewise text comparison at the source would not match the local executor's"
        ))));
    }
    let name = quoted(&table.name);
    let mut violations = vec![];
    let mut identity = vec![];
    for (field, declared, column) in &table.columns {
        let column = quoted(column);
        let classes = storage_classes(declared)
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(", ");
        violations.push(format!("typeof({column}) NOT IN ({classes})"));
        violations.push(format!(
            "(typeof({column}) = 'text' AND length(CAST({column} AS BLOB)) > {})",
            limits.field_bytes
        ));
        if declared.data_type == Type::Boolean {
            violations.push(format!(
                "({column} IS NOT NULL AND +{column} COLLATE BINARY NOT IN (0, 1))"
            ));
        }
        if source.identity.contains(*field) {
            violations.push(format!("{column} IS NULL"));
            identity.push(format!("+{column} COLLATE BINARY"));
        }
    }
    let violating: i64 = conn
        .query_row(
            &format!(
                "SELECT count(*) FROM {name} WHERE {}",
                violations.join(" OR ")
            ),
            [],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if violating > 0 {
        return Ok(Some(Fallback::Data(format!(
            "{violating} records fail a source-side identity, nullability, field_bytes or storage-class check; the whole table is read locally to diagnose them"
        ))));
    }
    // A from table may have a decimal identity even though reduction keys and
    // target identities cannot. Check that identity with the executor's exact
    // conversion rather than grouping decimal spellings in SQLite.
    if source
        .identity
        .iter()
        .any(|id| matches!(source.projection.fields[id].data_type, Type::Decimal { .. }))
    {
        let selected = source
            .identity
            .iter()
            .map(|id| quoted(column(&source.projection.fields[id])))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = conn
            .prepare(&format!("SELECT {selected} FROM {name}"))
            .map_err(sql_error)?;
        let mut records = statement.query([]).map_err(sql_error)?;
        let mut seen = BTreeSet::new();
        while let Some(record) = records.next().map_err(sql_error)? {
            check_cancel(cancel)?;
            let mut key = Vec::new();
            for (i, id) in source.identity.iter().enumerate() {
                let (value, _) = convert(
                    record.get_ref(i).map_err(sql_error)?,
                    &source.projection.fields[id],
                    limits,
                )?;
                match value {
                    Ok(value) => key.push(value),
                    Err(_) => {
                        return Ok(Some(Fallback::Data(
                            "identity requires local value diagnostics".into(),
                        )));
                    }
                }
            }
            if !seen.insert(serde_json::to_string(&key).map_err(io_error)?) {
                return Ok(Some(Fallback::Data(
                    "records share an identity; the whole table is read locally to diagnose them"
                        .into(),
                )));
            }
        }
        return Ok(None);
    }
    if !identity.is_empty() {
        let mut duplicated = conn
            .prepare(&format!(
                "SELECT 1 FROM {name} GROUP BY {} HAVING count(*) > 1 LIMIT 1",
                identity.join(", ")
            ))
            .map_err(sql_error)?;
        if duplicated.exists([]).map_err(sql_error)? {
            return Ok(Some(Fallback::Data(
                "records share an identity; the whole table is read locally to diagnose them"
                    .into(),
            )));
        }
    }
    Ok(None)
}

/// Convert one stored value by its own storage class. The outer error is
/// structural (a budget); the inner one is a value failure of this record.
fn convert(
    value: ValueRef<'_>,
    field: &Field,
    limits: &Limits,
) -> Result<(Result<Value>, serde_json::Value)> {
    let bad = || {
        err(
            "INVALID_VALUE",
            "source value does not satisfy declared type",
        )
    };
    Ok(match value {
        ValueRef::Null => (value_from_text(None, field), serde_json::Value::Null),
        ValueRef::Integer(n) => {
            let converted = match field.data_type {
                Type::Int64 | Type::Decimal { .. } => value_from_text(Some(&n.to_string()), field),
                Type::Boolean if n == 0 || n == 1 => Ok(Value::Boolean { value: n == 1 }),
                _ => Err(bad()),
            };
            (converted, serde_json::Value::Number(n.into()))
        }
        ValueRef::Text(bytes) => {
            limit(
                bytes.len() <= limits.field_bytes,
                "single SQLite text value bytes",
            )?;
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    let converted = match field.data_type {
                        Type::String | Type::Date | Type::Decimal { .. } => {
                            value_from_text(Some(text), field)
                        }
                        _ => Err(bad()),
                    };
                    (converted, serde_json::Value::String(text.into()))
                }
                Err(_) => (
                    Err(bad()),
                    serde_json::Value::String("<invalid text>".into()),
                ),
            }
        }
        // REAL has no exact decimal value, so it is never converted.
        ValueRef::Real(_) => (Err(bad()), serde_json::Value::String("<real>".into())),
        ValueRef::Blob(_) => (Err(bad()), serde_json::Value::String("<blob>".into())),
    })
}

/// Read the bound table as typed, referenced records in ascending rowid order.
/// With `pushed`, SQLite evaluates the predicate and only passing records cross
/// into the executor, provided the source-side checks pass over every record.
pub(super) fn read(
    source: &Source,
    binding: &Binding,
    conn: &Connection,
    reference: &SourceReference,
    policy: &Policy,
    cancel: &AtomicBool,
    pushed: Option<&Pushed>,
    group_fallback: Option<&Fallback>,
) -> Result<Scan> {
    let _cancellation = QueryCancellation::install(conn, cancel);
    let table = table(&conn, source, binding)?;
    let name = quoted(&table.name);
    let fallback = match group_fallback {
        Some(reason) => Some(reason.clone()),
        None => match pushed {
            Some(_) => source_checks(&conn, source, &table, &policy.limits, cancel)?,
            None => None,
        },
    };
    if let (Some(reason), Placement::RequireSource, Some(pushed)) =
        (&fallback, policy.placement, pushed)
    {
        // Check failures are diagnosed by the local read below; an encoding the
        // source cannot compare in is a placement the policy forbids.
        if reason.is_encoding() {
            return Err(Error::new(
                "PLACEMENT_INFEASIBLE",
                "source",
                format!(
                    "policy placement require_source cannot be met for {}: {reason}",
                    pushed.stage
                ),
            )
            .with_subject(pushed.stage.clone())
            .with_remediation(
                "Use placement prefer_source or local; the policy is not overridden.",
            ));
        }
    }
    let filter = pushed.filter(|_| fallback.is_none());
    check_cancel(cancel)?;
    let total_rows: i64 = conn
        .query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))
        .map_err(sql_error)?;
    let selected = table
        .columns
        .iter()
        .map(|(_, _, column)| quoted(column))
        .collect::<Vec<_>>()
        .join(", ");
    let condition = filter
        .map(|p| format!(" WHERE {}", p.sql))
        .unwrap_or_default();
    let rowid = table.rowid;
    let mut statement = conn
        .prepare(&format!(
            "SELECT {rowid}, {selected} FROM {name} AS iresiphi_target{condition} ORDER BY {rowid}"
        ))
        .map_err(sql_error)?;
    let parameters = filter
        .map(|p| p.parameters.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|p| match p {
            SqlValue::Integer(n) => rusqlite::types::Value::Integer(*n),
            SqlValue::Text(text) => rusqlite::types::Value::Text(text.clone()),
        });
    let mut records = statement
        .query(rusqlite::params_from_iter(parameters))
        .map_err(sql_error)?;
    let mut rows = RowCollector::new(policy);
    let mut failures = ValueFailures::default();
    while let Some(record) = records.next().map_err(sql_error)? {
        check_cancel(cancel)?;
        let id: i64 = record.get(0).map_err(sql_error)?;
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
        row.reference.location = format!("rowid:{id}");
        let mut failure = None;
        for (i, (field, declared, _)) in table.columns.iter().enumerate() {
            let stored = record.get_ref(i + 1).map_err(sql_error)?;
            let (converted, raw) = convert(stored, declared, &policy.limits)?;
            row.raw.insert((*field).clone(), raw);
            match converted {
                Ok(value) => {
                    row.fields.insert((*field).clone(), value);
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        match failure {
            None => rows.push(row)?,
            Some(error) => failures.record(error, &row.reference)?,
        }
    }
    failures.finish()?;
    let placement = pushed.map(|p| match &fallback {
        None => serde_json::json!({
            "scan": "source",
            "operation": p.stage,
            "checks": {
                "identity": "full: every record, at the source",
                "nullability": "full: every record, at the source",
                "storage_classes": "full: every record, at the source",
                "field_bytes": "full: every record, at the source",
                "boolean_values": "full: every record, at the source",
                "text_formats": "date text, decimal text, and valid UTF-8: scanned records only",
                "lookups": if p.stage.starts_with("relationship:") { "full: every delivered from record; target reduction preserves all candidates" } else { "scanned records only" },
            },
        }),
        Some(reason) => serde_json::json!({
            "scan": "local",
            "operation": p.stage,
            "reason": reason.reason(),
        }),
    });
    Ok(Scan {
        rows: rows.rows,
        total_rows: usize::try_from(total_rows).unwrap_or(usize::MAX),
        placement,
    })
}

/// Preflight without delivering or charging decoded records.
pub(super) fn preflight(
    conn: &Connection,
    source: &Source,
    binding: &Binding,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<Option<Fallback>> {
    let _cancellation = QueryCancellation::install(conn, cancel);
    let table = table(conn, source, binding)?;
    source_checks(conn, source, &table, &policy.limits, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_interrupts_a_query_before_it_delivers_a_record() {
        let conn = Connection::open_in_memory().unwrap();
        let cancel = AtomicBool::new(true);
        let query = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT sum(x) FROM n";
        {
            let _guard = QueryCancellation::install(&conn, &cancel);
            let error = conn
                .query_row(query, [], |row| row.get::<_, i64>(0))
                .unwrap_err();
            assert_eq!(sql_error(error).code, "CANCELLED");
        }
        // The callback must not retain its borrowed flag after the guard ends.
        assert_eq!(
            conn.query_row(query, [], |row| row.get::<_, i64>(0))
                .unwrap(),
            50_005_000
        );
    }

    #[test]
    fn decimal_identity_preflight_observes_cancellation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id TEXT); INSERT INTO items VALUES ('1.00'), ('2.00')",
        )
        .unwrap();
        let source: Source = serde_json::from_value(serde_json::json!({
            "id":"items", "identity":["id"], "grain":["id"],
            "projection":{"fields":{"id":{"path":"id","data_type":{"kind":"decimal","precision":10,"scale":2}}}}
        })).unwrap();
        let binding: Binding = serde_json::from_value(serde_json::json!({
            "adapter":"sqlite", "path":"unused.sqlite", "settings":{"table":"items"}
        }))
        .unwrap();
        let table = table(&conn, &source, &binding).unwrap();
        match source_checks(
            &conn,
            &source,
            &table,
            &Limits::default(),
            &AtomicBool::new(true),
        ) {
            Err(error) => assert_eq!(error.code, "CANCELLED"),
            Ok(_) => panic!("decimal identity scan ignored cancellation"),
        }
    }
}
