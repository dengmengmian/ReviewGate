# R Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [R1] Binary operations silently recycle the shorter vector to align lengths (only a warning, not an error, when the length is not an integer multiple), easily producing quietly wrong element-wise results.
- [R2] Indices start at 1, and a negative index means "exclude" rather than count-from-end; misusing negative or 0 indices yields a subset opposite to expectations.
- [R3] `NA` propagates through computation (e.g. `sum`, comparison, logical tests); aggregation needs `na.rm=TRUE`, and emptiness checks must use `is.na()` rather than `== NA`.
- [R4] `<<-` assigns to an enclosing/global scope and may create a global variable, causing hidden cross-function side effects.
- [R5] `read.*` / `data.frame` etc. defaulted to `stringsAsFactors=TRUE` in older versions; converting character columns to `factor` breaks subsequent string/numeric operations, so set it explicitly to `FALSE`.
