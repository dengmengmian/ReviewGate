# GitHub Actions Workflow Rules

Report only when the diff clearly hits one of these traps and creates real risk:

- [GHA1] `pull_request_target` (or `workflow_run`) combined with checking out the PR head (`ref:` using `github.event.pull_request.head.sha`/`head.ref`): untrusted fork code runs with a write token and secrets access.
- [GHA2] Untrusted input (`github.event.*` titles, bodies, branch names, commit messages) interpolated directly into `run:` via `${{ }}` — shell injection; pass it through `env:` and quote it instead.
- [GHA3] Secrets exposed to untrusted contexts: echoed into logs, written to artifacts, or made available to workflows triggered by forks.
- [GHA4] Overly broad token permissions on workflows that handle untrusted events (e.g. `permissions: write-all`, or granting `contents: write`/`pull-requests: write` where read would do).
- [GHA5] Third-party actions pinned to a mutable ref (`@main`, `@v1`-style movable tags) in workflows that handle secrets or have write permissions — pin to a full commit SHA.
