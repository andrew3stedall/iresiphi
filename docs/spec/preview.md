## Problem Statement

Analysts standardize invoice and lookup inputs to compare billed and contracted charges, then struggle to reconcile results to original observations. This preview must preserve evidence, unresolved populations and measure grain while providing a reproducible local Rust/Python workflow.

## Solution

Ship the bounded local invoice-review preview resolved by map #2. This is a NEW release specification; broad product issue #1 remains context and is not rewritten. The public seams are validate/explain/run in Rust and Python, as already approved. User authorization expressly waives further seam and granularity interviews.

## User Stories

1. As an analyst or developer, I want to project two different supplier invoice layouts without rewriting source data, so that original evidence remains inspectable.
2. As an analyst or developer, I want to bind local JSON, CSV and XLSX with explicit schemas, so that source formats do not determine relationship meaning.
3. As an analyst or developer, I want to preserve all invoice lines including unmatched and ambiguous ones, so that unresolved charges remain visible.
4. As an analyst or developer, I want to select rates by supplier, translated item and invoice-date validity, so that overlapping intervals never silently choose a winner.
5. As an analyst or developer, I want to inspect original lines, mapping candidates, enrichment, rates and components, so that each comparison can be reconciled.
6. As an analyst or developer, I want to calculate exact checked amounts with explicit rounding, so that binary floating-point cannot change currency results.
7. As an analyst or developer, I want to use typed null and date semantics, so that absent values do not masquerade as zero.
8. As an analyst or developer, I want to aggregate independent facts at their owned grain, so that component fanout cannot multiply invoice amounts.
9. As an analyst or developer, I want to broadcast only validated contextual measures, so that repeated values cannot become additive.
10. As an analyst or developer, I want to distinguish comparable billed 160, expected 145 and variance 15 from full billed 180 and unresolved 20, so that partial reconciliation is honest.
11. As an analyst or developer, I want to save, reload and rebind a versioned portable recipe, so that recipes remain independent of file paths and host objects.
12. As an analyst or developer, I want to reuse bundled catalog declarations with content hashes, so that reopening needs no remote catalog.
13. As an analyst or developer, I want to validate statically or explicitly scan all data, so that coverage is never overstated.
14. As an analyst or developer, I want to explain dependencies, population, grain, placement and unknown estimates, so that execution is inspectable.
15. As an analyst or developer, I want to receive structured failures and incomplete-check reports, so that errors remain useful without publishing partial success.
16. As an analyst or developer, I want to constrain local access and resource budgets, so that runs cannot silently read remote data or escape approved roots.
17. As an analyst or developer, I want to cancel a run and release handles, so that retained resources have predictable ownership.
18. As an analyst or developer, I want to consume bounded copied Python batches with exact Decimal and date values, so that Python and Rust agree.
19. As an analyst or developer, I want to publish a new atomic local bundle with unambiguous CSV null escaping, so that retry cannot overwrite a previous result.
20. As an analyst or developer, I want to run the complete 100,000-line workflow with measured evidence, so that preview capacity is demonstrated rather than asserted.
21. As an analyst or developer, I want to install aligned Rust and CPython 3.12/3.13 Linux packages, so that the preview can be reproduced.
22. As an analyst or developer, I want to receive explicit unsupported-operation errors, so that deferred product features are not silently approximated.

## Implementation Decisions

The following exact resolution extracts are normative. Planning-tense statements define implementation obligations, not implementation evidence. In #5 the date-validity amendment overrides snapshot-only selection. In #11 scale through 38, rounding, and explicit XLSX numeric conversion override the research restrictions. Verify version availability and record any corrective stack decision explicitly before substituting dependencies. All requirements below map to child acceptance criteria; absence from a short checklist does not waive a normative clause.

### Decision #3

Source: https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135

## Resolution — invoice-review workflow and preview acceptance

Confirmed by the developer after the live interview. This resolves the representative workflow and its acceptance criteria; it does not select the detailed policies and technologies assigned to other map tickets.

### Intended user and recurring problem

Analysts repeatedly assemble invoice-review outputs using standardization and lookups. Once invoice and rate-card fields have been standardized, reconciling a result with the raw inputs becomes difficult.

