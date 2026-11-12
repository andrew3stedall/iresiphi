# Contract clauses not met by the 0.1.0 preview

Ticket #24 binds decisions #3, #4, #11, #12, #13 and #14 (`docs/spec/ticket-08.md`). This file lists each normative clause that is not fully met at the end of the ticket, and why. A clause missing from this list is implemented and covered by a public test; the last section names those tests by decision.

A clause is blocked when it needs something this machine or this ticket cannot provide, partly met when the behaviour exists but does not cover the whole clause, unmet when nothing implements it, and unverified when it is implemented but no test exercises it.

## Packaging and release (decisions #4, #12, #14)

| Clause | Status | Reason |
| --- | --- | --- |
| Separate cp312/cp313 wheels for Linux x86_64 manylinux_2_28 (glibc 2.28 or later), built with a pinned build-image digest and maturin 1.16.0 | Blocked | This machine is aarch64 under PRoot, with no Docker, Podman, qemu-x86_64 or x86_64 cross compiler. The wheels built here are `linux_aarch64`. Their native module needs symbol versions up to `GLIBC_2.43` (for example `acosf`, `pthread_create@GLIBC_2.34`, `hypot@GLIBC_2.35`), so a wheel built on this host could not be relabelled manylinux_2_28. No build-image digest exists to record. See [packaging.md](packaging.md). |
| Audit native wheel dependencies (auditwheel) and test minimum-glibc installation and import on clean images | Blocked | Same reason. The aarch64 module was inspected with `readelf` and `ldd` only: it needs `libgcc_s.so.1`, `libm.so.6`, `libc.so.6` and the dynamic loader, and has no RPATH or RUNPATH. |
| Clean-wheel installation as a release gate (decision #12) | Partly met | The cp312 and cp313 wheels install offline into fresh `uv` virtual environments, and the Python suites pass against them. These are fresh environments on the build host, not clean images. |
| Linux x86_64 as the initial platform (decision #4) | Blocked | Nothing was built or run on x86_64. |
| Before publication, recheck crate and PyPI names and account ownership | Unmet | Publication is outside this ticket. Nothing was published, and the names were not rechecked. |
| Parallel standards and spec reviews completed and findings fixed (ticket acceptance) | Unmet | Not done by the implementation agent. The coordinator runs reviews. |

## Execution, cancellation and resources (decisions #11, #12, #13)

| Clause | Status | Reason |
| --- | --- | --- |
| Cancellation is checked between parsing chunks | Partly met | CSV checks between records, and the JSON depth preflight every 64 KiB. After the preflight, each JSON document is parsed whole by `serde_json` twice (duplicate-key check, then the value tree), with no check inside either call. XLSX checks between preflight chunks and rows, but calamine's workbook and sheet parsing cannot be interrupted. Decision #12 allows the XLSX delay; the JSON parse is a gap. A streaming JSON reader would close it. |
| Bound validation with `data_checks = "none"` inspects schemas without decoding records | Partly met | CSV reads only the header and XLSX only up to the header row, but the JSON check parses the whole document to resolve the array and header pointers. No records are decoded or type-checked, but the full document is read and parsed. |
| XLSX preflight and decompression accounting before range allocation | Met, with a limitation | The preflight decompresses and counts every part, including sheets the binding does not name. A large unrelated sheet can exceed `xlsx_bytes` or `worksheet_cells` and fail the run. This is stricter than the clause, not weaker. |
| Reject cross-device or unsupported publication | Unverified | `renameat2(RENAME_NOREPLACE)` failing with `EXDEV` maps to `CROSS_DEVICE`, and `ENOSYS`, `EINVAL` or `EOPNOTSUPP` to `PUBLICATION_UNSUPPORTED`. Staging is a sibling of the destination, so `EXDEV` needs a mount point between them. Every path here is on one device (`stat` reports device 65105 for `/tmp` and the repository), and creating a mount needs privileges this machine does not grant. |
| Manifest separates tracked reservations from measured peak RSS | Met, with a limitation | `peak_rss` is the process's `VmHWM`, a lifetime high-water mark for the whole process. It is not per run, and it includes earlier runs and anything else in the process. A per-run figure would need a separate process or cgroup, which decision #13 rules out as a hidden mechanism. |
| Measure and report cancellation latency | Partly met | `examples/workload.rs` measures the time from a cancel request to return for one 100,000-line run ([workload-100k.md](workload-100k.md)). The suites measure it for small runs. No latency bound is promised. |
| Internal batches are Arrow `RecordBatch` with declared schema, Decimal128/Date32 and validity bits (decision #12) | Partly met | Adapters convert each source to typed Arrow batches of at most `batch_rows` rows. The executor then keeps the observations as rows, and its checked arithmetic, quantization and aggregation run in Rust over those rows. Only lookup candidate pairs go through DataFusion. This is the semantic boundary decision #11 allows, but calculations do not run over Arrow arrays. |
| Source adapters report schema, identity, capabilities and location (decision #4) | Partly met | Adapters report schema (bound validation), binding identity and fingerprint, and per-record location. There is no capability descriptor per adapter. Capability is the fixed set of three built-in adapters, and an unknown adapter fails with `UNSUPPORTED_CAPABILITY`. |
| Explicit cleanup command for abandoned staging | Partly met | Rust exposes `iresiphi::sink::cleanup_staging`. The Python package does not expose it yet. |

## Closed in this ticket

These gaps from earlier tickets were fixed, with tests in `tests/preview.rs` and `tests/aggregation.rs`:

- Reader type errors now carry the exact count of affected records and up to 20 record references, for JSON, CSV and XLSX.
- A line expression that adds, subtracts or compares amounts with different currencies fails with `MIXED_CURRENCY`.
- A grouped output whose anchor declares no currency can no longer sum, min or max amounts from a source that declares one.
- Every run asserts output population and identity after execution (`output.population`), as decision #11 requires.
- Bundles include the reconciliation table that decision #4 lists (`_reconciliation`).
- The run manifest records each workload phase (`phases_us`), and the 100,000-line workload is measured.

## Where the met clauses are tested

| Decision | Public tests |
| --- | --- |
| #3 invoice workflow and acceptance | `tests/acceptance.rs`, `tests/review.rs` (160/145/15, 180 and 20, unresolved and ambiguous lines, component fanout, rebinding), `tests/python_acceptance.py`, `tests/python_parity.rs` |
| #4 bounded local preview | `tests/json_csv.rs`, `tests/xlsx.rs`, `tests/adapters.rs` (real files, strict dialects, formula caches), `tests/bundles.rs`, `tests/preview.rs` |
| #11 DataFusion behind the semantic plan | `tests/scalar.rs` (38-digit precision, ties, overflow), `tests/review.rs` (candidate counting before selection), `tests/limits.rs` (engine pool with no spill), `tests/preview.rs` (population invariant) |
| #12 PyO3 handles and exact values | `tests/python_handles.py`, `tests/python_cancellation.py`, `tests/python_acceptance.py` |
| #13 admission, cancellation, commit-only outputs | `tests/limits.rs`, `tests/delivery.rs` (budgets, cancellation at parse, join and write, cleanup failure, existing destination, interruption before and after commit), `tests/bundles.rs` (CSV `\N` escaping) |
| #14 evidence-gated preview | `examples/run.rs`, `examples/run.py`, [workload-100k.md](workload-100k.md), [packaging.md](packaging.md), `THIRD_PARTY_NOTICES.md` |
