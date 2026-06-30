# 意图评审实测：结构化强制 → 验收清单覆盖每条标准

验证「结构化强制」在**真实代码 + 真实意图**上能否输出覆盖每条验收标准的清单（不再空清单），并对未核对标准诚实降级。模型 `deepseek-v4-pro`，`--dimensions logic --intent --no-judge`。

> 背景：修复前用 `--intent-from-commit` 真实测过 axios 847d89b / gin 9914178 —— 两次都**输出 0 条意图 verdict、空清单**（模型探索完不回头逐条打勾）。这促成了结构化强制。

## A. gin 9914178 + 提交信息作意图（`--intent-from-commit`）

提交信息（多行）被拆成 4 条验收标准 → 验收清单 **4/4 ✓ met**（95% / 95% / 80% / 90%），每条带跨文件理由：
`Header.Values` + `strings.Join` 正确拼接多值 X-Forwarded-For、新测试用 `http.MethodGet`、lint 修复、XFF 测试重构。真实合并修复确实完整 → 全 met（正确）；判定 **PASS**。

## B. axios 847d89b + 详细 spec（4 条验收标准）

- **C1**（buildURL 接受 URL 对象）：`✓ met (95%)`，锚到 buildURL safeguard（第 34-36 行）+ `_request` 预 normalize。
- **C2**（dispatch 路径处理）/ **C3**（不破坏字符串行为）/ **C4**（测试覆盖）：`? not assessed` —— 评审把预算花在 C1 深核，其余未逐条核对 → 结构化**兜底标「未核对」**。
- 因有未核对标准 → 结果**降级 WARN**（绝不伪装 PASS）；清单**覆盖全部 4 条**（无空清单）。

## 结论

- **结构化强制达成目标**：清单一定覆盖每条标准，真实数据上不再出现空清单（对照修复前的 0 输出）。
- 未核对标准诚实标 `? not assessed` 并 **WARN**，不假装审过；提示放宽 `--timeout` 或拆分意图后重跑。
- 完整 verdict 质量仍受模型与意图清晰度影响（见 [`../LIMITATIONS.md`](../LIMITATIONS.md) §6）。
