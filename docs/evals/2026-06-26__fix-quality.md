# 修复建议质量（suggestion_code / --fix 可用性）

检测之外的另一维度：ReviewGate 给出的**修复建议是否正确、可直接应用**。

## 用例：SQL 注入（参数化 → 字符串拼接）
被审改动：
```js
- return db.prepare("SELECT * FROM users WHERE id = ?").bind(id).one();
+ return db.query("SELECT * FROM users WHERE id = " + id).one();
```
ReviewGate 输出（security，conf 1.0）：
- `existing_code`：`return db.query("SELECT * FROM users WHERE id = " + id).one();`（精确命中改坏的行）
- `suggestion_code`：`return db.prepare("SELECT * FROM users WHERE id = ?").bind(id).one();`
  —— **正确的参数化查询修复**，且与原安全版本一致。

## 结论
- `existing_code ↔ suggestion_code` 成对，可渲染 **red→green diff** 并经 `--fix` 一键应用（人工把关）。
- 即 ReviewGate **不仅能检测，还能给出正确、可应用的修复**——具备生产"可用"价值，而非仅提示。
- 注：修复建议质量同样受检测置信度影响；低置信/WARN 的发现其建议供参考。
