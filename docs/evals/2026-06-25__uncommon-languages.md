# 冷门语言覆盖（Round 3，2026-06-25）

植入式漏洞（SQL注入插值 + 删除 owner 校验），外置配置（干净 diff），模型 deepseek-v4-pro。

| 语言 | 闸口 | 命中 |
|---|---|---|
| Nim | BLOCK | ✓ SQL注入 |
| Haskell | BLOCK | ✓ SQL注入 |
| Perl | BLOCK | ✓ SQL注入 + 越权 |
| Lua | BLOCK | ✓ SQL注入 + 越权 |
| Crystal | BLOCK | ✓ SQL注入 + 越权 |
| OCaml | BLOCK | ✓ SQL注入 + 越权 |
| Julia | BLOCK | ✓ SQL注入 + 越权 |
| R | BLOCK | ✓ SQL注入 |

## 结论
- **8/8 冷门语言全部 BLOCK 并命中 SQL 注入**（含函数式 Haskell/OCaml、科学计算 Julia/R、脚本 Lua/Perl、系统 Nim/Crystal）。
- 这些语言均**无 tree-sitter 支持**，符号工具走 grep 兜底——再次印证 LLM 审查能力与语言无关。
- 连同 round1（22 语言+仓颉）与真实 CVE（5 语言），ReviewGate 已在 **30+ 种语言**上验证可用。
