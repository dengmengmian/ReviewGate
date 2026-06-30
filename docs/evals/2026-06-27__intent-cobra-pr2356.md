# 意图评审实测：spf13/cobra#2356 —— Go 微妙切片别名 bug（含诚实局限）

第三个仓库/语言（cobra，Go），故意挑一个**比 Flask 更难**的标的：Go 切片 `append` 别名 bug
（补全时 `append` 写进与 `os.Args` 共享的 backing array）。同一意图（C1/C2）审「正确修复」vs「还原回有 bug」。
模型 `deepseek-v4-pro`，`--dimensions logic --intent`。**这条记录同时是一处诚实局限的留痕。**

- PR: https://github.com/spf13/cobra/pull/2356 —— `fix: prevent completions from mutating os.Args via append side effect`（Fixes #2257）
- base: `61968e893eee2f27696c2fbc8e34fa5c4afaf7c4`  head: `a36b43475f0a868310fdce4ead976a0cfb8173ea`
- 评测时间: 2026-06-27

## 背景（真实 bug 的 ground truth）

`getCompletions` 旧代码 `trimmedArgs := args[:len(args)-1]` 是 `os.Args[1:]` 的子切片，**共享 backing array**。
后续 `append(finalArgs, "--")` 在容量充足时把 `"--"` 写进共享数组，**污染 os.Args**（用户可能在 `ValidArgsFunction` 里读它）。
PR 作者注明：`TraverseChildren: true` 时最明显，因为 `Traverse()` 返回保留容量的子切片。
修复：`trimmedArgs := make([]string, len(args)-1); copy(...)` —— 独立 backing array，append 再也碰不到 os.Args。

意图（`intent.md`，2 条验收标准）：
- **C1** `getCompletions` 不得在 append `"--"` 时改动调用方（可能是 `os.Args`）的 backing array——必须复制到新切片或限制容量。
- **C2** 须有回归测试，驱动补全走 `os.Args` 路径并断言 `os.Args` 未被污染。

## State A —— 还原回有 bug（召回：应「不满足」）

worktree 把 `completions.go` 还原回 base（移除 make+copy），对工作区 diff 跑意图。

- **C1 ⚠ deviation（76%）[completions.go:320]** —— 精确命中，且给出**完整调用链推理**：
  `trimmedArgs（共享 backing array）→ Traverse 返回 args[i+1:] / Find → finalArgs → append(finalArgs, "--")（第369行）写入共享数组 → 污染 os.Args`，并指出意图要求复制/限容、diff 移除防护且无替代。
- logic 维度：**1 条 Warning（low, conf 0.60）**，同一问题，附**诚实的影响校准**——「当前调用链中 args 返回后不再使用、补全是一次性进程，实际影响极小，但这是明确的安全性回退」，给出正确的 make+copy 补丁。
- C2 `? not assessed`（预算耗尽未核对）。
- 闸口：**WARN + incomplete**。

**人工核验**（关键，防止"听起来对"的幻觉）：
- `Traverse()` 确在 `command.go:821` 递归返回 `args[i+1:]` 子切片——模型的调用链推理与**真实代码及 PR 作者本人的解释一致**，非编造。
- 回归测试 `TestCompletionDoesNotMutateOsArgs` 确实存在（`completions_test.go:4076`）——所以 C2 的 `? not assessed` 是**预算没核到**，不是错误地判「缺失」。

## State B —— 正确的修复（精度 + 诚实局限）

`reviewgate review --from <base> --to <head> --dimensions logic --intent intent.md`。跑了两次（首次撞上第三方进程占用同端点被饿死；端点空出后重跑）：

- 两次结果一致：**C1/C2 均 `? not assessed`**，logic 0 发现，**WARN + incomplete**。
- 输出仅 ~7k token → 是**意图 agent 的内部轮次/预算上限**（非我的 `--timeout`、非端点占用）：在这个标的上，要**确证**「整条调用链都不会污染 os.Args」比「指出一处明显删除」更费探索，agent 在记下「met」前就耗尽轮次。

**两个正面结论**：① 对正确代码**没有误报缺口**（没有假的 deviation/missing）；② 结构化强制把未核对的标准**诚实标 `? not assessed` 并降级 WARN+incomplete，绝不伪装 PASS**。
**一个诚实局限**：在这种「需要全链路确证安全属性」的标的上，肯定式「met」可能因 agent 轮次预算不足而退化为「未核对」——见 [`../LIMITATIONS.md`](../LIMITATIONS.md) §6。

## 结论

| | State A 还原 bug | State B 正确修复 |
|---|---|---|
| C1 别名防护 | **⚠ deviation (76%)**（调用链推理已核验）| `? not assessed`（未误报缺口）|
| C2 回归测试 | `? not assessed` | `? not assessed` |
| logic 缺陷评审 | 1 Warning（low/0.60，影响校准诚实）| 0 发现 |
| 闸口 | WARN+incomplete | WARN+incomplete |

**召回侧**：在一个微妙的 Go 切片别名 bug 上，意图评审与 logic 评审双通道命中，推理锚定到真实代码（Traverse 子切片）。
**精度/诚实侧**：对正确实现不误报缺口；无法逐条确证时**诚实降级而非伪 PASS**。与 axios（[mvp-ab](2026-06-27__intent-mvp-ab.md)）、批量（[batch10](2026-06-27__intent-batch10.md)）的干净结果互补——后者是「能区分 / 全 met」，本条额外证明「不确定时如实承认」。

## 复现

```bash
export REVIEWGATE_CONFIG=~/.reviewgate/config.toml   # deepseek-v4-pro
git clone --filter=blob:none https://github.com/spf13/cobra.git && cd cobra
git fetch origin pull/2356/head:pr2356 && git checkout pr2356
# State B：正确修复
reviewgate review --from 61968e8 --to a36b434 --dimensions logic --intent intent.md
# State A：还原 completions.go 回 base，重新引入别名 bug
git checkout 61968e8 -- completions.go
reviewgate review --dimensions logic --intent intent.md
```