The developer preview will demonstrate an auditable invoice review that preserves original values and makes the source/lookup chain inspectable while comparing billed and contracted charges. It is repeatable batch work; no particular daily or monthly cadence is required.

### Representative inputs and variation

Use publicly reproducible synthetic data representing the described real workflow:

- Two suppliers with different invoice JSON header/line structures, native field names, and item descriptions.
- AI extraction supplies the JSON upstream; extraction itself is not part of this workflow.
- Separate Excel enrichment for invoice lines.
- An explicit translation table connecting original line values with contracted-item identifiers.
- Supplier-specific contracted rate records.
- Existing component records at a finer grain beneath invoice lines.

The same translated item identifier can occur for different suppliers, so its identifier alone must not select a rate. Concrete matching keys, validity-date rules, connector versions, and declaration interfaces remain downstream choices.

Synthetic fixtures are sufficient for initial preview acceptance. No organizational data access is a prerequisite. These fixtures represent the real workflow's shape, not evidence of customer demand, extraction accuracy, or observed production performance.

### Primary output and reconciliation

The primary output has one record per extracted invoice line and retains the entire extracted line population, including unmatched or ambiguous comparisons. It carries invoice-header context, original line values, mapped values, enrichment, billed charge, expected charge where resolved, and variance where resolved.

Observed component records remain linked detail. Multiple components must not multiply a line's billed amount, expected amount, or comparison result. This is source-supplied detail, not an implicit allocation of line spend. No universal equality between component sums and billed charges has been agreed.

Reviewers must be able to inspect original values and source record identifiers or JSON paths through the relevant enrichment, translation-table entry, selected rate-card record, and linked components. This is a targeted reconciliation requirement, not a promise of exhaustive computational provenance for arbitrary operations.

Missing mappings/rates and ambiguous matches remain identifiable. The output must not invent expected amounts, silently discard unresolved lines, or label a partly resolved population as fully reconciled. Exact diagnostic representations and execution/failure policies belong in the corresponding downstream tickets.

### Manually reviewable acceptance example

Use positive quantities and a single declared currency for this simple fixture. Expected charge is quantity multiplied by the applicable contracted unit rate. Variance in this fixture is billed charge minus expected charge, with positive values indicating overcharge.

- Supplier A: quantity 2, billed charge 110, selected contracted unit rate 50. Expected charge 100; variance 10.
- Supplier B: quantity 1, billed charge 50, selected contracted unit rate 45. Expected charge 45; variance 5.
- Both resolved lines use the same translated item identifier but supplier-specific rates.
- A third unmapped line has billed charge 20. It remains in the primary population with an unresolved expected charge and variance.
- The comparable subset has billed charges 160, expected charges 145, and variance 15.
- The complete population has billed charges 180, including unresolved charges of 20. It must not be described as fully reconciled.
- Include several observed components beneath at least one line and ensure the primary row count and totals are unchanged by their multiplicity.

These are confirmed synthetic expected values, not executed test or benchmark results. The example does not establish support for every tax, discount, credit, currency-conversion, or temporal pricing rule; precise pricing/date/numeric policies remain separate decisions.

### Observable completion criteria

Through the agreed public validate/explain/run boundary:

1. Accept the two supplier layouts through explicit projections, bindings, and mappings without requiring their stored data to be standardized first.
2. Preserve invoice-header/line relationships and return the complete requested line population with the expected resolved and unresolved comparisons.
3. Make the original values, mapping steps, selected rates, and observed component relationships inspectable.
4. Produce the manually reviewed values above and keep comparable-subset totals distinct from full-population totals.
5. Detect corrupted variants containing missing mappings or rates, ambiguous/duplicate matches, and join fanout. Do not fabricate charges, silently lose lines, or multiply totals.
6. Support another review run after rebinding or updating an explicitly declared lookup without rewriting original invoice data. Exact recipe/interface mechanics remain downstream decisions.
7. Exercise approximately 100,000 generated invoice lines across multiple invoices, with variable component counts, alongside the tiny fixture. Measure behavior under declared resources; no elapsed-time, memory-budget, hardware target, or comparative speed claim is settled here.

### Consequences for the remaining map

