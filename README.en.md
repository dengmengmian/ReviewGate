<p align="center">
  <img src="docs/assets/logo.svg" alt="ReviewGate" width="420">
</p>

<p align="center">
  Pre-merge quality gate for AI-written code: <b>catch high-risk issues first and reduce low-value review noise</b>
</p>

<p align="center">
  English · <a href="README.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/dengmengmian/ReviewGate/actions/workflows/ci.yml"><img src="https://github.com/dengmengmian/ReviewGate/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/dengmengmian/ReviewGate/releases/latest"><img src="https://img.shields.io/github/v/release/dengmengmian/ReviewGate" alt="Release"></a>
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT">
</p>

ReviewGate is a pre-merge quality gate for AI-generated, or AI-heavy, code. The core path is ready for real PRs and CI. It does not replace tests or human review; it filters PRs before merge by promoting high-risk findings and folding low-confidence noise by default.

| Core value | What it means for teams |
|---|---|
| Catch high-risk issues | Parallel review by security, logic, performance, business rules, and other focused dimensions |
| Reduce noise | Deduplication, counter-evidence judging, and confidence-based filtering |
| Avoid fake passes | Incomplete reviews, timeouts, and oversized context degrade to WARN instead of pretending to pass |

## Quick Start

You need three things: a git repository, an LLM API key, and the `reviewgate` command.

```bash
# 1) Install
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh

# 2) Create a global config. It works across all repositories.
mkdir -p ~/.reviewgate
cat > ~/.reviewgate/config.toml <<'EOF'
provider = "deepseek"

[providers.deepseek]
protocol = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-v4-pro"
EOF

# 3) Keep the API key in the environment, not in the config file.
export REVIEWGATE_API_KEY="your key"

# 4) Check that the model is reachable.
reviewgate llm test

# 5) Enter any git repository with local changes and review them.
cd /path/to/your/repo
reviewgate review
```

`BLOCK` means a high-confidence issue should be handled before merge. `WARN` means there is risk or the review was incomplete. `PASS` means no finding reached the configured gate threshold.

Windows users can install with PowerShell:

```powershell
irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex
```

## Example Output

```text
━━ ReviewGate ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ✖ BLOCK    1 files · 1 must-fix · 0 warn · 3 hidden
  LLM 120k in (cache 88%) · 2.1k out
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▌ MUST FIX

  1  handler.rs:3                       security · high · 100%

     SQL injection: req.user_id is interpolated directly into a DELETE statement...

     Patch
       - let q = format!("DELETE FROM users WHERE id = {}", req.user_id);
       + let q = "DELETE FROM users WHERE id = $1";

▌ NOT SHOWN

  3 low-confidence findings hidden. Run with --show-filtered to inspect them.
```

## When To Use It / Not Use It

| Good fit | Not a fit |
|---|---|
| AI changes many files and reviewers need risk prioritization | Replacing unit tests, integration tests, or human review |
| Permission, money, state-machine, or product rules need repeated checks | Auto-merging model-generated fixes without review |
| You want a high-confidence PR/CI gate | Teams that cannot tolerate conservative WARNs |
| You want `--intent` to check implementation against requirements/design | Environments without an LLM API key or permission to send code context to a model |

## Why Trust It

| Evidence | What it means |
|---|---|
| Public eval logs | Real PRs, revert gold sets, 45-language samples, large PRs, and intent-review checks are recorded under [`docs/evals/`](docs/evals/) |
| Read-only by default | Except for explicit `--fix` with per-finding confirmation, ReviewGate does not write the worktree or run arbitrary shell commands |
| Conservative gate | Low-confidence findings are folded by default; incomplete reviews, timeouts, and context overflow degrade to WARN |

<details>
<summary><b>How does it review code?</b></summary>

ReviewGate runs multiple agents in parallel, each focused on a review dimension:

| Dimension | Focus |
|---|---|
| security | injection, authorization bypasses, secret leaks, unsafe deserialization |
| perf | N+1 queries, unnecessary copies, hot-path complexity, blocking calls |
| logic | edge cases, null handling, error paths, concurrency races |
| ai_smell | hallucinated APIs, plausible-but-wrong code, assumption drift, unadapted copy/paste |
| style | naming, readability, duplicated code — **off by default** (a quality gate leaves pure style to linters; enable with `--dimensions style`) |
| business | project-specific rules, permission boundaries, state machines, money/order/inventory rules; enabled when `[business].rules` is configured |

> By default review runs the four defect dimensions (security / perf / logic / ai_smell). style/business/intent are opt-in — the gate stays focused on high-risk issues instead of drowning them in style noise.

Then it applies:

