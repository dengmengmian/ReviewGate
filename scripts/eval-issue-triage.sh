#!/usr/bin/env bash
# 真实仓库上的 Issue 分诊评测：同步真实 Issue → 本地分诊 → 出分布。
#
# 为什么要有它：合成样本能测规则对不对，测不出精度。分类关键词、查重阈值这类东西
# 只有在真实分布上才能证伪——`credential` 判安全、中文「泄露」判安全这两次误报，
# 都是靠跑真实仓库才发现的，语料全部放行了。
#
# 绝不发布：全程走 `issue watch`，只同步 + 本地分诊 + 打印。本脚本拒绝 --publish。
#
# 用法：
#   scripts/eval-issue-triage.sh cli/cli 500 [/path/to/checkout]
#   scripts/eval-issue-triage.sh alibaba/arthas 500 ./src-arthas --force-retriage
#   scripts/eval-issue-triage.sh cli/cli 500 -- --llm --no-sync
#
# 两套门禁（见 docs/ISSUE_TRIAGE.md）：
#   无 LLM：本脚本默认（不加 --llm），再 python3 scripts/eval-issue-groundtruth.py <db>
#   有 LLM：同一库加 --llm --force-retriage（改规则后必须强制复审，否则数字是旧的）
#
# 需要 GITHUB_TOKEN（或 `gh auth token`）。结果落在 ./.eval-issue/<repo>/。
set -euo pipefail

usage() {
  cat <<'EOF'
真实仓库上的 Issue 分诊评测：同步真实 Issue → 本地分诊 → 出分布。

全程走 `issue watch`，不发布。本脚本拒绝 --publish。

用法:
  scripts/eval-issue-triage.sh <owner/repo> [max] [repo-root] [flags]
  scripts/eval-issue-triage.sh --help

flags:
  --force-retriage  哈希未变也复审（改规则后重测；旧库不加重跑会 skip）
  --llm             规则没把握时问模型（有 LLM 召回门禁才加）
  --no-sync         只用本地已索引的 Issue，不再打平台 API
  --publish         拒绝。评测禁止往平台写

然后:
  python3 scripts/eval-issue-groundtruth.py .eval-issue/<owner_repo>/issues.db
EOF
}

REPO=""
MAX="500"
REPO_ROOT=""
MAX_SET=0
FORCE=0
LLM=0
NOSYNC=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --force-retriage)
      FORCE=1
      shift
      ;;
    --llm)
      LLM=1
      shift
      ;;
    --no-sync)
      NOSYNC=1
      shift
      ;;
    --publish)
      echo "评测禁止 --publish：本脚本只做本地分诊，不会往平台写" >&2
      exit 2
      ;;
    --)
      shift
      ;;
    -*)
      echo "未知参数: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -z "$REPO" ]]; then
        REPO="$1"
      elif [[ $MAX_SET -eq 0 ]]; then
        MAX="$1"
        MAX_SET=1
      elif [[ -z "$REPO_ROOT" ]]; then
        REPO_ROOT="$1"
      else
        echo "多余参数: $1" >&2
        usage >&2
        exit 2
      fi
      shift
      ;;
  esac
done

if [[ -z "$REPO" ]]; then
  usage >&2
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${REVIEWGATE_BIN:-$ROOT/target/release/reviewgate}"
OUT="$ROOT/.eval-issue/${REPO//\//_}"
mkdir -p "$OUT"

[ -x "$BIN" ] || { echo "先构建：cargo build --release" >&2; exit 1; }
: "${GITHUB_TOKEN:=$(gh auth token 2>/dev/null || true)}"
[ -n "$GITHUB_TOKEN" ] || { echo "需要 GITHUB_TOKEN 或已登录的 gh" >&2; exit 1; }
export GITHUB_TOKEN

WATCH_EXTRA=()
if [[ $FORCE -eq 1 ]]; then
  WATCH_EXTRA+=(--force-retriage)
fi
if [[ $LLM -eq 1 ]]; then
  WATCH_EXTRA+=(--llm)
fi
if [[ $NOSYNC -eq 1 ]]; then
  WATCH_EXTRA+=(--no-sync)
fi

if [[ $NOSYNC -eq 0 ]]; then
  echo "== 1/3 同步 $MAX 条真实 Issue（只读）=="
  "$BIN" issue init --repo "$REPO" --forge github --data-dir "$OUT" --max "$MAX"
else
  echo "== 1/3 跳过同步（--no-sync）=="
fi

echo "== 2/3 本地分诊（不发布）=="
VERIFY=()
[ -n "$REPO_ROOT" ] && VERIFY=(--verify --repo-root "$REPO_ROOT")
"$BIN" issue watch --repo "$REPO" --data-dir "$OUT" \
  --max-iterations 1 --interval 1s --max-issues-per-run "$MAX" \
  "${WATCH_EXTRA[@]}" "${VERIFY[@]}" \
  2>&1 | tee "$OUT/triage.log"

echo "== 3/3 分布 =="
grep -oE '→ [A-Z_]+ \([0-9]+%\) type=[a-z_]+ dup=[a-z_]+' "$OUT/triage.log" |
  awk '{v[$2]++; for(i=1;i<=NF;i++) if($i ~ /^type=/) t[$i]++; n++}
       END {
         printf "共 %d 条\n\n裁决:\n", n
         for (k in v) printf "  %-22s %5d  %5.1f%%\n", k, v[k], 100*v[k]/n
         printf "\n类型:\n"
         for (k in t) printf "  %-22s %5d  %5.1f%%\n", k, t[k], 100*t[k]/n
       }' | sort -k2 -nr -t' '

# panic 是硬失败：中文 Issue 曾经在摘要截断处崩掉整批。
if grep -qi panicked "$OUT/triage.log"; then
  echo "!! 分诊过程中发生 panic，见 $OUT/triage.log" >&2
  exit 1
fi
echo "OK（未发出任何评论）"
