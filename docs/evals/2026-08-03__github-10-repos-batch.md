# GitHub 10 仓真实评测：review + security

- 日期: 2026-08-03
- 工具: `reviewgate 0.10.0`
- 模型/配置: `/Users/mengmian/Develop/app/ReviewGate/reviewgate.toml`（provider 见配置，不写密钥）
- timeout: review=200s · security=200s
- 范围: 各仓 1 个已合并 PR 的 base…head

## 汇总表

| 仓库 | PR | 文件数 | review | security | 详情 |
|---|---|---:|---|---|---|
| BurntSushi/ripgrep | [#3496](https://github.com/BurntSushi/ripgrep/pull/3496) | 1 | PASS | PASS | [detail](batch-raw/2026-08-03/BurntSushi_ripgrep__pr3496.md) |
| junegunn/fzf | [#4875](https://github.com/junegunn/fzf/pull/4875) | 2 | PASS | PASS | [detail](batch-raw/2026-08-03/junegunn_fzf__pr4875.md) |
| psf/requests | [#7596](https://github.com/psf/requests/pull/7596) | 5 | PASS | PASS | [detail](batch-raw/2026-08-03/psf_requests__pr7596.md) |
| axios/axios | [#11109](https://github.com/axios/axios/pull/11109) | 3 | PASS | PASS | [detail](batch-raw/2026-08-03/axios_axios__pr11109.md) |
| gin-gonic/gin | [#4709](https://github.com/gin-gonic/gin/pull/4709) | 1 | PASS | PASS | [detail](batch-raw/2026-08-03/gin-gonic_gin__pr4709.md) |
| clap-rs/clap | [#6455](https://github.com/clap-rs/clap/pull/6455) | 3 | WARN | PASS | [detail](batch-raw/2026-08-03/clap-rs_clap__pr6455.md) |
| cli/cli | [#13987](https://github.com/cli/cli/pull/13987) | 3 | PASS | PASS | [detail](batch-raw/2026-08-03/cli_cli__pr13987.md) |
| pallets/flask | [#6013](https://github.com/pallets/flask/pull/6013) | 2 | PASS | PASS | [detail](batch-raw/2026-08-03/pallets_flask__pr6013.md) |
| expressjs/express | [#7366](https://github.com/expressjs/express/pull/7366) | 3 | PASS | PASS | [detail](batch-raw/2026-08-03/expressjs_express__pr7366.md) |
| sindresorhus/got | [#2454](https://github.com/sindresorhus/got/pull/2454) | 3 | WARN | PASS | [detail](batch-raw/2026-08-03/sindresorhus_got__pr2454.md) |

## 方法

1. `gh pr view` 取 base/head OID
2. blobless clone + checkout head
3. `reviewgate review --from base --to head --profile gate`
4. `reviewgate security --from base --to head`
5. 原始日志: `docs/evals/batch-raw/2026-08-03/`

## 说明

- 判定为 LLM 静态闸口结果，**不替代**测试与人工 review
- incomplete/超时会降级 WARN，不伪装 PASS
- 干净合并 PR 期望多为 PASS 或低噪声 WARN；security 深审可能更严

## 原始文件

```
drwxr-xr-x@ 32 mengmian  staff  1024  8月  3 14:06 .
drwxr-xr-x@  4 mengmian  staff   128  8月  3 13:39 ..
-rw-r--r--@  1 mengmian  staff   910  8月  3 13:48 axios_axios__pr11109__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  3 13:48 axios_axios__pr11109__security.log
-rw-r--r--@  1 mengmian  staff  2559  8月  3 13:48 axios_axios__pr11109.md
-rw-r--r--@  1 mengmian  staff   911  8月  3 13:42 BurntSushi_ripgrep__pr3496__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  3 13:43 BurntSushi_ripgrep__pr3496__security.log
-rw-r--r--@  1 mengmian  staff  2440  8月  3 13:43 BurntSushi_ripgrep__pr3496.md
-rw-r--r--@  1 mengmian  staff  3274  8月  3 13:55 clap-rs_clap__pr6455__review.log
-rw-r--r--@  1 mengmian  staff   932  8月  3 13:56 clap-rs_clap__pr6455__security.log
-rw-r--r--@  1 mengmian  staff  4923  8月  3 13:56 clap-rs_clap__pr6455.md
-rw-r--r--@  1 mengmian  staff   912  8月  3 13:58 cli_cli__pr13987__review.log
-rw-r--r--@  1 mengmian  staff   929  8月  3 13:58 cli_cli__pr13987__security.log
-rw-r--r--@  1 mengmian  staff  2531  8月  3 13:58 cli_cli__pr13987.md
-rw-r--r--@  1 mengmian  staff   911  8月  3 14:02 expressjs_express__pr7366__review.log
-rw-r--r--@  1 mengmian  staff   931  8月  3 14:03 expressjs_express__pr7366__security.log
-rw-r--r--@  1 mengmian  staff  2510  8月  3 14:03 expressjs_express__pr7366.md
-rw-r--r--@  1 mengmian  staff   909  8月  3 13:50 gin-gonic_gin__pr4709__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  3 13:51 gin-gonic_gin__pr4709__security.log
-rw-r--r--@  1 mengmian  staff  2424  8月  3 13:51 gin-gonic_gin__pr4709.md
-rw-r--r--@  1 mengmian  staff  1001  8月  3 13:44 junegunn_fzf__pr4875__review.log
-rw-r--r--@  1 mengmian  staff  1022  8月  3 13:44 junegunn_fzf__pr4875__security.log
-rw-r--r--@  1 mengmian  staff  2582  8月  3 13:44 junegunn_fzf__pr4875.md
-rw-r--r--@  1 mengmian  staff   909  8月  3 13:59 pallets_flask__pr6013__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  3 14:00 pallets_flask__pr6013__security.log
-rw-r--r--@  1 mengmian  staff  2434  8月  3 14:00 pallets_flask__pr6013.md
-rw-r--r--@  1 mengmian  staff   911  8月  3 13:45 psf_requests__pr7596__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  3 13:46 psf_requests__pr7596__security.log
-rw-r--r--@  1 mengmian  staff  2516  8月  3 13:46 psf_requests__pr7596.md
-rw-r--r--@  1 mengmian  staff  1972  8月  3 14:06 sindresorhus_got__pr2454__review.log
-rw-r--r--@  1 mengmian  staff   930  8月  3 14:07 sindresorhus_got__pr2454__security.log
-rw-r--r--@  1 mengmian  staff  3611  8月  3 14:07 sindresorhus_got__pr2454.md
```
