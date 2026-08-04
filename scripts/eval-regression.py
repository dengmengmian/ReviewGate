#!/usr/bin/env python3
"""
AACR-Bench 质量回归基线：把一次评测结果冻结成基线，之后每次改动与它对比。

为什么要有它：改动能不能提高质量，光看「某条发现命中了」说不清。改 dedup 的采样合并时，
能证明判定稳了（真实 PR 连跑 5 次一致），却证明不了它没有压低缺陷检出——因为没有基线。
没有仪表就调发动机，每次都只能赌。

**incomplete 必须排除在指标之外**。历史结果里 59% 的 PR 是 incomplete（超时/上下文溢出），
把它们当成「零发现」计进 recall 分母，会把 F1 从 0.155 压到 0.089——一次超时增加会伪装成
「改动降低了召回」，方向判断整个反过来。所以指标只用完整运行算，incomplete 率单列为健康度。

用法：
  python3 scripts/eval-regression.py snapshot            # 从当前结果冻结基线
  python3 scripts/eval-regression.py compare             # 当前结果 vs 基线
  python3 scripts/eval-regression.py compare --results DIR
"""
import argparse
import glob
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RESULTS = ROOT / "docs" / "evals" / "aacr-bench-results"
BASELINE = ROOT / "docs" / "evals" / "aacr-baseline.json"


def load(results_dir: Path) -> dict:
    """读一批评测结果，按 PR 汇总。incomplete 取自 rg.json（评分器不记录它）。"""
    prs = {}
    for path in sorted(glob.glob(str(results_dir / "*.eval.json"))):
        try:
            ev = json.load(open(path, encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            continue
        key = os.path.basename(path)[: -len(".eval.json")]
        try:
            rg = json.load(open(path.replace(".eval.json", ".rg.json"), encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            rg = {}
        prs[key] = {
            "expected": ev.get("positive_expected_nums") or 0,
            "generated": ev.get("total_generated_nums") or 0,
            "matched": ev.get("positive_match_nums") or 0,
            # 不完整的运行不代表「没问题」，指标里必须摘出去。
            "incomplete": bool(rg.get("incomplete")),
        }
    return prs


def metrics(prs: dict) -> dict:
    """只用完整运行算 P/R/F1；incomplete 率单列，它自己就是一个要盯的指标。"""
    ok = {k: v for k, v in prs.items() if not v["incomplete"]}
    exp = sum(v["expected"] for v in ok.values())
    gen = sum(v["generated"] for v in ok.values())
    hit = sum(v["matched"] for v in ok.values())
    p = hit / gen if gen else 0.0
    r = hit / exp if exp else 0.0
    return {
        "prs_total": len(prs),
        "prs_scored": len(ok),
        "incomplete": len(prs) - len(ok),
        "incomplete_rate": (len(prs) - len(ok)) / len(prs) if prs else 0.0,
        "gt_defects": exp,
        "reported": gen,
        "matched": hit,
        "precision": p,
        "recall": r,
        "f1": 2 * p * r / (p + r) if p + r else 0.0,
    }


def fmt(m: dict) -> str:
    return (
        f"  PR {m['prs_scored']}/{m['prs_total']} 计入指标"
        f"（{m['incomplete']} 个 incomplete，{m['incomplete_rate']*100:.0f}%）\n"
        f"  GT 缺陷 {m['gt_defects']} · 报告 {m['reported']} · 语义命中 {m['matched']}\n"
        f"  precision {m['precision']:.4f} · recall {m['recall']:.4f} · F1 {m['f1']:.4f}"
    )


def cmd_snapshot(args):
    prs = load(Path(args.results))
    if not prs:
        print(f"没有找到评测结果：{args.results}", file=sys.stderr)
        return 2
    m = metrics(prs)
    BASELINE.write_text(
        json.dumps({"metrics": m, "prs": prs}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"基线已写入 {BASELINE.relative_to(ROOT)}\n{fmt(m)}")
    # incomplete 越多，进指标的样本越少，基线越不可信。实测这些「失败」多半是评测
    # 超时设太紧（300s 时 59% 失败，实际只需 150~400s），不是真跑不完——先把
    # REVIEWGATE_EVAL_TIMEOUT 调大重跑，再拿它当基线，否则后续对比全在噪声上做文章。
    if m["incomplete_rate"] > 0.2:
        print(
            f"\n⚠ incomplete 率 {m['incomplete_rate']*100:.0f}%——只有 {m['prs_scored']} 个 PR 进了指标，"
            f"基线可信度低。\n"
            f"  建议先用更大的超时重跑（REVIEWGATE_EVAL_TIMEOUT，默认已提到 900）再冻结基线。"
        )
    return 0


def cmd_compare(args):
    if not BASELINE.exists():
        print(f"没有基线，先跑：python3 {Path(__file__).name} snapshot", file=sys.stderr)
        return 2
    base = json.loads(BASELINE.read_text(encoding="utf-8"))
    cur_prs = load(Path(args.results))
    if not cur_prs:
        print(f"没有找到评测结果：{args.results}", file=sys.stderr)
        return 2
    bm, cm = base["metrics"], metrics(cur_prs)

    print("基线:")
    print(fmt(bm))
    print("\n当前:")
    print(fmt(cm))

    print("\n差异:")
    rows = [
        ("precision", bm["precision"], cm["precision"], "高好"),
        ("recall", bm["recall"], cm["recall"], "高好"),
        ("F1", bm["f1"], cm["f1"], "高好"),
        ("incomplete 率", bm["incomplete_rate"], cm["incomplete_rate"], "低好"),
    ]
    worse = False
    for name, b, c, better in rows:
        d = c - b
        good = (d >= 0) if better == "高好" else (d <= 0)
        mark = "  " if abs(d) < 1e-9 else ("↑" if d > 0 else "↓")
        flag = "" if good or abs(d) < 1e-9 else "   ← 退步"
        if not good and abs(d) > 1e-9:
            worse = True
        print(f"  {name:14s} {b:.4f} → {c:.4f}  {mark}{d:+.4f}{flag}")

    # 逐 PR 差异：命中数变化最能定位是哪些 PR 变好/变坏。
    base_prs = base["prs"]
    changed = []
    for k, cv in cur_prs.items():
        bv = base_prs.get(k)
        if not bv:
            changed.append((k, "新增", 0, cv["matched"]))
        elif bv["matched"] != cv["matched"]:
            changed.append((k, "变化", bv["matched"], cv["matched"]))
    gone = [k for k in base_prs if k not in cur_prs]
    if changed or gone:
        print("\n逐 PR 命中变化:")
        for k, kind, b, c in sorted(changed, key=lambda x: x[3] - x[2]):
            print(f"  {'↑' if c > b else '↓'} {k}  {b} → {c}  ({kind})")
        for k in gone:
            print(f"  - {k}  基线里有、当前缺失")
    else:
        print("\n逐 PR 命中无变化")

    if worse:
        print("\n有指标退步——确认是改动导致还是评测波动后再合并。")
    return 1 if (worse and args.strict) else 0


def main():
    ap = argparse.ArgumentParser(description="AACR-Bench 质量回归基线")
    ap.add_argument("command", choices=["snapshot", "compare"])
    ap.add_argument("--results", default=str(RESULTS), help="评测结果目录")
    ap.add_argument("--strict", action="store_true", help="有退步时以非零码退出（供 CI 用）")
    args = ap.parse_args()
    return cmd_snapshot(args) if args.command == "snapshot" else cmd_compare(args)


if __name__ == "__main__":
    sys.exit(main())
