# Crystal Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [CR1] A union type (e.g. `String?`) used to call a method directly without narrowing via `if x` or abusing `.not_nil!`, when the compiler infers it as `Nil` and misuses it.
- [CR2] Integers default to `Int32`; loop counters, accumulators, shifts, file sizes, and similar cases not using `Int64`/`UInt64` or not guarding against overflow (Crystal integer overflow panics).
- [CR3] Splicing external identifiers directly in a `macro` without hygiene handling, causing variable capture, duplicate definitions, or ambiguous generated code.
- [CR4] Multiple Fibers sharing mutable state via `spawn` without `Channel`/`Mutex` synchronization, relying on the implicit assumption of Fiber cooperative scheduling rather than explicit synchronization.
- [CR5] A blocking call (heavy CPU computation, blocking C bindings) written in a Fiber without yielding, starving the event loop and hanging the whole system.
