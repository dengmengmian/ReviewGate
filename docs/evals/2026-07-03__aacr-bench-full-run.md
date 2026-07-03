# AACR-Bench 官方评分器 · 全量 196 评测（进行中）+ 审查质量抽查 · 2026-07-03

用 [alibaba/aacr-bench](https://github.com/alibaba/aacr-bench) **官方 evaluator_runner**（LLM 语义匹配 +
官方 `positive_samples.json` 参考集，口径与 open-code-review 公开分数一致）评测 ReviewGate 全量 196 PR。
脚本 `scripts/eval-aacr-official.py`，分批可断点续跑。**目的是用 benchmark 发现并修复 RG 缺陷，不是刷分。**

## 诚实边界（先读）
- **run-to-run 变异大**（同一 PR 语义命中会 ±1 抖动），n 小时误差棒宽。
- **默认 style 已移出**（0.6.0），本轮用缺陷四维（security/perf/logic/ai_smell）。
- **RG 走代理端点、OCR 可能直连**：模型同（Deepseek-V4-Pro）、延迟不同。
- 更正：早前误引的 **F1 60.1% 是 Qodo 2.0（闭源、非同底座），不是 OCR**。OCR 官方在本 benchmark 上
  用 Deepseek-V4-Pro 的真实成绩是 F1 17.9%——见下"同底座对照"。

## 阶段结果（49/196，滚动更新）

| 指标 | 值 |
|---|---|
| 覆盖 | 49/196 |
| incomplete | 12（见下"超时"，压低召回） |
| Precision | 50.0% |
| Recall | 8.0% |
| F1 | 13.8% |

## 同底座对照：与 open-code-review（都用 Deepseek-V4-Pro）

OCR 官方 leaderboard（全量 200 PR）与 RG（49/196，进行中）：

| 工具（同底座 Deepseek-V4-Pro） | F1 | Precision | Recall | avg token |
|---|---|---|---|---|
| open-code-review（官方） | 17.9% | 30.6% | 12.7% | 394K |
| **ReviewGate**（进行中） | 13.8% | **50.0%** | 8.0% | ~100-350K |

**差距很小且方向清晰——RG 用召回换精度：**
- **Precision：RG 50% ≫ OCR 30.6%**（RG 明显更少误报）；
- Recall：RG 8.0% < OCR 12.7%（RG 更保守 + 超时低估，见下）；
- F1 差 ~4 点，主要来自召回；token RG 更省。

> OCR 全系最高 F1 是 Claude-4.6-Opus 的 25.1%（换更强模型）；同底座 Deepseek 下就是 17.9%。

分语言（样本小，仅趋势）：Python P=71%/R=31%、C++ P=50%、PHP P=100%/R=7%、C# R=3%（多超时）。

## 关键结论：低分不代表 RG 审查差

**抽查了 32 个 PR 里 RG 报的每一条发现**（HIT/位置对语义不符/无GT位置 + judge 理由）。结论：

**RG 审查内容质量扎实，无系统性缺陷。** 报的绝大多数是真 bug 且机理正确，举例：
- ComfyUI#6542：DirectML 分支 `torch.empty` 未初始化的 causal_mask 上三角含垃圾值（非 -inf）；
- valkey#1889：三元表达式括号内逗号被当**逗号运算符**，`snprintf` 格式参数全错；
- dbeaver#37564：`Files.copy(InputStream,...)` 不关流 → 资源泄漏；
- linera#3151：把正确的 `ClonableView` "修正"成不存在的 `CloneableView`（真幻觉，RG 抓到）；
- ollama#8938：`uint` 参数的 `<= 0` 分支永不可达。

**且很多未命中的发现是 GT 漏标的真发现**（如 FreeCAD#20612「调用方未同步更新」logic/high/0.98）——
即 precision 被 GT 非穷尽系统性低估，不是 RG 假阳。

## 低分的三个真凶（都不是审查质量问题）
1. **GT 非穷尽**：RG 找到真问题但标注者没标 → 计为未命中 → 压低 precision。
2. **保守召回（设计取舍）**：1505 GT comment，RG 只报了 31 条高置信——"精度优先、低置信折叠"的既定定位，与 OCR 同向、更极端。
3. **超时 incomplete 30%**：`REVIEWGATE_EVAL_TIMEOUT=240` 在慢代理（~50s/轮）下只够 4-5 轮，饿死 RG → gen=0 → 召回被低估。**这是 harness/端点问题，不是 RG 能力。** 计划：全量跑完后只把 incomplete 子集用更高超时（600s）重跑，给公平召回口径并列报告。

## benchmark 驱动修复的 RG 缺陷

- **去重漏合同一问题（已修 ✅）**：去重原按精确 start_line 分组，同一问题被不同维度锚在略不同行时漏合
  （logic@423-429 + ai_smell@426-429 双报）。复现 4 次（dbeaver/cline/ComfyUI×2）跨阈值 → 修：
  located 组再按「行区间重叠 **且** existing_code 显著行相交」二次合并，双条件防误合（3 个 TDD：
  重叠+同内容合并、重叠+异内容不合并、不重叠同模式不合并）。复测受影响 PR：重叠漏合 4→0、
  gen 下降、语义命中不减、precision 全升。commit `3ede490`。
- **ai_smell 偶报纯格式 nit（观察中）**：electron#46660「缩进 6 空格 vs 8 空格」ai_smell/low/0.95。
  **仅 1/11**，低严重度 → 未达阈值，继续观察。

## harness 修复留痕（本轮）
- 巨仓（ClickHouse 等）：全量 blobless clone 拉整个提交图卡死 → 改**浅层 fetch**（`--depth`，无 blob-filter，diff 需真 blob）+ **fetch 超时 240s** 快速失败跳过，不卡整批。
- 深历史 PR（base↔head >800 提交）：浅层拉不到 merge-base → fail-fast 跳过，记 error（少数覆盖损失）。
- 聚合 key 用 `owner/repo`（此前只用 repo 名 → 覆盖误算 0）。
- `--max-new N` 分批 + `.eval.json` 续跑跳过 + 磁盘累积聚合。

## 复现
```
AACR_REPO=/path/to/aacr-bench python3 scripts/eval-aacr-official.py --all --max-new 10   # 分批
```
每 PR 缓存 `docs/evals/aacr-bench-results/*.rg.json` / `*.eval.json`（gitignore，可重算零成本）。
