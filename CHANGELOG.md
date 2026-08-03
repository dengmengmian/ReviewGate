# Changelog

本项目变更记录。格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循 SemVer。
每条变更先中文、后英文。
Changes are listed in Chinese first, then English.

## [0.10.0] - 2026-08-03

### Added
- 新增 **Issue 分诊**（`reviewgate issue …`）：帮维护者过一遍提上来的 Issue——分类（缺陷/需求/文档/提问/安全/广告，中英文都认）、查重（全文检索 + 错误签名 + 语义向量）、可选的代码验证（拉本地仓库把报错对到源码行、展开所在函数、找该文件的历史修复），最后写成一条按类型措辞的回复。安全报告不会被要求"贴日志、升级重试"，文档诉求不会被问"复现步骤"。支持 GitHub / GitLab / Gitee / AtomGit。
  Added **issue triage** (`reviewgate issue …`): a first pass over incoming issues — classification (defect / request / docs / question / security / advertisement, in English and Chinese), duplicate detection (full-text, error signature, semantic vector), optional verification against a local checkout (matching the reported error to a source line, expanding the enclosing function, finding prior fixes to that file), and a reply worded per issue type. Vulnerability reports are never asked to "paste logs and retry on the latest version"; documentation requests are never asked for reproduction steps. Works with GitHub, GitLab, Gitee, and AtomGit.
- Issue 分诊的**人工兜底**：结论没把握时不发结论、不关单子；配了处理人就改发一条移交评论、打 `needs-triage` 标签并指派给他。没配处理人时被拦下的单子也不会消失，`reviewgate issue stats --gated` 能列出有哪几条在等人接手。
  **Human fallback** for issue triage: when confidence is low nothing is concluded and nothing is closed; with a triage owner configured it posts a hand-off comment, adds `needs-triage`, and assigns the issue. Gated issues are never lost — `reviewgate issue stats --gated` lists exactly which ones are waiting for a person.
- Issue 分诊支持 **Webhook 与常驻模式**：`reviewgate serve` 收事件入队，`reviewgate issue watch` 轮询新单，`reviewgate daemon --serve` 两者一起跑。
  **Webhook and long-running modes** for issue triage: `reviewgate serve` queues incoming events, `reviewgate issue watch` polls for new issues, and `reviewgate daemon --serve` runs both together.
- Issue 分诊的写操作**默认全关**：打标签、指派、关闭都要显式开启；`close_spam` 可以只自动关广告，不必为此打开能关任意 Issue 的总开关。
  Every **write action is off by default** in issue triage: labels, assignment, and closing must each be enabled; `close_spam` closes advertisements only, so you don't have to enable the switch that can close anything.
- 新增**审查范围排除**：内置默认跳过 lock 文件、vendored 依赖、生成代码、压缩产物与二进制；可用仓库根 `.reviewgateignore`（gitignore 语法）或配置 `[exclude] patterns` 增删。被排除的文件会带原因出现在文本报告、JSON `excluded` 与 PR 评论里；全部文件被排除时明说"全被排除"，不会伪装成"没有改动"。
  Added **review-scope exclusion**: lock files, vendored dependencies, generated code, bundles, and binaries are skipped by default; extend or override via a repo-root `.reviewgateignore` (gitignore syntax) or `[exclude] patterns`. Excluded files are reported with their reason in the text report, in JSON (`excluded`), and in the PR comment; if everything is excluded the report says so instead of claiming "no changes".
- 新增 **`reviewgate findings list/show/resolve`**：每次审查把结果落进 `.reviewgate/cache/findings.json`，agent 可逐条读取、标记已处理，不必为下一条问题重跑整轮审查。输出始终带 `decision` 与 `incomplete`，空列表不会被误读成"没问题"。
  Added **`reviewgate findings list/show/resolve`**: each review saves its results to `.reviewgate/cache/findings.json` so an agent can work through issues one by one instead of re-running the whole review. Output always carries `decision` and `incomplete`, so an empty list is never mistaken for "all clear".
- `--comment` 本地可用：CI 变量解析不出上下文时，回退到已认证的 `gh` / `glab` 取仓库、PR/MR 号与 token，本地不必再单独配一份 token。CI 行为不变（环境变量始终优先）。
  `--comment` now works locally: when CI variables don't resolve a context, it falls back to your authenticated `gh` / `glab` for repo, PR/MR number, and token. CI behaviour is unchanged (environment variables always win).
  安全约束：只把 token 发给 `gh`/`glab` **确实登录过**的主机，且取 token 时按主机名精确指定，避免多主机登录下拿错 token 或被伪造远端诱导外发。
  Security constraints: the token is only sent to hosts `gh`/`glab` is actually authenticated to, and it is looked up per hostname, so a crafted remote or a multi-host login cannot redirect it.
- 新增**严重度标签自定义**（`[[severity_labels]]`）：改显示名与配色，并可写下本项目对每档的定义——定义会注入审查 prompt，直接影响模型怎么分级。
  Added **custom severity labels** (`[[severity_labels]]`): change the display name and color, and define what each level means for your project — the definition is injected into the review prompt and steers how the model grades.
- 新增 `--since-last-review`：只审上次审查之后新增的部分（新提交 + 未提交编辑）。找不到上次审查、上次没记基准、或基准提交已被 rebase/force-push 冲掉时**直接报错**，绝不悄悄退回全量或更小范围。
  Added `--since-last-review`: review only what changed since the previous review (new commits + uncommitted edits). If there is no previous review, no recorded base, or the base commit is gone (rebase/force-push), it **errors out** instead of silently falling back to a different scope.
- 新增 `--with-pr-discussion`：把 PR/MR 上已有的人类评审讨论作为上下文喂给审查，避免把别人提过的点当新发现重复报（目前支持 GitHub）。只做上下文注入，不会因此隐藏任何发现。
  Added `--with-pr-discussion`: feeds the PR/MR's existing human review discussion into the review as context so already-raised points aren't re-reported (GitHub for now). Context injection only — no finding is ever hidden because of it.
