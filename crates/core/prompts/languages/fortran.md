# Fortran Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [FORT1] Array subscripts may go out of bounds, or row-wise iteration over column-major arrays causes cache thrashing / out-of-bounds access.
- [FORT2] Missing `implicit none` relies on implicit typing rules (e.g. names starting with i-n default to integer), causing wrong variable types.
- [FORT3] Uninitialized variables or arrays are used, relying on compiler default values.
- [FORT4] Integer division truncates (e.g. `1/2` gives 0); convert to real explicitly or reorder the operations.
- [FORT5] Local variables lack the `save` attribute yet are expected to retain state across calls, or `save` is overused and introduces hidden global state.
