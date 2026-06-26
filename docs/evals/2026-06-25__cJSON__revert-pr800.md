# 召回(revert)：DaveGamble/cJSON#800 — 还原安全修复以重新引入漏洞

- 修复 PR: https://github.com/DaveGamble/cJSON/pull/800  (C 堆越界(删除检查) no-regression)
- 方法: checkout 修复提交 `3ef4e4e` 后 git revert -n 把工作区改回**有漏洞**状态, 审查该 diff
- 期望：security/ai_smell 维度指出被还原掉的漏洞

## 还原后的工作区 diff（节选）
```
```

## review
```
ReviewGate 审查中（3 个维度并行：security, logic, ai_smell）…
闸口：WARN ▲ 有需关注的问题    1 文件改动 · 2 条可信发现 · 0 条已过滤

cJSON.c
  ✗ [ai_smell · high · conf 0.73] L1662
    【AI Smell - 过度自信的边界处理】删除了 parse_object 循环内的 cannot_access_at_index(input_buffer, 1) 边界检查，该检查原本用于防止逗号后 EOF 导致的越界读取。删除后，当输入形如 {"key":"val",（逗号后无任何内容即 EOF）时：offset++ 越过逗号使 offset==length，buffer_skip_whitespace 不做修正直接返回，随后 parse_string 中 buffer_at_offset(input_buffer)[0] 将读取 content[length]（越界），触发 undefined behavior。该防护无等价替代。（另由 security 维度同时标记）
    ↳ 建议：恢复被删除的边界检查：if (cannot_access_at_index(input_buffer, 1)) { goto fail; }
  ✗ [logic · high · conf 0.76] L1663
    删除 `cannot_access_at_index(input_buffer, 1)` 检查导致缓冲区越界读取。当 JSON 对象以逗号结尾（如 `{"a":1,`）时：① do-while 条件通过，offset 指向逗号；② `input_buffer->offset++` 使 offset == length；③ `buffer_skip_whitespace` 检测到 `cannot_access_at_index(buffer, 0)` 为真，直接返回而不执行 offset-- 回退；④ 随后的 `parse_string` 中 `buffer_at_offset(input_buffer)[0]` 解引用 `content + length`，读取 buffer 末尾之外的内存。被删除的检查正是在 offset++ 之前阻止这种越界的防护。
    ↳ 建议：恢复被删除的检查，或采用等效的防护措施：在 `input_buffer->offset++` 之后、调用 `buffer_skip_whitespace` 和 `parse_string` 之前，确保 offset 仍在有效范围内。

```