- This invoice workflow replaces the customer/month example as the first-release acceptance anchor. The earlier example remains existing research/prior art; it is not deleted.
- JSON and Excel are concrete acceptance input requirements to account for when choosing the release support matrix. Physical formats for the remaining lookup/detail datasets, exact adapters, execution placement, runtimes, and sinks remain open.
- Allocation is not required by this observed-component workflow. Its developer-preview coverage remains a scope choice; the broader product requirement remains intact.
- The public-contract decision must account for targeted source-record reconciliation and identifiable unresolved comparisons. The identity/relationship and grain decisions must preserve the full line population and avoid component-induced fanout.
- The shared glossary now distinguishes value mapping, output population, observed detail, and source record reference. These are domain definitions; the decision itself lives in this resolution.
- Existing downstream tickets already cover the resulting precise questions. No new research, prototype, or preparation task is justified yet; add one when a specific evidence gap or access dependency becomes known.

The decision was explicitly confirmed by the developer. Proceed to the next frontier in a later decision session.


### Decision #4

Source: https://github.com/andrew3stedall/iresiphi/issues/4#issuecomment-6797517465

## Resolution — bounded local invoice-review preview

Decided autonomously under the developer's explicit authorization to resolve the whole map; this is a planning decision, not a claim of implemented support.

Ship a shared Rust library and Python interface on CPython 3.12/3.13, initially Linux x86_64. Inputs: local UTF-8 JSON invoice documents with explicit header/array projection; local XLSX worksheets with declared sheet, header row and typed columns; UTF-8 CSV lookup, rate and component tables with explicit dialect/schema. No inferred business mappings or schema coercion. XLSX formulas require stored cached values; missing caches fail, and the preview does not calculate formulas. Preserve original cells/paths and source identities for review.

All sources enter a single local executor through the same typed batch contract. Ship explicit local placement only, with no network reads, pushdown, remote staging, automatic transfer, or hybrid execution. Caller supplies permitted local roots and resource limits. Local files with different formats demonstrate cross-source composition without requiring database credentials. Output: a typed in-memory result and a local directory bundle of JSON manifest plus CSV line-review, linked-component, and reconciliation tables. Exact delivery semantics are settled by the resource/failure decision.

Defer TypeScript/Node/browser distribution, macOS/Windows wheels, dataframes, SQL, Snowflake, Spark, NDJSON, XML, arbitrary text, Parquet and remote stores. These remain product requirements: source adapters must report schema, identity, capabilities and location; executors must advertise semantic compatibility and access; sinks are independent of input. Unsupported bindings/operations fail explicitly, never silently collect data or substitute an executor. No pairwise connector-specific relationships.

The invoice acceptance workflow ships with explicit supplier-scoped mapping and rates, observed component detail, exact currency arithmetic, aggregation and safe broadcast. Allocation, arbitrary UDFs, currency conversion, tax engines and general temporal joins are deferred. Basic invoice-date rate validity filtering remains required. This bounds the preview without deleting broader functionality from the product specification.

Acceptance remains the already agreed two-supplier fixture and 100,000-line generated workload. This decision sets no speed claim or guaranteed platform beyond those above.


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


### Decision #8

Source: https://github.com/andrew3stedall/iresiphi/issues/8#issuecomment-6797588137

## Resolution — portable recipe document with runtime bindings

Autonomous decision under the developer's authorization. `.iresiphi` is UTF-8 JSON, not executable Python, a zip, or an engine plan. The root includes `format_version: "0.1"`, human recipe name, source declarations, named relationships, named features, outputs and required capabilities. Stable local IDs identify declarations; references resolve by ID and duplicate IDs fail. JSON object ordering is insignificant; list ordering is significant only where the schema states it (e.g. composite keys and output columns). Unknown semantic properties/versions fail, rather than being ignored; a reserved annotations object may carry nonsemantic metadata.

A source declaration carries projection, field types/nullability, identity, grain, and required adapter capabilities. Relationships carry named paths, typed keys/cardinality, and validity predicates where used. Expressions are a typed data AST. Outputs carry anchor, population/filter stages, fields, explicit grain operations, and detail relationships. Decimal/integer literals use the tagged exact string transport selected in the scalar decision. No arbitrary SQL, Python code, callbacks, engine-specific AST, or dynamic imports.

