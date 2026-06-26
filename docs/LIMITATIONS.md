# 已知局限（诚实边界）

ReviewGate 是**静态 LLM 质量闸口**，不执行代码。以下边界来自真实评测（见 `docs/evals/`），
明示局限是为了**可信**——宁可说清楚做不到什么，也不夸大。

## 1. 细微算法 off-by-one / 进位 / 需"真正执行"才暴露的 bug
- big.js 大数运算 base-10 进位错误（#125，**仅 ai_smell low → WARN**，未定性为精度 bug）。
- DST(#1003)/parseISO 24:00(#1229)/负零差值(#739) 经「执行推演」提示改进后已能 BLOCK。
- **进展（2026-06-26）**：revert `addBusinessDays` 周末修复（date-fns #1486）后，当前版本 **2/2 稳定 BLOCK**
  （见 `docs/evals/2026-06-26__addbusinessdays-retry.md`），命中周末分支的死循环/off-by-one——
  此前记录的"addBusinessDays 周末漏报"已不再静默放行。
- **诚实边界（仍未根除）**：
  1. 这次命中靠 logic 维度的**静态执行推演**，`run_check` **并未被调用**（exec-verify 开着也没用上）——
     瓶颈仍是"模型是否主动怀疑并验证"，执行工具只在模型确有怀疑/用户显式要核时才发挥作用。
  2. 大仓库上有维度**撞超时**（已标 incomplete、未静默放行，但该维度未审完）。
  3. 命中的是较显眼的周末分支缺陷；**最细微的单步 off-by-one / 逐位进位**仍可能误推演，属理论硬尾。
- **可靠防线仍是单元测试**：纯"读+推理"难稳定模拟多次迭代的边界算法，关键算法逻辑不要仅依赖静态审查。

## 2. 无上下文信号的"裸"危险调用
- 例：3 行 `requests.get(url)` 且无任何"url 来自用户"的线索——模型不一定判定为 SSRF。
- **缓解**：真实代码通常带 handler/请求参数上下文（带上下文时召回良好）；高危类型可用 `--samples N` 提升稳定性。

## 3. run-to-run 方差
- LLM 固有：同一改动多次运行命中的发现集合会有波动；borderline 置信度会在 WARN/BLOCK 间抖动。
- **缓解**：dedup + 证伪 judge 收敛；`--samples N` 取并集 + 保留最高置信度，稳定 flaky 闸口判定。

## 4. 未支持语言的精确工具降级
- tree-sitter 仅覆盖 rust/cpp/python/go/js/ts；其它语言（含仓颉等）**LLM 审查照常可用**，但
  `find_definition/callers/references` 走 grep 词法兜底、`find_duplicate_functions` 不可用、
  `<lang>.md` per-language 规则不路由。补一种语言＝加 tree-sitter grammar + 扩展名映射。

## 5. 仅审查 diff（改动）
- 默认只评审本次改动；不主动审计仓库既有代码。删除安全防护这类「负向改动」已专门覆盖
  （prompt 例外规则 + cJSON 删除式漏洞验证通过）。

---
这些局限均有评测留痕。随版本演进会持续缩小（每条都标注了已做/可做的缓解）。
