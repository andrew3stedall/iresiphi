# Preview implementation status

All existing implementation and research branches were consolidated into `main`.
The Rust/Python 0.1.0 invoice-review preview is implemented; no packages have been
published. [CI](ci.md) governs processor placement: x86 work runs in GitHub
Actions, while ARM-compatible development work may run locally on ARM. A change
counts as passing only when it passes on both ARM and x86 at the same commit; see
[Required passes](ci.md#required-passes-arm-and-x86).

## Implementation tickets

Tickets [#17](https://github.com/andrew3stedall/iresiphi/issues/17) through
[#23](https://github.com/andrew3stedall/iresiphi/issues/23) have implementations
and public Rust/Python tests on `main`. Follow-up integration work has fixed:

- Bundles overwriting outputs whose names collide with internal metadata tables.
- Currency checks missing predicates or incorrectly combining independent
  branches, and mixed source-value errors inheriting the first error's code.
- Uninterruptible whole-document JSON execution parsing.
- Missing Python access to the safe abandoned-staging cleanup API.
- Delivery timing being added to a manifest already committed to disk.

The final source-reader changes add explicit capabilities to bound explanations
and source-read manifests, and inspect JSON schema through borrowed spans rather
than decoding invoice records. PR #25 merged after its full x86 workflow passed;
the [evidence record](measurements/ci-x86.md) identifies the tested commits.

[#24](https://github.com/andrew3stedall/iresiphi/issues/24) adds the executable
walkthrough, measured 100,000-line workload, notices and distributions. The first
x86 wheel/audit/clean-install run and the complete PR #25 Rust, packaging and
workload run passed. Evidence is linked in the
[x86 record](measurements/ci-x86.md); known contract limitations remain in
[unmet.md](measurements/unmet.md).

The subsequent consolidation adopts the shared requirement for ARM and x86
passes at the same commit, and declares the workload's 1 GiB byte limits during
fixture generation. Its own architecture passes gate completion; issue states
and workflow results record the current acceptance outcome.

Both architectures have hosted CI jobs at the exact PR head. Native ARM CI
provides the full workload verification when a local run cannot finish; local
ARM development remains available. Neither interrupted local workload attempt
at `72ffd6c` is counted as a pass.

## Slice 3: SQLite source and source placement

[docs/spec/slice-3.md](spec/slice-3.md) adds a SQLite table as a source and lets an
output's anchor eligibility run at that source under `policy.placement`
([sqlite.md](sqlite.md)). It is implemented with tests in `tests/sqlite.rs`. The
Python and Node suites exercise the SQLite/CSV fixture through validate, explain
and run, including placement parity and decimal fallback/refusal. The Rust
adversarial suite covers anchor relationship guards, field byte limits, unknown
collations, parameter limits, validation coverage, UTF-16, generated rowid
shadowing and safe static expression depth. UTF-8 validation covers scanned text
only under source placement, as disclosed in the contract and manifest.

Local ARM verification of the slice 3 fixes is recorded in
[unmet.md](measurements/unmet.md). The integrated slice 3 fixes and slice 4 implementation passed all eight hosted
checks at `61c5e0c`, including both architecture and runtime matrices; see the
[verified run](measurements/ci-x86.md#slice-3-fixes-and-slice-4-verified-on-both-architectures).
Slice 4 implementation is described below.

## Slice 4: same-database lookup reduction

[Slice 4](spec/slice-4.md) shares an independently admitted SQLite snapshot
between bindings with byte-for-byte equal declared paths, under every placement.
It adds many-to-one target reduction to the delivered from keys, including a
pushed eligibility where applicable. Connected candidate groups finish their
whole-table preflight before any filtered scan; a check failure falls the entire
group back locally. Matching and evidence remain in the executor.

The Rust acceptance and parity cases are in `tests/sqlite.rs`; Python and Node
exercise the co-located fixture through their public validate/explain/run APIs.
The manifest and bound validation disclose scanned-only target text checks.
Hosted ARM/x86, pinned Python and Node matrices, workloads and packaging gates
passed at `61c5e0c`. [The evidence record](measurements/ci-x86.md#slice-3-fixes-and-slice-4-verified-on-both-architectures)
links the run and immutable artifact identities. Later heads require their own
passing checks. Local evidence and remaining contract limitations are recorded
in [unmet.md](measurements/unmet.md).

## Slice 5: key transfer to a lookup target

[Slice 5](spec/slice-5.md) adds the first transfer. Under `transfers: keys` and a
placement other than `local`, a many-to-one lookup whose target is a SQLite table
and whose other end is a file or a different database sends the distinct keys of
the delivered `from` records to the target as bound parameters, and the target
returns only matching records. Transfers are off by default; `require_source`
demands one only when the policy permits it. Explain lists planned transfers
without opening a source, and the manifest records key counts, never values.

Rust acceptance and parity cases are in `tests/transfer.rs`; Python and Node run
the CSV-orders/SQLite-customers fixture through their public APIs. Hosted
ARM/x86, Python, Node and packaging gates passed at `a5b5578`; see the
[verified run](measurements/ci-x86.md#slice-5-verified-on-both-architectures).
Remaining limitations are in [unmet.md](measurements/unmet.md).

## Slice 6: evidence for problem lines by default

[Slice 6](spec/slice-6.md) records a developer decision: the library describes how
datasets connect and should not have to emit much data. A run now records
primary, candidate and detail evidence only for lines that did not resolve. A
policy asks for the earlier per-line trace with `evidence: full`. Output rows,
counts, totals and diagnostics are the same under both levels. The 100,000-line
workload no longer raises its byte limits.

Rust cases are in `tests/evidence.rs`; Python and Node check the default level on
the two-supplier fixture. The earlier suites state `evidence: full` and keep
their expectations. Hosted verification is recorded in
[the evidence record](measurements/ci-x86.md) once it has run at the commit.

## Remaining scope

The broad product specification
[#1](https://github.com/andrew3stedall/iresiphi/issues/1) remains a roadmap
umbrella. Later slices added Node bindings, allocation, a SQLite source with eligibility filtering and same-database
lookup target reduction, and key transfer to a SQLite lookup target. Still deferred: remote and dataframe connectors, other
pushdown, other transfers, staging and wider hybrid execution, and other platforms. These
require their own bounded delivery specifications; they are not claims of this
preview.

Before any package publication, recheck crate/PyPI names and account ownership,
retain the exact release commit's CI artifacts durably, and review its dependency
inventory. Publication is outside the current implementation request. The
100,000-line workload runs under the default limits and evidence level; with
`evidence: full` its evidence volume needs 1 GiB output/retention limits
(`generate_fixture.py --evidence full --byte-limit 1073741824`). Process RSS remains a
measurement, not a hard memory bound.