1. **Line anchoring and validation**: agents report annotated line numbers; ReviewGate validates and relocates them with code anchors to reduce line drift.
2. **Cross-dimension deduplication**: findings on the same code are merged, and agreement across dimensions increases confidence.
3. **Counter-evidence judge**: each finding is independently checked with evidence before it is kept.
4. **Confidence gate**: high-confidence issues can block, while low-confidence noise is folded by default and can still be inspected.

Read-only tool boundaries, prompt-cache reuse, deterministic duplicate-function detection, and wall-clock timeout fallbacks are covered below.

</details>

## Install Options

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex
```

If you prefer not to execute a remote script directly, download and inspect `install.sh` / `install.ps1` first, or manually download the binary for your platform from GitHub Releases.

From source:

```bash
cargo install --path crates/cli
```

Windows needs Visual Studio Build Tools to compile tree-sitter dependencies.

To upgrade later, just re-run the install command above—it always fetches the latest release and overwrites the old binary (or run `reviewgate upgrade`).

## Configuration

ReviewGate does not lock you into a model. Use any OpenAI-compatible or Anthropic endpoint that matches your team's cost, latency, and context-window needs.

**Minimal config** needs just one provider (everything else has defaults):

```toml
provider = "deepseek"

[providers.deepseek]
protocol = "openai"          # OpenAI-compatible (DeepSeek/Kimi/GLM/Qwen…); use "anthropic" for Anthropic
base_url = "https://api.deepseek.com/v1"
model    = "deepseek-v4-pro"
# api_key = ""               # optional; prefer REVIEWGATE_API_KEY
```

<details>
<summary><b>Optional: gate thresholds · business rules · org skills · config location</b></summary>

```toml
[gate]
block_threshold = 0.8        # confidence ≥ 0.8 blocks the merge
warn_threshold  = 0.5        # ≥ 0.5 warns; lower is folded by default

