# 召回评测汇总（revert 法）— 2026-07-02

数据集：`dataset-recall.tsv` · 配置：`reviewgate.toml` · 单维度 timeout=300s

| 指标 | 值 |
|---|---|
| 计入评分 | 2 |
| HIT（block） | 0 |
| HIT（warn） | 2 |
| MISS | 0 |
| **召回率（提示即算）** | **100%** |
| 严格召回（block） | 0% |
| INCOMPLETE（不计入） | 1 |
| ERROR | 0 |

## 逐条明细

| repo | pr | verdict | decision | expect |
|---|---|---|---|---|
| date-fns/date-fns | 1588 | HIT(warn) | warn | `addBusinessDays` |
| syncthing/syncthing | 10738 | HIT(warn) | warn | `lib/scanner/blocks.go` |
| syncthing/syncthing | 10170 | INCOMPLETE | warn | `lib/` |

> 方法学：revert 修复 PR 的源文件（revert_globs 排除测试/CHANGELOG 避免提示污染）。
> 详见 dataset-recall.tsv 头部注释与 2026-06-25__recall-cve-reverts.md。

## 本次运行备注
- #10170（frontier 标本）此次为 INCOMPLETE（大 diff 单维度撞 300s 超时），而非此前手动基线的 MISS——
  超时先于漏报发生，按设计不计入分母。要复测它的真实 MISS，用 `REVIEWGATE_EVAL_TIMEOUT=600` 重跑。
- 本次为 eval-recall.sh 首跑（验证评分器本身）：两个已知可命中用例均正确判 HIT(warn)，与手动基线一致。
