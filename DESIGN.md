# Initial design notes

## Problem

Engineers often receive datasets with unique schemas and different levels of detail. Repeatedly standardising those datasets for recurring work takes time, obscures how sources connect, and makes data issues harder to trace.

Iresiphi aims to let engineers preserve source structures and define reusable instructions for relationships, feature transformations, granularity changes, and requested outputs.

## Vocabulary

The shared terminology lives in [CONTEXT.md](CONTEXT.md).

## Rust and performance

Rust is the preferred implementation language for recipe validation, dependency resolution, planning, and local execution. Python and TypeScript should expose thin interfaces to the same core so calculation semantics and planning behavior stay consistent. Use host-language adapters where needed to work with existing dataframe objects, connections, and execution environments.

For local computation, evaluate an existing Rust execution engine before building joins, aggregation, or an optimizer from scratch. The engine and binding technologies remain open decisions. Remote operations should execute in Snowflake, SQL databases, or Spark when the chosen plan calls for them; a Rust core does not require local execution of all work.

Performance requirements should shape the design from the beginning:

- Keep per-record calculation loops out of the Python and JavaScript interfaces. Prefer typed batch operations and explicit batch extension points.
- Minimize unnecessary copies, serialization, and transfers. Verify specific interchange paths before promising zero-copy behavior.
- Push filters, projections, and aggregation to an appropriate executor when this preserves meaning and reduces total work or transfer.
- Reuse shared calculations across requested fields and avoid join expansion when a semantically equivalent plan avoids it.
- Bound concurrency, buffering, and memory use. Evaluate partitioning and disk spill for work that exceeds available memory.
- Include required source validation and diagnostics in performance measurements. Make expensive contribution tracing explicit without silently weakening correctness checks.
- Measure complete recipes, including source access, staging, conversion, planning, execution, and output delivery. Record latency, throughput, transferred bytes, and peak memory against representative baselines.

Rust is an implementation direction, not evidence of a speed advantage. Engine choice and optimization priorities should follow correctness checks and measurements on the user's actual workflow. No benchmarks have been performed yet.

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
