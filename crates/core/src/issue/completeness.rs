//! Issue 信息完整度检查。

use super::model::{IssueType, NormalizedIssue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessResult {
    pub score: f32,
    pub missing_fields: Vec<String>,
    pub can_verify: bool,
}

/// 按类型评估完整度。
pub fn check_completeness(issue_type: IssueType, n: &NormalizedIssue) -> CompletenessResult {
    let mut missing = Vec::new();
    let mut present = 0usize;
    let mut total = 0usize;

    let mut need = |name: &str, ok: bool| {
        total += 1;
        if ok {
            present += 1;
        } else {
            missing.push(name.to_string());
        }
    };

    match issue_type {
        // 漏洞报告要的是「影响什么、怎么触发」，不是报错日志和本机环境。
        IssueType::Security => {
            need(
                "actual_behavior",
                !n.actual_behavior.is_empty() || !n.symptom.is_empty(),
            );
            need(
                "affected_scope",
                !n.environment.is_null() || !n.body_clean.is_empty(),
            );
            need("reproduction_steps", !n.reproduction_steps.is_empty());
        }
        IssueType::Bug | IssueType::Compatibility => {
            need(
                "actual_behavior",
                !n.actual_behavior.is_empty() || !n.symptom.is_empty(),
            );
            need("expected_behavior", !n.expected_behavior.is_empty());
            need("reproduction_steps", !n.reproduction_steps.is_empty());
            need(
                "error_or_log",
                !n.error_signatures.is_empty()
                    || n.body_clean.to_ascii_lowercase().contains("error"),
            );
            need(
                "environment",
                n.environment
                    .as_object()
                    .map(|o| !o.is_empty())
                    .unwrap_or(false),
            );
        }
        IssueType::Performance => {
            need("symptom", !n.symptom.is_empty());
            need(
                "environment",
                n.environment
                    .as_object()
                    .map(|o| !o.is_empty())
                    .unwrap_or(false),
            );
            need("reproduction_steps", !n.reproduction_steps.is_empty());
        }
        IssueType::Spam | IssueType::Advertisement | IssueType::Abuse => {
            return CompletenessResult {
                score: 1.0,
                missing_fields: vec![],
                can_verify: false,
            };
        }
        _ => {
            need("description", n.body_clean.len() >= 20);
            need("title", n.title.len() >= 5);
        }
    }

    let score = if total == 0 {
        0.5
    } else {
        present as f32 / total as f32
    };
    // 标题/正文已点名模块或错误时，即使缺 expected/env 仍允许 Level-0 检索
    let searchable = matches!(
        issue_type,
        IssueType::Bug | IssueType::Security | IssueType::Performance | IssueType::Compatibility
    ) && has_searchable_clues(n);
    CompletenessResult {
        can_verify: (score >= 0.6 || searchable)
            && !matches!(issue_type, IssueType::Spam | IssueType::Advertisement),
        score,
        missing_fields: missing,
    }
}

/// 是否已有足够线索做代码检索（不替代完整度分数，只放行 verify）。
pub fn has_searchable_clues(n: &NormalizedIssue) -> bool {
    if !n.error_signatures.is_empty() {
        return true;
    }
    if !n.stack_symbols.is_empty() {
        return true;
    }
    let blob = format!("{} {}", n.title, n.symptom).to_ascii_lowercase();
    // 路径/crate/工具名等可检索实体
    const CLUES: &[&str] = &[
        "skill",
        "dedup",
        "request_user_input",
        "webui",
        "sync",
        "retry",
        "oauth",
        "acp",
        "memory",
        "provider",
        "openai",
        "stream",
        "gateway",
        "tui",
        "daemon",
        "atomgit",
        "app",
        "口令",
        "重试",
        "连接",
        "网络",
        "crates/",
        "mod.rs",
        "/skills",
    ];
    CLUES.iter().any(|c| blob.contains(c))
        || n.title
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .any(|t| t.len() >= 6 && t.contains('_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::normalize::normalize_issue;

    #[test]
    fn incomplete_bug_flags_missing() {
        let n = normalize_issue("crash", "it crashes");
        let c = check_completeness(IssueType::Bug, &n);
        assert!(c.score < 0.8);
        assert!(!c.missing_fields.is_empty());
    }

    #[test]
    fn titled_module_bug_can_verify_despite_low_completeness() {
        let n = normalize_issue(
            "bug: 同名 skill 不同目录时去重逻辑误删全部而非保留一个 — /skills dedup-skill",
            "两个目录同 name frontmatter 时 /skills 为空",
        );
        let c = check_completeness(IssueType::Bug, &n);
        assert!(
            c.can_verify,
            "searchable bug must allow verify: score={} missing={:?}",
            c.score, c.missing_fields
        );
        assert!(has_searchable_clues(&n));
    }
}
