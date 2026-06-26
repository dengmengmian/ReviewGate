# 真实 PR 批量评测汇总（2026-06-25）

模型 `deepseek-v4-pro @ api.deepseek.com`，维度 logic/security/ai_smell（judge on）。
每条结论都**人工对照真实 diff/源码核验**，故"可验证"。

| PR | 类型 | ReviewGate 判定 | 人工核验 | 结论 |
|---|---|---|---|---|
| [ripgrep#3420](2026-06-25__BurntSushi_ripgrep__pr3420.md) | Rust 重构（+208/−136） | PASS，0 发现 | 维护者谨慎重构，确无问题 | ✅ 精度正确 |
| [fzf#4739](2026-06-25__junegunn_fzf__pr4739.md) | Go，结构体移入 goroutine 求栈分配 | PASS，0 发现 | `server` 仅在 goroutine 内使用，移动安全 | ✅ 精度正确 |
| [yt-dlp#16991](2026-06-25__yt-dlp_yt-dlp__pr16991.md) | Python，`['data']`→`traverse_obj(...{dict})` | PASS，0 发现 | 防御式修复，行为正确 | ✅ 精度正确 |
| [got#2454](2026-06-25__sindresorhus_got__pr2454.md) | TypeScript，修 searchParams 引用别名 bug（3f） | PASS，0 发现 | 克隆 URLSearchParams 修复正确 | ✅ 精度正确 |
| [fzf#4803](2026-06-25__junegunn_fzf__pr4803.md) | Go，导出 FZF_CURRENT_ITEM（4f） | PASS，0 发现 | 特性新增，正确 | ✅ 精度正确 |
| [ripgrep#3195](2026-06-25__BurntSushi_ripgrep__pr3195.md) | Rust，修 `--line-buffered` 回归 | **WARN**：ai_smell low/0.60，指出 `free_buffer().as_bytes_mut()` 冗余 | `free_buffer()->&mut [u8]`（L367），`as_bytes_mut()` 对 `[u8]` 是恒等 → **冗余属实** | ✅ **真阳性**，且正确定级为 WARN 不 BLOCK |
| 植入式 bug（[recall](2026-06-25__recall-planted-bugs.md)） | Python，5 类植入漏洞 | **BLOCK**，7 条 | SQL 注入/越权/float 全中 + 额外空值回归 | ✅ 召回正确 |

## 结论（覆盖 Rust / Go / Python / TypeScript）
- **精度**：5 个干净 PR 全部 PASS，0 误 BLOCK；唯一 WARN 经核验是**真实冗余**（非误报），且定级恰当。
- **召回**：植入的 5 类真实漏洞全部捕获并正确 BLOCK。
- **严重度校准**：维护者代码里的小冗余 → LOW/WARN；真实漏洞 → HIGH/BLOCK。校准合理。
- **健壮性**：ripgrep#3195 的 logic 维度在 240s 超时——**优雅收尾**生效，输出明确 `logic 未审完` 告警，
  未把"没审完"伪装成"通过"（这正是本轮修复的超时误 PASS 问题）。

## 运行观察（待优化）
- **reasoning 模型偏慢**：`deepseek-v4-pro` 每轮思考耗时高，logic 维度（调用链深挖）在中等 diff 上可能 >240s。
  建议该模型用 `--timeout 300` 及以上；优雅收尾保证超时也不丢已确认发现、并如实告警。
- 缓存命中稳定在 74–86%，token 成本可控。
