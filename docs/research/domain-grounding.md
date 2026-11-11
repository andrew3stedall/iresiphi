# Domain grounding for Iresiphi

Research date and source access date: 11 November 2026. This note grounds the direction in [README.md](../../README.md) and [DESIGN.md](../../DESIGN.md). It is research for further design, not an API specification or an engine selection. Iresiphi is planned, so all proposed behavior below is a hypothesis.

## Main finding and evidence limits

Iresiphi sits closest to a grain-aware semantic query planner packaged as a library. Its proposed combination is useful to investigate: preserve source names, describe relationships, request an output grain, and distinguish rollup, broadcast, and allocation. Those individual ideas have substantial precedent. MetricFlow already plans metric queries through entity relationships, Malloy models aggregate locality, and Featuretools computes relational features for a target dataframe. The opportunity is a coherent engineering workflow with explicit grain changes and explanations, rather than a claim to have invented relational feature computation. [MetricFlow overview](https://docs.getdbt.com/docs/build/about-metricflow), [Malloy aggregates](https://docs.malloydata.dev/documentation/language/aggregates), [Featuretools deep feature synthesis](https://docs.featuretools.com/en/stable/getting_started/afe.html).

Here, "works well" means a documented mechanism addresses a specific problem. The sources inspected do not establish comparative adoption, user satisfaction, or performance for Iresiphi's intended workload. No empirical user evidence was found in this research. Documentation examples demonstrate behavior, not representative production outcomes. Recommendations below are analysis based on those mechanisms, with no benchmarks performed.

The references are primary project documentation, a first-party modeling reference, and an author-hosted technical research survey. Version context matters. Featuretools pages identify version 1.31.0; PostgreSQL links explicitly target 18; DuckDB's stable redirect resolved to 2.0 LTS and its client table listed 2.0.0. Arrow's documentation identified library version 26.0.1 and columnar format version 1.5. OpenLineage identified 1.55.0. The dbt pages include changes marked for v1.12 and later. Other references are rolling documentation snapshots accessed on the date above, including DataFusion's `main` guide and PyO3's `main` guide. No repository commit or installed dependency version was selected. Recheck and pin these before depending on exact APIs or packaging support.

## Where the concept belongs

Schema-on-read postpones interpretation until data is read. Data virtualization exposes sources through a query interface without requiring every source to be copied into one store. PostgreSQL foreign tables illustrate this separation: they describe externally stored data, while foreign data wrappers fetch it for queries. Iresiphi could use similar late binding without becoming a federation service. Preserving schemas is compatible with projecting temporary relational views inside a plan. [PostgreSQL 18 foreign data](https://www.postgresql.org/docs/18/ddl-foreign-data.html).

Preserving raw names does not remove semantic integration. `customers.CustNo` and `transactions.buyer_ref` can identify the same entity despite different names; two fields named `amount` can represent different currencies or tax treatments. A recipe must declare equivalence, conversion, identity scope, and time meaning. This follows the same distinction that MetricFlow makes between physical expressions and entities used to connect semantic models. The analysis for Iresiphi is to avoid compulsory physical standardization while retaining explicit semantic mappings. [MetricFlow join logic](https://docs.getdbt.com/docs/build/join-logic).

Relational execution already supplies selection, joins, grouping, and expressions. SQL gives these operations a defined logical order; an optimizer can choose a physical plan that preserves the result. Iresiphi's additional job would be to decide which result is intended when sources represent different grains. An execution engine cannot infer whether a regional target is customer context or money assigned to that customer. [PostgreSQL 18 table expressions](https://www.postgresql.org/docs/18/queries-table-expressions.html).

Semantic and metrics layers attach business meaning and aggregation rules to data. Feature engineering overlaps when the output is a customer, account, or event feature table. Feature stores add operational concerns such as historical retrieval and online serving. Lineage describes dependencies and derivations across all of these activities. Iresiphi should establish which of these jobs it owns before expanding into serving infrastructure. Feast explicitly separates offline extraction from online feature serving. [Feast introduction](https://docs.feast.dev/).

## Closest designs and what to learn

The table compares mechanisms, not product quality. The final column is analysis for Iresiphi, not a statement that the other project cannot be extended.

| System | Documented mechanism | Tradeoff and lesson for Iresiphi |
| --- | --- | --- |
| dbt MetricFlow | Entity graph and metric-driven SQL generation | Strong precedent for constrained relationship traversal; broader transformations and allocation need their own semantics. |
| Cube | Relationship declarations and primary-key-based deduplication for multiplied measures | Protecting aggregates depends on correct identity declarations; deduplication does not distribute a total. |
| Malloy | Relationship graph and explicit aggregate locality | Closest conceptual precedent for reasoning about measures without flattening their meaning. |
| Ibis | Expressions separated from backend execution, including unbound tables | Useful precedent for a portable recipe; backend support still differs. |
| Featuretools | EntitySets, transformation and aggregation primitives, target feature matrices | Close to heterogeneous relational feature construction; automated feature discovery is a separate product choice. |
| Feast | Feature definitions, historical point-in-time retrieval, online serving | Strong temporal lessons; operating a feature store exceeds the proposed library's initial scope. |

### MetricFlow

MetricFlow constructs SQL from semantic models and metrics. Its overview describes entities, dimensions, and metric definitions, including simple, ratio, derived, cumulative, and conversion metrics. This gives reusable definitions a query-time interpretation instead of requiring each consumer to repeat SQL. [MetricFlow overview](https://docs.getdbt.com/docs/build/about-metricflow).

Its join rules are intentionally restrictive. The documented entity-type matrix permits foreign-to-primary and foreign-to-unique joins, while rejecting primary-to-foreign and foreign-to-foreign fanout paths. The guide describes left joins for fact-to-dimension traversal and full outer joins for combining multiple facts. Semantic validation includes referenced columns, supported functions, and ambiguous paths. This is a concrete correctness strategy: limit the relationships the planner will traverse. [MetricFlow join logic](https://docs.getdbt.com/docs/build/join-logic).

The lesson is to make invalid requests explainable. A rejected path can be better than a plausible number with multiplied contributions. The tradeoff is expressiveness. Iresiphi's explicit allocation is a different operation from a safe metric join and needs an explicit recipient population and weight rule. A declaration alone also cannot prove that the input still satisfies its cardinality contract.

### Cube

Cube declares one-to-one, one-to-many, and many-to-one relationships. For multiplied measures, its documented strategy selects distinct primary keys for the relevant cube, rejoins those keys to the cube, and computes the aggregate. With multiple multiplied measures, it computes corresponding results separately and joins them. Its modeling joins default to left joins, with documented limits on directly modeling right and full outer joins. [Cube joins](https://docs.cube.dev/reference/data-modeling/joins).

This makes row identity part of correctness. Deduplicating by the measure's numeric value would collapse two legitimate orders with identical amounts; deduplicating identities addresses a different problem. For Iresiphi, the lesson is to retain measure ownership and identity through planning. The tradeoff is trust in those identities and extra query structure. Cube's mechanism protects a regional measure from accidental multiplication, but it does not establish how much of it each customer should receive.

### Malloy

Malloy keeps relationships as a graph and distinguishes `join_one`, `join_many`, and cross relationships. Joined fields remain addressable through named paths. This is relevant to source preservation because relationship context survives instead of disappearing into a flat column collection. [Malloy joins](https://docs.malloydata.dev/documentation/language/join.html).

Its aggregate locality lets an expression specify the source whose rows contribute to an aggregate. Averaging aircraft-model seats over models and averaging them over aircraft represent different populations. The guide distinguishes these expressions and rejects certain asymmetric aggregates across many-side paths. This is a stronger precedent than merely storing a table's nominal grain. A measure needs a population and a location in the relationship graph. [Malloy aggregates](https://docs.malloydata.dev/documentation/language/aggregates).

Iresiphi should study this before designing a grain inference system. The recommendation is to explain the contributing population for every aggregate. Correct locality still cannot decide a business allocation policy, and Iresiphi should avoid promising "automatic correct joins" without the declarations and validations that make that statement meaningful.

### Ibis

Ibis provides unbound tables with declared schemas, so transformations can exist before a connection or actual source is supplied. Its examples execute expressions after binding appropriate data. The support matrix explicitly warns that operation support varies by backend and SQL dialect. [Ibis unbound expressions](https://ibis-project.org/how-to/extending/unbound_expression), [Ibis operation support](https://ibis-project.org/backends/support/matrix).

The architectural lesson is to separate recipe meaning from data handles. A dataframe expression API alone does not supply Iresiphi's desired contracts about grain, allocation conservation, or explainable relationship selection. Conversely, wrapping ordinary joins in a new API would offer little beyond established expression systems. Portability should mean a declared common semantic subset, with clear rejection of unsupported operations.

### Featuretools

An EntitySet stores dataframes and relationships. The official tutorial defines relationships through parent and child columns and can normalize an existing dataframe into related tables. Deep Feature Synthesis composes aggregation and transformation primitives to produce a feature matrix for a target dataframe. This is direct precedent for deriving outputs across heterogeneous relational inputs. [Featuretools EntitySets](https://docs.featuretools.com/en/stable/getting_started/using_entitysets.html), [Deep Feature Synthesis](https://docs.featuretools.com/en/stable/getting_started/afe.html).

Featuretools also models time indexes and cutoff times so feature calculations exclude data after the cutoff. The guide defines the time index in terms of when data became known. This is a useful warning against treating every timestamp as interchangeable. [Featuretools handling time](https://docs.featuretools.com/en/stable/getting_started/handling_time.html).

For Iresiphi, explicitly requested transformations may be a narrower starting point than automated feature generation. A customer/month output also requires deciding which customer/month rows exist. An entity target does not by itself answer calendar expansion, target allocation, or the distinction between missing activity and missing observation.

### Feast

Feast's offline and online stores address historical training data and feature serving. Its historical retrieval joins feature values onto timestamped entity rows. The point-in-time guide describes looking backward from each entity timestamp within a TTL, rather than applying TTL relative to execution time. [Feast introduction](https://docs.feast.dev/), [Feast point-in-time joins](https://docs.feast.dev/getting-started/concepts/point-in-time-joins).

A consequential detail in the accessed guide is that event-time filtering alone can return later backfills. It documents `filter_by_created_timestamp=True` to constrain availability time too. The guide notes that not all offline stores support this flag and that enabling it excludes rows with null created timestamps. This is a current-document capability, not a promise for an unspecified Feast release. Iresiphi should distinguish "valid at time t" from "known by time t" before offering leakage-safe outputs. [Feast point-in-time joins](https://docs.feast.dev/getting-started/concepts/point-in-time-joins).

The lesson is to preserve temporal meaning in the plan. The recommendation is to defer online serving and historical reconstruction claims until a real workflow establishes the necessary timestamps and source histories.

## Correctness starts before execution

The following are design implications and manually reasoned failure cases. SQL join semantics, key constraints, and the systems above supply the underlying mechanisms; the proposed Iresiphi responses are recommendations.

### Grain, identity, and relationship paths

"One row per customer per month" is a semantic claim. A surrogate row ID only identifies records; it does not prove uniqueness of customer/month. The relevant candidate key may be composite. It may also require an account namespace or a historical validity interval. PostgreSQL distinguishes primary keys from nullable unique constraints, illustrating why the word "unique" needs precise null behavior. [PostgreSQL 18 constraints](https://www.postgresql.org/docs/18/ddl-constraints.html).

Iresiphi should record expected grain, candidate keys, actual uniqueness checks, null-key policy, and relationship cardinality separately. Unknown keys should not all become one customer because an adapter applies a null-equality rule. A duplicated customer snapshot should fail a many-to-one enrichment check rather than silently double sales. Sampled checks can discover violations, but cannot certify a full source.

One-to-many joins repeat parent values. Joining two independent child collections through a parent creates combinations of their rows. Two transactions and three support tickets can become six records; summing transaction amounts then counts each transaction three times. Declaring "many-to-many" describes the relationship but does not resolve the requested measure. Safe choices include separate aggregation, an existence filter, or an explicit bridge with contribution weights. The basic row multiplication follows SQL's join definition. [PostgreSQL 18 table expressions](https://www.postgresql.org/docs/18/queries-table-expressions.html).

Multiple valid paths can mean different things. A transaction's purchase region and its customer's current region are distinct dimensions. Iresiphi should require a named path when meaning differs; a shortest-path algorithm is insufficient. MetricFlow's documented ambiguity checks provide a precedent for treating path ambiguity as a validation concern. [MetricFlow join logic](https://docs.getdbt.com/docs/build/join-logic).

### Measures and aggregation timing

Additive measures can be summed across their associated dimensions. Semi-additive measures, such as balances, cannot be summed over every dimension, especially time. Ratios are nonadditive; Kimball recommends aggregating their additive components before calculating the result. [Kimball measure categories](https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/additive-semi-additive-non-additive-fact/).

For Iresiphi, this suggests carrying sufficient aggregate state. Means need sum and count; weighted means need sum of weighted values and sum of weights. A conversion rate needs qualifying conversions and its eligible population. Distinct counts need identities or a declared mergeable approximation, since the same customer can occur in several groups. Two means of 10 and 100 based on 100 and 1 observations combine to 1100/101, approximately 10.891, not 55. A sum-distinct repair is wrong when equal values belong to different records.

Aggregate each independent fact stream to the necessary common grain before joining when that preserves the requested semantics. This avoids manufacturing cross-products, but is not an unconditional rewrite. A transaction-level eligibility filter must run before its aggregate. A threshold on total monthly spend must run afterward. Filtering a joined child in `WHERE` can also remove unmatched parent rows, while filtering the right side of a left join has different behavior. SQL documents these ordering distinctions. [PostgreSQL 18 table expressions](https://www.postgresql.org/docs/18/queries-table-expressions.html).

### Broadcast, allocation, and fabricated detail

A broadcast repeats context. An allocation distributes a quantity using a stated rule. Repeating a 900-unit target on two customer rows does not give each customer a 900-unit assignment. Iresiphi should attach the originating grain to broadcast measures and reject a later sum that treats repetitions as independent contributions unless the user explicitly requests that interpretation.

An allocation needs eligible recipients, partition keys, weight normalization, rounding, and a policy for zero or absent total weights. Conservation must state its domain: assigned amounts plus any unallocated residual equal the source amount within each source partition. Conservation alone does not establish fairness or business validity.

Requesting customer/month rows does not establish that every customer existed in every month. Calendar expansion needs explicit membership dates and an observation policy. Zero transactions can mean observed inactivity; a missing feed is unknown. Assigning a monthly target to days or customers creates modeled detail, not observed facts. The result should preserve that distinction and expose allocation rules in lineage.

### Shape, drift, time, and units

Preserving nested and wide sources still requires shape operations. Exploding a nested list changes grain; exploding two independent lists can create a product. Unpivoting monthly columns creates a month dimension and requires a parsing rule. These should be named operations whose source paths remain traceable. Polars' lazy schema guide shows a practical constraint: an optimizer needs schemas at intermediate stages, and data-dependent output schemas can prevent lazy planning. [Polars lazy schemas](https://docs.pola.rs/user-guide/lazy/schemas/).

Recommended drift policy should distinguish an extra unused column, a missing required path, an incompatible type, and a changed key distribution. Preserving names does not justify silently interpreting a renamed column as equivalent. Store mapping versions and fail with the missing dependency and affected outputs.

Temporal enrichment needs effective intervals or an as-of direction, boundary inclusivity, tie handling, and an allowed staleness. Availability time matters for training outputs; later corrections may otherwise leak into historical features. Units also belong to semantics: AUD and USD cannot be summed as money without a conversion rule and rate date. Month boundaries require a business timezone, especially around daylight-saving changes. A timestamp's physical representation alone does not specify the intended business calendar. Arrow distinguishes physical layouts from semantic data types, including timestamp and decimal types. [Feast temporal retrieval](https://docs.feast.dev/getting-started/concepts/point-in-time-joins), [Arrow columnar format](https://arrow.apache.org/docs/format/Columnar.html).

## Worked example with source names unchanged

Everything in this section is invented and illustrative. The rows and expected values were reasoned through manually. No code was executed and no performance was measured. Names below are the raw source names, not a proposed standardized schema.

`customers`, one current row per `CustNo`:

| CustNo | SalesArea | Share |
| --- | --- | ---: |
| A | North | 1 |
| B | North | 3 |
| C | South | 1 |

`transactions`, one row per `receipt_id`:

| receipt_id | buyer_ref | booked_at | gross |
| --- | --- | --- | ---: |
| t1 | A | 2026-01-05 | 60 |
| t2 | A | 2026-01-20 | 40 |
| t3 | B | 2026-01-11 | 200 |
| t4 | C | 2026-01-17 | 50 |
| t5 | B | 2026-02-08 | 120 |

`regional_monthly_targets`, one row per `Territory` and `Period`:

| Territory | Period | Goal |
| --- | --- | ---: |
| North | 2026-01 | 900 |
| South | 2026-01 | 300 |
| North | 2026-02 | 800 |
| South | 2026-02 | 400 |

Declare that `buyer_ref` refers to `CustNo`, and `SalesArea` matches `Territory`. Parse `booked_at` as a local business date and derive a month compatible with `Period`. All monetary values are AUD. Assume all three customers are eligible in both months, the transaction feed is complete, `Share` is fixed over this interval, and missing activity therefore means zero. These are necessary assumptions, not conclusions inferred from column names.

Request one output row for each eligible customer/month, January through February. Aggregate `gross` and transaction count by `buyer_ref` and derived month. Broadcast `Goal` as regional context. Separately allocate each region/month's `Goal` in proportion to eligible customers' `Share`.

| CustNo | month | sales | transaction_count | regional_goal_context | allocated_goal |
| --- | --- | ---: | ---: | ---: | ---: |
| A | 2026-01 | 100 | 2 | 900 | 225 |
| B | 2026-01 | 200 | 1 | 900 | 675 |
| C | 2026-01 | 50 | 1 | 300 | 300 |
| A | 2026-02 | 0 | 0 | 800 | 200 |
| B | 2026-02 | 120 | 1 | 800 | 600 |
| C | 2026-02 | 0 | 0 | 400 | 400 |

North's normalized weights are 1/4 and 3/4. January allocations conserve 900 because 225 + 675 = 900; February conserves 800 because 200 + 600 = 800. South has one eligible customer, so it conserves 300 and 400. Overall sales are 470 and allocated goals total 2400. The six repeated context values total 4100, which is not a meaningful regional target total.

The duplication trap appears even before calendar expansion. Joining January transactions to customers and then targets creates three North rows carrying `Goal=900`, so a naive target sum reports 2700 instead of 900. South contributes 300, producing a false January total of 3000 instead of 1200. Summing `gross` remains correct in this particular join because both enrichments are many-to-one; that does not make summing `Goal` correct.

Allocating on those raw transaction rows also fails. A's `Share` appears twice, so the mistaken denominator becomes 1 + 1 + 3 = 5. A receives 360 across its two transactions and B receives 540, instead of 225 and 675. This wrong allocation still conserves 900. The recipient grain matters as much as conservation.

A safe conceptual plan validates source keys, builds the eligible customer/month population, aggregates transactions to that grain, left joins activity, then joins the unique region/month target and applies allocation over eligible customers. The customer's identity and region remain available for each stage. Only absent activity is zero-filled under the stated completeness assumption; missing targets remain an error or an explicitly unknown value.

Filtering to A after allocation leaves A's January assignment at 225. Filtering the recipient population to A before normalization assigns A all 900. Both calculations are possible, but the plan must say which population defines the denominator. If a region has no eligible customers, retain its unallocated residual or fail explicitly. Do not invent recipients to make the conservation check pass.

## Provenance and debugging

Several meanings of provenance need separate promises. Database provenance research distinguishes why an output exists, how inputs combine, and where values originate. The author-hosted survey also describes eager and lazy approaches to collecting provenance. A dependency graph alone cannot answer every row-level question. [Cheney, Chiticariu, and Tan, provenance survey, DOI 10.1561/1900000006](https://homepages.inf.ed.ac.uk/jcheney/publications/provdbsurvey.pdf).

Iresiphi could offer progressively stronger explanations:

1. A logical explanation records source fields, expressions, relationship paths, output grain, and measure ownership. It can exist before execution.
2. A run explanation adds bound source identities, actual row counts, key checks, unmatched counts, selected backend plan, and allocation totals.
3. Row contribution tracing identifies the input records and coefficients behind a selected output. In the example, A's January sales depend on t1 and t2; its allocation depends on the North target and both eligible North customers' weights.

An allocation explanation must include denominator dependencies, not just the target row and recipient. Explaining a zero-filled result needs evidence of the output population and the missing match. Full contribution sets can be large. A useful initial hypothesis is logical lineage plus diagnostics for every run, with on-demand tracing for selected outputs when source snapshots can be reopened. Label sampled traces as samples.

OpenLineage's column lineage facet represents which input fields contribute to output fields and describes transformations. It is relevant for exporting dependencies to existing tooling. Its documented field-level representation is not a complete mapping from output records to physical input records with allocation coefficients. Exporting it should not be advertised as full row provenance. [OpenLineage 1.55.0 column lineage facet](https://openlineage.io/docs/spec/facets/dataset-facets/column_lineage_facet/).

Recommended diagnostics include expected versus observed grain, duplicate key examples, null keys, match multiplicity, unmatched left and right keys, row counts before and after joins, and allocation residuals per partition. Record whether a check scanned all data. A message such as "customer key A matched two customer rows; this would duplicate t1 and t2" is more actionable than "invalid join".

Reproduction needs identities beyond filenames. Record recipe and mapping versions, engine version and options, source snapshot IDs or content digests, query parameters, timezone, and callback versions. A database table name plus a timestamp is insufficient unless the source can restore that snapshot. A checksum of a recipe does not freeze live data. Physical row IDs should be scoped to source versions; row offsets may change after file rewriting.

## Rust, Python, and TypeScript implications

### Separate recipe meaning from execution state

The architectural hypothesis is a versioned declarative recipe with typed expressions, source references, relationship contracts, grain operations, and semantic parameters. Python and TypeScript builders should construct the same representation. A separate execution binding supplies files, database connections, or in-memory tables. This follows the useful separation demonstrated by Ibis's unbound expressions. [Ibis unbound expressions](https://ibis-project.org/how-to/extending/unbound_expression).

Saving a recipe should not silently serialize a connection, credentials, or a Python dataframe. Binding validation should compare actual schemas with required paths and types. A recipe may retain stable source labels while each run records concrete snapshot identities. Substrait is relevant prior art for cross-language serialization of relational operations, but its stated scope does not settle Iresiphi's business grain and allocation contracts. Investigate lowering to it later rather than assuming it is a complete recipe format. [Substrait specification overview](https://substrait.io/).

### Host callbacks and bindings

PyO3 documents Rust bindings for Python and native Python extension modules. NAPI-RS documents building Node.js addons in Rust. These support investigating thin host bindings around one core. They do not establish that browser TypeScript has the same deployment model as Node.js, or that a full selected engine will compile to WebAssembly. No minimum runtime versions or exact binding APIs are recommended here because the inspected guides are rolling snapshots. [PyO3 guide](https://pyo3.rs/main/), [NAPI-RS](https://napi.rs/).

Arbitrary Python and JavaScript closures are a portability boundary. A serialized expression can describe an addition; it cannot reconstruct an arbitrary closure's captured state and dependencies in another runtime. Proposed policy is to prefer built-in declarative operations and make host-only callbacks explicit. Persist a callback identifier, version, input/output types, null policy, and required runtime, or reject portable saving. Do not imply that source text alone reproduces an environment.

Polars documents both elementwise and batch Python callbacks, and warns about per-element call overhead. This supports studying batch extension points, not a claim about Iresiphi's speed. Black-box callbacks also need declared output shape and dependencies if the planner is to preserve meaningful lineage. [Polars Python functions](https://docs.pola.rs/user-guide/expressions/user-defined-python-functions/).

### Shared semantics before shared buffers

Both APIs need the same rules for nulls versus NaN, integer overflow, decimal rounding, division by zero, empty aggregates, date parsing, timestamp precision, timezones, and ordering. Large integers and decimals need an explicit TypeScript representation; silently using ordinary numbers for every numeric type risks changing values. Cross-language examples should eventually verify results at these boundaries rather than only demonstrate matching method names.

Arrow defines columnar memory layouts and transport conventions. Its C data interface supports sharing Arrow arrays between components in the same process, with ownership and release conventions. Arrow is not itself an execution engine or a guarantee that every Python/Rust/JavaScript boundary is zero-copy. Type conversions, memory ownership, process boundaries, and adapter support determine actual copying. Any zero-copy claim must name and verify a particular path. [Arrow columnar format 1.5](https://arrow.apache.org/docs/format/Columnar.html), [Arrow C data interface](https://arrow.apache.org/docs/format/CDataInterface.html).

### Candidate execution foundations

| Candidate | Documented basis | What an Iresiphi evaluation must establish |
| --- | --- | --- |
| Polars | Lazy Rust and Python expressions, query plans, predicate and projection pushdown | Fit for local transformation workloads; inspect intermediate-grain explanations, schema-dependent operations, and host packaging. |
| DataFusion | Rust library with extensible functions, table providers, optimizer passes, logical nodes, and execution nodes | Whether planner extension and diagnostic needs justify its integration work; prove the required aggregate and temporal semantics. |
| DuckDB | In-process database with documented Python, Node.js, and Rust client APIs | SQL lowering and deployment fit; avoid assuming independent client instances automatically share bound tables or connections. |
| Arrow | Columnar representation and interchange interfaces | Adapter and ownership fit; still requires an execution engine. |

These documented mechanisms come from the [Polars lazy guide](https://docs.pola.rs/user-guide/concepts/lazy-api/), [DataFusion library guide](https://datafusion.apache.org/library-user-guide/index.html), [DuckDB 2.0 client overview](https://duckdb.org/docs/2.0/clients/overview), and [Arrow format](https://arrow.apache.org/docs/format/Columnar.html). The right-hand column is evaluation advice. No speed or memory ranking follows from those docs.

For a Rust core, Polars and DataFusion deserve early feasibility probes; DuckDB is a useful SQL-oriented comparison. Choose one initial executor. Rebuilding a general optimizer before testing the semantic workflow would spend effort on established machinery while leaving the difficult business questions unresolved.

## Ranked hypotheses and boundaries

These rankings express research priorities, not validated demand.

1. Explicit grain transitions with checked contracts offer the strongest product hypothesis. Prove that an engineer can understand why a value was rolled up, repeated, or assigned, and can catch the worked example's allocation error. Correct relational aggregates already exist elsewhere; this explanation workflow must demonstrate additional value.
2. A source-preserving recipe may reduce repeated integration work when the same source families recur. Test that claim with the user's real datasets and a second run after drift. Preserving names alone is insufficient differentiation; stable mappings and reusable semantics must replace actual repeated effort.
3. Run diagnostics may be more valuable initially than exhaustive row provenance. Show precise failing keys, match counts, and affected outputs. Compare whether these answer real debugging questions before paying the storage cost of full contribution capture.
4. One portable plan may justify Python and TypeScript APIs if users truly exchange recipes. Establish whether TypeScript means Node.js data engineering, browser inspection, or both. A common expression vocabulary is more credible initially than arbitrary callback portability.

A narrow MVP opportunity is batch construction of a customer/month table from a few tabular sources on one machine. It would cover declared keys and grains, named many-to-one enrichment, grouped sums and counts, explicit output population, broadcast context, and one allocation rule with conservation checks. Support a small typed expression set, required schema validation, logical lineage, and a run report. The example is a candidate acceptance scenario, not a finished scope commitment.

Explicit deferrals should include automatic relationship discovery, unrestricted many-to-many planning, arbitrary portable callbacks, distributed execution, online feature serving, streaming corrections, and complete row provenance for every operation. Preserve a place for temporal semantics in the model, but do not promise leakage-safe ML outputs before availability-time behavior exists. Nested and wide inputs can initially require declared adapters rather than universal automatic interpretation.

No reviewed source proves that the proposed combination is unique across the market. Existing systems already provide semantic mappings, relationship-aware aggregation, reusable expressions, and historical feature retrieval. The plausible distinction is a small embedded library that makes grain changes and allocation auditable across Python and TypeScript while retaining source references. Whether that combination solves a sufficiently frequent problem remains untested.

## Questions for the real workflow and later engine evaluation

The next discussion should use actual inputs and one recurring output. Which transformations are repeated today? Are sources files, frames, or warehouse tables? Which columns identify a row, and which identities change over time? Does the user need observed customer/months, all eligible months, or prediction events? Are coarse targets context or assignments? Who approves weights, and what happens when they sum to zero? Is historical correctness about event time or what was known then? Which debugging question must lineage answer? Which runtime executes the recipe, and must another language reopen it?

Later engine evaluation should start with manually reviewed expected results. Include duplicated and null keys, independent fact streams, overlapping distinct populations, unequal weights, zero-weight groups, missing target rows, filter placement, historical corrections, timezone month boundaries, decimals, and required schema drift. A candidate fails if it cannot preserve the chosen semantics or explain an unsupported request.

Only after correctness should representative workload measurements compare scan volume, intermediate row counts, peak memory, spill behavior, startup cost, binding transfer costs, and packaging. Measure with pinned engine, adapter, and binding versions and the same source snapshots. Ibis's support matrix is a useful reminder that an abstraction can outgrow a backend's supported operations. A later decision should include explain-plan access and instrumentation, not throughput alone. [Ibis backend support](https://ibis-project.org/backends/support/matrix).

The immediate design question is whether explicit populations, checked grain transitions, and allocation explanations remove a real recurring failure in the user's workflow. That can be tested before committing to a broad integration platform.
