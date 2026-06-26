# PHP Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [PHP1] Prefer `===` / `!==` to avoid weak-comparison authorization or validation bypasses.
- [PHP2] External input reaching SQL, HTML, commands, or file paths must be parameterized, escaped, or allowlisted.
- [PHP3] `unserialize`, dynamic include, and reflective calls must not process untrusted input.
- [PHP4] File uploads must validate size, MIME type, extension, storage path, and execution permissions.
- [PHP5] Error handling must not leak stacks, paths, SQL, or secrets; production must not display internal errors.
