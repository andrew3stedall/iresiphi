# SQLite sources and source placement

A source can be a table in a SQLite database file, and an output's eligibility predicate can run inside SQLite instead of in the local executor. The recipe does not change for either: the adapter is named in the binding, and placement is set in the policy. The normative contracts are [slice 3](spec/slice-3.md) and [slice 4](spec/slice-4.md). This page describes what the library does.

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
| `require_source` | An eligibility predicate on a source that can filter must run there. Recipe/binding infeasibility fails with `PLACEMENT_INFEASIBLE` before opening sources. Encoding infeasibility is detected when bound validation or run opens the database; explain opens nothing. |

```python
policy = b.policy(["/data"], placement="prefer_source")
print(recipe.explain(bindings, policy)["bound"]["placement"])
```

An output's anchor `eligibility` can run as a filter on the anchor source's scan. It is pushed when all of these hold:

- The anchor is bound to `sqlite`.
- The source has no other use in the recipe. It is not a relationship target, another output's anchor, a detail output's parent, a population's recipient or period source, or an allocation's quantity source. Outgoing one-to-one and one-to-many relationships also keep the filter local for shared-target ambiguity and detail-parent checks. Those need the records the filter would drop.
- The predicate reads only the anchor's own fields and literals, through `equal`, `less`, `less_equal`, `and`, `or`, `not` and `is_null`.
- Every compared value is `int64`, `string`, `date` or `boolean`. SQLite compares these as the local executor does: integers numerically, text and ISO dates bytewise, and a null comparison is unknown. The generated SQL removes each column's declared affinity and collation, so a `NOCASE` column still compares bytewise.
- No comparison involves a `decimal`, and the source's identity has no `decimal` field. SQLite would compare decimal text bytewise (`"120.50" < "50.00"`) or through a float.

Explain lists each eligibility with its placement and, when it stays local, the reason. It opens no source.

## What a pushed predicate changes

Output rows, values, evidence records and output counts are the same under either placement. `tests/sqlite.rs` runs 22 predicates both ways and compares them. Three things differ, and the manifest states each:

- **Records scanned.** `sources.<id>.rows` is still the table's record count. `scanned_rows` is how many crossed into the executor. Row limits apply to scanned records, so a table larger than `limits.anchor_rows` can run when its eligible records fit.
- **Text format checks.** Decimal and date formats, and valid UTF-8, are checked for scanned records only. Invalid UTF-8 in a dropped record fails locally but may pass under source placement.
- **Lookup checks.** Unmatched, ambiguous and strict-cardinality checks cover scanned anchor records only. A closed order with an unknown customer raises `LOOKUP_UNMATCHED` under local placement and nothing under source placement.

Before a pushed scan, SQLite checks every record of the table: identity present and unique, required columns not null, each value's storage class acceptable for its declared type, and every projected TEXT value within `limits.field_bytes`. If any record fails, the run reads the whole table locally and reports the same diagnostic local placement would. A database whose text encoding is not UTF-8 also falls back locally under `prefer_source`; `require_source` refuses it with `PLACEMENT_INFEASIBLE`.

Bytes read do not change. The whole file is read and fingerprinted under either placement, so this slice saves decoding and row budget, not I/O.

Static validation rejects expressions deeper than 18, including feature references, before recursive typing. Predicates with more than 500 bound literals stay local (or fail under `require_source`).

## Same-database lookup target reduction

Bindings using `sqlite` with byte-for-byte equal declared path spellings share
one immutable in-memory snapshot under every placement. Every binding is
independently admitted against its settings and read policy. The bytes are read,
fingerprinted and charged to `input_bytes` once. Each manifest source reports the
file's full `bytes_read`, the common fingerprint, and the other source IDs in
`shared_read`. Different path spellings, including `./`, do not share a read or
qualify for reduction, even when they refer to the same file.

Under `prefer_source` and `require_source`, a many-to-one relationship between
distinct co-located bindings can reduce its target scan to keys in the records
delivered by its from scan. A pushed eligibility is included in that selection.
The keys must be direct fields of the from source, with types `int64`, `string`,
`date` or `boolean`. Decimal keys stay local because decimal spellings can be
equal in the executor while unequal as SQLite text. Validity and period
qualification stay local. The target must have no decimal identity and no other
use: another output anchor, relationship target or from source, population
recipient or period source, or allocation quantity source.

SQLite returns target records in rowid order using an existence condition; it
preserves duplicate keys and compares without declared affinity or collation.
Null key components match nothing. Matching, ambiguity, strict cardinality,
candidates and evidence remain in the executor. For the same delivered from
records, reduction preserves all lookup outcomes and evidence.

Whole-table identity, nullability, storage-class, Boolean and `field_bytes`
checks finish on the connected candidate group before any filtered records are
delivered. A failure disables every pushed scan in the group, including sibling
targets, for local diagnostics. Non-UTF-8 database encoding falls back under
`prefer_source` and fails with `PLACEMENT_INFEASIBLE` under `require_source`.
Date text, decimal text and valid UTF-8 checks on reduced targets cover scanned
records only. An invalid date on an unused customer can therefore fail locally
and pass with reduction; the target placement entry discloses this coverage.
Target row, decoded batch and retention limits apply to delivered records;
whole-file byte limits and whole-table `field_bytes` checks still apply.

Explain opens nothing and names `relationship:<id>/target` for each candidate,
with its placement and first unsupported reason. `require_source` rejects an
unsupported candidate before opening any source. A relationship between files
with different declared path spellings is not a candidate and imposes no such
requirement. Encoding infeasibility is detectable only after opening a snapshot.
Capabilities declare `relationship_target_reduction` and equal-path co-location.
Placement does not alter canonical recipe JSON or its hash.

## Not included

Remote databases, credentials, SQL bindings, pushdown of aggregation, joined rows or output filters, transfers and staging are deferred.
