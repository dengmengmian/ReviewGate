# Rust Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [RS1] Library code and service request paths should not use `unwrap` / `expect` for recoverable errors.
- [RS2] `as` casts may truncate, change signedness, or lose precision; boundary-sensitive code should use `try_from` or explicit checks.
- [RS3] `unsafe` must have locally verifiable invariants, especially around pointers, aliasing, lifetimes, and FFI boundaries.
- [RS4] Do not hold `Mutex` / `RwLock` guards across `await`, which can cause deadlocks or scheduler blocking.
- [RS5] Large-object `clone`, unbounded collection, or blocking I/O on hot paths must have an acceptable cost proven by context.
