# Decision history 11

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

## Question

Which existing execution foundation can implement the selected preview operations and integration contracts with acceptable packaging, diagnostics, and resource behavior?

Evaluate candidates against the workflow, types, grain rules, required adapters, capability model, and public explanation/run evidence. Establish the lowering/integration boundary and distinguish features supplied by the engine from semantics and checks Iresiphi must own.

Use the existing repository research as a starting point, not proof of current APIs or operational guarantees. When exact compatibility or behavior is uncertain, create focused research or prototype child decisions/evidence tickets with explicit success criteria and make this choice wait on them. Research uses a /research subagent and cited primary sources under the Wayfinder workflow.

Record the choice, supported limits, known incompatibilities and rejection rules, and rationale against viable alternatives. Do not choose by Rust branding, speculative speed claims, or a benchmark that omits validation and data movement.


https://github.com/andrew3stedall/iresiphi/issues/11#issuecomment-6797686446

## Resolution — DataFusion behind an Iresiphi semantic plan

Autonomous decision under the developer's authority. Choose DataFusion 55.2.0 with Rust 1.94.0 as the implementation baseline, using its Arrow re-export rather than selecting an independent Arrow version. Pin direct dependencies and Cargo.lock. [Engine and bindings feasibility evidence](https://github.com/andrew3stedall/iresiphi/issues/15) supplies primary-source research, not a successful combined build or benchmark.

DataFusion supplies relational execution, batches, optimizer/physical machinery and participating-operator memory reservations. Iresiphi owns the semantic DAG, typed scalar rules, identity checks, population preservation, lookup-candidate counting, unresolved statuses, grain ownership, source references, checks and public explanation. Lower only validated operations into the engine. Optimizations must preserve checks and hidden dependencies; assert output population/identity invariants after execution. Count date-qualified candidates before selecting a rate, never use a post-join filter that drops unmatched anchors.

Choose it over Polars 0.55.2 because the documented resource integration and execution-plan cancellation contracts give an appropriate seam for this preview. Polars remains a viable future alternative; neither engine has a measured speed advantage here. Building a relational engine from scratch would add unnecessary scope. Source readers remain explicit adapters: serde_json 1.0.154 with raw_value, csv 1.4.0, calamine 0.38.0. Built-in readers must not bypass the adapter's evidence or type rules.

The research's suggested scales <=18 and rejection of rounding do NOT supersede the resolved scalar contract. Support declared decimal precision/scale up to 38, half-even and toward-zero quantization, exact checked arithmetic, and the established inference/rejection rules. Implement checked Rust kernels or engine extensions when DataFusion's native coercion/arithmetic cannot satisfy that contract. Such kernels are a deliberate semantic boundary, not a claim native behavior already conforms. Exact integer-coefficient reference cases must test value, resulting schema, overflow, negative ties, and scale extremes. There is no fallback to float or silent narrower precision.

Likewise the research's blanket rejection of floating XLSX monetary cells is superseded by the explicit quantization policy: absent that policy, reject; with it, interpret the observed finite numeric cell via an explicitly documented conversion to its shortest round-trip decimal representation, then apply the declared checked quantization. Record observed numeric representation, precision policy and cell reference. This does not recover the originally intended decimal. Prefer text decimal cells in the acceptance fixture. Formula cells require cached values, never evaluation; missing/uncertain cache presence is an adapter error, and freshness is not certified. Validate both Excel epochs, reject the fictitious 1900 leap day and fractional date-only serials.

Unsupported plan operations fail before execution; invalid data and overflow fail before publication. No spill or remote placement is implied by choosing DataFusion. Integration gates: locked build, real adapters, exact checked kernels including quantization, date/ambiguity cases, safe grain operations, memory/cancellation/error cleanup and full invoice acceptance. Failure of an upstream native kernel triggers the specified wrapper implementation, not weakening the contract. If the chosen version cannot support these explicit seams, implementation must report a concrete blocker and reopen the decision; research has identified no such blocker now.
