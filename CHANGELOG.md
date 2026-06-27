# Changelog

本项目变更记录。格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循 SemVer。

## [Unreleased]

## [0.1.2] - 2026-06-27

### Added
- **意图 / 技术评审**：`reviewgate review --intent <文件|->`（或 `--intent-from-commit` 取提交信息）传入本次改动的需求/设计/验收标准，由一个**独立的整体性 Agent**（专用探索向系统提示）审「实现 vs 意图」——从 diff 出发主动跨文件探索(调用方/契约/测试),报告：缺失需求、与意图不符、破坏既有行为、方案风险。与常驻的 `business.rules` 正交(规则=不变量，intent=本次该做什么)。发现并入主结果过证伪 Judge / 闸口；未提供 `--intent` 时零退化。受控 A/B 实测(axios URL 对象特性)：不完整实现命中缺口、完整实现 0 误报。
- **需求锚定上报 + 验收清单视图**：意图 Agent 用专用工具 `report_intent_finding`（按验收标准上报 verdict：met/missing/deviation/breaking/suggestion，位置可选），文本渲染为 **Intent / Acceptance Checklist**（按标准分组、状态标签），JSON 也输出 `criterion`/`intent_status`。
- **验收清单结构化强制**：意图被解析成 N 条标准（C1..CN）注入评审；跑完后**未被逐条 verdict 的标准兜底标 `? not assessed`**，保证清单**覆盖每一条**（杜绝真实数据上常见的空清单）；有未核对标准则**降级 WARN**，绝不伪装 PASS。真实数据实测：gin 提交信息 → 4/4 met；axios 详细 spec → C1 met + 其余诚实标"未核对"。
- **审查实时进度**：终端下默认显示单行就地刷新的进度（spinner + 当前在调的工具/文件 + 工具计数 + 耗时），让长时审查（尤其意图评审）不再"静默看不出在跑没在跑"；结束清行并留一行紧凑完成摘要（细节收起）。仅 TTY 生效；`--format json` / 管道 / CI / `--verbose` 下不渲染（后者保留完整逐轮日志）。

### Fixed
- **`api_key` 改为可选配置**：此前是必填字段，省略它的配置文件会以 `missing field api_key` 解析失败——令「密钥只放环境变量、不写进配置」的推荐用法无法工作。现可省略，由 `REVIEWGATE_API_KEY` 注入；解析后仍为空则给出清晰错误而非困惑的解析失败。

### Changed
- **意图评审与维度 fan-out 并发执行**：意图 Agent 不依赖维度结果，改用 `tokio::join!` 与 fan-out 同时跑，总墙钟从 ≈`fan-out + intent`（翻倍）降到 ≈`max(fan-out, intent)`。10 个真实 commit 批量实测暴露的成本点。
- **CLI 文案统一为英文**：状态/进度/章节标题/报错/`--verbose` 日志/GitHub PR 评论一律英文（与已英文的提示词、章节标题、双语 README 的开源定位一致）；**finding 内容仍按 `REVIEWGATE_OUTPUT_LANGUAGE`/locale 本地化**（中文用户照样看中文发现）。
- 大 diff（多审查单元）下**采样固定为 1**：避免 `单元×维度×样本` 的成本放大；`--samples` 多采样只在单单元（正常 PR）上生效。

### Fixed
- 大 diff 切分预留**系统提示词 + 维度 focus 的固定开销**：此前 `plan_units` 只按 diff 计 token，小/中 `max_input_tokens` 下切出的单元会在 Agent 发送前预检**全部超预算**（审不到任何东西）。修复后单元在首轮一定可发送（真实 PR 实测：axios 847d89b @4k 由"全部超预算 / 0 发现"变为"8 单元正常审 / 13 发现"，首轮超预算归零）。默认 200k 预算下为无感知 no-op。

### Tests
- 新增 `diff_modes` 集成测试：真实 git 仓库覆盖 Workspace / Commit / Range 三种采集模式 + 未跟踪文件。

## [0.1.1] - 2026-06-26

### Added
- `reviewgate upgrade`：自更新到最新 release——按平台下载二进制并替换当前可执行文件（Windows 经 self-replace 处理运行中 exe）。

