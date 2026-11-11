# Iresiphi

**Iresiphi** (isiZulu for “recipe”) is a planned data integration library for describing how differently structured datasets connect and how they become a useful output.

Instead of repeatedly reshaping every source into a standard schema, define reusable recipes over the raw fields. A recipe can describe relationships, transformations, changes in granularity, lineage, and the output you need.

The planned implementation has a Rust core with Python and TypeScript interfaces. This repository captures the initial design; the API and implementation are not available yet.

## Design direction

- Keep source datasets in their native structure and field names.
- Declare how source fields correspond and how records relate.
- Select raw fields or define transformed features as recipe inputs.
- State the grain of each source and output.
- Make aggregation, broadcasting, and allocation explicit when data changes granularity.
- Track the source fields and operations behind each output for lineage and debugging.

The cooking metaphor is a guide to the concepts: datasets and fields are ingredients, a recipe records the steps, and the specified output is the finished dish. Technical terms remain preferred where the metaphor would make behavior less clear.

## Illustrative API

These examples show the naming direction only; they are not executable yet.

Python:

```python
import iresiphi as rsp

recipe = rsp.Recipe("customer_summary")
recipe.save("customer_summary.iresiphi")
```

TypeScript:

```typescript
import * as rsp from "iresiphi";

const recipe = new rsp.Recipe("customer_summary");
```

`rsp` is the preferred shorthand in both languages. `.iresiphi` is an illustrative file extension. The recipe serialization format and versioning are still to be designed.

## Project status

This is an early design project. There is no published package or implemented API yet. The current implementation intent is a shared Rust core with Python and TypeScript packages; binding technology, execution engine, and supported input/output formats remain open decisions.

The package name `iresiphi` returned no package record in direct checks of PyPI, npm, and crates.io on 11 November 2026. Registry availability can change, and this does not reserve the name.

## Name

“Iresiphi” is the isiZulu word for “recipe.” The name reflects the idea of a reusable set of instructions for turning raw data ingredients into a specified result.

## Design and research

- [Initial design notes](DESIGN.md) record the vocabulary and open design questions.
- [Domain grounding](docs/research/domain-grounding.md) examines related systems, documented tradeoffs, correctness pitfalls, lineage, and execution options. It includes a worked example and distinguishes evidence from hypotheses for Iresiphi.

## License

Not selected yet.
