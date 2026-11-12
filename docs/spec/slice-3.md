# Slice 3: a database source and source-side placement

Parent: https://github.com/andrew3stedall/iresiphi/issues/1.

Slices 1 and 2 read local files and ran every operation in the local executor. This slice adds the first source that is not a file format, and the first operation that can run at a source instead of in the local executor. It follows the "next design work" in [sources and execution](../design/sources-and-execution.md): two contrasting adapters, a cross-source join, the same recipe under two placements, and a policy that blocks an otherwise valid plan.

The database is SQLite. It needs no server, builds on ARM and x86, and its type system differs enough from the recipe's (dynamic storage classes, no decimal type) that the semantic checks are real. Remote databases, Snowflake, Spark and staging writes stay deferred.

Decisions below were made autonomously under the developer's instruction to proceed. They resolve part of unresolved choice 2 (initial contrasting adapters) and part of choice 9 (capability schemas) of issue #1, for this slice only.

## Acceptance fixture

Orders live in a SQLite table. Customers live in a CSV file. The recipe names neither format.

- `orders` table (`order_no` INTEGER, `cust` TEXT, `booked` TEXT, `net` TEXT, `open` INTEGER): 1/A/2026-01-05/60.00/1, 2/A/2026-01-20/40.00/0, 3/B/2026-01-11/200.00/1, 4/C/2026-02-08/120.50/1, 5/Z/2026-02-09/10.00/1, 6/B/2025-12-31/75.25/1.
- `customers.csv` (`CustNo`, `SalesArea`): A/North, B/North, C/South.
- Output `open_orders`: anchored at orders, eligible when `open` is true and `booked >= 2026-01-01`, with the customer's sales area looked up through a many-to-one relationship.

Expected, through the public validate/explain/run seam:

- Four output rows: orders 1, 3, 4 and 5. Order 5 is unresolved (customer Z is unmatched). Nets 60.00, 200.00, 120.50 and 10.00.
- The same recipe, with orders bound to a CSV file that has the same rows, gives the same output rows and values.
- Under local placement the run scans six order rows. Under source placement it scans four and reports six source rows. Output rows, evidence records and output counts are identical under both.
- A recipe whose eligibility compares the decimal `net` cannot be pushed to SQLite. `prefer_source` runs it locally and says why. `require_source` refuses it before any source is opened.

## Contract: SQLite source

1. A binding with adapter `sqlite` names a database file and one table: `settings.table`. No other setting is accepted. Views, virtual tables, `WITHOUT ROWID` tables and free-form SQL are rejected with `INVALID_BINDING`. The recipe is unchanged: a field's `path` is the column name, matched exactly.
2. The file is opened under the read policy and byte limits that apply to every source. The run reads the bytes once, fingerprints them, and queries that copy read-only in memory. The fingerprint therefore identifies exactly the data the query saw, and a concurrent writer cannot change a run halfway. Bytes read is the whole file under either placement.
3. Values are converted by storage class, never through a float:
   - `int64`: INTEGER.
   - `boolean`: INTEGER 0 or 1.
   - `string`: TEXT.
   - `date`: TEXT in `YYYY-MM-DD` form.
   - `decimal`: TEXT holding a decimal number, or INTEGER. REAL is rejected: it has no exact decimal value.
   - NULL is null where the field is nullable and `NULL_REQUIRED` otherwise.
   - Any other storage class, including BLOB, is `INVALID_VALUE`. Declared column types and affinities are not trusted; each value's own storage class decides.
4. Value failures are counted across the whole scan and reported with record references, as for CSV. A record's reference location is `rowid:<n>`. Records are read in ascending rowid order.
5. Identity, null-identity and row-limit checks apply as for every source.
6. Bound validation with `data_checks = "none"` opens the database, and checks the table and each declared column exist. It reads no rows.
7. Cancellation is checked between records. The database is a bounded in-memory copy, so no query runs unbounded between checks.
8. The adapter reports its capabilities: local location, typed read, bounded materialization, snapshot of the bytes read, and the pushdown it supports (contract below). Describing capabilities opens nothing.

