# Perl Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [PERL1] The top of a file/module must `use strict; use warnings;`; their absence permits undeclared variables and hidden errors.
- [PERL2] Mind the difference between list context and scalar context: assigning an array to a scalar yields the element count, not the first element, easily causing logic errors.
- [PERL3] Do not rely on global/regex-capture variables like `$_`, `@_`, `$1` retaining state across calls; `local`-ize or assign explicitly before use.
- [PERL4] Avoid injection when building `system`/`exec`/backticks/`eval`/regex from external input; enable taint mode and validate when necessary.
- [PERL5] Hashes are unreliable in boolean/numeric context, and `keys`/`each` order is random; do not depend on traversal order, and modifying a hash during `each` iteration causes errors.
- [PERL6] Compare strings with `eq`/`ne` and numbers with `==`/`!=`; mixing the two yields silently wrong results.
