//! Issue 分类的**语料级回归**：一个固定的真实 Issue 集合 + 期望类型，跑出一个准确率。
//!
//! 为什么要有它：分类规则历来是单点修的——`rce` 命中 "pe*rce*ntage"、`docs` 链接被当成
//! 文档诉求、中文故障词漏覆盖……每次都只用一个 case 钉住一条规则。单点用例能防同一处
//! 再错，但挡不住「改一条规则修好 A、悄悄弄坏 B」。这里把攒下来的真实 case 加上一批
//! 常规 Issue 固化成集合，改规则时看的是**整体准确率**，不是某一条。
//!
//! 语料在 `tests/fixtures/issue_classify.jsonl`，一行一条：
//! `{id, source, expect, title, body, note?, known_gap?}`。
//!
//! - `expect` 是**人工标注**，不迁就分类器当前输出。
//! - `known_gap: true` = 今天确实分不对、且暂不打算改的，单独列出不算失败，
//!   但仍计入准确率分母——不许用它把数字洗白。
//!
//! 加新 case 就直接往 jsonl 里加；不要为了让测试变绿去改 `expect`。

use reviewgate_core::issue::classify::classify_heuristic;
use reviewgate_core::issue::safety::score_safety;

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    source: String,
    expect: String,
    title: String,
    body: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    known_gap: bool,
}

fn load() -> Vec<Case> {
    let raw = include_str!("fixtures/issue_classify.jsonl");
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| {
            serde_json::from_str::<Case>(l).unwrap_or_else(|e| panic!("bad case line: {e}\n{l}"))
        })
        .collect()
}

#[test]
fn corpus_is_well_formed() {
    let cases = load();
    assert!(
        cases.len() >= 40,
        "语料太小说明不了问题，只有 {}",
        cases.len()
    );
    let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "case id 必须唯一");
    for c in &cases {
        assert!(!c.source.trim().is_empty(), "{} 缺来源标注", c.id);
        assert!(!c.title.trim().is_empty(), "{} 缺标题", c.id);
    }
}

#[test]
fn classification_accuracy_does_not_regress() {
    let cases = load();
    let mut failures: Vec<String> = Vec::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut fixed_gaps: Vec<String> = Vec::new();
    let mut correct = 0usize;

    for c in &cases {
        let got = classify_heuristic(&c.title, &c.body, &score_safety(&c.title, &c.body));
        let ok = got.primary_type.as_str() == c.expect;
        if ok {
            correct += 1;
        }
        // 带上 reasons：回归时最想知道的是"哪条规则把它拽过去的"。
        let line = format!(
            "  {:<32} want={:<16} got={:<16} conf={:.2} reasons={:?}  [{}]{}",
            c.id,
            c.expect,
            got.primary_type.as_str(),
            got.confidence,
            got.reasons,
            c.source,
            c.note
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default()
        );
        match (ok, c.known_gap) {
            (false, false) => failures.push(line),
            (false, true) => gaps.push(line),
            (true, true) => fixed_gaps.push(line),
            (true, false) => {}
        }
    }

    let total = cases.len();
    eprintln!(
        "issue classify corpus: {correct}/{total} = {:.1}%  (failures={}, known gaps={})",
        correct as f32 / total as f32 * 100.0,
        failures.len(),
        gaps.len()
    );
    if !gaps.is_empty() {
        eprintln!("known gaps (recorded, not failing):\n{}", gaps.join("\n"));
    }
    if !fixed_gaps.is_empty() {
        eprintln!(
            "these known gaps now pass — drop their `known_gap` flag:\n{}",
            fixed_gaps.join("\n")
        );
    }

    assert!(
        failures.is_empty(),
        "{} case(s) regressed:\n{}",
        failures.len(),
        failures.join("\n")
    );

    // 反洗白。这里**刻意不用"准确率下限"**：语料是会长的，每加一条真实硬样本
    // 都会拉低比率，即使分类器在变好——那样的门只会逼人少加样本，正好和语料的
    // 目的相反。改成盯住 known_gap 本身：
    //
    // 1) 每条缺口必须写清楚真实证据。挂个 flag 就想蒙混过去的，这里红。
    // 2) 缺口占比有上限。超了说明在拿"登记"当挡箭牌，而不是在改分类器。
    let gapped: Vec<&Case> = cases.iter().filter(|c| c.known_gap).collect();
    for c in &gapped {
        let note = c.note.as_deref().unwrap_or("");
        assert!(
            note.chars().count() >= 20,
            "{} 标了 known_gap 却没写清楚为什么不修（note 太短或缺失）",
            c.id
        );
    }
    const MAX_GAP_RATIO: f32 = 0.15;
    let gap_ratio = gapped.len() as f32 / total as f32;
    assert!(
        gap_ratio <= MAX_GAP_RATIO,
        "已知缺口占 {:.0}%（{}/{}），超过 {:.0}% 上限——该改分类器了，不是继续登记",
        gap_ratio * 100.0,
        gapped.len(),
        total,
        MAX_GAP_RATIO * 100.0
    );
}
