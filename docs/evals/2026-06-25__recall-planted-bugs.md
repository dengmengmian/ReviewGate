# Eval: 召回测试（植入式 bug）

- 模型: deepseek-v4-pro @ api.deepseek.com
- 维度: security, business, logic（judge on）；配置了 `[business].rules`
- 评测时间: 2026-06-25
- 目的: 验证**召回**——能否抓到真实 bug、业务规则是否生效、闸口是否正确 BLOCK。

## 被测 diff（Python，植入 5 类问题）
对一个有 `owner_id` 校验 + 参数化查询的干净 `get_order`，改为：
1. SQL 注入（f-string 拼接 `order_id`）
2. 删除 `owner_id` 越权校验
3. 新增 `refund`：无任何归属校验
4. `refund` 金额用 `float`（违反业务规则 B1）
5. `refund` 再次 SQL 注入

## 结果：BLOCK ✗ — 7 条可信发现（全部命中）

| 植入问题 | 命中维度 | 置信度 |
|---|---|---|
| SQL 注入 get_order | security（+logic） | 0.99 |
| 删除 owner_id 越权 | business `[B2]` + security + logic | 0.84–0.99 |
| refund 无归属校验 | security `[B2]`（+business/logic） | 0.99 |
| 金额用 float | `[B1]` + business + logic | 0.99 |
| SQL 注入 refund | security（+logic） | 0.99 |
| **额外**：None 处理回归（未刻意植入的真 bug） | logic | 0.79 |

管线：15 条原始 → 去重 8 → judge 保留 7 / 证伪 1 → 闸口 **BLOCK**。缓存命中 74%。

## 解读
- **召回（好）**：5 类植入 bug **全部抓到**，且多被两三个维度交叉印证；还额外发现了一个我未刻意植入的
  空值处理回归——说明逻辑维度在做真实推理而非模式匹配。
- **业务规则生效（好）**：金额/越权问题被打上 `[B1]`/`[B2]`，可追溯到配置的具体规则。
- **跨维度一致性（好）**："另由 logic 维度同时标记"出现，置信度加分生效。
- **judge 生效（好）**：8 候选证伪掉 1 条。
- **闸口（好）**：高危问题 → BLOCK（CI 退出码非 0）。

## 观察到的待优化点（下一轮）
- **维度归属偏移**：owner_id / float 这类**业务规则**问题，最终被去重保留在 `security` 维度名下
  （因各维度都看得到共享规则、都报了，去重按置信度取最高的 security 版本）。`[B1]/[B2]` 标签仍在，
  追溯不丢，但维度标签略乱。可考虑：去重合并时，若候选带业务规则标记，优先保留 `business` 维度归属。
- **buggy 文件上的报告量**：明显问题会被多维度重复报（去重前 15 条），依赖 dedup/judge 收敛——
  目前工作正常，但可继续观察大改动下的噪声。
