# Decision history 14

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). Use the confirmed invoice-review acceptance contract as the preview's workflow-specific release evidence baseline; it establishes synthetic expected/failing cases and a generated workload target, not measured performance or a general-production support commitment.

## Question

What distribution and compatibility commitments make the chosen developer preview installable and usable within its explicitly limited scope?

Choose package/artifact structure for the selected language surfaces, supported platforms and runtime versions, version alignment, recipe compatibility expectations, and the license. Establish what naming/availability checks and redistribution review are required before actual publication.

Define the preview's documented support matrix, known limitations, installation/example expectations, and required evidence: semantic acceptance, actual adapter/executor integrations, host parity, resource/failure cases, and representative complete-recipe measurements where needed. Determine what performance claims are justified by evidence and keep unmeasured claims out.

Reconcile these commitments with the upstream acceptance criteria and explicit deferrals. Record unresolved external prerequisites as precise evidence or task tickets that block completion. This ticket decides the release contract; it does not publish packages, build CI, or implement the preview.


https://github.com/andrew3stedall/iresiphi/issues/14#issuecomment-6797736842

## Resolution — narrowly supported, evidence-gated developer preview

Autonomous decision under the developer's authority. Select Apache-2.0 for new Iresiphi project code and documentation. Existing vendored skills retain their own licenses/notices; this decision does not relicense third-party material. Implementation must add the project license/required notices and inspect the locked dependency distribution before publication. This is a licensing choice, not legal certification of an unbuilt dependency graph.

Release aligned Rust `iresiphi` crate and Python `iresiphi` distribution at initial preview version0.1.0, with one shared Rust semantic core and thin Python module. A Python source distribution includes the Rust sources/lockfile and documents the required build toolchain. Build separate ordinary CPython3.12/3.13 wheels for Linux x86_64 manylinux_2_28 (glibc>=2.28) using pinned build-image digest and maturin1.16.0; no target-cpu=native. Rust1.94.0 and the exact dependency pins selected by engine/binding decisions are the baseline. Audit native wheel dependencies and test minimum-glibc installation/import on clean images. No abi3, free-threaded Python, PyPy, macOS, Windows, ARM, browser or Node/TypeScript support claim in this release.

Recipe format0.1 is explicitly versioned independently of package patches, under the exact compatibility/migration policy in the portable-recipe decision. Saved recipes contain no live bindings; examples must show save, reopen, rebind, validate, explain and run. Publish the support/deferral matrix, resource profile, XLSX quantization/cache limits, CSV null escaping, snapshot limitations, unresolved comparison semantics and cleanup behavior beside the executable example. Broader connectors, remote/hybrid execution, dataframe access, TypeScript and allocation remain product requirements deferred beyond this preview.

Required release evidence: (1) locked Rust build and audited wheels in clean supported environments; (2) common Rust/Python public fixtures including the exact two-supplier160/145/15 comparison, total billed180 and unresolved20; (3) checked scalar oracle cases including38-digit precision, quantization ties/negative values, integer boundaries, overflow/null/date behavior; (4) real JSON/CSV/XLSX adapter tests including numeric lexemes, duplicate keys, CSV escaping, formula cache failures, workbook epochs, ZIP/range limits and source evidence; (5) identity/path/date-overlap/fanout/grain failures; (6) persistence/rebinding parity; (7) budgets, backpressure-visible limits, cancellation, handle ownership, scratch cleanup and atomic publication/retry cases; (8) complete100,000-line workflow measurements including source access, validation, conversion, planning, execution, evidence and delivery.

Record hardware, OS/compiler/runtime versions, immutable fixture seed/hash, input sizes, policy, actual row/evidence counts, elapsed phases, peak RSS where measured, tracked reservations, bytes read/written and output size. Report observations only. No faster-than-Python/database/Spark claim or universal latency/memory promise follows from Rust or these synthetic measurements. There is no fixed elapsed-time release threshold; correctness, completion under the declared test policy and truthful resource reporting are required.

Before publication recheck crate/PyPI names and account ownership, generate and review dependency/license notices, ensure no bundled secrets or unintended source data, and document installation limitations. Name availability is not reservation. These are ordinary release execution gates on artifacts that do not yet exist, not unresolved planning decisions or a need for external account access now. If a required name is unavailable, do not silently publish under a substitute; report the concrete conflict as a new naming decision.

The decision route is ready for specification synthesis and implementation tickets. Nothing here claims packages have been built, tests executed, licenses audited, performance measured or artifacts published. Production implementation and publication remain outside this map.
