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


## Slice 3 fixes and slice 4: verified on both architectures

[PR #34](https://github.com/andrew3stedall/iresiphi/pull/34), commit
`61c5e0cd781df4ff871c720bb4044b7731166932`, passed all eight jobs in
[run 43424967912](https://github.com/andrew3stedall/iresiphi/actions/runs/43424967912)
on 2026-11-13. Both native ARM and x86 jobs passed the complete Rust suite,
formatting, dependency notices and 100,000-line workload with cancellation.
Installed CPython 3.12 and 3.13 wheels passed on both architectures. Node
22.16.0 and 24.11.1 each passed addon and strict TypeScript tests on ARM and x86.
The x86 packaging job built wheels from the sdist, audited them for
manylinux_2_28, and tested offline installs on glibc 2.28. Terraform validation
also passed. No x86 workload ran locally, and no packages were published.

The run retained these artifacts, all tied to that exact commit:

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14037748443 | `ab88cefbb54d47fe57896bf5e75815cc00b296f70afbff4a2fd7424733db614d` |
| x86-workload | 14037580031 | `0851e09642a55c94a8e56be194911125468c392b55920f09bc153d2842d4ba10` |
| arm-workload (including installed ARM wheels) | 14037470633 | `218bfbe2888f3025c991335083bd00f28ca0864641f8ed2c0a5539dabb652b2d` |

GitHub reports an expiry of 2027-02-11 for these artifacts. They must be retained
durably before publication. Local ENOSYS/futex failures during later development
runs were not counted as passes; this hosted run supplies verification for the
reviewed implementation. Evidence for this commit does not certify later heads;
their own PR checks remain authoritative under [the CI rules](../ci.md).

## Slice 5: verified on both architectures

[PR #35](https://github.com/andrew3stedall/iresiphi/pull/35), commit
`a5b55780826716927ba0e40559f4b2addf7cc07e`, passed all eight jobs in
[run 43561011276](https://github.com/andrew3stedall/iresiphi/actions/runs/43561011276)
on 2026-11-14: native ARM and x86 Rust suites with formatting, dependency
notices and the 100,000-line workload, installed CPython 3.12 and 3.13 wheels,
Node 22.16.0 and 24.11.1 on both architectures, x86 wheel audits with clean
installs, and Terraform validation. The Rust, Python and Node suites include the
slice 5 key-transfer tests.

Two earlier heads of the PR failed and are not passes. `7a879c0` failed
`tests/limits.rs` on both architectures: the default-profile expectation did not
list `transfer_keys`. `725748d` failed
`a_run_cancelled_during_a_sqlite_scan_reports_cancelled` on x86 only: a
cancellation observed between sources or in the identity pass carried no source
subject. `a5b5578` attributes it to the source being read.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14100528192 | `f9edd3649b164438c46e0eedbfbc996b5fb6c43b1dd8a416bb75440948c60cfe` |
| x86-workload | 14100500860 | `7395d0f8ae83e38748fb05823e166f0e54cb4ffd8c2511a867cdd1c71ed71193` |
| arm-workload (including installed ARM wheels) | 14100480750 | `429171609e11a7bb9c6eb00c7d3423eec060cb841083297deb310b83be86d154` |

GitHub reports an expiry of 2027-02-12 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Allocation cancellation, member bytes and rename codes: verified on both architectures

[PR #36](https://github.com/andrew3stedall/iresiphi/pull/36), commit
`6fb7b6f52c2ae3d7d216e6a9b139df65817672f9`, passed all eight jobs in
[run 43578207362](https://github.com/andrew3stedall/iresiphi/actions/runs/43578207362)
on 2026-11-14: native ARM and x86 Rust suites with the 100,000-line workload,
Node 22.16.0 and 24.11.1 on both architectures, and x86 wheel audits with clean
installs. The Rust suites include the new allocation cancellation, population
`retained_bytes` and rename-code tests. The full Rust suite was not completed
locally; this run is the only full pass.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14107243438 | `4f1b76f80a82744c7d3d69ff26aed8eb456e036c0a8a2fba7949b7ebd2cc6e0b` |
| x86-workload | 14106967336 | `d874a3b427f1ac7e0d6fed1501bf3bb692dc7c5f09987a4bea98d2d3d6c5c8b7` |
| arm-workload (including installed ARM wheels) | 14107667682 | `408eb9f4f4269a64be9fa38b8ef3f6b6b7e4c4a7134b061ae5f409a6cbe856db` |

GitHub reports an expiry of 2027-02-12 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.
