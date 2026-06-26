# Nim Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [NIM1] A `ref` object defaults to `nil` after declaration; dereferencing before `new`/initialization triggers `NilAccessDefect`; objects with `ref` fields likewise need layer-by-layer initialization.
- [NIM2] Default integer overflow behavior under release depends on compile switches (`-d:danger` disables checks, making it undefined/wrapping); handle wrapping semantics explicitly rather than relying on defaults.
- [NIM3] Lifetime and reference-cycle collection behavior differ across memory-management modes (`--mm:orc/arc/refc`); destructors/cycle code written for one mode may leak or use-after-free after switching.
- [NIM4] Passing data containing GC-managed references (`string`/`seq`/`ref`) across threads is unsafe; pass by value/`isolate` or use dedicated channels between threads instead of sharing GC-heap objects.
- [NIM5] Templates/macros are hygienic by default, isolating injected identifiers from call-site symbols; misusing `inject`/unqualified symbols leads to capture or resolution to unintended symbols.
