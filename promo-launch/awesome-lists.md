# awesome 列表投递草稿

> 建议时机：等 crates.io 发布 + Action 上 Marketplace + star 到 30+ 再投。
> 多数 awesome 列表维护者会看 star 数和活跃度，3 star 阶段被拒的概率高，
> 而同一个项目二次投递通常不受欢迎——一次投中比早投重要。
> 本文件不进 git（如果要进，先删这段再说）。

## 1. awesome-code-review (joho/awesome-code-review)

分区：Tools → 按字母序插入一行：

```markdown
- [ReviewGate](https://github.com/dengmengmian/ReviewGate) - Self-hosted, model-agnostic pre-merge quality gate for AI-generated code. Reviews diffs across security/logic/perf dimensions with counter-evidence validation; degrades to WARN instead of faking a PASS on incomplete reviews (MIT, Rust).
```

PR 标题：`Add ReviewGate (self-hosted AI code review gate)`

## 2. awesome-rust (rust-unofficial/awesome-rust)

分区：Development tools（或 Applications → Development tools），按字母序：

```markdown
* [ReviewGate](https://github.com/dengmengmian/ReviewGate) — Pre-merge quality gate for AI-generated code: parallel multi-dimension LLM review with counter-evidence validation and CI exit codes
```

注意：awesome-rust 要求项目有一定成熟度（CI 通过、近期活跃），投递前确认 CI 徽章是绿的。

## 3. awesome-claude-code（社区维护，投前确认最活跃的那个仓库）

分区：Skills / Integrations：

```markdown
- [ReviewGate](https://github.com/dengmengmian/ReviewGate/tree/main/integrations/claude-skill) - Pre-merge quality gate as a Claude Code skill: /reviewgate reviews your working diff across security/logic/perf dimensions with confidence gating.
```

## 4. 其他可登记的目录（无需 PR，表单提交）

| 平台 | 说明 |
|---|---|
| AlternativeTo | 登记为 CodeRabbit / Qodo 的 alternative，选 Open Source + Self-Hosted 标签 |
| GitHub Marketplace | reviewgate-action 的 release 页点 "Publish this Action to the GitHub Marketplace"（手动，一次性） |
| lib.rs | crates.io 发布后自动收录，无需操作 |
