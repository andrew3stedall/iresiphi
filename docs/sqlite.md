# SQLite sources and source placement

A source can be a table in a SQLite database file, and an output's eligibility predicate can run inside SQLite instead of in the local executor. The recipe does not change for either: the adapter is named in the binding, and placement is set in the policy. The normative contract is [docs/spec/slice-3.md](spec/slice-3.md). This page describes what the library does.

## Binding a table

```python
import iresiphi
from iresiphi import build as b

bindings = {
    "orders": b.binding("/data/sales.db", "sqlite", table="orders"),
    "customers": b.binding("/data/customers.csv", "csv"),
}
```

`settings.table` names one ordinary table, matched exactly. Views, virtual tables, `WITHOUT ROWID` tables and SQL text are rejected with `INVALID_BINDING`. A field's `path` is its column name. A missing column fails with `INVALID_PROJECTION`, and bound validation finds it without reading a record.

The run reads the file once, under the same read roots and byte limits as any other source, and queries that copy read-only in memory. The fingerprint in the manifest is therefore the fingerprint of exactly what the query saw. A file that is not a database fails with `INVALID_SQLITE`. So does a database in WAL mode, which SQLite cannot open from a copy; switch it to a rollback journal first.

Each record's reference location is `rowid:<n>`, and records are read in ascending rowid order.

## Types

SQLite stores each value with its own storage class, whatever the column was declared as. The reader converts by that class and never through a float.

| Declared type | Accepted storage |
| --- | --- |
| `int64` | INTEGER |
| `boolean` | INTEGER 0 or 1 |
| `string` | TEXT |
| `date` | TEXT as `YYYY-MM-DD` |
| `decimal` | TEXT holding a decimal number, or INTEGER |

REAL is never accepted, including for `decimal`: a float has no exact decimal value. Store amounts as text or as integers. NULL is null for a nullable field and `NULL_REQUIRED` otherwise. Anything else is `INVALID_VALUE`. Failures are counted across the whole scan and reported with record references.

## Placement

The policy's `placement` says where operations may run.

| Value | Meaning |
| --- | --- |
| `local` (default) | Everything runs in the local executor. Results are what they were before this setting existed. |
| `prefer_source` | An eligibility predicate runs at its source when the rules below allow it, and locally otherwise. |
| `require_source` | An eligibility predicate on a source that can filter must run there. If it cannot, validate, explain and run fail with `PLACEMENT_INFEASIBLE` before any source is opened. |

```python
policy = b.policy(["/data"], placement="prefer_source")
print(recipe.explain(bindings, policy)["bound"]["placement"])
```

Only one operation can be placed at a source: an output's anchor `eligibility`, as a filter on the anchor source's scan. It is pushed when all of these hold:

- The anchor is bound to `sqlite`.
- The source has no other use in the recipe. It is not a relationship target, another output's anchor, a detail output's parent, a population's recipient or period source, or an allocation's quantity source. Those need the records the filter would drop.
- The predicate reads only the anchor's own fields and literals, through `equal`, `less`, `less_equal`, `and`, `or`, `not` and `is_null`.
- Every compared value is `int64`, `string`, `date` or `boolean`. SQLite compares these as the local executor does: integers numerically, text and ISO dates bytewise, and a null comparison is unknown. The generated SQL removes each column's declared affinity and collation, so a `NOCASE` column still compares bytewise.
- No comparison involves a `decimal`, and the source's identity has no `decimal` field. SQLite would compare decimal text bytewise (`"120.50" < "50.00"`) or through a float.

Explain lists each eligibility with its placement and, when it stays local, the reason. It opens no source.

## What a pushed predicate changes

Output rows, values, evidence records and output counts are the same under either placement. `tests/sqlite.rs` runs 22 predicates both ways and compares them. Three things differ, and the manifest states each:

- **Records scanned.** `sources.<id>.rows` is still the table's record count. `scanned_rows` is how many crossed into the executor. Row limits apply to scanned records, so a table larger than `limits.anchor_rows` can run when its eligible records fit.
- **Text format checks.** Decimal and date text is parsed for scanned records only.
- **Lookup checks.** Unmatched, ambiguous and strict-cardinality checks cover scanned anchor records only. A closed order with an unknown customer raises `LOOKUP_UNMATCHED` under local placement and nothing under source placement.

Before a pushed scan, SQLite checks every record of the table: identity present and unique, required columns not null, and each value's storage class acceptable for its declared type. If any record fails, the run reads the whole table locally and reports the same diagnostic local placement would. The same happens for a database whose text encoding is not UTF-8.

Bytes read do not change. The whole file is read and fingerprinted under either placement, so this slice saves decoding and row budget, not I/O.

## Not included

Remote databases, credentials, SQL bindings, pushdown of aggregation, joins or output filters, transfers and staging are deferred.
