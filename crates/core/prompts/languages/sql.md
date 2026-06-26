# SQL Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [SQL1] Building queries by string concatenation instead of parameterization/prepared statements lets external input cause SQL injection.
- [SQL2] UPDATE/DELETE missing a WHERE clause or with an always-true WHERE rewrites or deletes the whole table.
- [SQL3] Column and literal type mismatch (e.g. passing a number to a string column, wrapping an indexed column in a date function) triggers implicit type conversion that disables the index or causes a full table scan.
- [SQL4] Using `= NULL` / `!= NULL` for null checks, or `NOT IN (subquery)` where the subquery contains NULL, makes the result always empty.
- [SQL5] Multi-step writes not wrapped in a transaction, or missing the necessary locks/isolation level, cause partial commits or concurrency races.
- [SQL6] Using `SELECT *` in production queries, views, or migrations breaks dependents or fetches excess data when the column structure changes.
