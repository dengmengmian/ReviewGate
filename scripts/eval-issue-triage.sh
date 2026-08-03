#!/usr/bin/env bash
# 真实仓库上的 Issue 分诊评测：同步真实 Issue → 本地分诊 → 出分布。
#
# 为什么要有它：合成样本能测规则对不对，测不出精度。分类关键词、查重阈值这类东西
# 只有在真实分布上才能证伪——`credential` 判安全、中文「泄露」判安全这两次误报，
# 都是靠跑真实仓库才发现的，语料全部放行了。
#
# 绝不发布：全程走 `issue watch`，它只同步 + 本地分诊 + 打印，不会往平台写任何东西。
#
# 用法：
#   scripts/eval-issue-triage.sh cli/cli 500 [/path/to/checkout]
#   scripts/eval-issue-triage.sh alibaba/arthas 500 ./src-arthas
#
# 需要 GITHUB_TOKEN（或 `gh auth token`）。结果落在 ./.eval-issue/<repo>/。
set -euo pipefail

REPO="${1:?用法: $0 <owner/repo> [max] [repo-root]}"
MAX="${2:-500}"
REPO_ROOT="${3:-}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${REVIEWGATE_BIN:-$ROOT/target/release/reviewgate}"
OUT="$ROOT/.eval-issue/${REPO//\//_}"
mkdir -p "$OUT"

[ -x "$BIN" ] || { echo "先构建：cargo build --release" >&2; exit 1; }
: "${GITHUB_TOKEN:=$(gh auth token 2>/dev/null || true)}"
[ -n "$GITHUB_TOKEN" ] || { echo "需要 GITHUB_TOKEN 或已登录的 gh" >&2; exit 1; }
export GITHUB_TOKEN

echo "== 1/3 同步 $MAX 条真实 Issue（只读）=="
"$BIN" issue init --repo "$REPO" --forge github --data-dir "$OUT" --max "$MAX"

echo "== 2/3 本地分诊（不发布、不调模型）=="
VERIFY=()
[ -n "$REPO_ROOT" ] && VERIFY=(--verify --repo-root "$REPO_ROOT")
"$BIN" issue watch --repo "$REPO" --data-dir "$OUT" \
  --max-iterations 1 --max-issues-per-run "$MAX" "${VERIFY[@]}" \
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
