#!/usr/bin/env bash
# 植入式对照：把同一个已知漏洞分别放在**普通源码路径**和**被排除路径**下，
# 直接测出排除规则的真实风险边界（而不是靠断言）。
#
# 用法：scripts/eval-planted-control.sh <工作目录>
#   依赖 eval-exclude-scope.sh 生成的 config-baseline.toml / config-new.toml。
#
# 预期：
#   normal-path   两组都 BLOCK —— 排除规则不得影响普通源码的召回
#   excluded-path baseline BLOCK，new PASS 且 excluded 非空 —— 已知代价，必须被如实披露
set -euo pipefail

WORK="${1:?usage: eval-planted-control.sh <workdir>}"
BIN="${BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/reviewgate}"
[ -x "$BIN" ] || BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/reviewgate"
out="$WORK/results/planted.jsonl"; mkdir -p "$WORK/results"; : > "$out"

POISON='# planted vulnerability for gate validation
def delete_user(conn, user_id: str) -> None:
    """Delete a user by id from an untrusted request parameter."""
    query = f"DELETE FROM users WHERE id = %s" % user_id
    conn.execute(query)

def get_user(conn, user_id: str):
    return conn.execute(f"SELECT * FROM users WHERE id = {user_id}").fetchone()
'

run_case() {
  local case_name="$1" rel_path="$2" cfg="$3"
  local dir="$WORK/planted/$case_name-$cfg"
  rm -rf "$dir"; mkdir -p "$dir"
  ( cd "$dir"
    git init -q .; git config user.email t@t.co; git config user.name t
    mkdir -p "$(dirname "$rel_path")" 2>/dev/null || true
    printf 'def noop():\n    return 1\n' > "$rel_path"
    git add -A >/dev/null; git commit -qm base
    printf '%s' "$POISON" > "$rel_path" )
  local json
  json=$(cd "$dir" && REVIEWGATE_CONFIG="$WORK/config-$cfg.toml" \
    "$BIN" review --dimensions security --timeout 600 --format json --no-metrics 2>/dev/null) || true
  [ -z "$json" ] && { echo "FAIL $case_name/$cfg"; return; }
  printf '%s' "$json" | python3 -c "
import sys, json
d = json.load(sys.stdin); s = d.get('summary') or {}
print(json.dumps({'case': '$case_name', 'path': '$rel_path', 'config': '$cfg',
  'decision': d.get('decision'), 'files_changed': d.get('files_changed'), 'kept': s.get('kept'),
  'excluded': [e['path'] for e in (d.get('excluded') or [])],
  'findings': [(f.get('path'), f.get('severity'), round(f.get('confidence',0),2))
               for f in (d.get('findings') or []) if not f.get('filtered')]}))" >> "$out"
  echo "done $case_name/$cfg"
}

for cfg in baseline new; do
  run_case normal-path   "src/handler.py"    "$cfg"
  run_case excluded-path "vendor/handler.py" "$cfg"
done
column -t -s$'\t' < /dev/null 2>/dev/null || true
python3 -c "
import json
for l in open('$out'):
    d = json.loads(l)
    print(f\"{d['case']:<15}{d['config']:<10}{d['decision']:<7}files={d['files_changed']} kept={d['kept']} excluded={d['excluded']}\")
"
