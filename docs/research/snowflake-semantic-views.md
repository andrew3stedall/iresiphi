# Snowflake semantic views and Iresiphi

Research and source access date: 11 November 2026. This is documentation research using opened, first-party Snowflake pages. No Snowflake account was available. SQL below is illustrative, manually reviewed against the cited syntax references, and unexecuted. There are no benchmark results or claims about observed runtime behavior. Project context comes from [README.md](../../README.md), [DESIGN.md](../../DESIGN.md), and the [domain-grounding note](domain-grounding.md).

## Finding and status

Snowflake semantic views address a substantial component of Iresiphi: a reusable semantic relationship graph that maps physical sources to named expressions and plans queries for requested dimensions and metrics. Source-preserving mappings, reusable measures, and grain-aware planning therefore cannot distinguish Iresiphi by themselves. Snowflake's overview explicitly separates modeling business entities and relationships, mapping them to physical data, and querying the resulting object. [Semantic views overview](https://docs.snowflake.com/en/user-guide/views-semantic/overview).

The scope remains different. Iresiphi proposes a Rust-core library with Python and TypeScript APIs, explicit allocation, run diagnostics, and contribution provenance. The comparison here concerns semantic planning, not a replacement for a warehouse, broad ETL, or an AI assistant. These are proposed Iresiphi properties, not implemented advantages.

Status needs feature-level precision:

