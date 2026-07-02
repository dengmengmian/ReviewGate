#!/usr/bin/env python3
"""
【非官方·仅粗筛】用 ReviewGate 跑 AACR-Bench，只按 path + ±行号做**位置匹配**。

⚠ 重要：这个脚本算的是「位置命中」，是 AACR-Bench 官方那个**更宽松**的 line-level 指标，
   不是 OCR F1 对标的**语义匹配**指标——只要 RG 在 GT 附近评论就算命中，不管说的是不是同一件事，
   会**系统性虚高** precision/recall。**任何对外/对标 OCR 的数字都不得用本脚本。**

要正确对标，用 `scripts/eval-aacr-official.py`（调 AACR-Bench 官方 evaluator_runner，
LLM 语义匹配 + 官方 positive_samples.json 参考集）。本脚本仅用于开发期 5 秒粗筛。

用法：
  python3 scripts/eval-aacr-bench.py [sample.tsv]
"""
import asyncio
import json
import os
import re
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVAL_DIR = ROOT / "docs" / "evals"
RESULTS_DIR = EVAL_DIR / "aacr-bench-results"
WORK_DIR = Path(os.environ.get("TMPDIR", "/tmp")) / "reviewgate-aacr-bench"
RG_BIN = ROOT / "target" / "release" / "reviewgate"
CONFIG = ROOT / "reviewgate.toml"
TIMEOUT = int(os.environ.get("REVIEWGATE_EVAL_TIMEOUT", "300"))
LINE_THRESHOLD = int(os.environ.get("AACR_LINE_THRESHOLD", "3"))


def parse_tsv(path: Path) -> list[dict]:
    rows = []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 8:
                continue
            rows.append({
                "repo": parts[0].strip(),
                "pr": parts[1].strip(),
                "source": parts[2].strip(),
                "target": parts[3].strip(),
                "label": parts[4].strip(),
                "language": parts[5].strip(),
                "category": parts[6].strip(),
                "change_line_count": int(parts[7].strip()),
            })
    return rows


def load_ground_truth(path: Path) -> dict:
    with open(path) as f:
        return json.load(f)


def repo_slug(repo: str) -> str:
    return repo.replace("/", "_")


def ensure_repo(repo: str) -> Path:
    slug = repo_slug(repo)
    clone = WORK_DIR / slug
    if (clone / ".git").exists():
        return clone
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    url = f"https://github.com/{repo}.git"
    print(f"  cloning {repo} ...")
    subprocess.run(
        ["git", "clone", "--quiet", "--filter=blob:none", url, str(clone)],
        check=True,
    )
    return clone


def fetch_commits(repo_dir: Path, source: str, target: str):
    for _ in range(3):
        r = subprocess.run(
            ["git", "fetch", "--quiet", "origin", source, target],
            cwd=repo_dir,
            capture_output=True,
        )
        if r.returncode == 0:
            return
    raise RuntimeError(f"fetch failed for {source} {target}")


