## Parent

https://github.com/andrew3stedall/iresiphi/issues/16

## What to build

Aggregate and broadcast measures without changing their ownership. Deliver observable behavior through the shared public validate/explain/run seam. Use red-before-green public tests, retain decision provenance, and attach actual validation evidence before closing.

## Acceptance criteria

- [ ] Count-records/count-non-null/sum/min/max execute at explicit grouping keys with checked arithmetic.
- [ ] Independent details aggregate before combining; absent/all-null observations and distinct equal-valued observations follow decision7.
- [ ] Currency grouping, explicit input/final filters and empty anchor populations have public expected-result cases.
- [ ] Validated to-one broadcast retains origin metadata; recipient-grain sums fail and named min/max context reduction checks equality.
- [ ] Allocation/average/windows/distinct-count and unsupported grain operations fail statically.
- [ ] Every normative clause in the applicable contracts below is implemented or explicitly reported unmet; the checklist does not narrow those contracts.

## Blocked by

- https://github.com/andrew3stedall/iresiphi/issues/20

## Applicable contracts (self-contained)

### Decision #5

Source: https://github.com/andrew3stedall/iresiphi/issues/5#issuecomment-6797526877

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


### Decision #5

Source: https://github.com/andrew3stedall/iresiphi/issues/5#issuecomment-6797555729

## Scope alignment — invoice-date validity selection

The now-resolved support matrix requires invoice-date validity lookup. This supersedes the snapshot-only rate-selection restriction above: match supplier and translated item by equality, then require `valid_from <= invoice_date < valid_to`, with an explicitly nullable `valid_to` meaning unbounded future. `valid_from` and invoice date are required calendar dates. Invalid or empty intervals are rejected during data validation. Zero qualifying intervals is unmatched; overlapping qualifying intervals are ambiguous, never latest-wins. Invoice date is an explicit projected field; no implicit date extraction from timestamp. This is business-validity selection only, not bitemporal, ingestion-time, or automatic historical reconstruction. An immutable rate-source snapshot remains necessary for reproducibility.


### Decision #6

Source: https://github.com/andrew3stedall/iresiphi/issues/6#issuecomment-6797556305

## Resolution — exact, deliberately small scalar language

Autonomous decision under the developer's authority; selected behavior, not implementation evidence.

Preview scalar types are nullable Boolean, signed Int64, UTF-8 string, Decimal128(p,s) with 1 <= p <= 38 and 0 <= s <= p, and calendar Date. Collections exist at projection/detail boundaries, not as arbitrary scalar expressions. Timestamp/timezone calculations, binary floating-point arithmetic, arbitrary Python callbacks, regex, and user-defined functions are deferred. Input NaN and infinity are rejected; they are not null. Missing JSON paths and blank spreadsheet cells become null only when the declared field is nullable; malformed present values are errors. Empty string stays empty string.

Expressions comprise typed literals, field/path references, explicit casts, add/subtract/multiply, comparisons, Boolean AND/OR/NOT, is-null, coalesce, case, and explicit decimal quantization. Division is deferred and rejected statically, including division by zero; no engine-dependent infinity. Date equality/order and the explicit rate-validity predicate are supported. No implicit string-to-number conversion or cross-type key comparison. Null comparisons yield unknown; three-valued Boolean logic applies; filters retain only true; case tests unknown as not true. Coalesce is explicit, never automatically used to fill unresolved charges.

All integer arithmetic is checked; overflow fails the run. Decimal arithmetic is exact until an explicit quantization. Addition/subtraction use scale=max(s1,s2) and precision=max(p1-s1,p2-s2)+scale+1; multiplication uses precision=p1+p2 and scale=s1+s2. An inferred precision beyond 38 is rejected statically rather than silently reduced. Quantization names target precision/scale and rounding mode; preview supports half-even and toward-zero. Narrowing overflow fails; there is no saturation or wrap. Avoid declaring every input decimal(38,s) when a narrower business range is appropriate.

