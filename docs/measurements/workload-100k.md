# 100,000-line workload measurement

Recorded 2026-11-12 for ticket #24. These are observations of a few runs on one shared development machine. They are not benchmarks, not comparisons with any other tool, and not promises about time or memory on any other machine.

## Summary

- Under the default resource profile the complete workflow does not succeed. All data and population checks pass, then the run fails with `RESOURCE_LIMIT` (`budget.evidence`): the results and evidence serialize to 757,303,499 bytes, about 2.8 times the 256 MiB `output_bytes` and `retained_bytes` defaults. Nothing is published.
- With `output_bytes` and `retained_bytes` raised explicitly to 1 GiB, the run succeeds with unresolved lines, publishes an 815,129,474-byte bundle, and reproduces the generator's expected totals exactly. `run_bundle` took 110.6 s, and the process peak resident set was 5.14 GB.
- Evidence building and bundle delivery take most of the time. Evidence volume, not input size (13.5 MB), drives both the byte budget and memory.

## Machine and software

| Item | Value |
| --- | --- |
| CPU | Qualcomm Oryon, aarch64; 8 CPUs reported, 4 available to the process (`nproc`) |
| Memory | 10.9 GiB RAM, 12 GiB swap. Other agent sessions shared the machine, and about 4.4 GiB of swap was already in use before the runs |
| OS | Ubuntu 26.04.1 LTS under PRoot (kernel string `6.17.0-PRoot-Distro`), glibc 2.43 |
| Compiler | rustc 1.94.0 (4a4ef493e 2026-03-02), LLVM 21.1.8; GCC 15.2.0 |
| Build | `cargo build --locked --release --examples` (opt-level 3, no debug info, no LTO, no target-specific CPU flags) |
| Harness | `examples/workload.rs`, which calls `ExecutionContext::run_bundle` through the public Rust API |
| Library | iresiphi 0.1.0, DataFusion 55.2.0, recipe format 0.1 |

The machine was not idle: other sessions were compiling and running tests. Treat elapsed times as rough.

## Fixture

Generated with `python3 scripts/generate_fixture.py DIR --lines 100000`. Generation is deterministic arithmetic with no random numbers; regenerating it gives the same hash.

