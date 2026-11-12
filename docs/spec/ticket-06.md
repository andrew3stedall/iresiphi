## Parent

https://github.com/andrew3stedall/iresiphi/issues/16

## What to build

Bound runs, cancellation and atomic local bundle delivery. Deliver observable behavior through the shared public validate/explain/run seam. Use red-before-green public tests, retain decision provenance, and attach actual validation evidence before closing.

## Acceptance criteria

- [ ] Every default budget and caller override is recorded and enforced, including candidate/evidence/result buffers, no spill and RUN_BUSY.
- [ ] Cancellation at parse/join/write returns incomplete-check reports, shuts down owned work and publishes no partial success.
- [ ] Memory results and bundle output share typed schema and complete manifest; CSV escaping round-trips null/empty/backslash text.
- [ ] Owned owner-only staging, approved roots, cleanup/leftover reports, no-replace same-filesystem commit and existing-destination retry satisfy decision13.
- [ ] Before/after commit interruption behavior and unknown-size overflow are tested through visible outputs.
- [ ] Every normative clause in the applicable contracts below is implemented or explicitly reported unmet; the checklist does not narrow those contracts.

## Blocked by

- https://github.com/andrew3stedall/iresiphi/issues/20

## Applicable contracts (self-contained)

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


### Decision #10

Source: https://github.com/andrew3stedall/iresiphi/issues/10#issuecomment-6797627647

## Resolution — batch contracts and explicit local placement

Autonomous decision under the developer's authorization. Preview execution placement is explicitly local. No remote source pushdown, remote staging, automatic federation or pairwise connector code ships. The following interface responsibilities preserve future execution choices without pretending those integrations exist.

A source adapter describes projection schema/nullability, stable binding identity and fingerprint support, record-reference production, supported extraction options, estimates with known/unknown status, and access/cancellation limits. It opens a bounded pull-based stream of typed column batches plus source references, validating declared values and preserving raw evidence fields. `close` releases owned resources on all terminal paths. No adapter may silently infer business keys, paths, grain, rate policy or monetary rounding. Batch transport is an internal versioned logical schema; an engine-specific dataframe is not the public adapter contract. Chosen bindings may implement Arrow interchange internally subject to the binding decision.

Capability declarations include supported logical operations, type/precision/null/date semantics, projection/filter restrictions, accessible locations, transfer directions, ordering guarantees, cancellation granularity and resource-accounting limits. They are negotiated against the exact required plan, not a Boolean 'supports SQL'. Unknown capability is unsupported until established. Source scanning/projection does not imply source-side aggregation support.

The logical plan contains scan/project, typed feature evaluation, checked named lookup including date validity, population/filter stages, grouped aggregate, validated broadcast, separate detail/evidence production, and output delivery. An executor accepts only a plan whose exact operations and semantics it can preserve; it returns batches, execution/check evidence and structured errors. Iresiphi owns semantic validation, unresolved-status propagation, identity/cardinality contracts and public lineage; lowering must not discard them. The engine decision selects a concrete implementation behind this seam.

A sink describes accepted types, delivery location, bounded-batch consumption, commit/abort semantics and ownership. Preview sinks are memory and local directory bundles containing primary/detail/evidence CSV or JSON plus schema/manifest. CSV requires accompanying type schema and an unambiguous null representation/escaping policy so empty strings differ from null; no bare CSV is claimed to be lossless. Output ordering is unspecified unless an explicit supported order is requested; the preview does not provide a general sort operation. Stable record IDs permit comparisons independent of order.

Planning takes the transitive closure of requested outputs and evidence, including hidden match keys, invoice date, validity endpoints, population/filter predicates and grain checks. Unrequested sources need not be read, but required validation columns cannot be projection-pruned. Policy declares allowed read roots, allowed output/temp roots, local transfer permission and resource limits. Unknown sizes remain unknown and require runtime enforcement; estimates never override hard policy. Shared reads/calculations may be reused only where population and semantic identity agree.

Concrete supported composition: supplier-specific invoice JSON projections, XLSX enrichment, CSV mappings/rates/components all stream through the same source contract into the local executor and produce one invoice-line output plus detail/evidence tables. No JSON-to-XLSX-specific join implementation is needed. Policy-blocked scenario: a required XLSX file lies outside allowed read roots, or the output bundle lies outside the approved destination; bound explain/validate reports infeasibility and run refuses before unauthorized access. A future remote-only source with local transfer prohibited is also infeasible; there is no hidden download fallback.

Future remote/hybrid extensions must declare same-environment accessibility, exact supported semantics, transfer/staging permission and ownership, estimates/uncertainty, cancellation and cleanup behavior, and sink capability. A planner may choose among only semantically equivalent feasible placements. Adding a new source must not require pairwise adapters for existing formats. Detailed remote integration remains deferred by scope, not unresolved preview fog.


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

