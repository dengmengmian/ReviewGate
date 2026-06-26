# OCaml Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [ML1] A `match` not covering all constructors (non-exhaustive match) raises `Match_failure` at runtime; an old `match` missing a branch after adding a variant is the same issue — address the compiler warning rather than masking it with a wildcard.
- [ML2] `List.hd` / `List.tl` / `List.assoc` / `Option.get` etc. raise on an empty list or missing key; switch to the safe `option`-returning forms like `List.assoc_opt` / pattern matching.
- [ML3] `ref` / `mutable` / `array` are aliasable shared mutable state with multiple bindings pointing at the same cell; mutating through an alias causes unexpected remote-visible side effects.
- [ML4] Polymorphic comparison (`(=)`, `compare`, `Stdlib.compare`) raises on function values, fails to terminate on cyclic structures, and its structural ordering does not match semantics; provide a dedicated `equal`/`compare` for custom types.
- [ML5] `=` is structural equality while `==` is physical equality; the two are easily confused, producing comparison results opposite to expectations.
