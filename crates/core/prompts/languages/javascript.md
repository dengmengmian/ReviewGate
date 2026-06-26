# JavaScript Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [JS1] Prefer `===` / `!==` unless the code deliberately relies on `null == undefined`.
- [JS2] Do not use `for...in` to iterate arrays; object iteration must account for prototype pollution.
- [JS3] Money, IDs, and counters must not exceed `Number.MAX_SAFE_INTEGER`; use `BigInt` or strings when needed.
- [JS4] Promises must be awaited, returned, or caught to avoid unhandled rejections and races.
- [JS5] External input reaching DOM, templates, regexes, commands, or URLs must be escaped, validated, or passed through safe APIs.
