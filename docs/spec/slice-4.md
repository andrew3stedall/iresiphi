# Slice 4: same-database lookup reduction

Parent: https://github.com/andrew3stedall/iresiphi/issues/1.

Slice 3 placed one operation over one binding: an output's anchor eligibility, as a filter on a SQLite scan. This slice adds the first operation that spans two bindings. When both ends of a many-to-one lookup are tables in the same SQLite file, the source sends the executor only the target records whose key some delivered record of the other table carries. Matching, lookup outcomes and evidence stay in the local executor and do not change.

It follows "Execute at the source … including joins when the participating bindings are accessible there" and "Plan from requested fields" in [sources and execution](../design/sources-and-execution.md): a lookup target is needed only for the keys the executor will ask about.

Two independent scope proposals were compared. The alternative was to produce candidate pairs in SQLite and feed them to the engine's resolver. It was not chosen: evidence embeds every candidate record in full, so the source would return every candidate anyway, and it needs a second path through the lookup engine while moving no fewer records. Reduction changes what crosses into the executor and leaves the engine's matching alone.

## Acceptance fixture

One file `sales.db` with two tables. The recipe is slice 3's `open_orders`.

- `orders` (`order_no` INTEGER, `cust` TEXT, `booked` TEXT, `net` TEXT, `open` INTEGER): 1/A/2026-01-05/60.00/1, 2/A/2026-01-20/40.00/0, 3/B/2026-01-11/200.00/1, 4/C/2026-02-08/120.50/1, 5/Z/2026-02-09/10.00/1, 6/B/2025-12-31/75.25/1, 7/D/2025-11-02/30.00/1.
- `customers` (`CustNo` TEXT, `SalesArea` TEXT, `Since` TEXT, a nullable date): A/North/2020-01-01, B/North/null, C/South/2021-06-30, D/West/2022-02-02, E/East/2023-03-03.
- Eligibility: `open` is true and `booked >= 2026-01-01`. Relationship `customer`: `orders.cust` to `customers.CustNo`, many-to-one.

Expected, through the public validate/explain/run seam:

