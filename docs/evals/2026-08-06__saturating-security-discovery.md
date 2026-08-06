# 2026-08-06 · `reviewgate security` 饱和式 discovery 真实评测

验证对象：`reviewgate security` 从「固定 2 次采样」改为「跑到挖不出新问题为止」
（连续 2 轮无新发现即停，上限 6 轮）。

- 工具：`reviewgate 0.11.1`（本地 release 构建）
- provider：deepseek `deepseek-v4-pro`
- 命令：`reviewgate security --from <base> --to <head> --verbose --timeout 600 --format json`

## 一、真实 PR：不误报

三个真实合并 PR（都是 clean 变更，不应报出 must-fix）。

| 仓库 | PR | 饱和轮数 | 收敛方式 | 判定 | 未过滤发现 |
|---|---|---|---|---|---|
| gin-gonic/gin | 4709 | 2 | converged | PASS | 0 |
| pallets/flask | 6013 | 2 | converged | PASS | 0 |
| psf/requests | 7596 | 2 | converged | PASS | 0 |

要点：

- **clean 变更上成本没有增加**。没有发现时，第 1、2 轮都是空转，连续 2 轮即收敛，
  总轮数 2 —— 与旧的固定 `samples=2` 完全持平，不多花一分钱。
- gin#4709 是此前记录过「低频高置信误报 BLOCK」的样本，本次三轮独立运行均为 PASS。

## 二、多漏洞样本：召回

自建样本，一次改动里埋 6 处互不相同的真实缺陷，检验饱和策略能否挖全。

```
[saturation] round 1: pool 0 -> 6 (new findings)
[saturation] round 2: pool 6 -> 6 (no new)
[saturation] round 3: pool 6 -> 6 (no new)
[saturation] converged after 3 round(s)
[secrets] 1 deterministic secret finding(s) (post-judge merge)
```

判定 `BLOCK`，`incomplete=false`，**6 处全部命中，无误报**：

| 缺陷 | 位置 | 严重度 | 置信度 |
|---|---|---|---|
| SQL 注入（字符串拼接） | app.py:7 | high | 1.00 |
| SSRF（无 allowlist） | app.py:15 | high | 1.00 |
| 路径穿越 | app.py:23 | high | 0.95 |
| 硬编码 Stripe live key | app.py:3 | high | 0.95 |
| 不安全 pickle 反序列化 | app.py:19 | high | 0.90 |
| 命令注入（`shell=True`） | app.py:11 | high | 0.78 |

要点：

- 有发现时比 clean 变更多跑一轮（3 vs 2）——第 1 轮挖出后，仍要连续 2 轮确认无新增才收手。
  这正是饱和策略的设计意图：成本随「还挖不挖得到东西」自适应，而不是固定付费。
- 硬编码密钥由确定性前置扫描命中，在证伪 Judge **之后**合并，未被证伪阶段误杀。

## 三、跑前成本估算

`[cost] … · 6 agents` —— 估算基数取轮数上界 `max_rounds`，不是实际轮数。

饱和轮数跑前无法预知，若按固定采样估算会低到实际的三分之一，`--max-cost` 会放行
本该拦下的运行。上界估算偏保守，但预算守卫宁可保守拦下，也不该给成本惊喜。

## 四、本轮评测暴露并修复的缺陷

**`--timeout` 语义被改坏**（已修）。旧 security 并行跑固定采样，`--timeout` 天然是总墙钟；
饱和改成串行多轮后，若每轮各拿一份完整超时，`--timeout 200` 最坏会跑到 20 分钟。

修复：总预算在轮次间检查，每轮只领剩余额度；预算耗尽停在轮次边界并标 `incomplete`。
回归测试 `crates/core/tests/security_timeout.rs` 锁住该行为（修复前跑满 8 轮 3.28s，
修复后 1.14s，预算 1s）。

## 五、已知限制

- **卡死的单个模型请求打不断**。总预算约束的是轮次之间；一个挂住的 HTTP 请求仍会拖住整轮。
  本次评测中 ripgrep#3496 的 `review`（与本次改动无关的纯搬移路径）也曾卡在同一处 20+ 分钟，
  说明根因在代理侧的大请求处理，非编排逻辑。属既有限制。
- 多单元（大 PR）拆包时，security 的成本估算沿用 review 线「强制 samples=1」的语义，
  该情形下会低估。见 `crates/core/src/review/stages.rs` 中 `prepare` 的注释。
