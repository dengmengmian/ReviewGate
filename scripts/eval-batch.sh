#!/usr/bin/env bash
# 批量评测 ReviewGate：跑数据集里的每个 PR，自动算「误报率」并出总分。
#
# 用法：scripts/eval-batch.sh [dataset.tsv]
#   默认数据集：docs/evals/dataset.tsv
#
# 依赖：gh（已登录）、git、cargo、jq。
# 必需：REVIEWGATE_API_KEY（这是真实 LLM 评测，缺 key 直接退出）。
# 配置：用本仓库 reviewgate.toml（provider=deepseek）。
#
# 这是 eval-pr.sh（单 PR 留痕）的「批量 + 自动打分」版：
#   单 PR 脚本只把原始输出塞进 md，靠人眼看；本脚本读 --format json，逐 PR 判对错，最后出三个数。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="$ROOT/reviewgate.toml"
EVAL_DIR="$ROOT/docs/evals"
DATASET="${1:-$EVAL_DIR/dataset.tsv}"
WORK="${TMPDIR:-/tmp}/reviewgate-eval"
DATE="$(date +%Y-%m-%d)"
SUMMARY="$EVAL_DIR/${DATE}__batch-summary.md"
TIMEOUT="${REVIEWGATE_EVAL_TIMEOUT:-300}"

# ── 前置检查 ──
for bin in gh git cargo jq; do
  command -v "$bin" >/dev/null 2>&1 || { echo "缺少依赖：$bin"; exit 1; }
done
[ -f "$DATASET" ] || { echo "数据集不存在：$DATASET"; exit 1; }
if [ -z "${REVIEWGATE_API_KEY:-}" ]; then
  echo "未设置 REVIEWGATE_API_KEY —— 本脚本需要真实 LLM 评测，请先 export 后重跑。"; exit 1
fi

mkdir -p "$EVAL_DIR" "$WORK"

echo "▶ 构建 reviewgate (release)…"
( cd "$ROOT" && cargo build --release -q ) || { echo "构建失败"; exit 1; }
RG="$ROOT/target/release/reviewgate"

# ── 累加器 ──
total=0; scored=0; passed=0; failed=0; incomplete=0; errored=0
rows=()        # 明细表行（markdown）
fail_detail=() # 误报明细

echo
printf '%-26s %-7s %-9s %-7s %-5s %s\n' "REPO" "PR" "DECISION" "MUST" "INC" "VERDICT"
printf '%-26s %-7s %-9s %-7s %-5s %s\n' "----" "--" "--------" "----" "---" "-------"

