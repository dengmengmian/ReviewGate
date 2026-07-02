# ai_smell「找不到=不存在」过度自信假阳性 — 复现与修复 — 2026-07-02

## 问题

ai_smell 会把「本仓库 `find_definition`/`code_search` 搜不到的符号」直接判为
「该 API 不存在 / AI 幻觉」，且给 **high · ~100% 置信 → BLOCK**。但这些工具**只搜被审仓库**，
外部依赖 / 标准库 / 内核·系统头 / 框架内部 / 生成代码都在仓外、搜不到——真实外部符号会被误杀。

首次是在跑 openEuler kernel MR #24448（`krealloc_array`，CVE-2026-53213 修复）时发现：
片段复现里被 BLOCK，理由「krealloc_array 不存在、是幻觉」。但 `krealloc_array` 是真实内核 API
（真内核 `include/linux/slab.h` 可 grep 到，自 v5.15 起）。

## 复现（真实场景，非片段）

构造一个**像样的 out-of-tree 内核模块仓库**（`ringbuf.c` + `Makefile`，完整结构；OOT 模块**天然不含**内核头文件）。
diff 把 `krealloc` 改成 `krealloc_array`（真实内核推荐做法）：

- **case ①（真外部）** `krealloc_array` → **BLOCK, ai_smell high 100%**「不存在/幻觉」= **假阳性**。
- **case ②（真幻觉）** `krealloc_checked_array`（内核 realloc 家族无此名）→ **BLOCK, high 0.99** = 正确。

两者都 BLOCK，且都因「仓内搜不到」——工具无法区分「真外部」与「真幻觉」，这是问题核心。
（值得注意：case ② 的 message 里模型正确写出「正确名是 krealloc_array」，却在 case ① 里反口说它不存在——
说明"仓内缺失 + 模型凭名字猜"是不可靠的 high-confidence 依据，run-to-run 会摇摆。）

## 修复

`crates/core/prompts/dimensions/ai_smell.md` 的 Hallucination 项加 guard：
- **找不到 ≠ 不存在**：这些工具只搜本仓；搜不到往往是真实外部符号。
- **不得**仅凭「仓内找不到定义」就高危判「不存在/幻觉」。
- 只有**正面证据**才判幻觉：找到定义但签名/元数不符；本 diff 新引入却全无定义；或有可靠的具体知识确证该 API 不存在（别凭名字模式猜）。
- 无法核实时最多**低置信**「无法核实 `X`（仓内未见，可能来自外部）」，绝不断言不存在、绝不作为 BLOCK 的唯一理由。

## 验证（改后各跑 2 次，ai_smell 维度）

| 用例 | 改前 | 改后 |
|---|---|---|
| ① `krealloc_array`（真外部，不该报） | BLOCK high 100% | **PASS / PASS** ✓ 假阳消除 |
| ② `krealloc_checked_array`（真幻觉，该报） | BLOCK high 0.99 | **BLOCK / BLOCK**（0.98 / 1.0）✓ 正职保留 |

区分得开的原因：case ② 模型有正面知识（能给出正确名）→ guard 允许报；
case ① 拿不出「它是假的」的正面证据 → 不再凭仓内缺失误报。

## 诚实边界
- prompt 改属 LLM 行为引导，**不是 100% 保证**——run-to-run 仍可能偶发波动，这里是「显著降低假阳、保留召回」。
- 残留成本：一个「模型也不认识、且仓内没有」的真幻觉会被降级为低置信而非 BLOCK——但此时模型确实无法区分它与真外部符号，降级为「无法核实」是诚实的校准，而非漏报式掩盖。
- 本轮只在该失效点做了前后对照（各 2 次）；更广的 clean 集回归可选（guard 属窄范围、对 clean 集只会减少假阳）。