1. Under every placement: four output rows, orders 1, 3, 4 and 5; areas North, North, South and null (order 5 is unresolved: customer Z is unmatched); nets 60.00, 200.00, 120.50 and 10.00.
2. `local`: orders report 7 rows and customers 5, with no `scanned_rows`. Both sources report the same fingerprint and name each other in `shared_read`. The file's bytes are charged to `input_bytes` once: a limit equal to the file size passes.
3. `prefer_source`: orders report 7 rows and 4 scanned; customers report 5 rows and 3 scanned (A, B, C). Output rows, evidence records (including each lookup's candidates and their `rowid:<n>` references) and output counts equal the `local` run's.
4. Explain opens nothing and lists `relationship:customer/target` as placed at the source, reduced to the keys of the orders records scanned.
5. Variants that stay local, each with a stated reason:
   - A second output anchored at `customers`. `prefer_source` scans customers in full. `require_source` is `PLACEMENT_INFEASIBLE`, naming `relationship:customer/target`, before any source is opened.
   - Customers bound to a byte-identical copy of the file at another path. Not co-located: a full scan, and no infeasibility under `require_source`.
   - Keys declared `decimal(10,2)`, with `orders.cust` stored as `'1.0'` and `customers.CustNo` as `'1.00'`. Not reduced. The local executor matches them.
6. A disclosed difference: with customer E's `Since` set to `'2023-02-30'`, `local` fails with `INVALID_VALUE` at `rowid:5`. `prefer_source` succeeds, and the manifest says the customers' text formats were checked for scanned records only.

## Contract

1. **Co-location.** Two bindings are co-located when both use adapter `sqlite` and their declared `path` spellings are byte-for-byte equal as given. Path-component normalization is not used for this comparison. Different spellings of one file are not co-located. That costs a reduction, never correctness.
2. **Shared read.** Under every placement, co-located bindings are read once per run: one read of the bytes, one fingerprint, one charge to `input_bytes`, one in-memory database. Every participating binding is independently checked against the read policy and its settings; sharing never bypasses admission. Shared materialization works regardless of the order of source declarations. SQLite snapshots are admitted and opened before per-record decoding; admission or schema failures can therefore precede errors in an earlier-declared source under every placement. Error precedence across independent faults is not preserved from slice 3. Each source's manifest entry reports that fingerprint, the file's `bytes_read`, and `shared_read`, the IDs of the other sources read from the same bytes. Output rows and evidence are as in slice 3.
3. **Candidate.** A relationship is a reduction candidate when its `from` and `to` sources are co-located and distinct. A candidate is reduced only when all of these hold; otherwise it stays local and explain gives the first reason that fails:
   - The policy's placement is not `local`.
   - The cardinality is `many_to_one`, with no validity and no period qualification.
   - Every `from` key is a field of the `from` source itself.
   - The `to` source has no other use: it is not an output's anchor, not the target of another relationship, not the `from` of any relationship, not a population's recipients or period source, and not an allocation's quantity source.
   - Every key field has type `int64`, `string`, `date` or `boolean`. A `decimal` key is never reduced: `1.0` and `1.00` are equal in the executor and unequal as SQLite text.
   - The `to` source's identity has no `decimal` field, as in slice 3.
4. **Reduction.** A reduced target scan returns exactly the target records whose keys, with no null component, equal the keys of at least one record that the `from` scan delivers to the executor. If the `from` scan carries a pushed eligibility, the reduction uses the same condition; otherwise it uses every `from` record. Equality is SQLite `=` over operands stripped of affinity and collation, as in slice 3. The source key set is materialized once with a non-correlated membership query, rather than rescanned for each target record. Records are returned in ascending rowid order.
5. **Source checks.** Before a reduced scan, slice 3's whole-table checks (identity, nullability, storage classes, Boolean values and `field_bytes`) run over both tables, and the database encoding is checked. UTF-8 database encoding does not certify the validity of every text value. These checks finish before either table's filtered records are delivered or charged to decoded-record limits. If any check fails, all pushed scans of both tables are disabled and both tables are read in full locally, so the diagnostic is the one local placement gives. Fallback applies to the entire set of connected reduction candidates, including several targets reduced from one source. Under `require_source`, a database that is not UTF-8 is `PLACEMENT_INFEASIBLE`.
6. **Exactness.** For an identical set of delivered `from` records, enabling target reduction preserves lookup outcomes, candidate counts, candidates, ambiguity, strict-cardinality results, diagnostic codes, affected-record samples and evidence. Slice 3's eligibility placement can change that delivered set and its disclosed lookup-check coverage; reduction adds no further narrowing of anchor checks. Output rows, values, evidence records and output counts are the same. Null keys match nothing. Duplicate target keys stay ambiguous. Text keys compare bytewise: `'a'` is not `'A'`, `'01'` is not `'1'`, and the empty string is a key.
7. **What reduction changes, all disclosed.** The target's `scanned_rows`. Row limits apply to scanned target records. Date text, decimal text and valid UTF-8 checks of the target cover scanned records only. Decoded batch and retention limits apply to delivered records, as in slice 3; file and aggregate input byte limits apply to the shared file bytes. Whole-table `field_bytes` checks still cover discarded records. The manifest's placement record for the target says so, and the check coverage notes count reduced sources. Bound validation that runs data checks reports the same coverage as the run.
8. **`require_source`.** Every reduction candidate must be reduced, in addition to slice 3's rule for eligibility. A statically unsupported candidate makes validation, explain and run report `PLACEMENT_INFEASIBLE`, naming `relationship:<id>/target`, the reason and the alternatives, before any source is opened. Data-dependent encoding failures are detected after opening the shared snapshot: bound validation and run report `PLACEMENT_INFEASIBLE` with the stage and encoding reason; source-free explain reports only feasibility from declarations. Data-check failures that require a full local diagnostic use the group fallback from clause 5, as in slice 3. A relationship whose ends are not co-located is not a candidate and is never infeasible. This makes `require_source` stricter than in slice 3 for recipes whose lookup target is in the same file.
9. **Explain** lists, without opening a source, each candidate relationship's target placement and, when local, the reason. A reduced target's scan reads "reduced at the source to keys of `<from>`". It lists no transfers: nothing crosses environments.
10. **Manifest.** A reduced target's source entry records the placement used, the stage ID `relationship:<id>/target`, and the coverage of the source-side checks.
11. **Cancellation** is checked between records of a reduced scan and decimal-identity preflight. A scoped SQLite progress callback also checks the flag during queries, including whole-table checks and searches that deliver no records; SQLite interruption is reported as `CANCELLED`. The callback is removed before its borrowed flag can expire.
12. **Capabilities.** The `sqlite` adapter adds `relationship_target_reduction` to its pushdown operations and reports co-location by equal declared path. Describing capabilities opens nothing.
13. Placement never changes a recipe's canonical JSON or hash.

## Out of scope

Joined rows returned from SQLite. Reducing one-to-one or one-to-many relationships, or relationships with validity or period qualification. Reduction chains. Decimal keys. Eligibility that reads through a relationship at the source. Transfers, temporary tables and staging, including a key transfer from a source that is not SQLite. Path canonicalization or inode-based co-location. Aggregate pushdown. Parallel reads. Remote databases. Fewer bytes read: the whole file is read under every placement.

## Tickets

1. Co-located SQLite bindings share one read: contract 1, 2 and 12, with fixture items 1 and 2. Slice 3's tests pass unchanged.
2. Same-database lookup reduction: contract 3 to 11 and 13, with fixture items 3 to 6, a parity matrix against the local executor (null keys on either side, duplicate target keys, two-column keys, `NOCASE` and numeric-affinity columns, boolean keys, an unfiltered `from`, reversed source declarations, multiple targets from one source, strict ambiguity with reduction enabled and disabled, a target check that forces both scans local, UTF-16 under `require_source`, malformed UTF-8 coverage, and row/batch/retention/input limits), and the fixture through Python and Node.

Ticket 2 depends on ticket 1.
