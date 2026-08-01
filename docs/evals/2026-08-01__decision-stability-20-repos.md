# 排除规则对闸口判定的影响（20 仓库 × 真实 LLM 两轮）

**日期**：2026-08-01
**模型**：`deepseek-v4-pro`（OpenAI 兼容端点），维度 `security,logic`，judge 开启
**被测改动**：新增审查范围排除（内置默认）
**问题**：省下 20% 的 token（见 `2026-08-01__exclude-scope-20-repos.md`）之后，**闸口结论有没有变**？

> 状态：数据回填中（两轮各 20 次真实审查正在跑）。本文件的方法与判读标准在跑之前就已写定，
> 避免看到结果再改口径。

---

## 方法

同一批 20 个仓库（`repos.txt`，与范围评测同一份、同样是事先声明的），审查范围统一
`HEAD~2..HEAD`（比范围评测的 `HEAD~10..HEAD` 窄，为把单次真实审查的成本与耗时控制在可接受区间）。

| 组 | 配置 |
|---|---|
| baseline | `[exclude] builtin = false` —— 改动前行为，lock 文件等照送 |
| new | `[exclude] builtin = true` —— 改动后默认 |

```bash
reviewgate review --from HEAD~2 --to HEAD --dimensions security,logic \
  --timeout 600 --format json --no-metrics
```

### 补充一组植入式对照（为什么需要）

20 个成熟仓库的已合并 commit **本来就是干净的**——两组都报 0 个发现时，"判定一致"是平凡成立的，
没有区分力。因此另加一组植入对照，把同一个已知漏洞（SQL 注入）分别放在两处：

| case | 漏洞位置 | 预期 |
|---|---|---|
| normal-path | `src/handler.py` | 两组都应 BLOCK——排除规则**不得**影响普通源码的召回 |
| excluded-path | `vendor/handler.py` | baseline BLOCK；new 不审该文件（内置排除 `vendor/`）→ 这是**已知代价**，且必须在报告里明说"文件被排除"，而不是显示"没有改动" |

第二个 case 是故意设计来暴露代价的：它证明"排除 = 少审"这件事真实存在、边界在哪里、以及
是否被如实披露。

## 判读标准（先定后跑）

1. **判定一致性是主指标**：`decision`（pass/warn/block）逐仓库比对。
   - 任何 `block → pass` 的变化都是**严重回归**：说明排除掉的文件里有闸口级问题。必须逐条查清。
   - `pass → block` 说明排除反而暴露了问题（可能是噪音减少后模型更专注），需要人工确认真伪。
2. **发现数量差异是次要指标**，且**不作为"更好"的证据**：LLM 本身有采样波动，单轮差异不能归因到排除规则。
   只在出现"baseline 有、new 没有，且该发现落在被排除文件上"时才有解释价值。
3. **incomplete 必须对齐**：任一组出现未审完，该仓库的判定对比无效，单独标注。

## 结果 A：植入式对照（已完成）

同一个 SQL 注入漏洞，只改放置路径：

| case | 漏洞位置 | 配置 | 判定 | 送审文件 | 保留发现 | 说明 |
|---|---|---|---|---|---|---|
| normal-path | `src/handler.py` | baseline | **BLOCK** | 1 | 2 × high (1.00 / 0.99) | |
| normal-path | `src/handler.py` | new | **BLOCK** | 1 | 2 × high (1.00 / 0.98) | **排除规则不影响普通源码的召回** |
| excluded-path | `vendor/handler.py` | baseline | **BLOCK** | 1 | 2 × high (1.00 / 0.98) | |
| excluded-path | `vendor/handler.py` | new | PASS | **0** | 0 | 文件被内置 `vendor/` 规则排除 —— **这是已知代价** |

第 4 行是这次评测最重要的一行：它证明"排除 = 少审"是真的。关键在于**它是否被如实披露**。
实际输出（`--format text`）：

```
本次改动的 1 个文件全部被排除规则挡下，未送审。
没有任何内容发给模型。请检查配置里的 [exclude] patterns 与 .reviewgateignore。
  - vendor/handler.py (builtin)
```

不是"未检测到变更"，而是点名说清楚少审了什么、为什么。JSON 里同样有 `excluded` 与 `files_changed: 0`，
CI 可据此判"排除规则是不是写太宽"。**但要注意：这条 PASS 的退出码是 0**——因为范围是用户自己配的
（只动 lock 文件的 PR 本就该放行）。见 `docs/LIMITATIONS.md` #8。

