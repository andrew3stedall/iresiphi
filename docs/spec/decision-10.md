# Decision history 10

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). Use the confirmed invoice-review acceptance contract's contrasting source layouts and lookup/detail relationships as the concrete adapter/composition scenario. Preserve source names and grain meaning while deciding capability and placement contracts.

## Question

What common logical and capability contracts let the selected preview sources, executors, and sinks compose while preserving the broader extension requirements?

Specify the minimal adapter information for schema/projection, source identity, access and batch exchange; operation support and semantic restrictions; same-environment accessibility; and supported sink delivery. Define the logical operations an executor accepts and evidence it returns.

Define placement feasibility from requested-field dependencies, including hidden join/filter/allocation inputs, permitted transfers and staging, semantic compatibility, estimates and uncertainty, and supported policy controls. Clarify whether the preview uses explicit placement or automatic choices within its selected scope.

Resolve at least one contrasting cross-source scenario and one policy-blocked scenario without pairwise connector logic. State rejection behavior when data cannot meet in an allowed location. Specify the extension requirements for deferred remote/hybrid execution even if those strategies do not ship in the preview. Engine-specific APIs are not the common contract.


https://github.com/andrew3stedall/iresiphi/issues/10#issuecomment-6797627647

## Resolution — batch contracts and explicit local placement

Autonomous decision under the developer's authorization. Preview execution placement is explicitly local. No remote source pushdown, remote staging, automatic federation or pairwise connector code ships. The following interface responsibilities preserve future execution choices without pretending those integrations exist.

A source adapter describes projection schema/nullability, stable binding identity and fingerprint support, record-reference production, supported extraction options, estimates with known/unknown status, and access/cancellation limits. It opens a bounded pull-based stream of typed column batches plus source references, validating declared values and preserving raw evidence fields. `close` releases owned resources on all terminal paths. No adapter may silently infer business keys, paths, grain, rate policy or monetary rounding. Batch transport is an internal versioned logical schema; an engine-specific dataframe is not the public adapter contract. Chosen bindings may implement Arrow interchange internally subject to the binding decision.

Capability declarations include supported logical operations, type/precision/null/date semantics, projection/filter restrictions, accessible locations, transfer directions, ordering guarantees, cancellation granularity and resource-accounting limits. They are negotiated against the exact required plan, not a Boolean 'supports SQL'. Unknown capability is unsupported until established. Source scanning/projection does not imply source-side aggregation support.

The logical plan contains scan/project, typed feature evaluation, checked named lookup including date validity, population/filter stages, grouped aggregate, validated broadcast, separate detail/evidence production, and output delivery. An executor accepts only a plan whose exact operations and semantics it can preserve; it returns batches, execution/check evidence and structured errors. Iresiphi owns semantic validation, unresolved-status propagation, identity/cardinality contracts and public lineage; lowering must not discard them. The engine decision selects a concrete implementation behind this seam.

A sink describes accepted types, delivery location, bounded-batch consumption, commit/abort semantics and ownership. Preview sinks are memory and local directory bundles containing primary/detail/evidence CSV or JSON plus schema/manifest. CSV requires accompanying type schema and an unambiguous null representation/escaping policy so empty strings differ from null; no bare CSV is claimed to be lossless. Output ordering is unspecified unless an explicit supported order is requested; the preview does not provide a general sort operation. Stable record IDs permit comparisons independent of order.

Planning takes the transitive closure of requested outputs and evidence, including hidden match keys, invoice date, validity endpoints, population/filter predicates and grain checks. Unrequested sources need not be read, but required validation columns cannot be projection-pruned. Policy declares allowed read roots, allowed output/temp roots, local transfer permission and resource limits. Unknown sizes remain unknown and require runtime enforcement; estimates never override hard policy. Shared reads/calculations may be reused only where population and semantic identity agree.

Concrete supported composition: supplier-specific invoice JSON projections, XLSX enrichment, CSV mappings/rates/components all stream through the same source contract into the local executor and produce one invoice-line output plus detail/evidence tables. No JSON-to-XLSX-specific join implementation is needed. Policy-blocked scenario: a required XLSX file lies outside allowed read roots, or the output bundle lies outside the approved destination; bound explain/validate reports infeasibility and run refuses before unauthorized access. A future remote-only source with local transfer prohibited is also infeasible; there is no hidden download fallback.

Future remote/hybrid extensions must declare same-environment accessibility, exact supported semantics, transfer/staging permission and ownership, estimates/uncertainty, cancellation and cleanup behavior, and sink capability. A planner may choose among only semantically equivalent feasible placements. Adding a new source must not require pairwise adapters for existing formats. Detailed remote integration remains deferred by scope, not unresolved preview fog.
