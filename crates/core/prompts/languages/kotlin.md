# Kotlin Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [KT1] Use `!!` only when the invariant is locally proven; external input and cross-boundary data must handle null safely.
- [KT2] Coroutines must be bound to lifecycles and cancellation propagation; avoid `GlobalScope` leaks.
- [KT3] `runBlocking` should not appear on service request paths, UI main threads, or inside async libraries.
- [KT4] Collection `first()` / `single()` / index access must handle empty collections and multi-element boundaries.
- [KT5] Java interop platform types require explicit nullable and exception semantics.
