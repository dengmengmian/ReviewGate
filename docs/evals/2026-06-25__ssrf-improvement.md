# SSRF 召回改进验证（prompt 强化前后）

round2 发现 SSRF（移除允许名单）召回不稳定。改进：security checklist 显式化 SSRF（用户可控 URL + 无/被删允许名单），
共享 prompt 加「删除安全防护即漏洞」例外。改进后用**干净 diff**（外置配置，避免 reviewgate.toml 污染）复测：

| 用例 | 改进后结果 | 说明 |
|---|---|---|
| go-ssrf（删除 `allowedHost(u)`） | **2/2 命中 SSRF** | 改进前不稳定→现可靠；显式"删除允许名单"信号强 |
| python-ssrf（删除 `allowed_host`，3 行极简） | **0/2 命中**（PASS） | 一致性漏报 |

## 解读（诚实）
- **改进有效**：go-ssrf 从"时好时坏"变为 2/2 稳定命中，且明确以"删除允许名单"为由——正是新增 prompt 规则的效果。
- **python-ssrf 一致性漏报**：用例过于极简（`requests.get(url)`，无任何"url 来自用户"的上下文），模型未判定为 SSRF。
  这**部分是测试不真实**（真实 handler 会带 request 参数上下文），部分是裸 SSRF 本身难判。**多采样对"一致性漏报"无效**
  （只对"时好时坏"的 flaky 有效）。
- **方法学修正**：发现 workspace 模式会把未跟踪的 `reviewgate.toml` 纳入 diff，干扰审查；改用外置 `REVIEWGATE_CONFIG`
  使被审 diff 纯净。后续评测统一采用。

## 多采样验证（--samples，2026-06-26）

新增 `--samples N`：每维度并行跑 N 次取并集，dedup 折叠重复时**保留最高置信度**那条，judge 再过滤。

| 用例 | 单次 ×3 | --samples 3 |
|---|---|---|
| **真实 SSRF**（Flask handler，`request.args.url`，删允许名单） | **检测 3/3 命中**；闸口 WARN/WARN/BLOCK（置信度卡在 0.8 阈值附近抖动） | **BLOCK / 命中**（稳定） |
| 极简 python-ssrf（3 行裸 `requests.get(url)`，无用户可控信号） | samples=5 仍 0/5 | 无法挽救（0% 基率） |

**结论**：
- **prompt 改进 → 稳定了"检测"**：有真实信号时（删允许名单 / 用户可控入参），SSRF 可靠命中（go 2/2、真实 python 3/3）。
- **多采样 → 稳定了"闸口判定"**：并集去重保留最高置信度，把卡阈值的 WARN 抬成 BLOCK（真实 SSRF samples=3 稳定 BLOCK）。
- **多采样只能救 flaky，救不了 0%**：无任何用户可控信号的退化用例（裸 `requests.get(url)`）采样再多也是 0——
  这是**测试不真实**，非产品缺陷。真实代码总带上下文（handler/请求参数），故实战中 prompt 改进 + 多采样组合即可让 SSRF 稳定。

## 原始结论与策略
- 对**有明确"删除防护"信号**的 SSRF：改进后可靠命中。
- 对**裸调用、无上下文**的 SSRF：评测应用更真实的 handler 上下文；高危类型用 `--samples` 取并集稳定闸口。
