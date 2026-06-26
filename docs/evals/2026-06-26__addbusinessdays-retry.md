# addBusinessDays 硬尾再攻关(revert #1486 周末修复 + --exec-verify + 执行推演)

原修复: Fix addBusinessDays for weekends (#1486) (closes #1485)
改动文件: 

## run 1 — decision: block
    · 墙钟超时，该维度未审完（已保留其部分发现）
    · 当 `amount = 0` 且输入日期为周末时，循环条件 `while (amount || isWeekend(date))` 会导致无限循环。具体场景：`addBusinessDays(周六, 0)` 或 `addBusinessDays(周日, 0)`，以及 `addBu
工具调用是否含 run_check: 0

## run 2 — decision: block
    · 当 `amount` 为 5 的倍数且起始日期为周末时，循环会陷入死循环。具体场景：`amount % 5 === 0` 且 `isWeekend(date)` 为真时，周末修正会将 `amount` 从 0 变为 -1（正向）或 1（反向）。由于 JavaScript 中非零数
工具调用是否含 run_check: 0


---

## 结论：周末分支已不再静默漏报(2/2 BLOCK),但靠的是静态推演而非 run_check
- revert date-fns #1486(addBusinessDays 周末修复)后,当前版本(英文 prompt + 执行推演 logic 提示 + samples=2)
  **两次都 BLOCK**,命中周末分支的死循环/off-by-one(`while(amount||isWeekend(date))` 在 amount=0/周末时不终止)。
  此前 LIMITATIONS 记录的"addBusinessDays 周末漏报"在本场景下已被稳定捕获。
- **诚实点**:
  1. `run_check` 调用次数 = 0 —— exec-verify 开着但模型没用上,命中靠 **logic 维度静态执行推演**。
     说明真正起作用的是"执行推演"提示,不是我建的执行工具;瓶颈仍是模型主动怀疑的意愿。
  2. 有维度撞 320s 超时(标 incomplete,未静默放行),但该维度未审完。
  3. 这是较显眼的周末分支缺陷;最细微的单步 off-by-one/逐位进位仍属理论硬尾,可靠防线仍是单测。
