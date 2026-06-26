# Python Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [PY1] Do not use mutable objects as function default arguments; use `None` and create the object inside the function.
- [PY2] Do not silently swallow errors with bare `except:` or broad `except Exception`.
- [PY3] Do not use `is` / `is not` to compare string, number, or other literal values.
- [PY4] Async functions, coroutines, and tasks must be awaited, have exceptions collected, or be bound to a lifecycle.
- [PY5] External input reaching SQL, shell, paths, pickle, or templates must be parameterized, allowlisted, or routed through safe APIs.
