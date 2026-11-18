# Slice 7: deferred pipelines and bounded execution

Parent: https://github.com/andrew3stedall/iresiphi/issues/1.

The developer decided on 2026-11-14 that Iresiphi should work like other data platforms: a caller builds a pipeline that describes the sources, how they connect and what to produce, and nothing runs until the caller asks for it. A run should not have to hold every record in memory. This is a developer decision.

## Where the preview stood

Most of the first half was already true. A recipe is a declaration: building, validating, saving and explaining it opens no source. What was missing was a named handle that binds a recipe to its sources and policy without running anything, and a run that streams.

A run read every record of every source into memory before doing anything else. Measured on the 100,000-line workload (13.5 MB of input, 350,004 records) after slice 6, the process held 1.11 GB right after reading. Joining added 0.14 GB, and building outputs added almost nothing. Each record cost about 3 KB: every record kept two maps keyed by field name, typed values and original values, and its own copies of the source name, binding path and 64-character fingerprint.

## Phases

Each phase ships on its own, keeps every result, diagnostic, evidence record and serialized byte count identical, and is measured on the 100,000-line workload. Phases 6 and 7 are measured on the customer/month workload instead, because the 100,000-line workload declares no explicit population or allocation.

1. **Deferred pipeline and compact records** (done).
   - `Pipeline` in Rust, `iresiphi.plan(recipe, bindings, policy)` in Python and `plan(...)` in Node own copies of their declarations. Building and explaining one opens no source. `collect()` runs and keeps the results in memory; `run(destination)` runs and delivers a bundle. Each call reads the sources as they are then.
   - Records of one source share one sorted list of field names, so a record holds only its values. Source name, binding and fingerprint are shared by every record of a read. Records serialize exactly as before.
   - Lookup candidates stream out of the join engine one batch at a time instead of being collected first. Byte budgets count serialized bytes without building a serialized copy.
2. **Streamed delivery** (done). A bundle run writes output rows and evidence to its staging directory as it produces them, instead of holding the tables until the end. Totals and counts accumulate as rows pass. `collect()` still keeps results, because that is what it is for.
3. **Streamed anchors** (done). A source read only as an output's anchor is not held. The run keeps each record's key fields and reference, which is all that lookups and the checks over every anchor record (identity, one-to-one exclusivity, strict cardinality, detail parents) read, and each output decodes the anchor's records again in batches. Only lookup targets, detail sources and populations are held whole.
4. **Targets larger than memory** (done). A lookup target over `limits.target_bytes` keeps only its key records. Each batch of anchor lines reads the target records it reaches, from a temporary file or, for a SQLite target where placement allows, from the source's snapshot.
5. **Detail sources larger than memory** (done). The phase 4 budget and stores apply to one-to-many detail sources too, and to a relationship target that outputs also anchor at. Each output reads its own anchor records, when they spilled, in the same batches.
6. **Bounded population activity** (done). A customer/month workload lands first and records the baseline. Allocation and activity coverage then run in batches of population members, so sources that population and allocating outputs reach can spill. A period source keeps only its distinct periods.
7. **Derived members and bounded recipients** (specified). Population members are derived from their recipient and period instead of stored, and a recipient source over the budget spills.

## Contract for phase 1

1. `Pipeline::new(recipe, bindings, policy)` takes ownership of its declarations. Changing or closing the caller's recipe, bindings or policy afterwards does not change the pipeline.
2. `explain()` opens no source.
3. `collect()` and `run(destination)` produce the same report as `ExecutionContext::run` and `run_bundle` with the same declarations. A source missing when the pipeline was built and present when it runs is read.
4. A record's `fields` and `raw` behave as maps keyed by field name and serialize as JSON objects with keys in ascending order, as before. Record references serialize as before.
5. Results, evidence, diagnostics, manifests (other than timings and memory measurements), bundle tables and `retained_result_bytes` are identical to the previous release for the same inputs.

## Out of scope for phase 1

