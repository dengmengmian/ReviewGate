#!/usr/bin/env python3
"""用**维护者自己打的标签**做 ground truth，量分诊的真实准确率与代码验证的判别力。

为什么需要它：自建语料只能防回归，测不出真实水位——规则是对着语料调的。
维护者在真实仓库上打的 `bug` / `enhancement` / `docs` 标签是独立的、没被我们污染过的
标注，拿它对齐才知道分类到底准不准、`--verify` 到底值不值。

用法（先跑过 scripts/eval-issue-triage.sh，库里要有判定记录）：
    python3 scripts/eval-issue-groundtruth.py .eval-issue/cli_cli/issues.db

输出三块：
1. 混淆矩阵 + 每类的召回/精确
2. 代码验证的判别力：真缺陷 vs 非缺陷 上的 LIKELY_BUG 率差距。**差距小 = 验证没在干活**
3. 最终裁决分布：看整条链路能不能把非缺陷挡下来

**读这些数字前必须知道的一件事：维护者标签是不完整的 ground truth。**
维护者常常只打最主要的那个标签——一条「README 缺少构建说明」很可能只标了
`enhancement`。所以 `documentation` 这类小众标签的**精确度会被系统性低估**：
实测中被判成 documentation 又"没有 docs 标签"的 62 条里，27 条其实判对了、
只是标签没打。

因此：
- **召回**（有标签的判没判出来）是可信的。
- **精确**（判出来的对不对）在小众类型上偏悲观，要人工抽查再下结论。
- 别拿精确度的下降直接判定一次改动失败——先看误判样本里有多少是标签缺失。
"""
import collections
import json
import sqlite3
import sys

# 维护者标签 → ReviewGate 类型。只用语义明确的三个，其余标签（priority-*、
# help wanted…）不表达类型，不参与对齐。
LABEL_MAP = {
    "bug": "bug",
    "defect": "bug",
    "enhancement": "feature_request",
    "feature": "feature_request",
    "feature request": "feature_request",
    "docs": "documentation",
    "documentation": "documentation",
}


def load(db):
    con = sqlite3.connect(db)
    labels = {
        n: {s.lower() for s in json.loads(l or "[]")}
        for n, l in con.execute("select issue_number, labels_json from issues")
    }
    dec = {}
    for (dj,) in con.execute("select decision_json from issue_reviews"):
        try:
            d = json.loads(dj)
        except json.JSONDecodeError:
            continue
        dec[d["issue_number"]] = d
    return labels, dec


def truth(ls):
    """标签唯一映射到一个类型时才算 ground truth，冲突的丢弃。"""
    hit = {LABEL_MAP[k] for k in ls if k in LABEL_MAP}
    return hit.pop() if len(hit) == 1 else None


def main(db):
    labels, dec = load(db)
    rows = [
        (n, t, dec[n])
        for n, ls in labels.items()
        if (t := truth(ls)) and n in dec
    ]
    if not rows:
        sys.exit("没有可对齐的样本：库里要既有标签又有分诊记录")

    print(f"可对齐样本: {len(rows)} 条（维护者标签唯一且已分诊）\n")

    conf = collections.Counter((t, d.get("primary_type")) for _, t, d in rows)
    gts = collections.Counter(t for _, t, _ in rows)
    preds = sorted({d.get("primary_type") for _, _, d in rows})
    print("混淆矩阵（行=维护者标签，列=ReviewGate 判定）")
    print(f"{'':<16}" + "".join(f"{str(p)[:13]:>15}" for p in preds))
    for gt in sorted(gts):
        print(f"{gt:<16}" + "".join(f"{conf[(gt, p)]:>15}" for p in preds))
    print()
    for gt in sorted(gts):
        tp = conf[(gt, gt)]
        got = sum(v for (_, p), v in conf.items() if p == gt)
        print(
            f"{gt:<16} 召回 {100 * tp / gts[gt]:5.1f}%  "
            f"精确 {100 * tp / got if got else 0:5.1f}%  "
            f"(标签 {gts[gt]} 条, 判出 {got} 条)"
        )

    print(
        "  ↑ 精确度在小众类型（documentation 等）上会被系统性低估：\n"
        "    维护者常只打主标签，判对了也可能算成误判。下结论前先人工抽查误判样本。"
    )

    print("\n代码验证（--verify）的判别力")
    for name, gt in [("真缺陷", "bug"), ("非缺陷", "feature_request")]:
        ran = [d for _, t, d in rows if t == gt and d.get("verification_ran")]
        allr = [d for _, t, d in rows if t == gt]
        if not allr:
            continue
        lb = sum(1 for d in ran if d.get("technical_verdict") == "LIKELY_BUG")
        rate = f"{100 * lb / len(ran):5.1f}%" if ran else "   n/a"
        print(
            f"  {name}({gt}): 共 {len(allr):>4} 条, 跑了验证 {len(ran):>4} 条"
            f"({100 * len(ran) / len(allr):5.1f}%), 其中判 LIKELY_BUG {rate}"
        )
    print("  ↑ 两行的 LIKELY_BUG 率差距越小，说明翻代码这一步越没在提供信息。")

    print("\n最终裁决 vs 维护者标签")
    for name, gt in [("真缺陷", "bug"), ("非缺陷", "feature_request")]:
        c = collections.Counter(d.get("verdict") for _, t, d in rows if t == gt)
        tot = sum(c.values()) or 1
        print(
            f"  {name}({tot} 条): "
            + " · ".join(f"{k} {100 * v / tot:.0f}%" for k, v in c.most_common(4))
        )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
