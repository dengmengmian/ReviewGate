<p align="center">
  <img src="docs/assets/logo.svg" alt="ReviewGate" width="420">
</p>

<p align="center">
  Let AI review AI-written code before merge: <b>catch high-risk issues first and reduce low-value review noise</b>
</p>

<p align="center">
  English · <a href="README.md">简体中文</a>
</p>

ReviewGate runs before PRs are merged and gives AI-generated, or AI-heavy, code a second review pass. It does not replace human review. It pre-filters the work for reviewers by promoting high-risk findings and folding low-confidence noise by default.

## Start In 30 Seconds

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

## When To Use It

| Scenario | What ReviewGate does |
|---|---|
| AI changed many files at once | Reviews the diff by security, performance, logic, and other focused dimensions |
| Review comments are noisy or scattered | Deduplicates overlapping findings and folds low-confidence feedback |
| AI code looks plausible but may be wrong | Checks hallucinated APIs, assumption drift, and unadapted copy/paste |
| Your team has business rules | Injects rules for permissions, money, state machines, and domain behavior on every review |
| You want a CI gate | High-confidence issues can block merges, and incomplete reviews do not silently pass |

<details>
<summary><b>How does it review code?</b></summary>

ReviewGate runs multiple agents in parallel, each focused on a review dimension:

| Dimension | Focus |
|---|---|
| security | injection, authorization bypasses, secret leaks, unsafe deserialization |
| perf | N+1 queries, unnecessary copies, hot-path complexity, blocking calls |
| logic | edge cases, null handling, error paths, concurrency races |
| style | naming, readability, duplicated code |
| ai_smell | hallucinated APIs, plausible-but-wrong code, assumption drift, unadapted copy/paste |
| business | project-specific rules, permission boundaries, state machines, money/order/inventory rules; enabled when `[business].rules` is configured |

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

## Quick Start

ReviewGate ships with no built-in model—first get an **OpenAI-compatible or Anthropic LLM endpoint + key** (DeepSeek recommended), then 3 steps:

```bash
# 1) Copy the example config and fill in base_url / api_key / model
cp reviewgate.toml.example reviewgate.toml

# 2) Test connectivity
reviewgate llm test

# 3) Review the current changes in any git repository
reviewgate review
```

> Put the config in `~/.reviewgate/config.toml` to make it **global**—every repo uses it, no per-project copy needed.

## Configuration

**Minimal config** needs just one provider (everything else has defaults):

```toml
provider = "deepseek"

[providers.deepseek]
protocol = "openai"          # OpenAI-compatible (DeepSeek/Kimi/GLM/Qwen…); use "anthropic" for Anthropic
base_url = "https://api.deepseek.com/v1"
api_key  = "sk-..."          # or leave empty and inject via REVIEWGATE_API_KEY (recommended in CI)
model    = "deepseek-v4-pro"
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

ReviewGate has one core engine and three thin wrappers: CLI, Claude Code Skill, and GitHub Action.

### CLI

```bash
reviewgate review                       # review current changes; 5 default dimensions, plus business when configured
reviewgate review --dimensions security,logic
reviewgate review --format json         # machine-readable output
reviewgate review --no-judge            # faster, with more false positives
reviewgate review --show-filtered       # show folded low-confidence findings
reviewgate review --fail-on block       # exit 1 on BLOCK, useful for CI
reviewgate review --timeout 120         # per-dimension wall-clock timeout in seconds
reviewgate review --samples 3           # sample each dimension multiple times and union results
reviewgate review --fix                 # apply suggested code after per-finding y/N confirmation
reviewgate review --judge-concurrency 4 # limit judge concurrency to avoid provider rate limits
reviewgate review --verbose             # print per-dimension rounds and token/cache stats
reviewgate review --commit <sha>        # review one commit; or use --from <base> --to <head>
```

`--exec-verify` lets the model generate self-contained JS/Python snippets and run them locally to verify edge cases. It is off by default. The current isolation is weak: a temporary directory, empty environment, and timeout, not an OS-level sandbox. Use it only in trusted or isolated CI environments.

ReviewGate asks the model to write findings in `REVIEWGATE_OUTPUT_LANGUAGE` or the terminal locale (`LC_ALL`, `LC_MESSAGES`, `LANG`). Example:

```bash
REVIEWGATE_OUTPUT_LANGUAGE="English" reviewgate review
```

Example output:

```text
Gate: BLOCK x blocking merge    1 file changed · 2 trusted findings · 3 filtered

handler.rs
  x [security · high · conf 1.00] L3
    SQL injection: req.user_id is interpolated directly into a DELETE statement...
    -> Suggestion: use parameterized queries.
```

Exit codes for CI: `BLOCK -> 1`, otherwise `0`. Adjust with `--fail-on block|warn|never`.

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

Personal use: copy `integrations/claude-skill/SKILL.md` to `~/.claude/skills/reviewgate/`, then ask Claude Code to review your changes.

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

## Why It Is Trustworthy

- Custom agent orchestration and LLM client, with no provider SDK dependency. ReviewGate uses `reqwest` directly and supports OpenAI-compatible and Anthropic protocols.
- Read-only, structured tools instead of arbitrary shell or write access. `confine_path` keeps reads inside the repository.
- Code context retrieval through tree-sitter symbol lookup and function-body extraction, with grep fallback.
- Prompt-cache reuse through shared system prompts and stable diff chunks.

### Extensibility

- **LLM providers**: `LlmClient` trait plus OpenAI-compatible and Anthropic protocols.
- **Code index backends**: `CodeIndex` trait, with `GrepIndex` and `TreeSitterIndex`.
- **Rules**: built-in language rules, `rules_dir/<language>.md`, `skills_dir`, and inline `[business].rules`.
- **Optional external tools**: `git` is the only hard dependency. Tools such as ripgrep, linters, and type checkers are used only when detected.
- **Execution verification**: `--exec-verify` is opt-in and disabled by default.
- **Thin wrappers**: CLI, Claude Code Skill, and GitHub Action all call the same core engine.

See [`CHANGELOG.md`](CHANGELOG.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Public Evaluations

The results below come from public samples recorded under [`docs/evals/`](docs/evals/) and are not a general accuracy guarantee. The current samples were mainly run with `deepseek-v4-pro`.

- **Precision**: no false BLOCK was observed in the recorded real PRs, clean 45-language samples, and real merged commit samples. Suspected false positives are kept with investigation notes in the eval logs.
- **Recall**: real CVE reverts, about 18 vulnerability classes, real user issues, and synthetic strong triggers are covered. The real PR revert gold set is 4/4: axios prototype-pollution SSRF, requests Content-Type parsing, gin ClientIP XFF, and ripgrep gitignore cache.
- **Languages**: 45 built-in language rules are enabled by default and can be disabled or overridden.
- **Large PRs / incomplete review**: context overflow, request failure, timeout, and skipped oversized files degrade to WARN and can make CI exit non-zero instead of silently passing.
- **Known limits**: subtle multi-step arithmetic and carry/rounding off-by-one bugs remain a hard tail for static LLM review. See [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md).

## Current Status

Beta. The core path is complete: parallel dimensions, counter-evidence judge, confidence gate, business rules, built-in rules for 45 languages, duplicate detection, multi-sampling, `--fix` anchor validation, reachability grading, incomplete-review handling, CLI, Skill, and Action.

CI covers fmt, clippy with `-D warnings`, tests, Windows, and Ubuntu.

## License

[MIT](LICENSE)