# Project business rules: enables the `business` dimension; findings tagged [B1].. for traceability
[business]
rules = [
  "Money fields must use integer cents, not float",
  "User-owned resources must check owner_id",
]
# rules_dir  = ".reviewgate/rules"  # <lang>.md injected per changed language; business.md etc. always injected
# skills_dir = ".claude/skills"     # reuse existing org review skills (frontmatter stripped)
```

- **Config discovery order** (first match wins): `REVIEWGATE_CONFIG` path → `./reviewgate.toml` (project override) → `~/.reviewgate/config.toml` (global default).
- **CI key injection**: use `REVIEWGATE_API_KEY` to avoid committing secrets (`REVIEWGATE_BASE_URL` / `REVIEWGATE_MODEL` also supported).
- **Reuse org skills**: `skills_dir` supports nested `<dir>/SKILL.md` and flat `*.md`; can combine with `rules_dir` (plain rule md).

</details>

## Ways To Use It

ReviewGate has one core engine and several wrappers, all of which just call the same `reviewgate` CLI. **CLI is primary and the GitHub Action is for PR/CI** — both are exercised in real use. **The Claude Code Skill, Codex, and AtomCode are thinner agent-instruction shells (experimental)**: calibrated to the current JSON schema, but less battle-tested than the first two.

### CLI

```bash
reviewgate review                       # review current worktree changes
reviewgate review --from main --to HEAD # review this branch against main
reviewgate review --intent spec.md      # check implementation against requirements/design
reviewgate review --format json         # machine-readable output
reviewgate review --fail-on block       # exit 1 on BLOCK, useful for CI
```

<details>
<summary><b>More CLI options</b></summary>

```bash
reviewgate review --dimensions security,logic
reviewgate review --no-judge
reviewgate review --show-filtered
reviewgate review --timeout 120
reviewgate review --samples 3
reviewgate review --fix                   # apply suggestions after per-finding y/N (acts on whatever this review covers)
reviewgate review --fix-all               # apply all fixes without per-finding prompts (works non-interactively, for CI/scripts)
reviewgate review --fix-all --fix-branch  # add --fix-branch (works with --fix or --fix-all): apply on a new branch (optionally named), keeping the current one clean
reviewgate review --commit HEAD --fix     # review the committed change and apply fixes (see note below)
reviewgate review --judge-concurrency 4
reviewgate review --fanout-concurrency 6
reviewgate review --verbose
reviewgate review --commit <sha>
reviewgate review --commit <sha> --intent-from-commit
```

> **Note: `--fix` / `--fix-all` only act on the diff this review covers.** With no range, review defaults to your **uncommitted working-tree changes** (`git diff HEAD`) — if the change is already committed and the working tree is clean, `--fix` will report "no changes / no applicable fixes". To fix **committed** changes, pass a range, e.g. `reviewgate review --commit HEAD --fix` or `reviewgate review --from main --to HEAD --fix`.

</details>

### Intent / Technical Review (`--intent`)

Defect review does not need to know "what this change was supposed to do"; **technical review does**. Pass this change's intent (requirement/design/acceptance criteria, as a file or `-` to read stdin) and ReviewGate runs an **additional, independent holistic agent**: starting from the diff, it actively follows callers, contracts, and tests across files to judge whether the implementation completely and correctly satisfies the intent, then emits an **acceptance checklist** (each criterion marked ✓ met / ✗ missing / ✗ breaking / ⚠ deviation / • suggestion). The intent is **split into N acceptance criteria (C1..CN) checked one by one**; any criterion not individually adjudicated falls back to `? not assessed` (so the checklist is never empty), and any unassessed criterion **degrades the result to WARN** rather than a fake PASS. It is orthogonal to the always-on `business.rules`: rules are invariants, while `--intent` is the per-change "what should this one do". Zero overhead when `--intent` is not passed.

```bash
reviewgate review --from main --to HEAD --intent docs/requirement.md
```

`--exec-verify` lets the model generate self-contained JS/Python snippets and run them locally to verify edge cases. It is off by default. The current isolation is weak: a temporary directory, empty environment, and timeout, not an OS-level sandbox. Use it only in trusted or isolated CI environments.

**Output language**: affects the **finding text** (issue descriptions / fix suggestions) **and the whole report chrome** (section headers like `MUST FIX`/`NEXT STEPS`, status words `PASS`/`WARN`/`BLOCK`, the count line, the acceptance checklist, and the live progress line) — all shown in your language under a matching locale, with English fallback for unsupported languages. Command names (`reviewgate review …`), dimension/severity identifiers, and the token-usage line stay English. The language is decided in this order:

1. **`REVIEWGATE_OUTPUT_LANGUAGE`** — explicit, used verbatim (e.g. `"Chinese (Simplified)"`, `"日本語"`).
2. **Terminal locale** — first non-empty of `LC_ALL` > `LC_MESSAGES` > `LANG`, mapped (`zh_CN`→Simplified, `zh_TW`/`zh_HK`/`zh_MO`→Traditional, `ja`, `ko`, `fr`, `de`, `es`, `pt_BR`, `ru`, `it`…).
3. **English fallback** — none of the above, or a `C` / `POSIX` locale.

Only environment variables are read (not git config or repo contents), so CI without a locale defaults to English. Force a language with:

```bash
REVIEWGATE_OUTPUT_LANGUAGE="English" reviewgate review
```

Exit codes for CI: `0` pass · `1` blocked by the gate (per `--fail-on block|warn|never`) · `2` the tool itself errored (config/network/key — not a code problem; CI should retry or alert, not treat it as a must-fix). Invalid `--fail-on` / `--format` values are rejected at parse time (exit 2), never silently coerced to the default.

```bash
REVIEWGATE_API_KEY=$SECRET reviewgate review --timeout 300 --fail-on block
```

Debug commands:

```bash
reviewgate diff
reviewgate tool find_callers '{"symbol":"foo"}'
reviewgate agent --dimension logic
```

### Claude Code Skill

Personal use: copy `integrations/claude-skill/SKILL.md` to `~/.claude/skills/reviewgate/` (then reload Claude Code). **Trigger it explicitly with `/reviewgate`** — a plain "review my changes" may be picked up by Claude Code's built-in generic code-review instead.

Team setup:

```bash
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/claude-skill/install-into-project.sh | sh
```

It creates, without overwriting existing files:

- `.claude/skills/reviewgate/SKILL.md`: shared team skill
- `.reviewgate/rules/business.md`: organization-specific business rules
- `.reviewgate/rules/<language>.md`: language-specific review rules
- `reviewgate.toml`: project config template

ReviewGate also ships built-in language rules for 45 languages. Custom `<language>.md` files can override or extend them. Disable built-in language rules with `[business] builtin_language_rules=false`.

### GitHub Action

Copy `integrations/github-action/example-workflow.yml` into `.github/workflows/`, configure the `REVIEWGATE_API_KEY` repository secret, and ReviewGate can review PRs, post summary comments, and block by confidence threshold.

```yaml
name: ReviewGate
on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: dengmengmian/ReviewGate/integrations/github-action@v0
        env:
          REVIEWGATE_API_KEY: ${{ secrets.REVIEWGATE_API_KEY }}
        with:
          dimensions: all
          fail-on: block
          comment: "true"
