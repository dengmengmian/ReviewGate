# C# Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [CS1] Async calls must not miss `await`; library code should avoid `async void`.
- [CS2] `Task.Result` / `.Wait()` can cause deadlocks or thread-pool blocking.
- [CS3] `IDisposable` / `IAsyncDisposable` resources must use `using` / `await using` or equivalent cleanup.
- [CS4] Deferred LINQ execution must not capture changing external state or repeatedly enumerate expensive queries.
- [CS5] SQL, command, path, and HTML string concatenation must be parameterized or escaped.
- [CS6] Empty or catch-all `catch {}` / `catch (Exception) {}` that swallows the exception without logging or rethrowing hides failures — report it.
