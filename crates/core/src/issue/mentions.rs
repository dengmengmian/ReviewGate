//! 评论 @mention 解析：按结论/类型决定拉谁看。
//!
//! 有结论时只 @ 不指派——抄送若干人却把 Issue 挂到某个人名下会造成假的责任归属。
//! 唯一例外是转人工：那时结论出不来，必须有明确的人接手，见 `on_needs_triage`。

use super::model::{IssueType, IssueVerdict};
use serde::{Deserialize, Serialize};

/// 评论里要 @ 的人（配置为 login，渲染时统一加 `@`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MentionConfig {
    /// 任意结论都会附加（spam/ad 仍遵守下方专用列表优先）。
    pub default: Vec<String>,
    pub on_needs_info: Vec<String>,
    pub on_probable_duplicate: Vec<String>,
    pub on_security: Vec<String>,
    pub on_likely_bug: Vec<String>,
    pub on_confirmed_bug: Vec<String>,
    pub on_regression: Vec<String>,
    pub on_already_fixed: Vec<String>,
    pub on_spam: Vec<String>,
    pub on_advertisement: Vec<String>,
    pub on_question: Vec<String>,
    pub on_feature_request: Vec<String>,
    /// 置信度不足、机器人不下结论时的指定处理人。空 = 不转人工，静默跳过。
    pub on_needs_triage: Vec<String>,
}

/// 低置信度时该叫谁来看。与 `resolve_mentions` 分开：那是「结论出来了 cc 谁」，
/// 这是「结论出不来，交给谁」。
pub fn resolve_triage_owners(cfg: &MentionConfig) -> Vec<String> {
    let mut out = Vec::new();
    push_all(&mut out, &cfg.on_needs_triage);
    normalize_logins(out)
}

/// 根据决策解析应 mention 的 login 列表（去重、保序）。
pub fn resolve_mentions(
    cfg: &MentionConfig,
    verdict: IssueVerdict,
    primary_type: IssueType,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // spam / ad：只用专用列表，不叠加 default（避免误 @ 维护者）
    if matches!(verdict, IssueVerdict::Spam) {
        push_all(&mut out, &cfg.on_spam);
        return normalize_logins(out);
    }
    if matches!(verdict, IssueVerdict::Advertisement) {
        push_all(&mut out, &cfg.on_advertisement);
        return normalize_logins(out);
    }

    push_all(&mut out, &cfg.default);

    match verdict {
        IssueVerdict::NeedsInfo => push_all(&mut out, &cfg.on_needs_info),
        IssueVerdict::Duplicate => push_all(&mut out, &cfg.on_probable_duplicate),
        IssueVerdict::LikelyBug => push_all(&mut out, &cfg.on_likely_bug),
        IssueVerdict::ConfirmedBug => push_all(&mut out, &cfg.on_confirmed_bug),
        IssueVerdict::Regression => push_all(&mut out, &cfg.on_regression),
        IssueVerdict::AlreadyFixed => push_all(&mut out, &cfg.on_already_fixed),
        IssueVerdict::NotABug if primary_type == IssueType::Question => {
            push_all(&mut out, &cfg.on_question);
        }
        IssueVerdict::NotABug if primary_type == IssueType::FeatureRequest => {
            push_all(&mut out, &cfg.on_feature_request);
        }
        _ => {}
    }

    // 类型侧补充：security 即使结论还是 LIKELY_BUG 也要拉安全
    if primary_type == IssueType::Security {
        push_all(&mut out, &cfg.on_security);
    }

    normalize_logins(out)
}

/// 格式化为评论行，如 `cc @alice @bob`；空则返回 None。
pub fn format_mention_line(logins: &[String]) -> Option<String> {
    if logins.is_empty() {
        return None;
    }
    let parts: Vec<String> = logins.iter().map(|l| format!("@{l}")).collect();
    Some(format!("cc {}", parts.join(" ")))
}

fn push_all(out: &mut Vec<String>, items: &[String]) {
    for s in items {
        let t = s.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
}

fn normalize_logins(raw: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for s in raw {
        let login = s.trim().trim_start_matches('@').trim().to_string();
        if login.is_empty() {
            continue;
        }
        // 拒绝明显非法（空格、注入式片段）
        if login.split_whitespace().count() != 1 {
            continue;
        }
        if !seen.insert(login.to_ascii_lowercase()) {
            continue;
        }
        out.push(login);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spam_skips_default() {
        let cfg = MentionConfig {
            default: vec!["triage".into()],
            on_spam: vec![],
            ..Default::default()
        };
        assert!(resolve_mentions(&cfg, IssueVerdict::Spam, IssueType::Spam).is_empty());
    }

    #[test]
    fn needs_info_merges_default_and_specific() {
        let cfg = MentionConfig {
            default: vec!["@triage".into()],
            on_needs_info: vec!["alice".into(), "triage".into()],
            ..Default::default()
        };
        let m = resolve_mentions(&cfg, IssueVerdict::NeedsInfo, IssueType::Bug);
        assert_eq!(m, vec!["triage".to_string(), "alice".to_string()]);
        let line = format_mention_line(&m).unwrap();
        assert_eq!(line, "cc @triage @alice");
    }

    #[test]
    fn security_type_adds_on_security() {
        let cfg = MentionConfig {
            on_likely_bug: vec!["dev".into()],
            on_security: vec!["sec-oncall".into()],
            ..Default::default()
        };
        let m = resolve_mentions(&cfg, IssueVerdict::LikelyBug, IssueType::Security);
        assert!(m.contains(&"dev".to_string()));
        assert!(m.contains(&"sec-oncall".to_string()));
    }

    #[test]
    fn triage_owners_are_independent_of_verdict_routing() {
        let cfg = MentionConfig {
            default: vec!["everyone".into()],
            on_needs_triage: vec!["@alice".into(), "alice".into(), "bob".into()],
            ..Default::default()
        };
        // 去重、去 @、不叠加 default——转人工要的是明确的责任人，不是抄送名单
        assert_eq!(
            resolve_triage_owners(&cfg),
            vec!["alice".to_string(), "bob".to_string()]
        );
    }

    #[test]
    fn no_triage_owner_configured_means_no_handoff() {
        assert!(resolve_triage_owners(&MentionConfig::default()).is_empty());
    }
}
