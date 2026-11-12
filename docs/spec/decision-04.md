# Decision history 4

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). Choose the preview support matrix against the confirmed invoice-review acceptance contract, including its concrete input variation and explicit distinction between required workflow inputs and deferred broader capabilities.

## Question

For the agreed representative workflow, which language interfaces, host runtimes, source binding families, execution environments, and output sinks must the developer preview actually support?

Distinguish language and runtime (for example, TypeScript does not itself select Node.js or browser execution), source format from location, and remote operation support from mere source ingestion. Select enough contrasting inputs to exercise common composition contracts rather than a single hard-coded integration.

State separately whether the preview demonstrates local execution, source pushdown, hybrid placement, staging, and user-selected versus automatic placement. These are release-scope choices; the concrete local execution engine and binding technologies are selected in later decisions.

For each broader product capability, record included, deferred, or intentionally unsupported under this release, with rationale and extension requirements. Preserve all broader requirements instead of silently redefining Iresiphi around the chosen preview adapters.


https://github.com/andrew3stedall/iresiphi/issues/4#issuecomment-6797517465

## Resolution — bounded local invoice-review preview

Decided autonomously under the developer's explicit authorization to resolve the whole map; this is a planning decision, not a claim of implemented support.

Ship a shared Rust library and Python interface on CPython 3.12/3.13, initially Linux x86_64. Inputs: local UTF-8 JSON invoice documents with explicit header/array projection; local XLSX worksheets with declared sheet, header row and typed columns; UTF-8 CSV lookup, rate and component tables with explicit dialect/schema. No inferred business mappings or schema coercion. XLSX formulas require stored cached values; missing caches fail, and the preview does not calculate formulas. Preserve original cells/paths and source identities for review.

All sources enter a single local executor through the same typed batch contract. Ship explicit local placement only, with no network reads, pushdown, remote staging, automatic transfer, or hybrid execution. Caller supplies permitted local roots and resource limits. Local files with different formats demonstrate cross-source composition without requiring database credentials. Output: a typed in-memory result and a local directory bundle of JSON manifest plus CSV line-review, linked-component, and reconciliation tables. Exact delivery semantics are settled by the resource/failure decision.

Defer TypeScript/Node/browser distribution, macOS/Windows wheels, dataframes, SQL, Snowflake, Spark, NDJSON, XML, arbitrary text, Parquet and remote stores. These remain product requirements: source adapters must report schema, identity, capabilities and location; executors must advertise semantic compatibility and access; sinks are independent of input. Unsupported bindings/operations fail explicitly, never silently collect data or substitute an executor. No pairwise connector-specific relationships.

The invoice acceptance workflow ships with explicit supplier-scoped mapping and rates, observed component detail, exact currency arithmetic, aggregation and safe broadcast. Allocation, arbitrary UDFs, currency conversion, tax engines and general temporal joins are deferred. Basic invoice-date rate validity filtering remains required. This bounds the preview without deleting broader functionality from the product specification.

Acceptance remains the already agreed two-supplier fixture and 100,000-line generated workload. This decision sets no speed claim or guaranteed platform beyond those above.
