//! 混合查重：FTS5 + 精确错误签名 + 向量语义召回，再二次判定。

use super::embedding::Embedder;
use super::model::{DuplicateCandidate, DuplicateStatus, NormalizedIssue};
use super::store::IssueStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateResult {
    pub status: DuplicateStatus,
    pub confidence: f32,
    pub duplicate_of: Option<u64>,
    pub candidates: Vec<DuplicateCandidate>,
    pub evidence: Vec<String>,
    pub vector_used: bool,
    pub vector_degraded: bool,
}

/// 混合召回并判定是否重复。
pub fn find_duplicates(
    store: &IssueStore,
    issue_number: u64,
    normalized: &NormalizedIssue,
    embedder: &dyn Embedder,
    vector_enabled: bool,
    candidate_limit: usize,
    min_similarity: f32,
) -> DuplicateResult {
    let mut merged: Vec<DuplicateCandidate> = Vec::new();
    let mut evidence = Vec::new();
    let mut vector_used = false;
    let mut vector_degraded = false;

    // 1) FTS5
    let fts_q = format!(
        "{} {}",
        normalized.title,
        normalized.error_signatures.join(" ")
    );
    if let Ok(fts) = store.fts_search(&fts_q, candidate_limit) {
        for c in fts {
            if c.issue_number != issue_number {
                push_merge(&mut merged, c);
            }
        }
        if !merged.is_empty() {
            evidence.push("fts5_candidates".into());
        }
    }

    // 2) exact error / signature
    if let Ok(exact) =
        store.exact_error_match(&normalized.error_signatures, issue_number, candidate_limit)
    {
        for c in exact {
            push_merge(&mut merged, c);
        }
        if merged
            .iter()
            .any(|c| c.sources.iter().any(|s| s == "exact_error"))
        {
            evidence.push("exact_error_signature".into());
        }
    }

    // 2b) 反向：当前正文里是否原样出现了历史 Issue 的错误签名
    if let Ok(rev) =
        store.reverse_error_match(&normalized.body_clean, issue_number, candidate_limit)
    {
        let hit = !rev.is_empty();
        for c in rev {
            push_merge(&mut merged, c);
        }
        if hit && !evidence.iter().any(|e| e == "exact_error_signature") {
            evidence.push("exact_error_signature".into());
        }
    }

    // 3) vector semantic
    if vector_enabled {
        match embedder.embed(&normalized.embed_text) {
            Ok(vec) => {
                match store.vector_search(&vec, issue_number, candidate_limit, min_similarity) {
                    Ok(vs) => {
                        if !vs.is_empty() {
                            vector_used = true;
                            evidence.push("vector_semantic".into());
                        }
                        for c in vs {
                            push_merge(&mut merged, c);
                        }
                    }
                    Err(e) => {
                        vector_degraded = true;
                        evidence.push(format!("vector_search_failed:{e}"));
                    }
                }
            }
            Err(e) => {
                vector_degraded = true;
                evidence.push(format!("embedding_failed:{e}"));
            }
        }
    }

    // 重排：多源召回 / 精确错误签名优先于纯 FTS 弱命中
    for c in &mut merged {
        c.score = rerank_score(c);
    }
    merged.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    merged.truncate(candidate_limit);

    // Secondary judgment (deterministic rules; no LLM required)
    let (status, confidence, duplicate_of, more_ev) =
        secondary_judge(issue_number, normalized, &merged);
    evidence.extend(more_ev);

    DuplicateResult {
        status,
        confidence,
        duplicate_of,
        candidates: merged,
        evidence,
        vector_used,
        vector_degraded,
    }
}

fn push_merge(merged: &mut Vec<DuplicateCandidate>, c: DuplicateCandidate) {
    if let Some(existing) = merged.iter_mut().find(|x| x.issue_number == c.issue_number) {
        existing.score = existing.score.max(c.score);
        for s in c.sources {
            if !existing.sources.contains(&s) {
                existing.sources.push(s);
            }
        }
    } else {
        merged.push(c);
    }
}

fn rerank_score(c: &DuplicateCandidate) -> f32 {
    let mut s = c.score;
    if c.sources.iter().any(|x| x == "exact_error") {
        s += 0.25;
    }
    if c.sources.iter().any(|x| x == "vector") {
        s += 0.12;
    }
    if c.sources.len() >= 2 {
        s += 0.15;
    }
    if c.sources.len() >= 3 {
        s += 0.1;
    }
    s.min(1.5)
}

fn secondary_judge(
    _self_num: u64,
    normalized: &NormalizedIssue,
    candidates: &[DuplicateCandidate],
) -> (DuplicateStatus, f32, Option<u64>, Vec<String>) {
    if candidates.is_empty() {
        return (
            DuplicateStatus::NotDuplicate,
            0.7,
            None,
            vec!["no_candidates".into()],
        );
    }
    // 按相关度排序后**逐个**判定：affinity 最高的未必是真重复，
    // 只看第一名会让第二名的真重复被 title_overlap_too_low 连坐否决。
    let mut ranked: Vec<&DuplicateCandidate> = candidates.iter().collect();
    ranked.sort_by(|a, b| {
        candidate_affinity(normalized, b)
            .partial_cmp(&candidate_affinity(normalized, a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut fallback: Option<(DuplicateStatus, f32, Option<u64>, Vec<String>)> = None;
    for cand in ranked.iter().take(DUP_CANDIDATES_TO_JUDGE) {
        let r = judge_candidate(normalized, cand);
        if r.0 != DuplicateStatus::NotDuplicate {
            return r;
        }
        if fallback.is_none() {
            fallback = Some(r);
        }
    }
    fallback.unwrap_or((
        DuplicateStatus::NotDuplicate,
        0.65,
        None,
        vec!["candidates_weak".into()],
    ))
}

/// 前 N 个候选逐个判定；再多就是噪声了。
const DUP_CANDIDATES_TO_JUDGE: usize = 5;

fn judge_candidate(
    normalized: &NormalizedIssue,
    top: &DuplicateCandidate,
) -> (DuplicateStatus, f32, Option<u64>, Vec<String>) {
    let mut ev: Vec<String> = Vec::new();
    let title_sim = token_overlap(&normalized.title, &top.title);
    let multi_source = top.sources.len() >= 2;
    let exact_err = top.sources.iter().any(|s| s == "exact_error");
    let high_vec = top.sources.iter().any(|s| s == "vector") && top.score >= 0.5;
    let affinity = candidate_affinity(normalized, top);

    ev.push(format!("top_candidate=#{}", top.issue_number));
    if exact_err {
        ev.push("shared_error_signature".into());
    }
    if multi_source {
        ev.push("multi_source_recall".into());
    }
    if title_sim >= 0.5 {
        ev.push(format!("title_token_overlap={title_sim:.2}"));
    }

    // 标题实质重叠过低时，禁止仅靠向量/多源升为 ProbableDuplicate（共创大赛批次误伤）
    if title_sim < 0.15 && !exact_err {
        ev.push(format!("title_overlap_too_low={title_sim:.2}"));
        return (DuplicateStatus::NotDuplicate, 0.55, None, ev);
    }

    if (exact_err && title_sim >= 0.25)
        || (multi_source && title_sim >= 0.25 && affinity >= 0.8)
        || (exact_err && multi_source && title_sim >= 0.12)
    {
        return (
            DuplicateStatus::ProbableDuplicate,
            (0.82_f32).max(title_sim).min(0.97),
            Some(top.issue_number),
            ev,
        );
    }
    if high_vec && title_sim >= 0.40 {
        return (
            DuplicateStatus::ProbableDuplicate,
            top.score.min(0.95),
            Some(top.issue_number),
            ev,
        );
    }
    if affinity >= 0.7 && (title_sim >= 0.2 || exact_err) {
        return (DuplicateStatus::Related, 0.65, Some(top.issue_number), ev);
    }
    if title_sim >= 0.35 {
        return (DuplicateStatus::Related, 0.55, Some(top.issue_number), ev);
    }
    // 语义高度相似 + 标题确有实词重合 → 至少提示「相关」。
    // 两个条件缺一不可：只有向量分会把「贪吃蛇报错」和「网络连接中断」凑成一对，
    // 只有标题重合又会被同批次的模板化命名绑架。
    if top.score >= 0.8 && title_sim >= 0.2 {
        ev.push(format!("strong_vector_with_title_overlap={title_sim:.2}"));
        return (DuplicateStatus::Related, 0.6, Some(top.issue_number), ev);
    }
    // 保留已累积的证据：判不出重复时，最需要知道差在哪一项。
    ev.push(format!(
        "candidates_weak:title_sim={title_sim:.2},affinity={affinity:.2},top_score={:.2}",
        top.score
    ));
    (DuplicateStatus::NotDuplicate, 0.65, None, ev)
}

fn candidate_affinity(normalized: &NormalizedIssue, c: &DuplicateCandidate) -> f32 {
    let title_sim = token_overlap(&normalized.title, &c.title);
    let mut s = title_sim;
    if c.sources.iter().any(|x| x == "exact_error") {
        s += 0.4;
    }
    if c.sources.iter().any(|x| x == "vector") {
        s += 0.2 * c.score.min(1.0);
    }
    if c.sources.len() >= 2 {
        s += 0.15;
    }
    // 错误签名文本重叠
    if !normalized.error_signatures.is_empty() && !c.error_signature.is_empty() {
        let es = normalized.error_signatures.join(" ");
        s += 0.3 * token_overlap(&es, &c.error_signature);
    }
    s
}

/// 标题切词。中文按相邻二字（bigram）切——`is_alphanumeric()` 对汉字为真，
/// 直接按非字母数字分割会把整句中文留成一个巨型 token，两条标题的交集恒为空。
fn title_tokens(
    s: &str,
    stop: &std::collections::HashSet<&str>,
) -> std::collections::HashSet<String> {
    let lower = s.to_lowercase();
    let mut out = std::collections::HashSet::new();
    // 拉丁/数字：按词
    for t in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if t.chars().count() >= 2
            && t.chars().any(|c| c.is_ascii_alphanumeric())
            && !stop.contains(t)
        {
            out.insert(t.to_string());
        }
    }
    // CJK：连续段内做 bigram
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, out: &mut std::collections::HashSet<String>| {
        for w in run.windows(2) {
            out.insert(w.iter().collect());
        }
        if run.len() == 1 {
            out.insert(run[0].to_string());
        }
        run.clear();
    };
    for c in lower.chars() {
        if is_cjk(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
        || ('\u{3400}'..='\u{4dbf}').contains(&c)
        || ('\u{f900}'..='\u{faff}').contains(&c)
}

fn token_overlap(a: &str, b: &str) -> f32 {
    // 去掉「共创大赛」等活动前缀后再比，避免批次文案绑架重复判定
    let a = crate::issue::normalize::strip_campaign_noise(a);
    let b = crate::issue::normalize::strip_campaign_noise(b);
    let stop: std::collections::HashSet<&str> = [
        "bug",
        "feature",
        "feat",
        "共创大赛",
        "大赛",
        "enhancement",
        "issue",
        "fix",
        "the",
        "and",
        "for",
        "with",
    ]
    .into_iter()
    .collect();
    let ta = title_tokens(&a, &stop);
    let tb = title_tokens(&b, &stop);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f32;
    let uni = ta.union(&tb).count() as f32;
    inter / uni
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::embedding::{
        embed_local, FailingEmbedder, LocalEmbedder, EMBED_MODEL, EMBED_VERSION,
    };
    use crate::issue::hash::content_hash;
    use crate::issue::model::StoredIssue;
    use crate::issue::normalize::normalize_issue;
    use crate::issue::store::{f32s_to_bytes, IssueStore};

    /// 线上回归（AtomGit new_review/RuView #8 vs #2）：向量分 0.84、标题实词重合明确，
    /// 却因为 0.40 这个按 ASCII 词级定的阈值判成不相关。中文 bigram 的
    /// Jaccard 天然更低，强向量 + 标题确有重合时至少该给「相关」。
    #[test]
    fn strong_vector_with_real_title_overlap_is_related() {
        let store = IssueStore::open_in_memory("o/r").unwrap();
        seed(
            &store,
            2,
            "[Bug] 保存大文件时崩溃：write_file 全量读入导致 OOM",
            "保存 200MB 日志文件时进程崩溃退出，memory allocation failed",
        );
        let n = normalize_issue(
            "保存大文件的时候程序会崩溃",
            "保存一个很大的日志文件时程序直接退出了，报 memory allocation failed",
        );
        let r = find_duplicates(&store, 8, &n, &LocalEmbedder, true, 20, 0.35);
        assert_ne!(
            r.status,
            DuplicateStatus::NotDuplicate,
            "same issue restated must surface: {:?}",
            r.evidence
        );
        assert_eq!(r.duplicate_of, Some(2));
    }

    /// 但向量分再高，标题讲的不是一回事就不能牵连。
    #[test]
    fn strong_vector_without_title_overlap_stays_apart() {
        let store = IssueStore::open_in_memory("o/r").unwrap();
        seed(
            &store,
            2,
            "生成的贪吃蛇游戏出现api不能访问报错",
            "游戏里调用 api 报错",
        );
        let n = normalize_issue(
            "频繁出现网络连接中断，远端关闭或重置了连接",
            "error decoding response body，自动重连仍失败",
        );
        let r = find_duplicates(&store, 9, &n, &LocalEmbedder, true, 20, 0.35);
        assert_eq!(r.status, DuplicateStatus::NotDuplicate, "{:?}", r.evidence);
    }

    /// 线上回归（AtomGit new_review/go-redis #5）：#5 和 #1 讲同一件事，
    /// 但 affinity 最高的候选是一条 docs Issue，判定只看 top 一个，
    /// 于是 title_overlap_too_low 直接否决，第二名的真重复根本没被看到。
    #[test]
    fn duplicate_check_looks_past_the_top_candidate() {
        let store = IssueStore::open_in_memory("o/r").unwrap();
        seed(
            &store,
            7,
            "docs: README 缺少 cluster 模式下 Watch 的限制说明",
            "文档里没写 cluster 模式 Watch 要求所有 key 在同一 slot，建议补一节",
        );
        // 还原真实原文：报错贴在代码块里，因此 #1 有可提取的错误签名
        seed(&store, 1, "[Bug] Watch 跨 slot 时报 redis: Watch requires all keys to be in the same slot",
             "## 实际现象\n在集群模式下对两个 key 调用 Watch，直接返回错误：\n\n              ```\nredis: Watch requires all keys to be in the same slot\n```\n");
        let n = normalize_issue(
            "集群模式下 Watch 多个 key 直接报错不能用",
            "在 cluster 上对两个不同 key 做 Watch，直接返回 redis: Watch requires all keys to be in the same slot",
        );
        let r = find_duplicates(&store, 5, &n, &LocalEmbedder, true, 20, 0.35);
        assert_ne!(r.status, DuplicateStatus::NotDuplicate, "{:?}", r.evidence);
        assert_eq!(
            r.duplicate_of,
            Some(1),
            "must reach the real duplicate: {:?}",
            r.evidence
        );
    }

    /// 线上回归（AtomGit new_review/RuView #8 vs #2）：两条讲同一件事的中文标题
    /// 重合度算出 0.00，被 `title_overlap_too_low` 硬否决——中文字符
    /// `is_alphanumeric()` 为真，整句不被切分，交集恒为空。
    #[test]
    fn chinese_titles_actually_overlap() {
        let a = "[Bug] 保存大文件时崩溃：write_file 全量读入导致 OOM";
        let b = "保存大文件的时候程序会崩溃";
        let sim = token_overlap(a, b);
        assert!(sim >= 0.2, "same topic must overlap, got {sim:.2}");
    }

    #[test]
    fn unrelated_chinese_titles_stay_low() {
        let sim = token_overlap(
            "保存大文件时崩溃导致数据丢失",
            "希望支持批量导出会话记录为文档",
        );
        assert!(sim < 0.15, "unrelated topics must stay apart, got {sim:.2}");
    }

    #[test]
    fn ascii_titles_keep_working() {
        assert!(
            token_overlap(
                "rewind does not restore checkpoint",
                "rewind fails to restore the checkpoint"
            ) >= 0.4
        );
        assert!(token_overlap("rewind broken", "export session as markdown") < 0.15);
    }

    fn seed(store: &IssueStore, num: u64, title: &str, body: &str) {
        let n = normalize_issue(title, body);
        let emb = embed_local(&n.embed_text);
        store
            .upsert_issue(&StoredIssue {
                repo_id: store.repo_id.clone(),
                issue_number: num,
                title: title.into(),
                body_raw: body.into(),
                body_clean: n.body_clean.clone(),
                state: "open".into(),
                labels_json: "[]".into(),
                author: "u".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                closed_at: None,
                error_signature: n.error_signatures.join(","),
                stack_symbols_json: "[]".into(),
                source_updated_at: "t".into(),
                content_hash: content_hash(title, body),
                comments_hash: "c".into(),
                embedding: Some(f32s_to_bytes(&emb)),
                embedding_model: Some(EMBED_MODEL.into()),
                embedding_version: Some(EMBED_VERSION.into()),
                embedding_content_hash: Some(content_hash(title, body)),
                last_synced_at: "t".into(),
                last_reviewed_at: None,
            })
            .unwrap();
    }

    #[test]
    fn hybrid_recall_uses_vector_and_degrades() {
        let store = IssueStore::open_in_memory("o/r").unwrap();
        seed(
            &store,
            10,
            "Windows save crash access violation",
            "click save causes access violation on Windows",
        );
        seed(
            &store,
            11,
            "readme typo",
            "documentation spelling fix needed",
        );

        let n = normalize_issue(
            "crash when saving config on Windows",
            "access violation after clicking save",
        );
        let emb = LocalEmbedder;
        let r = find_duplicates(&store, 99, &n, &emb, true, 10, 0.1);
        assert!(
            r.vector_used
                || r.candidates
                    .iter()
                    .any(|c| c.sources.iter().any(|s| s == "vector")),
            "expected vector path: {r:?}"
        );
        assert!(r.candidates.iter().any(|c| c.issue_number == 10));

        let fail = FailingEmbedder;
        let r2 = find_duplicates(&store, 99, &n, &fail, true, 10, 0.1);
        assert!(r2.vector_degraded, "must degrade when embed fails");
        // FTS/exact should still find candidates
        assert!(
            !r2.candidates.is_empty() || r2.evidence.iter().any(|e| e.contains("embedding_failed")),
            "{r2:?}"
        );
        // review path continues — we still get a status
        assert!(matches!(
            r2.status,
            DuplicateStatus::NotDuplicate
                | DuplicateStatus::Related
                | DuplicateStatus::ProbableDuplicate
        ));
    }

    #[test]
    fn contest_prefix_alone_does_not_make_probable_duplicate() {
        use crate::issue::model::DuplicateCandidate;
        let n = normalize_issue(
            "[共创大赛][Feature] apikey 复制丢失",
            "配置 apikey 后复制其它内容会丢 key",
        );
        let candidates = vec![
            DuplicateCandidate {
                issue_number: 1233,
                title: "[共创大赛]-[Feature] 同步技能和记忆功能".into(),
                score: 0.9,
                sources: vec!["vector".into()],
                error_signature: String::new(),
            },
            DuplicateCandidate {
                issue_number: 1226,
                title: "[共创大赛][Bug] request_user_input 直接返回错误".into(),
                score: 0.85,
                sources: vec!["vector".into(), "fts".into()],
                error_signature: String::new(),
            },
        ];
        let (status, conf, of, ev) = secondary_judge(1224, &n, &candidates);
        assert!(
            !matches!(status, DuplicateStatus::ProbableDuplicate | DuplicateStatus::ExactDuplicate),
            "must not mark probable dup on campaign prefix only: status={status:?} conf={conf} of={of:?} ev={ev:?}"
        );
    }
}