Reuse uses an in-memory builder/catalog that copies selected declarations into a self-contained recipe. The saved document has no unresolved catalog/network reference: named reusable definitions are bundled, with their content hashes and optional source-catalog provenance. A separately hosted/versioned catalog service is deferred. Conflicting names require explicit aliases; no latest-version lookup on reopening. A recipe may have a human revision label, but its exact semantic identity is a hash of canonical validated document content excluding explicitly nonsemantic annotations. Canonical JSON serialization and its hash algorithm are part of the format specification to implement; choose SHA-256 over recursively key-sorted, compact UTF-8 JSON with only string-encoded numeric literals, no insignificant whitespace, and preserved array order.

Bindings are supplied separately per run: source IDs map to concrete file paths/read handles and adapter settings; sink choice and execution/resource policy are also runtime inputs. Saved recipes contain no secrets, connections, live objects, absolute machine paths, or source data. A run manifest records their safe identities/fingerprints and redacted settings separately. Reopening validates version, references, types and supported capabilities before requesting bindings. Missing bindings have a structured error at bound validation/run, not at unbound recipe validation.

Rust owns validation and serialization. Python builders are ergonomic construction helpers over that same model; loading and saving in either supported surface preserves meaning. Future TypeScript must implement the same portable model and tagged values. Host-only extensions are rejected for this preview, so there is no silent loss of behavior on reopen.

Version compatibility is exact format-version acceptance for the preview. Package patches may fix bugs without changing the file schema; a semantic/schema change increments format version, and old files require an explicit migration tool with reviewable output. No automatic best-effort migration. Required tests are Rust/Python save-load equivalence; duplicate/missing IDs; unsupported version/property/capability; secret-free persistence; and rebinding the unchanged invoice recipe to a second input set.


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


### Decision #11

Source: https://github.com/andrew3stedall/iresiphi/issues/11#issuecomment-6797686446

## Resolution — DataFusion behind an Iresiphi semantic plan

Autonomous decision under the developer's authority. Choose DataFusion 55.2.0 with Rust 1.94.0 as the implementation baseline, using its Arrow re-export rather than selecting an independent Arrow version. Pin direct dependencies and Cargo.lock. [Engine and bindings feasibility evidence](https://github.com/andrew3stedall/iresiphi/issues/15) supplies primary-source research, not a successful combined build or benchmark.

DataFusion supplies relational execution, batches, optimizer/physical machinery and participating-operator memory reservations. Iresiphi owns the semantic DAG, typed scalar rules, identity checks, population preservation, lookup-candidate counting, unresolved statuses, grain ownership, source references, checks and public explanation. Lower only validated operations into the engine. Optimizations must preserve checks and hidden dependencies; assert output population/identity invariants after execution. Count date-qualified candidates before selecting a rate, never use a post-join filter that drops unmatched anchors.

Choose it over Polars 0.55.2 because the documented resource integration and execution-plan cancellation contracts give an appropriate seam for this preview. Polars remains a viable future alternative; neither engine has a measured speed advantage here. Building a relational engine from scratch would add unnecessary scope. Source readers remain explicit adapters: serde_json 1.0.154 with raw_value, csv 1.4.0, calamine 0.38.0. Built-in readers must not bypass the adapter's evidence or type rules.

The research's suggested scales <=18 and rejection of rounding do NOT supersede the resolved scalar contract. Support declared decimal precision/scale up to 38, half-even and toward-zero quantization, exact checked arithmetic, and the established inference/rejection rules. Implement checked Rust kernels or engine extensions when DataFusion's native coercion/arithmetic cannot satisfy that contract. Such kernels are a deliberate semantic boundary, not a claim native behavior already conforms. Exact integer-coefficient reference cases must test value, resulting schema, overflow, negative ties, and scale extremes. There is no fallback to float or silent narrower precision.

Likewise the research's blanket rejection of floating XLSX monetary cells is superseded by the explicit quantization policy: absent that policy, reject; with it, interpret the observed finite numeric cell via an explicitly documented conversion to its shortest round-trip decimal representation, then apply the declared checked quantization. Record observed numeric representation, precision policy and cell reference. This does not recover the originally intended decimal. Prefer text decimal cells in the acceptance fixture. Formula cells require cached values, never evaluation; missing/uncertain cache presence is an adapter error, and freshness is not certified. Validate both Excel epochs, reject the fictitious 1900 leap day and fractional date-only serials.