| Mechanism | Evidence at access date |
| --- | --- |
| Defining schema-level semantic views | GA in the June 2025 Summit announcements. [Release note](https://docs.snowflake.com/en/release-notes/2025/june-summit). |
| Querying through `SEMANTIC_VIEW(...)` | GA in server release 9.25, August 2025. [Release note](https://docs.snowflake.com/en/release-notes/2025/9_25). |
| Direct `FROM semantic_view` with `AGG(metric)` | GA on 2 March 2026. [Release note](https://docs.snowflake.com/en/release-notes/2026/other/2026-03-02-semantic-views-standard-sql). |
| SQL query as a logical table | GA on 26 June 2026. [Release note](https://docs.snowflake.com/en/release-notes/2026/other/2026-06-26-semantic-views-inline-view-ga). |
| Metric relationship selection with `USING` | Open Preview in the current DDL reference. [CREATE SEMANTIC VIEW](https://docs.snowflake.com/en/sql-reference/sql/create-semantic-view). |
| Level-of-detail expressions | Open Preview; the release index dates the announcement to 2 November 2026. This is especially recent evidence. [Release index](https://docs.snowflake.com/en/release-notes/new-features-2026), [preview list](https://docs.snowflake.com/en/release-notes/preview-features). |

These are rolling documentation snapshots, not a pinned account version. A GA core does not imply that every extension is GA.

## Documented mechanism

### Logical sources, identities, and relationships

`CREATE SEMANTIC VIEW` declares `TABLES`, `RELATIONSHIPS`, `FACTS`, `DIMENSIONS`, and `METRICS`. Logical aliases refer to physical sources; `PRIMARY KEY` and additional `UNIQUE` column sets describe identity. A relationship can have a name and maps foreign-key columns to referenced key columns. This does not require renaming physical columns. [DDL reference](https://docs.snowflake.com/en/sql-reference/sql/create-semantic-view).

A logical table can instead contain an SQL query, including joins and transformations. Its alias is mandatory; session variables are prohibited, and normal `CREATE VIEW` query restrictions apply. Thus schema adaptation can live inside the semantic object without first creating another physical table or ordinary view. [SQL-backed logical tables](https://docs.snowflake.com/en/user-guide/views-semantic/inline-view).

The YAML specification describes relationships through `left_table`, `right_table`, and paired `relationship_columns`. Unlike legacy semantic models, semantic views do not require `join_type` or `relationship_type`. One-to-one inference uses primary-key declarations on both sides. Do not turn the documentation's broader phrase “inferred from the data” into a claim that deployment performs exhaustive key validation. [YAML specification](https://docs.snowflake.com/en/user-guide/views-semantic/semantic-view-yaml-spec).

Physical integrity and semantic declarations must be distinguished. Standard Snowflake tables do not enforce primary/foreign-key referential integrity; hybrid tables do enforce constraints. This physical-table fact does not establish exactly which checks semantic-view compilation or execution performs. Treat logical keys as contracts requiring source validation unless experiments establish stronger guarantees. [Table design considerations](https://docs.snowflake.com/en/user-guide/table-considerations).

Relationships support one-to-one and many-to-one traversal, including transitive paths. Validation rejects circular relationships and direct self-reference. Multiple paths have restrictions on cross-table expression references. Row expressions can use related expressions at equal or coarser grain; referencing finer-grained data requires aggregation. Cross-table physical columns cannot simply be referenced directly. The validation page has an inconsistent granularity example for orders and line items, so its isolated “higher/lower” wording should not become an Iresiphi specification. [Validation rules](https://docs.snowflake.com/en/user-guide/views-semantic/validation-rules).

### Facts, dimensions, metrics, and output grain

Dimensions describe grouping context; facts are reusable row-level quantitative expressions; metrics aggregate values. Facts can also contain aggregates over related sources, producing a value at their owning table's grain. Derived metrics live at view level and can combine metrics from different tables. These distinctions preserve calculation ownership instead of flattening every number into an interchangeable column. [YAML specification](https://docs.snowflake.com/en/user-guide/views-semantic/semantic-view-yaml-spec).

The `SEMANTIC_VIEW` query construct requests dimensions, metrics, or facts. Requested dimensions define grouping. `FACTS` and `METRICS` cannot appear together. With `FACTS` and `DIMENSIONS`, all referenced facts and dimensions, including filters, must belong to one logical table; dimensions should determine the facts or results can be nondeterministic. This is not an unrestricted raw-row join interface. [SEMANTIC_VIEW reference](https://docs.snowflake.com/en/sql-reference/constructs/semantic_view).

A metric's query dimensions must be related and at equal or coarser table grain. `SHOW SEMANTIC DIMENSIONS IN <view> FOR METRIC <metric>` exposes compatible dimensions and whether a dimension is required. This is a useful precedent for asking what outputs are legal before execution. [Dimension compatibility reference](https://docs.snowflake.com/en/sql-reference/sql/show-semantic-dimensions-for-metric).

Direct semantic SQL uses `AGG(metric)` and `GROUP BY`; it also supports aggregates of facts or dimensions. `WHERE` refers to facts/dimensions, while `HAVING` filters metrics through the supported aggregate interface. The `SEMANTIC_VIEW` result can participate in ordinary SQL joins and CTEs. [Query guide](https://docs.snowflake.com/en/user-guide/views-semantic/querying).

### Fanout, multiple facts, and ambiguous paths

The modeling guide says many-to-many relationships are not directly supported and recommends a bridge with two many-to-one relationships. The SQL guide explicitly describes bridge-path inference and deduplication against fanout double-counting. These statements are compatible: a direct many-to-many edge and a modeled bridge path are different mechanisms. [Modeling guidance](https://docs.snowflake.com/en/user-guide/views-semantic/best-practices-modeling), [bridge example](https://docs.snowflake.com/en/user-guide/views-semantic/sql#modeling-many-to-many-relationships-with-bridge-tables).

Interpretation: neither “every fanout is rejected” nor “all arbitrary joins are safe” follows. Preserving a book's sales through an author bridge does not mean each author receives an allocated share. Overlapping groups may each legitimately include the same book; summing their subtotals is a different request.

For ambiguous paths, metric definitions can choose named relationships with `USING`; this is currently Preview. View-scoped derived metrics can combine table-scoped metrics, including metrics with chosen paths. Consequently multiple fact sources are within the model's expressive scope. This is not permission to join raw fact streams arbitrarily or combine them by dimensions unavailable to one component. [Relationship selection and derived metrics](https://docs.snowflake.com/en/sql-reference/sql/create-semantic-view), [authoring comparison](https://docs.snowflake.com/en/user-guide/views-semantic/yaml-vs-ddl).

Interpretation: model sales and targets as separate measures with their own grains, then establish compatible output dimensions. A common region does not make receipt-level and region/month values interchangeable. The inspected sources do not establish a universal physical algorithm, such as always using full outer joins for every multi-fact query. Exact null handling, missing groups, and bridge behavior need experiments.

A documented rejection illustrates the boundary: requesting an order-date dimension alongside a customer-owned order-count metric fails because the dimension table is finer-grained. Creating a coarser-owned summary changes what finer outputs remain legal. [Query guide](https://docs.snowflake.com/en/user-guide/views-semantic/querying#choosing-the-dimensions-that-you-can-return-for-a-given-metric).

### Filters, nonadditivity, and time

Filter placement also needs an explicit query boundary. A predicate inside a semantic request restricts its calculation inputs; an outer SQL predicate over the returned relation filters that relation. For ordinary metric queries, use dimension/fact conditions before aggregation and metric thresholds afterward. LOD below introduces a documented exception to ordinary input-filter expectations. [SEMANTIC_VIEW reference](https://docs.snowflake.com/en/sql-reference/constructs/semantic_view), [query guide](https://docs.snowflake.com/en/user-guide/views-semantic/querying).

Reusable Boolean facts/dimensions can carry `LABELS = (FILTER)` and be used in `WHERE`. Naming a filter does not make it a mandatory row-security rule. [Filter definitions](https://docs.snowflake.com/en/user-guide/views-semantic/filters).

`NON ADDITIVE BY` supplies snapshot-style semi-additivity. Snowflake orders by the specified dimensions and aggregates values from the last rows; ascending time selects the latest snapshot and descending time selects the earliest. This is more specific than “never aggregate this column.” [Semi-additive feature note](https://docs.snowflake.com/en/release-notes/2026/other/2026-03-05-semantic-views-semi-additive-metrics), [DDL semantics](https://docs.snowflake.com/en/sql-reference/sql/create-semantic-view).

Interpretation: balances across dates require a different rule from monthly sales. Ratios should combine their defining measures at the requested grain; neither snapshot selection nor deduplication supplies target allocation. Distinct populations likewise require their own expression semantics.

Window metrics operate on aggregate expressions. They can partition by selected dimensions except named exclusions. Required partition/order dimensions must be requested, and window metrics cannot feed other metrics or row calculations. Row-level window expressions are also documented. [Query guide](https://docs.snowflake.com/en/user-guide/views-semantic/querying#defining-and-querying-window-function-metrics), [authoring comparison](https://docs.snowflake.com/en/user-guide/views-semantic/yaml-vs-ddl).

Interpretation: a seven-row frame does not itself establish seven calendar days when dates are missing. Event-time grouping, snapshot choice, and “known at prediction time” are separate contracts. The DDL also documents ASOF and interval relationships; their existence alone does not prove bitemporal leakage safety. [Relationship syntax](https://docs.snowflake.com/en/sql-reference/sql/create-semantic-view).

The new LOD preview supports fixed grain and adding/removing dimensions relative to query grain. `FIXED` calculates before query filtering; `INCLUDE`/`EXCLUDE` calculate after filtering. LOD calculations cannot reference other LOD calculations, and table-scoped metrics cannot reference LOD expressions. Cortex Analyst/Agents do not yet use LOD metrics. [LOD definitions, filter example, and restrictions](https://docs.snowflake.com/en/user-guide/views-semantic/sql#defining-level-of-detail-lod-metrics).

This directly revises a differentiation hypothesis: explicit calculation grain and denominator filter timing already have Snowflake mechanisms. Iresiphi would need to make those choices easier to validate and explain in its intended library workflow.

## SQL objects, YAML, AI, and debugging

YAML and DDL author the same kind of native object, with some feature differences; they are not separate execution engines. Legacy staged YAML remains usable as Analyst context but lacks native semantic SQL querying. YAML can create native views through `SYSTEM$CREATE_SEMANTIC_VIEW_FROM_YAML`; export uses `SYSTEM$READ_YAML_FROM_SEMANTIC_VIEW`. [Migration guide](https://docs.snowflake.com/en/user-guide/views-semantic/semantic-models-vs-views), [YAML export reference](https://docs.snowflake.com/en/sql-reference/functions/system_read_yaml_from_semantic_view).

Cortex Analyst accepts a semantic-view name and generates SQL from a natural-language question. That interpretation step is different from deterministic execution of an explicit semantic SQL request. [Analyst API](https://docs.snowflake.com/en/user-guide/snowflake-cortex/cortex-analyst/rest-api). Routing Mode, itself Preview, attempts semantic SQL and can fall back to SQL over physical tables. An AI answer therefore needs inspection of the generated query before assuming semantic-view planning supplied all correctness protections. [Routing Mode](https://docs.snowflake.com/en/user-guide/snowflake-cortex/cortex-analyst/cortex-analyst-routing-mode).

Definitions are inspectable through `DESCRIBE SEMANTIC VIEW`, including expressions and relationships. Metadata views cover tables, relationships, facts, dimensions, and metrics in Information Schema and Account Usage. These are concrete foundations for catalog inspection. [DESCRIBE reference](https://docs.snowflake.com/en/sql-reference/sql/desc-semantic-view), [auditing metadata](https://docs.snowflake.com/en/user-guide/views-semantic/views).

Snowflake's general `EXPLAIN` returns logical operations and expressions without executing SQL. This note did not verify the detail exposed for semantic queries. A logical plan is not a contribution trace identifying every input record behind a result. [EXPLAIN reference](https://docs.snowflake.com/en/sql-reference/sql/explain). Analyst logs separately record generated SQL and errors; these explain the generation workflow, not allocation coefficients or input-row provenance. [Analyst monitoring](https://docs.snowflake.com/en/user-guide/snowflake-cortex/cortex-analyst/admin-observability).

Querying requires `SELECT` on the semantic view, without requiring `SELECT` on its underlying tables. Facts/metrics can be private calculation helpers; dimensions remain public. Visibility controls must not be confused with source-quality checks. [Query privileges](https://docs.snowflake.com/en/user-guide/views-semantic/querying#privileges-required-to-query-a-semantic-view), [authoring comparison](https://docs.snowflake.com/en/user-guide/views-semantic/yaml-vs-ddl).

A successful declaration establishes that Snowflake accepted the model under its structural and expression rules. It does not, from the evidence reviewed here, certify the business meaning of identity, source completeness, currency compatibility, historical eligibility, or allocation fairness. Those remain source checks and business contracts. This is the central distinction between a valid semantic model and validated inputs. [Model validation](https://docs.snowflake.com/en/user-guide/views-semantic/validation-rules).

## Illustrative customer/month example

This example adapts the existing domain note, not Snowflake sample data. Customers have `CustNo/SalesArea/Share`: A/North/1, B/North/3, C/South/1. Transactions retain `receipt_id/buyer_ref/booked_at/gross`: t1/A/2026-01-05/60, t2/A/2026-01-20/40, t3/B/2026-01-11/200, t4/C/2026-01-17/50, t5/B/2026-02-08/120. Targets retain `Territory/Period/Goal`: North/2026-01/900, South/2026-01/300, North/2026-02/800, South/2026-02/400.

Assume AUD, business-local dates, stable regions and shares, complete activity, and all customers eligible in both months. Those assumptions require business/source validation.

### Supported sales request

The following deliberately excludes targets. It preserves physical field spelling through quoted identifiers and declares receipt/customer identity. It assumes those exact source tables and date types exist in the current schema. The syntax follows the current DDL grammar, including named relationships, and query grammar. It has not been executed. [CREATE reference](https://docs.snowflake.com/en/sql-reference/sql/create-semantic-view), [SEMANTIC_VIEW reference](https://docs.snowflake.com/en/sql-reference/constructs/semantic_view).

```sql
CREATE SEMANTIC VIEW customer_sales
  TABLES (
    c AS customers PRIMARY KEY ("CustNo"),
    t AS transactions PRIMARY KEY (receipt_id)
  )
  RELATIONSHIPS (
    buyer AS t (buyer_ref) REFERENCES c ("CustNo")
  )
  FACTS (t.amount AS gross)
  DIMENSIONS (
    c.customer_id AS "CustNo",
    c.region AS "SalesArea",
    t.month AS DATE_TRUNC('MONTH', booked_at)
  )
  METRICS (
    t.sales AS SUM(t.amount),
    t.transaction_count AS COUNT(receipt_id)
  );

SELECT * FROM SEMANTIC_VIEW(
  customer_sales
  DIMENSIONS c.customer_id, t.month
  METRICS t.sales, t.transaction_count
);
```

Manual expected observed groups are A/January/100/2, B/January/200/1, C/January/50/1, B/February/120/1. Sales total 470. This request groups transaction activity; it does not explicitly declare the six-row eligible customer/month population. Do not promise absent February groups or zero filling without a population definition and execution check.

### Targets, context, and allocation

A target logical table can declare composite identity `(Territory, Period)` and `SUM(Goal)` at its own grain. Connecting customers to targets using region alone would be invalid reasoning: North has two target rows. Connecting raw transactions by both region and month needs adaptation, since region is reached through customers and month is derived. The key validation rules allow base columns or direct-column semantic references, not arbitrary computed key expressions. An SQL-backed logical table can explicitly expose the needed region/month fields. [Key validation](https://docs.snowflake.com/en/user-guide/views-semantic/validation-rules), [SQL-backed table support](https://docs.snowflake.com/en/user-guide/views-semantic/inline-view).

The following outcomes distinguish supported mechanisms from this note's unexecuted design proposals:

| Requested result | Assessment |
| --- | --- |
| Customer/month sales | Supported by the sales model above, subject to valid source contracts. |
| Region/month sales against region/month targets | Suitable multi-fact comparison after modeling compatible dimensions for both measures. Exact combined-query/null behavior remains unverified. |
| Target metric broken down by customer using only a shared region | Not a valid ordinary metric breakdown: customers are finer than regional targets, and region alone does not determine a target month. The compatibility rule predicts rejection rather than allocation. |
| `regional_goal_context` repeated on all six eligible customer/months | Requires an explicit eligible population and region/month lookup. An SQL logical-table definition can express this plan. A native LOD-only alternative remains unverified. |
| `allocated_goal` weighted by eligible `Share` | Needs an explicit SQL formula and recipient definition, possibly inside a logical table, or precomputation. No reviewed source establishes a dedicated allocation operator. This is not evidence that SQL cannot express it. |

The compatibility assessment uses the [metric-dimension rule](https://docs.snowflake.com/en/sql-reference/sql/show-semantic-dimensions-for-metric); the SQL-definition route uses [documented logical-table queries](https://docs.snowflake.com/en/user-guide/views-semantic/inline-view). The table's specific plans are interpretation, not executed Snowflake results.

For an explicit SQL definition, first build six eligible customer/month rows, aggregate transactions, and join each row to its unique regional target. Normalize `Share` over eligible customers within region/month, then multiply by `Goal`. This avoids counting A twice because A has two January receipts. January allocations are A=225, B=675, C=300; February allocations are A=200, B=600, C=400. They total 2400. Repeated context totals 4100 and must not become a target-total metric. Joining January receipts first would instead multiply North's 900 three times, producing a false January target sum of 3000 including South.

Filter timing is part of this recipe. Selecting A after allocation leaves January at 225; restricting eligibility to A before normalization produces 900. LOD's documented filter distinction makes an implementation worth testing, but does not settle its legality across this graph, denominator population, zero weights, or missing recipients. An explicit definition must handle those cases and monetary rounding. Conservation alone is insufficient: receipt-weighted allocations can conserve 900 while assigning the wrong customer shares.

## Implications for Iresiphi

The following mapping is design analysis grounded in the mechanisms above.

| Snowflake concept | Proposed Iresiphi counterpart | Design lesson |
| --- | --- | --- |
| Logical table and source expression | Source binding and adapter | Preserve raw identity separately from projected fields. |
| Named relationship and keys | Relationship contract | Store path, expected multiplicity, and observed validation separately. |
| Fact/dimension/metric ownership | Feature and measure definition | Keep row grain and aggregation population in the representation. |
| Semantic view plus requested dimensions | Catalog plus output recipe | Reuse relationships without making every output a new catalog. |
| LOD and window definitions | Explicit grain/time operations | Specify filter scope, denominator, ordering, and missing periods. |
| Metadata and compatibility queries | Logical explanation | Show why a request is legal before executing it. |

A recipe object should reference a versioned source/relationship catalog and add its requested output population, dimensions, measures, and grain changes. Snowflake is strong evidence for separating reusable definitions from a query-specific request. Iresiphi should still allow an output recipe to carry additional business rules such as eligibility and allocation.

Measure validation should cover identity, path compatibility, additivity, currency, and temporal meaning. A declared key is not evidence that the current source satisfies it. Record duplicate/null keys, unmatched rows, actual multiplicities, and whether checks scanned all data. A measure's allowed grouping grain should be inspectable, following Snowflake's compatibility-query precedent.

The credible differentiation hypothesis is an embedded, portable workflow with run evidence and explicit allocation contracts. Detailed provenance would include receipt contributions, the target row, every denominator participant, and coefficients. The inspected metadata/plan interfaces do not establish that complete facility; their silence is not proof that Snowflake has no other lineage tooling. Test the user need before claiming a competitive advantage.

Concrete later experiments should:

1. Execute the sales example and verify the four observed groups and total 470, then define the six-row population separately.
2. Corrupt `CustNo`, receipt identity, and target uniqueness; establish which stages detect violations and inspect wrong-result risks.
3. Compare shared-dimension multi-fact queries with manual aggregation, including missing targets and inactive customer/months.
4. Test the predicted fine-dimension rejection, bridge deduplication, ambiguous-path errors, and `USING` behavior.
5. Probe LOD filter timing against A's 225-versus-900 case without assuming LOD composition can express the full allocation.
6. Inspect `EXPLAIN`, metadata, and executed profiles for path/grain explanations; identify what additional row diagnostics require separate queries.
7. Test snapshot ties, missing dates, timezone boundaries, zero denominators, rounding residuals, and generated-SQL fallback privileges.

Those experiments would establish operational guarantees. This documentation study establishes substantial semantic-planning overlap and a narrower, still unproven opportunity for Iresiphi's local execution and debugging workflow.
