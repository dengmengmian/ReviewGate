# GitHub 10 仓真实评测：review + security

- **日期**: 2026-07-30  
- **工具**: `reviewgate 0.9.0`（release 构建）  
- **模型**: deepseek-v4-pro via 本地配置（密钥不入库）  
- **timeout**: review=180s · security=180s  
- **方法**: 每仓 1 个已合并 PR · `review --profile gate` + `security` 深审 · base…head  
- **脚本**: `scripts/batch-github-review.sh`  
- **原始 log**：本地 `docs/evals/batch-raw/`（已 gitignore，不随仓库提交）

---

## 汇总表

| # | 仓库 | PR | 文件 | review | security | 说明 |
|---:|---|---|---:|---|---|---|
| 1 | BurntSushi/ripgrep | [#3496](https://github.com/BurntSushi/ripgrep/pull/3496) | 1 | **WARN** | **PASS** | review：logic/ai_smell **超时 incomplete**，0 实质发现 |
| 2 | junegunn/fzf | [#4875](https://github.com/junegunn/fzf/pull/4875) | 2 | **PASS** | **PASS** | 依赖小 bump，干净 |
| 3 | psf/requests | [#7596](https://github.com/psf/requests/pull/7596) | 5 | **PASS** | **PASS** | Actions 组 bump，干净 |
| 4 | axios/axios | [#11109](https://github.com/axios/axios/pull/11109) | 3 | **PASS** | **PASS** | 错误栈容错修复，干净 |
| 5 | gin-gonic/gin | [#4709](https://github.com/gin-gonic/gin/pull/4709) | 1 | **BLOCK** | **PASS** | review：1× ai_smell high（测试与 MkdirAll 前提） |
| 6 | clap-rs/clap | [#6455](https://github.com/clap-rs/clap/pull/6455) | 3 | **WARN** | **PASS** | incomplete + 1× perf med 68% |
| 7 | cli/cli | [#13987](https://github.com/cli/cli/pull/13987) | 3 | **PASS** | **PASS** | skill 文案替换，干净 |
| 8 | pallets/flask | [#6013](https://github.com/pallets/flask/pull/6013) | 2 | **PASS** | **PASS** | autoescape 大小写，干净 |
| 9 | expressjs/express | [#7366](https://github.com/expressjs/express/pull/7366) | 3 | **PASS** | **PASS** | QUERY 条件重验证，干净 |
| 10 | sindresorhus/got | [#2454](https://github.com/sindresorhus/got/pull/2454) | 3 | **WARN** | **PASS** | logic/ai_smell **超时 incomplete**，0 实质发现 |

### 计数

| 模式 | PASS | WARN | BLOCK |
|---|---:|---:|---:|
| **review (gate)** | 6 | 3 | 1 |
| **security (deep)** | **10** | 0 | 0 |

> 中文 locale 下 BLOCK 显示为「拦截」（非「阻断」）。汇总表已按状态行图标人工核对。

---

## 非 PASS 拆解

### WARN（仅 incomplete，无 must-fix）

| PR | 原因 |
|---|---|
| ripgrep#3496 | logic + ai_smell 墙钟超时（180s）；0 发现；**诚实 WARN** |
| got#2454 | 同上 |

→ 产品行为正确（不伪 PASS）。缓解：`--timeout 300` 或拆维度。

### WARN + 实质发现

| PR | 发现 | 人工判断 |
|---|---|---|
| clap#6455 | perf · med · 68%：possible values 先全量构造再过滤 | **可疑/建议级**——可能为优化空间，置信度不高；且有 incomplete |

### BLOCK

| PR | 发现 | 人工判断 |
|---|---|---|
| gin#4709 | ai_smell · high · 95%：`SaveUploadedFile` 现会 `MkdirAll`，测试「目录不存在→应失败」前提可能被破坏 | **值得人工核**——属测试/行为一致性问题，security 未报安全洞 |

security 对上述 PR 均 **PASS**，说明「深审安全」与「全维缺陷闸口」关注面不同，符合设计。

---

## 结论（对 ReviewGate）

1. **无工具崩溃 / panic / 假 PASS**（incomplete 均降级 WARN）。  
2. **security 深审** 10/10 PASS，噪声低（本批 PR 多为测试/依赖/小修复）。  
3. **review 精度**：6 个干净 PR 全 PASS；2 个超时空 WARN 属超时策略；1 个 BLOCK 有具体代码锚点。  
4. **产品缺口（非崩溃）**：  
   - 180s 下 logic/ai_smell 在部分小仓仍易超时（与此前实跑一致）。  
   - 汇总脚本若只认「阻断」会漏标中文「拦截」——已在本报告人工校正；脚本后续可改。  
5. **本批未发现需立刻修的引擎逻辑 bug**（无密钥泄露、无 JSON 损坏、无 exit 语义错乱）。

---

## 复现

```bash
# API key 在 ~/.reviewgate/config.toml 或 REVIEWGATE_API_KEY（勿提交密钥）
cargo build --release -p reviewgate
REVIEW_TIMEOUT=180 SECURITY_TIMEOUT=180 ./scripts/batch-github-review.sh
```

单仓：

```bash
reviewgate review --from <baseOid> --to <headOid> --profile gate --timeout 180
reviewgate security --from <baseOid> --to <headOid> --timeout 180
```

---

## 后续建议

1. 批量 / CI 默认 timeout 提到 **300s**，减少空 WARN。  
2. 对 gin#4709 类测试语义问题：可考虑 judge 对 `*_test.go` 的 ai_smell 降权（可选，需评测防漏报）。  
