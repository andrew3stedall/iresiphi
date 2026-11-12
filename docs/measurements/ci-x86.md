# x86 CI evidence

All x86 processing runs in [GitHub Actions](../../.github/workflows/ci.yml).
Native ARM builds and tests remain useful for local development; only the x86
workflow provides evidence for the preview distribution target.

## Complete passing preview run

[Run 43215391634](https://github.com/andrew3stedall/iresiphi/actions/runs/43215391634)
passed all three jobs for PR #25 head
`540b1e435b919f060044cc30c7863bacd6d88cc3`, tested as merge commit
`3bc182aa76be8849ff330d77ba4844ed00d280e8`. PR #25 merged as
`b76a9ed9a0bf91462278ccf233a5bda320fabbeb`.

- Terraform formatting and validation passed.
- All 229 Rust tests passed, including the Rust/Python parity golden.
- The sdist and separate cp312/cp313 manylinux_2_28 wheels built and passed
  auditwheel checks. Each interpreter passed 33 Python tests after an offline
  wheel installation in a fresh glibc 2.28 container.
- The complete workload processed 100,000 invoice lines, 2,000 invoices and
  150,000 observed components under the recorded 1 GiB output/retention limits.
  It preserved 99,999 resolved lines and one unresolved line, and verified exact
  totals against the deterministic fixture oracle.

The workload took 29,276,631 microseconds to run and deliver its bundle. It read
13,464,645 bytes, wrote 815,130,525 bytes and reported process peak RSS of
8,678,895,616 bytes. A separate cooperative cancellation measurement returned
990 microseconds after its 50 ms cancellation request. These are observations
of this run, not performance or memory guarantees.

The [provenance record](x86-540b1e4/provenance.json), full run/cancellation
manifests, fixture hashes, policies and environment details are retained beside
this document. Distribution artifacts remain attached to the linked workflow
run. Subsequent changes require their own passes on both architectures.

## First verified packaging run

[Run 43209653236](https://github.com/andrew3stedall/iresiphi/actions/runs/43209653236),
commit `f4498b4fab0c9c8bd9a9acbd1fee515747c433d2`, successfully built an sdist and
separate CPython 3.12 and 3.13 x86 wheels from its extracted contents. Both wheels
passed `auditwheel show` and `auditwheel repair --plat manylinux_2_28_x86_64`,
then offline installation and the Python acceptance, ownership and cancellation
suites in fresh containers reporting glibc 2.28. Nothing was published.

The build and fresh test containers use this immutable image:

```text
quay.io/pypa/manylinux_2_28_x86_64@sha256:9555ba6c479fa69a2aa80ad1dc57154a60d562b19df159361d9379a02beb1e36
```

The `x86-distributions-<commit>` artifact contains distributions, SHA-256 checksums,
image inspection metadata, build and audit logs, archive inventories and clean
installation logs. Tests use installed wheels without the source package on
`PYTHONPATH`, and fresh installation containers have networking disabled.
The compiler is Rust 1.94.0 and the builder is maturin 1.16.0. Auditwheel and
interpreter versions are recorded by the pinned image and job logs.

That run's Rust job failed an existing test comparing the committed manifest
with post-commit delivery timing. The wheel success is packaging evidence for
that commit; it does not claim the whole run passed or certify subsequent code.
Later commits rerun the gates.

## Workload evidence

The `x86-workload-<commit>` artifact records hardware/software, immutable fixture
seed and hashes, the workload policy, full run manifests, phase
times, tracked memory, measured process RSS, I/O and cancellation latency.
The script verifies exact expected totals and resolved/unresolved populations.

The library's default profile refuses this fixture's evidence volume, so the
workload's policy declares 1 GiB output and retained-buffer limits
(`generate_fixture.py --byte-limit 1073741824`), leaving other limits at their
defaults. [The ARM measurement](workload-100k.md) describes
the same admission limitation; neither it nor the x86 measurement promises a
hard RSS ceiling or a fixed latency.

Workflow artifacts expire according to GitHub's retention policy. Before any
publication, retain the exact release commit's artifacts durably and recheck
registry names and account ownership. No package publication is automated here.
