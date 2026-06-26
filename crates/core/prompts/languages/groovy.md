# Groovy Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [GROOVY1] Under default dynamic dispatch, type errors in method/property access are deferred to runtime; hot paths or contract-critical code missing `@CompileStatic` / `@TypeChecked` let null pointers and typos slip through.
- [GROOVY2] Missing safe navigation: use `?.` for chained calls on possibly-null values, and mind Groovy truthiness (empty string, empty collection, and 0 are all false) to avoid misjudgment.
- [GROOVY3] Splicing a `GString` containing external input directly into SQL, Shell (`execute`), or `GroovyShell`/`Eval` causes injection; SQL should use placeholders (parameterization via `groovy.sql.Sql`).
- [GROOVY4] A closure's `delegate` and `resolveStrategy` determine the name-resolution target; in a DSL, not setting `DELEGATE_FIRST` or a wrong delegate object silently resolves to the outer scope, causing behavioral drift.
- [GROOVY5] `==` in Groovy means `equals`, not reference equality; use `is()` for reference comparison and do not misuse `==` when `Comparable` ordering is needed.