Unsupported plan operations fail before execution; invalid data and overflow fail before publication. No spill or remote placement is implied by choosing DataFusion. Integration gates: locked build, real adapters, exact checked kernels including quantization, date/ambiguity cases, safe grain operations, memory/cancellation/error cleanup and full invoice acceptance. Failure of an upstream native kernel triggers the specified wrapper implementation, not weakening the contract. If the chosen version cannot support these explicit seams, implementation must report a concrete blocker and reopen the decision; research has identified no such blocker now.


### Decision #12

Source: https://github.com/andrew3stedall/iresiphi/issues/12#issuecomment-6797699573

## Resolution — PyO3 handles and copied exact Python values

Autonomous decision under the developer's authority, grounded in [Engine and bindings feasibility evidence](https://github.com/andrew3stedall/iresiphi/issues/15). Choose PyO3 0.30.1 and maturin 1.16.0. Support ordinary GIL-enabled CPython 3.12 and 3.13 on Linux x86_64, glibc >=2.28; separate cp312/cp313 wheels. Defer abi3, free-threaded Python, PyPy, browsers and Node/TypeScript delivery.

Rust owns recipe validation, logical planning, serialization, execution and diagnostics. Python builders construct the same declarative values; opaque recipe/run/result handles own Rust state with reference counting. They never borrow Python buffers or connection lifetimes for worker use. Internal batches are DataFusion-compatible Arrow RecordBatch with declared schema, Decimal128/Date32 and validity bits. Python materialization copies to int, Decimal constructed from exact text, datetime.date, str, bool and None. No float intermediates for exact values, PyArrow dependency, dataframe input, Arrow C-data export or zero-copy guarantee. Dataframe/connection adapters are deferred, not secretly accepted through arbitrary callbacks.

Public memory results expose bounded iteration over copied batches and explicit `close`, plus context-manager support. Closing releases retained Rust batches, sources and worker state; access after close returns `HANDLE_CLOSED`. Returned Python values remain valid after close because they own their data. Handle finalization is a cleanup fallback, not a deterministic completion/cancellation API. Repeated close is harmless. No worker calls back into Python; host-only expressions/extensions are rejected as nonportable.

Execution detaches from Python while doing Rust work. The calling thread periodically reattaches to check signals during its bounded waits, translating KeyboardInterrupt to the common cancellation token. Explicit cancellation uses the same path. Dropping streams/resources follows executor cancellation, with no promise to instantly interrupt blocking XLSX parsing. Errors expose stable Iresiphi code/phase/context/report ID; Python raises an Iresiphi exception carrying that record, while user cancellation maps to a cancellation subtype or KeyboardInterrupt preserving the run report. Raw DataFusion/PyO3 exception text is diagnostic detail, not the stable interface.

Serialize/load uses the same versioned document and exact literal encoding in both Rust/Python. Values beyond 2^53, Int64 boundaries, decimal ties/scales up to38, null and date boundaries must round-trip unchanged. Future TypeScript must use exact tagged strings or equivalent lossless values, never JS Number for these domains. Rust/Python public expected-result parity, clean-wheel installation, Ctrl-C, explicit cancellation, repeated close, access-after-close and retained-value-after-close are implementation release gates. Version metadata is evidence of an available approach; a combined build has not yet been performed.


### Decision #13

Source: https://github.com/andrew3stedall/iresiphi/issues/13#issuecomment-6797720274

## Resolution — bounded admission, cooperative cancellation, commit-only outputs

Autonomous decision under the developer's authority. The default preview profile is one active run per execution context (additional attempts return `RUN_BUSY`), two execution partitions, batches of at most8192 rows, and at most two queued batches per producer. Pull-based backpressure bounds queues; adapters must also enforce byte limits because a row limit alone is insufficient.