Phases 2 to 4. A fluent builder that composes a pipeline step by step (the recipe builders already compose declarations). Results returned as a lazy stream to the caller.

## Contract for phase 2

1. A bundle run approves its destination (output policy, no existing entry) before it reads any source. A refused destination fails the run with the same diagnostic as before, without opening a source.
2. Staging is created when output production begins, and the run enters the `write` phase there. Each output table and each evidence table is a CSV file in staging, written row by row as the run produces it. Evidence tables are written as their records are produced.
3. Counts, review totals, evidence counts and the serialized result size accumulate as rows pass. A budget fails the run as soon as it is exceeded; the final check is exact, at the same boundary as before.
4. A bundle run's report carries no tables or evidence. The bundle holds them, with the same file names, CSV bytes, `schema.json` and `manifest.json` as before. `retained_result_bytes` in its manifest is the size the memory result would have.
5. A bundle run keeps no results, so `limits.retained_bytes` bounds only its retained source records and population members. `limits.output_bytes` bounds both the serialized results and the bytes written, as before.
6. A run that fails or is cancelled after staging was created removes it and records `delivery` (not committed) and `cleanup` in its manifest, as a failed publication did.
7. Memory runs (`collect()`, `ExecutionContext::run`) are unchanged.

Phase 2 does not change what a run holds while it executes: every source record is still read into memory first, so the process peak is set by sources (phase 3). What it removes is the copy of every output table and evidence record, and their string forms, held until delivery.

## Contract for phase 3

1. A source is streamed when outputs anchor at it, none of them is a detail output or allocates, no relationship reads it as a target, and no population or allocation takes recipients, periods or a quantity from it. Every other source is held as before. All four adapters stream.
2. The first read of a streamed source decodes every record and runs every check a held read runs, in the same order: values, row count, record and batch bytes, typed batch conversion, identity. Of each record it keeps only its key record: the identity fields, the fields its relationships key on (undotted from keys, validity date, period key) and its source reference.
3. Lookups, candidate counting, exclusivity, strict cardinality, transfers from the source and detail parents run over the key records exactly as over held records, and keep each anchor's candidates and outcome by record position.
4. Each output anchored at a streamed source decodes it again and produces its rows and evidence `limits.batch_rows` records at a time. A record is the one at the same position in the first read.
5. A later read decodes the encoded bytes the first read fingerprinted; a SQLite source queries the snapshot it read, with the same placement. Every read therefore sees the same records, and a source changed on disk after the first read is not seen. The run holds those bytes, not the decoded records. A later read that delivers a different number of records fails the run with `SOURCE_CHANGED`.
6. `limits.retained_bytes` counts a streamed source's key records, not its decoded records. Every other limit applies as before.
7. The manifest's read record of a streamed source adds `streamed`: the fields kept, the number of reads and the batch size. Cancellation noticed during a later read cancels the run like any cancellation during execution.
8. Results, evidence, diagnostics, bundle tables and `retained_result_bytes` are identical to phase 2 for the same inputs. Memory runs stream anchors too.

Phase 3 does not change what a lookup target, detail source or population costs. Those are held whole, and phase 4 decides how a target larger than memory is held.

## Contract for phase 4

The developer decided on 2026-11-16, choosing among the options put to them, that a lookup target over a declared budget keeps its key records like a streamed anchor and reads its other records again when a batch needs them. The records go to a temporary file only when the target is over the budget. Where placement allows work at the source, a SQLite target is queried at its snapshot instead. Detail sources stay held. This is a developer decision.

