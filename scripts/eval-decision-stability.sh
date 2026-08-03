#!/usr/bin/env bash
# 评测「审查范围排除」是否改变闸口判定：同一批仓库跑两轮真实 LLM 审查（baseline vs new）。
#
# 用法：scripts/eval-decision-stability.sh <工作目录> [baseline|new]
#   先跑 scripts/eval-exclude-scope.sh 准备仓库与配置，再对两个配置各跑一次。
#
# 这是**真实 LLM 评测**，会消耗 token。关注点不是"发现更多"，而是判定是否被改动改变。
# 注意：成熟仓库的已合并 commit 通常是干净的（两组都 0 发现），因此还需配合
# scripts/eval-planted-control.sh 的植入对照才有区分力。
set -euo pipefail

WORK="${1:?usage: eval-decision-stability.sh <workdir> [baseline|new]}"
CFG="${2:-new}"
BIN="${BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/reviewgate}"
[ -x "$BIN" ] || BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/reviewgate"
[ -x "$BIN" ] || { echo "找不到 reviewgate 二进制" >&2; exit 2; }
[ -f "$WORK/config-$CFG.toml" ] || { echo "缺 $WORK/config-$CFG.toml，先跑 eval-exclude-scope.sh" >&2; exit 2; }

out="$WORK/results/llm-$CFG.jsonl"
mkdir -p "$WORK/results"; : > "$out"

for dir in "$WORK"/repos/*/; do
  name="$(basename "$dir")"
  [ -d "$dir/.git" ] || continue
  start=$(date +%s)
  json=$(cd "$dir" && REVIEWGATE_CONFIG="$WORK/config-$CFG.toml" \
    "$BIN" review --from HEAD~2 --to HEAD --dimensions security,logic \
    --timeout 600 --format json --no-metrics 2>/dev/null) || true
  end=$(date +%s)
  if [ -z "$json" ]; then
    printf '{"repo":"%s","config":"%s","error":"no output","secs":%s}\n' "$name" "$CFG" "$((end-start))" >> "$out"
    echo "FAIL $name"; continue
  fi
  printf '%s' "$json" | python3 -c "
import sys, json
d = json.load(sys.stdin); s = d.get('summary') or {}; u = d.get('usage') or {}
print(json.dumps({'repo': '$name', 'config': '$CFG',
  'decision': d.get('decision'), 'incomplete': d.get('incomplete'),
  'files_changed': d.get('files_changed'), 'kept': s.get('kept'),
  'excluded': len(d.get('excluded') or []),
  'input_tokens': u.get('input_tokens'), 'output_tokens': u.get('output_tokens'),
  'findings': [(f.get('path'), f.get('start_line'), f.get('severity'), round(f.get('confidence',0),2))
               for f in (d.get('findings') or []) if not f.get('filtered')],
  'secs': $((end-start))}))" >> "$out"
  echo "done $name ($((end-start))s)"
done
echo "写入 $out"
