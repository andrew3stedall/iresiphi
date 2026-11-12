# Decision history 9

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). The confirmed invoice-review acceptance contract establishes a targeted source-record/lookup reconciliation requirement and identifiable unresolved comparisons. Resolve their concrete public representation without conflating this requirement with exhaustive arbitrary-operation provenance.

## Question

What externally observable behavior should callers receive from the preview's validate, explain, and run workflow?

Define the lifecycle and information available before binding, after binding, and after execution. Distinguish static recipe validation, schema compatibility, and actual key/cardinality/data checks; choose coverage/defaults, error versus warning behavior, diagnostic structure, useful failing examples, and affected-output reporting.

Define logical explanation and lineage fields, feasible-placement explanations, unsupported-operation errors, and run evidence including source identities, actual counts, transfers, check coverage, and reproducibility limits. State which records are available after failed or cancelled runs without implying that absent evidence proves correctness.

Describe host-language-neutral results and the common acceptance cases used by Rust and supported language interfaces. Preserve validate/explain/run as the principal test boundary and identify real integrations needed beyond fixtures. Keep full row-contribution tracing deferred unless evidence justifies a separate scope decision.


https://github.com/andrew3stedall/iresiphi/issues/9#issuecomment-6797608785

## Resolution — explicit validation coverage and inspectable run evidence

Autonomous decision under the developer's authorization. The shared public operations return language-neutral structured records; Python wraps the same core results. Names below define contracts, not implemented APIs.

`validate(recipe)` performs version/reference/type/path/grain/capability validation without opening sources. `validate(recipe, bindings, policy)` additionally inspects source schemas and access/capability feasibility, with `data_checks="none"` by default. Explicit `data_checks="full"` scans the selected source populations for identity, types, relationships and rate validity. There is no sampled mode in the preview. Reports list each check as passed/failed/not-run and its population/coverage; static validity never implies data correctness. All actual runs perform full required checks again against their own read materialization: earlier validation is evidence, not permission to skip checks on changed data.

`explain` exposes logical nodes, named source/field/path dependencies, population and filter stages, grain transitions and measure origins. With bindings/policy it adds feasible local placement, reads/transfers, estimates including unknown values, unsupported capability reasons and budgets. It never executes full data checks implicitly or claims a cost optimum. Explanation and logical lineage use stable declaration IDs rather than unstable engine internals.

`run` returns a run ID, terminal status (`succeeded`, `succeeded_with_unresolved`, `failed`, `cancelled`), output descriptors only for committed outputs, and a manifest/report. The invoice review's unmatched/ambiguous lookup lines yield `succeeded_with_unresolved` and retain one primary row with null dependent values and explicit reason. Missing/duplicate anchor identity, invalid typed data, overflow, policy violations and strict relationship violations fail publication. An empty selected population succeeds with zero counts and no completeness claim about excluded inputs.

Diagnostics include stable code, severity, phase, source/projection or expression/path ID, check ID, affected-output IDs, exact affected-count when known, bounded source-reference samples, and human message/remediation. Values are omitted by default to avoid leaking data; explicit diagnostic-value opt-in is recorded. The default sample cap is 20 references per diagnostic, with truncation and count status explicit. Per-line status and an evidence table preserve the targeted invoice reconciliation beyond these bounded diagnostic samples.

The invoice evidence outputs include source observation reference and original projected values for each line, matched translation/enrichment/rate source references and relevant original values, candidate counts and ambiguity references, and component parent links. Candidate evidence is materialized as a separate table and subject to explicit output/resource bounds; exceeding a bound fails rather than silently claiming complete evidence. It does not promise complete row-contribution provenance for arbitrary aggregate expressions. Logical lineage still identifies all contributing source fields and operations.

Run manifests include recipe hash/format/package versions, safe binding identities and content fingerprints, declared policy, executed nodes, actual source/eligible/output row counts, resolved/unresolved counts and amounts by currency, bytes read/written where measurable, unknown-measurement markers, checks/coverage, timings, and committed output/evidence locations. A source fingerprint refers to bytes actually read or the immutable materialization, not a later reread. There is no shared multi-file snapshot guarantee; concurrent changes detected during access fail or are reported as unreproducible, never silently certified.

Failed/cancelled runs retain bounded diagnostics and a manifest with incomplete checks/counts explicitly marked, plus cleanup evidence. They expose no committed result as successful. The public acceptance suite must cover the agreed tiny invoice fixture, corrupted identities/keys/intervals, unresolved preservation, fanout prevention, save/rebind/reopen parity, and budget/cancellation/sink failures. Real JSON/CSV/XLSX reads and actual output publication are required beyond fixture-only tests. The 100,000-line workload measures complete access-through-delivery behavior; it does not establish a performance guarantee in advance.
