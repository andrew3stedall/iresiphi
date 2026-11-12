# Slice 2: explicit populations, allocation and Node bindings

Parent: https://github.com/andrew3stedall/iresiphi/issues/1. Specification issue: #27. Tickets: #28, #29, #30.

The invoice-review preview (issues #16 to #24) delivered sources, relationships, aggregation and broadcast. This slice adds the two things the product specification names that the preview deferred and that need no new infrastructure: allocation over an explicit population, and a TypeScript interface. Connectors, pushdown and hybrid execution stay deferred.

Decisions below were made autonomously under the developer's instruction to proceed. They resolve unresolved choices 6 (TypeScript runtime) and 8 (allocation policy) of issue #1 for this slice only.

## Acceptance fixture

The customer/month example from issue #1 is the acceptance fixture. Keep the raw field names.

- Customers (`CustNo`, `SalesArea`, `Share`): A/North/1, B/North/3, C/South/1.
- Transactions (`receipt_id`, `buyer_ref`, `booked_at`, `gross`): t1/A/2026-01-05/60, t2/A/2026-01-20/40, t3/B/2026-01-11/200, t4/C/2026-01-17/50, t5/B/2026-02-08/120.
- Regional targets (`Territory`, `Period`, `Goal`): North/2026-01/900, South/2026-01/300, North/2026-02/800, South/2026-02/400.
- Declared: AUD amounts, business-local dates, stable regions and shares, complete activity, all three customers eligible in both months.

Expected, through the public validate/explain/run seam:

- Six output rows (customer by month).
- Sales 100/200/50 in January and 0/120/0 in February for A/B/C. Transaction counts 2/1/1 and 0/1/0. Total sales 470.
- Broadcast target context repeats the regional goal on each row. Its six-row sum is 4100, and the run must not report that as a target total.
- Allocated targets: January A=225, B=675, C=300; February A=200, B=600, C=400. Allocated total 2400, equal to the source target total.
- Selecting A after allocation keeps January's 225. Restricting eligible North recipients to A before normalization assigns A 900.
- Pruning B from the output while B stays in the recipient population keeps B's weight in the denominator.

## Contract: explicit population

1. An output may declare a population: the cross product of a recipient source (or a filtered subset of it) and a declared, finite period set. The period set is listed in the recipe or taken from the distinct values of a named field of a named source. Nothing infers a population from observed activity or from the requested grain.
2. Finer records join the population by declared keys and a declared period derivation (for the fixture, calendar month of a date field, with no timezone conversion: dates are business-local).
3. A population member with no finer records has aggregate value null and count 0, unless the output declares the activity source complete. With that declaration, sums are 0. The manifest and explain output state which rule applied, and that completeness is a declared assumption, not an observation.
4. Finer records that match no population member are counted and reported with samples. They never widen the population.

## Contract: allocation

1. Allocation is a distinct declared operation. Nothing derives it from a join, and broadcast never becomes allocation implicitly.
2. An allocation declares: the quantity (a field of a coarse source), the source partition (the coarse source's grain keys), the recipients (the output population or a declared subset, with an optional recipient filter applied before normalization), the weight expression at recipient grain, the result scale, and a rounding and residual policy.
3. Shares are computed exactly: `quantity * weight / sum(weights in partition)` in exact decimal arithmetic, then quantized to the declared scale. Division is allowed here only; general scalar division stays rejected.
4. Residual policies, one of which must be declared:
   - `largest_remainder`: quantize toward zero, then distribute the remaining units of the last place one each to recipients in descending fractional remainder, ties broken by ascending recipient identity. Allocations then sum exactly to the quantity.
   - `report`: quantize half-even and report the residual (quantity minus assigned sum) per partition. The residual may be negative.
5. Conservation is checked per source partition after every run: assigned sum plus reported residual equals the source quantity exactly. A violation fails the run. The check and its per-partition figures appear in the manifest and in evidence.
6. No implicit defaults for edge cases. Each produces a named diagnostic unless the recipe declares the alternative:
   - Null weight: error, or declared `null_weight: zero`.
   - Negative weight: always an error.
   - Partition whose weights sum to zero, or with no recipients: error, or declared `unallocated: report`, which reports the whole quantity as residual and assigns null to any recipients.
   - Recipient whose partition has no source quantity (missing target): allocation null and the row marked unresolved, never zero.
   - Duplicate source partition keys: error, as for any identity violation.
7. Filter timing is visible. A recipient filter runs before normalization and changes shares. An output filter runs after allocation and never changes shares. Explain names each filter's stage. Dependency pruning must keep recipients that are in the denominator but not in the output.
8. Allocated columns carry ownership and grain metadata in the output schema: origin source and partition grain, transition `allocated`. Totals for an allocated column are reported alongside the source total they conserve. Broadcast columns keep transition `broadcast` and are excluded from additive totals.
9. Mixed currency within a partition, or between quantity and a currency-carrying recipient, is rejected as today.
10. Evidence records, per partition: quantity, recipient count, weight sum, assigned sum, residual, policy. Per recipient: weight, exact share before quantization where it terminates within 38 digits (otherwise a flag), and assigned amount.
11. Budgets, cancellation and bundle delivery apply unchanged. The population size counts toward the existing row limits before it is materialized.

## Contract: Node bindings

1. TypeScript initially means Node.js 22 and 24 on Linux x86_64 and aarch64, glibc, through a native addon built with napi-rs from the same Rust core. No browser, Deno, Bun, Windows or macOS claim. Nothing is published to npm.
2. The addon lives in its own crate in this repository and depends on the `iresiphi` crate. It adds no semantics. The Python module is the model for its surface: recipe load, save, hash, validate, explain, run to memory, run to a bundle, cancellation, and the same structured errors.
3. Numeric transport never loses precision. Decimals cross as strings. 64-bit integers cross as `bigint`. Dates cross as ISO strings. No value is converted through an IEEE double.
4. Runs execute off the JavaScript main thread and return promises. An `AbortSignal` cancels a run through the existing cancellation token.
5. Type declarations (`.d.ts`) ship with the package and are checked by compiling a TypeScript test with `tsc --strict`.
6. Shared fixtures: the Node tests compare against the same parity golden file the Rust and Python suites use (`tests/fixtures/two_suppliers.parity.json`), and against the slice-2 customer/month golden once it exists.
7. CI builds and tests the addon on both x86 and ARM hosted runners. A pass on both at the same commit is required, as for everything else.

## Out of scope

Publication to crates.io, PyPI or npm. New source families. Pushdown, hybrid execution and staging. Weighted or ratio measures beyond the allocation division. Allocation across more than one coarse source in one operation. Time zone conversion.

## Tickets

1. #28: Explicit population and completeness, with the sales half of the fixture.
2. #29: Allocation with conservation, residual policies, filter timing and the full fixture through Rust and Python.
3. #30: Node bindings with exact numeric transport and shared fixtures.

Tickets 1 and 2 are sequential. Ticket 3 is independent of both.
