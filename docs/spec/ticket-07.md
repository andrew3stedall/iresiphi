## Parent

https://github.com/andrew3stedall/iresiphi/issues/16

## What to build

Expose owned Rust recipes and results through exact Python bindings. Deliver observable behavior through the shared public validate/explain/run seam. Use red-before-green public tests, retain decision provenance, and attach actual validation evidence before closing.

## Acceptance criteria

- [ ] Python validate/explain/run uses the shared core and exact int/Decimal/date/null copied values, including scale38 and beyond2^53.
- [ ] Bounded batch iteration, context manager, idempotent close, access-after-close and retained values meet decision12.
- [ ] Execution releases GIL; explicit cancellation and <=100ms signal-check waits use the common token and preserve reports.
- [ ] Save/reopen/rebind and tiny expected-result/error parity run through Rust and Python.
- [ ] Owned handles never borrow worker Python buffers or call Python from workers; host-only expressions are rejected.
- [ ] Every normative clause in the applicable contracts below is implemented or explicitly reported unmet; the checklist does not narrow those contracts.

## Blocked by

- https://github.com/andrew3stedall/iresiphi/issues/17
- https://github.com/andrew3stedall/iresiphi/issues/22

## Applicable contracts (self-contained)

### Decision #6

Source: https://github.com/andrew3stedall/iresiphi/issues/6#issuecomment-6797556305

## Resolution — exact, deliberately small scalar language

Autonomous decision under the developer's authority; selected behavior, not implementation evidence.

Preview scalar types are nullable Boolean, signed Int64, UTF-8 string, Decimal128(p,s) with 1 <= p <= 38 and 0 <= s <= p, and calendar Date. Collections exist at projection/detail boundaries, not as arbitrary scalar expressions. Timestamp/timezone calculations, binary floating-point arithmetic, arbitrary Python callbacks, regex, and user-defined functions are deferred. Input NaN and infinity are rejected; they are not null. Missing JSON paths and blank spreadsheet cells become null only when the declared field is nullable; malformed present values are errors. Empty string stays empty string.

Expressions comprise typed literals, field/path references, explicit casts, add/subtract/multiply, comparisons, Boolean AND/OR/NOT, is-null, coalesce, case, and explicit decimal quantization. Division is deferred and rejected statically, including division by zero; no engine-dependent infinity. Date equality/order and the explicit rate-validity predicate are supported. No implicit string-to-number conversion or cross-type key comparison. Null comparisons yield unknown; three-valued Boolean logic applies; filters retain only true; case tests unknown as not true. Coalesce is explicit, never automatically used to fill unresolved charges.

All integer arithmetic is checked; overflow fails the run. Decimal arithmetic is exact until an explicit quantization. Addition/subtraction use scale=max(s1,s2) and precision=max(p1-s1,p2-s2)+scale+1; multiplication uses precision=p1+p2 and scale=s1+s2. An inferred precision beyond 38 is rejected statically rather than silently reduced. Quantization names target precision/scale and rounding mode; preview supports half-even and toward-zero. Narrowing overflow fails; there is no saturation or wrap. Avoid declaring every input decimal(38,s) when a narrower business range is appropriate.

The invoice fixture declares quantity decimal(12,4), rate decimal(14,6), billed decimal(18,2); multiply exactly then half-even quantize expected charge to decimal(18,2), subtract billed minus expected at cent scale. Currency is a required explicit code; the fixture uses one currency. Mixed-currency summation/comparison without grouping by currency is rejected. FX, tax, tiering, credits, and discounts are deferred business operations. Positive quantities are an acceptance-fixture constraint, not a universal decimal restriction.

Transport canonical integers and decimal values as tagged decimal strings in portable JSON; decimal tags carry precision/scale. Python may expose native int/Decimal, but conversion must be exact and validated in Rust. JSON numeric literals are not accepted for declared decimal fields unless the adapter preserves their original lexical value and parses it exactly. XLSX numeric cells require an explicit decimal quantization policy because workbook numeric storage cannot establish original monetary precision; source evidence retains the observed cell representation. No zero-copy or exact-original-Excel-decimal claim follows.

Dates accept only explicit ISO YYYY-MM-DD strings or adapter-declared date cells mapped to Gregorian dates. Locale guessing is forbidden; Excel date-system metadata and invalid serials must be handled explicitly by the adapter. Timestamps require caller preprocessing to a declared business date; local timezone/calendar inference is deferred. Rate validity is inclusive lower/exclusive upper date, with explicit open-ended upper bound. No availability-time or bitemporal semantics.

Examples: 2*50 -> 100.00 and 110-100 -> 10.00; 1*45 ->45.00 and 50-45 ->5.00; half-even 1.005->1.00 and 1.015->1.02; null rate yields null expected/variance with unresolved status; Int64 max+1 fails; `null = null` is unknown; empty string is not null; a rate ending 2026-11-12 does not apply on that date. Identical results/errors are required through supported Rust/Python public interfaces. Future executors must prove these semantics before accepting pushdown.


### Decision #8

Source: https://github.com/andrew3stedall/iresiphi/issues/8#issuecomment-6797588137

## Resolution — portable recipe document with runtime bindings

Autonomous decision under the developer's authorization. `.iresiphi` is UTF-8 JSON, not executable Python, a zip, or an engine plan. The root includes `format_version: "0.1"`, human recipe name, source declarations, named relationships, named features, outputs and required capabilities. Stable local IDs identify declarations; references resolve by ID and duplicate IDs fail. JSON object ordering is insignificant; list ordering is significant only where the schema states it (e.g. composite keys and output columns). Unknown semantic properties/versions fail, rather than being ignored; a reserved annotations object may carry nonsemantic metadata.

