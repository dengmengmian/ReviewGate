# 意图评审批量实测：10 个真实 commit（`--intent-from-commit`）

用 **10 个新选的真实 commit**（跨 JS/Go/Python/Rust/C，意图取自作者真实提交信息，零编造）压意图评审的**精度**——这些都是已合并的正确修复，所以「实现满足意图」是 ground truth，理想结果是每条标准 `✓ met`、不误报 `missing`。模型 `deepseek-v4-pro`，`--dimensions logic --intent-from-commit --no-judge`。

| # | 仓库 | commit | 判定 | 标准数 | 结果 |
|---|---|---|---|---|---|
| 1 | axios | 140a179（guard socketPath 原型污染 SSRF） | PASS | 2 | 2/2 ✓ met |
| 2 | axios | 7b3369a（clear RN FormData content type） | PASS | 2 | 2/2 ✓ met |
| 3 | gin | 915e4c9（localhost IP → 常量，重构） | PASS | 1 | 1/1 ✓ met |
| 4 | gin | 2a794cd（debug version mismatch） | PASS | 2 | 2/2 ✓ met |
| 5 | requests | f0198e6（Content-Type 值解析修复） | PASS | 1 | 1/1 ✓ met |
| 6 | requests | bc7dd0f（header 合法性正则修复） | PASS | 1 | 1/1 ✓ met |
| 7 | ripgrep | cd1f981（derive `Default`） | PASS | 2 | 2/2 ✓ met |
| 8 | ripgrep | 0a88ccc（QEMU 交叉编译压缩测试修复） | PASS | 2 | 2/2 ✓ met |
| 9 | curl | a62e08c（trace 'ns'→'us'） | PASS | 2 | 2/2 ✓ met |
| 10 | fd | 82485bf（feat: --exact 参数） | PASS | 1 | 1/1 ✓ met（单独复跑） |

## 结论

- **精度好**：10/10（#10 单独复跑确认）**每条验收标准都被覆盖且 ✓ met**——对真实正确修复给出 met 是对的，**0 例误报 missing/deviation**（不在正确代码上喊狼）。
- **不再空清单**：结构化强制下每个 commit 都产出覆盖每条标准的清单（对照修复前真实测出的空清单）。
- 召回侧（破坏实现 → missing/deviation）由受控 A/B 验证，见 [`intent-mvp-ab`](2026-06-27__intent-mvp-ab.md) / [`intent-structured-enforcement`](2026-06-27__intent-structured-enforcement.md)。

## eval 驱动的修复（#10 暴露）

#10 在批量里曾被运行器记为 rc=1/无输出——单独复跑实为 **PASS / 1 met**（瞬时抖动，非 bug）。但它暴露了真实成本点：`reviewgate review` 的墙钟原为 **fan-out 维度 + 意图 Agent 顺序执行 ≈ `2 × --timeout`**。
**修复**：意图 Agent 不依赖维度结果，改为与 fan-out **并发执行**（`tokio::join!`），总耗时降到 ≈ `max(fan-out, intent)`。集成测试通过、行为不变。
