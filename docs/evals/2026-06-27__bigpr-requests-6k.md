# 大PR实测[requests-6k]: f8bec2f @ 预算 6000 tok
规模:  19 files changed, 73 insertions(+), 72 deletions(-)

## units 编排
  [units] diff 超输入预算（6000 tok），切成 2 个审查单元；多单元下采样固定为 1（控成本）
- 预检超预算次数: 2 (修复后应≈0)
## 结果
- decision: "decision": "warn"
- incomplete: "incomplete": true
- findings 条数: 2
- 样例:
