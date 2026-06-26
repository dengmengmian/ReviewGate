---
name: reviewgate
description: 在代码合入主干前，用多个并行 Agent 做一次质量判断——安全/性能/逻辑/规范/AI 代码专项（及可选业务规则）分维度审查，带置信度过滤与质量闸口，只留下可信问题。当用户想审查当前改动、做合并前质量闸口、或检查 AI 生成代码时使用。
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
3. **解读 JSON**：每条 finding 含
   `dimension / confidence(0–1) / severity(high|med|low) / path / start_line / end_line / message / suggestion / evidence / filtered / agreed_dimensions`。
   - `filtered=true`：被闸口过滤的低置信项，**默认忽略**，除非用户要看全部。
   - `agreed_dimensions ≥ 2`：多个维度独立指向同一处 → **更可信**，优先汇报。
   - 排序：先 `severity`（high 优先）再 `confidence`。
4. **汇报**：用简洁中文列出可信问题（`path:start_line` + 维度 + 一句话 + 建议），并说明闸口判定。
   退出码：`BLOCK→1`，否则 `0`（`--fail-on block|warn|never` 可调）。
5. **修复**（用户要求时）：逐条按 `suggestion` 改，改完再次 `reviewgate review` 复核。

## 注意

- `start_line=0` 表示行号未定位，按 `existing_code`/`message` 在文件里定位。
- 不要把 `filtered` 的低置信项当成必须处理的问题——那正是被过滤掉的「废话」。
- 维度名仅供分类参考；真问题与否以 `message` + `evidence` 为准。
- ReviewGate 只读，不会动用户代码。
