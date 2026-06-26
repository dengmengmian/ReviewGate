# Go Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [GO1] `err` must not be accidentally shadowed with `:=`, especially near transactions, defers, or named returns.
- [GO2] Goroutines, closures, and defers that capture loop variables must copy them explicitly.
- [GO3] `defer` inside loops may delay resource release and exhaust handles.
- [GO4] `context.Context` must be propagated; external I/O, RPC, and DB calls must not lose cancellation or timeouts.
- [GO5] Concurrent map/slice reads and writes require synchronization; do not assume an appended slice's backing array is safe to share.
