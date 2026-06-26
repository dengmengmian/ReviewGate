# PowerShell Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [PS1] External input concatenated into `Invoke-Expression`, `&`/`.` invocation, or a script block without validation/escaping, constituting command injection.
- [PS2] Using `$ErrorActionPreference = 'SilentlyContinue'` or a bare `try{}catch{}` to swallow errors, letting failures be silently ignored.
- [PS3] Writing the `$null` comparison on the right (`$x -eq $null`) returns the filtered collection rather than a boolean when `$x` is an array; write `$null -eq $x`.
- [PS4] Relying on the pipeline to pass objects while assuming a single value/specific type, where a collection or `$null` through the pipeline makes iteration count or type differ from expectations.
- [PS5] Parameters without constraints like `[ValidateSet]`/`[ValidatePattern]` or type annotations let illegal input reach dangerous downstream operations directly.