The invoice fixture declares quantity decimal(12,4), rate decimal(14,6), billed decimal(18,2); multiply exactly then half-even quantize expected charge to decimal(18,2), subtract billed minus expected at cent scale. Currency is a required explicit code; the fixture uses one currency. Mixed-currency summation/comparison without grouping by currency is rejected. FX, tax, tiering, credits, and discounts are deferred business operations. Positive quantities are an acceptance-fixture constraint, not a universal decimal restriction.

Transport canonical integers and decimal values as tagged decimal strings in portable JSON; decimal tags carry precision/scale. Python may expose native int/Decimal, but conversion must be exact and validated in Rust. JSON numeric literals are not accepted for declared decimal fields unless the adapter preserves their original lexical value and parses it exactly. XLSX numeric cells require an explicit decimal quantization policy because workbook numeric storage cannot establish original monetary precision; source evidence retains the observed cell representation. No zero-copy or exact-original-Excel-decimal claim follows.

Dates accept only explicit ISO YYYY-MM-DD strings or adapter-declared date cells mapped to Gregorian dates. Locale guessing is forbidden; Excel date-system metadata and invalid serials must be handled explicitly by the adapter. Timestamps require caller preprocessing to a declared business date; local timezone/calendar inference is deferred. Rate validity is inclusive lower/exclusive upper date, with explicit open-ended upper bound. No availability-time or bitemporal semantics.

Examples: 2*50 -> 100.00 and 110-100 -> 10.00; 1*45 ->45.00 and 50-45 ->5.00; half-even 1.005->1.00 and 1.015->1.02; null rate yields null expected/variance with unresolved status; Int64 max+1 fails; `null = null` is unknown; empty string is not null; a rate ending 2026-11-12 does not apply on that date. Identical results/errors are required through supported Rust/Python public interfaces. Future executors must prove these semantics before accepting pushdown.


### Decision #7

Source: https://github.com/andrew3stedall/iresiphi/issues/7#issuecomment-6797569343

## Resolution — owned measures and explicit grain transitions

Autonomous decision under delegated authority. Preview supports count-records, count-non-null, sum, min, max at explicitly declared grouping keys. Average, distinct-count, windows, semi-additive measures, custom aggregates and allocation are deferred, with `UNSUPPORTED_OPERATION` at validation. Allocation weights, denominator normalization, residual rules and conservation are therefore not silently defaulted; a later allocation effort must decide them together. Output filtering cannot be represented as an allocation surrogate.

Every measure has its originating grain. Aggregate each independent fact collection at the requested shared grain before combining it; never join independent detail collections and aggregate the expanded result. Grouping must retain required semantic dimensions such as currency. Duplicate observations remain separate only when they have distinct declared identities; duplicate identity is an error, not an invitation to DISTINCT the amounts. Billed 10 on two distinct lines sums to 20, not 10. Three components under one billed-110 line do not change the billed sum from 110.

Aggregate input predicates apply before grouping. Final output predicates apply after grouping and do not change the group population. Absent anchor groups are not manufactured. For an existing anchor with zero related observations, count is zero and sum/min/max are null. For all-null observations, sum/min/max are null and count-non-null is zero. Numeric overflow fails the run. Decimal sum retains input scale and accumulates checked decimal128 precision 38; further arithmetic whose inferred precision exceeds 38 requires an explicit checked narrowing cast justified by a declared range. No silently lossy reduction.

Broadcast is permitted only along a validated to-one path and carries originating-grain metadata. The repeated value is context, not a newly additive measure at recipient grain. Re-aggregating a broadcast measure at recipient grain is rejected; aggregate from its original source or request a named min/max context reduction whose equality is validated. A customer-level budget repeated on ten lines is not ten budgets. Nested component scalar selection remains invalid without explicit aggregation or the separate detail output.

