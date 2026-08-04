#!/usr/bin/env bash
# 校验 .github/workflows/*.yml：能否被解析，以及触发器/job 结构是否完整。
#
# 为什么要有它：release.yml 曾因一行顶格的引号变成无效 YAML，GitHub 根本解析不了它，
# 于是**发布流程一个多月从未运行**——0.10.0 静默丢失，`v0` 停在旧提交上，所有用 `@v0`
# 的使用者收不到任何更新。而每次 push 只在 Actions 页面留下一条 0s 的失败记录，
# 没有触发任何人的注意。语法错误本身好修，真正的问题是它不会在 PR 阶段变红。
#
# 用法：scripts/check-workflows.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${1:-$ROOT/.github/workflows}"

[ -d "$DIR" ] || { echo "找不到 workflow 目录：$DIR" >&2; exit 2; }

python3 - "$DIR" <<'PY'
import sys, glob, os
try:
    import yaml
except ImportError:
    print("需要 PyYAML：pip install pyyaml", file=sys.stderr)
    sys.exit(2)

files = sorted(glob.glob(os.path.join(sys.argv[1], "*.yml")) +
               glob.glob(os.path.join(sys.argv[1], "*.yaml")))
if not files:
    print("没有找到任何 workflow 文件", file=sys.stderr)
    sys.exit(2)

bad = 0
for path in files:
    name = os.path.basename(path)
    try:
        doc = yaml.safe_load(open(path, encoding="utf-8"))
    except yaml.YAMLError as e:
        mark = getattr(e, "problem_mark", None)
        where = f"第 {mark.line + 1} 行第 {mark.column + 1} 列" if mark else "位置未知"
        print(f"✖ {name}: 无效 YAML（{where}）——GitHub 将无法解析，该 workflow 永远不会运行")
        print(f"  {getattr(e, 'problem', e)}")
        bad += 1
        continue

    if not isinstance(doc, dict):
        print(f"✖ {name}: 顶层不是映射")
        bad += 1
        continue

    # PyYAML 按 YAML 1.1 把裸 `on:` 解析成布尔 True，两种键都认。
    triggers = doc.get("on", doc.get(True))
    if not triggers:
        print(f"✖ {name}: 缺少 `on` 触发器，workflow 不会被任何事件触发")
        bad += 1

    jobs = doc.get("jobs")
    if not isinstance(jobs, dict) or not jobs:
        print(f"✖ {name}: 缺少 `jobs`")
        bad += 1
        continue

    for jid, job in jobs.items():
        if not isinstance(job, dict):
            print(f"✖ {name}: job `{jid}` 不是映射")
            bad += 1
            continue
        # 复用型 job（uses:）没有 runs-on/steps，属正常写法。
        if "uses" in job:
            continue
        if "runs-on" not in job:
            print(f"✖ {name}: job `{jid}` 缺少 runs-on")
            bad += 1
        steps = job.get("steps")
        if not isinstance(steps, list) or not steps:
            print(f"✖ {name}: job `{jid}` 缺少 steps")
            bad += 1

    print(f"✓ {name}: {len(jobs)} 个 job")

if bad:
    print(f"\n{bad} 处问题——修复后 workflow 才会被 GitHub 执行", file=sys.stderr)
    sys.exit(1)
print(f"\nOK：{len(files)} 个 workflow 文件均可解析")
PY
