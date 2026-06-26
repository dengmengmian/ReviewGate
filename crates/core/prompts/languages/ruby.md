# Ruby Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [RB1] Do not use bare `rescue` to silently swallow errors; restrict exception types and preserve diagnostic information.
- [RB2] External input reaching SQL, shell, ERB, or paths must be parameterized, escaped, or allowlisted.
- [RB3] Be careful with destructive methods such as `map!` and `gsub!` when they may return `nil` or mutate shared objects.
- [RB4] Mutable constants, class variables, and global state must be isolated in concurrent or multi-request environments.
- [RB5] Money, time, and timezone logic must use explicit types and boundaries; do not rely on implicit conversions.
