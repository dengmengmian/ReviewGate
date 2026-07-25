Security deep review (sink-driven). Be thorough: high-severity misses are unacceptable.

## Method (mandatory)
1. **Sink inventory**: Scan every `+` line for dangerous sinks (list below). Record each hit.
2. **Taint / caller trace**: For each sink that may receive external data, spend at least one `find_callers` / `read_file` / `code_search` to confirm whether the input is user- or request-controlled and whether validation/allowlists were removed in this diff.
3. **Report or drop**: Report only when the sink is real and lacks an equivalent safe control. If data is clearly not attacker-controlled (constant, internal-only, already sanitized with evidence), do not report.
4. **Removed guards**: If this change deletes or weakens auth, allowlists, parameterization, escaping, TLS/JWT verification, or bounds checks without replacement, report the removal as a security finding.

## Dangerous sinks (non-exhaustive)
- **Injection**: SQL/string-built queries; shell/`exec`/`system`/backticks; template eval; path join + file open; LDAP/NoSQL string concat.
- **XSS / HTML**: `dangerouslySetInnerHTML`, `innerHTML`, `v-html`, `{@html}`, `| safe`, unescaped template output; URL sinks (`href`/`src`/`location`) from untrusted input.
- **SSRF / open redirect**: HTTP clients or redirects to user-controlled URL/host/IP without allowlist + scheme/private-IP checks.
- **AuthZ / AuthN**: missing auth on sensitive routes; IDOR (object id from client without ownership check); client-only role gates; JWT `decode` instead of `verify`; disabled signature/alg checks.
- **Secrets**: hardcoded API keys, tokens, passwords, private keys in source.
- **Crypto / tokens**: MD5/SHA1 for passwords; ECB; hardcoded IV/salt; non-crypto PRNG (`Math.random`, `rand`) for tokens/keys/session ids.
- **Deserialization / XXE**: `pickle`, unsafe `Marshal`/`ObjectInputStream`/`yaml.load`, XML parsers with external entities enabled on untrusted input.
- **CSRF / mass assignment**: state-changing endpoints without anti-CSRF where cookie auth applies; binding request bodies straight into privileged fields (role, balance, isAdmin).
- **ReDoS / path traversal**: nested-quantifier regex on untrusted input; `../` path open without root confinement.

## Reporting rules
- Prefer **high** severity for exploitable injection, auth bypass, secret leak, SSRF, RCE-class issues.
- One location may have multiple issue classes — report separately when real.
- Fill `suggestion_code` with a concrete safe replacement when possible.
- Call `report_finding` as soon as each issue is confirmed; call `task_done` when the sink inventory is complete.
