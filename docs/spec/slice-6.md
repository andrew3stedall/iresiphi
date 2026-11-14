# Slice 6: evidence for problem lines by default

Parent: https://github.com/andrew3stedall/iresiphi/issues/1.

Until this slice a run recorded evidence for every selected line: the line's original values, every candidate of every lookup, and every component, each with its own original values. For the 100,000-line workload that turned 13.5 MB of input into 757 MB of results and evidence. The run failed under the default 256 MiB limits and peaked above 5 GB of resident memory ([measurement](../measurements/workload-100k.md)).

The developer decided on 2026-11-14 that this is not what the library is for. Its job is to describe how datasets connect and to make them easy to use together; it should not have to hold or emit much data. A full per-line trace is an audit tool, so it becomes something a policy asks for. This is a developer decision. It supersedes the part of decision #9 that made complete line evidence unconditional.

## Contract

1. **Policy.** The policy gains `evidence`, one of:
   - `problems` (the default): a run records primary, candidate and detail evidence only for selected lines whose `_status` is not `matched`.
   - `full`: a run records evidence for every selected line, exactly as before this slice.
2. **What does not change.** Output rows and their `_status`, `_reason` and `_parent` columns, counts, totals, reconciliation, diagnostics and their samples, checks, source reads, placement and transfers are identical under both levels. Evidence level never changes a recipe's canonical JSON or hash.
3. **A recorded line is recorded whole.** Under `problems` an unresolved line carries the same primary record, candidate records and detail records it has under `full`. The `problems` evidence is the `full` evidence restricted to lines that did not resolve, in the same order.
4. **Allocation evidence** is per partition, not per line, and is recorded under both levels. A member's own `allocations` entry is part of its primary record and follows rule 1.
5. **Limits.** `limits.output_bytes` and `limits.retained_bytes` count what the run records. `limits.candidate_pairs` still bounds qualified lookup candidates, and the candidate and detail records the run writes. Evidence is never truncated to fit: a level's records are written in full or the run fails with `RESOURCE_LIMIT`.
6. **Disclosure.** The manifest's `policy` and `evidence.level` state the level. Under `problems` the `evidence` check's coverage says how many selected lines were recorded and that resolved lines are counted, not recorded. The `output.population` check claims each line appears once in its output; it claims the same of the evidence only under `full`.
7. **Bundles** always contain the `_evidence`, `_candidate_evidence` and `_detail_evidence` tables. Under `problems` they hold only unresolved lines and are empty when every line resolved.
8. **Bindings.** Python's `policy(...)` takes `evidence=`; the Node `Policy` type gains `evidence`. Both pass the value through unchanged.

## Acceptance

Through the public validate/explain/run seam:

1. The two-supplier invoice fixture under the default policy: the same rows, totals, diagnostics and counts as under `full`; evidence only for the unresolved lines, each record equal to its `full` counterpart.
2. A fixture in which every line resolves records no primary, candidate or detail evidence, and its bundle's evidence tables are empty.
3. The 100,000-line workload runs under the default limits, with no raised byte limits, on both architectures.
4. An unknown `evidence` value is `INVALID_POLICY`.

## Out of scope

Removing the repetition inside `full` evidence (each primary record still embeds its candidates, which the candidate table also holds). Sampling. Recording evidence for chosen lines or outputs. Re-deriving evidence for one line after a run. A level that records nothing.
