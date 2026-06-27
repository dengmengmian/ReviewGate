# Changelog

本项目变更记录。格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循 SemVer。

## [Unreleased]

### Changed
- 大 diff（多审查单元）下**采样固定为 1**：避免 `单元×维度×样本` 的成本放大；`--samples` 多采样只在单单元（正常 PR）上生效。

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
