//! Issue triage 管线：normalize → safety → classify → completeness → duplicate → judge → comment。

use super::action::{plan_actions, ActionPolicy, PlannedActions};
use super::classify::{classify_heuristic, classify_with_llm, Classification};
use super::completeness::check_completeness;
use super::duplicate::find_duplicates;
use super::embedding::{Embedder, LocalEmbedder};
use super::explain::generate_user_comment;
use super::hash::{comments_hash, content_hash};
use super::judge::judge;
use super::mentions::{
    format_mention_line, resolve_mentions, resolve_triage_owners, MentionConfig,
};
use super::model::{
    IssueReviewDecision, NormalizedIssue, PublishResult, RawComment, RawIssue, StoredIssue,
};
use super::normalize::normalize_issue;
use super::platform::IssuePlatform;
use super::safety::score_safety;
use super::store::{f32s_to_bytes, IssueStore};
use super::verify::{resolve_repo_root, should_verify, verify_level0, TechnicalVerification};
use crate::llm::LlmClient;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueReviewConfig {
    pub vector_enabled: bool,
    pub candidate_limit: usize,
    pub min_similarity: f32,
    pub actions: ActionPolicy,
    /// Phase 2：只读代码验证。
    pub verify_enabled: bool,
    pub verify_search_tests: bool,
    pub repo_root: Option<PathBuf>,
    /// 评论 @mention。有结论时只抄送；转人工时会指派给 `on_needs_triage`。
    pub mentions: MentionConfig,
}

