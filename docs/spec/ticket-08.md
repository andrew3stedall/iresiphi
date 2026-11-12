## Parent

https://github.com/andrew3stedall/iresiphi/issues/16

## What to build

Demonstrate complete preview, workload and audited packaging. Deliver observable behavior through the shared public validate/explain/run seam. Use red-before-green public tests, retain decision provenance, and attach actual validation evidence before closing.

## Acceptance criteria

- [ ] Complete acceptance and corrupted fixtures pass through real JSON/CSV/XLSX, DataFusion and both public interfaces.
- [ ] 100,000-line variable-component complete workflow runs with fixture seed/hash, hardware/software, policy, timings, counts, I/O, RSS and tracked metrics honestly recorded.
- [ ] Locked Rust build, sdist and separate cp312/cp313 manylinux_2_28 wheels are built/audited/tested where supported; exact image digest and genuine external blockers are recorded.
- [ ] Apache2 project license preserves vendored licenses; dependency notices and distribution inventory are reviewed.
- [ ] Executable save/reopen/rebind/validate/explain/run example and support/resource/CSV/XLSX/cleanup limitations are documented.
- [ ] Parallel standards/spec reviews are completed and findings fixed; no publication/deployment/main merge.
- [ ] Every normative clause in the applicable contracts below is implemented or explicitly reported unmet; the checklist does not narrow those contracts.

## Blocked by

- https://github.com/andrew3stedall/iresiphi/issues/21
- https://github.com/andrew3stedall/iresiphi/issues/22
- https://github.com/andrew3stedall/iresiphi/issues/23

## Applicable contracts (self-contained)

### Decision #3

Source: https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135

## Resolution — invoice-review workflow and preview acceptance

Confirmed by the developer after the live interview. This resolves the representative workflow and its acceptance criteria; it does not select the detailed policies and technologies assigned to other map tickets.

### Intended user and recurring problem

Analysts repeatedly assemble invoice-review outputs using standardization and lookups. Once invoice and rate-card fields have been standardized, reconciling a result with the raw inputs becomes difficult.

The developer preview will demonstrate an auditable invoice review that preserves original values and makes the source/lookup chain inspectable while comparing billed and contracted charges. It is repeatable batch work; no particular daily or monthly cadence is required.

### Representative inputs and variation

Use publicly reproducible synthetic data representing the described real workflow:

- Two suppliers with different invoice JSON header/line structures, native field names, and item descriptions.
- AI extraction supplies the JSON upstream; extraction itself is not part of this workflow.
- Separate Excel enrichment for invoice lines.
- An explicit translation table connecting original line values with contracted-item identifiers.
- Supplier-specific contracted rate records.
- Existing component records at a finer grain beneath invoice lines.

The same translated item identifier can occur for different suppliers, so its identifier alone must not select a rate. Concrete matching keys, validity-date rules, connector versions, and declaration interfaces remain downstream choices.

Synthetic fixtures are sufficient for initial preview acceptance. No organizational data access is a prerequisite. These fixtures represent the real workflow's shape, not evidence of customer demand, extraction accuracy, or observed production performance.

### Primary output and reconciliation

The primary output has one record per extracted invoice line and retains the entire extracted line population, including unmatched or ambiguous comparisons. It carries invoice-header context, original line values, mapped values, enrichment, billed charge, expected charge where resolved, and variance where resolved.

Observed component records remain linked detail. Multiple components must not multiply a line's billed amount, expected amount, or comparison result. This is source-supplied detail, not an implicit allocation of line spend. No universal equality between component sums and billed charges has been agreed.

Reviewers must be able to inspect original values and source record identifiers or JSON paths through the relevant enrichment, translation-table entry, selected rate-card record, and linked components. This is a targeted reconciliation requirement, not a promise of exhaustive computational provenance for arbitrary operations.

