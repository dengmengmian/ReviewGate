# Lua Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [LUA1] Local variables must be declared with `local`; omitting it pollutes the global table `_G`, causing hidden cross-module state leaks.
- [LUA2] Table indices start at 1 and `#` length is undefined for "arrays" containing `nil`; mixing with 0-based logic invites off-by-one errors.
- [LUA3] Arithmetic/concatenation on a possibly-`nil` value errors immediately; check for `nil` before accessing table fields.
- [LUA4] Do not wrap with `pcall`/`xpcall` and then ignore the returned error info, silently swallowing real failures.
- [LUA5] Inserting `nil` into a table truncates the sequence and changes the `ipairs` traversal range; remove elements with the dedicated approach rather than assigning `nil`.
- [LUA6] When comparing, note that `"1"` does not equal `1` (no implicit conversion), and both `nil` and `false` are falsy but semantically distinct.
