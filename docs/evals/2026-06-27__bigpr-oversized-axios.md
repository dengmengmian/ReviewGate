# 超长单文件 oversized 路径实测: axios de1a8100 @8000
组成: package-lock.json 2467行(超长) + package.json/CHANGELOG 等小文件

## 单元编排
  [units] diff 超输入预算（8000 tok），切成 7 个审查单元；多单元下采样固定为 1（控成本）
## oversized 跳过的文件(关键)
⚠ 文件 [package-lock.json] diff 超输入预算（约 53233 tok），已跳过未审
⚠ 文件 [tests/module/esm/package-lock.json] diff 超输入预算（约 11049 tok），已跳过未审
⚠ 文件 [tests/smoke/esm/package-lock.json] diff 超输入预算（约 10953 tok），已跳过未审
## 结果
- decision: "decision": "warn"
- incomplete: "incomplete": true
- oversized 告警: 3 条
- oversized 告警维度名(应含被跳文件):
  · unit:package-lock.json
  · unit:tests/module/esm/package-lock.json
  · unit:tests/smoke/esm/package-lock.json
- 其余文件是否仍审到(非 oversized 发现):
