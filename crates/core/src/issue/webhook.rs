//! Webhook 解析、签名校验与事件元信息提取。

use anyhow::{bail, Result};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// 校验 GitHub `X-Hub-Signature-256: sha256=<hex>`。
pub fn verify_github_signature(secret: &str, body: &[u8], header: &str) -> Result<()> {
    let header = header.trim();
    let hex = header
        .strip_prefix("sha256=")
        .ok_or_else(|| anyhow::anyhow!("signature must start with sha256="))?;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
    mac.update(body);
    let got = hex::decode(hex).map_err(|e| anyhow::anyhow!("bad signature hex: {e}"))?;
    mac.verify_slice(&got)
        .map_err(|_| anyhow::anyhow!("webhook signature mismatch"))?;
    Ok(())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.as_bytes().iter().zip(b.as_bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// GitLab token 头：先比长度，再常量时间比对。
pub fn verify_gitlab_token(secret: &str, header: Option<&str>) -> Result<()> {
    match header {
        Some(h) if constant_time_eq(h, secret) => Ok(()),
        _ => bail!("gitlab token mismatch"),
    }
}

#[derive(Debug, Clone)]
pub struct ParsedWebhook {
    pub delivery_id: String,
    pub event_type: String,
    pub action: String,
    pub repo_id: String,
    pub issue_number: Option<u64>,
    /// 是否需要完整 triage（opened/edited/reopened/comment）。
    pub needs_full_review: bool,
    /// 是否为机器人自身事件（应忽略）。
    pub is_bot_loop: bool,
}

/// 从 GitHub webhook JSON 提取元信息。
pub fn parse_github_event(
    event_header: &str,
    delivery_id: &str,
    body: &str,
    bot_logins: &[&str],
) -> Result<ParsedWebhook> {
    let v: Value = serde_json::from_str(body)?;
    let action = v
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("")
        .to_string();
    let repo_id = v
        .pointer("/repository/full_name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let issue_number = v
        .pointer("/issue/number")
        .or_else(|| v.pointer("/pull_request/number"))
        .and_then(|x| x.as_u64());

    let sender_login = v
        .pointer("/sender/login")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let sender_type = v
        .pointer("/sender/type")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let is_bot_loop = sender_type.eq_ignore_ascii_case("Bot")
        && bot_logins
            .iter()
            .any(|b| sender_login.eq_ignore_ascii_case(b));

    // comment body 若是我们自己的 marker，也忽略
    let comment_bot = v
        .pointer("/comment/body")
        .and_then(|x| x.as_str())
        .map(super::comment::is_bot_comment)
        .unwrap_or(false);

    let is_pr = v.pointer("/issue/pull_request").is_some() || v.get("pull_request").is_some();
    let event = event_header.to_ascii_lowercase();
    let mut needs_full_review = match event.as_str() {
        "issues" => matches!(action.as_str(), "opened" | "edited" | "reopened"),
        "issue_comment" => {
            matches!(action.as_str(), "created" | "edited") && !comment_bot && !is_bot_loop
        }
        _ => false,
    };
    if is_pr {
        needs_full_review = false;
    }

    Ok(ParsedWebhook {
        delivery_id: if delivery_id.is_empty() {
            format!("gen-{}", super::hash::content_hash(body, event_header))
        } else {
            delivery_id.to_string()
        },
        event_type: event_header.to_string(),
        action,
        repo_id,
        issue_number,
        needs_full_review,
        is_bot_loop: is_bot_loop || comment_bot,
    })
}

/// GitLab issue event 解析（简化）。只覆盖 Issue Hook，不解析 Note Hook。
pub fn parse_gitlab_event(
    body: &str,
    delivery_id: &str,
    bot_logins: &[&str],
) -> Result<ParsedWebhook> {
    let v: Value = serde_json::from_str(body)?;
    let object_kind = v
        .get("object_kind")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let action = v
        .pointer("/object_attributes/action")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let path = v
        .pointer("/project/path_with_namespace")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let iid = v.pointer("/object_attributes/iid").and_then(|x| x.as_u64());
    let username = v
        .pointer("/user/username")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let is_bot_loop =
        object_kind == "issue" && bot_logins.iter().any(|b| username.eq_ignore_ascii_case(b));
    let needs = object_kind == "issue"
        && matches!(action.as_str(), "open" | "reopen" | "update")
        && !is_bot_loop;
    Ok(ParsedWebhook {
        delivery_id: if delivery_id.is_empty() {
            format!("gl-{}", super::hash::content_hash(body, &object_kind))
        } else {
            delivery_id.to_string()
        },
        event_type: object_kind,
        action,
        repo_id: path,
        issue_number: iid,
        needs_full_review: needs,
        is_bot_loop,
    })
}

/// drain 用：按 payload 重算要不要 triage。`closed` / PR 评论 / labeled 为 false。
pub fn payload_needs_full_review(event_type: &str, payload: &str, bot_logins: &[&str]) -> bool {
    let ev = event_type.to_ascii_lowercase();
    if ev == "issue" || ev == "note" {
        parse_gitlab_event(payload, "-", bot_logins)
            .map(|p| p.needs_full_review)
            .unwrap_or(false)
    } else {
        parse_github_event(event_type, "-", payload, bot_logins)
            .map(|p| p.needs_full_review)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_roundtrip() {
        let secret = "whsec";
        let body = br#"{"action":"opened"}"#;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        verify_github_signature(secret, body, &sig).unwrap();
        assert!(verify_github_signature(secret, body, "sha256=dead").is_err());
    }

    #[test]
    fn parse_opened_issue() {
        let body = r#"{
          "action":"opened",
          "issue":{"number":42},
          "repository":{"full_name":"acme/app"},
          "sender":{"login":"alice","type":"User"}
        }"#;
        let p = parse_github_event("issues", "del-1", body, &["reviewgate[bot]"]).unwrap();
        assert_eq!(p.issue_number, Some(42));
        assert!(p.needs_full_review);
        assert!(!p.is_bot_loop);
    }

    #[test]
    fn ignore_bot_comment_loop() {
        let body = format!(
            r#"{{
          "action":"created",
          "issue":{{"number":1}},
          "repository":{{"full_name":"a/b"}},
          "sender":{{"login":"reviewgate[bot]","type":"Bot"}},
          "comment":{{"body":"{}"}}
        }}"#,
            super::super::model::BOT_COMMENT_MARKER
        );
        let p = parse_github_event("issue_comment", "d2", &body, &["reviewgate[bot]"]).unwrap();
        assert!(p.is_bot_loop);
    }

    #[test]
    fn pull_request_payload_is_not_full_review() {
        let body = r#"{
          "action":"opened",
          "issue":{"number":9,"pull_request":{"url":"https://api/pulls/9"}},
          "repository":{"full_name":"acme/app"},
          "sender":{"login":"alice","type":"User"}
        }"#;
        let p = parse_github_event("issues", "d3", body, &["reviewgate[bot]"]).unwrap();
        assert!(!p.needs_full_review);
        assert!(!p.is_bot_loop);
    }

    #[test]
    fn gitlab_bot_author_on_issue_hook_is_ignored() {
        let body = r#"{
          "object_kind":"issue",
          "object_attributes":{"action":"open","iid":3},
          "project":{"path_with_namespace":"g/p"},
          "user":{"username":"reviewgate-bot"}
        }"#;
        let p = parse_gitlab_event(body, "g1", &["reviewgate-bot"]).unwrap();
        assert!(p.is_bot_loop);
        assert!(!p.needs_full_review);
    }
}
