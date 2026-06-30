# 意图 / 技术评审 MVP —— 受控 A/B(ground truth)

验证独立的整体性「意图 agent」能否据传入意图区分**完整实现**与**不完整实现**。

**素材**：axios「URL 对象作为 config.url」特性(`847d89b`，后被官方 revert)。
- **A = 不完整**：在 `847d89b` 基础上回退掉 `lib/core/dispatchRequest.js` 的处理，故意制造「验收#2:dispatch 路径也须处理 URL 对象」的缺口。
- **B = 完整**：原始 `847d89b`。
- **意图(验收标准)** 作为 `--intent` 文件传入，含 #2。

| 版本 | intent 发现 | 命中 |
|---|---|---|
| A 不完整 | **1 条**(high / conf 0.74)：URL 对象未规范化即到达 dispatchRequest——且**跨文件追到拦截器链**(拦截器可再次把 config.url 设为 URL 对象，dispatch 前无规范化) | ✓ 精准命中故意制造的缺口 |
| B 完整 | **0 条** | ✓ 无误报缺口 |

**结论**：意图 agent 做的是「实现 vs 意图 + 主动跨文件探索」，能据需求判断完整性——与早期「把验收标准塞进 `business.rules` → 0 发现」的反证([techreview-intent-axios](2026-06-27__techreview-intent-axios.md))形成对照。MVP 成立。

复现：`reviewgate review --from <base> --to <head> --intent spec.md`（或 `--commit <sha> --intent spec.md`）。