The invoice primary output preserves every line. Comparable totals explicitly select resolved lines: billed 160, expected 145, variance 15. Full-population billed is 180; unresolved billed is 20; total and resolved/unresolved counts accompany the comparison summary. A sum ignoring null expected values must never be labelled complete reconciliation. Observed component totals are separate measures; no equality with billed charge is presumed.

Acceptance adds unequal component counts on lines with equal billed values, two independent detail collections, empty detail sets, all-null values, currency mixing, a forbidden broadcast sum, and input-filter versus final-filter examples. Unsupported allocation fixtures must fail validation, not produce invented shares. These decisions preserve allocation as a broader requirement without making it a preview prerequisite.


### Decision #9

Source: https://github.com/andrew3stedall/iresiphi/issues/9#issuecomment-6797608785

## Resolution — explicit validation coverage and inspectable run evidence

Autonomous decision under the developer's authorization. The shared public operations return language-neutral structured records; Python wraps the same core results. Names below define contracts, not implemented APIs.

`validate(recipe)` performs version/reference/type/path/grain/capability validation without opening sources. `validate(recipe, bindings, policy)` additionally inspects source schemas and access/capability feasibility, with `data_checks="none"` by default. Explicit `data_checks="full"` scans the selected source populations for identity, types, relationships and rate validity. There is no sampled mode in the preview. Reports list each check as passed/failed/not-run and its population/coverage; static validity never implies data correctness. All actual runs perform full required checks again against their own read materialization: earlier validation is evidence, not permission to skip checks on changed data.

`explain` exposes logical nodes, named source/field/path dependencies, population and filter stages, grain transitions and measure origins. With bindings/policy it adds feasible local placement, reads/transfers, estimates including unknown values, unsupported capability reasons and budgets. It never executes full data checks implicitly or claims a cost optimum. Explanation and logical lineage use stable declaration IDs rather than unstable engine internals.

`run` returns a run ID, terminal status (`succeeded`, `succeeded_with_unresolved`, `failed`, `cancelled`), output descriptors only for committed outputs, and a manifest/report. The invoice review's unmatched/ambiguous lookup lines yield `succeeded_with_unresolved` and retain one primary row with null dependent values and explicit reason. Missing/duplicate anchor identity, invalid typed data, overflow, policy violations and strict relationship violations fail publication. An empty selected population succeeds with zero counts and no completeness claim about excluded inputs.

Diagnostics include stable code, severity, phase, source/projection or expression/path ID, check ID, affected-output IDs, exact affected-count when known, bounded source-reference samples, and human message/remediation. Values are omitted by default to avoid leaking data; explicit diagnostic-value opt-in is recorded. The default sample cap is 20 references per diagnostic, with truncation and count status explicit. Per-line status and an evidence table preserve the targeted invoice reconciliation beyond these bounded diagnostic samples.

The invoice evidence outputs include source observation reference and original projected values for each line, matched translation/enrichment/rate source references and relevant original values, candidate counts and ambiguity references, and component parent links. Candidate evidence is materialized as a separate table and subject to explicit output/resource bounds; exceeding a bound fails rather than silently claiming complete evidence. It does not promise complete row-contribution provenance for arbitrary aggregate expressions. Logical lineage still identifies all contributing source fields and operations.

Run manifests include recipe hash/format/package versions, safe binding identities and content fingerprints, declared policy, executed nodes, actual source/eligible/output row counts, resolved/unresolved counts and amounts by currency, bytes read/written where measurable, unknown-measurement markers, checks/coverage, timings, and committed output/evidence locations. A source fingerprint refers to bytes actually read or the immutable materialization, not a later reread. There is no shared multi-file snapshot guarantee; concurrent changes detected during access fail or are reported as unreproducible, never silently certified.

