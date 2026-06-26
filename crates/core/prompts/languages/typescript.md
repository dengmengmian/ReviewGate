# TypeScript Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [TS1] Avoid `any`, double assertions, and non-null assertions that bypass real boundaries; external input must be parsed and validated first.
- [TS2] Prefer `===` / `!==`; do not rely on implicit type coercion.
- [TS3] Promises must be awaited, returned, or caught to avoid unhandled rejections and races.
- [TS4] Types exist only at compile time; runtime authorization, money, enum values, and data shapes must be explicitly validated.
- [TS5] External input reaching DOM, templates, regexes, commands, or URLs must be escaped, validated, or passed through safe APIs.
