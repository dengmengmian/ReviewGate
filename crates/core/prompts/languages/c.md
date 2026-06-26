# C Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [C1] Do not read uninitialized variables, struct padding, or uncleared buffers.
- [C2] Array, pointer, `memcpy` / `strcpy` / `snprintf` length calculations may go out of bounds or miss terminators.
- [C3] Integer arithmetic, length conversion, and mixing `size_t` with signed integers may overflow or underflow.
- [C4] Unclear resource ownership can cause double free, use-after-free, leaks, or missing cleanup on error paths.
- [C5] External input reaching format strings, paths, commands, or SQL must be explicitly validated or escaped.
