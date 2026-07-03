#!/usr/bin/env python3
"""
用 AACR-Bench **官方评分器**（evaluator_runner，语义 LLM 匹配）评测 ReviewGate。

这才是能和 open-code-review 公开 F1 对标的正确姿势：
  - 参考集用官方 dataset/positive_samples.json（人工-LLM 协作标注的真缺陷）；
  - 匹配用官方 get_evaluator_ans_from_json（语义 LLM judge + 行号），不是自己搓的位置匹配；
  - 指标口径与官方一致：positive_match_rate(precision) / positive_recall_rate(recall)。

诚实边界（务必写进报告）：
  - **非同底座对照**：RG 与 LLM judge 都走本地配置的端点（默认 deepseek），OCR 用它自己的模型。
    比的是「RG 按此配置」vs「OCR 按其公开配置」，不是控制变量后的工具对工具。
  - judge 模型会影响语义匹配判定，已固定并在报告注明。

用法：
  AACR_REPO=/path/to/aacr-bench python3 scripts/eval-aacr-official.py [--limit N] [--lang C++] [--pr repo#num ...]
环境：
  AACR_REPO         官方 aacr-bench 仓库路径（含 evaluator_runner/ 与 dataset/positive_samples.json）
  LLM_MODEL_URL/LLM_MODEL/LLM_API_KEY   judge 端点；缺省从 reviewgate.toml 的默认 provider 读取
  REVIEWGATE_EVAL_TIMEOUT   单维度超时（秒），默认 300
"""
import argparse
import asyncio
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVAL_DIR = ROOT / "docs" / "evals"
WORK_DIR = Path(os.environ.get("TMPDIR", "/tmp")) / "reviewgate-aacr-official"
RG_BIN = ROOT / "target" / "release" / "reviewgate"
CONFIG = ROOT / "reviewgate.toml"
TIMEOUT = int(os.environ.get("REVIEWGATE_EVAL_TIMEOUT", "300"))


def load_judge_env_from_toml():
    """judge 端点缺省复用 reviewgate.toml 的默认 provider（不控制模型，见文件头）。"""
    if os.environ.get("LLM_MODEL_URL") and os.environ.get("LLM_API_KEY"):
        return
    text = CONFIG.read_text()
    prov = {}
    try:
        import tomllib  # py3.11+
        cfg = tomllib.loads(text)
        prov = cfg.get("providers", {}).get(cfg.get("provider", ""), {})
    except ModuleNotFoundError:
        # 极简手工解析：取默认 provider 的 base_url/model/api_key（首个 provider 块即可）。
        import re
        def grab(field):
            m = re.search(rf'^\s*{field}\s*=\s*"([^"]*)"', text, re.MULTILINE)
            return m.group(1) if m else ""
        prov = {"base_url": grab("base_url"), "model": grab("model"), "api_key": grab("api_key")}
    key = os.environ.get("REVIEWGATE_API_KEY") or prov.get("api_key", "")
    os.environ.setdefault("LLM_MODEL_URL", prov.get("base_url", ""))
    os.environ.setdefault("LLM_MODEL", prov.get("model", ""))
    os.environ.setdefault("LLM_API_KEY", key)


def ensure_repo(repo: str) -> Path:
    clone = WORK_DIR / repo.replace("/", "_")
    if (clone / ".git").exists():
        return clone
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    for _ in range(5):
        r = subprocess.run(["git", "clone", "--quiet", "--filter=blob:none",
                            f"https://github.com/{repo}.git", str(clone)])
        if r.returncode == 0:
            return clone
        subprocess.run(["rm", "-rf", str(clone)])
    raise RuntimeError(f"clone failed: {repo}")


def _commits_present(repo_dir: Path, *shas) -> bool:
    """确认 commit 对象已在本地（blobless clone 下按需 fetch 可能未落地）。"""
    for sha in shas:
        if subprocess.run(["git", "cat-file", "-e", f"{sha}^{{commit}}"],
                          cwd=repo_dir, capture_output=True).returncode != 0:
            return False
    return True


def fetch(repo_dir: Path, *shas):
    # 拉两个 commit 的完整树（--filter=tree:0 只延迟 blob；但 diff 需要 blob，
    # 故这里不加 filter，确保 diff 所需对象都在本地，避免运行 RG 时按需 fetch 撞网络抖动）。
    for _ in range(4):
        subprocess.run(["git", "fetch", "--quiet", "origin", *shas],
                       cwd=repo_dir, capture_output=True)
        if _commits_present(repo_dir, *shas):
            return
    raise RuntimeError(f"fetch failed (commits not present): {shas}")


