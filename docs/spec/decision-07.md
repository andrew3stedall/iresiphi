# Decision history 7

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). Use the confirmed invoice-review acceptance contract to decide the required grain operations. Source-supplied component detail does not by itself require allocation; evaluate preview allocation coverage explicitly while preserving the broader product requirement.

## Question

What are the exact supported grain-transition operations and correctness rules for the developer preview?

For aggregation, define measure ownership, allowed grouping grain, filter timing, and how independent fact collections avoid multiplication. Decide the initial aggregate subset and the treatment of unsupported nonadditive or semi-additive measures.

For broadcast, define how originating grain and repeated context remain visible and what happens if a caller requests an invalid re-aggregation.

For allocation, define source partitions, eligible recipient grain, denominator population, weights and normalization, precision/rounding, residuals, missing targets, zero or absent total weights, and partitions with no recipients. State which choices are fixed defaults and which require explicit recipe policy.

Test the intended meanings with repeated transaction rows, unequal shares, output filtering after allocation, and recipient filtering before normalization. Conservation and correct recipient shares are separate obligations. Record examples and diagnostics that can become public-workflow acceptance tests.


https://github.com/andrew3stedall/iresiphi/issues/7#issuecomment-6797569343

## Resolution — owned measures and explicit grain transitions

Autonomous decision under delegated authority. Preview supports count-records, count-non-null, sum, min, max at explicitly declared grouping keys. Average, distinct-count, windows, semi-additive measures, custom aggregates and allocation are deferred, with `UNSUPPORTED_OPERATION` at validation. Allocation weights, denominator normalization, residual rules and conservation are therefore not silently defaulted; a later allocation effort must decide them together. Output filtering cannot be represented as an allocation surrogate.

Every measure has its originating grain. Aggregate each independent fact collection at the requested shared grain before combining it; never join independent detail collections and aggregate the expanded result. Grouping must retain required semantic dimensions such as currency. Duplicate observations remain separate only when they have distinct declared identities; duplicate identity is an error, not an invitation to DISTINCT the amounts. Billed 10 on two distinct lines sums to 20, not 10. Three components under one billed-110 line do not change the billed sum from 110.

Aggregate input predicates apply before grouping. Final output predicates apply after grouping and do not change the group population. Absent anchor groups are not manufactured. For an existing anchor with zero related observations, count is zero and sum/min/max are null. For all-null observations, sum/min/max are null and count-non-null is zero. Numeric overflow fails the run. Decimal sum retains input scale and accumulates checked decimal128 precision 38; further arithmetic whose inferred precision exceeds 38 requires an explicit checked narrowing cast justified by a declared range. No silently lossy reduction.

Broadcast is permitted only along a validated to-one path and carries originating-grain metadata. The repeated value is context, not a newly additive measure at recipient grain. Re-aggregating a broadcast measure at recipient grain is rejected; aggregate from its original source or request a named min/max context reduction whose equality is validated. A customer-level budget repeated on ten lines is not ten budgets. Nested component scalar selection remains invalid without explicit aggregation or the separate detail output.

The invoice primary output preserves every line. Comparable totals explicitly select resolved lines: billed 160, expected 145, variance 15. Full-population billed is 180; unresolved billed is 20; total and resolved/unresolved counts accompany the comparison summary. A sum ignoring null expected values must never be labelled complete reconciliation. Observed component totals are separate measures; no equality with billed charge is presumed.

Acceptance adds unequal component counts on lines with equal billed values, two independent detail collections, empty detail sets, all-null values, currency mixing, a forbidden broadcast sum, and input-filter versus final-filter examples. Unsupported allocation fixtures must fail validation, not produce invented shares. These decisions preserve allocation as a broader requirement without making it a preview prerequisite.
