# Swift Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [SWIFT1] Avoid forced unwrap `!` unless the invariant is locally proven.
- [SWIFT2] Closure captures of `self` must avoid retain cycles; async tasks must consider object lifetime.
- [SWIFT3] UI updates must run on the main thread or MainActor.
- [SWIFT4] `try?` must not swallow errors that require diagnosis or recovery.
- [SWIFT5] Array indexes, ranges, and string indexes must validate boundaries first.
