//! 聚合裁决：规则优先，技术验证可升级/降级结论。

use super::classify::Classification;
use super::completeness::CompletenessResult;
use super::duplicate::DuplicateResult;
use super::model::{IssueReviewDecision, IssueType, IssueVerdict};
use super::safety::SafetyScores;
use super::verify::TechnicalVerification;

pub fn judge(
    issue_number: u64,
    classification: &Classification,
    completeness: &CompletenessResult,
    safety: &SafetyScores,
    duplicate: &DuplicateResult,
    technical: Option<&TechnicalVerification>,
) -> IssueReviewDecision {
    let mut reasons = Vec::new();
    reasons.extend(classification.reasons.iter().cloned());
    reasons.extend(safety.reasons.iter().cloned());
    reasons.extend(duplicate.evidence.iter().cloned());

    let mut auto_action_allowed = false;

    let (mut verdict, mut confidence, needs_human, close_recommended) = if safety
        .advertisement_score
        >= 0.8
        || classification.primary_type == IssueType::Advertisement
    {
        reasons.push("high_advertisement_score".into());
        // 广告：高置信 + 规则命中才允许自动动作（仍需 policy 打开）
        auto_action_allowed = safety.advertisement_score >= 0.95;
        (
            IssueVerdict::Advertisement,
            safety.advertisement_score.max(0.8),
            true,
            auto_action_allowed,
        )
    } else if safety.spam_score >= 0.8 || classification.primary_type == IssueType::Spam {
        reasons.push("high_spam_score".into());
        auto_action_allowed = safety.spam_score >= 0.95 && safety.prompt_injection_score < 0.5;
        (
            IssueVerdict::Spam,
            safety.spam_score.max(0.8),
            true,
            auto_action_allowed,
        )
    } else if matches!(
        duplicate.status,
        super::model::DuplicateStatus::ExactDuplicate
            | super::model::DuplicateStatus::ProbableDuplicate
    ) && duplicate.confidence >= 0.8
    {
        reasons.push("duplicate_detected".into());
        // 重复：默认不关；极高置信 + 多源可建议关
        auto_action_allowed = duplicate.confidence >= 0.97
            && duplicate
                .candidates
                .first()
                .map(|c| c.sources.len() >= 2)
                .unwrap_or(false);
        (
            IssueVerdict::Duplicate,
            duplicate.confidence,
            false,
            auto_action_allowed,
        )
    } else if !completeness.missing_fields.is_empty()
        && completeness.score < 0.5
        && !completeness.can_verify
    {
        // 可检索线索已足够 Level-0 时，不因缺 expected/env 直接 NeedsInfo
        reasons.push("needs_more_info".into());
        (IssueVerdict::NeedsInfo, 0.7, false, false)
    } else if matches!(
        classification.primary_type,
        IssueType::Bug | IssueType::Security | IssueType::Performance | IssueType::Compatibility
    ) {
        if completeness.can_verify && classification.confidence >= 0.5 {
            reasons.push("likely_bug_from_triage".into());
            (
                IssueVerdict::LikelyBug,
                classification.confidence,
                false,
                false,
            )
        } else if completeness.can_verify {
            reasons.push("searchable_bug_unverified".into());
            (IssueVerdict::Unverified, 0.55, false, false)
        } else {
            (IssueVerdict::Unverified, 0.55, false, false)
        }
    } else if matches!(
        classification.primary_type,
        IssueType::FeatureRequest
            | IssueType::Documentation
            | IssueType::Question
            | IssueType::Support
    ) {
        // 这几类没有「代码缺陷」可验证，结论本身却是确定的。
        // 不能塞进 Unverified —— 那是「证据不足无法定性」，会用低分掩盖一个明确的分类结论。
        reasons.push("not_a_code_defect".into());
        (
            IssueVerdict::NotABug,
            classification.confidence,
            false,
            false,
        )
    } else {
        (
            IssueVerdict::Unverified,
            classification.confidence.max(0.4),
            true,
            false,
        )
    };

    let mut technical_verdict = IssueVerdict::Unverified;
    let mut technical_confidence = 0.0;
    let mut technical_evidence = Vec::new();
    let mut code_paths = Vec::new();
    let mut code_hits = Vec::new();
    let mut related_commits = Vec::new();
    let mut fix_prs = Vec::new();
    let mut verification_ran = false;

    if let Some(t) = technical {
        verification_ran = t.enabled;
        technical_verdict = t.technical_verdict;
        technical_confidence = t.confidence;
        technical_evidence = t.evidence.clone();
        code_paths = t.plan.likely_paths.clone();
        code_hits = t
            .code_hits
            .iter()
            .take(10)
            .map(|h| super::model::CodeEvidence {
                path: h.path.clone(),
                line: h.line,
                snippet: h.snippet.clone(),
            })
            .collect();
        if code_paths.is_empty() {
            code_paths = code_hits.iter().map(|h| h.path.clone()).take(8).collect();
            code_paths.sort();
            code_paths.dedup();
        }
        related_commits = t.git_commits.clone();
        fix_prs = t.fix_prs.clone();
        reasons.extend(t.evidence.iter().cloned());
        if let Some(skip) = &t.skipped_reason {
            reasons.push(format!("verify_skipped:{skip}"));
        }
        // 技术验证升级：仅当 triage 未判 spam/ad/dup
        if t.enabled
            && !matches!(
                verdict,
                IssueVerdict::Spam | IssueVerdict::Advertisement | IssueVerdict::Duplicate
            )
        {
            match t.technical_verdict {
                IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug => {
                    verdict = t.technical_verdict;
                    confidence = confidence.max(t.confidence);
                    reasons.push("technical_verification_upgrade".into());
                }
                IssueVerdict::AlreadyFixed => {
                    verdict = IssueVerdict::AlreadyFixed;
                    confidence = t.confidence;
                    reasons.push("technical_already_fixed".into());
                }
                IssueVerdict::Regression => {
                    verdict = IssueVerdict::Regression;
                    confidence = t.confidence;
                    reasons.push("technical_regression".into());
                }
                IssueVerdict::NotABug => {
                    verdict = IssueVerdict::NotABug;
                    confidence = t.confidence;
                }
                _ => {
                    confidence = (confidence + t.confidence) / 2.0;
                }
            }
        }
    }

    let mut suggested_labels = vec!["reviewgate-reviewed".to_string()];
    match verdict {
        IssueVerdict::NeedsInfo => suggested_labels.push("needs-info".into()),
        IssueVerdict::Duplicate => suggested_labels.push("possible-duplicate".into()),
        IssueVerdict::Spam => suggested_labels.push("spam".into()),
        IssueVerdict::Advertisement => suggested_labels.push("advertisement".into()),
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug => {
            suggested_labels.push("bug".into());
        }
        IssueVerdict::AlreadyFixed => suggested_labels.push("already-fixed".into()),
        IssueVerdict::Regression => suggested_labels.push("regression".into()),
        IssueVerdict::NotABug if classification.primary_type == IssueType::FeatureRequest => {
            suggested_labels.push("feature-request".into());
        }
        IssueVerdict::NotABug if classification.primary_type == IssueType::Question => {
            suggested_labels.push("question".into());
        }
        _ => {}
    }

    IssueReviewDecision {
        issue_number,
        primary_type: classification.primary_type,
        type_confidence: classification.confidence,
        type_reasons: classification.reasons.clone(),
        completeness_score: completeness.score,
        missing_fields: completeness.missing_fields.clone(),
        can_verify: completeness.can_verify,
        spam_score: safety.spam_score,
        advertisement_score: safety.advertisement_score,
        abuse_score: safety.abuse_score,
        prompt_injection_score: safety.prompt_injection_score,
        duplicate_status: duplicate.status,
        duplicate_confidence: duplicate.confidence,
        duplicate_of: duplicate.duplicate_of,
        duplicate_candidates: duplicate.candidates.clone(),
        duplicate_evidence: duplicate.evidence.clone(),
        verdict,
        confidence,
        reasons,
        suggested_labels,
        suggested_comment: String::new(),
        close_recommended,
        auto_action_allowed,
        needs_human_review: needs_human || safety.prompt_injection_score >= 0.5,
        vector_used: duplicate.vector_used,
        vector_degraded: duplicate.vector_degraded,
        analyzer_version: IssueReviewDecision::analyzer_version().into(),
        technical_verdict,
        technical_confidence,
        technical_evidence,
        code_paths,
        code_hits,
        related_commits,
        fix_prs,
        verification_ran,
        issue_title: String::new(),
        symptom_summary: String::new(),
        misrouted_repos: vec![],
        misrouted_confidence: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::classify::classify_heuristic;
    use crate::issue::completeness::check_completeness;
    use crate::issue::duplicate::DuplicateResult;
    use crate::issue::model::DuplicateStatus;
    use crate::issue::normalize::normalize_issue;
    use crate::issue::safety::score_safety;

    fn judge_issue(title: &str, body: &str) -> IssueReviewDecision {
        let normalized = normalize_issue(title, body);
        let safety = score_safety(title, body);
        let classification = classify_heuristic(title, body, &safety);
        let completeness = check_completeness(classification.primary_type, &normalized);
        let duplicate = DuplicateResult {
            status: DuplicateStatus::NotDuplicate,
            confidence: 0.7,
            duplicate_of: None,
            candidates: vec![],
            evidence: vec![],
            vector_used: false,
            vector_degraded: false,
        };
        judge(1, &classification, &completeness, &safety, &duplicate, None)
    }

    /// 线上回归：atomgit#2「希望添加最佳实践」被兜底成 UNVERIFIED 40%。
    /// 文档诉求这个结论本身是确定的，只是无从做代码验证——不该用「证据不足」的低分表达。
    #[test]
    fn documentation_ask_is_not_a_bug_not_unverified() {
        let d = judge_issue("docs：希望添加最佳实践", "如题，希望官方提供一些最佳实践");
        assert_eq!(d.primary_type, IssueType::Documentation);
        assert_eq!(d.verdict, IssueVerdict::NotABug);
        assert!(d.confidence >= 0.6, "got {}", d.confidence);
    }

    #[test]
    fn unclassifiable_issue_stays_unverified() {
        let d = judge_issue("这个", "帮忙看看这个东西对不对，谢谢了非常感谢");
        assert_eq!(d.verdict, IssueVerdict::Unverified);
    }

    #[test]
    fn crash_report_still_reaches_bug_path() {
        let d = judge_issue(
            "保存时崩溃 panic",
            "点击保存后 panic on save，版本 5.0.3，macOS 15，必现，期望正常保存",
        );
        assert_eq!(d.primary_type, IssueType::Bug);
        assert_ne!(d.verdict, IssueVerdict::NotABug);
    }
}