1. `limits.target_bytes` (default 16 MiB) bounds the serialized bytes of one lookup target's records held in memory. A source can spill when it is read only as the target of to-one relationships. That means no output anchors at it, no one-to-many relationship starts or ends at it, and no population or allocation takes records from it. No output that allocates or anchors at a population may reach it through relationships either. Every other source is held as before.
2. A target's first read runs every check a held read runs. Its records are held until their serialized bytes pass `limits.target_bytes`. From then on, the run keeps only a key record for each record of the target, including those already held. A key record holds the identity fields, the fields relationships into the target key on (`to_keys`, validity interval, period date), the fields a dotted key reads from it, the fields its own relationships key on, and its source reference.
3. A spilled file target, or a SQLite target under placement `local`, writes each record's serialized form (the JSON that evidence uses) to a new file in the system temporary directory. The file is created readable only by the run's user and removed from the directory as soon as it is open, so nothing is left behind however the run ends. Under `prefer_source` or `require_source`, a SQLite target writes nothing. It is queried again by rowid at the snapshot its first read used.
4. Matching, candidate counting, exclusivity, strict cardinality, interval checks, period keys, dotted keys and transfers run over the key records exactly as over held records.
5. Every output runs its anchor lines in batches of `limits.batch_rows`. Before each batch, the run reads the records of spilled targets that the batch's lines reach through any chain of relationships. The batch's columns, aggregates and evidence read those records. A record read again that does not decode, or is not at the position the first read gave it, fails the run with `SOURCE_CHANGED`. Failing to write or read the temporary file fails the run with `RESOURCE_LIMIT`.
6. `limits.retained_bytes` counts a spilled target's key records. The temporary file holds at most the target's admitted records, each within `limits.batch_bytes`.
7. The manifest's read record of a spilled target adds `spilled`. It gives the store (`temporary_file` or `source`), the fields kept, the record count, the budget and, for a file, the bytes written.
8. Results, evidence, diagnostics, bundle tables and `retained_result_bytes` are identical to phase 3 for the same inputs. Memory runs spill targets too.

Phase 4 does not change what a detail source or population costs. They are held whole. Process memory is still a measurement, not a bound.

## Contract for phase 5

The developer decided on 2026-11-17, choosing among the open items put to them, that detail sources should be bounded the way phase 4 bounds lookup targets. After phase 4 the workload's 150,000-record component detail was the largest source still held whole. This is a developer decision.

1. `limits.target_bytes` bounds the serialized bytes of any one relationship target's records held in memory: a to-one lookup target or a one-to-many detail. A source can spill when it is the target of at least one relationship and no population or allocation takes records from it. No output that allocates or anchors at a population may anchor at it or reach it through relationships. One-to-many relationships may start or end at it, and other outputs, detail outputs included, may anchor at it. Every other source is held as before. A source streamed under phase 3 is never a relationship target, so it never spills.
2. The first read, the key record and the stores are those of phase 4 (clauses 2 and 3). The key record therefore also holds the fields a spilled source's own relationships key on, so a spilled detail can be the parent of another detail.
3. Matching, candidate counting, exclusivity, strict cardinality, interval checks, period keys, dotted keys, transfers, detail parents and orphan detail checks run over the key records exactly as over held records.
4. Every output runs its anchor lines in batches of `limits.batch_rows`. Before each batch the run reads that batch's own records when its anchor spilled, and the records of spilled sources the batch's lines reach through any chain of relationships, one-to-many details included. The batch's columns, per-line detail aggregates, selection predicates and candidate and detail evidence read those records. A detail output's parent reference reads only its parent's identity and reference, which the key record holds. Phase 4 clause 5 applies to every record read again.
5. `limits.retained_bytes` counts a spilled source's key records. `limits.component_rows` and `limits.anchor_rows` apply to a spilled source as to a held one.
6. The manifest's read record of a spilled source carries `spilled` as in phase 4.
7. Results, evidence, diagnostics, bundle tables and `retained_result_bytes` are identical to phase 4 for the same inputs. Memory runs spill details too.

Phase 5 does not change what a population or an allocation costs, or what a source reached by one costs. They are held whole. Process memory is still a measurement, not a bound.

## Customer/month workload

The developer decided on 2026-11-18, choosing among the options put to them, that explicit populations and allocations should be bounded like the sources phases 3 to 5 bound, in two phases, measured on a new workload. Before phase 6, no population or allocation input could be larger than memory: members, recipients, period sources, allocation quantities and every source a population or allocating output reached were held whole. This is a developer decision.

