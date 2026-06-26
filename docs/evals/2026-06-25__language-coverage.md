# 全语言覆盖 sweep（round 1，2026-06-25）

每个语言：baseline(安全) → buggy(类型匹配漏洞)，模型 deepseek-v4-pro，维度 security/logic(+business)。
**期望**：含漏洞的 → BLOCK 并命中；CSS 对照（无攻击面）→ PASS。

| 语言 | 漏洞类型 | 闸口 | 命中 | tree-sitter |
|---|---|---|---|---|
| Go | SQL注入+越权 | BLOCK | ✓ security L2 + business L1 | 兜底 |
| Python | SQL注入+越权 | BLOCK | ✓ security L2 | ✓精确 |
| JavaScript | SQL注入+越权 | BLOCK | ✓ security+business | ✓精确 |
| TypeScript | SQL注入+越权 | BLOCK | ✓ security L2 | ✓精确 |
| Rust | SQL注入+越权 | BLOCK | ✓ security+business（+第3条） | ✓精确 |
| C# | SQL注入+越权 | BLOCK | ✓ security+business | 兜底 |
| Dart | SQL注入+越权 | BLOCK | ✓（business/越权） | 兜底 |
| Scala | SQL注入+越权 | BLOCK | ✓ business+logic | 兜底 |
| Objective-C | SQL注入+越权 | BLOCK | ✓ security+business | 兜底 |
| Ruby | SQL注入+越权 | BLOCK | ✓ security L2 | 兜底 |
| PHP | SQL注入+越权 | BLOCK | ✓ security+business | 兜底 |
| C++ | SQL注入+越权 | BLOCK | ✓ security+business | ✓精确 |
| Java | SQL注入+越权 | BLOCK | ✓ security+business | 兜底 |
| Kotlin | SQL注入+越权 | BLOCK | ✓ security+business | 兜底 |
| Swift | SQL注入+越权 | BLOCK | ✓ business×2 | 兜底 |
| C | 命令注入(system) | BLOCK | ✓ business+security | ✓精确 |
| Shell | 命令注入(eval) | BLOCK | ✓ business×2 | 兜底 |
| SQL | 动态 SQL 注入 | BLOCK | ✓ business | 兜底 |
| HTML | XSS(innerHTML) | BLOCK | ✓ business+security | 兜底 |
| Vue | XSS(v-html) | BLOCK | ✓ business | 兜底 |
| Svelte | XSS({@html}) | BLOCK | ✓ business | 兜底 |
| **CSS** | **对照(无漏洞)** | **PASS** | **✓ 0 误报** | 兜底 |
| 仓颉 Cangjie | SQLi+越权+命令注入 | BLOCK | ✓ 4 条 | 兜底 |

## 结论
- **22 个含漏洞语言全部 BLOCK 并命中**，覆盖 SQL注入 / 越权 / 命令注入 / 动态SQL / XSS 五类漏洞。
- **CSS 对照（无攻击面）→ PASS，0 误报**，证明非"逢改必报"。
- **语言无关性**：仅 6 种语言有 tree-sitter 精确工具，其余（含全新语言仓颉）走 grep 兜底，LLM 审查照常命中——
  证明 ReviewGate 审查能力与语言无关，精确符号工具只是增强项。

## 方法学教训（已修正）
- 初次 CSS 误报源于**测试配置错误**：注入了无意义的业务规则 `"无"`，模型据此臆造。去掉后 CSS 正确 PASS。
  教训：业务规则 garbage-in→garbage-out；规则质量直接影响结果。
- dimension 标签存在 run-to-run 抖动（同一 SQLi 有时 security、有时随 business 规则合并归 business），
  属正常 LLM 方差；不影响"是否命中 + 是否 BLOCK"。

## 后续轮次（持续补充）
- round 2：换漏洞类型重跑（SSRF / 路径穿越 / 反序列化 / 硬编码密钥 / 竞态），验证跨类型召回。
- 为高频语言补 tree-sitter grammar（如 Java/C#/Kotlin/Swift）以启用精确符号检索与重复检测。