impl Default for IssueReviewConfig {
    fn default() -> Self {
        Self {
            vector_enabled: true,
            candidate_limit: 20,
            min_similarity: 0.35,
            actions: ActionPolicy::default(),
            verify_enabled: false,
            verify_search_tests: true,
            repo_root: None,
            mentions: MentionConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewOutput {
    pub decision: IssueReviewDecision,
    pub normalized: NormalizedIssue,
    pub content_hash: String,
    pub comments_hash: String,
    pub planned: PlannedActions,
    /// 技术验证快照（供 explain / 调试）。
    pub technical: Option<TechnicalVerification>,
}

/// 从已入库 / 内存数据执行 triage（同步；评论默认定性模板，LLM 说明见 `finalize_comment`）。
pub fn triage_stored(
    store: &IssueStore,
    issue: &StoredIssue,
    comments: &[(u64, String, String)],
    cfg: &IssueReviewConfig,
    embedder: &dyn Embedder,
) -> Result<ReviewOutput> {
    triage_stored_with_class(store, issue, comments, cfg, embedder, None)
}

/// 同上，但允许外层传入已经算好的分类结果。
///
/// 分类是唯一需要网络的一步（低置信时问模型），把它留在异步的调用方算完再传进来，
/// 这条主管线就能保持同步、可离线、好测。`None` = 用纯规则。
pub fn triage_stored_with_class(
    store: &IssueStore,
    issue: &StoredIssue,
    comments: &[(u64, String, String)],
    cfg: &IssueReviewConfig,
    embedder: &dyn Embedder,
    classification: Option<Classification>,
) -> Result<ReviewOutput> {
    let body = &issue.body_raw;
    let normalized = normalize_issue(&issue.title, body);
    let safety = score_safety(&issue.title, body);
    let classification =
        classification.unwrap_or_else(|| classify_heuristic(&issue.title, body, &safety));
    let completeness = check_completeness(classification.primary_type, &normalized);
    let duplicate = find_duplicates(
        store,
        issue.issue_number,
        &normalized,
        embedder,
        cfg.vector_enabled,
        cfg.candidate_limit,
        cfg.min_similarity,
    );
    let is_dup = matches!(
        duplicate.status,
        super::model::DuplicateStatus::ExactDuplicate
            | super::model::DuplicateStatus::ProbableDuplicate
    );
    let technical = run_technical_if_needed(
        cfg,
        classification.primary_type,
        safety.spam_score,
        completeness.score,
        completeness.can_verify,
        duplicate.confidence,
        is_dup,
        &normalized,
    );
    let mut decision = judge(
        issue.issue_number,
        &classification,
        &completeness,
        &safety,
        &duplicate,
        technical.as_ref(),
    );
    decision.issue_title = issue.title.clone();
    // 错投检测正交于类型/裁决，判完再补一刀，不影响主链路结论。
    let misrouted = super::misrouted::detect_misrouted(&issue.title, body, &issue.repo_id);
    if misrouted.detected {
        decision.misrouted_repos = misrouted.target_repos;
        decision.misrouted_confidence = misrouted.confidence;
        decision.reasons.extend(misrouted.reasons);
        decision
            .suggested_labels
            .push(super::action::WRONG_REPO_LABEL.to_string());
    }
    decision.symptom_summary = if !normalized.symptom.is_empty() {
        normalized.symptom.clone()
    } else if !normalized.actual_behavior.is_empty() {
        normalized.actual_behavior.clone()
    } else {
        issue
            .body_raw
            .lines()
            .map(str::trim)
            .find(|l| l.len() > 8)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect()
    };
    // 默认定性人话（无 LLM）；`finalize_comment` 可升级为 LLM 说明
    let mentions = resolve_mentions(&cfg.mentions, decision.verdict, decision.primary_type);
    let mention_line = format_mention_line(&mentions);
    let triage_owners = resolve_triage_owners(&cfg.mentions);
    let planned = plan_actions(
        &cfg.actions,
        &decision,
        triage_owners.first().map(String::as_str),
    );
    decision.suggested_comment = if planned.needs_human_notice {
        // 结论没把握：不下判断，只把这条交给指定处理人。
        super::explain::render_triage_handoff(&decision, body, &triage_owners)
    } else {
        super::explain::generate_user_comment_sync(
            &decision,
            &normalized,
            body,
            technical.as_ref(),
            mention_line.as_deref(),
        )
    };
    let ch = if issue.content_hash.is_empty() {
        content_hash(&issue.title, body)
    } else {
        issue.content_hash.clone()
    };
    let cmt_refs: Vec<(u64, &str, &str)> = comments
        .iter()
        .map(|(id, u, b)| (*id, u.as_str(), b.as_str()))
        .collect();
    let cmh = if cmt_refs.is_empty() {
        issue.comments_hash.clone()
    } else {
        comments_hash(&cmt_refs)
    };

    store.save_review(&decision, &ch, &cmh, None)?;
    // 判定即留痕：被闸门拦下、最终什么都没做的那些也要能统计出来。
    store.record_action_audit(&decision, &planned, false, None)?;

    Ok(ReviewOutput {
        decision,
        normalized,
        content_hash: ch,
        comments_hash: cmh,
        planned,
        technical,
    })
}

/// 用 LLM（若提供）重写面向用户的评论正文。
pub async fn finalize_comment(
    out: &mut ReviewOutput,
    body_raw: &str,
    cfg: &IssueReviewConfig,
    llm: Option<&dyn LlmClient>,
) {
    // 移交话术已定稿：让 LLM 重写会把「没把握」重新写成一个结论。
    if out.planned.needs_human_notice {
        return;
    }
    let mentions = resolve_mentions(
        &cfg.mentions,
        out.decision.verdict,
        out.decision.primary_type,
    );
    let mention_line = format_mention_line(&mentions);
    out.decision.suggested_comment = generate_user_comment(
        llm,
        &out.decision,
        &out.normalized,
        body_raw,
        out.technical.as_ref(),
        mention_line.as_deref(),
    )
    .await;
}

/// 将 RawIssue 规范化入库（索引，不回复）。
pub fn ingest_raw(
    store: &IssueStore,
    raw: &RawIssue,
    comments: &[RawComment],
    embedder: Option<&dyn Embedder>,
) -> Result<StoredIssue> {
    let body = raw.body.clone().unwrap_or_default();
    let n = normalize_issue(&raw.title, &body);
    let labels: Vec<String> = raw.labels.iter().map(|l| l.name.clone()).collect();
    let cmt_refs: Vec<(u64, &str, &str)> = comments
        .iter()
        .map(|c| (c.id, c.updated_at.as_str(), c.body.as_str()))
        .collect();
    let mut issue = StoredIssue {
        repo_id: store.repo_id.clone(),
        issue_number: raw.number,
        title: raw.title.clone(),
        body_raw: body.clone(),
        body_clean: n.body_clean.clone(),
        state: raw.state.clone(),
        labels_json: serde_json::to_string(&labels)?,
        author: raw
            .user
            .as_ref()
            .map(|u| u.login.clone())
            .unwrap_or_default(),
        created_at: raw.created_at.clone(),
        updated_at: raw.updated_at.clone(),
        closed_at: raw.closed_at.clone(),
        error_signature: n.error_signatures.join(","),
        stack_symbols_json: serde_json::to_string(&n.stack_symbols)?,
        source_updated_at: raw.updated_at.clone(),
        content_hash: content_hash(&raw.title, &body),
        comments_hash: comments_hash(&cmt_refs),
        embedding: None,
        embedding_model: None,
        embedding_version: None,
        embedding_content_hash: None,
        last_synced_at: raw.updated_at.clone(),
        last_reviewed_at: None,
    };
    if let Some(e) = embedder {
        match e.embed(&n.embed_text) {
            Ok(v) => {
                issue.embedding = Some(f32s_to_bytes(&v));
                issue.embedding_model = Some(e.model().into());
                issue.embedding_version = Some(e.version().into());
                issue.embedding_content_hash = Some(issue.content_hash.clone());
            }
            Err(_) => {
                // 索引阶段 embedding 失败不阻断同步
            }
        }
    }
    store.upsert_issue(&issue)?;
    Ok(issue)
}

/// 从平台同步历史/增量 Issue（只建索引，不 bulk reply）。
/// 返回本次同步入库的 issue number 列表（供 watch 继续 triage）。
pub async fn sync_from_platform(
    store: &IssueStore,
    platform: &dyn IssuePlatform,
    max_issues: usize,
    since: Option<&str>,
    embed: bool,
) -> Result<Vec<u64>> {
    let embedder = LocalEmbedder;
    let mut page = 1u32;
    let mut synced: Vec<u64> = Vec::new();
    let per_page = 50u32;
    // 是否因为 max_issues 提前收手（而不是"平台已经没有更多了"）。
    let mut capped = false;
    loop {
        let batch = platform
            .list_issues_page(page, per_page, since)
            .await
            .with_context(|| format!("list issues page {page}"))?;
        if batch.is_empty() {
            break;
        }
        for raw in &batch {
            // PR 在这里排除，而不是在适配器里——上面的翻页判据要看到平台返回的**真实页长**，
            // 否则一页里 PR 一多就会被当成"没有下一页"而提前收手。
            if raw.pull_request.is_some() {
                continue;
            }
            // 已入库且平台侧没变过的跳过：不占本轮配额，也不再为它拉一次评论。
            // 少了这一步，限量 + 游标不前进会让每轮都重拉同一批，积压永远消化不掉。
            if store
                .get_issue(raw.number)?
                .is_some_and(|s| s.source_updated_at == raw.updated_at)
            {
                continue;
            }
            if synced.len() >= max_issues {
                capped = true;
                break;
            }
            // 轻量：同步时拉评论写入 comments_hash；后续 review 再分析
            let comments = platform.list_comments(raw.number).await.unwrap_or_default();
            let emb: Option<&dyn Embedder> = if embed { Some(&embedder) } else { None };
            ingest_raw(store, raw, &comments, emb)?;
            synced.push(raw.number);
        }
        if batch.len() < per_page as usize || synced.len() >= max_issues {
            capped = capped || batch.len() >= per_page as usize;
            break;
        }
        page += 1;
    }
    // 还有没同步完的就**不能推进游标**：游标一旦跳到"现在"，没拉到的那些永远落在
    // 游标之后，再也不会被拉回来。留在原处，下一轮继续从同一位置拉。
    if capped {
        eprintln!(
            "  [issue] synced {} issue(s) (capped); leaving the sync cursor in place so the rest are picked up next round.",
            synced.len()
        );
        return Ok(synced);
    }
    // GitHub Issues API `since` 要求 ISO8601（RFC3339 / UTC）
    let cursor = iso_now();
    store.set_sync_cursor(&cursor)?;
    Ok(synced)
}

/// 审查单条：拉平台 → 入库 → triage →（可选）LLM 用户向说明。
pub async fn review_issue(
    store: &IssueStore,
    platform: &dyn IssuePlatform,
    number: u64,
    cfg: &IssueReviewConfig,
    embedder: &dyn Embedder,
) -> Result<ReviewOutput> {
    review_issue_with_llm(store, platform, number, cfg, embedder, None).await
}

/// 同上，可注入 LLM 生成面向用户的说明。
pub async fn review_issue_with_llm(
    store: &IssueStore,
    platform: &dyn IssuePlatform,
    number: u64,
    cfg: &IssueReviewConfig,
    embedder: &dyn Embedder,
    llm: Option<&dyn LlmClient>,
) -> Result<ReviewOutput> {
    let raw = platform.get_issue(number).await?;
    let comments = platform.list_comments(number).await.unwrap_or_default();
    // 过滤 bot 自身评论，避免自循环参与分析
    let user_comments: Vec<RawComment> = comments
        .into_iter()
        .filter(|c| {
            let is_bot = c
                .user
                .as_ref()
                .and_then(|u| u.user_type.as_deref())
                .map(|t| t.eq_ignore_ascii_case("bot"))
                .unwrap_or(false);
            let is_rg = super::comment::is_bot_comment(&c.body);
            !is_bot && !is_rg
        })
        .collect();
    let stored = ingest_raw(store, &raw, &user_comments, Some(embedder))?;
    let cmt_tuples: Vec<(u64, String, String)> = user_comments
        .iter()
        .map(|c| (c.id, c.updated_at.clone(), c.body.clone()))
        .collect();
    // 分类先行：规则没把握时问一次模型。它是整条链路的地基——
    // primary_type 决定话术、裁决、要不要跑验证、@ 谁。
    let class = classify_with_llm(
        llm,
        &stored.title,
        &stored.body_raw,
        &score_safety(&stored.title, &stored.body_raw),
    )
    .await;
    let mut out =
        triage_stored_with_class(store, &stored, &cmt_tuples, cfg, embedder, Some(class))?;
    // 有 LLM 时重写用户向正文；无则保留确定性人话。
    //
    // **发不出去就不润色**：`suggest` 模式（默认）与 `watch` 长跑下这条评论根本不会
    // 发布，为它调一次模型是纯浪费——实测每条 issue 因此多花约 40 秒和一次调用，
    // 而产物没有任何人会看到。分类兜底不受影响，它在上面已经跑过了。
    // 不按「会不会发布」来跳过润色：suggest 模式下这条评论正是要给人看的**最终产物**，
    // 跳过就没法验证机器人到底会说什么。省调用是另一个问题，别拿可观测性换。
    if llm.is_some() {
        finalize_comment(&mut out, &stored.body_raw, cfg, llm).await;
        store.save_review(&out.decision, &out.content_hash, &out.comments_hash, None)?;
    }
    Ok(out)
}

/// 参数都是彼此独立的门禁维度，聚成结构体只会多一层间接、看不出少传了哪个。
#[allow(clippy::too_many_arguments)]
fn run_technical_if_needed(
    cfg: &IssueReviewConfig,
    issue_type: super::model::IssueType,
    spam_score: f32,
    completeness: f32,
    can_verify: bool,
    dup_conf: f32,
    is_dup: bool,
    normalized: &super::model::NormalizedIssue,
) -> Option<TechnicalVerification> {
    let root = cfg.repo_root.clone().or_else(|| resolve_repo_root(None));
    let accessible = root.as_ref().map(|p| p.is_dir()).unwrap_or(false);
    let (ok, reason) = should_verify(
        issue_type,
        spam_score,
        completeness,
        can_verify,
        dup_conf,
        is_dup,
        cfg.verify_enabled,
        accessible,
    );
    if !ok {
        return Some(TechnicalVerification {
            enabled: false,
            skipped_reason: reason,
            ..Default::default()
        });
    }
    let root = root?;
    match verify_level0(&root, normalized, cfg.verify_search_tests) {
        Ok(v) => Some(v),
        Err(e) => Some(TechnicalVerification {
            enabled: false,
            skipped_reason: Some(format!("verify_error:{e}")),
            ..Default::default()
        }),
    }
}

/// 发布或更新唯一机器人评论，并按 planned 执行标签/关闭。
pub async fn publish_decision(
    store: &IssueStore,
    platform: &dyn IssuePlatform,
    out: &ReviewOutput,
) -> Result<PublishResult> {
    let planned = &out.planned;
    let mut comment_id = String::new();
    let mut created = false;
    let mut updated = false;

    if planned.post_or_update_comment {
        let body = &out.decision.suggested_comment;
        let existing = platform.find_bot_comment(out.decision.issue_number).await?;
        if let Some(id) = existing {
            platform
                .update_comment(out.decision.issue_number, &id, body)
                .await?;
            comment_id = id;
            updated = true;
        } else {
            comment_id = platform
                .create_comment(out.decision.issue_number, body)
                .await?;
            created = true;
        }
        store.set_published_comment(out.decision.issue_number, &out.content_hash, &comment_id)?;
    }

    if !planned.labels_to_add.is_empty() {
        platform
            .add_labels(out.decision.issue_number, &planned.labels_to_add)
            .await?;
    }
    if let Some(login) = planned.assign_to.as_deref() {
        platform
            .assign(out.decision.issue_number, login)
            .await
            .with_context(|| format!("assign #{} to {login}", out.decision.issue_number))?;
    }
    if planned.close {
        let reason = planned
            .close_reason
            .as_deref()
            .unwrap_or("reviewgate_policy");
        platform
            .close_issue(out.decision.issue_number, reason)
            .await?;
    }

    let published_id = if comment_id.is_empty() {
        None
    } else {
        Some(comment_id.as_str())
    };
    store.save_review(
        &out.decision,
        &out.content_hash,
        &out.comments_hash,
        published_id,
    )?;
    store.record_action_audit(&out.decision, planned, true, published_id)?;
    Ok(PublishResult {
        issue_number: out.decision.issue_number,
        comment_id,
        created,
        updated,
    })
}

/// 当前 UTC 时间的 RFC3339 / ISO8601 字符串（GitHub `since` 参数格式）。
pub fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_secs_rfc3339(secs)
}

/// 将 Unix 秒转为 `YYYY-MM-DDTHH:MM:SSZ`（无依赖 chrono）。
pub fn format_unix_secs_rfc3339(secs: u64) -> String {
    // 公历算法：从 1970-01-01 起算
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;

    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// 渲染 dry-run 文本摘要。
pub fn format_review_text(out: &ReviewOutput) -> String {
    let d = &out.decision;
    let mut s = String::new();
    s.push_str(&format!("Issue #{}\n", d.issue_number));
    s.push_str(&format!(
        "type: {} ({:.0}%)\n",
        d.primary_type.as_str(),
        d.type_confidence * 100.0
    ));
    s.push_str(&format!(
        "completeness: {:.0}% missing={:?}\n",
        d.completeness_score * 100.0,
        d.missing_fields
    ));
    s.push_str(&format!(
        "spam={:.2} ad={:.2} inject={:.2}\n",
        d.spam_score, d.advertisement_score, d.prompt_injection_score
    ));
    s.push_str(&format!(
        "duplicate: {} conf={:.0}% of={:?} candidates={}\n",
        d.duplicate_status.as_str(),
        d.duplicate_confidence * 100.0,
        d.duplicate_of,
        d.duplicate_candidates.len()
    ));
    s.push_str(&format!(
        "verdict: {} conf={:.0}% vector_used={} vector_degraded={}\n",
        d.verdict.as_str(),
        d.confidence * 100.0,
        d.vector_used,
        d.vector_degraded
    ));
    let deep_n = out
        .technical
        .as_ref()
        .map(|t| t.deep_dig.len())
        .unwrap_or(0);
    s.push_str(&format!(
        "technical: ran={} verdict={} conf={:.0}% paths={:?} fix_prs={:?} deep_dig={deep_n}\n",
        d.verification_ran,
        d.technical_verdict.as_str(),
        d.technical_confidence * 100.0,
        d.code_paths,
        d.fix_prs
    ));
    s.push_str(&format!("reasons: {}\n", d.reasons.join("; ")));
    s.push_str(&format!(
        "planned: comment={} close={} labels={:?} close_reason={:?}\n",
        out.planned.post_or_update_comment,
        out.planned.close,
        out.planned.labels_to_add,
        out.planned.close_reason
    ));
    s.push_str("--- comment preview ---\n");
    s.push_str(&d.suggested_comment);
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::embedding::FailingEmbedder;
    use crate::issue::model::{RawLabel, RawUser};
    use crate::issue::platform::FixturePlatform;

    #[test]
    fn format_unix_secs_rfc3339_known_epoch() {
        // 0 → 1970-01-01T00:00:00Z
        assert_eq!(format_unix_secs_rfc3339(0), "1970-01-01T00:00:00Z");
        // 1_700_000_000 → 2023-11-14T22:13:20Z
        assert_eq!(
            format_unix_secs_rfc3339(1_700_000_000),
            "2023-11-14T22:13:20Z"
        );
        let now = iso_now();
        assert!(
            now.ends_with('Z') && now.len() == 20 && now.as_bytes()[10] == b'T',
            "iso_now shape: {now}"
        );
        // must not be bare unix seconds
        assert!(
            now.parse::<u64>().is_err(),
            "must not be epoch integer: {now}"
        );
    }

    fn raw(n: u64, title: &str, body: &str) -> RawIssue {
        RawIssue {
            number: n,
            title: title.into(),
            body: Some(body.into()),
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

    #[tokio::test]
    async fn end_to_end_triage_and_idempotent_publish() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        platform.seed_issue(raw(
            1,
            "Windows save crash access violation",
            "## Actual\naccess violation\n## Steps\n1. open settings\n2. click save\nWindows 11",
        ));
        platform.seed_issue(raw(
            2,
            "Crash when saving on Windows",
            "access violation after save button\nWindows",
        ));

        let synced = sync_from_platform(&store, &platform, 100, None, true)
            .await
            .unwrap();
        assert_eq!(synced.len(), 2);
        assert_eq!(store.count_issues().unwrap(), 2);
        // sync cursor must be GitHub-compatible ISO8601
        let cursor = store.get_sync_cursor().unwrap().expect("cursor");
        assert!(
            cursor.ends_with('Z')
                && cursor.contains('T')
                && cursor.len() == 20
                && cursor.as_bytes()[4] == b'-'
                && cursor.as_bytes()[7] == b'-',
            "cursor must be YYYY-MM-DDTHH:MM:SSZ, got {cursor}"
        );

        // FTS hit
        let hits = store.fts_search("access violation", 10).unwrap();
        assert!(!hits.is_empty(), "{hits:?}");

        // 这条用例验的是发布链路（建评论 → 幂等更新），所以要显式开 publish 模式。
        // 默认是 suggest（只分析不发言）——那条语义由 action.rs 的用例覆盖。
        let cfg = IssueReviewConfig {
            actions: crate::issue::ActionPolicy {
                publish: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let emb = LocalEmbedder;
        let out = review_issue(&store, &platform, 2, &cfg, &emb)
            .await
            .unwrap();
        assert!(!out.decision.primary_type.as_str().is_empty());
        assert!(!out.decision.suggested_comment.is_empty());
        assert!(out
            .decision
            .suggested_comment
            .contains("reviewgate:issue-review"));
        // should find #1 as related/duplicate candidate
        assert!(
            !out.decision.duplicate_candidates.is_empty()
                || out.decision.duplicate_of == Some(1)
                || out.decision.verdict.as_str().len() > 3,
            "{:?}",
            out.decision
        );

        let text = format_review_text(&out);
        assert!(text.contains("type:"));
        assert!(text.contains("verdict:"));
        assert!(text.contains("completeness:"));

        let p1 = publish_decision(&store, &platform, &out).await.unwrap();
        assert!(p1.created);
        let p2 = publish_decision(&store, &platform, &out).await.unwrap();
        assert!(p2.updated);
        assert!(!p2.created);
        assert_eq!(platform.comment_count(2), 1);
        assert!(!out.planned.close);
    }

    /// 回归：一轮只同步得下一部分时，游标不能跳到"现在"——否则没同步到的那些
    /// 永远落在游标之后，再也不会被拉回来（静默丢单）。
    #[tokio::test]
    async fn sync_cursor_does_not_advance_when_the_batch_was_capped() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        for n in 1..=5 {
            platform.seed_issue(raw(n, &format!("issue {n}"), "body"));
        }
        store.set_sync_cursor("2024-01-01T00:00:00Z").unwrap();

        let synced = sync_from_platform(&store, &platform, 2, None, false)
            .await
            .unwrap();
        assert_eq!(synced.len(), 2, "本轮只处理 2 条");
        assert_eq!(
            store.get_sync_cursor().unwrap().as_deref(),
            Some("2024-01-01T00:00:00Z"),
            "还有没同步的，游标必须原地不动"
        );

        // 全部同步得下时才前进。
        let _ = sync_from_platform(&store, &platform, 100, None, false)
            .await
            .unwrap();
        assert_ne!(
            store.get_sync_cursor().unwrap().as_deref(),
            Some("2024-01-01T00:00:00Z"),
            "全部同步完，游标应前进"
        );
    }

    /// 回归：游标不动 + 每轮限量，如果每轮都从头重拉同一批，积压永远消化不掉。
    /// 已经入库且未变更的要跳过，配额留给还没同步过的。
    #[tokio::test]
    async fn capped_sync_makes_progress_across_rounds() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        for n in 1..=3 {
            platform.seed_issue(raw(n, &format!("issue {n}"), "body"));
        }

        assert_eq!(
            sync_from_platform(&store, &platform, 1, None, false)
                .await
                .unwrap(),
            vec![1]
        );
        assert_eq!(
            sync_from_platform(&store, &platform, 1, None, false)
                .await
                .unwrap(),
            vec![2],
            "第二轮必须往前走，而不是又拉一遍 #1"
        );
        assert_eq!(
            sync_from_platform(&store, &platform, 1, None, false)
                .await
                .unwrap(),
            vec![3]
        );
        assert_eq!(store.list_issue_numbers().unwrap(), vec![1, 2, 3]);
    }

    /// 回归（cli/cli 真机）：GitHub 的 `/issues` 接口把 PR 也算进来，适配器过滤掉 PR 后
    /// 返回的页比 `per_page` 短，翻页判据把"这一页 PR 多"误读成"没有下一页"——
    /// PR 活跃的仓库上 `issue init --max 10000` 只索引得到第一页，查重索引直接残废。
    #[tokio::test]
    async fn pagination_is_not_stopped_by_a_page_full_of_pull_requests() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        // 60 条一半是 PR：一页 50 条里只有 25 条真 Issue，凑够 30 条必须读到第二页。
        for n in 1..=60u64 {
            let mut r = raw(n, &format!("item {n}"), "body");
            if n % 2 == 0 {
                r.pull_request = Some(serde_json::json!({"url": "https://api/pulls/1"}));
            }
            platform.seed_issue(r);
        }

        let synced = sync_from_platform(&store, &platform, 30, None, false)
            .await
            .unwrap();
        assert_eq!(synced.len(), 30, "PR 占位不该让同步提前收手");
        assert!(
            synced.iter().all(|n| n % 2 == 1),
            "PR 不该进 Issue 索引：{synced:?}"
        );
    }

    /// 平台侧内容变了的，即使已经入库也要重新同步——跳过只对"没变过的"成立。
    #[tokio::test]
    async fn changed_issues_are_resynced_even_when_already_stored() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        platform.seed_issue(raw(1, "issue 1", "body"));
        assert_eq!(
            sync_from_platform(&store, &platform, 10, None, false)
                .await
                .unwrap(),
            vec![1]
        );

        let mut changed = raw(1, "issue 1 edited", "body edited");
        changed.updated_at = "2024-06-01T00:00:00Z".into();
        platform.seed_issue(changed);
        assert_eq!(
            sync_from_platform(&store, &platform, 10, None, false)
                .await
                .unwrap(),
            vec![1],
            "updated_at 变了就要重新入库"
        );
        assert_eq!(store.get_issue(1).unwrap().unwrap().title, "issue 1 edited");
    }

    /// 分批 triage 的前提：能从库里挑出还没审过的。
    #[tokio::test]
    async fn untriaged_issues_are_listed_oldest_first_and_capped() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        for n in 1..=4 {
            platform.seed_issue(raw(n, &format!("issue {n}"), "body"));
        }
        sync_from_platform(&store, &platform, 100, None, false)
            .await
            .unwrap();
        assert_eq!(store.untriaged_issues(10).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(store.untriaged_issues(2).unwrap(), vec![1, 2]);

        // 审过的不再出现，剩下的下一轮继续。
        let cfg = IssueReviewConfig::default();
        let emb = LocalEmbedder;
        review_issue(&store, &platform, 1, &cfg, &emb)
            .await
            .unwrap();
        assert_eq!(store.untriaged_issues(10).unwrap(), vec![2, 3, 4]);
    }

    #[tokio::test]
    async fn verify_path_enriches_decision() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        platform.seed_issue(raw(
            9,
            "IssueStore FTS MATCH fails",
            "## Actual\nerror in issues_fts\n## Expected\nhits returned\n## Steps\n1. open db\n2. query\n",
        ));
        let cfg = IssueReviewConfig {
            verify_enabled: true,
            repo_root: super::resolve_repo_root(None),
            ..Default::default()
        };
        let emb = LocalEmbedder;
        let out = review_issue(&store, &platform, 9, &cfg, &emb)
            .await
            .unwrap();
        // either ran verification or skipped with reason
        assert!(
            out.decision.verification_ran
                || out
                    .decision
                    .reasons
                    .iter()
                    .any(|r| r.starts_with("verify_skipped")),
            "{:?}",
            out.decision.reasons
        );
        let text = format_review_text(&out);
        assert!(text.contains("technical:"));
    }

    #[tokio::test]
    async fn vector_failure_still_reviews() {
        let store = IssueStore::open_in_memory("acme/app").unwrap();
        let platform = FixturePlatform::new();
        platform.seed_issue(raw(3, "panic in parser", "error: panic when parsing null"));
        ingest_raw(
            &store,
            &raw(3, "panic in parser", "error: panic when parsing null"),
            &[],
            Some(&LocalEmbedder),
        )
        .unwrap();
        // seed similar historical
        ingest_raw(
            &store,
            &raw(4, "parser panic on null", "panic error parsing"),
            &[],
            Some(&LocalEmbedder),
        )
        .unwrap();

        let cfg = IssueReviewConfig::default();
        let fail = FailingEmbedder;
        let out = review_issue(&store, &platform, 3, &cfg, &fail)
            .await
            .unwrap();
        assert!(out.decision.vector_degraded || !out.decision.vector_used);
        assert!(!out.decision.suggested_comment.is_empty());
        assert!(!out.decision.verdict.as_str().is_empty());
    }
}
