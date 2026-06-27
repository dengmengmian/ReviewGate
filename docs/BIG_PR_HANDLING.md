# 大 PR / 大 diff 的处理（机制 · 保证 · 实测）

ReviewGate 面向「AI 一次改很多文件」的场景，因此对超出模型上下文窗口的大 diff 有一套自适应处理，核心目标是：**宁可标记「没审完」，也绝不把大 PR 当成干净 PASS 放行。**

## 机制

1. **审查单元（`plan_units`）**——输入预算 = provider 的 `max_input_tokens`（默认 200k）。diff 在预算内时整包为 **1 个单元**（正常 PR 零退化）；超预算时按目录就近装箱，切成多个单元分别审。
2. **固定开销预留**——切分时预留「系统提示词 + 维度 focus」的固定开销（`overhead = estimate(system) + max(focus) + 余量`，`plan_budget = budget − overhead`）。保证每个单元在 **首轮一定可发送**，不会切出一堆「一发就超预算、审不到任何东西」的单元。默认 200k 预算下为无感知 no-op。
3. **多单元采样固定为 1**——大 PR 本就庞大，不再叠 `--samples`，避免 `单元 × 维度 × 样本` 的成本放大；多采样只在单单元（正常 PR）上用于提升 flaky 漏报的召回稳定性。
4. **逐单元自适应降级**——单元 prompt 先带「改动文件的完整新版本」上下文；若超预算 → 退化为 diff-only；仍超（单文件 diff 本身就超预算）→ 跳过该单元并标记 `oversized`（计入未审完，绝不静默放行）。
5. **Agent 内每轮预检**——审查过程中 Agent 会用工具（read_file / grep 等）按需取上下文。若累积对话超预算，下一轮发送前预检会**优雅收尾**：保留本轮已得到的发现，停止继续取，而不是崩溃或静默截断。

## 保证：绝不静默通过

任何单元被跳过、超时、请求失败或提前收尾，结果都会：

- `incomplete = true`，并产生对应 `warnings`（`oversized` / `timed_out` / `failed` / `incomplete`）；
- 判定降级为 **WARN**（不会是 PASS）；
- 配合 `--fail-on`（CI 中）可使进程非 0 退出，避免「没审完」被误读为「干净通过」。

## 实测（2026-06-27）

用真实开源 PR / 多提交范围压测大 diff（修复了一处「固定开销未预留导致单元首轮全部超预算」的健壮性 Bug 后）：

| PR / 范围 | 语言 | 规模 | 预算 | 单元数 | 首轮超预算 | 发现 | 判定 |
|---|---|---|---|---|---|---|---|
| axios `847d89b` | JS | 9 文件 | 4k | 8 | **0** | 13 | WARN+incomplete |
| requests `f8bec2f` | Python | 19 文件 | 6k | 2 | **0** | 2 | WARN+incomplete |
| axios `HEAD~30..HEAD` | JS | **55 文件 / ~5000 行** | 16k | 8 | **0** | 12 | WARN+incomplete |
| ripgrep `HEAD~40..HEAD` | Rust | 28 文件 | 12k | 4 | **0** | 6\* | WARN+incomplete |
| gin `HEAD~80..HEAD` | Go | 46 文件 / ~2500 行 | 16k | 6 | **0** | 13\*\* | **BLOCK**+incomplete |

\* 含一处真实 **AI 幻觉 API** 捕获（`[u8]` 上不存在 `as_bytes_mut()`）。
\*\* 含真实 **Slowloris DoS**（`#nosec G112` 抑制告警却未设超时）+ **复制粘贴残留**（错误信息引用不存在的 `maskHeaders()`）——大 PR 上同样能给出 BLOCK。

结论：

- 跨 **4 种语言（JS / Python / Rust / Go）** 验证，最大 **55 文件 / ~5000 行**：切单元后**首轮超预算全部归零**，均产出真实发现（含 AI 幻觉、复制粘贴残留、Slowloris DoS 等）。
- 大 PR 既能 WARN（未审完）也能 **BLOCK**（命中高置信问题）——切分不削弱召回。
- 「绝不静默通过」不变量全程成立：凡有单元跳过 / 提前收尾，一律降级且 `incomplete = true`。
- 残留的 round≥2 超预算均为 Agent 取上下文后的**优雅收尾**（保留已得发现），属预期安全行为。

详细单次记录见 [`docs/evals/`](evals/)（文件名 `2026-06-27__bigpr-*.md`）。

## 调参

| 参数 | 作用 |
|---|---|
| `max_input_tokens`（provider 配置） | 输入预算，决定何时切单元。小上下文模型可调小；大模型保持默认 200k。 |
| `--samples N` | 单单元 PR 的每维度采样次数（>1 取并集提召回，成本 ×N）；多单元下自动固定为 1。 |
| `--timeout S` | 单维度墙钟上限，超时跳过该维度并保留其余（计入未审完）。 |
| `--fail-on block\|warn\|never` | CI 中由哪种判定触发非 0 退出；未审完会降级 WARN。 |