A source declaration carries projection, field types/nullability, identity, grain, and required adapter capabilities. Relationships carry named paths, typed keys/cardinality, and validity predicates where used. Expressions are a typed data AST. Outputs carry anchor, population/filter stages, fields, explicit grain operations, and detail relationships. Decimal/integer literals use the tagged exact string transport selected in the scalar decision. No arbitrary SQL, Python code, callbacks, engine-specific AST, or dynamic imports.

Reuse uses an in-memory builder/catalog that copies selected declarations into a self-contained recipe. The saved document has no unresolved catalog/network reference: named reusable definitions are bundled, with their content hashes and optional source-catalog provenance. A separately hosted/versioned catalog service is deferred. Conflicting names require explicit aliases; no latest-version lookup on reopening. A recipe may have a human revision label, but its exact semantic identity is a hash of canonical validated document content excluding explicitly nonsemantic annotations. Canonical JSON serialization and its hash algorithm are part of the format specification to implement; choose SHA-256 over recursively key-sorted, compact UTF-8 JSON with only string-encoded numeric literals, no insignificant whitespace, and preserved array order.

Bindings are supplied separately per run: source IDs map to concrete file paths/read handles and adapter settings; sink choice and execution/resource policy are also runtime inputs. Saved recipes contain no secrets, connections, live objects, absolute machine paths, or source data. A run manifest records their safe identities/fingerprints and redacted settings separately. Reopening validates version, references, types and supported capabilities before requesting bindings. Missing bindings have a structured error at bound validation/run, not at unbound recipe validation.

Rust owns validation and serialization. Python builders are ergonomic construction helpers over that same model; loading and saving in either supported surface preserves meaning. Future TypeScript must implement the same portable model and tagged values. Host-only extensions are rejected for this preview, so there is no silent loss of behavior on reopen.

Version compatibility is exact format-version acceptance for the preview. Package patches may fix bugs without changing the file schema; a semantic/schema change increments format version, and old files require an explicit migration tool with reviewable output. No automatic best-effort migration. Required tests are Rust/Python save-load equivalence; duplicate/missing IDs; unsupported version/property/capability; secret-free persistence; and rebinding the unchanged invoice recipe to a second input set.


### Decision #9

Source: https://github.com/andrew3stedall/iresiphi/issues/9#issuecomment-6797608785

## Resolution — explicit validation coverage and inspectable run evidence

Autonomous decision under the developer's authorization. The shared public operations return language-neutral structured records; Python wraps the same core results. Names below define contracts, not implemented APIs.

`validate(recipe)` performs version/reference/type/path/grain/capability validation without opening sources. `validate(recipe, bindings, policy)` additionally inspects source schemas and access/capability feasibility, with `data_checks="none"` by default. Explicit `data_checks="full"` scans the selected source populations for identity, types, relationships and rate validity. There is no sampled mode in the preview. Reports list each check as passed/failed/not-run and its population/coverage; static validity never implies data correctness. All actual runs perform full required checks again against their own read materialization: earlier validation is evidence, not permission to skip checks on changed data.

`explain` exposes logical nodes, named source/field/path dependencies, population and filter stages, grain transitions and measure origins. With bindings/policy it adds feasible local placement, reads/transfers, estimates including unknown values, unsupported capability reasons and budgets. It never executes full data checks implicitly or claims a cost optimum. Explanation and logical lineage use stable declaration IDs rather than unstable engine internals.

`run` returns a run ID, terminal status (`succeeded`, `succeeded_with_unresolved`, `failed`, `cancelled`), output descriptors only for committed outputs, and a manifest/report. The invoice review's unmatched/ambiguous lookup lines yield `succeeded_with_unresolved` and retain one primary row with null dependent values and explicit reason. Missing/duplicate anchor identity, invalid typed data, overflow, policy violations and strict relationship violations fail publication. An empty selected population succeeds with zero counts and no completeness claim about excluded inputs.

Diagnostics include stable code, severity, phase, source/projection or expression/path ID, check ID, affected-output IDs, exact affected-count when known, bounded source-reference samples, and human message/remediation. Values are omitted by default to avoid leaking data; explicit diagnostic-value opt-in is recorded. The default sample cap is 20 references per diagnostic, with truncation and count status explicit. Per-line status and an evidence table preserve the targeted invoice reconciliation beyond these bounded diagnostic samples.

The invoice evidence outputs include source observation reference and original projected values for each line, matched translation/enrichment/rate source references and relevant original values, candidate counts and ambiguity references, and component parent links. Candidate evidence is materialized as a separate table and subject to explicit output/resource bounds; exceeding a bound fails rather than silently claiming complete evidence. It does not promise complete row-contribution provenance for arbitrary aggregate expressions. Logical lineage still identifies all contributing source fields and operations.

Run manifests include recipe hash/format/package versions, safe binding identities and content fingerprints, declared policy, executed nodes, actual source/eligible/output row counts, resolved/unresolved counts and amounts by currency, bytes read/written where measurable, unknown-measurement markers, checks/coverage, timings, and committed output/evidence locations. A source fingerprint refers to bytes actually read or the immutable materialization, not a later reread. There is no shared multi-file snapshot guarantee; concurrent changes detected during access fail or are reported as unreproducible, never silently certified.

Failed/cancelled runs retain bounded diagnostics and a manifest with incomplete checks/counts explicitly marked, plus cleanup evidence. They expose no committed result as successful. The public acceptance suite must cover the agreed tiny invoice fixture, corrupted identities/keys/intervals, unresolved preservation, fanout prevention, save/rebind/reopen parity, and budget/cancellation/sink failures. Real JSON/CSV/XLSX reads and actual output publication are required beyond fixture-only tests. The 100,000-line workload measures complete access-through-delivery behavior; it does not establish a performance guarantee in advance.


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

