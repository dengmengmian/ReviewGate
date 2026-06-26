# F# Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [FS1] `List.head` / `List.find` / `Option.get` / `Map.find` and the like throw on an empty collection or no match; use `tryHead` / `tryFind` etc. that return an `option`.
- [FS2] `null` returned from .NET interop is outside the F# type system; handle it explicitly with `isNull` / `Option.ofObj`, treating it as non-null triggers an NRE.
- [FS3] `async { }` is cold-started and only runs via `Async.Start`/`RunSynchronously`, unlike the hot-started .NET `Task`; mixing `task`/`async` or forgetting `Async.AwaitTask` loses execution or context.
- [FS4] Types with function fields, reference types, or custom semantics that rely on default structural equality/comparison will misbehave or throw; use `[<CustomEquality>]`/`[<CustomComparison>]` or implement them explicitly.
- [FS5] A non-exhaustive `match` (missing a union case or `None` branch) only produces a compile warning but throws at runtime; complete it or handle the warning explicitly.
