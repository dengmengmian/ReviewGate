# Eval: BurntSushi/ripgrep#3420 — ignore: scope compiled parent matchers by root

- URL: https://github.com/BurntSushi/ripgrep/pull/3420
- base: `48a6ad93f152dc848f1883ceb3bf2c7baab6738c`  head: `b6722c177f85a01ed2304781118651c058df3286`
- 模型: deepseek-v4-pro @ api.deepseek.com
- 维度: logic, security, ai_smell（judge on）
- 评测时间: 2026-06-25

## 改动
1 文件 `crates/ignore/src/dir.rs`（+208 −136，19 hunks）。作者是 ripgrep 维护者的一次重构。

## 结果：PASS ✓ — 0 条发现

| 维度 | 轮数 | 工具 | 缓存命中 | 结论 |
|---|---|---|---|---|
| security | 2 | read_file | 49% | 无 |
| ai_smell | 6 | code_search / find_duplicate_functions / read_file | 78% | 无 |
| logic | 7 | find_callers / find_references / read_file / code_search | 86% | 无 |
| **总计** | 15 LLM 次 | 22 工具次 | **80%**（277760/346204 tok）| 输出 39513 tok |

## 解读
- **精度信号（好）**：对来自顶级维护者的谨慎重构，ReviewGate **未臆造问题**——三维度都做了实际调研
  （追调用方/引用、读相邻代码、查重复函数）后给"无问题"，符合"宁缺毋滥"。
- **缓存验证（好）**：跨维度/跨轮缓存命中 **80%**（logic 86%）——缓存重排在真实负载上生效。
- **工具链验证（好）**：tool-calling、符号检索、`find_duplicate_functions` 在真实大 diff 上均正常。
- **未覆盖**：本 PR 是"干净样本"，只验证**精度**（无误报）；**召回**见 `_run_recall`（植入式 bug 测试）。

## 由本次评测驱动的修复
- 超时从"硬 cancel 丢工作"改为"Agent 内每轮检查、优雅收尾保留已上报发现"，避免超时被误判为 PASS。