while IFS=$'\t' read -r REPO PR LABEL NOTE; do
  # 跳过注释与空行；字段做去空白。
  REPO="$(echo "$REPO" | xargs)"; PR="$(echo "$PR" | xargs)"; LABEL="$(echo "$LABEL" | xargs)"
  case "$REPO" in ''|\#*) continue;; esac
  [ -z "$PR" ] && continue
  total=$((total+1))

  # PR 元数据 → base/head 精确 oid（审 PR 净改动，等同 doc 里的 range 模式）。
  META="$(gh pr view "$PR" --repo "$REPO" --json baseRefOid,headRefOid \
          --jq '[.baseRefOid,.headRefOid]|@tsv' 2>/dev/null)"
  if [ -z "$META" ]; then
    printf '%-26s %-7s %-9s %-7s %-5s %s\n' "$REPO" "$PR" "-" "-" "-" "ERROR(meta)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | $LABEL | — | — | ERROR(meta) |"); continue
  fi
  IFS=$'\t' read -r BASE HEAD <<< "$META"

  SLUG="$(echo "$REPO" | tr '/' '_')"
  CLONE="$WORK/$SLUG"
  if [ ! -d "$CLONE/.git" ]; then
    git clone --quiet --filter=blob:none "https://github.com/$REPO.git" "$CLONE" \
      || { printf '%-26s %-7s %s\n' "$REPO" "$PR" "ERROR(clone)"; errored=$((errored+1)); \
           rows+=("| $REPO | $PR | $LABEL | — | — | ERROR(clone) |"); continue; }
  fi
  ( cd "$CLONE" && git fetch --quiet origin "$BASE" "$HEAD" 2>/dev/null )

  # 真实评测：JSON 输出，machine-readable。stderr（进度行）丢弃。
  JSON="$( cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" review \
            --from "$BASE" --to "$HEAD" --format json --timeout "$TIMEOUT" 2>/dev/null )"
  if ! echo "$JSON" | jq -e . >/dev/null 2>&1; then
    printf '%-26s %-7s %-9s %-7s %-5s %s\n' "$REPO" "$PR" "-" "-" "-" "ERROR(json)"
    errored=$((errored+1)); rows+=("| $REPO | $PR | $LABEL | — | — | ERROR(json) |"); continue
  fi

  DECISION="$(echo "$JSON" | jq -r '.decision')"
  INC="$(echo "$JSON" | jq -r '.incomplete')"
  MUST="$(echo "$JSON" | jq '[.findings[] | select(.filtered==false and .severity=="high")] | length')"

  # ── 评分 ──
  if [ "$INC" = "true" ]; then
    verdict="INCOMPLETE"; incomplete=$((incomplete+1))
  elif [ "$LABEL" = "clean" ]; then
    scored=$((scored+1))
    if [ "$MUST" -eq 0 ] && [ "$DECISION" != "block" ]; then
      verdict="PASS"; passed=$((passed+1))
    else
      verdict="FAIL(误报)"; failed=$((failed+1))
      fp="$(echo "$JSON" | jq -r '[.findings[] | select(.filtered==false and .severity=="high") | "\(.path):\(.start_line) [\(.dimension)] \(.message)"] | join("; ")')"
      fail_detail+=("- **$REPO#$PR**：$fp")
    fi
  else
    verdict="SKIP(未知label:$LABEL)"
  fi

  printf '%-26s %-7s %-9s %-7s %-5s %s\n' "$REPO" "$PR" "$DECISION" "$MUST" "$INC" "$verdict"
  rows+=("| $REPO | $PR | $LABEL | $DECISION | $MUST | $verdict |")
done < "$DATASET"

# ── 汇总 ──
fp_rate="n/a"
if [ "$scored" -gt 0 ]; then
  fp_rate="$(awk "BEGIN{printf \"%.1f%%\", $failed/$scored*100}")"
fi

echo
echo "================ 总分 ================"
echo "数据集条目      : $total"
echo "计入评分(clean) : $scored   （已审完的 clean PR）"
echo "  ├ PASS(无误报): $passed"
echo "  └ FAIL(误报)  : $failed"
echo "误报率          : $fp_rate"
echo "INCOMPLETE      : $incomplete   （端点慢/超时，不计入分母）"
echo "ERROR           : $errored   （元数据/克隆/JSON 失败）"
echo "===================================="

# ── 留痕 ──
{
  echo "# 批量评测汇总 — $DATE"
  echo
  echo "数据集：\`$(basename "$DATASET")\` · 配置：\`reviewgate.toml\` · 单维度 timeout=${TIMEOUT}s"
  echo
  echo "| 指标 | 值 |"
  echo "|---|---|"
  echo "| 数据集条目 | $total |"
  echo "| 计入评分（clean，已审完） | $scored |"
  echo "| PASS（无误报） | $passed |"
  echo "| FAIL（误报） | $failed |"
  echo "| **误报率** | **$fp_rate** |"
  echo "| INCOMPLETE（不计入） | $incomplete |"
  echo "| ERROR | $errored |"
  echo
  echo "## 逐 PR 明细"
  echo
  echo "| repo | pr | label | decision | must-fix | verdict |"
  echo "|---|---|---|---|---|---|"
  for r in "${rows[@]}"; do echo "$r"; done
  if [ "${#fail_detail[@]}" -gt 0 ]; then
    echo
    echo "## 误报明细（需人工复核：是真误报还是该提级的真问题）"
    echo
    for d in "${fail_detail[@]}"; do echo "$d"; done
  fi
} > "$SUMMARY"

echo
echo "✓ 汇总已写入：$SUMMARY"
