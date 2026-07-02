# 召回评测汇总（revert 法）— 2026-07-02

数据集：`dataset-recall.tsv` · 配置：`reviewgate.toml` · 单维度 timeout=300s

| 指标 | 值 |
|---|---|
| 计入评分 | 2 |
| HIT（block） | 1 |
| HIT（warn） | 1 |
| MISS | 0 |
| **召回率（提示即算）** | **100%** |
| 严格召回（block） | 50% |
| INCOMPLETE（不计入） | 1 |
| ERROR | 0 |

## 逐条明细

| repo | pr | verdict | decision | expect |
|---|---|---|---|---|
| date-fns/date-fns | 1588 | HIT | block | `addBusinessDays` |
| syncthing/syncthing | 10738 | HIT(warn) | warn | `lib/scanner/blocks.go` |
| syncthing/syncthing | 10170 | INCOMPLETE | warn | `lib/` |

> 方法学：revert 修复 PR 的源文件（revert_globs 排除测试/CHANGELOG 避免提示污染）。
> 详见 dataset-recall.tsv 头部注释与 2026-06-25__recall-cve-reverts.md。
