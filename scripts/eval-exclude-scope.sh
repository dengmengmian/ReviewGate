#!/usr/bin/env bash
# 评测「审查范围排除」对送审范围与成本的影响：baseline（无排除）vs new（默认内置排除）。
#
# 用法：scripts/eval-exclude-scope.sh <工作目录> [轮数]
#   例：scripts/eval-exclude-scope.sh /tmp/rg-eval 10
#
# 只用 --estimate-only，**不调 LLM**：结果确定、零 token 成本，可重复多轮验证稳定性。
# 判定是否受影响是另一件事，见 scripts/eval-decision-stability.sh。
#
# 依赖：git、python3、已构建的 reviewgate 二进制（BIN 环境变量可覆盖路径）。
# 必需：~/.reviewgate/config.toml 里有可用 provider（估算需要构造 client，但不发请求）。
set -euo pipefail

WORK="${1:?usage: eval-exclude-scope.sh <workdir> [rounds]}"
ROUNDS="${2:-10}"
BIN="${BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/reviewgate}"
[ -x "$BIN" ] || BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/reviewgate"
[ -x "$BIN" ] || { echo "找不到 reviewgate 二进制，先 cargo build 或设 BIN=" >&2; exit 2; }

mkdir -p "$WORK/repos" "$WORK/results"

# 评测样本：跑之前就定好，不按结果挑。
cat > "$WORK/repos.txt" <<'EOF'
rust-lang/cargo
BurntSushi/ripgrep
sharkdp/bat
tokio-rs/tokio
denoland/deno
cli/cli
junegunn/fzf
gohugoio/hugo
grpc/grpc-go
kubernetes/client-go
axios/axios
expressjs/express
vitejs/vite
prettier/prettier
facebook/react
sequelize/sequelize
psf/requests
pallets/flask
yt-dlp/yt-dlp
python-poetry/poetry
EOF

# 两份配置只差 builtin 开关；baseline 等价于「没有排除机制」的旧行为。
strip_exclude() { python3 -c "
import re,sys
s=open(sys.argv[1]).read()
print(re.sub(r'\n\[exclude\][\s\S]*?(?=\n\[|\Z)','\n',s).rstrip())
" "$HOME/.reviewgate/config.toml"; }
# 这两份配置是从 ~/.reviewgate/config.toml 复制来的，**可能带 api_key**：
# 先建成 0600 再写入，避免在 /tmp 这类共享目录里把密钥暴露给同机其他用户。
for f in "$WORK/config-baseline.toml" "$WORK/config-new.toml"; do
  : > "$f"; chmod 600 "$f"
done
{ strip_exclude; printf '\n[exclude]\npatterns = []\nbuiltin = false\n'; } > "$WORK/config-baseline.toml"
{ strip_exclude; printf '\n[exclude]\npatterns = []\nbuiltin = true\n'; }  > "$WORK/config-new.toml"

echo "== 克隆样本仓库（浅克隆）=="
while read -r slug; do
  [ -z "$slug" ] && continue
  name="${slug//\//__}"
  [ -d "$WORK/repos/$name/.git" ] && { echo "SKIP $slug"; continue; }
  git clone --quiet --filter=blob:none --depth 30 "https://github.com/$slug.git" \
    "$WORK/repos/$name" 2>/dev/null && echo "OK $slug" || echo "FAIL $slug"
done < "$WORK/repos.txt"

echo "== 跑 $ROUNDS 轮 =="
for round in $(seq 1 "$ROUNDS"); do
  out="$WORK/results/round-$round.jsonl"
  : > "$out"
  for dir in "$WORK"/repos/*/; do
    name="$(basename "$dir")"
    [ -d "$dir/.git" ] || continue
    for cfg in baseline new; do
      json=$(cd "$dir" && REVIEWGATE_CONFIG="$WORK/config-$cfg.toml" \
        "$BIN" review --from HEAD~10 --to HEAD --estimate-only --format json --no-metrics 2>/dev/null) || true
      if [ -z "$json" ]; then
        printf '{"repo":"%s","config":"%s","round":%s,"error":"no output"}\n' "$name" "$cfg" "$round" >> "$out"
        continue
      fi
      printf '%s' "$json" | python3 -c "
import sys, json
d = json.load(sys.stdin); ce = d.get('cost_estimate') or {}
print(json.dumps({'repo': '$name', 'config': '$cfg', 'round': $round,
  'files_changed': d.get('files_changed'), 'excluded': len(d.get('excluded') or []),
  'est_input_tokens': ce.get('est_input_tokens'), 'units': ce.get('units'),
  'scope': d.get('scope')}))" >> "$out"
    done
  done
  echo "round $round: $(wc -l < "$out") 行"
done

echo "== 汇总 =="
python3 - "$WORK" <<'PY'
import json, glob, sys, collections
work = sys.argv[1]
rows = [json.loads(l) for f in glob.glob(f"{work}/results/round-*.jsonl") for l in open(f)]
errs = [r for r in rows if "error" in r]
by = collections.defaultdict(set)
for r in rows:
    if "error" in r: continue
    by[(r["repo"], r["config"])].add((r["files_changed"], r["excluded"], r["est_input_tokens"]))
nondet = [k for k, v in by.items() if len(v) != 1]
print(f"样本 {len(rows)} 行，失败 {len(errs)}，非确定性 {len(nondet)}")
first = {}
for r in rows:
    if "error" in r: continue
    first.setdefault((r["repo"], r["config"]), r)
tb = sum(v["est_input_tokens"] for k, v in first.items() if k[1] == "baseline")
tn = sum(v["est_input_tokens"] for k, v in first.items() if k[1] == "new")
print(f"估算输入 token 合计 {tb} → {tn}  ({(tn-tb)/tb*100:+.1f}%)" if tb else "无数据")
PY
