#!/usr/bin/env python3
"""
把 alibaba/aacr-bench 数据集转换为 ReviewGate 可消费的格式。

输入：/Users/mengmian/Develop/app/other/aacr-bench/dataset/{positive,negative}_samples.json
输出：
  - docs/evals/aacr-bench-all.tsv       全部 351 个 PR 的评测清单
  - docs/evals/aacr-bench-ground-truth.json  每个 PR 的 ground truth comments
  - docs/evals/aacr-bench-sample.tsv    一个小样本（默认 12 个 PR）

用法：
  python3 scripts/convert-aacr-bench.py
"""
import json
import random
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AACR_ROOT = Path("/Users/mengmian/Develop/app/other/aacr-bench")
OUT_DIR = ROOT / "docs" / "evals"


def parse_pr_url(url: str) -> tuple[str, int]:
    parts = url.replace("https://github.com/", "").split("/")
    repo = f"{parts[0]}/{parts[1]}"
    pr = int(parts[3])
    return repo, pr


def load_samples() -> tuple[list[dict], list[dict]]:
    with open(AACR_ROOT / "dataset" / "positive_samples.json") as f:
        pos = json.load(f)
    with open(AACR_ROOT / "dataset" / "negative_samples.json") as f:
        neg = json.load(f)
    return pos, neg


def build_records(pos: list[dict], neg: list[dict]) -> list[dict]:
    records = []
    for item in pos:
        repo, pr = parse_pr_url(item["githubPrUrl"])
        records.append({
            "repo": repo,
            "pr": pr,
            "source_commit": item["source_commit"],
            "target_commit": item["target_commit"],
            "language": item["project_main_language"],
            "category": item.get("category", ""),
            "label": "positive",
            "change_line_count": item.get("change_line_count", 0),
            "comments": item.get("comments", []),
        })
    for item in neg:
        repo, pr = parse_pr_url(item["githubPrUrl"])
        records.append({
            "repo": repo,
            "pr": pr,
            "source_commit": item["source_commit"],
            "target_commit": item["target_commit"],
            "language": item["project_main_language"],
            "category": item.get("category", ""),
            "label": "negative",
            "change_line_count": item.get("change_line_count", 0),
            "comments": item.get("comments", []),
        })
    return records


def select_sample(records: list[dict], n: int = 12, seed: int = 42) -> list[dict]:
    """
     stratified sampling：按语言 + 问题类别尽量覆盖，优先 change_line_count 适中的 positive PR。
    """
    random.seed(seed)
    # 只从 positive 里选，因为我们要测 recall；negative 可以单独测 precision。
    pos = [r for r in records if r["label"] == "positive"]
    # 按 (language, dominant_comment_category) 分组
    buckets = defaultdict(list)
    for r in pos:
        cats = [c["category"] for c in r["comments"] if c.get("note")]
        dom = max(set(cats), key=cats.count) if cats else "Unknown"
        buckets[(r["language"], dom)].append(r)

    sample = []
    # 每个桶先取一个，尽量多样化
    for key in sorted(buckets):
        candidates = [r for r in buckets[key] if 50 <= r["change_line_count"] <= 2000]
        if not candidates:
            candidates = buckets[key]
        chosen = random.choice(candidates)
        sample.append(chosen)
        if len(sample) >= n:
            break

    # 如果不够，补充不同语言
    used_keys = {f"{r['repo']}#{r['pr']}" for r in sample}
    remaining = [r for r in pos if f"{r['repo']}#{r['pr']}" not in used_keys
                 and 50 <= r["change_line_count"] <= 2000]
    random.shuffle(remaining)
    sample.extend(remaining[:n - len(sample)])
    return sample[:n]


def write_tsv(records: list[dict], path: Path):
    with open(path, "w") as f:
        f.write("# AACR-Bench -> ReviewGate 评测清单\n")
        f.write("# 数据来源：阿里巴巴开源数据集 alibaba/aacr-bench\n")
        f.write("# 原始描述：200 real Pull Requests from 50 active open-source projects, 10 languages,\n")
        f.write("#          human-LLM collaborative annotation, 1,505 review comments as ground truth.\n")
        f.write("# 列（Tab 分隔）：repo\tpr\tsource_commit\ttarget_commit\tlabel\tlanguage\tcategory\tchange_line_count\tnote\n")
        f.write("# source_commit = PR base, target_commit = PR head\n")
        for r in records:
            note = f"{r['language']} | {r['category']} | {len(r['comments'])} comments"
            f.write(f"{r['repo']}\t{r['pr']}\t{r['source_commit']}\t{r['target_commit']}\t"
                    f"{r['label']}\t{r['language']}\t{r['category']}\t"
                    f"{r['change_line_count']}\t{note}\n")


def write_ground_truth(records: list[dict], path: Path):
    gt = {
        f"{r['repo']}#{r['pr']}": {
            "repo": r["repo"],
            "pr": r["pr"],
            "source_commit": r["source_commit"],
            "target_commit": r["target_commit"],
            "language": r["language"],
            "category": r["category"],
            "label": r["label"],
            "comments": r["comments"],
        }
        for r in records
    }
    with open(path, "w") as f:
        json.dump(gt, f, ensure_ascii=False, indent=2)


def main():
    pos, neg = load_samples()
    records = build_records(pos, neg)
    print(f"Loaded {len(records)} PRs: {len(pos)} positive, {len(neg)} negative")

    OUT_DIR.mkdir(parents=True, exist_ok=True)

    # 全部数据
    write_tsv(records, OUT_DIR / "aacr-bench-all.tsv")
    write_ground_truth(records, OUT_DIR / "aacr-bench-ground-truth.json")
    print(f"Wrote aacr-bench-all.tsv ({len(records)} rows)")
    print(f"Wrote aacr-bench-ground-truth.json")

    # 小样本
    sample = select_sample(records, n=12)
    write_tsv(sample, OUT_DIR / "aacr-bench-sample.tsv")
    print(f"Wrote aacr-bench-sample.tsv ({len(sample)} rows)")
    for r in sample:
        print(f"  - {r['repo']}#{r['pr']} [{r['language']}] {len(r['comments'])} comments "
              f"({r['change_line_count']} lines)")


if __name__ == "__main__":
    main()
