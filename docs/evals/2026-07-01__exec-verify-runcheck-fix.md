# exec-verify / run_check 修复 + 硬尾用例复核 — 2026-07-01

围绕 LIMITATIONS #1「细微算术 off-by-one/进位」硬尾做了一轮证据驱动核查，得三个结论：
一个真 bug 修复、一个文档误标勘误、一个过时结论更新。

## 1. run_check 一直是坏的（已修）

`run_check`（`--exec-verify` 开启的沙箱执行）在 `run_sandboxed` 里 `spawn()` 只设了 `stdin(null)`，
**没设 `stdout/stderr` 为 piped**。tokio 默认继承父进程 fd，后果：

- 片段的 `console.log/print` **泄漏到 ReviewGate 自身 stdout** → `--format json` 输出非法 JSON，CI 里 `jq` 直接解析失败。
- `wait_with_output()` 因未 piped **拿不到子进程输出** → 反手给模型 `(no output; ...)`。

也就是说 exec-verify 的"真正执行核验"能力**名存实亡**——模型每次 run_check 都收到空输出，只能退回心算。
LIMITATIONS 里"exec-verify 开着也没用上"的真实原因是**工具坏了**，不是模型不调用。

**修复**：`.stdout(piped).stderr(piped)`。回归测试 `captures_child_stdout_instead_of_leaking` 锁死
（片段打印 marker，断言 marker 出现在**返回值**而非泄漏）。

**修前/修后**（date-fns addBusinessDays #1584 revert，只 revert index.js，logic 维度 + exec-verify）：

| | `--format json` | run_check 输出回传模型 | decision |
|---|---|---|---|
| 修前 | 非法（被子进程输出污染） | 否，恒 `(no output)` | — |
| 修后 | 有效 | 是 | **block** 2/2 |

## 2. 勘误：big.js #125 不是 bug

此前 LIMITATIONS / business-real-issues 把 big.js #125 记为"base-10 进位漏报（部分命中）"。核查后是**误标**：

- **issue #125 原文**是作者问"这行能不能从 `c[j]=(c[j]+b)%10` 简化成 `c[j]=b`，我觉得这里不会有进位"——
  是**简化提问**，不是错误输出报告。
- **静态证明**：`P.times` 乘法循环里，外层 `i` 递减，位置 `i` 在其 `c[j]=(c[j]+b)%10`（848 行）执行前从未被写过，
  故 `c[j]` 恒 0；进位 `b ≤ 9`，于是 `(c[j]+b)%10 === b`。两种写法**可证等价**。
- **实证**：buggy vs fixed 差分，20 万随机乘法（含负数/小数/变长）+ 21600 个全 9 最大进位对抗用例，**0 差异**。

ReviewGate 实际输出 `ai_smell low`「二者等价、疑似冗余」——**判对了**，不是漏报。文档已勘误。

## 3. 过时结论：addBusinessDays 静态已能抓

business-real-issues 记 addBusinessDays #1584 为"PASS 静默漏报"。当前版本静态（无 exec-verify）复测：
2 次分别 block / warn，**均命中 must-fix high**（循环体"先判断后前进"顺序颠倒，周六+1 得周二而非周一）。
该 bug 已确认为真（差分 3534/12400 相异），"静默漏报"结论已过时。

## 方法学诚实说明
- addBusinessDays 的 revert 只改了 `index.js`，同仓的 `CHANGELOG.md`/`test.js` 仍是修复版，
  对模型构成提示（模型 finding 里引用了 PR #1588 与 test.js 期望）。更严格应剥离测试与 changelog；
  但模型的逐步推演本身也正确，且此处主要用于验证 run_check 修复的端到端效果（json 是否干净、输出是否回传）。
- run_check 修复的价值是**恢复被广告但失效的执行核验能力 + 修复 CI 致命的 json 污染**，
  **不是**"救回一个漏报"——addBusinessDays 静态本就能抓。不夸大。
