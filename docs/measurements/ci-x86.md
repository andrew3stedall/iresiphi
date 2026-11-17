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

## Slice 6: verified on both architectures

[PR #37](https://github.com/andrew3stedall/iresiphi/pull/37), commit
`a838be548dd417a8f7c4d53a336dda13220560b0`, passed all eight jobs in
[run 43589353015](https://github.com/andrew3stedall/iresiphi/actions/runs/43589353015)
on 2026-11-14: native ARM and x86 Rust suites with formatting, dependency
notices and the 100,000-line workload, installed CPython 3.12 and 3.13 wheels,
Node 22.16.0 and 24.11.1 on both architectures, x86 wheel audits with clean
installs, and Terraform validation. The suites include the slice 6
evidence-level tests.

This is the first hosted workload run under the default resource profile:
`scripts/ci/workload.sh` no longer raises `output_bytes` or `retained_bytes`.
On x86 the run succeeded with one unresolved line, serialized 52,133,436 bytes of
results and evidence, wrote a 5,161,798-byte bundle and recorded a process peak
RSS of 1,411,559,424 bytes; `run_bundle` took 7.6 s. A cancellation requested
50 ms in returned after 2.1 ms.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14112137862 | `9e0a88ee3773636b6753bd57ee1e16a3ec27d0274d2696e4ec2e32852db9f627` |
| x86-workload | 14111342536 | `b29ace161a8ca49ace4ca8e1be59bee2ad410bf852a77a0fa3450066b39d1850` |
| arm-workload (including installed ARM wheels) | 14111723610 | `d55811ce920b51eb1770d7a091f5fae09e3368495412b4a4ed88fead61a4d00f` |

GitHub reports an expiry of 2027-02-12 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Slice 7 phase 1: verified on both architectures

[PR #38](https://github.com/andrew3stedall/iresiphi/pull/38), commit
`b1774bc93014259ec9d26107ac11e36bea1a0175`, passed all eight jobs in
[run 43651658874](https://github.com/andrew3stedall/iresiphi/actions/runs/43651658874)
on 2026-11-14, including the Rust, Python and Node suites with the deferred
pipeline tests. The first head, `47e1b09`, failed every Node job before any
test ran: the Node addon's own `Cargo.lock` did not list the new `futures`
dependency. `b1774bc` adds it.

The x86 workload under the default profile matches slice 6 byte for byte in
serialized results (52,133,436 bytes) and bundle size (5,161,798 bytes), with
process peak RSS down from 1,411,559,424 bytes at `a838be5` to 840,863,744 bytes.
`run_bundle` took 6.6 s. A cancellation requested 50 ms in returned after 1.0 ms.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14136394836 | `792ee1a59357f0eae43759b927e7e1c7053e4730508a76e0998c60dd6076078b` |
| x86-workload | 14136981353 | `b4f48d3c6c9153dd2e4843a4672d14c23d8987d67351893504b0f9b2acc4598b` |
| arm-workload (including installed ARM wheels) | 14136883117 | `8934d679556c5bd9ea79c2fc4269099d52940ff8db0b635a087260f04150ab6f` |

GitHub reports an expiry of 2027-02-12 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Slice 7 phase 2: verified on both architectures

[PR #40](https://github.com/andrew3stedall/iresiphi/pull/40), commit
`1a80353d04e194ea030270da1fb27fc80bc5f912`, passed all eight jobs in
[run 43700888070](https://github.com/andrew3stedall/iresiphi/actions/runs/43700888070)
on 2026-11-15, including the new streamed-delivery cases in `tests/bundles.rs`.

The workload streams its bundle under the default profile. Serialized results
are unchanged at 52,133,436 bytes. The bundle wrote 5,161,799 bytes on x86 and
5,161,796 bytes on ARM, against 5,161,798 at phase 1. Every byte outside
`manifest.json` (5,141,918) is the same on both architectures and at phase 1;
the differences are run timings, run IDs and memory figures in the manifest.
Process peak RSS fell from 840,863,744 bytes at phase 1 to 819,257,344 bytes
on x86 (813,649,920 on ARM). Source records still set the peak; phase 3 is
what lowers it. `run_bundle` took 7.8 s on x86 and 6.1 s on ARM. A cancellation
requested 50 ms in returned after 1.1 ms on x86 and 0.9 ms on ARM.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14156672447 | `0054b4c1fa34dfe9d73a0eaeacaf4c8832f84c389a055b102b182d7576cb0c22` |
| x86-workload | 14156386667 | `43620c04d735416af4794d9245c791fcd302418b765ec862cf64a6072213a62e` |
| arm-workload (including installed ARM wheels) | 14156014915 | `1d3a8ab87c1d031b1a4f51e7ae16e6c5573114c272d8935984b4adb2147188d4` |

GitHub reports an expiry of 2027-02-13 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Slice 7 phase 3: verified on both architectures

[PR #41](https://github.com/andrew3stedall/iresiphi/pull/41), commit
`8f2c7d9fb4819e6925d0c4a68a491038541b7594`, passed all eight jobs in
[run 43714425240](https://github.com/andrew3stedall/iresiphi/actions/runs/43714425240)
on 2026-11-15, including the new streamed-anchor cases in `tests/streaming.rs`.

The workload's two JSON documents (`a`, `b`) streamed: each kept `date`, `id`,
`invoice`, `raw` and `supplier` of its 50,000 records and was read twice. The
lookup targets and the component detail were held. Results match phase 2:
serialized results and evidence are unchanged at 52,133,436 bytes, with the
same counts (99,999 resolved, 1 unresolved), evidence records (1 / 1 / 2) and
one `LOOKUP_UNMATCHED` warning. The bundle wrote 5,162,328 bytes on both
architectures, 529 more than phase 2 on x86; the two new `streamed` read records
in `manifest.json` add 558 bytes before timing digits vary. The workload
artifacts do not keep the bundle files, so the tables were not compared byte for
byte here; `tests/streaming.rs` compares bundles across batch sizes.

Process peak RSS fell from 819,257,344 bytes at phase 2 to 709,414,912 bytes
on x86 (813,649,920 to 702,570,496 on ARM). The held targets (100,000 workbook
rows and 150,000 components) now set the peak; phase 4 addresses them.
`run_bundle` took 6.2 s on x86 and 7.1 s on ARM, against 7.8 s and 6.1 s at
phase 2; runner timing varies, so the second decode of each anchor is not
separately visible. A cancellation requested 50 ms in returned after 2.0 ms on
x86 and 0.9 ms on ARM, during source parsing.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14162906661 | `94f2e66acc028dc1c93f848fe03b58cdaef4b1207ea7e554aadb1e994c87cdfb` |
| x86-workload | 14162055927 | `295e365298c750fed180fe5df47075ba1acf6d2a1fa408500dce90b908a273bb` |
| arm-workload (including installed ARM wheels) | 14162195499 | `f4953cda04b95d5e2e7fb7df1902f88cab76e4b1d7bf46db9a43211d5dc19a1d` |

GitHub reports an expiry of 2027-02-13 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Slice 7 phase 4: verified on both architectures

[PR #42](https://github.com/andrew3stedall/iresiphi/pull/42), commit
`067b31fd3e62074557c328aa585d98215b11bf25`, passed all eight jobs in
[run 43745802345](https://github.com/andrew3stedall/iresiphi/actions/runs/43745802345)
on 2026-11-15, including the new spilled-target cases in `tests/spill.rs`.

Under the default `limits.target_bytes` (16 MiB), the workload's `enrichment`
workbook spilled: it kept `id` and `supplier` of its 100,000 records and wrote
80,033,390 bytes to a temporary file. `mapping` and `rates` (two records each)
stayed within the budget and were held, and the `components` detail is held by
design. Results match phase 3: serialized results and evidence are unchanged
at 52,133,436 bytes, with the same counts (99,999 resolved, 1 unresolved) and
evidence records (1 / 1 / 2). The bundle wrote 5,162,822 bytes on x86, 494
more than phase 3. The difference is in `manifest.json`, which gains the
`spilled` read record and whose timing digits vary; the workload artifacts do
not keep the bundle files to compare the tables byte for byte.

Process peak RSS fell from 709,414,912 bytes at phase 3 to 436,948,992 bytes
on x86. `run_bundle` took 7.0 s on x86, against 6.2 s at phase 3. A
cancellation requested 50 ms in returned after 1.7 ms on x86. The ARM job
passed the same workload assertions; its artifact could not be downloaded to
this machine, so its peak RSS and timings are not copied here.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14174278005 | `078cb6b9a0ce88bd052010010781b2d4458921adbc31714d9d0eb029f2e920a6` |
| x86-workload | 14174706367 | `11bde5287d1586867a62d14be538e66d778fe7e3aae603322c636337595f638a` |
| arm-workload (including installed ARM wheels) | 14174216872 | `9f4751436e1c7309bf0e44bb3f15d7d2c2f00672651ac45c7613173d266c4f51` |

GitHub reports an expiry of 2027-02-13 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Slice 7 phase 5: verified on both architectures

[PR #43](https://github.com/andrew3stedall/iresiphi/pull/43), commit
`ef8776616346262d659601396a366e983fab9277`, passed all eight jobs in
[run 43937531695](https://github.com/andrew3stedall/iresiphi/actions/runs/43937531695)
on 2026-11-16, including the new spilled-detail cases in `tests/spill.rs`. The
same cases at the test-only commit `dfa90da` failed in run 43935801270, before
the executor spilled details.

Under the default `limits.target_bytes` (16 MiB), the workload's `components`
detail spilled: it kept `id`, `invoice`, `line` and `supplier` of its 150,000
records and wrote 78,990,174 bytes to a temporary file. The `enrichment`
workbook spilled as in phase 4. Results match phase 4: serialized results and
evidence are unchanged at 52,133,436 bytes, with the same counts (99,999
resolved, 1 unresolved), evidence records (1 / 1 / 2) and one
`LOOKUP_UNMATCHED` warning. The bundle wrote 5,163,454 bytes on x86, 632 more
than phase 4. The difference is in `manifest.json`, which gains the components'
`spilled` read record and whose timing digits vary.

Process peak RSS fell from 436,948,992 bytes at phase 4 to 414,269,440 bytes
on x86. The saving is small because a component's key record keeps four of its
five fields: its identity and the three fields its parent lines match on. Only
`amount` and the original values leave memory. `run_bundle` took 8.7 s on x86,
against 7.0 s at phase 4; each batch now reads its components back from the
temporary file. A cancellation requested 50 ms in returned after 2.7 ms on x86.
On ARM the components spilled the same 78,990,174 bytes, the results matched,
the bundle wrote 5,163,460 bytes, the peak was 406,855,680 bytes and
`run_bundle` took 11.1 s; a cancellation returned after 1.1 ms. The phase 4
ARM artifact was not downloaded, so there is no ARM peak to compare against.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14263052194 | `afa62df7b758d4618d25ba79e6b2cf7b33c31154a4bd3c947218162145f532a3` |
| x86-workload | 14260966775 | `01a73139ea774a2d562a76b49d886072d336462b485d51f58fedf204d821724e` |
| arm-workload (including installed ARM wheels) | 14262067378 | `bf4b4d1cfecc678d2bc011ca7557d3c27b8b858441ffae0ca1846e5e24e072d6` |

GitHub reports an expiry of 2027-02-14 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.

## Customer/month workload baseline: verified on both architectures

[PR #45](https://github.com/andrew3stedall/iresiphi/pull/45), commit
`12dc5a62ebcd666847e00d9eb925ade03262e970`, passed all eight jobs in
[run 44176529565](https://github.com/andrew3stedall/iresiphi/actions/runs/44176529565)
on 2026-11-17. It adds the customer/month workload that slice 7 phases 6 and 7
are measured against, and changes no executor code, so every population input
is still held.

The workload (fixture `e723f04e…2cd2e0`) reads 11,468,605 bytes: 8,000
customers, 48 regional targets and 350,000 transactions. Its population has
96,000 members, of which 64,000 have no transactions and take the declared
complete sum 0. The run succeeded with no diagnostics. Every member resolved,
all 350,000 transactions matched a member, sales totalled 71,748,250.00, and the
allocated target conserved 4,874,640.00 across 48 partitions. Evidence at the
default level holds the 48 allocation partitions only. Serialized results are
45,840,368 bytes, and the bundle wrote 5,264,101 bytes on x86 and 5,264,098 on
ARM; the difference is timing digits in `manifest.json`.

Process peak RSS was 1,184,505,856 bytes on x86 and 1,176,399,872 bytes on ARM,
against 414,814,208 and 406,921,216 bytes for the 100,000-line workload in the
same jobs. The customer/month input is a sixth the size of the larger
workload, but nothing in it streams or spills: members, customers,
transactions and targets are all held. `run_bundle` took 4.7 s on x86 and
4.6 s on ARM. Execution took 2.8 s of the x86 run and parsing 0.8 s.

| Artifact | ID | SHA-256 digest |
| --- | --- | --- |
| x86-distributions | 14371900594 | `b6c313eba52774221530aa9fa54e9985d2ef2241bebe457bc081a0c77615b373` |
| x86-workload (including customer/month) | 14370362562 | `841380a98eb87e38957d650107b96eab6f5124bf7548e9f37658e6a5a62c8855` |
| arm-workload (including customer/month and installed ARM wheels) | 14369483991 | `9a563460caa40546a67277a7d174a1499dd7ff76d9f408061df178f2cddd6fd2` |

GitHub reports an expiry of 2027-02-15 for these artifacts. Evidence for this
commit does not certify later heads; their own PR checks remain authoritative.