Missing mappings/rates and ambiguous matches remain identifiable. The output must not invent expected amounts, silently discard unresolved lines, or label a partly resolved population as fully reconciled. Exact diagnostic representations and execution/failure policies belong in the corresponding downstream tickets.

### Manually reviewable acceptance example

Use positive quantities and a single declared currency for this simple fixture. Expected charge is quantity multiplied by the applicable contracted unit rate. Variance in this fixture is billed charge minus expected charge, with positive values indicating overcharge.

- Supplier A: quantity 2, billed charge 110, selected contracted unit rate 50. Expected charge 100; variance 10.
- Supplier B: quantity 1, billed charge 50, selected contracted unit rate 45. Expected charge 45; variance 5.
- Both resolved lines use the same translated item identifier but supplier-specific rates.
- A third unmapped line has billed charge 20. It remains in the primary population with an unresolved expected charge and variance.
- The comparable subset has billed charges 160, expected charges 145, and variance 15.
- The complete population has billed charges 180, including unresolved charges of 20. It must not be described as fully reconciled.
- Include several observed components beneath at least one line and ensure the primary row count and totals are unchanged by their multiplicity.

These are confirmed synthetic expected values, not executed test or benchmark results. The example does not establish support for every tax, discount, credit, currency-conversion, or temporal pricing rule; precise pricing/date/numeric policies remain separate decisions.

### Observable completion criteria

Through the agreed public validate/explain/run boundary:

1. Accept the two supplier layouts through explicit projections, bindings, and mappings without requiring their stored data to be standardized first.
2. Preserve invoice-header/line relationships and return the complete requested line population with the expected resolved and unresolved comparisons.
3. Make the original values, mapping steps, selected rates, and observed component relationships inspectable.
4. Produce the manually reviewed values above and keep comparable-subset totals distinct from full-population totals.
5. Detect corrupted variants containing missing mappings or rates, ambiguous/duplicate matches, and join fanout. Do not fabricate charges, silently lose lines, or multiply totals.
6. Support another review run after rebinding or updating an explicitly declared lookup without rewriting original invoice data. Exact recipe/interface mechanics remain downstream decisions.
7. Exercise approximately 100,000 generated invoice lines across multiple invoices, with variable component counts, alongside the tiny fixture. Measure behavior under declared resources; no elapsed-time, memory-budget, hardware target, or comparative speed claim is settled here.

### Consequences for the remaining map

- This invoice workflow replaces the customer/month example as the first-release acceptance anchor. The earlier example remains existing research/prior art; it is not deleted.
- JSON and Excel are concrete acceptance input requirements to account for when choosing the release support matrix. Physical formats for the remaining lookup/detail datasets, exact adapters, execution placement, runtimes, and sinks remain open.
- Allocation is not required by this observed-component workflow. Its developer-preview coverage remains a scope choice; the broader product requirement remains intact.
- The public-contract decision must account for targeted source-record reconciliation and identifiable unresolved comparisons. The identity/relationship and grain decisions must preserve the full line population and avoid component-induced fanout.
- The shared glossary now distinguishes value mapping, output population, observed detail, and source record reference. These are domain definitions; the decision itself lives in this resolution.
- Existing downstream tickets already cover the resulting precise questions. No new research, prototype, or preparation task is justified yet; add one when a specific evidence gap or access dependency becomes known.

The decision was explicitly confirmed by the developer. Proceed to the next frontier in a later decision session.


### Decision #4

Source: https://github.com/andrew3stedall/iresiphi/issues/4#issuecomment-6797517465

## Resolution — bounded local invoice-review preview

Decided autonomously under the developer's explicit authorization to resolve the whole map; this is a planning decision, not a claim of implemented support.

Ship a shared Rust library and Python interface on CPython 3.12/3.13, initially Linux x86_64. Inputs: local UTF-8 JSON invoice documents with explicit header/array projection; local XLSX worksheets with declared sheet, header row and typed columns; UTF-8 CSV lookup, rate and component tables with explicit dialect/schema. No inferred business mappings or schema coercion. XLSX formulas require stored cached values; missing caches fail, and the preview does not calculate formulas. Preserve original cells/paths and source identities for review.

