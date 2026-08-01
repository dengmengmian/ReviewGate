//! Action Policy：低风险默认可开；高风险关闭需双闸（policy + auto_action_allowed）。

use super::model::{IssueReviewDecision, IssueVerdict};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPolicy {
    pub comment: bool,
    pub update_existing_comment: bool,
    pub add_labels: bool,
    pub close_issue: bool,
    pub close_spam: bool,
    pub close_invalid: bool,
    pub close_duplicate: bool,
    /// 仅允许添加的「安全」标签（若空且 add_labels=true 则用 decision.suggested_labels 全量）。
    #[serde(default)]
    pub safe_labels_only: bool,
    /// 结论置信度低于该阈值时不对外发言、不关闭，只保留打标签。
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    /// 转人工时是否同时指派给处理人。配了处理人就说明责任归属明确，默认开。
    #[serde(default = "default_true_bool")]
    pub assign_on_triage: bool,
}

fn default_min_confidence() -> f32 {
    0.5
}

fn default_true_bool() -> bool {
    true
}

impl Default for ActionPolicy {
    fn default() -> Self {
        Self {
            comment: true,
            update_existing_comment: true,
            add_labels: false,
            close_issue: false,
            close_spam: false,
            close_invalid: false,
            close_duplicate: false,
            safe_labels_only: true,
            min_confidence: default_min_confidence(),
            assign_on_triage: true,
        }
    }
}

/// 默认允许自动添加的低风险标签。
pub const SAFE_LABELS: &[&str] = &[
    "needs-info",
    "question",
    "feature-request",
    "possible-duplicate",
    "reviewgate-reviewed",
    "bug",
    "already-fixed",
    "regression",
    "needs-triage",
    "possible-wrong-repo",
];

/// 低置信度跳过时贴的流程标签：让被跳过的 Issue 可被筛出来，而不是静默消失。
pub const TRIAGE_LABEL: &str = "needs-triage";

/// 疑似错投仓库的标签。只提示、不关闭——判错的代价由反馈者承担，必须留人工决定。
pub const WRONG_REPO_LABEL: &str = "possible-wrong-repo";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedActions {
    pub post_or_update_comment: bool,
    pub labels_to_add: Vec<String>,
    pub close: bool,
    pub close_reason: Option<String>,
    pub reasons_blocked: Vec<String>,
    /// 这条评论是「交给人看」而非「给出结论」，正文该用移交话术。
    #[serde(default)]
    pub needs_human_notice: bool,
    /// 转人工时要指派的处理人 login。@ 只是一条会被淹没的通知，
    /// 指派才能让这条 Issue 在列表里按「谁负责」筛出来。
    #[serde(default)]
    pub assign_to: Option<String>,
}

