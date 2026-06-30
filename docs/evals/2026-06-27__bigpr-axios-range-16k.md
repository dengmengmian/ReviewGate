# 大PR实测[axios-range-16k]: HEAD~30..HEAD @ 预算 16000 tok
规模:  55 files changed, 4142 insertions(+), 823 deletions(-)

## 编排 (耗时 312s)
  [units] diff 超输入预算（16000 tok），切成 8 个审查单元；多单元下采样固定为 1（控成本）
- 首轮(round1)超预算: 0 (应=0)
- 取context后(round≥2)优雅收尾: 8
- 跳过(单文件超预算)未审: 1
## 结果
- decision: "decision": "warn"
- incomplete: "incomplete": true
- findings 总条数: 12
- 实质发现样例:
  · 测试用例 `should not advertise zstd by default` 中，如果 `accept-encoding` 请求头不存在，`acceptEncoding`