def run_rg(repo_dir: Path, source: str, target: str) -> dict:
    env = os.environ.copy()
    env["REVIEWGATE_CONFIG"] = str(CONFIG)
    cmd = [str(RG_BIN), "review", "--from", source, "--to", target,
           "--format", "json", "--timeout", str(TIMEOUT)]
    last = ""
    # RG 退出码 2 = 工具自身出错；若是 git 类瞬时错（按需 fetch blob 抖动），重试一次。
    for attempt in range(2):
        proc = subprocess.run(cmd, cwd=repo_dir, capture_output=True, text=True,
                              env=env, timeout=TIMEOUT * 6)
        if proc.returncode in (0, 1):
            return json.loads(proc.stdout)
        last = proc.stderr[-400:]
        if proc.returncode == 2 and "git" in last and attempt == 0:
            fetch(repo_dir, source, target)  # 重新确保对象在本地
            continue
        break
    raise RuntimeError(f"rg exited non-0/1: {last}")


def rg_findings_to_generated(rg_result: dict) -> list[dict]:
    """RG finding → 官方 generated_comment 格式（path/side/from_line/to_line/note）。"""
    out = []
    for f in rg_result.get("findings", []):
        if f.get("filtered"):
            continue
        out.append({
            "path": f.get("path", ""),
            "side": "right",  # RG 只审新增/修改（新文件行号）
            "from_line": f.get("start_line", 0) or 0,
            "to_line": f.get("end_line", 0) or f.get("start_line", 0) or 0,
            "note": f.get("message", ""),
        })
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, default=3)
    ap.add_argument("--lang", default=None, help="只跑某语言（project_main_language 精确匹配）")
    ap.add_argument("--pr", action="append", default=[], help="只跑指定 repo#num（可多次）")
    ap.add_argument("--all", action="store_true", help="跑全部 196 个 positive_samples（可断点续跑）")
    ap.add_argument("--rescore", action="store_true", help="忽略已存在的 .eval.json，重跑 judge")
    ap.add_argument("--max-new", type=int, default=0, help="本次最多评测多少个新 PR（0=不限；分批用）")
    args = ap.parse_args()

    aacr = os.environ.get("AACR_REPO")
    if not aacr or not (Path(aacr) / "evaluator_runner").is_dir():
        sys.exit("请设置 AACR_REPO 指向官方 aacr-bench 仓库（含 evaluator_runner/）")
    sys.path.insert(0, aacr)
    load_judge_env_from_toml()
    if not os.environ.get("LLM_API_KEY"):
        sys.exit("judge 端点缺 LLM_API_KEY")

    from evaluator_runner import get_evaluator_ans_from_json, EvaluatorConfig  # noqa: E402

    samples = json.loads((Path(aacr) / "dataset" / "positive_samples.json").read_text())
    # 建索引：repo#pr -> entry
    def key_of(e):
        u = e["githubPrUrl"].rstrip("/")
        parts = u.split("/")
        return f"{parts[-4]}/{parts[-3]}#{parts[-1]}"
    by_key = {key_of(e): e for e in samples}

    if args.pr:
        picked = [by_key[k] for k in args.pr if k in by_key]
    elif args.all:
        pool = [e for e in samples if not args.lang or e.get("project_main_language") == args.lang]
        # 小改动优先：中断时已完成的 PR 更多，且早期出信号快。
        picked = sorted(pool, key=lambda e: e.get("change_line_count", 0))
    else:
        pool = [e for e in samples if not args.lang or e.get("project_main_language") == args.lang]
        picked = pool[: args.limit]

    if not RG_BIN.exists():
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)

    cfg = EvaluatorConfig()  # semantic=LLM, threshold=1（官方默认）
    rows = []
    print(f"judge: {os.environ.get('LLM_MODEL')} @ {os.environ.get('LLM_MODEL_URL')}")
    print(f"PRs: {len(picked)}\n")

    resdir = EVAL_DIR / "aacr-bench-results"
    resdir.mkdir(parents=True, exist_ok=True)
    new_done = 0
    for e in picked:
        if args.max_new and new_done >= args.max_new:
            print(f"\n[batch] 已评测 {new_done} 个新 PR，达到 --max-new，停止本批。")
            break
        url = e["githubPrUrl"].rstrip("/")
        parts = url.split("/")
        repo = f"{parts[-4]}/{parts[-3]}"
        key = f"{repo}#{parts[-1]}"
        slug = f"{repo.replace('/', '_')}__pr{parts[-1]}"
        good = e.get("comments", [])
        eval_cache = resdir / f"{slug}.eval.json"
        # 断点续跑：已有完整 eval 结果 → 跳过（RG + judge 都不重跑）。--rescore 强制重评。
        if eval_cache.exists() and not args.rescore and not os.environ.get("RG_NOCACHE"):
            try:
                res = json.loads(eval_cache.read_text())
                if "positive_match_nums" in res:
                    print(f"  ✓ cached {key} (match={res.get('positive_match_nums')}/{res.get('total_generated_nums')})")
                    continue
            except Exception:
                pass
        print(f"▶ {key} [{e.get('project_main_language')}] good={len(good)}")
        try:
            rd = ensure_repo(repo)
            fetch(rd, e["source_commit"], e["target_commit"])
            # 缓存 RG 原始输出：重评/诊断时零 RG 成本（只重跑 judge）。RG_NOCACHE=1 可强制重审。
            rg_cache = resdir / f"{slug}.rg.json"
            if rg_cache.exists() and not os.environ.get("RG_NOCACHE"):
                rg = json.loads(rg_cache.read_text())
            else:
                rg = run_rg(rd, e["source_commit"], e["target_commit"])
                rg_cache.write_text(json.dumps(rg, ensure_ascii=False, indent=2))
            gen = rg_findings_to_generated(rg)
            res = asyncio.run(get_evaluator_ans_from_json(
                github_pr_url=url, generated_comments=gen, good_comments=good, config=cfg,
                pr_metadata={"category": e.get("category"),
                             "project_main_language": e.get("project_main_language")},
            ))
            if "error" in res:
                raise RuntimeError(res["error"])
            # 存完整 evaluator 结果（含 match_details / llm_comparisons）供诊断 precision/recall。
            (resdir / f"{slug}.eval.json").write_text(json.dumps(res, ensure_ascii=False, indent=2))
            new_done += 1
            m = res.get("positive_match_nums", 0)
            tg = res.get("total_generated_nums", 0)
            pe = res.get("positive_expected_nums", 0)
            print(f"  gen={tg} good={pe} semantic_match={m} "
                  f"precision={res.get('positive_match_rate')} recall={res.get('positive_recall_rate')} "
                  f"decision={rg.get('decision')} incomplete={rg.get('incomplete')}")
            rows.append({"key": key, "lang": e.get("project_main_language"),
                         "gen": tg, "good": pe, "match": m,
                         "precision": res.get("positive_match_rate"),
                         "recall": res.get("positive_recall_rate"),
                         "decision": rg.get("decision"), "incomplete": rg.get("incomplete")})
        except Exception as ex:
            print(f"  ERROR: {ex}")
            rows.append({"key": key, "error": str(ex)})

    # 汇总从磁盘扫描全部 .eval.json（跨重启累积），并按 196 参考集统计覆盖度与分语言指标。
    from collections import defaultdict
    lang_of = {key_of(e): e.get("project_main_language", "?") for e in samples}
    all_keys = set(by_key.keys())
    tg = tgood = tm = 0
    done = 0
    by_lang = defaultdict(lambda: {"gen": 0, "good": 0, "match": 0, "prs": 0})
    detail = []
    for slug_file in resdir.glob("*.eval.json"):
        try:
            r = json.loads(slug_file.read_text())
        except Exception:
            continue
        key = f"{r.get('repo')}#{r.get('pr_number')}"
        if key not in all_keys or "positive_match_nums" not in r:
            continue
        done += 1
        g = r.get("total_generated_nums", 0); gd = r.get("positive_expected_nums", 0); mm = r.get("positive_match_nums", 0)
        tg += g; tgood += gd; tm += mm
        lang = lang_of.get(key, "?")
        b = by_lang[lang]; b["gen"] += g; b["good"] += gd; b["match"] += mm; b["prs"] += 1
        detail.append({"key": key, "lang": lang, "gen": g, "good": gd, "match": mm})

    def prf(m, g, gd):
        p = m / g if g else 0.0; rr = m / gd if gd else 0.0
        return p, rr, (2 * p * rr / (p + rr) if (p + rr) else 0.0)

    P, R, F1 = prf(tm, tg, tgood)
    print(f"\n==== 官方口径（语义匹配）micro 汇总 ====")
    print(f"覆盖 {done}/196 PR  generated={tg}  good={tgood}  semantic_match={tm}")
    print(f"Precision={P:.1%}  Recall={R:.1%}  F1={F1:.1%}")
    print("按语言：")
    for lang, b in sorted(by_lang.items()):
        p, rr, f = prf(b["match"], b["gen"], b["good"])
        print(f"  {lang:12s} PRs={b['prs']:2d}  P={p:.0%} R={rr:.0%} F1={f:.0%}")

    out = {"judge_model": os.environ.get("LLM_MODEL"),
           "note": "非同底座对照：RG 与 judge 均走本地端点；对标 OCR 需读其公开配置",
           "coverage": f"{done}/196",
           "micro": {"prs": done, "generated": tg, "good": tgood, "semantic_match": tm,
                     "precision": round(P, 4), "recall": round(R, 4), "f1": round(F1, 4)},
           "by_language": {lang: dict(v, **dict(zip(("precision", "recall", "f1"),
                            (round(x, 4) for x in prf(v["match"], v["gen"], v["good"])))))
                           for lang, v in sorted(by_lang.items())},
           "detail": sorted(detail, key=lambda d: d["key"])}
    (resdir / "official-summary.json").write_text(json.dumps(out, ensure_ascii=False, indent=2))
    print(f"\n✓ {resdir / 'official-summary.json'}  (rows this run: {len(rows)})")


if __name__ == "__main__":
    main()
