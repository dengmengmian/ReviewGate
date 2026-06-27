# ReviewGate 评测总览（真实模型 · 真实代码 · 全程留痕）

所有评测用 `deepseek-v4-pro`（OpenAI 兼容协议）真实跑通。每个 `*.md` 是一次可复现的记录。
方法学贯穿：**用真实世界代码做 ground truth**——干净 PR 应 PASS（测精度）、已知 bug 应 BLOCK（测召回）。
对真实 bug 用 **revert 法**：把已合并的修复 `git revert` 回去重新引入 bug，再审查能否命中（公告/issue 即 ground truth）。

> 📌 **一页汇总**：[`生产/开源就绪验证：≥30 PR × 5 维 + 意图`](2026-06-27__production-readiness-30pr.md) —— 把下面各节汇成一张「能力 × 证据」矩阵，含 perf/style 维度补齐与就绪评估。

## 一、精度（干净代码不误报）

| 来源 | 结果 |
|---|---|
| 真实干净 PR：ripgrep#3420 / fzf#4739,#4803 / got#2454 / yt-dlp#16991 | 全 PASS，0 误 BLOCK |
| 干净改动压测：8 语言的重构/加固/类型化（`precision-stress`） | 8/8 PASS |
| judge 放宽后回归：安全的 jwt/crypto 良性改动 | PASS（0 误报）|

**累计 ~15 个干净用例，0 误 BLOCK。**

## 二、召回（真实漏洞要 BLOCK）

- **真实 CVE（revert 法，5 语言）** [`recall-cve-reverts`](2026-06-25__recall-cve-reverts.md)：
  axios 原型污染 / gradio 路径穿越 / echo 目录穿越 / smallvec 缓冲区溢出 / cJSON 堆越界(删除式)——**全 BLOCK**。
- **植入式漏洞** [`recall-planted-bugs`](2026-06-25__recall-planted-bugs.md) + 多轮类型扩展：
  SQLi / 越权 / 命令注入 / XSS / XXE / SSTI / 反序列化 / 弱加密 / 弱随机 / ReDoS / 开放重定向 / 整数溢出 等 ~18 类，全 BLOCK。
- **真实用户 issue（business review）** [`business-real-issues`](2026-06-26__business-real-issues.md)：
  **16 个真实用户报告的 bug → 13 BLOCK / 2 部分(WARN) / 1 漏**，覆盖 **13 个领域**：
  date / web 路由 / 输入校验 / 状态副作用 / ORM 事务 / 并发竞态 / 大数运算 / 金额 / 缓存淘汰 /
  授权 / 序列化 / 资源泄漏 / HTTP2 时序竞态。
  （revert 真实修复重新引入 bug，命中率 13/16 直接 BLOCK + 2 WARN 提示，仅 1 例细微算术静默漏报）

## 三、语言覆盖

[`language-coverage`](2026-06-25__language-coverage.md) + [`uncommon-languages`](2026-06-25__uncommon-languages.md)：
**30+ 语言**（含 Go/Java/JS/TS/Python/PHP/C/C++/C#/Rust/Ruby/Kotlin/Swift/Dart/Scala/ObjC/Shell/SQL/HTML/CSS/Vue/Svelte +
仓颉/Nim/Haskell/OCaml/Julia/Crystal/R…）。仅 6 种有 tree-sitter 精确工具，其余走 grep 兜底，LLM 审查照常命中——**语言无关**。

## 四、eval 驱动的改进（每条都验证过）

| 改进 | 触发 | 验证 |
|---|---|---|
| SSRF/删除防护 prompt 强化 | round2 SSRF 漏报 | go-ssrf 2/2、删除式漏洞命中 |
| `--samples N` 多采样 | flaky 闸口抖动 | 真实 SSRF samples=3 稳定 BLOCK |
| judge insecure-by-construction 例外 | weak-random 被误证伪 | PASS→BLOCK，精度不退 |
| logic「具体用例执行推演」 | 真实业务 bug 漏报 | DST/parseISO/负零 WARN/漏→BLOCK，精度不退 |
| 行号越界保护 | 短 diff 行号外推 | 单测覆盖 |

## 五、大 PR / 大 diff 健壮性（2026-06-27）

针对「AI 一次改很多文件」场景，用真实 PR / 多提交范围压测大 diff 的单元切分与「绝不静默通过」保证。机制与完整结论见 [`../BIG_PR_HANDLING.md`](../BIG_PR_HANDLING.md)。

