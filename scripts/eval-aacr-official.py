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
    for e in picked:
        url = e["githubPrUrl"].rstrip("/")
        parts = url.split("/")
        repo = f"{parts[-4]}/{parts[-3]}"
        key = f"{repo}#{parts[-1]}"
        slug = f"{repo.replace('/', '_')}__pr{parts[-1]}"
        good = e.get("comments", [])
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

    ok = [r for r in rows if "error" not in r]
    tg = sum(r["gen"] for r in ok)
    tgood = sum(r["good"] for r in ok)
    tm = sum(r["match"] for r in ok)
    P = tm / tg if tg else 0.0
    R = tm / tgood if tgood else 0.0
    F1 = 2 * P * R / (P + R) if (P + R) else 0.0
    print(f"\n==== 官方口径（语义匹配）micro 汇总 ====")
    print(f"PRs={len(ok)}  generated={tg}  good={tgood}  semantic_match={tm}")
    print(f"Precision={P:.1%}  Recall={R:.1%}  F1={F1:.1%}")

    out = {"judge_model": os.environ.get("LLM_MODEL"),
           "note": "非同底座对照：RG 与 judge 均走本地端点；对标 OCR 需读其公开配置",
           "micro": {"prs": len(ok), "generated": tg, "good": tgood, "semantic_match": tm,
                     "precision": round(P, 4), "recall": round(R, 4), "f1": round(F1, 4)},
           "rows": rows}
    (resdir / "official-summary.json").write_text(json.dumps(out, ensure_ascii=False, indent=2))
    print(f"\n✓ {resdir / 'official-summary.json'}")


if __name__ == "__main__":
    main()
