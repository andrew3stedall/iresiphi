# Decision history 6

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

Workflow context: [Choose the representative workflow and developer-preview acceptance criteria](https://github.com/andrew3stedall/iresiphi/issues/3#issuecomment-6796579135). Use the confirmed invoice-review acceptance contract's unit-rate comparison and numeric examples when defining canonical types, expression behavior, and temporal limits; the workflow did not settle all pricing or date policies.

## Question

Which typed expressions and scalar semantics does the preview require, and what exact results or diagnostics do they produce across supported runtimes and executors?

Choose the initial expression set and types required by the workflow. Specify numeric precision, overflow, decimal and integer transport, rounding, division by zero, empty values/aggregates, null versus NaN, supported comparisons/collation, and explicit conversions where relevant.

Define date parsing, timestamp/timezone handling, business-calendar boundaries, and the temporal guarantees actually supported. Distinguish unsupported historical/as-of/availability-time operations from supported date transformations.

State the canonical behavior expected through each supported host interface and the conditions under which pushdown is semantically incompatible. Do not choose a backend's incidental behavior as a business rule without a documented decision. Resolve the highest-risk examples into reviewable expected values; unsupported operations must have defined rejection behavior.


https://github.com/andrew3stedall/iresiphi/issues/6#issuecomment-6797556305

## Resolution — exact, deliberately small scalar language

Autonomous decision under the developer's authority; selected behavior, not implementation evidence.

Preview scalar types are nullable Boolean, signed Int64, UTF-8 string, Decimal128(p,s) with 1 <= p <= 38 and 0 <= s <= p, and calendar Date. Collections exist at projection/detail boundaries, not as arbitrary scalar expressions. Timestamp/timezone calculations, binary floating-point arithmetic, arbitrary Python callbacks, regex, and user-defined functions are deferred. Input NaN and infinity are rejected; they are not null. Missing JSON paths and blank spreadsheet cells become null only when the declared field is nullable; malformed present values are errors. Empty string stays empty string.

Expressions comprise typed literals, field/path references, explicit casts, add/subtract/multiply, comparisons, Boolean AND/OR/NOT, is-null, coalesce, case, and explicit decimal quantization. Division is deferred and rejected statically, including division by zero; no engine-dependent infinity. Date equality/order and the explicit rate-validity predicate are supported. No implicit string-to-number conversion or cross-type key comparison. Null comparisons yield unknown; three-valued Boolean logic applies; filters retain only true; case tests unknown as not true. Coalesce is explicit, never automatically used to fill unresolved charges.

All integer arithmetic is checked; overflow fails the run. Decimal arithmetic is exact until an explicit quantization. Addition/subtraction use scale=max(s1,s2) and precision=max(p1-s1,p2-s2)+scale+1; multiplication uses precision=p1+p2 and scale=s1+s2. An inferred precision beyond 38 is rejected statically rather than silently reduced. Quantization names target precision/scale and rounding mode; preview supports half-even and toward-zero. Narrowing overflow fails; there is no saturation or wrap. Avoid declaring every input decimal(38,s) when a narrower business range is appropriate.

The invoice fixture declares quantity decimal(12,4), rate decimal(14,6), billed decimal(18,2); multiply exactly then half-even quantize expected charge to decimal(18,2), subtract billed minus expected at cent scale. Currency is a required explicit code; the fixture uses one currency. Mixed-currency summation/comparison without grouping by currency is rejected. FX, tax, tiering, credits, and discounts are deferred business operations. Positive quantities are an acceptance-fixture constraint, not a universal decimal restriction.

Transport canonical integers and decimal values as tagged decimal strings in portable JSON; decimal tags carry precision/scale. Python may expose native int/Decimal, but conversion must be exact and validated in Rust. JSON numeric literals are not accepted for declared decimal fields unless the adapter preserves their original lexical value and parses it exactly. XLSX numeric cells require an explicit decimal quantization policy because workbook numeric storage cannot establish original monetary precision; source evidence retains the observed cell representation. No zero-copy or exact-original-Excel-decimal claim follows.

Dates accept only explicit ISO YYYY-MM-DD strings or adapter-declared date cells mapped to Gregorian dates. Locale guessing is forbidden; Excel date-system metadata and invalid serials must be handled explicitly by the adapter. Timestamps require caller preprocessing to a declared business date; local timezone/calendar inference is deferred. Rate validity is inclusive lower/exclusive upper date, with explicit open-ended upper bound. No availability-time or bitemporal semantics.

Examples: 2*50 -> 100.00 and 110-100 -> 10.00; 1*45 ->45.00 and 50-45 ->5.00; half-even 1.005->1.00 and 1.015->1.02; null rate yields null expected/variance with unresolved status; Int64 max+1 fails; `null = null` is unknown; empty string is not null; a rate ending 2026-11-12 does not apply on that date. Identical results/errors are required through supported Rust/Python public interfaces. Future executors must prove these semantics before accepting pushdown.
