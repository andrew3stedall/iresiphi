# Decision history 13

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

## Question

What observable execution and failure guarantees can the preview make with the chosen engine and bindings?

Specify memory/batch/concurrency budgets, backpressure, cancellation, stateful-operation behavior, and spill/partition/rejection choices for oversized work. Define what happens when statistics are unknown and prevent silent full collection or transfer that violates policy.

For any supported staging, specify permission, ownership, lifetime, cleanup on success/failure/cancellation, and evidence of leftovers. Explicitly record staging as deferred if not supported. Define sink delivery and partial-output behavior, including whether output is atomic, how incomplete output is identified, and whether retry is safe for the supported sinks.

Separate the guarantees controlled by Iresiphi from integration limitations. State public diagnostics and evidence required to test each guarantee without private scheduling assertions. Do not imply shared snapshots or transactions where sources and sinks cannot provide them.


https://github.com/andrew3stedall/iresiphi/issues/13#issuecomment-6797720274

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
