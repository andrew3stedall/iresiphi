# Slice 7: deferred pipelines and bounded execution

Parent: https://github.com/andrew3stedall/iresiphi/issues/1.

The developer decided on 2026-11-14 that Iresiphi should work like other data platforms: a caller builds a pipeline that describes the sources, how they connect and what to produce, and nothing runs until the caller asks for it. A run should not have to hold every record in memory. This is a developer decision.

## Where the preview stood

Most of the first half was already true. A recipe is a declaration: building, validating, saving and explaining it opens no source. What was missing was a named handle that binds a recipe to its sources and policy without running anything, and a run that streams.

A run read every record of every source into memory before doing anything else. Measured on the 100,000-line workload (13.5 MB of input, 350,004 records) after slice 6, the process held 1.11 GB right after reading. Joining added 0.14 GB, and building outputs added almost nothing. Each record cost about 3 KB: every record kept two maps keyed by field name, typed values and original values, and its own copies of the source name, binding path and 64-character fingerprint.

## Phases

Each phase ships on its own, keeps every result, diagnostic, evidence record and serialized byte count identical, and is measured on the 100,000-line workload.

1. **Deferred pipeline and compact records** (this change).
   - `Pipeline` in Rust, `iresiphi.plan(recipe, bindings, policy)` in Python and `plan(...)` in Node own copies of their declarations. Building and explaining one opens no source. `collect()` runs and keeps the results in memory; `run(destination)` runs and delivers a bundle. Each call reads the sources as they are then.
   - Records of one source share one sorted list of field names, so a record holds only its values. Source name, binding and fingerprint are shared by every record of a read. Records serialize exactly as before.
   - Lookup candidates stream out of the join engine one batch at a time instead of being collected first. Byte budgets count serialized bytes without building a serialized copy.
2. **Streamed delivery.** A bundle run writes output rows and evidence to its staging directory as it produces them, instead of holding the tables until the end. Totals and counts accumulate as rows pass. `collect()` still keeps results, because that is what it is for.
3. **Streamed anchors.** A source read only as an output's anchor is read in batches and passes through lookups to delivery without being held. Checks that need every anchor record (identity, one-to-one exclusivity, strict cardinality) keep compact per-key state or read the source a second time, and the run fails if its fingerprint changed between reads. Only lookup targets, detail sources and populations are held, indexed by key.
4. **Targets larger than memory.** Lookup targets beyond a declared budget are spilled to disk or looked up at their source. This needs its own decision on the spill format and on what a target held at a source may cost.

## Contract for phase 1

1. `Pipeline::new(recipe, bindings, policy)` takes ownership of its declarations. Changing or closing the caller's recipe, bindings or policy afterwards does not change the pipeline.
2. `explain()` opens no source.
3. `collect()` and `run(destination)` produce the same report as `ExecutionContext::run` and `run_bundle` with the same declarations. A source missing when the pipeline was built and present when it runs is read.
4. A record's `fields` and `raw` behave as maps keyed by field name and serialize as JSON objects with keys in ascending order, as before. Record references serialize as before.
5. Results, evidence, diagnostics, manifests (other than timings and memory measurements), bundle tables and `retained_result_bytes` are identical to the previous release for the same inputs.

## Out of scope for phase 1

Phases 2 to 4. A fluent builder that composes a pipeline step by step (the recipe builders already compose declarations). Results returned as a lazy stream to the caller.
