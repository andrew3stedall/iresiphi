# Decision history 8

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

## Question

How should callers define, reuse, save, version, and reopen a preview recipe while preserving its meaning independently of runtime bindings?

Choose whether reusable sources and relationships form a separately versioned catalog or another explicit reuse boundary, and how output-specific rules refer to them. The catalog recommendation in the research is an option to evaluate, not a settled decision.

Define the public declarative model, required semantic information, recipe identity/versioning, serialization format, compatibility rules, and validation of unsupported versions or missing definitions. Decide what the .iresiphi naming direction means for the preview.

Specify what is persisted versus rebound, including source requirements, secrets/connections/live objects, and any host-only extension identifiers and limitations. State the cross-language portability guarantee for supported declarations and explicit rejections for nonportable ones. A compact example or prototype may be added as evidence, but a production implementation is outside this decision.


https://github.com/andrew3stedall/iresiphi/issues/8#issuecomment-6797588137

## Resolution — portable recipe document with runtime bindings

Autonomous decision under the developer's authorization. `.iresiphi` is UTF-8 JSON, not executable Python, a zip, or an engine plan. The root includes `format_version: "0.1"`, human recipe name, source declarations, named relationships, named features, outputs and required capabilities. Stable local IDs identify declarations; references resolve by ID and duplicate IDs fail. JSON object ordering is insignificant; list ordering is significant only where the schema states it (e.g. composite keys and output columns). Unknown semantic properties/versions fail, rather than being ignored; a reserved annotations object may carry nonsemantic metadata.

A source declaration carries projection, field types/nullability, identity, grain, and required adapter capabilities. Relationships carry named paths, typed keys/cardinality, and validity predicates where used. Expressions are a typed data AST. Outputs carry anchor, population/filter stages, fields, explicit grain operations, and detail relationships. Decimal/integer literals use the tagged exact string transport selected in the scalar decision. No arbitrary SQL, Python code, callbacks, engine-specific AST, or dynamic imports.

Reuse uses an in-memory builder/catalog that copies selected declarations into a self-contained recipe. The saved document has no unresolved catalog/network reference: named reusable definitions are bundled, with their content hashes and optional source-catalog provenance. A separately hosted/versioned catalog service is deferred. Conflicting names require explicit aliases; no latest-version lookup on reopening. A recipe may have a human revision label, but its exact semantic identity is a hash of canonical validated document content excluding explicitly nonsemantic annotations. Canonical JSON serialization and its hash algorithm are part of the format specification to implement; choose SHA-256 over recursively key-sorted, compact UTF-8 JSON with only string-encoded numeric literals, no insignificant whitespace, and preserved array order.

Bindings are supplied separately per run: source IDs map to concrete file paths/read handles and adapter settings; sink choice and execution/resource policy are also runtime inputs. Saved recipes contain no secrets, connections, live objects, absolute machine paths, or source data. A run manifest records their safe identities/fingerprints and redacted settings separately. Reopening validates version, references, types and supported capabilities before requesting bindings. Missing bindings have a structured error at bound validation/run, not at unbound recipe validation.

Rust owns validation and serialization. Python builders are ergonomic construction helpers over that same model; loading and saving in either supported surface preserves meaning. Future TypeScript must implement the same portable model and tagged values. Host-only extensions are rejected for this preview, so there is no silent loss of behavior on reopen.

Version compatibility is exact format-version acceptance for the preview. Package patches may fix bugs without changing the file schema; a semantic/schema change increments format version, and old files require an explicit migration tool with reviewable output. No automatic best-effort migration. Required tests are Rust/Python save-load equivalence; duplicate/missing IDs; unsupported version/property/capability; secret-free persistence; and rebinding the unchanged invoice recipe to a second input set.
