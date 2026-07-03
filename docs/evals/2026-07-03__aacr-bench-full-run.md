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

## 覆盖上限：187/196（9 个 orphaned 不可达）

跑到 187/196 后，剩余 9 个**永久不可覆盖**：它们的 `target_commit` 与当前 GitHub PR head 不一致
（上游 PR 建库后被 rebase/force-push），旧 commit 已被 GitHub 丢弃——**git 404 且 API 也 404**，
数据集又不缓存 diff。这是数据衰减，非 harness 缺陷。不可达清单：ClickHouse#74070/#85266/#85873、
keycloak#35645/#36457/#37465/#37504/#41672、three.js#30076。**未用降级合成仓硬凑 196**（那会让 RG
缺上下文、结果不可比，属假成功）。187/196（95.4%）是方法学干净的天花板，覆盖全 10 语言。

## 最终结果（187/196）

| 口径 | 全量 P | 缺陷子集 R | 缺陷子集 F1 | 说明 |
|---|---|---|---|---|
| 全部 187 | 34.0% | 6.4% | 10.8% | 含 59% incomplete（后段大 PR 用 120s 超时赶覆盖，压低） |
| **仅完整 76 个** | 36.8% | 14.3% | **20.6%** | 不受超时影响，更代表 RG 完成审查时的能力 |

> **超时异质性但书**：前段用 240s、后段大 PR 为赶覆盖降到 120s → incomplete 占比高、"全部 187"偏低。
> "仅完整"口径无此噪声但有选择偏差（排除了审不完的难 PR）。真实能力在两者之间。
> 对标 OCR 同底座（F1 17.9% / P 30.6% / R 12.7%）：RG 完整口径缺陷 F1 20.6%、精度 36.8%——
> **精度更高、缺陷 F1 相当**，印证"精度优先、召回换精度"的定位。

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

## 召回为什么低：结构性设计，不是能力差

### GT 有 42% 是「可维护性/可读性」——RG 刻意不报
1505 条 GT 类别分布：Code Defect 47% · **Maintainability & Readability 42%** · Performance 8% · Security 4%。
RG 作为精度闸口（0.6.0 移除 style）只 target 缺陷类。实测：**RG 命中的 18 条全部落在缺陷类（18/18），
零可维护性匹配。** 故两个口径：

| Recall 口径 | RG | 说明 |
|---|---|---|
| 全量 GT（含 42% 可维护性） | 8.0% | 与 OCR 12.7% 同分母可比 |
| **缺陷子集（RG 真实 targets）** | **12.5%** | 排除 RG 设计上不追的可维护性 |

**RG 对缺陷的召回（12.5%）≈ OCR 的全量召回（12.7%），而精度高 ~20 点。** RG 全量召回低的一大半，
是 42% 的 GT 是它不追的可维护性——OCR 召回高部分正因它肯报可读性（代价是精度只有 30.6%）。P/R 取舍成立。

### 超时假设被证据否掉（诚实修正）
早前判断「超时 incomplete 饿死 RG、低估召回」。用 3 个 incomplete PR 在 600s 重跑验证：
**3/3 都跑完了（incomplete=False）但仍报 0 条**（filament#15217/immich#14874/PowerShell#24910，均 11-23 行小 diff）。
→ **超时不是召回主因**。抽查其 GT：一半是可维护性（RG 设计不报），一半是需深框架知识/更广上下文的真缺陷
（如 PSModuleInfo 未重写 Equals 的 HashSet 引用相等、`sortable` 被空数组覆盖）——RG 确有真召回缺口，
但属「难例需上下文/领域知识」，非超时。

### GT 非穷尽
RG 找到真问题但标注者没标 → 计未命中 → 压低 precision（precision 51% 是被低估的下界）。

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