Defaults: 256MiB participating-engine reservation budget; 64MiB per input file and 256MiB aggregate encoded input; 256MiB aggregate actual decompressed XLSX bytes; one million decoded cells per worksheet; 1MiB per field/token; JSON depth64; 100,000 invoice lines, one million component rows and 100,000 rows per lookup source; one million total qualified lookup-candidate pairs; 256MiB serialized output/evidence; 256MiB retained result batch buffers. Each batch also has a 16MiB decoded-buffer limit, with a single over-limit record rejected. Limits may be raised explicitly in policy and are recorded; they are admission choices, not measured capacity promises. The generated100,000-line fixture must be designed within the declared profile, and actual complete-recipe measurements gate release.

No spill ships. Stateful joins/aggregates either reserve within their tracked budget or fail with `RESOURCE_LIMIT`; they cannot silently collect elsewhere. Unknown source sizes require counting while reading, and exceeding a limit aborts. ZIP preflight checks and actual decompression/cell accounting precede unbounded workbook range allocation. Candidate pairs are capped during candidate production, before materializing a join explosion. Memory results must fit retained-buffer and serialized-size budgets before becoming successful; Python batch conversion copies only one requested batch at a time. User-retained Python values are outside library accounting.

These controls are not a hard process RSS ceiling: DataFusion reservations omit some allocations, adapters and runtime overhead exist, and Python owns copies. The manifest separates tracked reservations/buffers from measured peak RSS when available. Deployments requiring a hard RSS/kill deadline must use OS-isolated execution; no hidden subprocess contract is promised.

Cancellation is cooperative via one run token checked between parsing chunks, batches, output writes and explicit wrapper loops; cancel drops all executor streams and joins owned worker shutdown before reporting completion. Signal checks occur at most100ms apart while the Python caller is waiting, but blocking adapter/operator work may delay actual shutdown. No bounded cancellation latency is promised; measure and report it. Failure/cancellation publishes no successful partial result. Checks not completed remain marked incomplete.

Remote staging is deferred. Local scratch/output staging is run-owned under an approved root, with exclusive unpredictable names and owner-only permissions. No writes outside approved roots, no following untrusted path/symlink escapes, and no recursive deletion of caller-owned data. Success/failure/cancellation removes owned temporary artifacts; failed cleanup records exact leftover paths and errors in the run report. Process death can leave scratch; an explicit cleanup command may remove only recognizable run-owned artifacts after ensuring the run is inactive. No promise that crash cleanup happens automatically.

Directory output is one bundle containing primary/detail/evidence, schema and manifest. Write a sibling temporary directory on the same filesystem, finish/close files, and atomically rename to a NEW destination with no-replace semantics. Refuse an existing destination; no overwrite/merge. Support local filesystems providing that rename primitive; reject cross-device or unsupported publication. A complete-marker manifest is inside the renamed bundle. This is atomic visibility of the bundle, not an fsync-backed power-loss durability guarantee or a transaction with inputs. Cancellation/failure before commit aborts; after successful rename the run is committed and subsequent cancellation cannot revoke it. A post-commit reporting failure leaves a discoverable complete bundle and its run ID. Retry at the same destination returns `DESTINATION_EXISTS`; callers inspect its manifest, never duplicate/overwrite silently.

JSON outputs use schema-tagged exact scalar representations. CSV bundle encoding is UTF-8, comma delimiter, RFC4180 quoting, LF record endings and one header row. Null is the field text `\N`; every non-null text representation beginning with backslash gains one leading backslash before CSV quoting. On read, exact `\N` is null; otherwise remove one leading backslash only from a doubled-backslash prefix, then parse according to schema. Empty string remains empty; literal `\N` becomes `\\N`. This escaping applies to every non-null field representation and is specified in the companion schema; arbitrary spreadsheet readers are not claimed to decode it automatically.

Public failure tests force each budget, unknown-size overflow, cancellation at parse/join/write, cleanup failure, existing destination, process interruption before/after commit and repeated retry. Verify diagnostics and visible outputs, not private scheduler order. No snapshot consistency across independent sources is implied.


### Decision #14

Source: https://github.com/andrew3stedall/iresiphi/issues/14#issuecomment-6797736842

## Resolution — narrowly supported, evidence-gated developer preview