### Fixed
- `install.sh`：修复 macOS `sh`(bash 3.2) `set -u` 下 `$INSTALL_DIR` 紧跟中文全角括号导致的 `unbound variable` 安装崩溃（fallback 路径）。
- GitHub Action：PR 事件按 `--from base --to head` 范围审（此前默认工作区 diff 为空、CI 上审不到任何东西）；新增 `--timeout` 防挂起。
- Release workflow 加固：`fail-fast: false` + `CARGO_NET_RETRY`，单平台网络抖动不再毁掉整个发布。

## [0.1.0] - 2026-06-26

首个公开发布：给 AI 生成或 AI 大量参与的代码加一道合并前质检。高置信问题优先暴露，低置信噪音默认折叠；
包含多 Agent 分维度审查、证伪 Judge、45 语言内置规则、大 PR 未审完不静默放行、只读安全边界，
并用真实模型评测留痕（`docs/evals/`）。

### Added
- **45 语言内置起步规则**：常见+不常见 45 种语言（含仓颉/Zig/Nim/Crystal/OCaml/F#/Solidity/
  COBOL/Fortran/Dockerfile/Terraform…）的公认陷阱清单随二进制内置，按改动语言注入；
  `[business] builtin_language_rules`（默认 true）可整体关，用户 `rules_dir/<lang>.md` 可覆盖/追加。
- **大 diff 自适应审查单元**：`plan_units` 按 token 预算切单元（N 默认=1，正常 PR 零退化；
  放不下才按目录就近装箱以保跨文件推理）；`ProviderConfig.max_input_tokens`（默认 200k）。
- **未审完不静默放行**：`GateConfig.fail_on_incomplete`（默认 true）+ `AgentExitReason` + 发送前 token 预检；
  任何未审完（请求失败/上下文超限/超时/超大文件跳过）一律 PASS→WARN、CI 非 0 退出、输出醒目标注。
- `--fix`：逐条确认后把 `suggestion_code` 应用到工作区，替换前用 `existing_code` 锚点校验（行号漂移则拒绝改错）。
- `--exec-verify`：opt-in 弱隔离沙箱执行自包含 JS/Python 片段验证边界用例（默认关，仅可信环境用）。
- `reachability`（可达性/latent）分级：latent 发现不阻断闸口。
- 输出语言探测（`REVIEWGATE_OUTPUT_LANGUAGE` / locale）；system prompt 英文化以利国际化。
- 端到端集成测试（mock LLM）：全链路编排 + `oversized_diff_never_silently_passes` 安全网守卫。
- 业务规则注入：`[business].rules` + `rules_dir`（`<语言>.md` 按改动语言按需注入），
  新增 `business` 审查维度（配置规则时自动启用），规则编号 `[B1]..` 可追溯。
- `find_duplicate_functions` 工具：确定性检测改动文件内/间的重复函数（diff-scoped + 样板过滤），
  候选交 Agent/Judge 判断。
- `--timeout <秒>`：单维度墙钟超时兜底，超时跳过该维度并保留其余（CI 友好）。
- `diff` 命令支持 `--commit` / `--from --to`（与 `review` 共用范围解析）。
- 真实 PR 评测闭环：`scripts/eval-pr.sh`，结果留痕 `docs/evals/`。
- CI 工作流：fmt + clippy（拒绝告警）+ test。
- token 用量与**缓存命中率**观测（`--verbose`）。

### Changed
- prompt 缓存重排：`system` 通用化 + diff/文件大块挂 `cache_control`，跨维度/跨轮复用。
- 证伪 Judge 改为带证据单次裁决，不确定才升级工具（`MAX_ROUNDS` 8→4）。
- 模型直报标注行号，`relocate` 降为锚点校验/兜底。
- 结果排序改为复合：未过滤 → severity → confidence。

### Security
- `read_file` / `code_search` 接入 `confine_path`，挡住绝对路径与 `..` 穿越
  （修复 workspace 模式下可读取仓库外文件的越界问题）。

### Performance
- 跨维度一致性加分；loop guard 防重复工具调用空转；工具结果 32 KiB 上限。