def run_reviewgate(repo_dir: Path, source: str, target: str) -> dict:
    env = os.environ.copy()
    env["REVIEWGATE_CONFIG"] = str(CONFIG)
    cmd = [
        str(RG_BIN),
        "review",
        "--from", source,
        "--to", target,
        "--format", "json",
        "--timeout", str(TIMEOUT),
    ]
    proc = subprocess.run(
        cmd,
        cwd=repo_dir,
        capture_output=True,
        text=True,
        env=env,
        timeout=TIMEOUT * 5,
    )
    # 进度信息在 stderr，JSON 在 stdout
    if proc.returncode not in (0, 1):
        raise RuntimeError(f"reviewgate exited {proc.returncode}: {proc.stderr[:500]}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"invalid JSON: {e}\nstdout[:1000]: {proc.stdout[:1000]}")


def path_match(gt_path: str, rg_path: str) -> bool:
    """ground truth path 和 ReviewGate finding path 匹配。"""
    if not gt_path or not rg_path:
        return False
    gt = gt_path.replace("\\", "/")
    rg = rg_path.replace("\\", "/")
    if gt == rg:
        return True
    if rg.endswith(gt) or gt.endswith(rg):
        return True
    # basename 相等也算
    if Path(gt).name == Path(rg).name:
        return True
    return False


def line_overlap(a_from: int, a_to: int, b_from: int, b_to: int, threshold: int = 0) -> bool:
    """两段行号是否重叠或在 threshold 范围内相邻。"""
    a_start, a_end = min(a_from, a_to), max(a_from, a_to)
    b_start, b_end = min(b_from, b_to), max(b_from, b_to)
    if a_start == 0 or b_start == 0:
        # 行号为 0 时只依赖 path
        return True
    return a_start <= b_end + threshold and b_start <= a_end + threshold


def match_findings(gt_comments: list[dict], findings: list[dict], threshold: int = LINE_THRESHOLD) -> tuple[int, int, list[dict]]:
    """
    返回 (命中 gt 数, 命中 finding 数, 明细)。
    一条 finding 可匹配多条 gt，但每条 gt 只计一次命中。
    """
    matched_gt_ids = set()
    matched_finding_indices = set()
    details = []

    for fi, f in enumerate(findings):
        if f.get("filtered"):
            continue
        f_hits = []
        for gi, c in enumerate(gt_comments):
            if gi in matched_gt_ids:
                continue
            if not path_match(c.get("path", ""), f.get("path", "")):
                continue
            if line_overlap(
                c.get("from_line", 0),
                c.get("to_line", 0),
                f.get("start_line", 0),
                f.get("end_line", 0),
                threshold,
            ):
                matched_gt_ids.add(gi)
                matched_finding_indices.add(fi)
                f_hits.append({
                    "gt_index": gi,
                    "gt_path": c.get("path"),
                    "gt_from": c.get("from_line"),
                    "gt_to": c.get("to_line"),
                    "gt_note": c.get("note")[:200],
                    "gt_category": c.get("category"),
                })
        if f_hits:
            details.append({
                "finding_index": fi,
                "rg_path": f.get("path"),
                "rg_start": f.get("start_line"),
                "rg_end": f.get("end_line"),
                "rg_dimension": f.get("dimension"),
                "rg_severity": f.get("severity"),
                "rg_confidence": f.get("confidence"),
                "rg_message": f.get("message")[:200],
                "hits": f_hits,
            })

    return len(matched_gt_ids), len(matched_finding_indices), details


def evaluate_pr(row: dict, gt: dict) -> dict:
    repo = row["repo"]
    pr = row["pr"]
    source = row["source"]
    target = row["target"]
    key = f"{repo}#{pr}"

    print(f"\n▶ {key} [{row['language']}] {row['change_line_count']} lines")
    repo_dir = ensure_repo(repo)
    fetch_commits(repo_dir, source, target)

    out_file = RESULTS_DIR / f"{repo_slug(repo)}__pr{pr}.json"
    if out_file.exists():
        print(f"  reusing existing result: {out_file}")
        with open(out_file) as f:
            rg_result = json.load(f)
    else:
        rg_result = run_reviewgate(repo_dir, source, target)
        out_file.parent.mkdir(parents=True, exist_ok=True)
        with open(out_file, "w") as f:
            json.dump(rg_result, f, ensure_ascii=False, indent=2)

    kept = [f for f in rg_result.get("findings", []) if not f.get("filtered")]
    gt_comments = gt.get(key, {}).get("comments", [])
    hit_gt, hit_rg, details = match_findings(gt_comments, rg_result.get("findings", []))

    recall = hit_gt / len(gt_comments) if gt_comments else 0.0
    precision = hit_rg / len(kept) if kept else 0.0

    print(f"  decision={rg_result.get('decision')} incomplete={rg_result.get('incomplete')} "
          f"kept={len(kept)} gt={len(gt_comments)} hit_gt={hit_gt} hit_rg={hit_rg} "
          f"recall={recall:.2%} precision={precision:.2%}")

    return {
        "repo": repo,
        "pr": pr,
        "language": row["language"],
        "category": row["category"],
        "label": row["label"],
        "decision": rg_result.get("decision"),
        "incomplete": rg_result.get("incomplete"),
        "kept_findings": len(kept),
        "gt_comments": len(gt_comments),
        "hit_gt": hit_gt,
        "hit_rg": hit_rg,
        "recall": round(recall, 4),
        "precision": round(precision, 4),
        "details": details,
        "raw_file": str(out_file.relative_to(ROOT)),
    }


def summarize(rows: list[dict]) -> dict:
    total_gt = sum(r["gt_comments"] for r in rows)
    total_hit_gt = sum(r["hit_gt"] for r in rows)
    total_kept = sum(r["kept_findings"] for r in rows)
    total_hit_rg = sum(r["hit_rg"] for r in rows)

    overall_recall = total_hit_gt / total_gt if total_gt else 0.0
    overall_precision = total_hit_rg / total_kept if total_kept else 0.0
    f1 = 2 * overall_precision * overall_recall / (overall_precision + overall_recall) if (overall_precision + overall_recall) else 0.0

    by_lang = defaultdict(lambda: {"gt": 0, "hit_gt": 0, "kept": 0, "hit_rg": 0, "count": 0})
    for r in rows:
        b = by_lang[r["language"]]
        b["gt"] += r["gt_comments"]
        b["hit_gt"] += r["hit_gt"]
        b["kept"] += r["kept_findings"]
        b["hit_rg"] += r["hit_rg"]
        b["count"] += 1

    return {
        "overall": {
            "prs": len(rows),
            "gt_comments": total_gt,
            "kept_findings": total_kept,
            "hit_gt": total_hit_gt,
            "hit_rg": total_hit_rg,
            "recall": round(overall_recall, 4),
            "precision": round(overall_precision, 4),
            "f1": round(f1, 4),
        },
        "by_language": {
            lang: {
                "count": v["count"],
                "recall": round(v["hit_gt"] / v["gt"], 4) if v["gt"] else 0,
                "precision": round(v["hit_rg"] / v["kept"], 4) if v["kept"] else 0,
            }
            for lang, v in sorted(by_lang.items())
        },
        "prs": rows,
    }


def write_markdown(summary: dict, out: Path):
    o = summary["overall"]
    lines = [
        f"# AACR-Bench × ReviewGate 评测汇总",
        "",
        f"- 评测时间: {time.strftime('%Y-%m-%d %H:%M')}",
        f"- 模型/端点: reviewgate.toml 配置",
        f"- 行号匹配阈值: ±{LINE_THRESHOLD} 行",
        "",
        "## 总体指标",
        "",
        "| 指标 | 值 |",
        "|---|---|",
        f"| PR 数 | {o['prs']} |",
        f"| Ground truth comments | {o['gt_comments']} |",
        f"| ReviewGate kept findings | {o['kept_findings']} |",
        f"| 命中 GT | {o['hit_gt']} |",
        f"| 命中 Finding | {o['hit_rg']} |",
        f"| **Recall** | **{o['recall']:.2%}** |",
        f"| **Precision** | **{o['precision']:.2%}** |",
        f"| **F1** | **{o['f1']:.2%}** |",
        "",
        "## 按语言",
        "",
        "| 语言 | PR 数 | Recall | Precision |",
        "|---|---|---|---|",
    ]
    for lang, v in summary["by_language"].items():
        lines.append(f"| {lang} | {v['count']} | {v['recall']:.2%} | {v['precision']:.2%} |")

    lines.extend([
        "",
        "## 逐 PR 明细",
        "",
        "| repo | pr | lang | decision | incomplete | kept | gt | hit_gt | hit_rg | recall | precision |",
        "|---|---|---|---|---|---|---|---|---|---|---|",
    ])
    for r in summary["prs"]:
        lines.append(
            f"| {r['repo']} | {r['pr']} | {r['language']} | {r['decision']} | {r['incomplete']} | "
            f"{r['kept_findings']} | {r['gt_comments']} | {r['hit_gt']} | {r['hit_rg']} | "
            f"{r['recall']:.2%} | {r['precision']:.2%} |"
        )

    out.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    tsv_path = Path(sys.argv[1]) if len(sys.argv) > 1 else EVAL_DIR / "aacr-bench-sample.tsv"
    if not RG_BIN.exists():
        print("building reviewgate (release)...")
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)

    rows = parse_tsv(tsv_path)
    gt = load_ground_truth(EVAL_DIR / "aacr-bench-ground-truth.json")
    print(f"dataset: {tsv_path}")
    print(f"PRs to evaluate: {len(rows)}")

    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    evaluated = []
    for row in rows:
        try:
            evaluated.append(evaluate_pr(row, gt))
        except Exception as e:
            print(f"  ERROR: {e}")
            evaluated.append({
                "repo": row["repo"],
                "pr": row["pr"],
                "language": row["language"],
                "category": row["category"],
                "label": row["label"],
                "error": str(e),
            })

    summary = summarize([r for r in evaluated if "error" not in r])
    summary["errors"] = [r for r in evaluated if "error" in r]

    json_out = RESULTS_DIR / "summary.json"
    with open(json_out, "w") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)

    md_out = EVAL_DIR / f"{time.strftime('%Y-%m-%d')}__aacr-bench-summary.md"
    write_markdown(summary, md_out)

    print(f"\n✓ summary json: {json_out}")
    print(f"✓ summary markdown: {md_out}")
    print(f"\nOverall recall={summary['overall']['recall']:.2%} "
          f"precision={summary['overall']['precision']:.2%} "
          f"f1={summary['overall']['f1']:.2%}")


if __name__ == "__main__":
    main()
