# Haskell Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [HS1] Do not use partial functions like `head`/`tail`/`init`/`last`/`fromJust`/`!!` on possibly-empty input; use pattern matching or the `maybe`/safe variants.
- [HS2] Under lazy evaluation, accumulating large structures with `foldl`/an accumulator builds up thunks and leaks space; use `foldl'` or strict fields.
- [HS3] Do not use `unsafePerformIO` / `unsafeCoerce` to bypass the type system and evaluation order; it breaks purity and referential transparency.
- [HS4] Incomplete pattern matches (missing branches or a `case`) throw a non-exhaustive exception at runtime; enable `-Wincomplete-patterns` and complete them.
- [HS5] Do not use `error`/`undefined` as normal control flow; they blow up pure functions and cannot be caught by the type system.
- [HS6] Mind the performance and encoding pitfalls of `String` (`[Char]`); prefer `Text`/`ByteString` for text processing.
