# Iresiphi 0.1.0 developer preview

This preview runs one workflow: an auditable invoice review. It reads two suppliers' invoice JSON, an XLSX enrichment workbook, and CSV mapping, rate and component tables. Then it compares billed charges with contracted rates and keeps every invoice line, including lines it cannot resolve. It ships as a Rust crate and a Python package that share one Rust core, both at version 0.1.0. The recipe format is versioned separately, at `0.1`.

Nothing has been published to crates.io or PyPI. The supported wheel target, Linux x86_64 manylinux_2_28, has not been built or audited yet; see [Packaging](#packaging).

## Walkthrough: save, reopen, rebind, validate, explain and run

`examples/run.rs` and `examples/run.py` run the same steps through the public API:

1. Reopen a saved recipe, save it again, reopen the copy and check that its semantic hash has not changed.
2. Validate it without bindings (static checks), then with bindings and a policy (access and source schemas, no data checks).
3. Explain it with the bindings.
4. Run it into a new directory bundle.
5. Rebind one source to a different file and run the unchanged recipe again into a second bundle.

Generate the small fixture, write an updated rate table beside it, and run the Rust example:

```sh
python3 scripts/generate_fixture.py /tmp/iresiphi-demo
printf 'id,supplier,item,rate,start,end\nr1,A,shared,52,2026-01-01,2030-01-01\nr2,B,shared,45,2026-01-01,2030-01-01\n' \
  > /tmp/iresiphi-demo/rates-2027.csv
cargo run --example run -- /tmp/iresiphi-demo/recipe.iresiphi /tmp/iresiphi-demo/bindings.json \
  /tmp/iresiphi-demo/policy.json /tmp/iresiphi-demo/walkthrough-rs rates=/tmp/iresiphi-demo/rates-2027.csv
```

The Python example takes the same arguments once the wheel is installed:

```sh
python examples/run.py /tmp/iresiphi-demo/recipe.iresiphi /tmp/iresiphi-demo/bindings.json \
  /tmp/iresiphi-demo/policy.json /tmp/iresiphi-demo/walkthrough-py rates=/tmp/iresiphi-demo/rates-2027.csv
```

The Rust example printed this on the development machine (run IDs replaced with `<run id>`):

```text
saved and reopened /tmp/iresiphi-demo/walkthrough-rs/saved.iresiphi (c1ec549ae593222c62f3eee9ac31752abb49d796971e27f55c7480cdf198be1c)
validate (static): valid=true checks=9 failed=0
validate (bound): valid=true checks=11 failed=0
explain: 32 nodes, feasible=true
run <run id>: succeeded_with_unresolved into /tmp/iresiphi-demo/walkthrough-rs/bundle-1; 2 resolved and 1 unresolved lines; reconciliation "partial"
  AUD billed: full "180.00", comparable "160.00", unresolved "20.00"
  AUD enriched: full "22.00", comparable "15.00", unresolved "7.00"
  AUD expected: full null, comparable "145.00", unresolved null
  AUD variance: full null, comparable "15.00", unresolved null
  warning LOOKUP_UNMATCHED relationship:a_mapping: 1 records
run <run id>: succeeded_with_unresolved into /tmp/iresiphi-demo/walkthrough-rs/bundle-2; 2 resolved and 1 unresolved lines; reconciliation "partial"
  AUD billed: full "180.00", comparable "160.00", unresolved "20.00"
  AUD enriched: full "22.00", comparable "15.00", unresolved "7.00"
  AUD expected: full null, comparable "149.00", unresolved null
  AUD variance: full null, comparable "11.00", unresolved null
  warning LOOKUP_UNMATCHED relationship:a_mapping: 1 records
rebound run: recipe hash unchanged=true
```

The first run reproduces the agreed acceptance values: billed 180 in full, 160 comparable against 145 expected, variance 15, and 20 unresolved on the unmapped line. The second run reads `rates-2027.csv`, where supplier A's rate is 52, so A's expected charge becomes 104 and the comparable totals move to 149 and 11. The invoice inputs and the recipe hash do not change. The Python example prints the same lines, plus the first two rows of the first output as exact `Decimal` values.

The saved recipe holds no paths, data or secrets. Each run takes its bindings (source ID to file, adapter and settings) and its policy (allowed roots and limits) as separate inputs. Rebinding `rates` changes the rate source and nothing else.

## Support and deferral matrix

| Area | In this preview | Deferred |
| --- | --- | --- |
| Interfaces | Rust crate `iresiphi`; Python package `iresiphi` on ordinary (GIL) CPython 3.12 and 3.13 | TypeScript, Node, browsers; abi3, free-threaded CPython, PyPy |
| Platforms | Target: Linux x86_64, glibc 2.28 or later, as manylinux_2_28 wheels. Built and tested so far only on Linux aarch64 (glibc 2.43), which is not a supported target | macOS, Windows and ARM wheels |
| Inputs | Local UTF-8 JSON with one declared array and header fields; local CSV with an explicit dialect; local XLSX with a named sheet, declared header row and typed columns | Dataframes, SQL, Snowflake, Spark, NDJSON, XML, arbitrary text, Parquet, remote stores, network reads |
| Execution | One local executor; DataFusion 55.2.0 produces lookup candidate pairs | Pushdown, remote staging, hybrid placement, spill to disk |
| Scalars | Boolean, Int64, UTF-8 string, Decimal128 (precision 1 to 38), calendar date | Floating point, timestamps and time zones |
| Expressions | Literals, field and path references, casts, add, subtract, multiply, comparisons, AND/OR/NOT, is-null, coalesce, case, quantize (`half_even`, `toward_zero`), rate validity | Division, regex, user-defined functions, Python callbacks |
| Relationships | Typed equality on composite keys; one-to-one, many-to-one, one-to-many; invoice-date validity (`valid_from <= date < valid_to`, null `valid_to` open) | Many-to-many, fuzzy or normalized matching, automatic as-of selection, general temporal joins |
| Grain | `count_records`, `count_non_null`, `sum`, `min`, `max` at declared group keys; broadcast along a validated to-one path; context `min`/`max` | Average, distinct count, windows, semi-additive measures, allocation |
| Business rules | One explicit currency per amount; mixed currencies are rejected | Currency conversion, tax, tiered pricing, credits, discounts |
| Outputs | Typed memory results; local directory bundles (CSV tables, `schema.json`, `manifest.json`) | Other sinks |
| XLSX formulas | Stored cached values only | Formula evaluation |

Unsupported bindings and operations fail with a diagnostic (`UNSUPPORTED_OPERATION`, `UNSUPPORTED_CAPABILITY`, `INVALID_BINDING`). The library never switches to another executor and never collects data some other way.

## Resource profile

The policy sets allowed read and output roots and these limits. The values below are the defaults. They are admission limits, not measured capacity, and any of them can be raised in the policy. The run manifest records the limits each run used.

| Limit (`policy.limits`) | Default | What it bounds |
| --- | --- | --- |
| `engine_bytes` | 256 MiB | DataFusion memory pool for the lookup joins; no spill |
| `file_bytes` | 64 MiB | One input file, counted while reading |
| `input_bytes` | 256 MiB | All encoded input of a run |
| `xlsx_bytes` | 256 MiB | Actual decompressed bytes of all XLSX parts |
| `worksheet_cells` | 1,000,000 | Decoded cells per worksheet |
| `field_bytes` | 1 MiB | One field or token |
| `json_depth` | 64 | JSON nesting |
| `anchor_rows` | 100,000 | Invoice lines per anchor source |
| `component_rows` | 1,000,000 | Rows of a one-to-many detail source |
| `lookup_rows` | 100,000 | Rows of each other source |
| `candidate_pairs` | 1,000,000 | Qualified lookup candidates, and candidate plus detail evidence records |
| `output_bytes` | 256 MiB | Serialized results and evidence, and bundle bytes |
| `retained_bytes` | 256 MiB | Retained source materialization and memory results |
| `batch_rows` | 8,192 | Rows per batch |
| `batch_bytes` | 16 MiB | Decoded bytes per batch; one larger record is rejected |

A run that exceeds a limit fails with `RESOURCE_LIMIT` and publishes nothing. Each execution context runs one run at a time, and another attempt gets `RUN_BUSY`. Execution uses two partitions.

These limits do not cap process memory. DataFusion does not track every allocation, the adapters hold each source's rows in memory, and Python owns any values it copies out. The manifest reports the tracked engine reservation peak (`engine_peak_reservation`) separately from the process's peak resident set (`peak_rss`, read from `/proc/self/status`). That peak covers the whole process lifetime, not one run. If you need a hard memory ceiling, run Iresiphi in an OS-isolated process with its own limits. [The 100,000-line measurement](measurements/workload-100k.md) shows what one complete run used on one machine.

## XLSX: quantization, cached values and limits

An XLSX binding must name its sheet and its 1-based header row:

```json
{"path": "enrichment.xlsx", "adapter": "xlsx",
 "settings": {"sheet": "Review", "header_row": 1,
              "decimal_quantization": {"amount": "half_even"},
              "excel_dates": ["valid_from"]}}
```

- Text cells that hold decimals parse exactly against the declared precision and scale, with no rounding. Use text cells for money where you can.
- A numeric cell for a decimal field needs an entry in `decimal_quantization` (`half_even` or `toward_zero`). Without one the run fails with `XLSX_QUANTIZATION_REQUIRED`. Iresiphi converts the stored binary number to its shortest round-trip decimal text, then quantizes that text to the declared scale with checked arithmetic. This cannot recover the decimal someone originally typed. Each cell's evidence records its reference, cell type, observed representation, formula, quantization, conversion and date system.
- An integer field accepts a numeric cell only when the value is whole and smaller than 2^53 in magnitude.
- A date field reads ISO `YYYY-MM-DD` text. It reads date and numeric cells only when `excel_dates` lists the field. The adapter handles both the 1900 and 1904 date systems, and rejects the fictitious 1900-02-29 (serial 60), serial 0 in the 1900 system, and fractional serials.
- Iresiphi never evaluates formulas. A formula cell must have a stored cached value, or the run fails with `FORMULA_CACHE_MISSING`. If the workbook asks for recalculation on load, the run fails with `FORMULA_CACHE_UNCERTAIN`, because the cached values may be stale. A cache that is present is still not certified as fresh.
- An error cell fails the run with `XLSX_CELL_ERROR`.
- Before reading the sheet, a preflight decompresses every part of the package, counting actual bytes against `xlsx_bytes` and worksheet cells against `worksheet_cells`. It covers every part and sheet, not only the declared one, so a large unrelated sheet can fail a run.
- The adapter checks for cancellation between preflight chunks and between rows. Calamine's workbook parsing cannot be interrupted, so a cancel request waits for that call to return.

## CSV input dialect and bundle escaping

CSV inputs are UTF-8 with one header record and RFC 4180 quoting. A binding may set `delimiter` and `quote` (one ASCII byte each, which must differ and must not be a line ending or a backslash). It may also set `null_tokens`, a list of non-empty strings, which defaults to `["\N"]`. An empty field is an empty string, never null. A field that starts with two backslashes loses one, so `\\N` reads as the text `\N`.

Bundle CSV tables use UTF-8, commas, RFC 4180 quoting, LF line endings and one header row. Escaping works like this:

| Value | Field text |
| --- | --- |
| null | `\N` |
| empty string | (empty) |
| text `\N` | `\\N` |
| text `\x` | `\\x` |
| any other text | unchanged |

Every non-null value whose text starts with a backslash gets one more backslash before CSV quoting. To read a field back: exact `\N` is null; otherwise remove one backslash only from a doubled-backslash prefix, then parse the text by the column type in `schema.json`. `schema.json` documents this rule. Spreadsheet programs and generic CSV readers do not apply it, so open bundle tables with `iresiphi::sink::read_bundle` (Rust) or `iresiphi.read_bundle` (Python), or apply the rule yourself.

A bundle holds one CSV file per output, three evidence tables (`_evidence`, `_candidate_evidence`, `_detail_evidence`, each one JSON record per row) and `_reconciliation`, a table of full-population, comparable and unresolved totals per output, currency and numeric column. `schema.json` gives each table's typed columns, and `manifest.json` carries the complete run record.

## Snapshot limitations

Each source is read once per run. The manifest records the SHA-256 of the bytes that run actually read. If a file's size, modification time or change time differs between the start and end of its read, the run fails with `SOURCE_CHANGED`. There is no snapshot across files: two sources read moments apart may reflect different states of the world, and nothing detects a change to one file while another is being read. To reproduce a run, keep the exact input files; the fingerprints in its manifest identify them. Rate validity uses the invoice date only. It is not bitemporal and does not reconstruct historical pricing.

## Unresolved comparisons

Every invoice line stays in the primary output. Each to-one lookup (mapping, rate, enrichment) is decided only after all qualified candidates have been counted:

- One candidate: `matched`, and its values are used.
- No candidate: `unmatched`. This includes a line whose key has a null component, and a line with key matches none of which is valid on the invoice date.
- More than one candidate: `ambiguous`. No value is chosen; there is no first-match or latest-wins rule.

A line's `_status` is `ambiguous` if any lookup it depends on is ambiguous, `unmatched` if any is unmatched, and `matched` otherwise. `_reason` names each lookup that failed and why. Values that depend on an unresolved lookup, such as expected charge and variance, are null. They are never zero or estimated. Such a run ends `succeeded_with_unresolved`, and the diagnostics count the affected lines (`LOOKUP_UNMATCHED`, `LOOKUP_AMBIGUOUS`).

Totals are reported three ways for each currency: the full population, the comparable (resolved) subset, and the unresolved remainder. For the agreed two-supplier fixture that is billed 180 in total, 160 comparable against 145 expected (variance 15), and 20 unresolved. A total over a column with any null value is itself null, so a sum that skipped unresolved lines is never presented as complete. The manifest's `reconciliation.status` is `complete`, `partial` or `empty`, and a partial run says how many lines it covers.

Mixed currencies are rejected. A grouped output must group by the anchor's currency. A per-line aggregate fails if its detail rows and the line carry different currencies. A line expression that adds, subtracts or compares amounts with different currencies fails with `MIXED_CURRENCY`. A grouped output whose anchor declares no currency cannot sum amounts from a source that declares one.

## Cleanup behaviour

A bundle run stages its files in a sibling directory of the destination, named `.iresiphi-` plus 32 random hex digits, with owner-only permissions (0700 directory, 0600 files). The run holds an exclusive lock on that directory. Publication is one `renameat2(RENAME_NOREPLACE)` to a destination that must not exist yet. Iresiphi never overwrites or merges, so retrying at the same destination returns `DESTINATION_EXISTS`. If the filesystem lacks that rename primitive the run fails with `PUBLICATION_UNSUPPORTED`, and a cross-device rename fails with `CROSS_DEVICE`; no test here exercises either path. The rename makes the bundle visible all at once. It is not an fsync-backed guarantee against power loss.

- After success, failure or cancellation, the run removes its staging directory. If removal fails, the run report lists each leftover path and its error under `cleanup`.
- Cancellation before the rename publishes nothing. After the rename the run is committed, and a later cancel request has no effect.
- If the process dies, staging can remain. `iresiphi::sink::cleanup_staging(directory, &policy)` removes only directories that are named like run staging, owned by the current user with owner-only permissions, not locked by an active run, and holding only regular files. Everything else is reported as skipped, and nothing is deleted recursively. This call is not yet exposed in Python.

## Packaging

- `cargo build --locked` builds the crate from the committed `Cargo.lock`. The toolchain is Rust 1.94.0, pinned in `rust-toolchain.toml`.
- The Python sdist (`maturin sdist`) contains the Rust sources, `Cargo.toml` and `Cargo.lock`, so building from it needs Rust 1.94.0, a C compiler, and maturin 1.16.0 as the PEP 517 backend.
- Wheels are built separately for CPython 3.12 and 3.13 (`cp312`, `cp313`) with the `python` feature, which links PyO3 0.30.1.

The release target is Linux x86_64 manylinux_2_28 wheels built in a pinned build image and audited with auditwheel. That has not happened: this preview was built on an aarch64 machine with no x86_64 toolchain or container runtime. [docs/measurements/packaging.md](measurements/packaging.md) records what was built and audited locally, and the external blockers.

Third-party crate licenses and NOTICE texts are listed in [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md), which `scripts/third_party_notices.py` generates from `Cargo.lock`.

## Known gaps

[docs/measurements/unmet.md](measurements/unmet.md) lists each contract clause this preview does not yet meet, and why.
