# GitHub 10 仓真实评测：review + security

- 日期: 2026-08-04
- 工具: `reviewgate 0.11.0`
- 模型/配置: `/Users/mengmian/Develop/app/ReviewGate/reviewgate.toml`（provider 见配置，不写密钥）
- timeout: review=600s · security=200s
- 范围: 各仓 1 个已合并 PR 的 base…head

## 汇总表

| 仓库 | PR | 文件数 | review | security | 详情 |
|---|---|---:|---|---|---|
| BurntSushi/ripgrep | [#3496](https://github.com/BurntSushi/ripgrep/pull/3496) | 1 | PASS | PASS | [detail](batch-raw/2026-08-04/BurntSushi_ripgrep__pr3496.md) |
| junegunn/fzf | [#4875](https://github.com/junegunn/fzf/pull/4875) | 2 | PASS | PASS | [detail](batch-raw/2026-08-04/junegunn_fzf__pr4875.md) |
| psf/requests | [#7596](https://github.com/psf/requests/pull/7596) | 5 | PASS | PASS | [detail](batch-raw/2026-08-04/psf_requests__pr7596.md) |
| axios/axios | [#11109](https://github.com/axios/axios/pull/11109) | 3 | PASS | PASS | [detail](batch-raw/2026-08-04/axios_axios__pr11109.md) |
| gin-gonic/gin | [#4709](https://github.com/gin-gonic/gin/pull/4709) | 1 | BLOCK | PASS | [detail](batch-raw/2026-08-04/gin-gonic_gin__pr4709.md) |
| clap-rs/clap | [#6455](https://github.com/clap-rs/clap/pull/6455) | 3 | PASS | PASS | [detail](batch-raw/2026-08-04/clap-rs_clap__pr6455.md) |
| cli/cli | [#13987](https://github.com/cli/cli/pull/13987) | 3 | PASS | PASS | [detail](batch-raw/2026-08-04/cli_cli__pr13987.md) |
| pallets/flask | [#6013](https://github.com/pallets/flask/pull/6013) | 2 | PASS | PASS | [detail](batch-raw/2026-08-04/pallets_flask__pr6013.md) |
| expressjs/express | [#7366](https://github.com/expressjs/express/pull/7366) | 3 | PASS | PASS | [detail](batch-raw/2026-08-04/expressjs_express__pr7366.md) |
| sindresorhus/got | [#2454](https://github.com/sindresorhus/got/pull/2454) | 3 | PASS | PASS | [detail](batch-raw/2026-08-04/sindresorhus_got__pr2454.md) |

## 判定核验：gin#4709 的 BLOCK 是低频高置信误报

本轮唯一非 PASS。实跑上游测试核验，**判定错误**：

```
$ go test -run TestSaveUploadedFileWithPermissionFailed -v .
--- PASS: TestSaveUploadedFileWithPermissionFailed (0.00s)
```

发现的论据是「改用 `t.TempDir()` 后目录干净可写，`MkdirAll` 和 `Create` 都会成功，而测试仍断言
`require.Error`，故必然失败」。漏掉了同一行的 `mode`：

```go
var mode fs.FileMode = 0o644
dst := filepath.Join(t.TempDir(), "test", "permission_test")
require.Error(t, c.SaveUploadedFile(f, dst, mode))
```

`SaveUploadedFile`（`context.go:729`）以 `mode` 建父目录后还显式 `os.Chmod(dir, mode)`，0o644 无 x 位，
非 root 下在其中建文件即 permission denied——错误路径依然成立，只是触发机制从「同名文件挡路」换成
「父目录不可进入」。上游改动（去掉对工作目录的依赖以修 WSL）成立。

**一处需要说清的边界**：该结论依赖非 root。root 有 CAP_DAC_OVERRIDE，可无视缺失的 x 位。容器内实测：

```
uid=0     mkdir -m 644 d && touch d/f  →  OK      （测试会失败）
uid=1000  mkdir -m 644 d && touch d/f  →  EACCES  （测试通过）
```

所以「测试必挂」只在 root 环境成立。但发现的**推理链在任何 uid 下都是错的**——它断言的原因是「TempDir
可写所以写入成功」，从未提及 mode 或 uid。这是结论偶然半对、依据完全错，不是一个抓到了真问题的告警。
（gin 的 CI 以非 root 跑，故上游 CI 绿是符合预期的；root 容器里跑测试确实会挂，属该测试本身的脆弱性。）

### 复现频率

同一 diff、同一二进制、`--profile gate`（与批量评测同命令）、`--show-filtered`，连跑 34 轮：

| 轮次 | 判定 | 维度 | severity | confidence | 被 judge 过滤 | 耗时 |
|---:|---|---|---|---:|---|---:|
| 9 | BLOCK | logic | high | 0.96 | 否 | 274s |
| 11 | WARN | logic | high | 0.74 | 否 | 251s |
| 20 | BLOCK | logic | high | 0.98 | 否 | 352s |
| 22 | WARN | logic | high | 0.76 | 否 | 405s |
| 33 | BLOCK | logic | high | 0.98 | 否 | 278s |
| 其余 29 | PASS | — | — | — | — | 中位数 143s |

**命中 5/34 = 14.7%**（Wilson 95% CI 6.4%–30.1%）；并入 07-30 / 08-03 / 08-04 三次批量（2 命中）为
**7/37 = 18.9%**（CI 9.5%–34.2%）。其中 BLOCK 3/34 ≈ 8.8%，即每 11 次左右就会拦一次正当 PR。

三条可据以定位的事实：

- **维度归属不稳定**：34 轮复现里 5 次全部出自 `logic`，而 08-04 批量日志把同一条标为 `ai_smell`。
  另跑 11 轮 `--dimensions ai_smell` 单维为 0 命中——单维复现不出来，是因为它本就主要由 logic 生成。
- **置信度双峰且跨阈值**：同一个错误论断，confidence 落在 {0.74, 0.76} 或 {0.96, 0.98, 0.98} 两簇，
  severity 恒为 high。前簇降级 WARN，后簇直接 BLOCK——判定差异完全由这次抽样落在阈值哪边决定。
- **反证 judge 从未拦下**：5 次 `filtered` 全为 false。所以问题不在 judge 阈值，在生成侧的置信度标注。

现有的同维度多采样中位数共识对它无效：14.7% 的命中率下，samples=2 时两次都命中的概率仅约 2%，
绝大多数情况下这条发现在组内只有一条，`merge_group` 的共识分支根本不触发，保留单次原值。
这正是已知遗留「只命中一次采样的发现没有共识可取」在真实 PR 上的实例。

**成本旁证**：命中轮耗时 251–405s，PASS 轮中位数 143s。发现一旦生成就要走反证 judge，耗时翻倍——
批量评测里 gin 那次 BLOCK 跑了 2 分多钟是同一原因。

## 方法

1. `gh pr view` 取 base/head OID
2. blobless clone + checkout head
3. `reviewgate review --from base --to head --profile gate`
4. `reviewgate security --from base --to head`
5. 原始日志: `docs/evals/batch-raw/2026-08-04/`

## 说明

- 判定为 LLM 静态闸口结果，**不替代**测试与人工 review
- incomplete/超时会降级 WARN，不伪装 PASS
- 干净合并 PR 期望多为 PASS 或低噪声 WARN；security 深审可能更严

## 原始文件

```
drwxr-xr-x@ 32 mengmian  staff  1024  8月  4 15:19 .
drwxr-xr-x@  5 mengmian  staff   160  8月  4 14:25 ..
-rw-r--r--@  1 mengmian  staff   910  8月  4 14:46 axios_axios__pr11109__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 14:47 axios_axios__pr11109__security.log
-rw-r--r--@  1 mengmian  staff  2559  8月  4 14:47 axios_axios__pr11109.md
-rw-r--r--@  1 mengmian  staff   911  8月  4 14:37 BurntSushi_ripgrep__pr3496__review.log
-rw-r--r--@  1 mengmian  staff   931  8月  4 14:39 BurntSushi_ripgrep__pr3496__security.log
-rw-r--r--@  1 mengmian  staff  2441  8月  4 14:39 BurntSushi_ripgrep__pr3496.md
-rw-r--r--@  1 mengmian  staff   911  8月  4 14:54 clap-rs_clap__pr6455__review.log
-rw-r--r--@  1 mengmian  staff   933  8月  4 14:56 clap-rs_clap__pr6455__security.log
-rw-r--r--@  1 mengmian  staff  2561  8月  4 14:56 clap-rs_clap__pr6455.md
-rw-r--r--@  1 mengmian  staff   912  8月  4 14:59 cli_cli__pr13987__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 14:59 cli_cli__pr13987__security.log
-rw-r--r--@  1 mengmian  staff  2532  8月  4 14:59 cli_cli__pr13987.md
-rw-r--r--@  1 mengmian  staff   911  8月  4 15:13 expressjs_express__pr7366__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 15:15 expressjs_express__pr7366__security.log
-rw-r--r--@  1 mengmian  staff  2509  8月  4 15:15 expressjs_express__pr7366.md
-rw-r--r--@  1 mengmian  staff  2623  8月  4 14:50 gin-gonic_gin__pr4709__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 14:50 gin-gonic_gin__pr4709__security.log
-rw-r--r--@  1 mengmian  staff  4138  8月  4 14:50 gin-gonic_gin__pr4709.md
-rw-r--r--@  1 mengmian  staff  1001  8月  4 14:39 junegunn_fzf__pr4875__review.log
-rw-r--r--@  1 mengmian  staff  1022  8月  4 14:40 junegunn_fzf__pr4875__security.log
-rw-r--r--@  1 mengmian  staff  2582  8月  4 14:40 junegunn_fzf__pr4875.md
-rw-r--r--@  1 mengmian  staff   910  8月  4 15:00 pallets_flask__pr6013__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 15:01 pallets_flask__pr6013__security.log
-rw-r--r--@  1 mengmian  staff  2435  8月  4 15:01 pallets_flask__pr6013.md
-rw-r--r--@  1 mengmian  staff   911  8月  4 14:41 psf_requests__pr7596__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 14:42 psf_requests__pr7596__security.log
-rw-r--r--@  1 mengmian  staff  2516  8月  4 14:42 psf_requests__pr7596.md
-rw-r--r--@  1 mengmian  staff   911  8月  4 15:19 sindresorhus_got__pr2454__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  4 15:20 sindresorhus_got__pr2454__security.log
-rw-r--r--@  1 mengmian  staff  2550  8月  4 15:20 sindresorhus_got__pr2454.md
```