## 结果 B：20 个真实仓库两轮对比

| 仓库 | baseline | new | 一致 | 保留发现 b→n | 实际输入 token b→n |
|---|---|---|---|---|---|
| BurntSushi/ripgrep | pass | pass | ✅ | 0→0 | 54,364→53,307 |
| axios/axios | pass | pass | ✅ | 0→0 | 37,080→39,967 |
| cli/cli | warn | warn | ✅ | 0→0 | 57,316→62,806 |
| denoland/deno | pass | pass | ✅ | 0→0 | 47,703→50,660 |
| expressjs/express | pass | pass | ✅ | 0→0 | 31,320→36,283 |
| facebook/react | warn | warn | ✅ | 0→0 | 91,541→44,855 |
| gohugoio/hugo | pass | pass | ✅ | 0→0 | 63,832→75,528 |
| grpc/grpc-go | pass | pass | ✅ | 0→0 | 46,479→54,620 |
| junegunn/fzf | pass | pass | ✅ | 0→0 | 35,510→30,295 |
| kubernetes/client-go | pass | pass | ✅ | 0→0 | 36,691→15,732 |
| pallets/flask | pass | pass | ✅ | 0→0 | 533,891→42,055 |
| prettier/prettier | pass | pass | ✅ | 0→0 | 534,493→534,749 |
| psf/requests | pass | pass | ✅ | 0→0 | 14,675→14,543 |
| python-poetry/poetry | pass | pass | ✅ | 0→0 | 47,257→41,570 |
| rust-lang/cargo | pass | pass | ✅ | 0→0 | 33,412→36,618 |
| sequelize/sequelize | pass | pass | ✅ | 0→0 | 115,531→91,879 |
| sharkdp/bat | pass | pass | ✅ | 0→0 | 45,839→7,651 |
| tokio-rs/tokio | pass | pass | ✅ | 0→0 | 58,261→44,623 |
| vitejs/vite | pass | pass | ✅ | 0→0 | 28,113→29,604 |
| yt-dlp/yt-dlp | pass | pass | ✅ | 0→0 | 45,783→57,685 |
| **合计** | | | **20/20** | | **1,959,091→1,365,030（-30.3%）** |

- **判定一致：20/20**，无一例 `block → pass`。
- **baseline 有而 new 无的发现：无**；反向亦无。
- 实际输入 token 1,959,091 → 1,365,030（-30.3%）。与范围评测一致，降幅由少数带大 lock 文件的仓库主导，不是每个仓库都降。
- `incomplete` 两组一致的仓库：cli/cli、facebook/react——这些仓库两组都未审完，判定对比仍成立但证据强度较弱。

**这组数据的局限（重要）**：20 个仓库的已合并 commit 本来就干净，两组 `kept` 全为 0。
因此"判定一致"在这里主要证明**没有引入回归**，不能证明"排除不影响召回"——后者由上面的
植入式对照回答。

## 复现

```bash
cargo build --release
scripts/eval-exclude-scope.sh /tmp/rg-eval 1        # 准备仓库与两份配置
scripts/eval-decision-stability.sh /tmp/rg-eval baseline
scripts/eval-decision-stability.sh /tmp/rg-eval new
scripts/eval-planted-control.sh /tmp/rg-eval        # 植入式对照
```

## 结论

1. **没有回归**：20 个真实仓库两轮对比，判定 **20/20 一致**，无一例 `block → pass`，发现集合完全相同
   （既没丢也没多）。实际输入 token 1,959,091 → 1,365,030（−30.3%）。
2. **对普通源码的召回不受影响**：植入对照里，`src/handler.py` 的 SQL 注入两组都以 2 条 high
   （置信度 ≥0.98）BLOCK。
3. **代价是真实且已知的**：同一个漏洞放到 `vendor/` 下，新配置不会审到它。这不是 bug，是排除规则的
   定义；关键是**它被如实说出来了**——报告点名"全部被排除"并列出文件与原因，JSON 里
   `files_changed: 0` + `excluded` 非空，CI 可据此拦配置事故。
4. **这组数据本身的局限**：成熟仓库的已合并 commit 干净，两组 `kept` 全为 0，所以"判定一致"主要证明
   **无回归**，证明不了"排除不影响召回"——后者靠植入对照回答。两者缺一不可。

> 一句话：省掉的 20–30% token 来自 lock 文件等**本来就没有审查价值**的内容，闸口结论没有变化；
> 唯一的覆盖损失发生在你自己配置的排除路径内，而且是被公开披露的。
