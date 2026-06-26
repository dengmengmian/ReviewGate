# Terraform Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [TF1] Plaintext secrets/credentials in `.tf` or a variable `default` (e.g. `default = "S3cr3t"` for a password/token/key var), or sensitive values landing in unencrypted state — always report.
- [TF2] IAM policies using `*` wildcards, or security groups/firewalls exposing sensitive ports (SSH 22, RDP 3389, DB 3306/5432) to `0.0.0.0/0` via `cidr_blocks` — always report.
- [TF3] Stateful resources (databases, buckets) lacking lifecycle protection such as `prevent_destroy`.
- [TF4] Provider or module versions not pinned (no `version`/`required_providers` constraint), so upgrades introduce breaking changes.
- [TF5] Changing the keys or order of `count`/`for_each` causes resources to be destroyed and recreated instead of updated in place.