All sources enter a single local executor through the same typed batch contract. Ship explicit local placement only, with no network reads, pushdown, remote staging, automatic transfer, or hybrid execution. Caller supplies permitted local roots and resource limits. Local files with different formats demonstrate cross-source composition without requiring database credentials. Output: a typed in-memory result and a local directory bundle of JSON manifest plus CSV line-review, linked-component, and reconciliation tables. Exact delivery semantics are settled by the resource/failure decision.

Defer TypeScript/Node/browser distribution, macOS/Windows wheels, dataframes, SQL, Snowflake, Spark, NDJSON, XML, arbitrary text, Parquet and remote stores. These remain product requirements: source adapters must report schema, identity, capabilities and location; executors must advertise semantic compatibility and access; sinks are independent of input. Unsupported bindings/operations fail explicitly, never silently collect data or substitute an executor. No pairwise connector-specific relationships.

The invoice acceptance workflow ships with explicit supplier-scoped mapping and rates, observed component detail, exact currency arithmetic, aggregation and safe broadcast. Allocation, arbitrary UDFs, currency conversion, tax engines and general temporal joins are deferred. Basic invoice-date rate validity filtering remains required. This bounds the preview without deleting broader functionality from the product specification.

Acceptance remains the already agreed two-supplier fixture and 100,000-line generated workload. This decision sets no speed claim or guaranteed platform beyond those above.


### Decision #11

Source: https://github.com/andrew3stedall/iresiphi/issues/11#issuecomment-6797686446

## Resolution — DataFusion behind an Iresiphi semantic plan

Autonomous decision under the developer's authority. Choose DataFusion 55.2.0 with Rust 1.94.0 as the implementation baseline, using its Arrow re-export rather than selecting an independent Arrow version. Pin direct dependencies and Cargo.lock. [Engine and bindings feasibility evidence](https://github.com/andrew3stedall/iresiphi/issues/15) supplies primary-source research, not a successful combined build or benchmark.

DataFusion supplies relational execution, batches, optimizer/physical machinery and participating-operator memory reservations. Iresiphi owns the semantic DAG, typed scalar rules, identity checks, population preservation, lookup-candidate counting, unresolved statuses, grain ownership, source references, checks and public explanation. Lower only validated operations into the engine. Optimizations must preserve checks and hidden dependencies; assert output population/identity invariants after execution. Count date-qualified candidates before selecting a rate, never use a post-join filter that drops unmatched anchors.

Choose it over Polars 0.55.2 because the documented resource integration and execution-plan cancellation contracts give an appropriate seam for this preview. Polars remains a viable future alternative; neither engine has a measured speed advantage here. Building a relational engine from scratch would add unnecessary scope. Source readers remain explicit adapters: serde_json 1.0.154 with raw_value, csv 1.4.0, calamine 0.38.0. Built-in readers must not bypass the adapter's evidence or type rules.

The research's suggested scales <=18 and rejection of rounding do NOT supersede the resolved scalar contract. Support declared decimal precision/scale up to 38, half-even and toward-zero quantization, exact checked arithmetic, and the established inference/rejection rules. Implement checked Rust kernels or engine extensions when DataFusion's native coercion/arithmetic cannot satisfy that contract. Such kernels are a deliberate semantic boundary, not a claim native behavior already conforms. Exact integer-coefficient reference cases must test value, resulting schema, overflow, negative ties, and scale extremes. There is no fallback to float or silent narrower precision.

