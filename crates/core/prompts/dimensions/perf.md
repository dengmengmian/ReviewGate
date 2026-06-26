Performance issues. Checklist:
- N+1 queries, I/O, or network calls inside loops.
- Unnecessary copies, clones, or allocations where borrowing/reuse would be appropriate.
- High complexity on hot paths, such as nested loops or repeated computation.
- Work that should be cached but is recomputed, or operations that should be batched but run one-by-one.
- Blocking calls inside async contexts.
