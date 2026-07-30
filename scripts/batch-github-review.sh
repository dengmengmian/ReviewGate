#!/usr/bin/env bash
# 对 10 个真实 GitHub PR 跑 review + security，结果写入 docs/evals/。
# 用法：scripts/batch-github-review.sh
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${REVIEWGATE_CONFIG:-$ROOT/reviewgate.toml}"
RG="${REVIEWGATE_BIN:-$ROOT/target/release/reviewgate}"
DATE="$(date +%Y-%m-%d)"
WORK="${TMPDIR:-/tmp}/reviewgate-batch-$$"
OUT_DIR="$ROOT/docs/evals"
SUMMARY="$OUT_DIR/${DATE}__github-10-repos-batch.md"
RAW_DIR="$OUT_DIR/batch-raw/${DATE}"
mkdir -p "$OUT_DIR" "$RAW_DIR" "$WORK"

if [ ! -x "$RG" ]; then
  echo "building release…"
  (cd "$ROOT" && cargo build --release -p reviewgate -q) || exit 1
  RG="$ROOT/target/release/reviewgate"
fi

if [ -z "${REVIEWGATE_API_KEY:-}" ] && [ -n "${DEEPSEEK_API_KEY:-}" ]; then
  export REVIEWGATE_API_KEY="$DEEPSEEK_API_KEY"
fi
export REVIEWGATE_CONFIG="$CONFIG"

# repo pr  （每仓一条适中 PR）
PAIRS=(
  "BurntSushi/ripgrep 3496"
  "junegunn/fzf 4875"
  "psf/requests 7596"
  "axios/axios 11109"
  "gin-gonic/gin 4709"
  "clap-rs/clap 6455"
  "cli/cli 13987"
  "pallets/flask 6013"
  "expressjs/express 7366"
  "sindresorhus/got 2454"
)

REVIEW_TIMEOUT="${REVIEW_TIMEOUT:-200}"
SECURITY_TIMEOUT="${SECURITY_TIMEOUT:-200}"

run_one() {
  local REPO="$1" PR="$2"
  local SLUG OUT META TITLE BASE HEAD URL CLONE DIFF_STAT
  SLUG="$(echo "$REPO" | tr '/' '_')"
  OUT="$RAW_DIR/${SLUG}__pr${PR}.md"
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "▶ $REPO#$PR"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

  META="$(gh pr view "$PR" --repo "$REPO" --json title,baseRefOid,headRefOid,url,changedFiles,additions,deletions \
    --jq '[.title,.baseRefOid,.headRefOid,.url,(.changedFiles|tostring),(.additions|tostring),(.deletions|tostring)]|@tsv' 2>/dev/null)" || true
  if [ -z "$META" ]; then
    echo "FAIL: cannot fetch PR meta for $REPO#$PR" | tee -a "$OUT"
    echo "| $REPO | #$PR | — | ERROR | ERROR | meta |" >> "$SUMMARY.tsv"
    return 1
  fi
  IFS=$'\t' read -r TITLE BASE HEAD URL FILES ADD DEL <<< "$META"

  CLONE="$WORK/$SLUG"
  if [ ! -d "$CLONE/.git" ]; then
    echo "  clone (blobless)…"
    git clone --quiet --filter=blob:none "https://github.com/$REPO.git" "$CLONE" || {
      echo "FAIL: clone $REPO"
      echo "| $REPO | #$PR | $TITLE | ERROR | ERROR | clone |" >> "$SUMMARY.tsv"
      return 1
    }
  fi
  (
    cd "$CLONE" || exit 1
    git fetch --quiet origin "$BASE" "$HEAD" 2>/dev/null || true
    git checkout --quiet "$HEAD" 2>/dev/null || git checkout --quiet -f "$HEAD"
  )

  DIFF_STAT="$(cd "$CLONE" && git diff --stat "$BASE"..."$HEAD" 2>/dev/null | tail -5)"

  {
    echo "# $REPO#$PR — $TITLE"
    echo
    echo "- URL: $URL"
    echo "- base: \`$BASE\`"
    echo "- head: \`$HEAD\`"
    echo "- files: $FILES · +$ADD/-$DEL"
    echo "- date: $DATE"
    echo "- tool: \`$("$RG" --version 2>/dev/null | tr -d '\n')\` · timeout=${REVIEW_TIMEOUT}s"
    echo
    echo "## Diff stat"
    echo '```'
    echo "$DIFF_STAT"
    echo '```'
    echo
  } > "$OUT"

  # --- normal review ---
  echo "  review …"
  local REV_LOG="$RAW_DIR/${SLUG}__pr${PR}__review.log"
  local REV_EC=0
  set +e
  (
    cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" review \
      --from "$BASE" --to "$HEAD" \
      --profile gate \
      --timeout "$REVIEW_TIMEOUT" \
      --fail-on never \
      --format text \
      2>&1
  ) | tee "$REV_LOG"
  REV_EC=${PIPESTATUS[0]}
  set -e

  local REV_DEC REV_INC
  REV_DEC="$(grep -E '通过|警告|阻断|PASS|WARN|BLOCK' "$REV_LOG" | head -3 | tr '\n' ' ' | sed 's/  */ /g')"
  REV_INC="$(grep -c 'timed_out\|incomplete\|不完整' "$REV_LOG" || true)"
  {
    echo "## review (gate profile)"
    echo
    echo "- exit: $REV_EC"
    echo "- incomplete_signals: $REV_INC"
    echo
    echo '```text'
    # keep report readable: head of log if huge
    if [ "$(wc -l < "$REV_LOG")" -gt 200 ]; then
      head -120 "$REV_LOG"
      echo "… (truncated, full: ${SLUG}__pr${PR}__review.log) …"
      tail -40 "$REV_LOG"
    else
      cat "$REV_LOG"
    fi
    echo '```'
    echo
  } >> "$OUT"

  # --- security deep ---
  echo "  security …"
  local SEC_LOG="$RAW_DIR/${SLUG}__pr${PR}__security.log"
  local SEC_EC=0
  set +e
  (
    cd "$CLONE" && REVIEWGATE_CONFIG="$CONFIG" "$RG" security \
      --from "$BASE" --to "$HEAD" \
      --timeout "$SECURITY_TIMEOUT" \
      --fail-on never \
      --format text \
      2>&1
  ) | tee "$SEC_LOG"
  SEC_EC=${PIPESTATUS[0]}
  set -e

  local SEC_DEC SEC_INC
  SEC_DEC="$(grep -E '通过|警告|阻断|PASS|WARN|BLOCK' "$SEC_LOG" | head -3 | tr '\n' ' ' | sed 's/  */ /g')"
  SEC_INC="$(grep -c 'timed_out\|incomplete\|不完整' "$SEC_LOG" || true)"
  {
    echo "## security (deep)"
    echo
    echo "- exit: $SEC_EC"
    echo "- incomplete_signals: $SEC_INC"
    echo
    echo '```text'
    if [ "$(wc -l < "$SEC_LOG")" -gt 200 ]; then
      head -120 "$SEC_LOG"
      echo "… (truncated, full: ${SLUG}__pr${PR}__security.log) …"
      tail -40 "$SEC_LOG"
    else
      cat "$SEC_LOG"
    fi
    echo '```'
  } >> "$OUT"

  # extract gate decision from status line (优先状态行，避免把后文「警告」段落误判)
  decision_of() {
    python3 - "$1" <<'PY'
import re, sys
t = open(sys.argv[1], encoding="utf-8", errors="replace").read()
# status line: ✓ 通过 / ⚠ 警告 / ✖ 阻断  or PASS/WARN/BLOCK
for pat in (
    r"[✓✔]\s*(通过|PASS)",
    r"[⚠]\s*(警告|WARN)",
    r"[✖✕]\s*(阻断|拦截|BLOCK)",  # 中文 locale：Block → 拦截
    r"\b(PASS|WARN|BLOCK)\b",
    r"(通过|警告|阻断|拦截)",
):
    m = re.search(pat, t)
    if m:
        s = m.group(1)
        print({"通过": "PASS", "警告": "WARN", "阻断": "BLOCK", "拦截": "BLOCK"}.get(s, s))
        raise SystemExit
print("?")
PY
  }
  local RD SD
  RD="$(decision_of "$REV_LOG")"
  SD="$(decision_of "$SEC_LOG")"

  echo "| $REPO | [#$PR]($URL) | $FILES | $RD | $SD | [detail](batch-raw/${DATE}/${SLUG}__pr${PR}.md) |" >> "$SUMMARY.rows"
  echo "  done: review=$RD security=$SD → $OUT"
}

