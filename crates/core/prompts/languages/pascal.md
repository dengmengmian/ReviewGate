# Pascal/Delphi Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [PAS1] `New`/`GetMem`/object creation not paired with `Dispose`/`FreeMem`/`Free`, or an exception path not releasing via try-finally, causing memory leaks.
- [PAS2] Array, enum, or subrange index out of bounds, exceeding the declared range.
- [PAS3] Assigning an over-long value to a fixed-length `string[n]`/`ShortString` gets silently truncated.
- [PAS4] Using an uninitialized local variable or pointer (including a dangling pointer not set to `nil` after release).
- [PAS5] Integer arithmetic overflowing the type range (e.g. `Integer` accumulation, `Byte` wraparound).
