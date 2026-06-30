<!-- BEGIN ReviewGate -->
## ReviewGate — 合并前质量闸口

当用户想审查当前改动、做合并前质量闸口、或检查 AI 生成代码时，用 ReviewGate CLI（`reviewgate`）。
它**只读、不改代码**——多 Agent 并行分维度审查 + 证伪 Judge + 置信度过滤，修复由你执行。

### 前置检查
1. CLI 已装：`reviewgate --version`。没有则提示安装：
   - Linux/macOS：`curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh`
   - Windows：`irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex`
2. LLM 已配：`./reviewgate.toml` 或 `~/.reviewgate/config.toml`；密钥也可用环境变量 `REVIEWGATE_API_KEY` 注入。
   先 `reviewgate llm test` 验证连通——**不通就别往下走**，让用户补好密钥/端点。

### 运行审查（务必带 `--timeout`，防止慢端点长时间挂起）
```bash
reviewgate review --format json --timeout 300
```
- 默认审工作区相对 HEAD 的改动（含未跟踪文件），跑全部维度（security/perf/logic/style/ai_smell，配了 `[business].rules` 则加 business）。
- 审某个 commit / 范围：`--commit <sha>` 或 `--from <base> --to <head>`；只看部分维度：`--dimensions security,logic`；更快但误报略多：`--no-judge`。

### 解读 JSON
顶层是一个信封 `{ decision, incomplete, files_changed, summary, warnings[], findings[], usage }`——`decision` 为 `pass|warn|block`，问题都在 `findings[]` 里。每条 finding 含：
`path / start_line / end_line / dimension / severity(high|med|low) / confidence(0–1) / message / suggestion（修复思路，文字）/ suggestion_code（可直接套用的替换代码，可能为空串）/ existing_code（原始代码片段）/ evidence / filtered / agreed_dimensions`。
- `filtered=true`：被闸口过滤的低置信项，**默认忽略**。
- `agreed_dimensions ≥ 2`：多维度独立指向同一处 → **更可信**，优先汇报。
- 排序：先 `severity`（high 优先）再 `confidence`。
- **未审完检查（重要）**：顶层 `incomplete=true` 或 `warnings` 非空 → 有维度/单元因超时、上下文超限或请求失败**没审完**。此时**绝不能报告"无问题/通过"**，必须明确告知"审查不完整"并建议重跑（调大 `--timeout`）或人工补审。

### 汇报与修复
- 汇报：用简洁中文列出可信问题（`path:start_line` + 维度 + 一句话 + 建议），说明 PASS/WARN/BLOCK 判定。
- 退出码：`0`=放行 · `1`=被闸口拦截**或审查未完成** · `2`=工具自身出错（配置/网络/密钥）。`--fail-on block|warn|never` 调节判定；只要 `incomplete=true`，即便 decision 是 warn/pass 也会非 0 退出。
- 修复（用户要求时）：优先套用 `suggestion_code`（可直接替换的代码；为空串则按 `suggestion` 的文字思路改），用 `existing_code` / `start_line` 定位，改完再次 `reviewgate review` 复核。
<!-- END ReviewGate -->
