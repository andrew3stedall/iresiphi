# Decision history 15

Map: https://github.com/andrew3stedall/iresiphi/issues/2

Investigate primary-source, version-specific DataFusion versus Polars suitability for the resolved local invoice preview; exact decimals, wrapper-owned joins/cardinality and validity, memory/cancellation, PyO3/maturin CPython 3.12/3.13 Linux x86_64, JSON/CSV and calamine XLSX ingestion. Deliver one committed docs/research note with precise baselines, limits and integration acceptance gates. Evidence for engine and binding decisions #11 and #12; does not resolve those decisions.

https://github.com/andrew3stedall/iresiphi/issues/15#issuecomment-6797650660

## Resolution — engine and binding evidence complete

Autonomous research under the developer's authorization; no implementation or benchmark performed.

Context pointer: commit `4cacaf1` on local research branch `t3/research-engine-bindings`, path `docs/research/preview-engine-bindings.md`. The commit is local and available for integration/cherry-pick; this is not a claim it is published on the default branch.

Recommend DataFusion 55.2.0, PyO3 0.30.1, maturin 1.16.0, calamine 0.38.0, serde_json 1.0.154 and csv 1.4.0, with Rust 1.94.0 and separately tested CPython 3.12/3.13 manylinux_2_28 x86_64 wheels. Polars Rust 0.55.2 remains viable; prefer DataFusion's documented resource reservations and stream cancellation, not speculative speed.

The note distinguishes documented APIs from untested integration. Iresiphi must own exact checked decimal semantics, candidate counting and line-preserving joins, date eligibility, source references and resource admission. DataFusion memory accounting is not a hard RSS ceiling. Calamine Float cells cannot establish exact decimal source fidelity: require text decimals or validated integer cells for exact arithmetic. Cached-formula availability, date epochs, locked wheel builds, numeric edge cases and cancellation are explicit implementation acceptance gates.

Evidence is sufficient for the downstream engine and binding choices with those gates. Native parent and blocking relationships are installed. Closing this research ticket only; engine/binding decisions remain open and the map body is unchanged.