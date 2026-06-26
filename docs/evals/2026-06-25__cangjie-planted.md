# Eval: 仓颉（Cangjie）植入式漏洞 — 未支持语言的优雅降级

- 语言：仓颉 `.cj`（华为新语言，**不在 tree-sitter 支持集** rust/cpp/python/go/js/ts 内）
- 模型：deepseek-v4-pro；维度 security/logic + business（配置 owner_id 规则自动启用）
- 目的：验证**未知语言**下 ReviewGate 是否仍可用（LLM 审查与语言无关；符号工具应优雅降级）

## 被测 diff（仓颉，植入 SQL注入插值 + 删除 owner 校验 + 命令注入）

## 结果：BLOCK ✗ — 4 条，全部命中（退出码 1）

| 漏洞 | 维度 | 置信度 |
|---|---|---|
| getOrder 删除 owner 校验（越权） | business `[B1]` | 0.97 |
| getOrder SQL 注入（插值） | security | 0.99 |
| exportOrder 无 owner 校验（越权） | business `[B1]` | 0.89 |
| exportOrder 命令注入 + 路径穿越 | security | 0.99 |

## 解读
- **语言无关性成立**：仓颉对 tree-sitter 完全不支持，但 LLM 把代码当文本推理，4 类植入漏洞全部抓到、
  且业务规则 `[B1]` 正确引用。说明 ReviewGate 对**任意新语言/小众语言**都能做 LLM 级审查。
- **优雅降级**：`find_duplicate_functions`（依赖 tree-sitter 列函数）对 `.cj` 返回"无重复"而非崩溃；
  `find_definition/callers/references` 走 `git grep` 词法兜底（精度下降但可用）。`read_file`/`code_search` 与语言无关。
- **能力边界（诚实）**：未支持语言会失去——精确符号检索、确定性重复函数检测、`<lang>.md` per-language 规则路由
  （`.cj` 不在扩展名映射里）。要补只需在 `index/treesitter.rs` 加 grammar、在 `review/rules.rs` 加扩展名映射。
