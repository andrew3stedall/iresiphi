# Slice 5: key transfer to a lookup target

Parent: https://github.com/andrew3stedall/iresiphi/issues/1.

Slice 4 reduced a lookup target only when both ends of the lookup were tables in one SQLite file. Nothing moved: the source already held both sides. This slice adds the first transfer. When a many-to-one lookup's target is a SQLite table and its other end is anywhere else (a CSV, JSON or XLSX file, or a table in a different SQLite file), the executor may send the keys of the records it holds to the target's source, which then returns only the target records carrying one of those keys. Matching, lookup outcomes and evidence stay in the local executor and do not change.

It follows "Combine both" in [sources and execution](../design/sources-and-execution.md): data from one source meets another only through a transfer the policy permits and the plan shows. It also supplies that document's missing case, a policy that prevents an otherwise valid plan: transfers are off unless the policy turns them on.

Decisions below were made autonomously under the developer's instruction to proceed. They resolve, for this slice only, what a transfer is and how the policy governs it.

## Acceptance fixture

Slice 3's fixture with the formats swapped. The recipe is `open_orders`.

- `orders.csv` (`order_no`, `cust`, `booked`, `net`, `open`): 1/A/2026-01-05/60.00/true, 2/A/2026-01-20/40.00/false, 3/B/2026-01-11/200.00/true, 4/C/2026-02-08/120.50/true, 5/Z/2026-02-09/10.00/true, 6/B/2025-12-31/75.25/true, 7/D/2025-11-02/30.00/true.
- `crm.db`, table `customers` (`CustNo` TEXT, `SalesArea` TEXT, `Since` TEXT, a nullable date): A/North/2020-01-01, B/North/null, C/South/2021-06-30, D/West/2022-02-02, E/East/2023-03-03.
- Eligibility: `open` is true and `booked >= 2026-01-01`. Relationship `customer`: `orders.cust` to `customers.CustNo`, many-to-one.

Expected, through the public validate/explain/run seam:

1. Under every policy: four output rows, orders 1, 3, 4 and 5; areas North, North, South and null (order 5 is unresolved).
2. `placement: local`, or `transfers: none` under any placement: customers report 5 rows, no `scanned_rows` and no placement record. `require_source` with `transfers: none` succeeds. Explain gives the reason the target is local and names `transfers: keys` as the setting that would reduce it.
3. `transfers: keys` with `prefer_source` or `require_source`: customers report 5 rows and 4 scanned (A, B, C, D). The transfer carries 5 keys: every order is delivered from the CSV file, including ineligible ones, so D and Z are sent. Output rows, evidence, candidate evidence, diagnostics and output counts equal the `local` run's.
4. With orders in a SQLite table of a different file, the pushed eligibility narrows the keys: 4 keys (A, B, C, Z) and 3 customers scanned.
5. Explain opens nothing and lists one transfer, `relationship:customer/keys`, from `source:orders` to `source:customers`.
6. `limits.transfer_keys = 4`: `prefer_source` reads customers in full and the manifest says why; `require_source` is `PLACEMENT_INFEASIBLE`.
7. A second output anchored at `customers`: never reduced. With `require_source` and `transfers: keys` it is `PLACEMENT_INFEASIBLE` before any source is opened.
8. A disclosed difference, as in slice 4: with customer E's `Since` set to `'2023-02-30'`, `local` fails at `rowid:5` and the transfer run succeeds, stating that the customers' text formats were checked for scanned records only.

## Contract

1. **Policy.** The policy gains `transfers`, one of:
   - `none` (the default): nothing moves from one source to another. Existing policies mean this, and existing results and manifests do not change.
   - `keys`: the executor may send lookup key values of records it holds to the lookup target's source.

   `limits.transfer_keys` (default 10,000) bounds the key values one transfer sends: distinct keys times key columns. It never exceeds 32,766, SQLite's bound-parameter ceiling, whatever the policy says.
2. **Candidate.** A relationship is a transfer candidate when its `to` source is bound to `sqlite`, its `from` and `to` sources are distinct, and the two bindings are not co-located (slice 4 covers co-located bindings, and no transfer is needed there). A candidate is reduced only when all of these hold; otherwise it stays local and explain gives the first reason that fails:
   - The policy's placement is not `local`.
   - The policy's transfers are `keys`.
   - Slice 4's conditions on the relationship and its target: `many_to_one` with no validity or period qualification; every `from` key a field of the `from` source itself; the target has no other use; no `decimal` key; no `decimal` field in the target's identity.
