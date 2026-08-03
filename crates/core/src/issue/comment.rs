//! 旧路径兼容：渲染时走与 explain 一致的「自然段落」风格，无表单小标题。
//! 主路径请用 `explain::generate_user_comment*`。

use super::mentions::{format_mention_line, resolve_mentions, MentionConfig};
use super::model::{IssueReviewDecision, NormalizedIssue, BOT_COMMENT_MARKER};

/// 渲染 Issue Review 评论正文（无 @mention）。
pub fn render_comment(d: &IssueReviewDecision) -> String {
    render_comment_with_mentions(d, &MentionConfig::default())
}

/// 渲染评论：自然段落，无「定位/原因/建议」死板标题。
pub fn render_comment_with_mentions(d: &IssueReviewDecision, mentions: &MentionConfig) -> String {
    let mut md = String::new();
    md.push_str(BOT_COMMENT_MARKER);
    md.push_str("\n\n");

    let logins = resolve_mentions(mentions, d.verdict, d.primary_type);
    if let Some(line) = format_mention_line(&logins) {
        md.push_str(&line);
        md.push_str("\n\n");
    }

    // 用 explain 的确定性人话（按结论形态），保证与主路径一致
    let n = NormalizedIssue {
        title: d.issue_title.clone(),
        symptom: d.symptom_summary.clone(),
        body_clean: d.symptom_summary.clone(),
        ..Default::default()
    };
    let body = super::explain::deterministic_narrative(d, &n, &d.symptom_summary, None);
    md.push_str(body.trim());
    md.push('\n');
    md
}

/// 评论是否为 ReviewGate Issue Review 机器人评论。
pub fn is_bot_comment(body: &str) -> bool {
    body.contains(BOT_COMMENT_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::model::*;

    fn base_decision() -> IssueReviewDecision {
        IssueReviewDecision {
            issue_number: 1,
            primary_type: IssueType::Bug,
            type_confidence: 0.8,
            type_reasons: vec!["error_language".into()],
            completeness_score: 0.7,
            missing_fields: vec!["reproduction_steps".into()],
            can_verify: true,
            spam_score: 0.0,
            advertisement_score: 0.0,
            abuse_score: 0.0,
            prompt_injection_score: 0.0,
            duplicate_status: DuplicateStatus::NotDuplicate,
            duplicate_confidence: 0.7,
            duplicate_of: None,
            duplicate_candidates: vec![],
            duplicate_evidence: vec![],
            verdict: IssueVerdict::LikelyBug,
            confidence: 0.8,
            reasons: vec!["error_language".into()],
            suggested_labels: vec!["bug".into()],
            suggested_comment: String::new(),
            close_recommended: false,
            auto_action_allowed: false,
            needs_human_review: false,
            vector_used: false,
            vector_degraded: false,
            analyzer_version: "t".into(),
            technical_verdict: IssueVerdict::LikelyBug,
            technical_confidence: 0.72,
            technical_evidence: vec![],
            code_paths: vec![],
            code_hits: vec![CodeEvidence {
                path: "crates/foo/src/retry.rs".into(),
                line: 1380,
                snippet: "网络连接中断:远端关闭或重置了连接".into(),
            }],
            related_commits: vec![],
            fix_prs: vec![],
            verification_ran: true,
            issue_title: "网络连接中断".into(),
            symptom_summary: "频繁出现远端关闭连接".into(),
            misrouted_repos: vec![],
            misrouted_confidence: 0.0,
        }
    }

    #[test]
    fn comment_is_user_friendly_not_form_headers() {
        let body = render_comment(&base_decision());
        assert!(is_bot_comment(&body));
        assert!(body.contains("谢谢") || body.contains("你好"));
        assert!(body.contains("retry") || body.contains("你好"));
        assert!(!body.contains("**定位：**"));
        assert!(!body.contains("**原因：**"));
        assert!(!body.contains("**已核实**"));
        assert!(!body.contains("### "));
        assert!(!body.contains("error_language"));
    }

    #[test]
    fn comment_includes_cc_mentions() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::NeedsInfo;
        d.verification_ran = false;
        d.code_hits.clear();
        let cfg = MentionConfig {
            on_needs_info: vec!["alice".into(), "@bob".into()],
            ..Default::default()
        };
        let body = render_comment_with_mentions(&d, &cfg);
        assert!(body.contains("cc @alice @bob"), "{body}");
        assert!(body.starts_with(BOT_COMMENT_MARKER));
        assert!(!body.contains("**定位：**"));
    }
}