- 报告、JSON（`scope`）与 PR 评论现在都写明**本次审查覆盖的范围**（如 `main...HEAD`、`since last review (…)`）——增量审查的 PASS 不该被读成整个 PR 通过。
  The report, JSON (`scope`), and PR comment now state **what range was reviewed** (e.g. `main...HEAD`, `since last review (…)`) — a PASS on an incremental review must not read as a PASS on the whole PR.
- `reviewgate findings` 支持短序号：`findings show 3` / `findings resolve 3`，对话里引用比 12 位指纹方便（指纹仍可用，且跨运行稳定）。
  `reviewgate findings` accepts short sequence numbers: `findings show 3` / `findings resolve 3` — easier to reference in conversation than a 12-char fingerprint (fingerprints still work and remain stable across runs).

### Fixed
- **`reviewgate daemon --serve` 不再用写死的默认 webhook secret 启动**：此前没配 `--webhook-secret` / `REVIEWGATE_WEBHOOK_SECRET` 时会回退到源码里公开的常量，等于签名校验作废，任何人都能伪造事件驱动 Issue 的评论/标签/关闭/指派。现在与 `reviewgate serve` 一致：缺 secret 直接报错退出。
  **`reviewgate daemon --serve` no longer starts with a hardcoded default webhook secret**: without `--webhook-secret` / `REVIEWGATE_WEBHOOK_SECRET` it used to fall back to a constant published in the source, which voided signature verification — anyone could forge events and drive issue comments/labels/closes/assignments. It now matches `reviewgate serve` and exits with an error instead.
- PR/MR 摘要评论此前散文部分写死中文，与终端报告跟随 `REVIEWGATE_OUTPUT_LANGUAGE` 的约定不一致；现已统一（维度名、severity、路径等技术标识保持英文）。
  The PR/MR summary comment had its prose hardcoded in Chinese while the terminal report follows `REVIEWGATE_OUTPUT_LANGUAGE`; both now follow the same setting (dimension names, severities, and paths stay English).

### Changed
- `reviewgate upgrade` 支持指定版本（`reviewgate upgrade 0.8.0`）以回退到已知好版本；若二进制由 Homebrew / Cargo / mise / Nix 安装则不再直接覆盖，改为提示用对应包管理器升级（`--force` 可强制）。
  `reviewgate upgrade` accepts a version (`reviewgate upgrade 0.8.0`) for rolling back to a known-good build, and no longer overwrites binaries installed by Homebrew / Cargo / mise / Nix — it points you at that package manager instead (`--force` overrides).

## [0.9.0] - 2026-07-30

### Added
- 新增 **`reviewgate init`**：交互/非交互写出全局配置（provider 预设 deepseek/openai/anthropic/custom）；密钥走环境变量，不写进文件；支持 `--force` / `--config-dir` / `--test`。
  Added **`reviewgate init`**: interactive or non-interactive global config (deepseek/openai/anthropic/custom presets); API key stays in the environment; supports `--force` / `--config-dir` / `--test`.
- 新增 **`reviewgate demo`**：内置 SQL 注入样例仓库，验证闸口会 BLOCK（`--prepare-only` 只建仓不调 LLM）。
  Added **`reviewgate demo`**: built-in poisoned SQL-injection fixture to verify the gate BLOCKs (`--prepare-only` seeds without LLM).
- 新增 **`--profile gate|audit`**：gate 默认严闸口；audit 更宽（samples≥2、默认含 style）。
  Added **`--profile gate|audit`**: gate is the default precision profile; audit is wider (samples≥2, style on by default).
- 新增跑前成本估算与预算：`[cost]` 行、`--estimate-only`、`--max-cost`、`--max-input-tokens`；可选 `price_per_mtok_*` 换 USD。
  Added pre-run cost estimate and budgets: `[cost]` line, `--estimate-only`, `--max-cost`, `--max-input-tokens`; optional `price_per_mtok_*` for USD.
- 新增大 PR 合成报告：`unit_plan`（目录装箱 unit 清单）+ `coverage`（covered/unfinished/oversized 路径与建议）；文本与 JSON 均输出；干净单 unit 不刷假「未覆盖」。
  Added large-PR composite report: `unit_plan` (directory-packed units) + `coverage` (covered/unfinished/oversized paths and advice) in text and JSON; clean single-unit runs do not invent fake gaps.
- 新增关键路径 incomplete 策略：`force_fail_incomplete_paths`（默认 auth/payment/… glob；`[]` 关闭）。
  Added critical-path incomplete policy: `force_fail_incomplete_paths` (default auth/payment/… globs; `[]` disables).
- 新增运行指标落盘：`.reviewgate/cache/metrics.jsonl`（`--no-metrics` 可关）。
  Added run metrics append to `.reviewgate/cache/metrics.jsonl` (disable with `--no-metrics`).
- GitHub 行内评论改为只发 **high 或 ≥ block 置信度** 的已定位发现，并使用配置的 `gate.block_threshold`。
  GitHub inline comments now post only **high or ≥ block-threshold** located findings, using configured `gate.block_threshold`.

### Fixed
- `post_inline_suggestions` 不再写死 0.8，与闸口阈值一致。
  `post_inline_suggestions` no longer hardcodes 0.8; matches gate threshold.
- `home_dir` 在 core 导出，cli `init` 复用，去掉复制。
  `home_dir` is exported from core and reused by CLI `init` (no duplicate copy).
- 关键路径 incomplete 判定逻辑去重整理，去掉死分支。
  Critical-path incomplete logic cleaned up (removed dead branches).

### Docs
- README 冷启动路径改为 init → demo → review；补充 profile / 成本 / 大 PR 覆盖说明。
  README cold-start path is init → demo → review; documents profile / cost / large-PR coverage.
- `docs/BIG_PR_HANDLING.md` 补充用户可见 unit/coverage 报告。
  `docs/BIG_PR_HANDLING.md` documents the user-visible unit/coverage report.
