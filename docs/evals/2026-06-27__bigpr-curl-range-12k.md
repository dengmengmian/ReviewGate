# 大PR实测[curl-range-12k]: HEAD~30..HEAD @ 预算 12000 tok (dims=security,logic)
规模:  132 files changed, 1241 insertions(+), 1156 deletions(-)

## 编排 (耗时 104s)
  [units] diff 超输入预算（12000 tok），切成 11 个审查单元；多单元下采样固定为 1（控成本）
- 首轮(round1)超预算: 0 (应=0)
- round≥2 优雅收尾: 15
- 跳过(单文件超预算): 0
## 结果
- decision: "decision": "warn"
- incomplete: "incomplete": true
- findings 总条数: 15
- 实质发现样例:
