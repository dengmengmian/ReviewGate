//! Issue 查重的**语料级回归**：固定场景集合，跑出召回与误报两个数。
//!
//! 与分类语料同一个思路，但查重要量的是两件相反的事：
//! - **召回**：换一套说法讲同一件事，还认不认得出来。
//! - **误报**：同模块的两个不同缺陷、同症状不同根因、同话题的需求 vs 缺陷，
//!   会不会被硬凑成重复。误判重复的代价比漏判大得多——它会让一条真问题被当成
//!   旧单子关掉，所以误报单独计数、单独看。
//!
//! 语料在 `tests/fixtures/issue_duplicate.jsonl`，一行一个场景：
//! `{id, source, corpus:[{number,title,body}], query:{number,title,body}, expect, note?, known_gap?}`
//! `expect` 是期望被判为重复的那条 Issue 号；`null` = 不该判重。
//!
//! `known_gap: true` = 已知抓不到、且暂不打算改（比如跨语言）。不算失败，但计入分母。

use reviewgate_core::issue::duplicate::find_duplicates;
use reviewgate_core::issue::model::{DuplicateStatus, RawIssue, RawLabel, RawUser};
use reviewgate_core::issue::{ingest_raw, normalize_issue, IssueStore, LocalEmbedder};

#[derive(serde::Deserialize)]
struct Item {
    number: u64,
    title: String,
    body: String,
}

#[derive(serde::Deserialize)]
struct Scenario {
    id: String,
    source: String,
    corpus: Vec<Item>,
    query: Item,
    /// 期望判定为重复的 Issue 号；None = 不该判重。
    expect: Option<u64>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    known_gap: bool,
}

fn load() -> Vec<Scenario> {
    include_str!("fixtures/issue_duplicate.jsonl")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| {
            serde_json::from_str::<Scenario>(l).unwrap_or_else(|e| panic!("bad scenario: {e}\n{l}"))
        })
        .collect()
}

fn raw(item: &Item) -> RawIssue {
    RawIssue {
        number: item.number,
        title: item.title.clone(),
        body: Some(item.body.clone()),
        state: "open".into(),
        labels: vec![RawLabel { name: "bug".into() }],
        user: Some(RawUser {
            login: "alice".into(),
            user_type: Some("User".into()),
        }),
        created_at: "2024-01-01T00:00:00Z".into(),
        updated_at: "2024-01-02T00:00:00Z".into(),
        closed_at: None,
        pull_request: None,
    }
}

struct Outcome {
    /// 摆到评论里给人看的疑似重复（含 Related）。这是查重的**主要产品行为**：
    /// 关单默认关闭，绝大多数场景下它只负责"把线索指出来"。
    surfaced: Option<u64>,
    /// 会真正驱动动作的判重（Exact / Probable，见 pipeline 的 `is_dup`）。
    acted: Option<u64>,
    status: DuplicateStatus,
}

fn run(s: &Scenario) -> Outcome {
    let store = IssueStore::open_in_memory("acme/app").expect("store");
    let emb = LocalEmbedder;
    for item in &s.corpus {
        ingest_raw(&store, &raw(item), &[], Some(&emb)).expect("ingest");
    }
    let n = normalize_issue(&s.query.title, &s.query.body);
    let r = find_duplicates(&store, s.query.number, &n, &emb, true, 20, 0.35);

    let surfaced = (r.status != DuplicateStatus::NotDuplicate)
        .then_some(r.duplicate_of)
        .flatten();
    let acted = matches!(
        r.status,
        DuplicateStatus::ExactDuplicate | DuplicateStatus::ProbableDuplicate
    )
    .then_some(r.duplicate_of)
    .flatten();
    Outcome {
        surfaced,
        acted,
        status: r.status,
    }
}

#[test]
fn corpus_is_well_formed() {
    let cases = load();
    assert!(cases.len() >= 10, "场景太少，只有 {}", cases.len());
    let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(n, ids.len(), "场景 id 必须唯一");
    assert!(
        cases.iter().any(|c| c.expect.is_some()),
        "得有真重复的正样本"
    );
    assert!(
        cases.iter().any(|c| c.expect.is_none()),
        "得有不该判重的负样本，否则只量了召回"
    );
}

#[test]
fn duplicate_detection_does_not_regress() {
    let cases = load();
    let mut failures = Vec::new();
    let mut gaps = Vec::new();
    let mut fixed_gaps = Vec::new();
    // 正样本 = 该判重的；负样本 = 不该判重的。分开统计。
    let (mut pos, mut pos_hit, mut neg, mut neg_ok) = (0usize, 0usize, 0usize, 0usize);
    let mut acted_hit = 0usize;

    for c in &cases {
        let o = run(c);
        // 两侧判据不同，因为代价不对称：
        // - 正样本看**有没有摆出来**（surfaced）：查重的产品价值在提示，关单默认关闭。
        // - 负样本只看**有没有被当成重复处置**（acted）：`related` 是"顺带列个线索"，
        //   两条不同的启动失败互相列出来对人是有用的，不算误报。
        let ok = match c.expect {
            Some(_) => o.surfaced == c.expect,
            None => o.acted.is_none(),
        };
        if c.expect.is_some() {
            pos += 1;
            if ok {
                pos_hit += 1;
            }
            if o.acted == c.expect {
                acted_hit += 1;
            }
        } else if !c.known_gap {
            // 已登记的缺口不进硬断言的分母：它们记录的是"我们知道会这样、且决定不改"，
            // 不是待修的回归。仍会在下面单独列出来。
            neg += 1;
            if ok {
                neg_ok += 1;
            }
        }
        let line = format!(
            "  {:<28} want={:<6} surfaced={:<6} acted={:<6} status={:<24} [{}]{}",
            c.id,
            c.expect.map(|n| n.to_string()).unwrap_or("-".into()),
            o.surfaced.map(|n| n.to_string()).unwrap_or("-".into()),
            o.acted.map(|n| n.to_string()).unwrap_or("-".into()),
            o.status.as_str(),
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

    eprintln!(
        "issue duplicate corpus: surfaced {pos_hit}/{pos}, acted-on {acted_hit}/{pos}, \
         no-false-positive {neg_ok}/{neg}  (failures={}, known gaps={})",
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
        "{} scenario(s) regressed:\n{}",
        failures.len(),
        failures.join("\n")
    );

    // 误报是不能退让的那一侧：把一条真问题当旧单关掉，比漏判贵得多。
    assert_eq!(
        neg_ok, neg,
        "负样本必须零误报（{neg_ok}/{neg}）——判错重复会把真问题当旧单处理"
    );

    // 召回下限。改了 duplicate 的阈值/权重后掉下来就红。
    const MIN_SURFACED: usize = 3;
    assert!(
        pos_hit >= MIN_SURFACED,
        "查重召回 {pos_hit}/{pos} 低于下限 {MIN_SURFACED}"
    );
}
