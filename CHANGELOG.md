# Changelog

本项目变更记录。格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循 SemVer。
每条变更先中文、后英文。
Changes are listed in Chinese first, then English.

## [Unreleased]

### Added
- GitHub Action 新增 `intent` 输入：`intent: "auto"` 自动把 PR 标题+描述作为 `--intent` 做「实现 vs 意图」评审（也可传固定意图文档路径）。用于覆盖「每个 hunk 都自洽、但整体没做到 PR 声称的事」这类缺陷向审查抓不到的问题。默认关闭：标题含糊会产生「未核对」项并降级 WARN。
  The GitHub Action gained an `intent` input: `intent: "auto"` automatically feeds the PR title + description to `--intent` for an "implementation vs intent" review (a fixed intent-document path also works). It covers the class of issue defect-oriented review can't see — every hunk looks consistent, but the change doesn't do what the PR claims. Off by default: vague titles produce "not assessed" items and downgrade to WARN.
- TypeScript 换用专用语法解析：`interface` / `type` 别名 / `enum` / `abstract class` 现在能被 `find_definition` 等精确工具识别为定义（此前 TS 复用 JS 语法，这些构造会被漏掉或解析错位）；`.tsx` 用 JSX 感知的语法解析。
  TypeScript now uses its dedicated grammar: `interface` / `type` aliases / `enum` / `abstract class` are recognized as definitions by `find_definition` and friends (previously TS reused the JS grammar, which missed or mis-parsed these constructs); `.tsx` is parsed with the JSX-aware grammar.

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
