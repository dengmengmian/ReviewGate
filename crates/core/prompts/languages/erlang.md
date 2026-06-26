# Erlang Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [ERL1] A process `receive` must match and consume every message it may receive; unmatched messages stay in the mailbox forever, causing unbounded growth and memory exhaustion.
- [ERL2] Do not use `list_to_atom` to turn external/dynamic data into atoms; atoms are never reclaimed and will exhaust the atom table and crash the node, use `list_to_existing_atom`.
- [ERL3] Do not swallow failures by discarding errors after `catch` / `try`; follow let-it-crash and let the supervisor handle it, or match `{error, _}` explicitly.
- [ERL4] Hot code upgrades must handle `code_change` and process state migration correctly; lingering old `fun` references or incompatible state formats will crash after the upgrade.
- [ERL5] A `receive` without an `after` timeout can block forever; waits that interact with the outside world should set a timeout branch.
- [ERL6] Mind the difference between `=:=` strict equality and `==` arithmetic equality (e.g. `1 == 1.0` is true but `=:=` is false); mixing them yields silently wrong results.
