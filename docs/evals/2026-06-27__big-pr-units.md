# 大 PR 实测：axios 847d89b(URL对象支持,9文件) + 小预算(2000tok)强制多单元

改动规模:  9 files changed, 272 insertions(+), 29 deletions(-)

## 编排观察(verbose)
  [units] diff 超输入预算（2000 tok），切成 5 个审查单元；多单元下采样固定为 1（控成本）
⚠ 文件 [lib/core/Axios.js] diff 超输入预算（约 1599 tok），已跳过未审
  [security] 第 1 轮预检超预算（估算 3381 > 2000 tok），提前收尾（保留 0 条）
  [logic] 第 1 轮预检超预算（估算 4229 > 2000 tok），提前收尾（保留 0 条）
  [ai_smell] 第 1 轮预检超预算（估算 3237 > 2000 tok），提前收尾（保留 0 条）
  [security] 第 1 轮预检超预算（估算 3171 > 2000 tok），提前收尾（保留 0 条）
  [logic] 第 1 轮预检超预算（估算 4019 > 2000 tok），提前收尾（保留 0 条）
  [ai_smell] 第 1 轮预检超预算（估算 3027 > 2000 tok），提前收尾（保留 0 条）
  [security] 第 1 轮预检超预算（估算 3400 > 2000 tok），提前收尾（保留 0 条）
  [logic] 第 1 轮预检超预算（估算 4248 > 2000 tok），提前收尾（保留 0 条）

## 结果
- decision: "decision": "warn"
- incomplete: "incomplete": true
- warnings:   12 "kind": "incomplete"    1 "kind": "oversized" 
- findings(前4条):
    · 该文件 diff 超出输入预算（约 1599 tok > 2000），已跳过未审；请拆分改动或调大 max_input_tokens
    · 上下文超输入预算，发送前预检提前收尾，该维度未审完
    · 上下文超输入预算，发送前预检提前收尾，该维度未审完
    · 上下文超输入预算，发送前预检提前收尾，该维度未审完
