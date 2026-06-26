# Shell/Bash Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [SH1] Variable expansion without double quotes (`$VAR`/`$@`) causes word splitting and glob expansion, behaving incorrectly when paths contain spaces or special characters.
- [SH2] Script lacks `set -euo pipefail`, so errors do not exit, undefined variables are treated as empty, and failures of intermediate pipeline commands are ignored.
- [SH3] Dangerous operations like `rm -rf "$VAR/..."` or `cd $DIR` do not guard against `$VAR` being empty and expanding to the root path or wiping the current directory.
- [SH4] External input concatenated into `eval`, a command string, or backtick execution constitutes command injection.
- [SH5] Using `[ ]` instead of `[[ ]]` in Bash for tests involving `&&`/`||`/regex/unquoted variables triggers word-splitting and operator ambiguity.
