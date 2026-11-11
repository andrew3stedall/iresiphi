# Decision history 5

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). Resolve identity, relationship, and population rules against the confirmed invoice-review acceptance contract, especially supplier scope, preservation of unresolved invoice lines, and the separation of invoice-line records from finer observed detail.

## Question

What precise identity, relationship, and output-population rules must the preview support to make the selected workflow unambiguous?

Define record projection and source grain separately from row identity; supported keys and cardinalities; uniqueness and null-key requirements; matching and unmatched-record behavior; and how named relationship paths resolve differing meanings. Specify which requests must be rejected rather than implicitly expanded or repaired.

Define how the eligible output population is supplied or derived, whether absent observations mean zero or unknown, and how population construction differs from output selection. Make filter stages explicit, especially eligibility filters versus filters on completed aggregates or allocations.

Use concrete duplicate-key, unmatched-key, fanout, missing-period, and ambiguous-path examples. Record the supported subset, explicit exclusions, and examples sufficient for independent expected-result fixtures. Add or refine glossary terms only when their meanings have been agreed.


https://github.com/andrew3stedall/iresiphi/issues/5#issuecomment-6797526877

## Resolution — explicit identities, anchored population, and named paths

Autonomous decision under the developer's instruction to resolve the map without further interviews. These are selected preview contracts, not implemented behavior or a claimed live interview.

- A record projection declares record collection, field paths, types, and grain without changing stored data. One JSON projection may expand one declared array and carry its parent header fields; unrelated arrays are separate projections, never an implicit Cartesian product. XLSX projects one named sheet/header region. Each projected record has a source reference: run binding identity and content fingerprint, projection name, and JSON pointer or sheet/row coordinate. This reference identifies an observation, not a business entity across runs.
- Business identity is an explicitly declared non-null composite key with type-consistent components. Invoice lines use supplier/invoice/line identifier; if no stable line identifier exists, an explicit positional key scoped to that input document is permitted and described as unstable across edits. Duplicate/null keys in an anchor or observed-detail identity fail the run before output publication. Grain describes the meaning of one record; row references do not prove uniqueness of business grain.
- Preview relationships use typed equality over named composite keys. Declare one-to-one, many-to-one, or one-to-many. Null matching components never match. No implicit coercion, trimming, case folding, fuzzy matches, many-to-many expansion, or first/last-match repair. Normalization is a separately named feature retaining the original value.
- Translation keys include supplier and raw item value; rate keys include supplier and translated item plus an explicit rate-period/version key. Preview uses a caller-selected, versioned rate snapshot. Automatic effective-date/as-of selection is deferred; duplicate candidate rates remain ambiguous, rather than guessing a temporal rule. A recipe cannot claim historical pricing correctness merely from invoice date.
- Every nonlocal field names its relationship path. Two routes such as invoice supplier versus customer supplier are distinct even if currently equivalent. Omitted ambiguous paths are static errors. Cyclic dependency graphs are rejected; a reused source occurrence has its own alias and explicit path.
- The output anchor is the extracted invoice-line projection. Its eligible population is formed only by explicit anchor eligibility predicates; the acceptance recipe uses all lines. Enrichment/mapping/rate lookups preserve every anchor. Zero candidates produces `unmatched`; more than one candidate for a declared to-one lookup produces `ambiguous`, with no selected value. A unique candidate produces `matched`. Status propagates to dependent expected charge/variance as unresolved null plus reason. Lookup cardinality violations are recorded and quarantined per affected anchor for the invoice review; recipes may instead explicitly demand strict failure. Duplicates are never silently deduplicated, even if values agree.
- Observed components are a separate keyed detail output with parent-line reference. Access at line grain requires an explicit aggregate; selecting scalar component fields at line grain is rejected. Three components beneath a line with billed charge 110 leave one primary row and charge 110. Orphan detail records are reported in source diagnostics, not turned into new invoice lines.
- Filter stages are explicit: source/projection eligibility, anchor population eligibility, aggregate input predicate, and final output selection. A final selection does not redefine lookup uniqueness or aggregate inputs. Acceptance's unresolved rows remain present; comparable-subset totals use an explicit resolved-status predicate. There is no implicit dense calendar or zero filling: a missing month has no row unless supplied by an explicit anchor; a present anchor with no measure observation is unknown.

Acceptance cases: supplier A and B sharing item ID select their own rates; absent translation retains the line with unresolved charge; two rate candidates retain one ambiguous line with null expected charge; duplicate anchor key fails; two competing paths require a named choice; component multiplicity never changes line totals; a missing month is not fabricated. Cross-run reproducibility requires retained immutable inputs and bindings, not source coordinates alone.

No new investigation is needed for these semantic choices. Engine and adapter decisions must implement or explicitly reject these contracts, not weaken them.


https://github.com/andrew3stedall/iresiphi/issues/5#issuecomment-6797555729

## Scope alignment — invoice-date validity selection

The now-resolved support matrix requires invoice-date validity lookup. This supersedes the snapshot-only rate-selection restriction above: match supplier and translated item by equality, then require `valid_from <= invoice_date < valid_to`, with an explicitly nullable `valid_to` meaning unbounded future. `valid_from` and invoice date are required calendar dates. Invalid or empty intervals are rejected during data validation. Zero qualifying intervals is unmatched; overlapping qualifying intervals are ambiguous, never latest-wins. Invoice date is an explicit projected field; no implicit date extraction from timestamp. This is business-validity selection only, not bitemporal, ingestion-time, or automatic historical reconstruction. An immutable rate-source snapshot remains necessary for reproducibility.
