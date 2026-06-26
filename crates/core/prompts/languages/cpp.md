# C++ Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [CPP1] Avoid raw `new` / `delete` for complex ownership; prefer RAII and smart pointers.
- [CPP2] Iterators, references, `string_view`, and `span` must not outlive container reallocations or temporary objects.
- [CPP3] Container indexing, pointer arithmetic, and `memcpy` / `memmove` lengths must have provably safe bounds.
- [CPP4] Integer conversions, narrowing, and mixing `size_t` with signed integers may truncate or bypass bounds checks.
- [CPP5] Error paths must preserve resource cleanup and object invariants; do not silently swallow errors with catch-all handlers.
