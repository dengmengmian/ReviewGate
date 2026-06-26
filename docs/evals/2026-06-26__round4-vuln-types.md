# Round 4：更多高价值漏洞类型（真实 handler 上下文 + --samples 3）

| # | 用例 | 漏洞类型 | 初次（samples=3） | 备注 |
|---|---|---|---|---|
| 1 | java-xxe | XXE（未禁用外部实体） | **BLOCK** security 0.95 | ✅ |
| 2 | python-ssti | SSTI（render_template_string 插值） | **BLOCK** security 0.98 | ✅ |
| 3 | go-race | 竞态/TOCTOU 双花（删锁） | **BLOCK** logic 0.83 + security 0.76 | ✅ |
| 4 | js-jwt | 鉴权绕过（verify→decode） | **BLOCK** security 0.99 | ✅ |
| 5 | python-openredirect | 开放重定向（删 is_safe_url） | **BLOCK** security 0.97 | ✅ |
| 6 | js-weakrandom | 弱随机数令牌（randomBytes→Math.random） | 初次 **PASS** → 修复后 **BLOCK** | ⚠→✅ 见下 |

## 关键发现：js-weakrandom 的"检出但被 judge 误杀"
- agent **检出**该漏洞，置信度 **0.98**（"不安全随机数：resetToken 用 Math.random"）；
- 但**证伪 judge 把它 refute 掉 → PASS**。根因：judge"宁可错杀误报"的偏向，对**本质即不安全**的写法
  （弱 PRNG 做令牌）也因"无法本地证明被利用"而证伪。
- **修复**（`judge/prompt.rs`）：加例外——弱 PRNG 令牌 / MD5 存口令 / 硬编码密钥 / 关签名校验 / 已知注入 sink 等
  insecure-by-construction 类，判定标准是「是否确实是这种写法」而非「能否本地证明可利用」。
- **验证**：js-weakrandom **PASS→BLOCK**；js-jwt 仍 BLOCK（无回归）。Round 4 最终 **6/6**。

## 结论
- 6 类新漏洞（XXE/SSTI/竞态/JWT 绕过/开放重定向/弱随机）经"真实 handler 上下文 + 多采样"全部 BLOCK。
- 本轮暴露并修复了一个**judge 层召回缺陷**（对 insecure-by-construction 过度证伪）——这是持续评测的价值。
- 精度风险评估：judge 例外**范围很窄**（仅限明确的不安全写法类别），干净代码不含这些，故对误报率影响很小。

## 精度回归验证（judge 改动后）
对**安全代码的良性改动**复测，确认 judge 放宽未引入误报：

| 用例 | 改动 | 期望 | 结果 |
|---|---|---|---|
| jwt-clean | 安全的 `jwt.verify` 上加算法约束+注释 | PASS | **PASS** ✅ |
| random-clean | 安全的 `crypto.randomBytes(32→48)` | PASS | **PASS** ✅ |

结论：judge 例外只对**真正不安全的写法**触发，对安全的 jwt/crypto 代码 0 误报——**召回↑、精度不降**。