## Contract: placement

1. The policy gains `placement`, one of:
   - `local` (the default): every operation runs in the local executor. Existing policies mean this, and existing results do not change.
   - `prefer_source`: an operation runs at its source where this contract allows it, and locally otherwise. Explain states which, and why.
   - `require_source`: every output's eligibility on a source that supports pushdown must run at the source. If one cannot, validation, explain and run report `PLACEMENT_INFEASIBLE`, naming the blocked operation, the reason and the alternative (`prefer_source` or `local`). Nothing is opened first, and the policy is never overridden.
2. One operation can be placed at a source in this slice: an output's anchor eligibility predicate, as a filter on the anchor source's scan. Aggregation, joins, allocation and output filters always run locally.
3. Placement is negotiated against the exact predicate, not against a "supports filters" flag. A predicate is pushed only when all of these hold:
   - The anchor source's adapter declares filter pushdown (in this slice, `sqlite`).
   - The source has exactly one use in the recipe: it is the anchor of this one output. It is not the target of a relationship, the anchor of another output, the recipient or period source of a population, or the quantity source of an allocation. Otherwise other operations need the rows the filter would drop.
   - The predicate reads only fields of the anchor source, with no relationship path, feature, aggregate or arithmetic.
   - The source's identity has no `decimal` field. SQLite cannot check a decimal identity's uniqueness exactly, and that check must cover every record.
   - The predicate uses only `equal`, `less`, `less_equal`, `and`, `or`, `not`, `is_null`, fields and literals, over `int64`, `string`, `date` and `boolean` values. SQLite compares these exactly as the local executor does: integers numerically, text and ISO dates bytewise, null as unknown, and only true keeps a row.
   - No comparison involves a `decimal`. SQLite would compare decimal text bytewise or through a float, and neither is the recipe's exact numeric comparison. This is the semantic mismatch that rejects pushdown.
4. Before a pushed scan, the source checks every row of the table, not only the rows the predicate keeps: identity columns are non-null and unique, non-nullable columns are non-null, and each column's storage classes are ones the declared type accepts. If any check fails, the run reads the whole table locally instead, so the diagnostic is the same one local placement gives. A database whose text encoding is not UTF-8 is also read locally, because its bytewise text order differs; under `require_source` that is `PLACEMENT_INFEASIBLE`.
5. What a pushed predicate changes, all of it disclosed:
   - Rows scanned. The manifest reports the table's row count as `rows` and the rows that crossed into the executor as `scanned_rows`. Row limits apply to scanned rows.
   - Text format checks (decimal and date text) cover scanned rows only.
   - Lookup checks (unmatched, ambiguous, strict cardinality) cover scanned anchor records only. An ineligible order with an unknown customer produces a warning under local placement and nothing under source placement.
   The manifest's check coverage and the placement record say so. Output rows, values, evidence records and output counts (`source_rows`, `eligible_rows`, `output_rows`) do not change.
6. Explain with bindings and a policy lists, per source, the placement of its scan and, per output eligibility, whether it is pushed, and if not, the first reason. It lists no transfers: nothing moves between environments in this slice. Explain opens no source.
7. The run manifest records, per source, the placement that was used, the pushed predicate's stage ID, and the coverage of the source-side checks.
8. Placement never changes a recipe's canonical JSON or hash. It is run policy, not recipe meaning.

## Out of scope

Remote databases and credentials. Free-form SQL bindings and views. Pushdown of aggregation, joins, population filters, allocation or output filters. Transfers and staging. Cost-based placement and size estimates. Decimal comparison at the source. Publication.

## Tickets

1. SQLite source: contract "SQLite source", with the fixture's local-placement half and CSV parity.
2. Source placement: contract "placement", with the fixture's placement half, through Rust, Python and Node.

Ticket 2 depends on ticket 1.
