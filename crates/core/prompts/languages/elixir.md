# Elixir Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [EX1] Do not use String.to_atom/list_to_atom to turn external input into atoms; atoms are not GC'd and will exhaust the atom table and crash the VM, use to_existing_atom.
- [EX2] A spawned process should be placed under a supervision tree (Supervisor/Task.Supervisor); a bare spawn has no one to restart it after a crash and loses the error.
- [EX3] Pattern matching in case/with/function clauses must cover possible inputs; a missing branch throws MatchError/CaseClauseError instead of degrading gracefully.
- [EX4] Distinguish String (UTF-8 binary) from charlist ('...') and generic binary; mixing them causes errors in comparison, concatenation, and library calls.
- [EX5] A GenServer handle_call is synchronous, blocking, and has a timeout; long-running operations placed in call will bog down the caller and trigger a timeout.
- [EX6] Do not directly destructure operations that can fail (e.g. elem(.., 1), [h | _]) without handling {:error, _}; respect the {:ok, _}/{:error, _} convention.
