# Zig Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [ZIG1] Every alloc/create needs a matching free; follow an allocation with defer allocator.free/destroy. Missing free on an early return or error path leaks.
- [ZIG2] Default integer arithmetic overflow panics under Debug/ReleaseSafe and is undefined behavior under ReleaseFast; use the wrapping operators +% / -% / *% explicitly when wraparound is intended.
- [ZIG3] Memory initialized to undefined has indeterminate contents; reading an undefined value that was never actually written is undefined behavior, so do not rely on it being zero or any fixed value.
- [ZIG4] A call returning an error union must handle the error (try / catch / if-else); using catch unreachable or ignoring the error triggers undefined behavior at runtime or drops the failure branch.
- [ZIG5] Holding a pointer/slice to freed memory or to a local/temporary that has left scope causes a dangling reference; beware of returning stack data or using memory after free.
