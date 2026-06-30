---
name: reviewgate
description: 合并前质量闸口 · Pre-merge quality gate. 驱动外部 ReviewGate CLI（reviewgate review）对当前 git 改动分维度并行审查、证伪、按置信度过滤，给出 PASS/WARN/BLOCK。当用户想用 ReviewGate / 做质量闸口 / pre-merge review 审查改动时使用；显式触发 /reviewgate，区别于内置 code-review。
---

# ReviewGate Skill

ReviewGate 是一个命令行质量闸口：多 Agent 并行审查 + 分维度专家 + 证伪 Judge + 置信度过滤。
本 Skill 教你（Agent）正确调用它的 CLI，并把结果转达给用户。**ReviewGate 只读、不改代码——修复由你执行。**

## 安装本 Skill

把本目录的 `SKILL.md` 放到 Claude Code 的 skill 目录即可被发现：
```bash
mkdir -p ~/.claude/skills/reviewgate
cp integrations/claude-skill/SKILL.md ~/.claude/skills/reviewgate/SKILL.md
```

## 前置检查

1. **已安装 CLI**：`reviewgate --version`。没有则提示用户安装：
   - Linux/macOS：`curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh`
   - Windows：`irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex`
2. **已配置 LLM**：当前目录 `./reviewgate.toml` 或全局 `~/.reviewgate/config.toml`；密钥也可用环境变量 `REVIEWGATE_API_KEY` 注入。
   先跑 `reviewgate llm test` 验证连通——**不通就别往下走**，先让用户补好密钥/端点。

## 工作流

1. **了解背景**（可选）：用户若描述了改动意图，记住它，便于解读结果。
2. **运行审查**（务必带 `--timeout`，防止慢端点上长时间挂起）：
   ```bash
   reviewgate review --format json --timeout 300
   ```
   - 默认审工作区相对 HEAD 的改动（含未跟踪文件），跑全部维度（security/perf/logic/style/ai_smell，配了 `[business].rules` 则加 business）。
   - 审某个 commit / 范围：`--commit <sha>` 或 `--from <base> --to <head>`。
   - 只看部分维度：`--dimensions security,logic`。
   - 更快但误报略多：`--no-judge`。
   - **慢端点提示**：deepseek 等推理模型每轮慢，单维度可能跑几分钟；`--timeout` 到点会跳过该维度但**保留已确认的发现并如实告警**，不会把"没审完"伪装成"通过"。
3. **解读 JSON**：顶层是一个信封 `{ decision, incomplete, files_changed, summary, warnings[], findings[], usage }`——`decision` 为 `pass|warn|block`，问题都在 `findings[]` 里。每条 finding 含
   `path / start_line / end_line / dimension / severity(high|med|low) / confidence(0–1) / message / suggestion（修复思路，文字）/ suggestion_code（可直接套用的替换代码，可能为空串）/ existing_code（原始代码片段）/ evidence / filtered / agreed_dimensions`。
   - `filtered=true`：被闸口过滤的低置信项，**默认忽略**，除非用户要看全部。
   - `agreed_dimensions ≥ 2`：多个维度独立指向同一处 → **更可信**，优先汇报。
   - 排序：先 `severity`（high 优先）再 `confidence`。
   - **未审完检查（重要）**：顶层 `incomplete=true` 或 `warnings` 非空 → 有维度/单元因超时、上下文超限或请求失败**没审完**。此时**绝不能报告"无问题/通过"**，必须明确告知用户"审查不完整"，并建议重跑（如调大 `--timeout`）或人工补审。
4. **汇报**：用简洁中文列出可信问题（`path:start_line` + 维度 + 一句话 + 建议），并说明闸口判定。
   退出码：`0`=放行 · `1`=被闸口拦截**或审查未完成** · `2`=工具自身出错（配置/网络/密钥）。`--fail-on block|warn|never` 调节闸口判定；只要 `incomplete=true` 即便 decision 是 warn/pass 也会非 0 退出（杜绝漏审放行）。
5. **修复**（用户要求时）：优先套用 `suggestion_code`（可直接替换的代码；为空串则按 `suggestion` 的文字思路改），用 `existing_code` / `start_line` 定位，改完再次 `reviewgate review` 复核。

## 注意

- `start_line=0` 表示行号未定位，按 `existing_code`/`message` 在文件里定位。
- 不要把 `filtered` 的低置信项当成必须处理的问题——那正是被过滤掉的「废话」。
- 维度名仅供分类参考；真问题与否以 `message` + `evidence` 为准。
- ReviewGate 只读，不会动用户代码。
