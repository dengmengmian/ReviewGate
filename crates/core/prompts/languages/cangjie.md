# Cangjie Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [CJ1] Bypassing null-safety on `Option<T>` via `getOrThrow()`/`!!`, unwrapping without a `match`/`if let` null check.
- [CJ2] Shared mutable state across threads or actors without a lock (`Mutex`/atomics) or with wrong lock granularity, causing data races or deadlocks.
- [CJ3] Files, connections, handles, and other resources not released in `try-finally` or an equivalent mechanism, leaking on exception paths.
- [CJ4] Swallowing exceptions after `try-catch` (empty catch or print-only) without rethrowing or converting to a meaningful error signal.
- [CJ5] Fields accessed across threads not declared thread-safe or not synchronized, relying on a single-thread assumption while actually running concurrently.