# header
{
  echo "# GitHub 10 仓真实评测：review + security"
  echo
  echo "- 日期: $DATE"
  echo "- 工具: \`$("$RG" --version 2>/dev/null | tr -d '\n')\`"
  echo "- 模型/配置: \`$CONFIG\`（provider 见配置，不写密钥）"
  echo "- timeout: review=${REVIEW_TIMEOUT}s · security=${SECURITY_TIMEOUT}s"
  echo "- 范围: 各仓 1 个已合并 PR 的 base…head"
  echo
  echo "## 汇总表"
  echo
  echo "| 仓库 | PR | 文件数 | review | security | 详情 |"
  echo "|---|---|---:|---|---|---|"
} > "$SUMMARY"

: > "$SUMMARY.rows"
: > "$SUMMARY.tsv"

FAILS=0
for pair in "${PAIRS[@]}"; do
  set -- $pair
  if ! run_one "$1" "$2"; then
    FAILS=$((FAILS + 1))
  fi
done

cat "$SUMMARY.rows" >> "$SUMMARY"

{
  echo
  echo "## 方法"
  echo
  echo "1. \`gh pr view\` 取 base/head OID"
  echo "2. blobless clone + checkout head"
  echo "3. \`reviewgate review --from base --to head --profile gate\`"
  echo "4. \`reviewgate security --from base --to head\`"
  echo "5. 原始日志: \`docs/evals/batch-raw/${DATE}/\`"
  echo
  echo "## 说明"
  echo
  echo "- 判定为 LLM 静态闸口结果，**不替代**测试与人工 review"
  echo "- incomplete/超时会降级 WARN，不伪装 PASS"
  echo "- 干净合并 PR 期望多为 PASS 或低噪声 WARN；security 深审可能更严"
  echo
  echo "## 原始文件"
  echo
  echo "\`\`\`"
  ls -la "$RAW_DIR" | tail -n +2
  echo "\`\`\`"
} >> "$SUMMARY"

echo
echo "✓ 汇总: $SUMMARY"
echo "  fails(meta/clone): $FAILS"
echo "  raw: $RAW_DIR"