The workload ships on its own, before phase 6, so that hosted x86 records a baseline in which everything is still held.

1. `scripts/generate_fixture.py --workload customer-month` generates a customer/month workload with the recipe of `examples/customer_month.iresiphi`: 8,000 customers across four sales areas, 12 months taken from the targets' periods, 350,000 transactions dated in those months, and a regional target for each area and month. The 96,000 members stay under the default `limits.anchor_rows`, so the workload runs under default limits. One output anchors at the explicit population with summed sales, a transaction count, the broadcast target and an allocated target under `largest_remainder`.
2. The generator states the expected totals: sales equal the transaction total, and the allocated total equals the target total.
3. CI runs the workload on both architectures beside the 100,000-line workload (`scripts/ci/customer-month-workload.sh`), checks those totals, member and partition counts and conservation, and records peak RSS the same way.

## Contract for phase 6

1. A source can spill when it is the target of at least one relationship, no population takes recipients from it and no allocation takes a quantity from it. Outputs that allocate or anchor at an explicit population may reach it through relationships. The rest of phase 5 clause 1 is unchanged. An allocation's quantity source is held.
2. Allocation reads the population members in batches of `limits.batch_rows`. Before each batch the run reads the records of spilled sources that the batch's members reach, as phase 5 clause 4 does for anchor lines. The recipient filter and weight read those records. Phase 4 clause 5 applies to every record read again.
3. Allocation keeps one entry per member until it has assigned every partition: identity, recipient flag, weight, partition, currency and the outcomes of the lookups its weight read. `limits.retained_bytes` counts these entries.
4. Activity coverage, the matched and unmatched activity per relationship from an explicit population, is counted over key records. Its samples are source references, which key records hold.
5. A source that a population takes periods from is not held for that reason. Its first read runs every check a held read runs, takes the distinct periods and fails with `INVALID_PERIOD` as before. If nothing else reads the source, the run keeps only those periods. Otherwise its other uses decide whether it is held, streamed or spilled.
6. Members are still stored, and `limits.retained_bytes` still counts them. `limits.anchor_rows` still counts a population's members before any is created.
7. The manifest's read record of a spilled source carries `spilled` as in phase 4. A source read only for periods records the number of distinct periods it gave. Each allocation's summary gives its entry count and their bytes.
8. Results, evidence, diagnostics, bundle tables and `retained_result_bytes` are identical to phase 5 for runs within their limits. Because spilled sources count only their key records and allocation entries now count, a run near `limits.retained_bytes` may pass or fail where phase 5 did not. Memory runs bound populations too.

## Contract for phase 7

1. Members are derived, not stored. Member `n` of a population with `P` periods is eligible recipient `n / P` (integer division) in period `n mod P`: recipients in read order, periods in the population's order, as members are numbered today. A member's fields, reference and original values are those a stored member has, built when a batch reads it.
2. A population's recipient source can spill under `limits.target_bytes` like a phase 4 target, unless an allocation takes a quantity from it. The first read evaluates the population filter on every full record and keeps the positions of eligible recipients. The key record also holds the fields that relationships from the population key on.
3. Each batch of member lines, and each allocation batch, first reads its recipients' records when they spilled, then the records of spilled sources the members reach. Phase 4 clause 5 applies.
4. `limits.retained_bytes` counts a spilled recipient source's key records and no member bytes. `limits.anchor_rows` counts members before any batch is read.
5. The manifest's population record states that members are derived. A spilled recipient source's read record carries `spilled` as in phase 4.
6. Results, evidence, diagnostics, bundle tables and `retained_result_bytes` are identical to phase 6 for runs within their limits. Members no longer count toward `limits.retained_bytes`, so the boundary moves again. Memory runs derive members too.

Phases 6 and 7 do not change what an allocation's quantity source costs. It is the coarse grain and is held whole. Allocation keeps one entry per member, because a partition's weights must be summed before any share is assigned. Process memory is still a measurement, not a bound.