3. **What is transferred.** The distinct keys, with no null component, of the `from` records delivered to the executor. If the `from` scan carries a pushed eligibility, only its scanned records contribute; otherwise every `from` record does, eligible or not. Keys are bound as query parameters on the target's read-only in-memory snapshot. Nothing is written, staged or sent outside the process, and no temporary table is created. The `from` source may use any adapter.
4. **Reduction.** A reduced target scan returns exactly the target records whose keys, with no null component, equal a transferred key. Equality is SQLite `=` over operands stripped of affinity and collation, against parameters bound by the key's declared type: `int64` and `boolean` as INTEGER, `string` and `date` as TEXT. Records are returned in ascending rowid order. An empty key set scans no record.
5. **Order.** A transfer target is read after every other source. Error precedence across independent faults in different sources therefore follows read order, not declaration order, when a transfer is planned.
6. **Source checks.** Before a reduced scan, slice 3's whole-table checks run over the target and the database encoding is checked. If a check fails, the target is read in full locally, so the diagnostic is the one local placement gives. A database that is not UTF-8 falls back locally under `prefer_source` and is `PLACEMENT_INFEASIBLE` under `require_source`, in run and in bound validation.
7. **Key limit.** A transfer whose key values exceed the limit is not sent. Under `prefer_source` the target is read in full locally and its placement record gives the reason. Under `require_source` the run fails with `PLACEMENT_INFEASIBLE`, phase `source`, naming `relationship:<id>/target`. The diagnostic carries counts, never key values.
8. **Exactness.** A transfer changes no lookup outcome, candidate count, candidate, ambiguity, strict-cardinality result, diagnostic or evidence record: every delivered `from` record still meets every target record that shares its key. Null keys match nothing. Duplicate target keys stay ambiguous. Text keys compare bytewise.
9. **What a transfer changes, all disclosed.** The target's `scanned_rows`. Row limits apply to scanned target records, so a target larger than `limits.lookup_rows` can run when the referenced records fit. Date text, decimal text and valid UTF-8 checks of the target cover scanned records only. Bytes read do not change.
10. **`require_source`** binds a transfer candidate only when the policy also permits transfers. With `transfers: keys`, a statically unsupported candidate makes validation, explain and run report `PLACEMENT_INFEASIBLE`, naming `relationship:<id>/target`, before any source is opened. With `transfers: none` the candidate is local and never infeasible: neither setting overrides the other, and explain states which setting blocks the reduction.
11. **Explain** lists, without opening a source, each candidate's target placement and, when local, the reason. `bound.transfers` lists each planned transfer: its ID `relationship:<id>/keys`, the `from` and `to` sources, the key fields, what is sent, how, and the key limit. Row counts are unknown before a run and say so. A reduced target's scan reads "reduced at the source to keys transferred from `<from>`". `bound.placement.transfers` echoes the policy.
12. **Manifest.** A reduced target's placement record adds `reduction: transferred_keys` and a `transfer` record: its ID, the `from` source, the number of distinct keys and of key values, and the mechanism. It never records the values.
13. **Capabilities.** The `sqlite` adapter adds `relationship_target_key_transfer` to its pushdown operations and reports `transfers_accepted: ["keys"]`. Describing capabilities opens nothing.
14. Placement and transfers never change a recipe's canonical JSON or hash.

## Out of scope

Transfers of anything but keys. Transfers to a source that is not SQLite, or out of the process. Temporary tables, staging and writes. Chunked transfers above the key limit. Decimal keys. One-to-one, one-to-many, validity-qualified and period-qualified relationships. Chains, where a reduced target is itself the `from` of another lookup. Cost-based choice between a transfer and a full read. Remote databases. Fewer bytes read.

## Tickets

1. Key transfer: the whole contract, with the fixture and a parity matrix against the local executor (null and duplicate keys, case and empty-string keys, compound keys over `NOCASE` and numeric-affinity columns, boolean keys, an empty key set, thousands of keys, a failed source check, UTF-16, the key limit, a target with another use) in `tests/transfer.rs`, and the fixture through Python and Node.