| 记录 | 语言 | 规模 | 单元 | 首轮超预算 | 发现 | 判定 |
|---|---|---|---|---|---|---|
| [`bigpr-axios-4k`](2026-06-27__bigpr-axios-4k.md) | JS | 9 文件 | 8 | 0 | 13 | WARN+incomplete |
| [`bigpr-requests-6k`](2026-06-27__bigpr-requests-6k.md) | Python | 19 文件 | 2 | 0 | 2 | WARN+incomplete |
| [`bigpr-axios-range-16k`](2026-06-27__bigpr-axios-range-16k.md) | JS | **55 文件 / ~5000 行** | 8 | 0 | 12 | WARN+incomplete |
| [`bigpr-ripgrep-range-12k`](2026-06-27__bigpr-ripgrep-range-12k.md) | Rust | 28 文件 | 4 | 0 | 6（含 1 处 AI 幻觉 API）| WARN+incomplete |
| [`bigpr-gin-range-16k`](2026-06-27__bigpr-gin-range-16k.md) | Go | 46 文件 / ~2500 行 | 6 | 0 | 13（Slowloris DoS + 复制粘贴残留）| **BLOCK**+incomplete |
| [`bigpr-curl-range-12k`](2026-06-27__bigpr-curl-range-12k.md) | C | 132 文件 / ~2400 行 | 11 | 0 | 15（预算偏紧→未审完为主，诚实标记）| WARN+incomplete |
| [`bigpr-oversized-axios`](2026-06-27__bigpr-oversized-axios.md) | JS | 单文件 2467 行 lock + 小文件 | 7 | 0 | **3 文件 oversized 跳过(点名)**，余正常审 | WARN+incomplete |

**eval 驱动的修复**：实测发现切分单元时未预留「系统提示词 + focus」固定开销，导致小/中预算下单元首轮**全部超预算、审不到内容**（但仍安全 WARN+incomplete，未静默通过）。修复后首轮超预算归零；55 文件/5000 行真实大 PR 切 8 单元正常审、出 12 条发现。「绝不静默通过」不变量全程成立。

## 六、意图 / 技术评审（`--intent`，2026-06-27）

缺陷评审不知道「本该做什么」，意图评审知道。在**真实代码 + 真实意图**上验证：① 受控 A/B 能否区分完整 vs 不完整实现；② 结构化强制能否保证验收清单覆盖每条标准。

| 记录 | 方法 | 结果 |
|---|---|---|
| [`intent-mvp-ab`](2026-06-27__intent-mvp-ab.md) | axios URL 对象特性，受控 A/B（删 dispatch 处理造缺口） | 不完整实现命中缺口（跨文件追到拦截器链）、完整实现 0 误报 |
| [`intent-structured-enforcement`](2026-06-27__intent-structured-enforcement.md) | gin 提交信息意图 / axios 详细 spec | gin **4/4 ✓ met**；axios C1 met + 其余诚实标 `? not assessed` → **WARN** |
| [`intent-batch10`](2026-06-27__intent-batch10.md) | 10 个真实 commit（JS/Go/Python/Rust/C），`--intent-from-commit` | 10/10 全 PASS、每条标准覆盖且 ✓ met（真实正确修复→met 正确，**0 误报 missing**）；#10 暴露的「意图串行致耗时翻倍」已改为并发 |
| [`intent-cobra-pr2356`](2026-06-27__intent-cobra-pr2356.md) | **Go**（cobra）微妙切片别名 bug（污染 os.Args）受控 A/B；含诚实局限 | 缺口：**C1 ⚠ deviation**（调用链推理已核验 `Traverse` 子切片）+ logic Warning → WARN；正确：未误报缺口，但需全链路确证→诚实 **`? not assessed`** + WARN+incomplete（绝不伪 PASS）|

**结构化强制**：意图解析成 N 条标准（C1..CN）注入评审，未被逐条 verdict 的标准兜底标 `? not assessed`，保证清单**覆盖每条**（杜绝修复前真实测出的空清单），有未核对标准则降级 WARN，绝不伪装 PASS。完整性的模型局限见 [`../LIMITATIONS.md`](../LIMITATIONS.md) §6。

## 七、诚实的局限

见 [`../LIMITATIONS.md`](../LIMITATIONS.md)。核心：**细微多步算术/逐位进位 off-by-one**（addBusinessDays、big.js 进位）
是静态 LLM 审查的硬尾，建议与单元测试互补。命中与否大致取决于"需要心算的步数"。

## 复现
```bash
export REVIEWGATE_API_KEY=<deepseek-key>
export REVIEWGATE_BASE_URL=https://api.deepseek.com/v1
export REVIEWGATE_MODEL=deepseek-v4-pro
./scripts/eval-pr.sh <owner/repo> <pr#>            # 真实 PR
# 真实 bug 召回：checkout 修复提交 → git revert -n → reviewgate review
```