| Item | Value |
| --- | --- |
| Seed | `iresiphi-preview-v1` |
| Fixture SHA-256 (data files and recipe) | `b015de79f08da46b05288e2b878304afb6ba644d6459ce737a6993aefdb2bbb6` |
| Invoice lines | 100,000: 50,000 in supplier A's JSON document and 50,000 in supplier B's |
| Invoices | 2,000 (each supplier's document has 1,000 invoice identifiers, 50 lines each) |
| Observed components | 150,000; line *i* has *i* mod 4 components (0 to 3) |
| Unresolved lines | 1 (an unmapped item, billed 20) |

| Input | Adapter | Bytes | Rows | SHA-256 |
| --- | --- | ---: | ---: | --- |
| `a.json` | JSON | 4,039,155 | 50,000 | `9d7dd5eb…2e976` |
| `b.json` | JSON | 4,489,165 | 50,000 | `e21238fd…57759` |
| `components.csv` | CSV | 3,550,669 | 150,000 | `3b071800…3beeb` |
| `enrichment.xlsx` | XLSX | 1,385,485 | 100,000 | `a5a35269…934fb` |
| `mapping.csv` | CSV | 65 | 2 | `a3df67dc…6ef55` |
| `rates.csv` | CSV | 106 | 2 | `b0cce2dd…ece3b` |
| Total | | 13,464,645 | 350,004 | |

`fixture-manifest.json`, written beside the fixture, holds the full hashes.

## Policy

Run A used the generated `policy.json`: the fixture directory as the only read and output root, and every limit at its default (see [the resource profile](../preview.md#resource-profile)). Run B used the same policy with two limits raised explicitly, and its manifest records them:

```json
{"limits": {"output_bytes": 1073741824, "retained_bytes": 1073741824}}
```

Admission was the default profile in every run: one active run per context, two partitions, 8,192-row batches, no spill.

## Results

| | Run A: default limits | Run B: raised output and retained bytes |
| --- | --- | --- |
| Status | `failed`, `RESOURCE_LIMIT` at `budget.evidence` (phase `delivery`) | `succeeded_with_unresolved` |
| `run_bundle` wall time | 67.4 s | 110.6 s |
| Process peak RSS (`VmHWM` after return) | 5.78 GB | 5.14 GB |
| RSS before the run | 10.2 MB | 10.2 MB |
| Tracked DataFusion reservation peak | 10,102,780 bytes (limit 256 MiB) | 10,102,780 bytes (limit 256 MiB) |
| Bytes read | 13,464,645 | 13,464,645 |
| Serialized results and evidence | 757,303,499 bytes (over the 268,435,456 budget) | 757,303,499 bytes |
| Bytes written | none (no bundle) | 815,129,474 |
| Staging left behind | none | none |

Peak RSS is the whole process's high-water mark, read from `/proc/self/status`. It includes the harness and everything allocated before and after the run, and it is not a per-run bound. Under swap pressure it also understates the memory the process touched.

### Phase timings, run B

The manifest's `phases_us` records each phase in microseconds. Delivery is added to the run report after commit.

| Phase | Time |
| --- | ---: |
| Static validation | 0.04 ms |
| Source access (open, read, fingerprint) | 0.06 s |
| Parse (decode, type and identity checks per source) | 3.83 s |
| Conversion to typed Arrow batches and back | 0.51 s |
| Data validation (identity, rate validity intervals) | 0.31 s |
| Planning (dependency order, engine session) | 0.04 s |
| Execution (lookups and output rows) | 11.03 s |
| Evidence (records and the budget measurement) | 30.46 s |
| Run total, before delivery | 49.69 s |
| Delivery (bundle tables, staging, write, rename) | 37.32 s |
| Not attributed | 23.58 s |

The unattributed 23.6 s falls between the end of the run and the start of delivery, plus the time after delivery before `run_bundle` returns. That covers the manifest's second serialization of the results to count `retained_result_bytes`, and freeing several gigabytes of evidence values. The phases do not time either step separately.

Per source, parsing took 0.85 s for the XLSX workbook (100,000 rows), 0.46 s for the components CSV (150,000 rows) and 0.30 to 0.33 s for each JSON document (50,000 lines).

### Counts, run B

| Item | Count |
| --- | ---: |
| Source records | 350,004 |
| Output rows (`a_review`, `b_review`) | 50,000 + 50,000 |
| Resolved / unresolved lines | 99,999 / 1 |
| Primary evidence records | 100,000 |
| Candidate evidence records | 299,998 (mapping 99,999, rate 99,999, enrichment 100,000) |
| Detail evidence records | 150,000 |
| Diagnostics | one warning: `LOOKUP_UNMATCHED`, 1 line |

Combined AUD totals, which match `fixture-manifest.json`:

| Column | Full population | Comparable | Unresolved |
| --- | ---: | ---: | ---: |
| billed | 7,999,910.00 | 7,999,890.00 | 20.00 |
| expected | null (1 null line) | 7,249,900.00 | null |
| variance | null (1 null line) | 749,990.00 | null |

### Bundle, run B

| Table | File | Bytes |
| --- | --- | ---: |
| `_evidence` | `table-2.csv` | 487,342,131 |
| `_candidate_evidence` | `table-0.csv` | 234,576,576 |
| `_detail_evidence` | `table-1.csv` | 88,056,219 |
| `a_review` | `table-4.csv` | 2,639,256 |
| `b_review` | `table-5.csv` | 2,489,179 |
| `_reconciliation` | `table-3.csv` | 859 |
| `manifest.json`, `schema.json` | | 25,254 |
| Total | | 815,129,474 |

Evidence makes up 99.4 % of the bundle. Each primary evidence record repeats the full candidate observations of its lookups, which the candidate evidence table also holds.

## Cancellation

With `IRESIPHI_CANCEL_AFTER_MS`, a second thread sets the run's cancel flag after a delay. Both runs used the default policy.

| Request after | Where the run was | Status | Request to return |
| --- | --- | --- | ---: |
| 2 s | parsing sources | `cancelled` (phase `source`) | 0.31 s |
| 20 s | building output rows and evidence | `cancelled` (phase `execution`) | 25.2 s |

Neither run published anything or left staging behind, and both marked unfinished checks `not-run`. The 25.2 s delay after a cancel during output building was not diagnosed. The output loop checks the flag on every line. The most likely cost is freeing the partly built evidence (peak RSS 4.53 GB) while the machine was swapping, but that is not confirmed. No cancellation latency is promised, and this one is long.

## Reproducing

```sh
python3 scripts/generate_fixture.py /tmp/w100k --lines 100000
cargo build --locked --release --examples
target/release/examples/workload /tmp/w100k/recipe.iresiphi /tmp/w100k/bindings.json \
  /tmp/w100k/policy.json /tmp/w100k/bundle-a
# Run B: the same with a policy whose limits raise output_bytes and retained_bytes to 1073741824.
IRESIPHI_CANCEL_AFTER_MS=2000 target/release/examples/workload ... /tmp/w100k/bundle-cancel
```

The 100,000-line fixture and the bundles are not committed. The generator, seed and hash above identify the fixture.
