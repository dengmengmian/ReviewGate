# GraphQL Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [GQL1] No limit on query depth/complexity or pagination cap; nested queries can be used for DoS.
- [GQL2] A resolver queries the database per item on a list field, forming an N+1 without `DataLoader` batching.
- [GQL3] Authorization is done only at the entry point rather than per field/object, so sensitive fields can be read by bypassing it.
- [GQL4] Introspection is enabled in production, or `__schema`/`__type` is left exposed, revealing internal structure.
- [GQL5] No limit on how many times the same field can be aliased; batched aliasing amplifies single-request cost and bypasses rate limiting.