- 公开评测汇总：`docs/evals/2026-07-30__github-10-repos-batch.md`（10 仓 review+security）。
  Public batch eval: `docs/evals/2026-07-30__github-10-repos-batch.md` (10-repo review+security).

## [0.8.0] - 2026-07-26

### Added
- 新增 **`reviewgate security`**：安全深审入口（同一引擎、`Dimension::Security`，非新维度）。默认仅 security、samples≥2、sink 驱动 + 强制追源 checklist、确定性密钥/凭证预检、未审完永不 PASS。日常 `review` 仍为标准四维，不付深审成本。确定性预检发现在证伪 Judge **之后**合并，避免被 judge 误杀。
  Added **`reviewgate security`**: a security deep-review entry (same engine, `Dimension::Security`, no new dimension). Defaults: security-only, samples≥2, sink-driven + mandatory taint-trace checklist, deterministic secret/credential precheck, incomplete never PASS. Default `review` stays the standard four defect dimensions without deep cost. Deterministic precheck findings are merged **after** the counter-evidence judge so they cannot be false-negatived.

## [0.7.2] - 2026-07-11

### Changed
- tokio 从 `full` 特性改为按需引入（`rt-multi-thread, macros, fs, process, time, sync`），减少编译时间与二进制体积。
  tokio switched from `full` feature to only the needed features (`rt-multi-thread, macros, fs, process, time, sync`), reducing compile time and binary size.
- tree-sitter 各语言解析器改为可选的 Cargo feature（默认全部开启），可按需裁剪不需要的语言以进一步减小二进制体积。
  Tree-sitter language parsers are now optional Cargo features (all enabled by default), allowing users to drop unneeded languages for a smaller binary.
- 审查 prompt 在 Agent 运行与维度 fan-out 间改用 `Arc<String>` 共享，避免多次 clone 同一份大 prompt 的分配开销。
  Review prompts now use `Arc<String>` across the agent runtime and dimension fan-out to avoid redundant allocations from cloning the same large prompt.

## [0.7.1] - 2026-07-06

### Fixed
- 安装与自更新现在会下载 release 附带的 `sha256sum.txt`，并在替换二进制前校验当前平台资产的 SHA-256；校验文件缺失、资产缺失或 hash 不匹配时直接失败，不再执行未校验的下载产物。
  Install and self-update now download the release `sha256sum.txt` and verify the current platform asset's SHA-256 before replacing the binary; missing checksum files, missing asset entries, or hash mismatches fail closed instead of executing an unverified download.
- GitHub Action 的 `reviewgate review` 参数从字符串拼接改为 bash array 传参，避免包含空格的 intent 路径或误配置输入被 shell 重新拆词成额外 CLI 参数。
  The GitHub Action now invokes `reviewgate review` with a bash array instead of string-concatenated arguments, preventing paths with spaces or misconfigured inputs from being re-split by the shell into extra CLI arguments.

## [0.7.0] - 2026-07-06

### Fixed
- LLM 客户端:响应**读取 body 失败**（慢响应超时、连接被重置）此前被 `unwrap_or_default` 吞成空字符串，导致——① 瞬时错误**不重试**、直接判该维度失败；② 上层报出误导的「failed to parse LLM response（空）」，掩盖真正原因（网络/超时，而非模型输出坏）。现在读 body 失败被当作可重试错误正常重试，并给出清晰错误信息。对**推理型模型**（响应较慢、后期大上下文轮次更慢）尤其明显——此前会零星出现「维度未审完、0 发现」，实为 body 读取瞬时失败被误吞。
  LLM client: a **failure to read the response body** (slow-response timeout, connection reset) was previously swallowed into an empty string by `unwrap_or_default`, which — (1) skipped retry and failed that dimension outright, and (2) surfaced a misleading "failed to parse LLM response (empty)" that hid the real cause (network/timeout, not bad model output). Body-read failures are now treated as retryable and retried, with a clear error message. This especially affected **reasoning models** (slower responses, slower still on large later-round contexts), which previously showed sporadic "dimension incomplete, 0 findings" that were actually swallowed transient body-read failures.

### Added
- 新增**全仓符号索引** `reviewgate index build`（opt-in）：预扫整库、tree-sitter 提取所有符号定义，持久化到 `.reviewgate/cache/symbols.json`。之后审查时 `find_definition` 从"每次 git grep + 解析候选文件"变成**全仓完整查表**——Agent 追跨文件定义更快更全，不再受候选文件截断影响。纯本地、无外部依赖、无嵌入；建了自动用、没建则回退现有按需检索（优雅降级，索引非必需）。索引存 `.reviewgate/cache/`（自带 `.gitignore`）。**陈旧安全**：命中项会校验位置是否仍成立（读该行比对建库内容），定义被移动/删除的陈旧项校验失败、安全回退按需；新增符号本就是 miss 回退——所以陈旧索引**既不漏、也不给过时位置**；审查发现 `HEAD` 已变会提示重建。
  Added an **opt-in whole-repo symbol index** `reviewgate index build`: it pre-scans the repository, extracts every symbol definition with tree-sitter, and persists them to `.reviewgate/cache/symbols.json`. During review `find_definition` then becomes a **complete whole-repo table lookup** instead of a per-query git grep + candidate-file parse — the agent follows cross-file definitions faster and more completely, no longer bounded by candidate-file truncation. It is local-only, dependency-free, and embedding-free; used automatically when present, with graceful fallback to the existing on-demand lookup when absent. The index lives in `.reviewgate/cache/` (self-`.gitignore`d). **Stale-safe**: each hit is validated against the current file (re-reading the line), so entries whose definition moved or was deleted fail validation and safely fall back, and newly added symbols miss and fall back too — a stale index neither causes a miss nor returns an outdated location; review hints you to rebuild when `HEAD` has changed.
