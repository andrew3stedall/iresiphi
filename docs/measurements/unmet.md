# Contract clauses not met by the 0.1.0 preview

Ticket #24 binds decisions #3, #4, #11, #12, #13 and #14 (`docs/spec/ticket-08.md`). This file lists each normative clause that is not fully met at the end of the ticket, and why. A clause missing from this list is implemented and covered by a public test; the last section names those tests by decision.

A clause is blocked when it needs something this machine or this ticket cannot provide, partly met when the behaviour exists but does not cover the whole clause, unmet when nothing implements it, and unverified when it is implemented but no test exercises it.

## Packaging and release (decisions #4, #12, #14)

The initial x86 wheel build, audits and fresh glibc 2.28 installs are now verified
by [GitHub Actions](ci-x86.md). The workflow reruns these gates for later commits.
Standards/spec reviews identified output-name, currency-scope and diagnostic-count
errors; regression fixes are on main. Source-capability and schema-inspection
changes passed the complete PR #25 workflow. Later changes still require their
own ARM and x86 evidence before acceptance.

| Clause | Status | Reason |
| --- | --- | --- |
| Before publication, recheck crate and PyPI names and account ownership | Unmet | Publication is outside this ticket. Nothing was published, and the names were not rechecked. |

## Execution, cancellation and resources (decisions #11, #12, #13)

| Clause | Status | Reason |
| --- | --- | --- |
| The generated 100,000-line fixture is designed within the declared profile, and complete-recipe measurements gate release (decision #13) | Met by a declared profile | The workload's policy now declares 1 GiB `output_bytes` and `retained_bytes`, and the run passes on ARM under it (run B). The x86 pass is recorded separately in [ci-x86.md](ci-x86.md). The library defaults remain 256 MiB. The fixture's inputs (13.5 MB, 350,004 records) are well inside the input limits, but the run's results and evidence serialize to 757,303,499 bytes, about 2.8 times the 256 MiB `output_bytes` and `retained_bytes` defaults. Under the default profile the run fails with `RESOURCE_LIMIT` before publishing. It succeeds only with both limits raised explicitly to 1 GiB, and then peaks at 5.14 GB of process RSS. Each primary evidence record repeats the full candidate observations that the candidate evidence table also holds; slimming that is a format change for earlier tickets' evidence contract, so it was left for a decision. See [workload-100k.md](workload-100k.md). |
| XLSX preflight and decompression accounting before range allocation | Met, with a limitation | The preflight decompresses and counts every part, including sheets the binding does not name. A large unrelated sheet can exceed `xlsx_bytes` or `worksheet_cells` and fail the run. This is stricter than the clause, not weaker. |
| Reject cross-device or unsupported publication | Unverified | `renameat2(RENAME_NOREPLACE)` failing with `EXDEV` maps to `CROSS_DEVICE`, and `ENOSYS`, `EINVAL` or `EOPNOTSUPP` to `PUBLICATION_UNSUPPORTED`. Staging is a sibling of the destination, so `EXDEV` needs a mount point between them. Every path here is on one device (`stat` reports device 65105 for `/tmp` and the repository), and creating a mount needs privileges this machine does not grant. |
| Manifest separates tracked reservations from measured peak RSS | Met, with a limitation | `peak_rss` is the process's `VmHWM`, a lifetime high-water mark for the whole process. It is not per run, and it includes earlier runs and anything else in the process. A per-run figure would need a separate process or cgroup, which decision #13 rules out as a hidden mechanism. |
| Measure and report cancellation latency | Met, with a finding | `examples/workload.rs` measured two 100,000-line runs ([workload-100k.md](workload-100k.md)): 0.31 s from request to return during parsing, and 25.2 s during output building. The second delay was not diagnosed; freeing the partly built evidence under swap is the likely cost. No latency bound is promised. |
| Internal batches are Arrow `RecordBatch` with declared schema, Decimal128/Date32 and validity bits (decision #12) | Partly met | Adapters convert each source to typed Arrow batches of at most `batch_rows` rows. The executor then keeps the observations as rows, and its checked arithmetic, quantization and aggregation run in Rust over those rows. Only lookup candidate pairs go through DataFusion. This is the semantic boundary decision #11 allows, but calculations do not run over Arrow arrays. |

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
