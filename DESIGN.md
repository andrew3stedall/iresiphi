# Initial design notes

## Problem

Engineers often receive datasets with unique schemas and different levels of detail. Repeatedly standardising those datasets for recurring work takes time, obscures how sources connect, and makes data issues harder to trace.

Iresiphi aims to let engineers preserve source structures and define reusable instructions for relationships, feature transformations, granularity changes, and requested outputs.

## Initial vocabulary

- **Source:** a dataset and its fields, including a description of what one row represents.
- **Relationship:** a declared connection between sources, including keys and expected match cardinality.
- **Feature:** a selected raw field or a named transformation of fields.
- **Grain:** the entities and dimensions that identify one row, such as customer by month.
- **Recipe:** a reusable specification of source dependencies, relationships, operations, and requested output.
- **Output:** the requested fields, structure, and grain produced by applying a recipe.
- **Lineage:** the record of source fields, relationships, and operations that produced an output field or row.

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
2. Which input table formats and execution backends should the first version support?
3. Should recipes be declared in Python/TypeScript, serialized as data, or support both?
4. Which serialization format and schema versioning policy should recipe files use?
5. How should the engine detect and report unmatched keys, duplicate matches, and unexpected row-count changes?
6. Which Rust binding approach best supports Python and Node.js while keeping the core portable?
