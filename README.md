# Iresiphi

**Iresiphi** (isiZulu for “recipe”) is a planned data integration library for describing how differently structured datasets connect and how they become a useful output.

Instead of repeatedly reshaping every source into a standard schema, define reusable recipes over the raw fields. A recipe can describe relationships, transformations, changes in granularity, lineage, and the output you need.

The planned implementation has a Rust core with Python and TypeScript interfaces. A developer preview, version 0.1.0, implements one workflow in Rust and Python: an auditable invoice review over local JSON, CSV and XLSX files. [docs/preview.md](docs/preview.md) describes what it supports, its limits, and an example you can run. Source-built Node.js bindings with TypeScript declarations are available; the broader design below remains illustrative.

Rust is preferred for validation, planning, and local execution, with thin language bindings. Performance work will cover complete recipes, including data movement and memory use, while retaining execution in databases or Spark where the plan calls for it. No performance advantage has been measured yet.

## Design direction

- Keep source datasets in their native structure and field names.
- Compose inputs across formats and locations, including databases, dataframes, Spark dataframes, documents, and files, through extensible adapters.
- Declare how source fields correspond and how records relate.
- Select raw fields or define transformed features as recipe inputs.
- State the grain of each source and output.
- Make aggregation, broadcasting, and allocation explicit when data changes granularity.
- Track the source fields and operations behind each output for lineage and debugging.
- Plan execution at sources, through parallel ingestion and transformation, or across both, with explicit control over data movement and output destinations.

The cooking metaphor is a guide to the concepts: datasets and fields are ingredients, a recipe records the steps, and the specified output is the finished dish. Technical terms remain preferred where the metaphor would make behavior less clear.

## Illustrative API

The preview's real API is in [docs/preview.md](docs/preview.md), [examples/run.rs](examples/run.rs) and [examples/run.py](examples/run.py). The examples below show the longer-term naming direction only and do not run.

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

`rsp` is the preferred shorthand in both languages. The preview saves `.iresiphi`
documents using recipe format `0.1`; the Node package provides a lower-level portable recipe interface ([Node guide](docs/node.md)).

## Project status

Node.js 22/24 source bindings: [build and use the Node package](docs/node.md).

The 0.1.0 Rust/Python developer preview is implemented but has not been published
to crates.io or PyPI. Its scope and remaining limitations are described in the
[preview guide](docs/preview.md). Preview decisions are tracked in the
[developer-preview map](https://github.com/andrew3stedall/iresiphi/issues/2).
Its resolution comments govern preview contracts; the design direction above
preserves broader product intent.

The package name `iresiphi` returned no package record in direct checks of PyPI, npm, and crates.io on 11 November 2026. Registry availability can change, and this does not reserve the name.

## Name

“Iresiphi” is the isiZulu word for “recipe.” The name reflects the idea of a reusable set of instructions for turning raw data ingredients into a specified result.

## Design and research

- [Developer-preview decision map](https://github.com/andrew3stedall/iresiphi/issues/2) indexes the invoice-review workflow and resolved planning decisions.
- [Implementation status](docs/status.md) records completed work, verification gates and remaining scope.
- [Engine and bindings evidence](docs/research/preview-engine-bindings.md) records primary-source findings and implementation acceptance gates.

- [Initial design notes](DESIGN.md) record the vocabulary and open design questions.
- [Shared terminology](CONTEXT.md) defines the domain language.
- [Sources and execution](docs/design/sources-and-execution.md) captures format/location independence, cross-source composition, and proposed execution strategies.
- [Domain grounding](docs/research/domain-grounding.md) examines related systems, documented tradeoffs, correctness pitfalls, lineage, and execution options. It includes a worked example and distinguishes evidence from hypotheses for Iresiphi.
- [Snowflake semantic views](docs/research/snowflake-semantic-views.md) studies reusable source mappings, relationship-aware metrics, level-of-detail calculations, and their implications for Iresiphi.

## Continuous integration

[CI and processor architecture](docs/ci.md) describes the GitHub-hosted x86 job,
Terraform-managed repository settings, and where to add ARM-incompatible workloads
as implementation proceeds. Work requiring x86 must run through GitHub Actions;
ARM-compatible builds, tests and processing may run locally on ARM. Every change
and release gate needs a pass on both ARM and x86 at the same commit; see
[Required passes](docs/ci.md#required-passes-arm-and-x86).

## License

Iresiphi's code and documentation are licensed under the [Apache License 2.0](LICENSE). The Rust crates it depends on keep their own licenses; [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) lists them and reproduces their NOTICE files.

The agent skills vendored under `.claude/skills` and `.agents/skills` are third-party material under their own MIT licenses (`MATT-POCOCK-LICENSE`, and `unslop/LICENSE` with `unslop/NOTICE.md`). Apache-2.0 does not apply to them, and they are not part of the crate or the Python distributions.