/// 根据策略与决策规划可执行动作。
///
/// `has_triage_owner`：是否配置了低置信度的指定处理人。配了就把这条移交给他，
/// 没配就保持静默——闸门拦的是「机器人下结论」，不是「叫人来看」。
pub fn plan_actions(
    policy: &ActionPolicy,
    decision: &IssueReviewDecision,
    triage_owner: Option<&str>,
) -> PlannedActions {
    let has_triage_owner = triage_owner.is_some();
    let mut blocked = Vec::new();
    // 结论没把握就别对外说话：低置信度只留打标签，人来接手。
    let confident = decision.confidence >= policy.min_confidence;
    if !confident {
        blocked.push(format!(
            "low_confidence:{:.2}<{:.2}",
            decision.confidence, policy.min_confidence
        ));
    }
    let handoff = !confident && has_triage_owner;
    let post = policy.comment && (confident || handoff);
    if !policy.comment {
        blocked.push("comment_disabled".into());
    }
    if !confident && !has_triage_owner {
        blocked.push("no_triage_owner".into());
    }

    let mut labels = Vec::new();
    if policy.add_labels {
        // 不发言不等于当没发生过：留一个可筛选的标记，人才知道这条待接手。
        if !confident {
            labels.push(TRIAGE_LABEL.to_string());
        }
        for l in &decision.suggested_labels {
            if policy.safe_labels_only {
                // spam/advertisement 标签默认不自动加，除非 close_spam 策略打开
                if (l == "spam" || l == "advertisement") && !policy.close_spam {
                    blocked.push(format!("label_blocked:{l}"));
                    continue;
                }
                if SAFE_LABELS.contains(&l.as_str())
                    || (policy.close_spam && (l == "spam" || l == "advertisement"))
                {
                    labels.push(l.clone());
                }
            } else {
                labels.push(l.clone());
            }
        }
    } else {
        blocked.push("add_labels_disabled".into());
    }

    let mut close = false;
    let mut close_reason = None;
    if decision.close_recommended {
        let allow = match decision.verdict {
            IssueVerdict::Spam | IssueVerdict::Advertisement => {
                policy.close_spam || policy.close_issue
            }
            IssueVerdict::Duplicate => policy.close_duplicate || policy.close_issue,
            _ => policy.close_issue,
        };
        if allow && decision.auto_action_allowed && confident {
            close = true;
            close_reason = Some(decision.verdict.as_str().to_string());
        } else if allow && !decision.auto_action_allowed {
            blocked.push("auto_action_not_allowed".into());
        } else {
            blocked.push("close_disabled_by_policy".into());
        }
    }

    PlannedActions {
        post_or_update_comment: post,
        labels_to_add: labels,
        close,
        close_reason,
        reasons_blocked: blocked,
        needs_human_notice: handoff && policy.comment,
        assign_to: if handoff && policy.assign_on_triage {
            triage_owner.map(str::to_string)
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::model::*;

    fn dec(
        v: IssueVerdict,
        close_rec: bool,
        auto: bool,
        labels: Vec<String>,
    ) -> IssueReviewDecision {
        IssueReviewDecision {
            issue_number: 1,
            primary_type: IssueType::Spam,
            type_confidence: 0.99,
            type_reasons: vec![],
            completeness_score: 1.0,
            missing_fields: vec![],
            can_verify: false,
            spam_score: 0.99,
            advertisement_score: 0.0,
            abuse_score: 0.0,
            prompt_injection_score: 0.0,
            duplicate_status: DuplicateStatus::NotDuplicate,
            duplicate_confidence: 0.5,
            duplicate_of: None,
            duplicate_candidates: vec![],
            duplicate_evidence: vec![],
            verdict: v,
            confidence: 0.99,
            reasons: vec![],
            suggested_labels: labels,
            suggested_comment: String::new(),
            close_recommended: close_rec,
            auto_action_allowed: auto,
            needs_human_review: true,
            vector_used: false,
            vector_degraded: false,
            analyzer_version: "t".into(),
            technical_verdict: IssueVerdict::Unverified,
            technical_confidence: 0.0,
            technical_evidence: vec![],
            code_paths: vec![],
            fix_prs: vec![],
            verification_ran: false,
            code_hits: vec![],
            related_commits: vec![],
            issue_title: String::new(),
            symptom_summary: String::new(),
            misrouted_repos: vec![],
            misrouted_confidence: 0.0,
        }
    }

    #[test]
    fn default_policy_never_closes() {
        let p = ActionPolicy::default();
        let plan = plan_actions(
            &p,
            &dec(
                IssueVerdict::Spam,
                true,
                true,
                vec!["spam".into(), "reviewgate-reviewed".into()],
            ),
            None,
        );
        assert!(!plan.close);
        assert!(plan.post_or_update_comment);
        assert!(plan.labels_to_add.is_empty());
    }

    #[test]
    fn high_confidence_spam_close_when_enabled() {
        let p = ActionPolicy {
            close_spam: true,
            add_labels: true,
            ..Default::default()
        };
        let plan = plan_actions(
            &p,
            &dec(
                IssueVerdict::Spam,
                true,
                true,
                vec!["spam".into(), "reviewgate-reviewed".into()],
            ),
            None,
        );
        assert!(plan.close);
        assert!(plan.labels_to_add.contains(&"reviewgate-reviewed".into()));
        assert!(plan.labels_to_add.contains(&"spam".into()));
    }

    /// 线上回归：atomgit#2 的结论置信度只有 40%，仍然直接公开发了评论。
    /// 低置信度只允许打标签，不允许对外发言/关闭。
    #[test]
    fn low_confidence_blocks_comment_but_keeps_labels() {
        let p = ActionPolicy {
            add_labels: true,
            ..Default::default()
        };
        let mut d = dec(
            IssueVerdict::Unverified,
            false,
            false,
            vec!["reviewgate-reviewed".into()],
        );
        d.confidence = 0.4;
        let plan = plan_actions(&p, &d, None);
        assert!(!plan.post_or_update_comment, "40% must not speak publicly");
        assert!(plan.labels_to_add.contains(&"reviewgate-reviewed".into()));
        assert!(plan
            .reasons_blocked
            .iter()
            .any(|r| r.starts_with("low_confidence")));
    }

    /// 借鉴需求文档 1.3「人工兜底」：低置信度不执行动作，但必须留痕。
    /// 我们原来是完全静默——issue 被跳过，维护者无从知道有东西待审。
    #[test]
    fn low_confidence_marks_issue_for_human_triage() {
        let p = ActionPolicy {
            add_labels: true,
            ..Default::default()
        };
        let mut d = dec(IssueVerdict::Unverified, false, false, vec![]);
        d.confidence = 0.4;
        let plan = plan_actions(&p, &d, None);
        assert!(
            plan.labels_to_add.contains(&"needs-triage".to_string()),
            "skipped issues must be findable: {:?}",
            plan.labels_to_add
        );
    }

    #[test]
    fn confident_issue_gets_no_triage_label() {
        let p = ActionPolicy {
            add_labels: true,
            ..Default::default()
        };
        let plan = plan_actions(
            &p,
            &dec(IssueVerdict::NeedsInfo, false, false, vec![]),
            None,
        );
        assert!(!plan.labels_to_add.contains(&"needs-triage".to_string()));
    }

    #[test]
    fn confidence_at_threshold_still_comments() {
        let p = ActionPolicy::default();
        let mut d = dec(IssueVerdict::NeedsInfo, false, false, vec![]);
        d.confidence = p.min_confidence;
        assert!(plan_actions(&p, &d, None).post_or_update_comment);
    }

    /// 转人工的本意是「叫人来看」，不是「什么都不做」。
    /// 结论性回复该拦，但求助性回复必须放行，否则该来的人永远收不到通知。
    #[test]
    fn low_confidence_with_a_triage_owner_still_speaks() {
        let p = ActionPolicy::default();
        let mut d = dec(IssueVerdict::Unverified, false, false, vec![]);
        d.confidence = 0.4;
        let plan = plan_actions(&p, &d, Some("alice"));
        assert!(plan.post_or_update_comment, "hand-off must reach the issue");
        assert!(
            plan.needs_human_notice,
            "and it must be the hand-off wording"
        );
        assert!(!plan.close, "still never closes on low confidence");
    }

    #[test]
    fn low_confidence_without_owner_stays_silent() {
        let p = ActionPolicy::default();
        let mut d = dec(IssueVerdict::Unverified, false, false, vec![]);
        d.confidence = 0.4;
        let plan = plan_actions(&p, &d, None);
        assert!(!plan.post_or_update_comment);
        assert!(!plan.needs_human_notice);
    }

    #[test]
    fn confident_issue_never_uses_handoff_wording() {
        let p = ActionPolicy::default();
        let d = dec(IssueVerdict::NeedsInfo, false, false, vec![]);
        let plan = plan_actions(&p, &d, Some("alice"));
        assert!(plan.post_or_update_comment);
        assert!(
            !plan.needs_human_notice,
            "a confident verdict answers directly"
        );
    }

    /// 关掉评论就是关掉评论，转人工不能绕过它。
    #[test]
    fn comment_disabled_blocks_handoff_too() {
        let p = ActionPolicy {
            comment: false,
            ..Default::default()
        };
        let mut d = dec(IssueVerdict::Unverified, false, false, vec![]);
        d.confidence = 0.4;
        assert!(!plan_actions(&p, &d, Some("alice")).post_or_update_comment);
    }

    /// 关广告是低风险动作，不该被迫打开能关任意 Issue 的总闸。
    #[test]
    fn spam_can_be_closed_without_the_master_switch() {
        let p = ActionPolicy {
            close_spam: true,
            ..Default::default()
        };
        assert!(!p.close_issue, "master switch stays off");
        let plan = plan_actions(
            &p,
            &dec(IssueVerdict::Advertisement, true, true, vec![]),
            None,
        );
        assert!(plan.close, "an ad with auto_action_allowed should close");
        assert_eq!(plan.close_reason.as_deref(), Some("ADVERTISEMENT"));
    }

    /// 但普通缺陷不会因为开了 close_spam 就被关掉。
    #[test]
    fn close_spam_does_not_close_ordinary_issues() {
        let p = ActionPolicy {
            close_spam: true,
            ..Default::default()
        };
        let plan = plan_actions(&p, &dec(IssueVerdict::LikelyBug, true, true, vec![]), None);
        assert!(!plan.close, "{:?}", plan.reasons_blocked);
    }

    /// 转人工要落到「谁负责」上：@ 是会被淹没的通知，指派才进 Issue 列表的筛选。
    #[test]
    fn handoff_assigns_the_triage_owner() {
        let p = ActionPolicy::default();
        let mut d = dec(IssueVerdict::Unverified, false, false, vec![]);
        d.confidence = 0.4;
        let plan = plan_actions(&p, &d, Some("alice"));
        assert_eq!(plan.assign_to.as_deref(), Some("alice"));
    }

    #[test]
    fn assignment_can_be_turned_off() {
        let p = ActionPolicy {
            assign_on_triage: false,
            ..Default::default()
        };
        let mut d = dec(IssueVerdict::Unverified, false, false, vec![]);
        d.confidence = 0.4;
        let plan = plan_actions(&p, &d, Some("alice"));
        assert!(plan.assign_to.is_none());
        assert!(plan.needs_human_notice, "still hands off in the comment");
    }

    #[test]
    fn confident_issue_is_never_assigned() {
        let p = ActionPolicy::default();
        let d = dec(IssueVerdict::NeedsInfo, false, false, vec![]);
        assert!(plan_actions(&p, &d, Some("alice")).assign_to.is_none());
    }

    #[test]
    fn low_confidence_never_closes() {
        let p = ActionPolicy {
            close_spam: true,
            ..Default::default()
        };
        let mut d = dec(IssueVerdict::Spam, true, true, vec![]);
        d.confidence = 0.3;
        let plan = plan_actions(&p, &d, None);
        assert!(!plan.close);
    }

    #[test]
    fn safe_labels_can_enable_without_close() {
        let p = ActionPolicy {
            add_labels: true,
            ..Default::default()
        };
        let plan = plan_actions(
            &p,
            &dec(
                IssueVerdict::NeedsInfo,
                false,
                false,
                vec!["needs-info".into(), "reviewgate-reviewed".into()],
            ),
            None,
        );
        assert!(!plan.close);
        assert!(plan.labels_to_add.contains(&"needs-info".into()));
    }
}
