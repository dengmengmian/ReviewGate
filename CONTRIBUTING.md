# 贡献指南

感谢参与 ReviewGate！本项目是一个**只读的代码审查质量闸口**：多维度专家 Agent 并行审查 +
证伪 Judge 过滤误报 + 置信度闸口。请保持这个边界——不引入写文件 / 跑 shell 的能力。

## 开发环境

- Rust stable（含 `rustfmt`、`clippy`）
- `git`（diff 解析与工具检索都依赖它）
- 评测真实 PR 需要 `gh`（已登录）

```bash
cargo build
cargo test --all
```

## 提交前的三道闸门（与 CI 一致）

CI（`.github/workflows/ci.yml`）会拒绝任何告警。本地请先跑：

```bash
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features
cargo test --all
```

## 项目结构

```
crates/core/src/
  agent/      多维度 Agent 的 tool-use 循环 + 提示词
  judge/      证伪 Judge（带证据单次裁决）
  review/     编排：多维并行 → 校验/去重 → judge → 闸口；rules.rs 规则注入
  tool/       只读工具：read_file/code_search/find_*/find_duplicate_functions（confine_path 安全边界）
  index/      tree-sitter 符号检索 + 函数体提取
  relocate/   行号校验/兜底
  model/      Finding/Usage/Message 等协议无关模型
  llm/        Anthropic / OpenAI 兼容客户端（prompt 缓存）
crates/cli/   reviewgate CLI
```

## 常见扩展

- **加审查维度**：在 `model::Dimension` 增变体 → `as_str` → `agent::prompt::dimension_focus`
  → `cli::parse_dimension`；是否进默认 `ALL` 看是否该默认启用。
- **加工具**：实现 `tool::Tool`，在 `readonly_tools()` 注册。保持**只读**。
- **加业务规则**：用户侧配置 `[business].rules` / `rules_dir`，无需改代码。

## 测试与留痕

- 单元测试与代码同文件 `#[cfg(test)]`。新行为请补测试。
- 真实 PR 评测：`scripts/eval-pr.sh <owner/repo> <pr#>`，结果留痕在 `docs/evals/`。
- 优化决策记录在 `docs/PROGRESS.md`（动机 / 业内对标 / 改动 / 验证）。

## 提交规范

- 提交信息用祈使句，说明**动机**而非仅罗列改动。
- 一个 PR 聚焦一件事；附测试与（涉及行为时）一条 `docs/PROGRESS.md` 记录。
