# 大PR实测[gin-range-16k]: HEAD~80..HEAD @ 预算 16000 tok
规模:  46 files changed, 2063 insertions(+), 481 deletions(-)

## 编排 (耗时 240s)
  [units] diff 超输入预算（16000 tok），切成 6 个审查单元；多单元下采样固定为 1（控成本）
- 首轮(round1)超预算: 0 (应=0)
- 取context后(round≥2)优雅收尾: 4
- 跳过(单文件超预算)未审: 0
## 结果
- decision: "decision": "block"
- incomplete: "incomplete": true
- findings 总条数: 13
- 实质发现样例:
  · AI 幻觉：`actions/checkout@v6` 不存在。GitHub 官方 `actions/checkout` 的最新主版本为 v4（当前最新为 v4.2.x），不存在 
  · 复制粘贴残留：错误信息中引用了不存在的函数 `maskHeaders()`，但实际测试的函数是 `secureRequestDump`。这是从旧的 `TestMaskAuthori
  · `#nosec G112` 显式抑制了 Slowloris DoS 漏洞警告，但未设置任何超时防护。`http.Server` 缺少 `ReadTimeout`、`WriteTim
  · `#nosec G112` 显式抑制了 Slowloris DoS 漏洞警告 — RunUnix 中的 http.Server 未设置超时防护。
