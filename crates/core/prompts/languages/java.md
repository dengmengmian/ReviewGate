# Java Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [JAVA1] Do not construct monetary `BigDecimal` values from `double`; money, ratios, and rounding must be explicit.
- [JAVA2] `equals` / `hashCode` must remain consistent; fields used as collection keys must not mutate after insertion.
- [JAVA3] `Optional.get()`, nullable unboxing, and chained calls must have null/empty protection.
- [JAVA4] Thread pools, streams, connections, and locks must be released on both success and error paths.
- [JAVA5] SQL, command, path, and deserialization inputs must be parameterized, allowlisted, or explicitly validated.
