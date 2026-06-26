Security vulnerabilities. Checklist:
- Injection: SQL/command/path/template concatenation without parameterization or escaping.
- Authentication and authorization: privilege bypass, missing authentication, trusting client-controlled data.
- Secrets: hardcoded keys, passwords, or tokens.
- **SSRF**: requests to **user-controlled URLs, hosts, or IPs** (`http.Get`, `fetch`, `curl`, `requests`, `openConnection`, etc.) without allowlists, protocol checks, or private-network address checks. Pay special attention when this change removes an existing allowlist or validation.
- Unsafe deserialization (`pickle`, `Marshal`, `ObjectInputStream`, etc. on untrusted data), XXE, and insecure randomness.
- Weak cryptography: MD5/SHA1 for passwords, ECB mode, hardcoded IVs or salts.
- Missing input validation, dangerous defaults, path traversal, and ReDoS from nested-quantifier regexes on untrusted input.
- **Do not be distracted by a more obvious bug on the same line**: one location can contain multiple issue classes, such as both ignored errors and SSRF. Check each class and report them separately when real.
