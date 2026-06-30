# 大PR实测[ripgrep-range-12k]: HEAD~40..HEAD @ 预算 12000 tok
规模:  28 files changed, 397 insertions(+), 261 deletions(-)

## 编排 (耗时 124s)
  [units] diff 超输入预算（12000 tok），切成 4 个审查单元；多单元下采样固定为 1（控成本）
- 首轮(round1)超预算: 0 (应=0)
- 取context后(round≥2)优雅收尾: 4
- 跳过(单文件超预算)未审: 1
## 结果
- decision: "decision": "warn"
- incomplete: "incomplete": true
- findings 总条数: 6
- 实质发现样例:
  · AI 幻觉：`[u8]` 类型上不存在 `as_bytes_mut()` 方法。`free_buffer()` 返回 `&mut [u8]`，而 Rust 标准库和 `bstr` 