Autonomous decision under the developer's authority. Select Apache-2.0 for new Iresiphi project code and documentation. Existing vendored skills retain their own licenses/notices; this decision does not relicense third-party material. Implementation must add the project license/required notices and inspect the locked dependency distribution before publication. This is a licensing choice, not legal certification of an unbuilt dependency graph.

Release aligned Rust `iresiphi` crate and Python `iresiphi` distribution at initial preview version0.1.0, with one shared Rust semantic core and thin Python module. A Python source distribution includes the Rust sources/lockfile and documents the required build toolchain. Build separate ordinary CPython3.12/3.13 wheels for Linux x86_64 manylinux_2_28 (glibc>=2.28) using pinned build-image digest and maturin1.16.0; no target-cpu=native. Rust1.94.0 and the exact dependency pins selected by engine/binding decisions are the baseline. Audit native wheel dependencies and test minimum-glibc installation/import on clean images. No abi3, free-threaded Python, PyPy, macOS, Windows, ARM, browser or Node/TypeScript support claim in this release.

Recipe format0.1 is explicitly versioned independently of package patches, under the exact compatibility/migration policy in the portable-recipe decision. Saved recipes contain no live bindings; examples must show save, reopen, rebind, validate, explain and run. Publish the support/deferral matrix, resource profile, XLSX quantization/cache limits, CSV null escaping, snapshot limitations, unresolved comparison semantics and cleanup behavior beside the executable example. Broader connectors, remote/hybrid execution, dataframe access, TypeScript and allocation remain product requirements deferred beyond this preview.

Required release evidence: (1) locked Rust build and audited wheels in clean supported environments; (2) common Rust/Python public fixtures including the exact two-supplier160/145/15 comparison, total billed180 and unresolved20; (3) checked scalar oracle cases including38-digit precision, quantization ties/negative values, integer boundaries, overflow/null/date behavior; (4) real JSON/CSV/XLSX adapter tests including numeric lexemes, duplicate keys, CSV escaping, formula cache failures, workbook epochs, ZIP/range limits and source evidence; (5) identity/path/date-overlap/fanout/grain failures; (6) persistence/rebinding parity; (7) budgets, backpressure-visible limits, cancellation, handle ownership, scratch cleanup and atomic publication/retry cases; (8) complete100,000-line workflow measurements including source access, validation, conversion, planning, execution, evidence and delivery.

Record hardware, OS/compiler/runtime versions, immutable fixture seed/hash, input sizes, policy, actual row/evidence counts, elapsed phases, peak RSS where measured, tracked reservations, bytes read/written and output size. Report observations only. No faster-than-Python/database/Spark claim or universal latency/memory promise follows from Rust or these synthetic measurements. There is no fixed elapsed-time release threshold; correctness, completion under the declared test policy and truthful resource reporting are required.

Before publication recheck crate/PyPI names and account ownership, generate and review dependency/license notices, ensure no bundled secrets or unintended source data, and document installation limitations. Name availability is not reservation. These are ordinary release execution gates on artifacts that do not yet exist, not unresolved planning decisions or a need for external account access now. If a required name is unavailable, do not silently publish under a substitute; report the concrete conflict as a new naming decision.

The decision route is ready for specification synthesis and implementation tickets. Nothing here claims packages have been built, tests executed, licenses audited, performance measured or artifacts published. Production implementation and publication remain outside this map.


## Testing Decisions

Use public validate/explain/run tests, independent exact expected values and real adapter/executor integrations. The repository currently has no executable test prior art. Apply red-before-green per vertical slice. Test all normative error cases as well as the tiny fixture; test Rust/Python parity and persistence/rebinding; measure the full 100,000-line workflow; validate package builds achievable in this environment and record external gaps. Standards and spec reviews are separate axes. Close tickets only with actual evidence, never planning assertions.

## Out of Scope

No publication, deployment or main merge. TypeScript, remote/hybrid execution, SQL/Snowflake/Spark, dataframe inputs, allocation, arbitrary UDFs, currency conversion, tax engines, general temporal joins, unsupported operating systems/architectures and all other explicit deferrals above remain broader product requirements.

## Further Notes

Decision provenance: https://github.com/andrew3stedall/iresiphi/issues/2. Research #15 informs version checks but #11 controls the final semantic contract. Local implementation commits and evidence are required; unsupported external packaging environments must be reported honestly.
