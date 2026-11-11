# Sources and execution

## Product requirement

Recipes should compose data across formats, locations, and runtimes. Users may bring SQL or Snowflake data, in-memory dataframes, Spark dataframes, NDJSON, XML, Excel, CSV, text, Parquet, and additional sources through adapters. They should not have to standardise source columns, upload all inputs into one warehouse, or convert every input into a prescribed dataframe before describing relationships.

Any pair of supported source adapters should be composable through the same planning model. Building a connector for one source should not require implementing a separate integration with every other source. This requirement does not imply that every operation can execute at every source or that every input has an inferable record structure.

The following design is a proposal for meeting this requirement. No adapters, planner, or executors are implemented yet.

## Separate four concerns

1. **Recipe meaning.** Sources, record projections, relationships, transformations, populations, and output grain express what result is required.
2. **Source access.** Adapters bind those sources to queries, files, dataframe objects, or other inputs. Format, storage location, and host runtime are separate properties.
3. **Execution placement.** A planner assigns operations to available executors and identifies required transfers under an execution policy.
4. **Output delivery.** An output binding specifies a dataframe, file, database destination, or other sink. The destination does not determine where every preceding operation must run.

A CSV file may be local or remote. A Spark dataframe represents work in an execution environment, rather than simply a block of local memory. A database source may already be a filtered query. These distinctions belong in source bindings and capabilities, rather than changing the meaning of the recipe.

## Adapter and executor seams

A source adapter should expose the source's available structure, record projection, access methods, and supported operations. The planner needs to know whether a source can apply filters, select fields, transform expressions, aggregate, join another source in the same environment, or deliver batches. Capabilities must include semantic restrictions, rather than a single assertion that an operation is supported.

An executor should accept a supported part of the plan and report its results and execution evidence. Candidate roles include executing within a database, within Spark, or within a local Rust executor. One adapter may support remote execution as well as reading data into another executor.

The recipe and logical operation model should provide the common contract. Adapters can use different physical representations. A shared internal representation is an implementation choice and must not impose a new stored schema or preprocessing workflow on callers.

Format support also needs interpretation. XML requires a record path and namespaces where relevant. Excel requires sheet/range and header rules. NDJSON may contain heterogeneous nested records. Text requires an explicit extraction or matching rule before relational fields exist. Exploding a list or unpivoting columns changes grain and must remain visible in the recipe and lineage. An adapter can supply defaults, but should expose the interpretation it chose.

## Execution strategies

- **Execute at the source.** Push supported operations to the environment containing the data, including joins when the participating bindings are accessible there.
- **Ingest and execute.** Read the required data into an executor and apply the remaining operations there.
- **Combine both.** Filter, project, or aggregate remotely; transfer the necessary intermediate results; then complete cross-source work elsewhere.

These strategies should be available under a user-selected or automatic policy. Automatic planning is constrained by declared capabilities, data movement permissions, estimated size, and resource budgets. If execution is infeasible under that policy, return an explanation of the blocked operation or transfer. Offer an alternative plan without silently changing the policy.

Source-local query execution is not a universal solution for cross-source joins. A database cannot join a local dataframe unless the plan provides a way for the data to meet. Options include reading remote results locally, transferring selected data to another permitted executor, or using a permitted staging mechanism. Staging is a write and must be visible in the plan and covered by the run's policy.

The Rust core can own shared recipe semantics, validation, and planning while other executors perform remote work. The requirement for a Rust implementation does not require pulling every dataset into the Rust process.

## Plan from requested fields

Placement should follow the dependencies of the requested output fields. For each raw or transformed field, resolve its relationship path, required source fields, transformations, contributing population, and grain changes. Then consider where those operations can execute and what data must move. This makes loading external data into Snowflake a valid choice when it benefits the required calculation, alongside processing outside Snowflake or dividing the work across executors.

The relationship path establishes meaning; the execution plan chooses how to compute that meaning. Choosing between purchase region and current customer region is a semantic choice, not a cost optimization. Once that choice is fixed, moving its computation must preserve it.

Planning should consider the whole requested output. Several fields may share source scans or intermediate calculations. Other dependencies may not appear in the selected fields at all: join keys, filter inputs, and every participant in an allocation denominator still affect the result. Selecting one customer must not accidentally remove the other customers needed to calculate that customer's agreed share.

Compare feasible placements using available statistics, transfer volume, operation support, loading cost, resource budgets, and the execution policy. Estimates should identify uncertainty. A reusable or already loaded intermediate may affect later choices; its identity, freshness, and lifetime need to be explicit. Any Snowflake staging should identify its destination and cleanup policy rather than imply permanent ingestion.

An explainable plan should connect each requested field to its dependencies, calculation grain, execution locations, and transfers. Users can then see why one calculation runs in Snowflake while another runs elsewhere.

## Parallel work in flight

Independent source reads and transformations should overlap where the plan permits, using bounded batches, backpressure, cancellation, and explicit memory limits. This can improve batch processing without promising support for indefinitely running event streams.

Some operations need retained state or cannot emit complete results until sufficient input arrives. Joins, sorting, deduplication, and aggregation require operation-specific planning. A scalable design may partition work or spill to disk. It must not promise that arbitrary joins use constant memory or immediately emit final rows.

For Spark-sized or warehouse-sized inputs, an automatic fallback should not silently collect the entire source into local memory. The plan should expose estimated transfers and their uncertainty, then honor the configured execution policy and memory budget.

## Correctness across executors

Moving an operation must preserve its meaning. Key types, null equality, decimal precision, collation, timezone handling, filter timing, and supported functions need compatible semantics or an explicit conversion. A faster pushdown that changes results is invalid.

Logical lineage should describe the same recipe regardless of placement. Run evidence should additionally record where each operation ran, what was transferred, source identities, actual row counts, key/cardinality checks, and relevant executor versions. Independently changing sources may not offer a shared snapshot; report that limitation rather than imply a transaction across all bindings.

Saved recipes should retain source names and requirements. Runtime bindings supply connections, credentials, files, and live dataframe objects. Reopening a recipe in another language may require new bindings or an explicitly declared host-only operation.

## Example plans to explore

An engineer requests customer/month spend from Snowflake transactions, customer-region mappings in an Excel workbook, and weighted targets in NDJSON. One candidate plan filters and aggregates transactions in Snowflake, reads the workbook and NDJSON concurrently, validates their keys, then performs the remaining joins and allocation in a local executor. A policy allowing staging could enable a different plan. Neither should require rewriting the recipe's relationship meaning.

If the requested field instead requires joining the external mappings to detailed transactions before aggregation, loading the mappings into Snowflake may avoid exporting the detailed transaction dataset. If a required transformation is supported only outside Snowflake, another part of the same recipe may execute elsewhere. These are placement candidates whose benefit must be evaluated for the actual workload.

Another engineer relates a Spark dataframe to a CSV lookup. A candidate plan sends the small lookup to the Spark executor and leaves the large dataset there. If transfers to Spark are prohibited, planning should explain the restriction and available alternatives. Output may be delivered through a different supported sink after execution.

These are scenarios to design and evaluate, not tested integrations or fixed routing rules.

## Next design work

Choose two contrasting adapters and a cross-source join that require real variation at the adapter seam. Specify the shared logical operations, capability descriptions, and execution policy from that example before defining a large connector interface. Use the same recipe to compare ingestion and hybrid placement, including a case where policy prevents an otherwise valid plan.

Keep the broad format/location requirement while implementing connectors incrementally. Source support, operation support, and execution placement should be reported separately so callers can see exactly what a release supports.
