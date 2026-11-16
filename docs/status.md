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
their expectations. Hosted ARM/x86, Python, Node and packaging gates passed at
`a838be5`, the first hosted workload run under the default limits; see the
[verified run](measurements/ci-x86.md#slice-6-verified-on-both-architectures).

## Slice 7: deferred pipelines and bounded execution

[Slice 7](spec/slice-7.md) records a developer decision: Iresiphi should work like
other data platforms, where a caller builds a pipeline and nothing runs until it
is called, and a run should not hold every record in memory. It is delivered in
phases. Phase 1 adds `Pipeline` (Rust), `iresiphi.plan` (Python) and `plan`
(Node), which bind a recipe to its sources and policy without opening them. It
also stores each source's field names once instead of in every record, shares
source identity strings across records, streams lookup candidates out of the join
engine, and counts byte budgets without building serialized copies. Results and
serialized bytes are unchanged. Before phase 1 the 100,000-line workload held
1.11 GB right after reading its 350,004 records; on hosted x86 its process peak
fell from 1.41 GB to 0.84 GB at `b1774bc` (see the
[verified run](measurements/ci-x86.md#slice-7-phase-1-verified-on-both-architectures)).

Phase 2 streams delivery. A bundle run approves its destination before reading
any source, writes each output and evidence row to staging as it is produced,
and accumulates counts, totals and the result size as rows pass. Its report
carries no tables or evidence; the bundle does. Because it keeps no results,
`limits.retained_bytes` no longer bounds a bundle run's results, while
`limits.output_bytes` still does. Memory runs are unchanged. On hosted x86 the
workload's bundle matched phase 1 outside its manifest and its peak fell to
0.82 GB at `1a80353` (see the
[verified run](measurements/ci-x86.md#slice-7-phase-2-verified-on-both-architectures));
source records still set the peak.

Phase 3 streams anchors. A source read only as the anchor of outputs is checked
in full on its first read, which keeps only each record's identity and
relationship key fields and its reference. Lookups and the checks over every
anchor record run over those key records unchanged, and each output decodes
the source's bytes again in batches. `limits.retained_bytes` counts the key
records. Results, evidence, diagnostics and bundle tables are unchanged.
On hosted x86 the workload's results
matched phase 2 and its peak fell to 0.71 GB at `8f2c7d9` (see the
[verified run](measurements/ci-x86.md#slice-7-phase-3-verified-on-both-architectures)).
Lookup targets, detail sources and populations are still held whole and now set
the peak; spilling targets larger than memory is phase 4.

Phase 4 spills lookup targets. A target whose records pass `limits.target_bytes`
(16 MiB by default) keeps only its identity and relationship key fields and its
reference. Its records go to a temporary file that has no name in any directory;
under placement `prefer_source` or `require_source`, a SQLite target is queried
again at its snapshot by rowid instead. Each batch of anchor lines reads the
target records it reaches. `limits.retained_bytes` counts the key records.
Results, evidence, diagnostics and bundle tables are unchanged. Detail sources
and populations are still held whole. On hosted x86 the workload's
100,000-record enrichment workbook spilled, its results matched phase 3, and
its peak fell from 0.71 GB to 0.44 GB at `067b31f` (see the
[verified run](measurements/ci-x86.md#slice-7-phase-4-verified-on-both-architectures)).

Phase 5 spills detail sources. The developer chose it as the next item on
2026-11-17. A one-to-many detail over `limits.target_bytes` keeps its key records
and goes to the same stores as a phase 4 target. So does any relationship target
an output anchors at, and an output anchored at a spilled source, a detail
output included, reads its own records back in batches. A spilled detail can be
the parent of another detail. Populations, allocation quantities and the sources
an allocating or population output reaches are still held whole. Cases are in
`tests/spill.rs`; hosted verification is pending.

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
