# Decision history 3

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

## Question

Which recurring data-integration workflow should anchor the developer preview, and what observable result makes that preview useful?

Decide the intended user, recurring task, representative input shapes and approximate scale, requested output population and grain, and the correctness/debugging problem the preview must demonstrate. Establish business assumptions separately from facts the library can validate.

Identify the manually reviewable expected results, failure examples, and completion criteria through validate/explain/run. The existing customer/month sales-and-target allocation example is a candidate fixture, not an already selected real workflow. Decide whether it anchors the preview, supplements another workflow, or needs replacement.

Identify the minimal cross-source variation the acceptance workflow must exercise, while leaving the exact release support matrix to its own decision. Record data/access constraints already known and surface precise evidence or preparation tickets where needed. The answer must let downstream decisions evaluate their choices against a concrete workflow.


https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6795437819

## Progress checkpoint — decision still open

Confirmed with the developer during the live interview:

- Replace the customer/month example as the anchor with an invoice-review workflow. Public synthetic fixtures are sufficient for initial preview acceptance; they should represent this real workflow's structure.
- Inputs described: AI-extracted invoice JSON with header and line detail; separate Excel enrichment for invoice lines; translation-table mappings from invoice line values to contracted items/rate cards; and existing source records providing finer component detail beneath invoice lines.
- The purpose is an auditable review of billed versus contracted charges with supporting enrichment and component detail.
- Analysts currently standardize data and perform lookups to assemble the review output. That standardization makes it difficult to reconcile invoice/rate-card comparisons back to raw data. Maintaining inspectable original values and lookup relationships is the central workflow need.
- Component detail is observed source data. No allocation requirement has been established for this workflow.

The interview is continuing. Current questions concern the primary output grain, the representative pricing calculation, and the minimum raw-source/lookup evidence reviewers must inspect. Population, concrete matching examples, expected/failing cases, representative scale, recurring cadence, and final acceptance criteria still need to be established as relevant.

No resolution is recorded, and the issue remains claimed and open. Detailed relationship, numeric, API, adapter, engine, and release-support decisions remain in their existing map tickets.

https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135

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
