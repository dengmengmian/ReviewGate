#!/usr/bin/env bash
# 用真实 GitHub PR 评测 ReviewGate，并把结果留痕到 docs/evals/。
#
# 用法：scripts/eval-pr.sh <owner/repo> <pr-number>
# 例：  scripts/eval-pr.sh BurntSushi/ripgrep 2800
#
# 依赖：gh（已登录）、git、cargo。
# 配置：用本仓库的 reviewgate.toml（provider=deepseek）；API key 走 REVIEWGATE_API_KEY 环境变量。
# 没有 key 时仍会跑 `reviewgate diff`（真实世界 diff 解析压测），并记录 review 因缺 key 跳过。
set -uo pipefail

REPO="${1:?用法: eval-pr.sh <owner/repo> <pr-number>}"
PR="${2:?用法: eval-pr.sh <owner/repo> <pr-number>}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="$ROOT/reviewgate.toml"
EVAL_DIR="$ROOT/docs/evals"
WORK="${TMPDIR:-/tmp}/reviewgate-eval"
DATE="$(date +%Y-%m-%d)"
SLUG="$(echo "$REPO" | tr '/' '_')"
OUT="$EVAL_DIR/${DATE}__${SLUG}__pr${PR}.md"

mkdir -p "$EVAL_DIR" "$WORK"

echo "▶ 构建 reviewgate (release)…"
( cd "$ROOT" && cargo build --release -q ) || { echo "构建失败"; exit 1; }
RG="$ROOT/target/release/reviewgate"

echo "▶ 拉取 PR 元数据 $REPO#$PR …"
# 用 gh 内置 --jq 提取，避免 sed 解析特殊字符（标题含引号/反斜杠）出错。
META="$(gh pr view "$PR" --repo "$REPO" --json title,baseRefOid,headRefOid,url \
        --jq '[.title,.baseRefOid,.headRefOid,.url]|@tsv' 2>/dev/null)"
if [ -z "$META" ]; then echo "无法获取 PR 元数据（gh 未登录或 PR 不存在）"; exit 1; fi
IFS=$'\t' read -r TITLE BASE HEAD URL <<< "$META"

CLONE="$WORK/$SLUG"
if [ ! -d "$CLONE/.git" ]; then
  echo "▶ 克隆 ${REPO}（blobless，按需取文件，大仓也快）…"
  # --filter=blob:none：只取提交图与树，blob 在 checkout 时按需拉，避免大仓全量克隆超时。
  git clone --quiet --filter=blob:none "https://github.com/$REPO.git" "$CLONE" || { echo "克隆失败"; exit 1; }
fi
( cd "$CLONE" && git fetch --quiet origin "$BASE" "$HEAD" 2>/dev/null; git checkout --quiet "$HEAD" 2>/dev/null )

echo "▶ 写入 $OUT"
{
  echo "# Eval: $REPO#$PR — $TITLE"
  echo
  echo "- URL: $URL"
  echo "- base: \`$BASE\`  head: \`$HEAD\`"
  echo "- 评测时间: $DATE"
  echo
  echo "## diff 解析（reviewgate diff，无需 LLM）"
  echo '```'
  ( cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" diff --from "$BASE" --to "$HEAD" 2>&1 | head -60 ) || \
  ( cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" diff 2>&1 | head -60 )
  echo '```'
  echo
  echo "## review（reviewgate review，需要 REVIEWGATE_API_KEY）"
  if [ -z "${REVIEWGATE_API_KEY:-}" ]; then
    echo "> ⚠ 未设置 REVIEWGATE_API_KEY，已跳过 LLM 审查。设置后重跑即可获得完整结果。"
  else
    echo '```'
    ( cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" review --from "$BASE" --to "$HEAD" --verbose --format text 2>&1 )
    echo '```'
  fi
} > "$OUT"

echo "✓ 完成：$OUT"
