# Round 2：跨漏洞类型召回（2026-06-25）

验证召回不局限于 SQL 注入：8 个**不同漏洞类型 × 不同语言**，纯 security/logic/ai_smell（无 business 规则）。

| # | 语言 | 漏洞类型 | 闸口 | 是否命中目标类型 |
|---|---|---|---|---|
| 1 | Go | SSRF（移除 host 允许名单） | BLOCK | ⚠ 部分：抓到**真实 nil 解引用**（忽略 http.Get 错误），SSRF 本身 run-to-run 不稳定 |
| 2 | Python | 不安全反序列化（pickle.loads） | BLOCK | ✅ logic high 0.86 |
| 3 | Java | 弱加密（MD5 存口令） | BLOCK | ✅ security high 0.81 + ai_smell 0.95 |
| 4 | Ruby | 命令注入（backtick 插值） | BLOCK | ✅ security high 0.99 |
| 5 | PHP | LFI/路径穿越（include $_GET） | BLOCK | ✅ security high 0.99 |
| 6 | JavaScript | 硬编码密钥（sk_live_…） | BLOCK | ✅ ai_smell high 0.95 |
| 7 | TypeScript | ReDoS（嵌套量词正则） | BLOCK | ✅ logic high 0.99 |
| 8 | C | 整数溢出→堆溢出（int n） | BLOCK | ✅ security high 0.99 + logic 0.99 |

## 结论
- **8/8 全部 BLOCK**；**7/8 明确命中目标漏洞类型**（反序列化/弱加密/命令注入/LFI/硬编码密钥/ReDoS/整数溢出）。
  说明召回明显**不局限于 SQL 注入**，跨类型有效。
- **SSRF 这例诚实标注为"部分"**：模型优先报了同一行上**真实存在的 nil 指针解引用**（忽略 `http.Get` 错误 →
  `resp` 可能为 nil），这本身是正确的高危发现、闸口 BLOCK 合理；但**对"移除 host 允许名单=SSRF"的识别 run-to-run 不稳定**
  （批量那次另有一条 ai_smell 发现疑似指向它，json 复跑那次只报了 nil 解引用）。→ SSRF 类（尤其"删除允许名单"形态）是
  后续要加强的召回点。

## 观察（待优化）
- **极短文件的行号**：本轮玩具文件仅 3–6 行，个别 finding 报到 L5/L6（超出文件），属短 diff 上模型行号外推、
  锚点又未能重定位。真实文件（CVE 测试里 L152/L418 等）行号准确。可在 relocate 加"行号超出文件长度则强制锚点重定位"。
- **run-to-run 方差**：同一 case 不同次运行命中的 finding 集合有波动（LLM 固有）。多 finding 时靠 dedup/judge 收敛，
  单一微妙漏洞（如 SSRF）建议未来用"多次采样取并集"或针对性 checklist 提升稳定性。
