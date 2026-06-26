# 召回(revert)：servo/rust-smallvec#254 — 还原安全修复以重新引入漏洞

- 修复 PR: https://github.com/servo/rust-smallvec/pull/254  (RUSTSEC-2021-0003 insert_many 缓冲区溢出)
- 方法: checkout 修复提交 `9998ba0` 后 git revert -n 把工作区改回**有漏洞**状态, 审查该 diff
- 期望：security/ai_smell 维度指出被还原掉的漏洞

## 还原后的工作区 diff（节选）
```
```

## review
```
ReviewGate 审查中（3 个维度并行：security, logic, ai_smell）…
闸口：BLOCK ✗ 阻断合并    3 文件改动 · 1 条可信发现 · 0 条已过滤

src/lib.rs
  ✗ [logic · high · conf 0.81] L1042
    溢出路径中的 `self.reserve(1)` 在 `self.set_len(0)` 之后调用，此时 `len=0`。对于 inline 的 SmallVec，`triple_mut()` 返回的 cap 为 `inline_capacity()`，所以 `cap - 0 >= 1` 总是成立（只要 inline_capacity ≥ 1），`reserve(1)` 不会触发扩容。但当迭代器实际产生的元素数量超过 inline 缓冲区容量时，`ptr::copy(cur, cur.add(1), old_len - index)` 和后续 `ptr::write` 会写入超出 inline 数组边界的内存，造成**缓冲区溢出（out-of-bounds write）**。旧代码通过 `self.insert()` 逐个插入溢出的元素来处理此情况，新代码将此逻辑合并到循环中但 `reserve` 无法正确检测到需要 spill 到堆上。被删除的 `test_insert_many_overflow` 测试正是覆盖此场景。（另由 ai_smell 维度同时标记）
    ↳ 建议：在溢出路径中，不应依赖 `reserve(1)` 来判断是否需要扩容，因为 `set_len(0)` 破坏了长度信息。可以考虑：(1) 在溢出前先恢复 len 再调用 reserve，或 (2) 在溢出时检查是否已 spilled，若未 spilled 且 `index + num_added + (old_len - index) > inline_capacity()` 则强制 grow，或 (3) 像旧代码一样对溢出元素使用 `self.insert()` 逐个插入。

```