Likewise the research's blanket rejection of floating XLSX monetary cells is superseded by the explicit quantization policy: absent that policy, reject; with it, interpret the observed finite numeric cell via an explicitly documented conversion to its shortest round-trip decimal representation, then apply the declared checked quantization. Record observed numeric representation, precision policy and cell reference. This does not recover the originally intended decimal. Prefer text decimal cells in the acceptance fixture. Formula cells require cached values, never evaluation; missing/uncertain cache presence is an adapter error, and freshness is not certified. Validate both Excel epochs, reject the fictitious 1900 leap day and fractional date-only serials.

Unsupported plan operations fail before execution; invalid data and overflow fail before publication. No spill or remote placement is implied by choosing DataFusion. Integration gates: locked build, real adapters, exact checked kernels including quantization, date/ambiguity cases, safe grain operations, memory/cancellation/error cleanup and full invoice acceptance. Failure of an upstream native kernel triggers the specified wrapper implementation, not weakening the contract. If the chosen version cannot support these explicit seams, implementation must report a concrete blocker and reopen the decision; research has identified no such blocker now.


### Decision #12

Source: https://github.com/andrew3stedall/iresiphi/issues/12#issuecomment-6797699573

## Resolution — PyO3 handles and copied exact Python values

Autonomous decision under the developer's authority, grounded in [Engine and bindings feasibility evidence](https://github.com/andrew3stedall/iresiphi/issues/15). Choose PyO3 0.30.1 and maturin 1.16.0. Support ordinary GIL-enabled CPython 3.12 and 3.13 on Linux x86_64, glibc >=2.28; separate cp312/cp313 wheels. Defer abi3, free-threaded Python, PyPy, browsers and Node/TypeScript delivery.

Rust owns recipe validation, logical planning, serialization, execution and diagnostics. Python builders construct the same declarative values; opaque recipe/run/result handles own Rust state with reference counting. They never borrow Python buffers or connection lifetimes for worker use. Internal batches are DataFusion-compatible Arrow RecordBatch with declared schema, Decimal128/Date32 and validity bits. Python materialization copies to int, Decimal constructed from exact text, datetime.date, str, bool and None. No float intermediates for exact values, PyArrow dependency, dataframe input, Arrow C-data export or zero-copy guarantee. Dataframe/connection adapters are deferred, not secretly accepted through arbitrary callbacks.

Public memory results expose bounded iteration over copied batches and explicit `close`, plus context-manager support. Closing releases retained Rust batches, sources and worker state; access after close returns `HANDLE_CLOSED`. Returned Python values remain valid after close because they own their data. Handle finalization is a cleanup fallback, not a deterministic completion/cancellation API. Repeated close is harmless. No worker calls back into Python; host-only expressions/extensions are rejected as nonportable.

Execution detaches from Python while doing Rust work. The calling thread periodically reattaches to check signals during its bounded waits, translating KeyboardInterrupt to the common cancellation token. Explicit cancellation uses the same path. Dropping streams/resources follows executor cancellation, with no promise to instantly interrupt blocking XLSX parsing. Errors expose stable Iresiphi code/phase/context/report ID; Python raises an Iresiphi exception carrying that record, while user cancellation maps to a cancellation subtype or KeyboardInterrupt preserving the run report. Raw DataFusion/PyO3 exception text is diagnostic detail, not the stable interface.

Serialize/load uses the same versioned document and exact literal encoding in both Rust/Python. Values beyond 2^53, Int64 boundaries, decimal ties/scales up to38, null and date boundaries must round-trip unchanged. Future TypeScript must use exact tagged strings or equivalent lossless values, never JS Number for these domains. Rust/Python public expected-result parity, clean-wheel installation, Ctrl-C, explicit cancellation, repeated close, access-after-close and retained-value-after-close are implementation release gates. Version metadata is evidence of an available approach; a combined build has not yet been performed.


### Decision #13

Source: https://github.com/andrew3stedall/iresiphi/issues/13#issuecomment-6797720274

## Resolution — bounded admission, cooperative cancellation, commit-only outputs

