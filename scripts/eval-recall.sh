#!/usr/bin/env bash
# 召回评测：revert 法。把真实修复 PR 的修复撤销（重新引入 bug），看 ReviewGate 能否命中。
#
# 用法：scripts/eval-recall.sh [dataset.tsv] [--dry-run]
#   默认数据集：docs/evals/dataset-recall.tsv
#   --dry-run   只验证克隆/revert/diff 机械部分，不调 LLM（零成本自检）。
#
# 依赖：git、jq、cargo。API key 走 reviewgate.toml 或 REVIEWGATE_API_KEY（env 覆盖 config）。
#
# 与 eval-batch.sh（clean PR 测误报率/precision）互补：本脚本测**召回/recall**。
# 判定：
#   HIT         命中（finding 落在 expect_path）且 decision=block
#   HIT(warn)   命中但仅 warn —— 已提示未拦截
#   MISS        未命中 —— 静默放行（最糟）
#   INCOMPLETE  未审完且未命中 —— 不计入分母，单独报告
# 召回率 = (HIT + HIT(warn)) / (HIT + HIT(warn) + MISS)；同时报告严格 block-召回。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="$ROOT/reviewgate.toml"
EVAL_DIR="$ROOT/docs/evals"
DATASET="$EVAL_DIR/dataset-recall.tsv"
DRY=0
for a in "$@"; do
  case "$a" in
    --dry-run) DRY=1 ;;
    *) DATASET="$a" ;;
  esac
done
WORK="${TMPDIR:-/tmp}/reviewgate-eval-recall"
DATE="$(date +%Y-%m-%d)"
SUMMARY="$EVAL_DIR/${DATE}__recall-summary.md"
TIMEOUT="${REVIEWGATE_EVAL_TIMEOUT:-300}"

for bin in git jq cargo; do
  command -v "$bin" >/dev/null 2>&1 || { echo "缺少依赖：$bin"; exit 1; }
done
[ -f "$DATASET" ] || { echo "数据集不存在：$DATASET"; exit 1; }
if [ "$DRY" = 0 ] && [ ! -f "$CONFIG" ] && [ -z "${REVIEWGATE_API_KEY:-}" ]; then
  echo "找不到 $CONFIG 且未设置 REVIEWGATE_API_KEY —— 真实评测需要 LLM。机械自检请加 --dry-run。"
  exit 1
fi

mkdir -p "$EVAL_DIR" "$WORK"

if [ "$DRY" = 0 ]; then
  echo "▶ 构建 reviewgate (release)…"
  ( cd "$ROOT" && cargo build --release -q ) || { echo "构建失败"; exit 1; }
fi
RG="$ROOT/target/release/reviewgate"

# 带重试的 clone/fetch（代理环境偶发抖动）。
clone_repo() { # $1 url  $2 dir
  [ -d "$2/.git" ] && return 0
  for _ in 1 2 3 4 5; do
    git clone -q --filter=blob:none "$1" "$2" 2>/dev/null && return 0
    rm -rf "$2"
  done
  return 1
}
fetch_shas() { # $1 dir  $2 base  $3 head
  for _ in 1 2 3; do
    ( cd "$1" && git fetch -q origin "$2" "$3" 2>/dev/null ) && return 0
  done
  return 1
}

total=0; scored=0; hit_block=0; hit_warn=0; miss=0; incomplete=0; errored=0
rows=()

printf '%-22s %-7s %-11s %-9s %s\n' "REPO" "PR" "VERDICT" "DECISION" "EXPECT"
printf '%-22s %-7s %-11s %-9s %s\n' "----" "--" "-------" "--------" "------"