- **PR/MR 摘要评论新增 GitLab 与 AtomGit 支持**（此前仅 GitHub）：`--comment` 现在按环境自动识别平台并把审查摘要发到对应 PR/MR。GitHub Actions（`GITHUB_*`）、GitLab CI（`CI_PROJECT_ID`/`CI_MERGE_REQUEST_IID`/`CI_API_V4_URL` + `GITLAB_TOKEN`）自动识别；AtomGit 及任意平台用 `REVIEWGATE_FORGE=atomgit|github|gitlab` + `REVIEWGATE_REPO` + `REVIEWGATE_PR` + `REVIEWGATE_TOKEN` 显式指定。行内 suggestion 评论目前仍仅 GitHub。内部把原 `github` 模块重构为平台无关的 `forge`。
  **PR/MR summary comments now support GitLab and AtomGit** (previously GitHub only): `--comment` auto-detects the platform from the environment and posts the review summary to the corresponding PR/MR. GitHub Actions (`GITHUB_*`) and GitLab CI (`CI_PROJECT_ID`/`CI_MERGE_REQUEST_IID`/`CI_API_V4_URL` + `GITLAB_TOKEN`) are auto-detected; AtomGit and any other platform are configured explicitly via `REVIEWGATE_FORGE=atomgit|github|gitlab` + `REVIEWGATE_REPO` + `REVIEWGATE_PR` + `REVIEWGATE_TOKEN`. Inline suggestion comments remain GitHub-only. Internally the `github` module was refactored into a platform-agnostic `forge` module.
- 新增**增量复审** `--incremental`（opt-in）：按**文件**缓存发现，只重审自上次以来 diff 变化的文件，未变文件直接复用缓存、跳过最贵的 LLM fan-out——迭代 PR（追加 commit）直接省 token 和墙钟。缓存键含"评审签名"（维度/模型/规则/采样/exec_verify），任一变化即整体失效，绝不复用过期结果；缓存存 `.reviewgate/cache/`（自带 `.gitignore`，永不进 review）。这是拿覆盖度换成本的取舍，默认关闭，边界见 `docs/LIMITATIONS.md`。
  Added **incremental review** `--incremental` (opt-in): caches findings per **file** and only re-reviews files whose diff changed since the last run, reusing cached findings for unchanged files and skipping the most expensive LLM fan-out — saving tokens and wall-clock on iterative PRs (follow-up commits). The cache key includes a "review signature" (dimensions/model/rules/samples/exec_verify); any change invalidates the whole cache so stale results are never reused. The cache lives in `.reviewgate/cache/` (self-`.gitignore`d, never reviewed). It trades coverage for cost, is off by default, and its limits are documented in `docs/LIMITATIONS.md`.
