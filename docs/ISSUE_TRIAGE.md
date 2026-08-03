# Issue 分诊运维手册

`reviewgate issue …` 帮维护者过一遍提上来的 Issue：分类、查重、（可选）对着代码验证，
最后写一条按类型措辞的回复。这份文档写给**要把它部署起来的人**——README 讲的是它能做什么，
这里讲的是怎么让它稳定跑起来、出问题时怎么查。

能力边界与已知缺口见 [LIMITATIONS.md 第 11 节](./LIMITATIONS.md#11-issue-分诊reviewgate-issue-的边界)。

---

## 先记住一件事：什么情况下它会写东西

**只有 `reviewgate issue review --publish` 会往平台上写。** 其余命令——`init`、`sync`、
`review`（不带 `--publish`）、`watch`、`daemon`——只读平台、只写本地 SQLite。

在此之上还有三道闸：

| 闸 | 位置 | 默认 |
| --- | --- | --- |
| 每个动作单独开关 | `[issue_review.actions]` | 评论开；打标签、关闭**全关** |
| 置信度阈值 | `actions.min_confidence` | 0.5，低于就不下结论、改转人工 |
| 关广告与关任意 Issue 分开 | `close_spam` vs `close_issue` | 都关 |

想先看效果而不冒任何风险：跑 `watch`，它永远不写。

---

## 三种部署形态怎么选

| 形态 | 命令 | 适合 | 代价 |
| --- | --- | --- | --- |
| **手动逐条** | `issue review <N> --publish` | 刚接入、想人工把关每一条 | 得有人盯着 |
| **轮询** | `issue watch` | 中低流量仓库；不想开公网端口 | 有延迟（默认 5 分钟）；只读，不发布 |
| **Webhook + 队列** | `daemon --serve` | 高流量、要求实时 | 要暴露端口、配 secret |

`watch` 与 `daemon` 都**不发布**，它们负责把新 Issue 同步进本地索引并跑出结论存档；
要真正回复仍然是 `issue review --publish`。这么设计是为了让"分析"和"写回"两件事能分开审计。

---

## 从零跑起来

### 1. 建索引（必须先做）

查重靠的是本地历史索引，没有它每条 Issue 都会被判成"不重复"。

```bash
export GITHUB_TOKEN=...            # 或 REVIEWGATE_TOKEN
reviewgate issue init --repo owner/repo --forge github --max 2000
```

- `--max` 是本次最多拉多少条。大仓库第一次建议分几次跑：没拉完时**同步游标不会前进**，
  下次接着上次的位置继续，不会漏。
- 只读，不会发任何评论。
- 数据落在 `.reviewgate/issue/issues.db`（可用 `--data-dir` 改）。这个目录该进 `.gitignore`。

**API 配额**：每条 Issue 会额外拉一次评论列表，所以 N 条 ≈ N + N/50 次请求。
GitHub 认证后 5000 次/小时，一次拉 2000 条没问题；GitLab 的限制更紧，建议分批。

### 2. 试跑一条，确认措辞和判定合意

```bash
reviewgate issue review 123 --repo owner/repo            # dry-run，什么都不发
reviewgate issue review 123 --repo owner/repo --verify --repo-root /path/to/checkout
```

`--verify` 会把报错对到源码行、展开所在函数、找该文件的历史修复。它需要**本地 checkout**，
且版本要对得上；找不到仓库时结论退化为 `UNVERIFIED`——那是"没验证"，不是"验证通过"。

### 3. 长跑

```bash
# 轮询：每 5 分钟一轮，一轮最多消化 20 条
reviewgate issue watch --repo owner/repo --interval 5m --max-issues-per-run 20

# Webhook + 队列消费
export REVIEWGATE_WEBHOOK_SECRET=...
reviewgate daemon --repo owner/repo --serve --listen 0.0.0.0:8080
```

`--max-issues-per-run` 是刹车：一轮同步与分诊的条数上限，剩下的排队等下一轮。
第一次接上一个存量很多的仓库时，它防止一口气打满平台配额。日志会打印还剩多少条在等。

---

## Webhook 配置

### 服务端

```bash
export REVIEWGATE_WEBHOOK_SECRET='一串足够长的随机串'
reviewgate serve --listen 0.0.0.0:8080
# 或与轮询一起跑
reviewgate daemon --repo owner/repo --serve --listen 0.0.0.0:8080
```

**没有 secret 会直接报错退出，不会退回"不校验"模式。** 这是有意的：签名校验一旦失效，
任何人都能伪造事件来驱动评论、打标签、关闭、指派。

### 平台侧

| 平台 | 事件 | 校验头 |
| --- | --- | --- |
| GitHub | Issues, Issue comments | `X-Hub-Signature-256`（HMAC-SHA256） |
| GitLab | Issues events | `X-Gitlab-Token`（定值比对） |

事件先落进 SQLite 队列（`.reviewgate/issue/webhook.db`）再由 daemon 消费，
所以 HTTP 侧不会因为分诊慢而超时，重启也不丢事件。

### Token 权限

| 平台 | 最小权限 |
| --- | --- |
| GitHub | classic: `repo`（私有）/ `public_repo`（公开）；fine-grained: Issues **读写** |
| GitLab | `api` scope 的 project/personal access token |
| Gitee / AtomGit | 有 Issue 评论权限的私人令牌 |

只读跑（`init` / `watch`）时读权限就够。

---

## 配置全表

```toml
[issue_review]
enabled = true
mode = "suggest"            # suggest | publish（publish 仍需 CLI 显式 --publish）

[issue_review.sync]
interval = "5m"
overlap = "10m"             # 游标回退量，防止边界事件漏掉
max_history_issues = 2000

[issue_review.actions]
comment = true              # 发/更新回复
update_existing_comment = true   # 复审时改同一条，而不是刷新评论
add_labels = false          # ← 默认关
close_issue = false         # ← 默认关，能关任意类型
close_spam = false          # ← 只关广告；不必为此打开 close_issue
min_confidence = 0.5        # 低于此值不下结论，转人工；0 = 关闭该闸门
assign_on_triage = true     # 转人工时同时指派

[issue_review.duplicate]
enabled = true
candidate_limit = 20
min_similarity = 0.35

[issue_review.mentions]
default = []
on_needs_triage = ["triage-owner"]    # 留空 = 不转人工，被拦下的会静默跳过
on_needs_info = ["triage-owner"]
on_probable_duplicate = ["triage-owner"]
on_security = ["sec-oncall"]
# 其余：on_likely_bug / on_confirmed_bug / on_regression / on_already_fixed /
#       on_spam / on_advertisement / on_question / on_feature_request
```

> `on_needs_triage` 留空时，被置信度闸门拦下的 Issue 不会消失，但也不会通知任何人——
> 只能靠 `reviewgate issue stats --gated` 查。**长期不看这个列表，等于这些单子没人管。**

---

## 日常巡检

```bash
reviewgate issue stats                  # 判定与动作分布
reviewgate issue stats --gated          # 有哪几条在等人接手 ← 最该定期看的
reviewgate issue inspect 123            # 单条的存档判定
```

---

## 排查

| 现象 | 多半是 | 怎么办 |
| --- | --- | --- |
| 每条都判"不重复" | 索引是空的 | 先跑 `issue init`；`stats` 看 total |
| `init` 拉到的条数远少于 `--max` | 平台限流，或已到末页 | 看日志有没有 `(capped)`；有就再跑一次接着拉 |
| 大量 `unknown` / 40% | 分类信号不足（见 LIMITATIONS #11） | 正常行为：会走转人工，不会发错结论 |
| `tech=UNVERIFIED` | 没给 `--repo-root`，或版本对不上 | 指到对应版本的 checkout |
| Webhook 返回 401 | secret 不一致 | 两侧用同一个值；GitLab 是定值比对不是 HMAC |
| 复审时又发了一条新评论 | `update_existing_comment` 被关了 | 打开它 |
| 什么都没发生 | 动作开关默认全关 | 逐项打开需要的，并确认过了 `min_confidence` |

---

## 在自己的仓库上评测一次

接入前想知道它在**你的** Issue 分布上表现如何，跑：

```bash
cargo build --release
scripts/eval-issue-triage.sh owner/repo 500 /path/to/checkout
```

它会同步 500 条真实 Issue、在本地跑一遍分诊、打印类型与裁决分布。**全程 `watch`，
不会发出任何评论。** 结果落在 `.eval-issue/`（已 gitignore）。

看什么：

- `unknown` 占比 —— 这些会走转人工。太高说明你的 Issue 标题措辞对规则不友好。
- `security` 那几条点开核对 —— 误判成安全会 @ 安全接口人，代价最大。
- 有没有 panic —— 脚本会以非零退出码报出来。

发现误判就把它加进 `crates/core/tests/fixtures/issue_classify.jsonl`：
**真实仓库跑出来的误报是最有价值的负样本**，合成样本测不出精度。

## 成本

分诊主链路**不调用大模型**：分类是规则、查重是本地哈希嵌入 + FTS、验证是 grep + 语法树。
`watch` / `daemon` 全程零模型开销，只花平台 API 配额。

只有 `issue review` 会为"把结论写成人话"调一次模型（每条一次）。不想花这个钱就用
`watch` 看结论，或者直接用模板措辞。