while IFS=$'\t' read -r REPO PR BASE HEAD EXPECT GLOBS NOTE; do
  REPO="$(echo "${REPO:-}" | xargs)"
  case "$REPO" in ''|\#*) continue;; esac
  PR="$(echo "$PR" | xargs)"; BASE="$(echo "$BASE" | xargs)"; HEAD="$(echo "$HEAD" | xargs)"
  EXPECT="$(echo "$EXPECT" | xargs)"; GLOBS="$(echo "${GLOBS:--}" | xargs)"
  total=$((total+1))

  SLUG="$(echo "$REPO" | tr '/' '_')"
  CLONE="$WORK/$SLUG"
  if ! clone_repo "https://github.com/$REPO.git" "$CLONE"; then
    printf '%-22s %-7s %s\n' "$REPO" "$PR" "ERROR(clone)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | — | — | ERROR(clone) |"); continue
  fi
  if ! fetch_shas "$CLONE" "$BASE" "$HEAD"; then
    printf '%-22s %-7s %s\n' "$REPO" "$PR" "ERROR(fetch)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | — | — | ERROR(fetch) |"); continue
  fi

  # 就位到修复后的 head，再把改动文件退回 base ⇒ 工作区 diff = bug 重新引入。
  # revert_globs 作为 git pathspec，`-` 表示全部改动文件。
  (
    cd "$CLONE" || exit 9
    git checkout -qf "$HEAD" && git clean -qfd || exit 9
    if [ "$GLOBS" = "-" ]; then
      git checkout -q "$BASE" -- . 2>/dev/null \
        || git diff --name-only "$BASE" "$HEAD" -z | xargs -0 git checkout -q "$BASE" -- 2>/dev/null
    else
      # shellcheck disable=SC2086 — globs 按空格拆成多个 pathspec 是有意的
      git checkout -q "$BASE" -- $GLOBS || exit 9
    fi
    git diff --quiet HEAD && exit 8   # revert 后无 diff：修复不在 globs 范围内？
    exit 0
  )
  rc=$?
  if [ $rc = 8 ]; then
    printf '%-22s %-7s %s\n' "$REPO" "$PR" "ERROR(empty-diff)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | — | — | ERROR(empty-diff) |"); continue
  elif [ $rc != 0 ]; then
    printf '%-22s %-7s %s\n' "$REPO" "$PR" "ERROR(revert)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | — | — | ERROR(revert) |"); continue
  fi

  if [ "$DRY" = 1 ]; then
    STAT="$(cd "$CLONE" && git --no-pager diff --stat HEAD | tail -1 | xargs)"
    printf '%-22s %-7s %-11s %s\n' "$REPO" "$PR" "DRY-OK" "$STAT"
    rows+=("| $REPO | $PR | dry | — | $STAT |")
    ( cd "$CLONE" && git checkout -qf "$HEAD" )
    continue
  fi

  JSON="$( cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" review --format json --timeout "$TIMEOUT" 2>/dev/null )"
  ( cd "$CLONE" && git checkout -qf "$HEAD" )   # 立即还原，失败也不留脏工作树
  if ! echo "$JSON" | jq -e . >/dev/null 2>&1; then
    printf '%-22s %-7s %s\n' "$REPO" "$PR" "ERROR(json)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | — | — | ERROR(json) |"); continue
  fi

  DEC="$(echo "$JSON" | jq -r '.decision')"
  INC="$(echo "$JSON" | jq -r '.incomplete')"
  HITS="$(echo "$JSON" | jq --arg p "$EXPECT" \
    '[.findings[] | select(.filtered==false) | select(.path | contains($p))] | length')"

  if [ "$HITS" -gt 0 ] && [ "$DEC" = "block" ]; then
    verdict="HIT"; hit_block=$((hit_block+1)); scored=$((scored+1))
  elif [ "$HITS" -gt 0 ]; then
    verdict="HIT(warn)"; hit_warn=$((hit_warn+1)); scored=$((scored+1))
  elif [ "$INC" = "true" ]; then
    verdict="INCOMPLETE"; incomplete=$((incomplete+1))
  else
    verdict="MISS"; miss=$((miss+1)); scored=$((scored+1))
  fi
  printf '%-22s %-7s %-11s %-9s %s\n' "$REPO" "$PR" "$verdict" "$DEC" "$EXPECT"
  rows+=("| $REPO | $PR | $verdict | $DEC | \`$EXPECT\` |")
done < "$DATASET"

recall="n/a"; brecall="n/a"
if [ "$scored" -gt 0 ]; then
  recall="$(awk "BEGIN{printf \"%.0f%%\", ($hit_block+$hit_warn)/$scored*100}")"
  brecall="$(awk "BEGIN{printf \"%.0f%%\", $hit_block/$scored*100}")"
fi

echo
echo "================ 召回总分 ================"
echo "数据集条目        : $total"
echo "计入评分          : $scored"
echo "  ├ HIT(block)    : $hit_block"
echo "  ├ HIT(warn)     : $hit_warn"
echo "  └ MISS          : $miss"
echo "召回率(提示即算)  : $recall"
echo "严格召回(block)   : $brecall"
echo "INCOMPLETE        : $incomplete   （不计入分母）"
echo "ERROR             : $errored"
echo "=========================================="

[ "$DRY" = 1 ] && { echo "(dry-run：未调 LLM，不写汇总)"; exit 0; }

{
  echo "# 召回评测汇总（revert 法）— $DATE"
  echo
  echo "数据集：\`$(basename "$DATASET")\` · 配置：\`reviewgate.toml\` · 单维度 timeout=${TIMEOUT}s"
  echo
  echo "| 指标 | 值 |"
  echo "|---|---|"
  echo "| 计入评分 | $scored |"
  echo "| HIT（block） | $hit_block |"
  echo "| HIT（warn） | $hit_warn |"
  echo "| MISS | $miss |"
  echo "| **召回率（提示即算）** | **$recall** |"
  echo "| 严格召回（block） | $brecall |"
  echo "| INCOMPLETE（不计入） | $incomplete |"
  echo "| ERROR | $errored |"
  echo
  echo "## 逐条明细"
  echo
  echo "| repo | pr | verdict | decision | expect |"
  echo "|---|---|---|---|---|"
  for r in "${rows[@]}"; do echo "$r"; done
  echo
  echo "> 方法学：revert 修复 PR 的源文件（revert_globs 排除测试/CHANGELOG 避免提示污染）。"
  echo "> 详见 dataset-recall.tsv 头部注释与 2026-06-25__recall-cve-reverts.md。"
} > "$SUMMARY"

echo
echo "✓ 汇总已写入：$SUMMARY"
