# Clojure Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [CLJ1] Side effects wrapped in lazy sequences (map/filter/for, etc.) do not run until realized; side effects should use doseq/run!/doall to force evaluation.
- [CLJ2] A dynamic var binding only takes effect on the current thread and does not propagate to new threads via future/pmap/agent, so the rebinding is lost.
- [CLJ3] A null returned from Java interop becomes nil in Clojure and must be handled explicitly; repeated boxing/unboxing of primitives also introduces performance and NPE risk.
- [CLJ4] A dosync transaction body may be retried automatically on conflict, so it must not contain non-idempotent side effects (I/O, sending messages, atomic counters); move side effects out of the transaction or mark them with io!.
- [CLJ5] Comparing numbers with = is type-sensitive (e.g. (= 1 1.0) is false); cross-type numeric equality should use ==.
