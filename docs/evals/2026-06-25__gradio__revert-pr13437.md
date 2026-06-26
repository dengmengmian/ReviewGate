# 召回(revert)：gradio-app/gradio#13437 — 还原安全修复以重新引入漏洞

- 修复 PR: https://github.com/gradio-app/gradio/pull/13437  (GHSA-qqr5-x4m8-g4gq 路径穿越)
- 方法: checkout 修复提交 `97d541f3d5fd05b2587a69ecc94b68fe5d2d7004` 后 git revert -n 把工作区改回**有漏洞**状态, 审查该 diff
- 期望：security/ai_smell 维度指出被还原掉的漏洞

## 还原后的工作区 diff（节选）
```
```

## review
```
ReviewGate 审查中（3 个维度并行：security, logic, ai_smell）…
闸口：BLOCK ✗ 阻断合并    3 文件改动 · 3 条可信发现 · 0 条已过滤

gradio/components/file_explorer.py
  ✗ [logic · high · conf 0.89] L152
    【路径遍历漏洞回归】`preprocess()` 中单文件路径拼接改用 `os.path.normpath(os.path.join(...))` 替代了 `self._safe_join()`，完全绕过了 `safe_join()` 中的路径验证（拒绝绝对路径、`..`、`../` 等）。攻击者可通过传入 `[["/etc/passwd"]]`（绝对路径段使 `os.path.join` 丢弃 `root_dir` 前缀）或 `[["..", "..", "etc", "passwd"]]`（`..` 爬升）逃逸 `root_dir`。这是 GHSA-qqr5-x4m8-g4gq 安全漏洞的回归。（另由 ai_smell/security 维度同时标记）
    ↳ 建议：恢复使用 `self._safe_join(payload.root[0])`，或至少在新路径拼接后调用 `safe_join` / 手动验证结果路径仍在 `root_dir` 内（如 `os.path.commonpath([self.root_dir, result]) == self.root_dir`）
  ✗ [logic · high · conf 0.89] L155
    【路径遍历漏洞回归·多文件分支】与第 152 行相同问题：`preprocess()` 多文件分支同样用 `os.path.normpath(os.path.join(self.root_dir, *file))` 替代了 `self._safe_join(file)`，绕过了 `safe_join()` 的路径验证。攻击者可在 `payload.root` 任一元素中注入 `"/etc/passwd"` 或 `"../"` 序列来逃逸 `root_dir`。（另由 ai_smell/security 维度同时标记）
    ↳ 建议：恢复使用 `self._safe_join(file)`

test/components/test_file_explorer.py
  ▲ [security · med · conf 0.76] L68
    【安全测试被删除】专门验证 preprocess() 路径穿越防护的测试 test_preprocess_prevents_path_traversal 被整体移除。该测试覆盖了绝对路径攻击（["/etc/passwd"]）和 .. 逃逸（["..","..","etc","passwd"]）两种场景，是防止路径穿越回归的关键安全网。删除后即使漏洞被引入也无法在 CI 中检测。
    ↳ 建议：恢复 test_preprocess_prevents_path_traversal 测试，确保 preprocess() 对越权路径抛出 InvalidPathError。

```