Failed/cancelled runs retain bounded diagnostics and a manifest with incomplete checks/counts explicitly marked, plus cleanup evidence. They expose no committed result as successful. The public acceptance suite must cover the agreed tiny invoice fixture, corrupted identities/keys/intervals, unresolved preservation, fanout prevention, save/rebind/reopen parity, and budget/cancellation/sink failures. Real JSON/CSV/XLSX reads and actual output publication are required beyond fixture-only tests. The 100,000-line workload measures complete access-through-delivery behavior; it does not establish a performance guarantee in advance.


### Decision #10

Source: https://github.com/andrew3stedall/iresiphi/issues/10#issuecomment-6797627647

## Resolution — batch contracts and explicit local placement

Autonomous decision under the developer's authorization. Preview execution placement is explicitly local. No remote source pushdown, remote staging, automatic federation or pairwise connector code ships. The following interface responsibilities preserve future execution choices without pretending those integrations exist.

A source adapter describes projection schema/nullability, stable binding identity and fingerprint support, record-reference production, supported extraction options, estimates with known/unknown status, and access/cancellation limits. It opens a bounded pull-based stream of typed column batches plus source references, validating declared values and preserving raw evidence fields. `close` releases owned resources on all terminal paths. No adapter may silently infer business keys, paths, grain, rate policy or monetary rounding. Batch transport is an internal versioned logical schema; an engine-specific dataframe is not the public adapter contract. Chosen bindings may implement Arrow interchange internally subject to the binding decision.

Capability declarations include supported logical operations, type/precision/null/date semantics, projection/filter restrictions, accessible locations, transfer directions, ordering guarantees, cancellation granularity and resource-accounting limits. They are negotiated against the exact required plan, not a Boolean 'supports SQL'. Unknown capability is unsupported until established. Source scanning/projection does not imply source-side aggregation support.

The logical plan contains scan/project, typed feature evaluation, checked named lookup including date validity, population/filter stages, grouped aggregate, validated broadcast, separate detail/evidence production, and output delivery. An executor accepts only a plan whose exact operations and semantics it can preserve; it returns batches, execution/check evidence and structured errors. Iresiphi owns semantic validation, unresolved-status propagation, identity/cardinality contracts and public lineage; lowering must not discard them. The engine decision selects a concrete implementation behind this seam.

A sink describes accepted types, delivery location, bounded-batch consumption, commit/abort semantics and ownership. Preview sinks are memory and local directory bundles containing primary/detail/evidence CSV or JSON plus schema/manifest. CSV requires accompanying type schema and an unambiguous null representation/escaping policy so empty strings differ from null; no bare CSV is claimed to be lossless. Output ordering is unspecified unless an explicit supported order is requested; the preview does not provide a general sort operation. Stable record IDs permit comparisons independent of order.

Planning takes the transitive closure of requested outputs and evidence, including hidden match keys, invoice date, validity endpoints, population/filter predicates and grain checks. Unrequested sources need not be read, but required validation columns cannot be projection-pruned. Policy declares allowed read roots, allowed output/temp roots, local transfer permission and resource limits. Unknown sizes remain unknown and require runtime enforcement; estimates never override hard policy. Shared reads/calculations may be reused only where population and semantic identity agree.

Concrete supported composition: supplier-specific invoice JSON projections, XLSX enrichment, CSV mappings/rates/components all stream through the same source contract into the local executor and produce one invoice-line output plus detail/evidence tables. No JSON-to-XLSX-specific join implementation is needed. Policy-blocked scenario: a required XLSX file lies outside allowed read roots, or the output bundle lies outside the approved destination; bound explain/validate reports infeasibility and run refuses before unauthorized access. A future remote-only source with local transfer prohibited is also infeasible; there is no hidden download fallback.

Future remote/hybrid extensions must declare same-environment accessibility, exact supported semantics, transfer/staging permission and ownership, estimates/uncertainty, cancellation and cleanup behavior, and sink capability. A planner may choose among only semantically equivalent feasible placements. Adding a new source must not require pairwise adapters for existing formats. Detailed remote integration remains deferred by scope, not unresolved preview fog.

