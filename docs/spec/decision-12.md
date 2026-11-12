# Decision history 12

Map: [Wayfinder: Iresiphi implementation-ready developer preview](https://github.com/andrew3stedall/iresiphi/issues/2).
Starting specification: [Specification: Iresiphi source-preserving recipes, grain semantics, and execution](https://github.com/andrew3stedall/iresiphi/issues/1).

Mode: human in the loop. Use /grilling and /domain-modeling with the developer driving the map. Read the map's Notes and resolved prerequisite tickets before starting. Claim this issue before work; record its resolution in a comment rather than rewriting the question.

## Question

Which binding and interchange approach should expose the shared Rust core to the host runtimes selected for the preview?

Decide the supported host versions and binding technologies, the boundary between shared semantics and host adapters, input/output value representation, and batch data interchange. Specify memory ownership, lifetimes, release behavior, copying expectations, error transport, and cancellation integration where relevant.

Check large-integer/decimal fidelity, date/null semantics, recipe reopening, and real dataframe or connection access against the chosen engine. Identify host-only extensions and what happens if another runtime cannot provide them.

Use focused research or a throwaway prototype for uncertain compatibility; record version-specific evidence and precise limits rather than promising universal zero-copy or arbitrary callback portability. If a product language is deferred, preserve its contract requirements and record the deferral instead of designing an unshipped binding in full.


https://github.com/andrew3stedall/iresiphi/issues/12#issuecomment-6797699573

## Resolution — PyO3 handles and copied exact Python values

Autonomous decision under the developer's authority, grounded in [Engine and bindings feasibility evidence](https://github.com/andrew3stedall/iresiphi/issues/15). Choose PyO3 0.30.1 and maturin 1.16.0. Support ordinary GIL-enabled CPython 3.12 and 3.13 on Linux x86_64, glibc >=2.28; separate cp312/cp313 wheels. Defer abi3, free-threaded Python, PyPy, browsers and Node/TypeScript delivery.

Rust owns recipe validation, logical planning, serialization, execution and diagnostics. Python builders construct the same declarative values; opaque recipe/run/result handles own Rust state with reference counting. They never borrow Python buffers or connection lifetimes for worker use. Internal batches are DataFusion-compatible Arrow RecordBatch with declared schema, Decimal128/Date32 and validity bits. Python materialization copies to int, Decimal constructed from exact text, datetime.date, str, bool and None. No float intermediates for exact values, PyArrow dependency, dataframe input, Arrow C-data export or zero-copy guarantee. Dataframe/connection adapters are deferred, not secretly accepted through arbitrary callbacks.

Public memory results expose bounded iteration over copied batches and explicit `close`, plus context-manager support. Closing releases retained Rust batches, sources and worker state; access after close returns `HANDLE_CLOSED`. Returned Python values remain valid after close because they own their data. Handle finalization is a cleanup fallback, not a deterministic completion/cancellation API. Repeated close is harmless. No worker calls back into Python; host-only expressions/extensions are rejected as nonportable.

Execution detaches from Python while doing Rust work. The calling thread periodically reattaches to check signals during its bounded waits, translating KeyboardInterrupt to the common cancellation token. Explicit cancellation uses the same path. Dropping streams/resources follows executor cancellation, with no promise to instantly interrupt blocking XLSX parsing. Errors expose stable Iresiphi code/phase/context/report ID; Python raises an Iresiphi exception carrying that record, while user cancellation maps to a cancellation subtype or KeyboardInterrupt preserving the run report. Raw DataFusion/PyO3 exception text is diagnostic detail, not the stable interface.

Serialize/load uses the same versioned document and exact literal encoding in both Rust/Python. Values beyond 2^53, Int64 boundaries, decimal ties/scales up to38, null and date boundaries must round-trip unchanged. Future TypeScript must use exact tagged strings or equivalent lossless values, never JS Number for these domains. Rust/Python public expected-result parity, clean-wheel installation, Ctrl-C, explicit cancellation, repeated close, access-after-close and retained-value-after-close are implementation release gates. Version metadata is evidence of an available approach; a combined build has not yet been performed.
