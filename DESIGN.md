# Initial design notes

## Problem

Engineers often receive datasets with unique schemas and different levels of detail. Repeatedly standardising those datasets for recurring work takes time, obscures how sources connect, and makes data issues harder to trace.

Iresiphi aims to let engineers preserve source structures and define reusable instructions for relationships, feature transformations, granularity changes, and requested outputs.

## Vocabulary

The shared terminology lives in [CONTEXT.md](CONTEXT.md).

## Source and execution requirements

Users should be able to relate inputs across formats, locations, and runtimes without first moving everything into a prescribed store or converting everything to one user-facing dataframe format. Required source families include SQL databases, Snowflake, in-memory dataframes, Spark dataframes, NDJSON, XML, Excel, CSV, text, and Parquet.

Any pair of supported sources should be eligible to participate in the same recipe. Source support should be extensible through adapters. Record extraction, relationship meaning, and execution feasibility still need explicit definitions.

Execution should support operations pushed to a source's execution environment, ingestion into an executor, and hybrid plans. Independent reads and transformations should run in parallel where dependencies and resource limits permit. Output destinations should be independent of input locations and execution placement.

Plan from the requested fields and their relationship and transformation dependencies. Depending on that route, execution may load external data into Snowflake, process Snowflake results elsewhere, or combine both. Preserve semantic path choices and consider shared dependencies across the complete output when choosing placement.

These are product requirements, not a claim that connectors or executors already exist. Proposed module responsibilities, constraints, and examples are recorded in [Sources and execution](docs/design/sources-and-execution.md).

## Granularity operations

These operations should be explicit because they have different meanings and risks:

- **Aggregate / roll up:** combine finer-grained rows using a named aggregation rule.
- **Broadcast:** repeat a coarser-grained value across finer-grained rows where the relationship permits it.
- **Allocate:** distribute a coarser-grained value across finer-grained rows using an explicit weight or rule.

## Naming

- Project and intended package name: `iresiphi`.
- Preferred Python and TypeScript import alias: `rsp`.
- Core configuration object: `Recipe`.
- Illustrative recipe extension: `.iresiphi`.

The Python distribution, Python import package, npm package, Rust crate, repository, and extension may have different technical constraints; keep their names aligned where practical. Recheck registry availability before publishing.

## Open questions

1. What concrete recurring workflow should become the first example?
2. Which contrasting source adapters and executors should demonstrate cross-source composition first, while preserving the broader source and execution requirements?
3. Should recipes be declared in Python/TypeScript, serialized as data, or support both?
4. Which serialization format and schema versioning policy should recipe files use?
5. How should the engine detect and report unmatched keys, duplicate matches, and unexpected row-count changes?
6. Which Rust binding approach best supports Python and Node.js while keeping the core portable?
