# 路径排除对送审范围与成本的影响（20 仓库 × 10 轮）

**日期**：2026-08-01
**被测改动**：新增审查范围排除（内置默认 + `.reviewgateignore` + `[exclude] patterns`）
**结论**：20 个真实开源仓库上，送审输入 token 合计 **8.29M → 6.61M（−20.3%）**；被排除的文件**全部**是依赖锁 / vendored 依赖 / node_modules 夹具，**无一真实源码**；10 轮重复结果**逐位一致**。

---

## 方法

### 为什么用 `--estimate-only`

要测的是"排除规则改变了送审范围与成本"，这是**确定性**问题，不需要调 LLM。`--estimate-only` 走完整的
diff 解析 → 排除 → 单元规划 → 成本估算，只是不发请求。因此：结果可重复、零 token 成本、可以跑足 10 轮
验证稳定性。判定是否被改动影响另有一组真实 LLM 对比（见 `2026-08-01__decision-stability-20-repos.md`）。

### 对照组

| 组 | 配置 | 等价于 |
|---|---|---|
| baseline | `[exclude] patterns = [] / builtin = false` | 改动前的行为（当时没有任何排除机制） |
| new | `[exclude] patterns = [] / builtin = true` | 改动后的**默认**行为 |

两组只差 `builtin` 一个开关，其余配置（provider、模型、维度）完全相同。刻意**不加任何自定义
patterns**——测的是开箱默认值，不是调参后的最好成绩。

### 样本

20 个仓库在跑之前就已选定并写进 `repos.txt`（跨 Rust / Go / JS / TS / Python），不是按结果挑的：

```
rust-lang/cargo      BurntSushi/ripgrep   sharkdp/bat          tokio-rs/tokio
denoland/deno        cli/cli              junegunn/fzf         gohugoio/hugo
grpc/grpc-go         kubernetes/client-go axios/axios          expressjs/express
vitejs/vite          prettier/prettier    facebook/react       sequelize/sequelize
psf/requests         pallets/flask        yt-dlp/yt-dlp        python-poetry/poetry
```

每个仓库浅克隆（`--depth 30`），审查范围统一 `HEAD~10..HEAD`（各仓库最近 10 次提交的真实改动）。
命令：

```bash
reviewgate review --from HEAD~10 --to HEAD --estimate-only --format json --no-metrics
```

10 轮 × 20 仓库 × 2 组 = **400 次运行，0 次失败**。

---

## 结果

### 逐仓库

| 仓库 | 文件数 base→new | 排除 | 估算输入 token base→new | Δ |
|---|---|---|---|---|
| python-poetry/poetry | 19→18 | 1 | 1,050,540→99,940 | **−90.5%** |
| sharkdp/bat | 6→5 | 1 | 148,900→25,590 | **−82.8%** |
| vitejs/vite | 83→82 | 1 | 615,380→343,080 | **−44.2%** |
| axios/axios | 81→79 | 2 | 975,450→756,560 | **−22.4%** |
| gohugoio/hugo | 13→12 | 1 | 131,100→105,620 | −19.4% |
| sequelize/sequelize | 20→19 | 1 | 400,280→352,110 | −12.0% |
| prettier/prettier | 67→66 | 1 | 307,420→290,320 | −5.6% |
| BurntSushi/ripgrep | 32→31 | 1 | 260,660→254,460 | −2.4% |
| kubernetes/client-go | 25→24 | 1 | 235,210→231,550 | −1.6% |
| pallets/flask | 23→22 | 1 | 90,730→89,530 | −1.3% |
| denoland/deno | 73→67 | 6 | 371,580→367,480 | −1.1% |
| junegunn/fzf | 19→18 | 1 | 134,120→133,150 | −0.7% |
| cli/cli | 188→183 | 5 | 1,777,410→1,766,500 | −0.6% |
| expressjs/express | 15→15 | 0 | 83,240→83,240 | 0.0% |
| facebook/react | 50→50 | 0 | 611,310→611,310 | 0.0% |
| grpc/grpc-go | 24→24 | 0 | 634,630→634,630 | 0.0% |
| psf/requests | 8→8 | 0 | 26,990→26,990 | 0.0% |
| rust-lang/cargo | 20→20 | 0 | 150,550→150,550 | 0.0% |
| tokio-rs/tokio | 14→14 | 0 | 115,050→115,050 | 0.0% |
| yt-dlp/yt-dlp | 12→12 | 0 | 171,120→171,120 | 0.0% |
| **合计** | 772→751 | 21 | **8,291,670→6,608,780** | **−20.3%** |

- **13/20** 仓库至少排除了一个文件；**7/20** 一个都没排到，**Δ 精确为 0.0%**——没有匹配时零退化，不是"几乎不变"。
- 收益分布极不均匀：省下的几乎全部来自**少数几个巨大的 lock 文件**（poetry.lock、Cargo.lock、pnpm-lock.yaml、package-lock.json）。中位数只有 −1.3%，平均值被少数极值拉到 −20.3%。**用平均值预期单个仓库的收益会落空**。

### 被排除的文件（全量核对）

| 类别 | 实例 |
|---|---|
| 依赖锁 | `poetry.lock`、`Cargo.lock`(×2)、`pnpm-lock.yaml`、`yarn.lock`(×2)、`package-lock.json`(×2)、`go.sum`(×4)、`uv.lock`、`Gemfile.lock` |
| vendored 依赖 | `cli/cli` 里 `.github/codeql/tests/**/vendor/**` 的 3 个文件 + `modules.txt` |
| node_modules 夹具 | `deno` 里 `tests/specs/**/node_modules/**` 的 6 个文件 |

**21 个被排除文件，逐个核对，无一是真实源码。**

### 确定性（10 轮）

400 行结果按 `(仓库, 配置)` 聚合后共 40 组，每组 10 轮的 `files_changed / excluded / est_input_tokens / units`
**取值集合大小均为 1**——即 10 轮完全一致，无抖动。排除逻辑不含随机性，这一点被实测确认而非假设。

---

## 已知边界（不藏）

1. **`node_modules/` 下的测试夹具会被排除。** deno 把 npm 包桩代码放在 `tests/specs/**/node_modules/`，
   这些文件本轮被内置规则挡下。对 deno 这类仓库，如果需要审这些夹具，得显式写 `!tests/specs/**` 反选。
   同理 `cli/cli` 的 CodeQL 测试里有 vendored 的 `sanitizer.go`。
   **这类漏审是可见的**——被排除清单会打进报告、JSON 和 PR 评论，不是静默发生。
2. **收益高度依赖仓库是否提交了大 lock 文件。** 7/20 的仓库本轮收益为零。宣传时不能只报 −20.3%。
3. **本组不衡量审查质量。** 少送 lock 文件是否影响结论，由真实 LLM 那组回答。
4. **`--estimate-only` 估的是上界**，不是实际消耗；两组用同一套估算口径，比值可比，绝对值不可当账单。

## 复现

```bash
cargo build --release
scripts/eval-exclude-scope.sh /tmp/rg-eval 10   # 克隆 20 个样本仓库并跑 10 轮
```

脚本会自己写出 `config-baseline.toml` / `config-new.toml`（两者只差 `builtin` 开关），
原始结果落在 `/tmp/rg-eval/results/round-*.jsonl`，最后打印确定性检查与合计降幅。
