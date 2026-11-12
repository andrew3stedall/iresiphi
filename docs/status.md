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
Python builder and the Node type declarations accept the new adapter and policy
field; neither binding has its own SQLite test yet. Hosted ARM and x86 passes at
the same commit are required before this counts as passing.

## Remaining scope

The broad product specification
[#1](https://github.com/andrew3stedall/iresiphi/issues/1) remains a roadmap
umbrella. Later slices added Node bindings, allocation and a SQLite source with one
pushdown operation. Still deferred: remote and dataframe connectors, other
pushdown, transfers and hybrid execution, and other platforms. These
require their own bounded delivery specifications; they are not claims of this
preview.

Before any package publication, recheck crate/PyPI names and account ownership,
retain the exact release commit's CI artifacts durably, and review its dependency
inventory. Publication is outside the current implementation request. The
100,000-line workload's policy declares 1 GiB output/retention limits
(`generate_fixture.py --byte-limit 1073741824`); the default 256 MiB profile
refuses its evidence volume. Process RSS remains a
measurement, not a hard memory bound.