Autonomous decision under the developer's authority. The default preview profile is one active run per execution context (additional attempts return `RUN_BUSY`), two execution partitions, batches of at most8192 rows, and at most two queued batches per producer. Pull-based backpressure bounds queues; adapters must also enforce byte limits because a row limit alone is insufficient.

Defaults: 256MiB participating-engine reservation budget; 64MiB per input file and 256MiB aggregate encoded input; 256MiB aggregate actual decompressed XLSX bytes; one million decoded cells per worksheet; 1MiB per field/token; JSON depth64; 100,000 invoice lines, one million component rows and 100,000 rows per lookup source; one million total qualified lookup-candidate pairs; 256MiB serialized output/evidence; 256MiB retained result batch buffers. Each batch also has a 16MiB decoded-buffer limit, with a single over-limit record rejected. Limits may be raised explicitly in policy and are recorded; they are admission choices, not measured capacity promises. The generated100,000-line fixture must be designed within the declared profile, and actual complete-recipe measurements gate release.

No spill ships. Stateful joins/aggregates either reserve within their tracked budget or fail with `RESOURCE_LIMIT`; they cannot silently collect elsewhere. Unknown source sizes require counting while reading, and exceeding a limit aborts. ZIP preflight checks and actual decompression/cell accounting precede unbounded workbook range allocation. Candidate pairs are capped during candidate production, before materializing a join explosion. Memory results must fit retained-buffer and serialized-size budgets before becoming successful; Python batch conversion copies only one requested batch at a time. User-retained Python values are outside library accounting.

These controls are not a hard process RSS ceiling: DataFusion reservations omit some allocations, adapters and runtime overhead exist, and Python owns copies. The manifest separates tracked reservations/buffers from measured peak RSS when available. Deployments requiring a hard RSS/kill deadline must use OS-isolated execution; no hidden subprocess contract is promised.

Cancellation is cooperative via one run token checked between parsing chunks, batches, output writes and explicit wrapper loops; cancel drops all executor streams and joins owned worker shutdown before reporting completion. Signal checks occur at most100ms apart while the Python caller is waiting, but blocking adapter/operator work may delay actual shutdown. No bounded cancellation latency is promised; measure and report it. Failure/cancellation publishes no successful partial result. Checks not completed remain marked incomplete.

Remote staging is deferred. Local scratch/output staging is run-owned under an approved root, with exclusive unpredictable names and owner-only permissions. No writes outside approved roots, no following untrusted path/symlink escapes, and no recursive deletion of caller-owned data. Success/failure/cancellation removes owned temporary artifacts; failed cleanup records exact leftover paths and errors in the run report. Process death can leave scratch; an explicit cleanup command may remove only recognizable run-owned artifacts after ensuring the run is inactive. No promise that crash cleanup happens automatically.

Directory output is one bundle containing primary/detail/evidence, schema and manifest. Write a sibling temporary directory on the same filesystem, finish/close files, and atomically rename to a NEW destination with no-replace semantics. Refuse an existing destination; no overwrite/merge. Support local filesystems providing that rename primitive; reject cross-device or unsupported publication. A complete-marker manifest is inside the renamed bundle. This is atomic visibility of the bundle, not an fsync-backed power-loss durability guarantee or a transaction with inputs. Cancellation/failure before commit aborts; after successful rename the run is committed and subsequent cancellation cannot revoke it. A post-commit reporting failure leaves a discoverable complete bundle and its run ID. Retry at the same destination returns `DESTINATION_EXISTS`; callers inspect its manifest, never duplicate/overwrite silently.

JSON outputs use schema-tagged exact scalar representations. CSV bundle encoding is UTF-8, comma delimiter, RFC4180 quoting, LF record endings and one header row. Null is the field text `\N`; every non-null text representation beginning with backslash gains one leading backslash before CSV quoting. On read, exact `\N` is null; otherwise remove one leading backslash only from a doubled-backslash prefix, then parse according to schema. Empty string remains empty; literal `\N` becomes `\\N`. This escaping applies to every non-null field representation and is specified in the companion schema; arbitrary spreadsheet readers are not claimed to decode it automatically.