- 新增 **pre-commit 钩子清单**（根目录 `.pre-commit-hooks.yaml`）：用 [pre-commit](https://pre-commit.com/) 的项目可一行接入 ReviewGate 作为提交前闸口（`repo: .../ReviewGate` + `id: reviewgate`），高置信问题 `BLOCK` 时 `git commit` 失败。走 `language: system` 调用已安装的 `reviewgate`，不在每台机器上从源码编译。
  Added a **pre-commit hook manifest** (repo-root `.pre-commit-hooks.yaml`): projects using [pre-commit](https://pre-commit.com/) can wire ReviewGate as a pre-commit gate in one block (`repo: .../ReviewGate` + `id: reviewgate`), failing `git commit` when a high-confidence issue `BLOCK`s. It uses `language: system` to call the installed `reviewgate` rather than compiling from source on every machine.
- 新增**误报抑制**：团队确认某条发现是误报后，把它的**指纹**写进仓库根的 `.reviewgate/ignore`（提交后全队共享），下次审查命中同一指纹的发现会被折叠、不再计入闸口（`BLOCK`/`WARN` 降级），但仍以已过滤状态保留、可 `--show-filtered` 展开审计——**不静默删除**。指纹随每条发现打印在文本与 JSON 输出里（`fp <hash>` / `"fingerprint"`），复制即可。指纹按 `路径 + 维度 + 归一化代码` 计算、**不含行号**，所以后续改动导致行号漂移后同一误报仍被抑制。
  Added **false-positive suppression**: once a team confirms a finding is a false positive, put its **fingerprint** into the repo-root `.reviewgate/ignore` (committed, shared across the team); on the next review any finding matching that fingerprint is folded and excluded from the gate (`BLOCK`/`WARN` downgrade), yet still kept as filtered and inspectable via `--show-filtered` — **never silently dropped**. The fingerprint is printed alongside every finding in both text and JSON output (`fp <hash>` / `"fingerprint"`), ready to copy. It is computed from `path + dimension + normalized code` and **excludes line numbers**, so the same false positive stays suppressed even after later edits shift its lines.

## [0.6.1] - 2026-07-04

### Fixed
- 去重：同一处问题被不同维度锚定在**相邻但不同的行**时（如 logic 报 423-429、ai_smell 报 426-429），现在会正确合并为一条，不再重复上报。此前仅按精确起始行分组会漏合，导致同一缺陷显示两次、并虚增发现数。合并要求行区间重叠**且**代码内容重合，避免误合相邻的不同问题。
  Deduplication: when the same issue is anchored by different dimensions on **adjacent-but-different lines** (e.g. logic at 423-429, ai_smell at 426-429), the findings are now correctly merged into one instead of double-reported. The previous exact-start-line grouping missed these, showing the same defect twice and inflating the finding count. Merging requires both overlapping line ranges **and** shared code content, so distinct adjacent issues are not wrongly merged.
- skill/规则的 frontmatter 里 `name:` 或 `description:` 留空时，不再被解析成空字符串，而是正确视为「未设置」——避免空标题/空描述被当成有效值注入评审提示。
  When a skill/rule frontmatter leaves `name:` or `description:` empty, it is no longer parsed as an empty string but correctly treated as unset — preventing blank titles/descriptions from being injected into review prompts as if valid.

### Changed
- 大幅补齐单元测试覆盖（评审去重、行号重定位、diff 解析、渲染、LLM 客户端、工具分发等 40+ 模块），提升重构与发版的回归安全网。纯内部改动，不影响使用行为。
  Substantially expanded unit-test coverage (review dedup, line relocation, diff parsing, rendering, LLM clients, tool dispatch, and 40+ modules), strengthening the regression safety net for refactors and releases. Internal only, no behavior change.

## [0.6.0] - 2026-07-03

### Changed
- 默认审查维度由 5 个收敛为 4 个缺陷维度（security / perf / logic / ai_smell）；**`style` 移出默认集、改为 opt-in**（`--dimensions style` 或 `...,style`）。作为合并前质量闸口，纯风格/格式问题属噪声、该交给 linter/formatter——在 AACR-Bench 官方语义评测里 style 命中真缺陷≈0 却把精度从 57% 拉低到 33%。默认少一个维度也更快更省；需要风格审查照旧可显式开启。
  The default review dimension set shrank from 5 to the 4 defect dimensions (security / perf / logic / ai_smell); **`style` moved out of the default set to opt-in** (`--dimensions style` or `...,style`). As a pre-merge quality gate, pure style/formatting is noise best left to linters/formatters — on the official AACR-Bench semantic eval, style matched ≈0 real defects yet dragged precision from 57% down to 33%. One fewer default dimension is also faster and cheaper; style review is unchanged when explicitly enabled.

### Added
- 新增**路径规则**：`[[business.path_rules]]` 用 glob 把定向规则路由到改动文件（如 `migrations/**` → 迁移必须可回滚），命中才注入、带 `[P1]` 编号可追溯；非法 glob 在加载时告警而非静默忽略。另附两组**内置路径规则**（默认开，`builtin_path_rules = false` 可关）：`.github/workflows/*` 命中 GitHub Actions 安全清单（`pull_request_target`+PR head 检出、`${{ }}` 注入、密钥外泄、过宽权限、未钉死的第三方 action）；无扩展名的 `Dockerfile` 现在也能命中镜像规则（此前按扩展名路由会漏掉）。
  Added **path rules**: `[[business.path_rules]]` routes targeted rules to changed files by glob (e.g. `migrations/**` → migrations must be reversible), injected only on match with traceable `[P1]` ids; invalid globs warn at load instead of being silently ignored. Two **built-in path rules** ship enabled by default (disable with `builtin_path_rules = false`): `.github/workflows/*` triggers a GitHub Actions security checklist (`pull_request_target` + PR-head checkout, `${{ }}` injection, secret exposure, over-broad permissions, unpinned third-party actions), and extensionless `Dockerfile` files now get the image rules (previously missed by extension-based routing).
- 新增两个安装渠道：`brew install dengmengmian/tap/reviewgate` 与 `cargo install reviewgate`（cli crate 由 `reviewgate-cli` 更名为 `reviewgate` 并发布到 crates.io）；GitHub Action 另有独立仓库 [dengmengmian/reviewgate-action](https://github.com/dengmengmian/reviewgate-action) 供 Marketplace 使用。
  Two new install channels: `brew install dengmengmian/tap/reviewgate` and `cargo install reviewgate` (the CLI crate was renamed from `reviewgate-cli` to `reviewgate` and published to crates.io); the GitHub Action also ships as a standalone repo [dengmengmian/reviewgate-action](https://github.com/dengmengmian/reviewgate-action) for the Marketplace.

## [0.5.0] - 2026-07-02

### Added
- GitHub Action 新增 `intent` 输入：`intent: "auto"` 自动把 PR 标题+描述作为 `--intent` 做「实现 vs 意图」评审（也可传固定意图文档路径）。用于覆盖「每个 hunk 都自洽、但整体没做到 PR 声称的事」这类缺陷向审查抓不到的问题。默认关闭：标题含糊会产生「未核对」项并降级 WARN。
  The GitHub Action gained an `intent` input: `intent: "auto"` automatically feeds the PR title + description to `--intent` for an "implementation vs intent" review (a fixed intent-document path also works). It covers the class of issue defect-oriented review can't see — every hunk looks consistent, but the change doesn't do what the PR claims. Off by default: vague titles produce "not assessed" items and downgrade to WARN.
- TypeScript 换用专用语法解析：`interface` / `type` 别名 / `enum` / `abstract class` 现在能被 `find_definition` 等精确工具识别为定义（此前 TS 复用 JS 语法，这些构造会被漏掉或解析错位）；`.tsx` 用 JSX 感知的语法解析。
  TypeScript now uses its dedicated grammar: `interface` / `type` aliases / `enum` / `abstract class` are recognized as definitions by `find_definition` and friends (previously TS reused the JS grammar, which missed or mis-parsed these constructs); `.tsx` is parsed with the JSX-aware grammar.

### Changed
- 评审开始前，改动符号的调用点现在会被本地预取（git/AST，毫秒级）并随 diff 一起提供给评审 Agent——模型开局就能看到「谁在用这段被改的代码」，省掉最贵的取数往返。跨维度共享同一块、可被 prompt 缓存摊薄；有严格上限防注意力稀释；若因它超输入预算会自动退回无预取版本。召回评测无回归（date-fns off-by-one 用例从 warn 升到 block）；墙钟收益受服务商延迟噪声影响，未定量宣称。
  Call sites of changed symbols are now prefetched locally (git/AST, milliseconds) and provided to review agents alongside the diff — the model sees "who uses this changed code" from turn one, skipping its most expensive lookup round-trips. The block is shared across dimensions (prompt-cache friendly), strictly capped to avoid attention dilution, and automatically dropped if it would exceed the input budget. Recall evals show no regression (the date-fns off-by-one case moved from warn to block); wall-clock gains are not quantified due to provider latency noise.
- 评审 Agent 现在会把互不依赖的查询（读多个文件、追多个符号）合并到一轮里发起，同样时间预算内多看 ~40-55% 的上下文（实测于 19 文件大 diff，logic 维度）。大 PR 单维度审不完的根本瓶颈仍在（见 LIMITATIONS），此项是缓解不是根治。
  Review agents now batch independent lookups (multiple file reads, multiple symbol traces) into a single turn, covering ~40-55% more context within the same time budget (measured on a 19-file diff, logic dimension). The underlying bottleneck for very large PRs remains (see LIMITATIONS); this is mitigation, not a cure.

### Fixed
- `--timeout` 现在会软着陆：时间预算耗到 75% 时自动切入收口轮，把剩余时间用来上报已确信的发现，而不是继续探索直到硬超时、一条都没报就被标「未审完」。大 PR / 慢服务商下的超时维度从「空手 incomplete」变成「带部分发现的 incomplete」。
  `--timeout` now lands softly: once 75% of the time budget is spent the agent switches to a wrap-up round, spending the remainder reporting findings it is already confident about instead of exploring until the hard cutoff with nothing reported. On large PRs / slow providers a timing-out dimension now yields partial findings instead of an empty "incomplete".
- 收敛 ai_smell 的「幻觉 API」误判：以前把「本仓库搜不到定义的符号」直接当成「该 API 不存在」并高置信拦截，会误杀真实的外部依赖/标准库/内核/系统头符号（如内核的 `krealloc_array`）。现在「找不到 ≠ 不存在」，仅凭仓内缺失不再判幻觉、不再据此 BLOCK；仍保留对有正面证据的真幻觉的拦截。
  Tightened ai_smell's "hallucinated API" false positives: it used to treat any symbol whose definition wasn't found in the repo as a nonexistent API and block with high confidence, wrongly flagging real external/stdlib/kernel/system-header symbols (e.g. the kernel's `krealloc_array`). Now "not found ≠ nonexistent" — repo-absence alone no longer marks a symbol hallucinated or triggers a BLOCK, while genuine hallucinations backed by positive evidence are still caught.

## [0.4.0] - 2026-07-01

### Added
- 新增 `--fix-all`：跳过逐条 y/N 确认，一次应用全部可自动修复项。与 `--fix` 不同，它**可在非交互环境运行**（CI/脚本），仍保留改前的 `existing_code` 锚点校验以防改错行。
  Added `--fix-all`: apply every auto-applicable fix at once, skipping the per-finding y/N prompt. Unlike `--fix`, it **works non-interactively** (CI/scripts), while still keeping the pre-edit `existing_code` anchor check to avoid editing the wrong lines.
- `--fix` 新增 `--fix-branch [名字]`：应用修复前先从当前 HEAD 新建并切到一个分支，让原分支保持干净。给名字就用它，留空则自动生成（`reviewgate-fix-<时间戳>`）。分支只在确有可应用修复且处于交互终端时才创建，不会留下空分支。
  Added `--fix-branch [name]` to `--fix`: create and switch to a new branch off the current HEAD before applying fixes, keeping your original branch clean. Provide a name or leave it blank to auto-generate (`reviewgate-fix-<timestamp>`). The branch is created only when there is at least one applicable fix and the session is interactive, so no empty branch is left behind.

## [0.3.0] - 2026-07-01

### Fixed
- 修复 `--exec-verify` 的 `run_check`：子进程输出此前会泄漏到 ReviewGate 自身 stdout（在 `--format json` 下产出非法 JSON、CI 解析失败），且执行结果从未回传给模型（一律显示「无输出」）。现在输出被正确捕获、喂回模型，`--format json` 也不再被污染。
  Fixed `run_check` under `--exec-verify`: the snippet's output leaked into ReviewGate's own stdout (producing invalid JSON under `--format json` and breaking CI parsing), and the execution result was never returned to the model (always shown as "no output"). Output is now captured and fed back to the model, and `--format json` is no longer corrupted.

### Added
- Java 现在也走精确的代码检索：`find_definition` / `find_callers` / `find_references` / `find_duplicate_functions` 由 tree-sitter AST 解析（能跳过注释和字符串里的同名文本），不再退回按行 grep。
  Java now uses precise code lookup: `find_definition` / `find_callers` / `find_references` / `find_duplicate_functions` are backed by tree-sitter AST parsing (skipping same-name text in comments and strings) instead of falling back to line-based grep.

## [0.2.1] - 2026-06-30

### Added
- 新增 Codex 与 AtomCode 集成：在 OpenAI Codex CLI（经 `AGENTS.md`）和 AtomCode 里也能用同一套 ReviewGate 审查，一键装入项目。
  Added Codex and AtomCode integrations: drive the same ReviewGate review from OpenAI Codex CLI (via `AGENTS.md`) and AtomCode, installable into a project in one command.

### Fixed
- Claude Code skill 的使用说明对齐当前真实输出：修正修复字段、退出码（含「未审完不放行」），并让触发更不易和内置 code-review 混淆。
  The Claude Code skill instructions now match the real output: corrected the fix field and exit codes (including "incomplete never passes"), and made its trigger less likely to clash with the built-in code-review.

## [0.2.0] - 2026-06-30

### Changed
- 退出码语义更清晰：`0` 放行、`1` 被闸口拦截、`2` 工具自身出错（配置/网络/密钥等）。以前工具出错和「代码被拦」都返回 1，CI 无法区分该重试还是该当成 must-fix；现在两者分开。
  Clearer exit codes: `0` pass, `1` blocked by the gate, `2` the tool itself errored (config/network/key). Previously tool errors and real blocks both returned 1, so CI couldn't tell a retryable failure from a must-fix; now they're distinct.

### Fixed
- `--fail-on` / `--format` 写错值时立即报错并列出可选值，不再被静默当成默认值——以前 `--fail-on blcok` 这类拼写错误会让闸口悄悄失效、永远放行。
  Misspelled `--fail-on` / `--format` values now fail fast and list the valid choices instead of silently falling back to the default — previously a typo like `--fail-on blcok` could quietly disable the gate and pass everything.
- 配置里拼错的字段名（如 `block_treshold`）现在在加载阶段直接报错，不再被静默忽略、让你以为调了阈值其实没生效。
  Misspelled config keys (e.g. `block_treshold`) now error at load time instead of being silently ignored, so a mistyped threshold can no longer look applied when it isn't.
- 修复 GitHub Action 示例入口：示例 workflow 现在指向实际的 `integrations/github-action` action 路径，并同步到当前发布版本，避免用户照抄后找不到 action。
  Fixed the GitHub Action example entrypoint: the sample workflow now points to the real `integrations/github-action` action path and the current release version, so copy-paste setup works.

### Docs
- README 增加可直接复制的 GitHub Action workflow；配置样例改为环境变量注入密钥优先，避免把占位 `api_key` 当成真实配置。
  README now includes a copy-paste GitHub Action workflow; the config example now prefers environment-injected secrets instead of an active placeholder `api_key`.
- README 按运营漏斗重排：首屏聚焦核心价值，快速开始去掉 active key，前置输出示例和可信证据，长 CLI 参数与实现细节下沉。
  README was reorganized around the user funnel: sharper first screen, no active key in quick config, earlier output/trust signals, and advanced CLI/design details moved lower.
- README 状态说明从 Beta 改为“核心链路已可用于真实 PR 和 CI”，同时保留先 WARN/评论模式再强制 BLOCK 的接入建议。
  README status now says the core path is ready for real PRs and CI, while still recommending WARN/comment-only rollout before enforcing BLOCK.

## [0.1.4] - 2026-06-29

### Changed
- 评审报告和实时进度现在跟随你的语言：中文环境下，章节标题（必须修复 / 警告 / 后续步骤…）、状态（通过 / 警告 / 拦截）、计数行和进度提示都显示中文；其它语言自动回退英文。命令、维度名等保持英文，方便直接复制运行。
  Review output now follows your language: under a Chinese locale the section titles, status, counts, and live progress all show in Chinese; other languages fall back to English. Commands and dimension names stay English so you can copy-paste them as-is.

### Fixed
- 修复在较窄终端里进度提示不断换行、刷满整屏的问题，现在稳定地在同一行原地刷新。
  Fixed the live progress line wrapping and flooding the screen on narrower terminals; it now refreshes cleanly in place on a single line.

## [0.1.3] - 2026-06-29

### Added
- 遇到服务商限流（429）或请求超时（408）会自动重试并尊重 `Retry-After`，偶发的一次限流不再把审查误标成「未审完」。
  Automatically retries provider rate-limits (429) and request timeouts (408), honoring `Retry-After`, so a one-off limit no longer marks a review as "incomplete".
- 大 PR 不再瞬间拉起几十路并发请求打满限流：并发数默认 6，可用 `--fanout-concurrency` 调整。
  Large PRs no longer fire dozens of concurrent requests and trip rate limits — concurrency defaults to 6 and is tunable via `--fanout-concurrency`.
- API key 错误（401/403）会被单独、如实地报出来，不再笼统说成「上下文溢出 / 未审完」。
  Authentication errors (401/403) are now reported clearly and as-is, instead of being lumped into "context overflow / incomplete".
- 配置里还留着模板占位 key（如 `YOUR_API_KEY`）时，加载阶段就直接报错，而不是发出去换回一条看不懂的服务端错误。
  If the config still contains a placeholder key (e.g. `YOUR_API_KEY`), it now fails fast at load time instead of sending it and getting a cryptic server error back.
- 输出配色尊重 `NO_COLOR`（关色）与 `FORCE_COLOR` / `CLICOLOR_FORCE`（在管道 / CI 里强制开色）。
  Output honors `NO_COLOR` (disable color) and `FORCE_COLOR` / `CLICOLOR_FORCE` (force color in pipes/CI).

### Changed
- 文本结果更易读：加入分隔线、状态图标（`✓ PASS` / `⚠ WARN` / `✖ BLOCK`）、区块标记和语义化配色（must-fix 红、warn 黄），英文长词也不再被从中间断开。
  More readable text output: separators, status icons (`✓ PASS` / `⚠ WARN` / `✖ BLOCK`), section markers, and color cues (must-fix red, warn yellow); long English words no longer break mid-word.
- 中文等非 ASCII 文本的 token 估算更准，预算不再被低估。
  More accurate token estimates for non-ASCII text (e.g. Chinese), so budgets are no longer under-counted.

### Fixed
- 修复低置信发现列表在极端情况下排序错乱的问题。
  Fixed unstable ordering of the low-confidence findings list in edge cases.
- 修复发现很多、且大多无法定位到具体行时去重变慢的问题。
  Fixed slow de-duplication when there are many findings that can't be pinned to a specific line.

### Docs
- README 补全输出语言的优先级说明、`--fanout-concurrency` 用法和刷新后的输出示例。
  README now documents output-language precedence, `--fanout-concurrency`, and refreshed output examples.

## [0.1.2] - 2026-06-27

### Added
- 意图 / 技术评审：用 `reviewgate review --intent <文件|->`（或 `--intent-from-commit` 取提交信息）传入本次改动的需求 / 设计 / 验收标准，由一个独立 Agent 跨文件检查「实现是否符合意图」，报告缺失的需求、与意图不符之处、破坏既有行为和方案风险。不传 `--intent` 时行为完全不变。
  Intent / spec review: pass your change's requirements / design / acceptance criteria with `reviewgate review --intent <file|->` (or `--intent-from-commit`), and a dedicated agent checks the implementation against intent across files — reporting missing requirements, deviations, broken behavior, and risky approaches. Behavior is unchanged when `--intent` is omitted.
- 验收清单：意图评审按每条验收标准给出结论（满足 / 缺失 / 偏差 / 破坏 / 建议），在文本里以「验收清单」分组展示，JSON 也带相应字段；没有逐条核对的标准会如实标「未核对」并降级为 WARN，绝不伪装成通过。
  Acceptance checklist: intent review gives a verdict per criterion (met / missing / deviation / breaking / suggestion), shown as a grouped checklist (and in JSON); any criterion left unchecked is honestly marked "not assessed" and downgrades to WARN rather than faking a PASS.
- 实时进度：在终端里默认单行显示评审进度（当前在调的工具 / 文件、调用次数、耗时），长时间评审不再像「卡住没动」；在 JSON / 管道 / CI / `--verbose` 下不显示。
  Live progress: a single-line progress display in the terminal (current tool/file, call count, elapsed) so long reviews no longer look stuck; hidden under JSON / pipes / CI / `--verbose`.

### Changed
- 意图评审与常规维度并行跑，整体更快——总耗时接近两者中较慢的一个，而不是相加。
  Intent review now runs in parallel with the regular dimensions, so total time is closer to the slower of the two rather than their sum.
- 大 diff 下采样固定为 1，避免成本成倍放大；`--samples` 的多采样只在普通单文件 PR 上生效。
  On large diffs, sampling is fixed to 1 to avoid multiplying cost; `--samples` multi-sampling applies only to normal single-unit PRs.

### Fixed
- `api_key` 改为可选配置：此前省略它的配置会解析失败，让「密钥只放环境变量、不写进配置」的推荐用法无法工作；现在可省略，由 `REVIEWGATE_API_KEY` 提供。
  `api_key` is now optional: previously omitting it failed to parse, breaking the recommended "keep the key in env only" setup; it can now be supplied via `REVIEWGATE_API_KEY`.
- 修复大 diff 在较小 token 预算下「所有单元都超预算、什么都没审到」的问题（真实 PR 实测从 0 发现恢复为正常审查）。
  Fixed large diffs hitting "every unit over budget, nothing reviewed" under smaller token budgets (a real PR went from 0 findings back to a full review).

## [0.1.1] - 2026-06-26

### Added
- `reviewgate upgrade`：自更新到最新发布版本——按平台下载二进制并替换当前可执行文件。
  `reviewgate upgrade`: self-update to the latest release — downloads the right binary for your platform and replaces the current executable.

### Fixed
- 修复 macOS 自带 shell 下安装脚本可能崩溃的问题。
  Fixed a possible install-script crash on macOS's built-in shell.
- GitHub Action 在 PR 事件下改为审 base→head 的改动（此前在 CI 上常常什么都审不到），并加了超时防止挂起。
  The GitHub Action now reviews base→head on PR events (previously it often reviewed nothing in CI) and adds a timeout to prevent hangs.
- 发布流程更稳：单个平台的网络抖动不再拖垮整个发布。
  More robust releases: a network blip on one platform no longer fails the whole release.

## [0.1.0] - 2026-06-26

首个公开发布：给 AI 生成（或 AI 深度参与）的代码加一道合并前质检——高置信问题优先暴露，低置信噪音默认折叠。
First public release: a pre-merge quality gate for AI-generated (or AI-heavy) code — high-confidence issues surface first, low-confidence noise is folded by default.

### Added
- 多维度并行审查 + 证伪复核：多个维度同时找问题，再由一个「先试着推翻它」的环节复核，显著降低误报。
  Multi-dimension review with refutation: several dimensions find issues in parallel, then a "try to refute it first" pass re-checks them to cut false positives.
- 45 种语言的内置起步规则，按改动文件的语言自动注入；可整体关闭，也可用你自己的规则覆盖或追加。
  Built-in starter rules for 45 languages, auto-injected by the changed file's language; can be turned off entirely or overridden/extended with your own rules.
- 大 PR 自适应切分：按 token 预算把大改动切成多个审查单元，普通 PR 不受影响。
  Adaptive splitting for large PRs: big diffs are chunked into review units by token budget, with no impact on normal PRs.
- 未审完绝不静默放行：任何没审完的情况（请求失败 / 超出上限 / 超时 / 跳过超大文件）都会把 PASS 降为 WARN、在 CI 里以非 0 退出，并在输出里醒目标注。
  Never silently passes an incomplete review: any unfinished case (request failure / over-limit / timeout / skipped oversized file) downgrades PASS to WARN, exits non-zero in CI, and is clearly flagged in the output.
- `--fix`：逐条确认后把建议补丁应用到工作区，替换前用原始代码做锚点校验，行号漂移就拒绝改错地方。
  `--fix`: applies suggested patches to your working tree after per-item confirmation, validating against the original code so it refuses to patch the wrong place when line numbers drift.
- `--exec-verify`：可选的弱隔离沙箱，运行自包含的 JS / Python 片段来验证边界用例（默认关闭，仅建议在可信环境使用）。
  `--exec-verify`: an opt-in weak-isolation sandbox that runs self-contained JS / Python snippets to check edge cases (off by default; trusted environments only).
- 业务规则：通过 `[business].rules` / `rules_dir` 注入你自己的规则（按语言按需加载），命中的发现带可追溯的规则编号。
  Business rules: inject your own rules via `[business].rules` / `rules_dir` (loaded by language on demand); matching findings carry traceable rule IDs.
- 重复函数检测：确定性地找出改动文件内部 / 之间的重复函数，交给评审判断。
  Duplicate-function detection: deterministically finds repeated functions within and across changed files for the review to judge.
- `--timeout <秒>`：给单个维度设墙钟超时，超时就跳过该维度并保留其余结果（对 CI 友好）。
  `--timeout <seconds>`: a per-dimension wall-clock cap; on timeout it skips that dimension and keeps the rest (CI-friendly).
- 输出语言探测（`REVIEWGATE_OUTPUT_LANGUAGE` / locale）；通过 `--verbose` 观察 token 用量与缓存命中率。
  Output-language detection (`REVIEWGATE_OUTPUT_LANGUAGE` / locale); token usage and cache-hit rate visible via `--verbose`.
- 真实模型评测留痕（`docs/evals/`）。
  Real-model evaluations kept on record (`docs/evals/`).

### Security
- 文件读取 / 搜索限定在仓库内，挡住绝对路径与 `..` 越界（修复了 workspace 模式下能读到仓库外文件的问题）。
  File read/search is confined to the repository, blocking absolute paths and `..` traversal (fixes reading files outside the repo in workspace mode).

### Performance
- 提示词缓存复用、防止重复工具调用空转、工具结果大小上限等优化，让重复审查更快也更省 token。
  Prompt-cache reuse, guards against repeated no-op tool calls, and a cap on tool-result size make repeated reviews faster and cheaper.
