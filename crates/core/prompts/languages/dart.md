# Dart Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [DART1] A late variable must be guaranteed initialized before use, otherwise the first access throws LateInitializationError.
- [DART2] Do not use the ! null-assertion or as to force past null-safety; do an explicit null check or use ?. / ??.
- [DART3] Do not drop await on a Future returned by an async function (especially in loops or transactions), to avoid out-of-order execution and uncaught exceptions.
- [DART4] Use == to compare values and identical() to compare references; do not use == expecting object identity.
- [DART5] Future/Stream errors must be handled (try/catch around await, catchError, onError); unhandled ones become unhandled exceptions.
- [DART6] Do not use BuildContext after awaiting a Future without checking mounted; using a disposed context across an async boundary will crash.