Public failure tests force each budget, unknown-size overflow, cancellation at parse/join/write, cleanup failure, existing destination, process interruption before/after commit and repeated retry. Verify diagnostics and visible outputs, not private scheduler order. No snapshot consistency across independent sources is implied.


### Decision #14

Source: https://github.com/andrew3stedall/iresiphi/issues/14#issuecomment-6797736842

## Resolution — narrowly supported, evidence-gated developer preview

Autonomous decision under the developer's authority. Select Apache-2.0 for new Iresiphi project code and documentation. Existing vendored skills retain their own licenses/notices; this decision does not relicense third-party material. Implementation must add the project license/required notices and inspect the locked dependency distribution before publication. This is a licensing choice, not legal certification of an unbuilt dependency graph.

Release aligned Rust `iresiphi` crate and Python `iresiphi` distribution at initial preview version0.1.0, with one shared Rust semantic core and thin Python module. A Python source distribution includes the Rust sources/lockfile and documents the required build toolchain. Build separate ordinary CPython3.12/3.13 wheels for Linux x86_64 manylinux_2_28 (glibc>=2.28) using pinned build-image digest and maturin1.16.0; no target-cpu=native. Rust1.94.0 and the exact dependency pins selected by engine/binding decisions are the baseline. Audit native wheel dependencies and test minimum-glibc installation/import on clean images. No abi3, free-threaded Python, PyPy, macOS, Windows, ARM, browser or Node/TypeScript support claim in this release.

Recipe format0.1 is explicitly versioned independently of package patches, under the exact compatibility/migration policy in the portable-recipe decision. Saved recipes contain no live bindings; examples must show save, reopen, rebind, validate, explain and run. Publish the support/deferral matrix, resource profile, XLSX quantization/cache limits, CSV null escaping, snapshot limitations, unresolved comparison semantics and cleanup behavior beside the executable example. Broader connectors, remote/hybrid execution, dataframe access, TypeScript and allocation remain product requirements deferred beyond this preview.

Required release evidence: (1) locked Rust build and audited wheels in clean supported environments; (2) common Rust/Python public fixtures including the exact two-supplier160/145/15 comparison, total billed180 and unresolved20; (3) checked scalar oracle cases including38-digit precision, quantization ties/negative values, integer boundaries, overflow/null/date behavior; (4) real JSON/CSV/XLSX adapter tests including numeric lexemes, duplicate keys, CSV escaping, formula cache failures, workbook epochs, ZIP/range limits and source evidence; (5) identity/path/date-overlap/fanout/grain failures; (6) persistence/rebinding parity; (7) budgets, backpressure-visible limits, cancellation, handle ownership, scratch cleanup and atomic publication/retry cases; (8) complete100,000-line workflow measurements including source access, validation, conversion, planning, execution, evidence and delivery.

Record hardware, OS/compiler/runtime versions, immutable fixture seed/hash, input sizes, policy, actual row/evidence counts, elapsed phases, peak RSS where measured, tracked reservations, bytes read/written and output size. Report observations only. No faster-than-Python/database/Spark claim or universal latency/memory promise follows from Rust or these synthetic measurements. There is no fixed elapsed-time release threshold; correctness, completion under the declared test policy and truthful resource reporting are required.

Before publication recheck crate/PyPI names and account ownership, generate and review dependency/license notices, ensure no bundled secrets or unintended source data, and document installation limitations. Name availability is not reservation. These are ordinary release execution gates on artifacts that do not yet exist, not unresolved planning decisions or a need for external account access now. If a required name is unavailable, do not silently publish under a substitute; report the concrete conflict as a new naming decision.

The decision route is ready for specification synthesis and implementation tickets. Nothing here claims packages have been built, tests executed, licenses audited, performance measured or artifacts published. Production implementation and publication remain outside this map.

