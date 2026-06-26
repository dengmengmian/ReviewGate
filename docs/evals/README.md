# ReviewGate 评测总览（真实模型 · 真实代码 · 全程留痕）

所有评测用 `deepseek-v4-pro`（OpenAI 兼容协议）真实跑通。每个 `*.md` 是一次可复现的记录。
方法学贯穿：**用真实世界代码做 ground truth**——干净 PR 应 PASS（测精度）、已知 bug 应 BLOCK（测召回）。
对真实 bug 用 **revert 法**：把已合并的修复 `git revert` 回去重新引入 bug，再审查能否命中（公告/issue 即 ground truth）。

## 一、精度（干净代码不误报）

| 来源 | 结果 |
|---|---|
| 真实干净 PR：ripgrep#3420 / fzf#4739,#4803 / got#2454 / yt-dlp#16991 | 全 PASS，0 误 BLOCK |
| 干净改动压测：8 语言的重构/加固/类型化（`precision-stress`） | 8/8 PASS |
| judge 放宽后回归：安全的 jwt/crypto 良性改动 | PASS（0 误报）|

**累计 ~15 个干净用例，0 误 BLOCK。**

## 二、召回（真实漏洞要 BLOCK）

- **真实 CVE（revert 法，5 语言）** [`recall-cve-reverts`](2026-06-26__recall-cve-reverts.md)：
  axios 原型污染 / gradio 路径穿越 / echo 目录穿越 / smallvec 缓冲区溢出 / cJSON 堆越界(删除式)——**全 BLOCK**。
- **植入式漏洞** [`recall-planted-bugs`](2026-06-26__recall-planted-bugs.md) + 多轮类型扩展：
  SQLi / 越权 / 命令注入 / XSS / XXE / SSTI / 反序列化 / 弱加密 / 弱随机 / ReDoS / 开放重定向 / 整数溢出 等 ~18 类，全 BLOCK。
- **真实用户 issue（business review）** [`business-real-issues`](2026-06-26__business-real-issues.md)：
  **16 个真实用户报告的 bug → 13 BLOCK / 2 部分(WARN) / 1 漏**，覆盖 **13 个领域**：
  date / web 路由 / 输入校验 / 状态副作用 / ORM 事务 / 并发竞态 / 大数运算 / 金额 / 缓存淘汰 /
  授权 / 序列化 / 资源泄漏 / HTTP2 时序竞态。
  （revert 真实修复重新引入 bug，命中率 13/16 直接 BLOCK + 2 WARN 提示，仅 1 例细微算术静默漏报）

## 三、语言覆盖

[`language-coverage`](2026-06-26__language-coverage.md) + [`uncommon-languages`](2026-06-26__uncommon-languages.md)：
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

## 五、诚实的局限

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