```

> **Versioning**: use `@v0` to track compatible 0.x Action updates. The Action downloads the latest CLI by default, so CLI releases usually need no workflow change; for reproducible CI, set `with: { version: "v0.2.0" }` to pin the CLI engine.

> **Intent review (optional)**: with `with: { intent: "auto" }` the Action automatically feeds the **PR title + description** to `--intent`, running an "implementation vs intent" review with an acceptance checklist — exactly the class of issue defect-oriented review can't see (every hunk looks consistent, but the change doesn't do what the PR claims). The more your PR description reads like acceptance criteria, the better it works; vague titles produce "not assessed" items and downgrade to WARN, hence off by default. You can also pass a path to a fixed intent document.

### 4. Codex (AGENTS.md, experimental)

OpenAI Codex CLI reads `AGENTS.md` at the repo root. Merge ReviewGate's usage into it idempotently (existing content is preserved):

```bash
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/codex/install-into-project.sh | sh
```

It appends a ReviewGate section to `./AGENTS.md` and creates `reviewgate.toml` + `.reviewgate/rules/` templates. Then tell Codex to "review my changes with ReviewGate". Same source and JSON schema as the Claude Skill.

### 5. AtomCode (experimental)

[AtomCode](https://github.com/dengmengmian/AtomCode) uses the same `SKILL.md` format as Claude Code and auto-discovers `.atomcode/skills/` and `.claude/skills/` (project and global). Install the project-level skill (the same SKILL.md as claude-skill) in one command:

```bash
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/atomcode/install-into-project.sh | sh
```

It creates `.atomcode/skills/reviewgate/SKILL.md` + `reviewgate.toml` + `.reviewgate/rules/` templates. If you already installed claude-skill, AtomCode auto-discovers `.claude/skills/`, so no separate install is needed.

## Design Details

- Custom agent orchestration and LLM client, with no provider SDK dependency. ReviewGate uses `reqwest` directly and supports OpenAI-compatible and Anthropic protocols.
- Read-only, structured tools instead of arbitrary shell or write access. `confine_path` keeps reads inside the repository.
- Code context retrieval through tree-sitter symbol lookup and function-body extraction, with grep fallback.
- Prompt-cache reuse through shared system prompts and stable diff chunks.

### Extensibility

- **LLM providers**: `LlmClient` trait plus OpenAI-compatible and Anthropic protocols.
- **Code index backends**: `CodeIndex` trait, with `GrepIndex` and `TreeSitterIndex`.
- **Rules**: built-in language rules, built-in path rules (GitHub Actions workflow security, extensionless `Dockerfile`; disable with `builtin_path_rules=false`), glob-targeted `[[business.path_rules]]` (e.g. `migrations/**` → must be reversible), `rules_dir/<language>.md`, `skills_dir`, and inline `[business].rules`.
- **Optional external tools**: `git` is the only hard dependency. Tools such as ripgrep, linters, and type checkers are used only when detected.
- **Execution verification**: `--exec-verify` is opt-in and disabled by default.
- **Thin wrappers**: CLI, Claude Code Skill, and GitHub Action all call the same core engine.

See [`CHANGELOG.md`](CHANGELOG.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Public Evaluations

The results below come from public samples recorded under [`docs/evals/`](docs/evals/) and are not a general accuracy guarantee. The current samples were mainly run with `deepseek-v4-pro`.

| Signal | Current record |
|---|---|
| False BLOCK | No false BLOCK observed in recorded real PRs, clean 45-language samples, and real merged commit samples |
| Revert gold set | Real PR revert gold set **4/4**: axios, requests, gin, and ripgrep |
| Language coverage | **45 built-in language rules** enabled by default; can be disabled or overridden |
| Large PRs | Context overflow, request failure, timeout, and skipped oversized files degrade to WARN |
| Intent review | 10 real correct-fix commits across 5 languages are **10/10 met with 0 false misses** |

See [`docs/evals/`](docs/evals/) for details, [`docs/BIG_PR_HANDLING.md`](docs/BIG_PR_HANDLING.md) for large PR handling, and [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) for known limits.

## Current Status

ReviewGate's core path is ready for real PRs and CI. For shared repositories, start with `WARN` / comment-only mode before making `BLOCK` a required merge gate.

| Status | Notes |
|---|---|
| Ready to use | CLI, Claude Code Skill, GitHub Action, business rules, intent review, and large-PR degradation |
| Default boundary | Review is read-only; `--fix` requires per-finding confirmation; incomplete reviews never silently PASS |
| Still needs support | Does not replace tests or human review; subtle multi-step runtime behavior still needs test coverage |
| Quality checks | CI covers fmt, clippy with `-D warnings`, tests, Ubuntu, and Windows |

See [`CHANGELOG.md`](CHANGELOG.md) for release notes.

## License

[MIT](LICENSE)
