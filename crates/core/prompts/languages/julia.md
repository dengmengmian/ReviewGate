# Julia Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [JL1] Type instability (return type depending on a runtime value, container element type `Any`, or unparameterized fields) triggers dynamic dispatch and severely slows hot paths; verify with `@code_warntype`.
- [JL2] Array indices start at 1 and ranges are inclusive of the endpoint; porting from 0-based languages or hand-computing `1:n` / `end` offsets easily causes off-by-one out-of-bounds.
- [JL3] Reading or writing non-`const` global variables in performance-sensitive code introduces type uncertainty and boxing overhead; pass them as arguments or declare them `const`.
- [JL4] Broadcasting `.` applies element-wise and aligns by shape; a missing or extra dot, or a dimension mismatch, yields wrong results or unintended implicit expansion.
- [JL5] `@inbounds` / `@boundscheck` disable bounds checking; once an index goes out of bounds it is undefined behavior (may read garbage memory or crash), use only when safety is already proven.
