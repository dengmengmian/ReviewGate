# AACR-Bench × ReviewGate —— 官方评分器（语义匹配）评测 · 2026-07-03

用阿里 [alibaba/aacr-bench](https://github.com/alibaba/aacr-bench) 的**官方 evaluator_runner**
（LLM 语义匹配 + 官方 `positive_samples.json` 参考集）评测 ReviewGate，口径与 open-code-review 公开分数一致。
脚本：`scripts/eval-aacr-official.py`。样本：sample.tsv 的 12 个 PR（C/C#/C++/Go）。

## 诚实边界（务必先读）

- **非同底座对照**：RG 与 LLM judge 都走本地 deepseek 端点；OCR 用它自己的模型。这比的是
  「RG 按此配置」vs「OCR 按其公开配置」，**不是控制变量后的工具对工具**。任何"RG vs OCR"的直接结论都不成立。
- **run-to-run 变异大**：同一 PR（lvgl#8689）两次运行语义命中 3↔2 抖动。n=12 误差棒宽，数字看趋势不看小数点。
- **GT 非穷尽**：参考集含 72% AI 协作标注，且未覆盖 RG 报的部分真发现 → precision 被系统性低估（见下）。

## 结果（12 PR，micro，官方语义口径）

| 配置 | Precision | Recall | F1 |
|---|---|---|---|
| 含 style（首轮全量） | 33.3% | 8.5% | 13.6% |
| **不含 style（默认，本次改动后）** | **~57%** | 8.5% | ~15% |
| 参考：OCR 公开 F1（其配置） | — | — | 60.1% |

## 关键诊断：style 维度是精度噪声（已修）

拆解 RG 全部生成 finding 的匹配情况：
- 命中 GT 的 8 条**全部**来自 security/perf/logic/ai_smell；
- **style 维度：10 条发现、语义命中真缺陷 = 0**，纯拖低精度（把 P 从 ~57% 拉到 33%）；
- 未匹配的非 style 发现（调用方未同步更新、释放条件写反、SQL 参数未读、死代码字段…）**多是 GT 漏标的真发现**——不是 RG 假阳，是标注非穷尽。

**修复**：`style` 移出默认维度集（`Dimension::ALL` 4→ security/perf/logic/ai_smell），改为 opt-in
（`--dimensions style`）。理由：合并前**质量闸口不该为纯风格拦截/告警**，交给 linter/formatter——
这与 RG「降噪音」定位、业内标准（CodeRabbit/Qodo 亦精度优先）一致。

**验证（变异受控）**：固定第一轮 RG 输出、事后剔除 style → P 33.3%→57.1%，R 8.5% **不变**
（style 命中=0，无从丢失）；8 个命中置信度均 ≥0.81，即便失去 style 的跨维度加分也掉不出闸口
（阈值 0.5）→ 去 style 不丢任何真命中。当前缓存交叉印证同向（36.8%→50%，style 命中=0）。

## 怎么看这个分数

- **Precision ~57%**：与 OCR 量级可比（其精度优先设计）。剩余差距主因是 GT 非穷尽（RG 真发现未被标注也算未命中）。
- **Recall 8.5% 明显低**：RG 是**极保守的闸口**——12 PR / 89 GT 只报了 14 条高置信非 style 发现。这是
  **设计取舍**（精度优先、低置信默认折叠），不是 bug；提升召回是研究方向（见 LIMITATIONS #1），非可直接修的缺陷。
- 一句话：**RG 在这个基准上"报得少但准"**，方向与 OCR 一致、更极端；style 噪声这个真缺陷已修。

## 复现
```
AACR_REPO=/path/to/aacr-bench python3 scripts/eval-aacr-official.py --pr owner/repo#num ...
```
RG 原始输出与 evaluator 明细缓存在 `docs/evals/aacr-bench-results/*.rg.json` / `*.eval.json`。
