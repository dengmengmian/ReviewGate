//! Deterministic secret / hardcoded-credential precheck (deep security profile).
//!
//! Pure scan over **added** diff lines — no LLM. Hits become high-severity
//! security findings. Designed for vibe-coding gates where secrets must not
//! depend on model variance.
//!
//! Implemented without the `regex` crate (keep core deps lean): token shapes use
//! prefix + charset scans; assignment forms use simple substring parsers.

use crate::diff::{Diff, LineKind};
use crate::model::{Dimension, Finding, Reachability, Severity};

/// Default confidence for deterministic secret hits (above default block threshold).
pub const SECRET_CONFIDENCE: f32 = 0.95;

/// Scan added lines in a diff for hardcoded secrets / credentials.
/// Empty diffs and clean code produce no findings.
pub fn scan_diff(diff: &Diff) -> Vec<Finding> {
    let mut findings = Vec::new();
    for file in &diff.files {
        if file.binary {
            continue;
        }
        let path = file.path().to_string();
        for hunk in &file.hunks {
            for line in &hunk.lines {
                if line.kind != LineKind::Added {
                    continue;
                }
                let Some(lineno) = line.new_lineno else {
                    continue;
                };
                if let Some(f) = match_added_line(&path, lineno, &line.content) {
                    findings.push(f);
                }
            }
        }
    }
    findings
}

/// Match a single added source line (unit-test entry point).
pub fn match_added_line(path: &str, line_no: u32, content: &str) -> Option<Finding> {
    if let Some((id, message)) = detect_secret(content) {
        return Some(Finding {
            dimension: Dimension::Security,
            confidence: SECRET_CONFIDENCE,
            severity: Severity::High,
            path: path.to_string(),
            start_line: line_no,
            end_line: line_no,
            message: message.to_string(),
            existing_code: content.trim().to_string(),
            evidence: format!("deterministic secret precheck ({id})"),
            suggestion: Some(
                "Move the credential to an environment variable or secret manager; never commit real keys."
                    .into(),
            ),
            suggestion_code: String::new(),
            reachability: Reachability::Reachable,
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        });
    }
    None
}

fn detect_secret(content: &str) -> Option<(&'static str, &'static str)> {
    if let Some(hit) = detect_token_shapes(content) {
        return Some(hit);
    }
    if content.contains("-----BEGIN ") && content.contains("PRIVATE KEY-----") {
        return Some(("private_key", "Private key material embedded in source"));
    }
    if let Some(val) = assignment_value(content, &["password", "passwd", "pwd"]) {
        if !is_placeholder(val) && val.len() >= 6 {
            return Some(("password_assign", "Hardcoded password assignment in source"));
        }
    }
    if let Some(val) = assignment_value(
        content,
        &[
            "api_key",
            "api-key",
            "apikey",
            "secret_key",
            "secret-key",
            "access_token",
            "access-token",
            "auth_token",
            "auth-token",
        ],
    ) {
        if !is_placeholder(val) && val.len() >= 8 {
            return Some((
                "api_key_assign",
                "Hardcoded API key / token assignment in source",
            ));
        }
    }
    None
}

fn detect_token_shapes(content: &str) -> Option<(&'static str, &'static str)> {
    // Stripe
    if let Some(rest) = find_prefix_token(content, "sk_live_", is_alnum, 16) {
        if rest {
            return Some((
                "stripe_live",
                "Hardcoded Stripe live secret key (sk_live_…) in source",
            ));
        }
    }
    if let Some(rest) = find_prefix_token(content, "sk_test_", is_alnum, 16) {
        if rest {
            return Some((
                "stripe_test",
                "Hardcoded Stripe test secret key (sk_test_…) in source",
            ));
        }
    }
    // AWS access key id
    if find_prefix_token(content, "AKIA", is_aws_key_char, 16).is_some() {
        // AKIA + exactly 16 more uppercase alnum is the classic shape; we accept ≥16.
        if let Some(idx) = content.find("AKIA") {
            let body = &content[idx + 4..];
            let n = body.chars().take_while(|c| is_aws_key_char(*c)).count();
            if n >= 16 {
                return Some((
                    "aws_access_key",
                    "Hardcoded AWS access key id (AKIA…) in source",
                ));
            }
        }
    }
    // GitHub
    if find_prefix_token(content, "ghp_", is_alnum, 20).is_some() {
        return Some((
            "github_pat",
            "Hardcoded GitHub personal access token (ghp_…) in source",
        ));
    }
    if find_prefix_token(content, "gho_", is_alnum, 20).is_some() {
        return Some((
            "github_oauth",
            "Hardcoded GitHub OAuth token (gho_…) in source",
        ));
    }
    // Slack xox[baprs]-...
    for pref in ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"] {
        if find_prefix_token(content, pref, is_slack_char, 10).is_some() {
            return Some(("slack_token", "Hardcoded Slack token in source"));
        }
    }
    None
}

fn is_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn is_aws_key_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

fn is_slack_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Returns `Some(true)` if prefix exists with ≥ min_len following charset chars.
fn find_prefix_token(
    content: &str,
    prefix: &str,
    charset: fn(char) -> bool,
    min_len: usize,
) -> Option<bool> {
    let mut search = content;
    while let Some(idx) = search.find(prefix) {
        let after = &search[idx + prefix.len()..];
        let n = after.chars().take_while(|c| charset(*c)).count();
        if n >= min_len {
            return Some(true);
        }
        search = &search[idx + prefix.len()..];
    }
    None
}

/// Extract quoted value after `key = "value"` / `key: 'value'` (case-insensitive key).
fn assignment_value<'a>(content: &'a str, keys: &[&str]) -> Option<&'a str> {
    let lower = content.to_ascii_lowercase();
    for key in keys {
        let key_l = key.to_ascii_lowercase();
        let mut rest = lower.as_str();
        let mut base = content;
        while let Some(idx) = rest.find(&key_l) {
            // Ensure key is a word-ish boundary (start or non-alnum before).
            if idx > 0 {
                let before = rest[..idx].chars().next_back().unwrap_or(' ');
                if before.is_ascii_alphanumeric() || before == '_' {
                    rest = &rest[idx + key_l.len()..];
                    base = &base[idx + key_l.len()..];
                    continue;
                }
            }
            let after_key = &base[idx + key.len()..];
            let after_trim = after_key.trim_start();
            let mut chars = after_trim.chars();
            let sep = chars.next()?;
            if sep != '=' && sep != ':' {
                rest = &rest[idx + key_l.len()..];
                base = &base[idx + key.len()..];
                continue;
            }
            let value_region = after_trim[sep.len_utf8()..].trim_start();
            let quote = value_region.chars().next()?;
            if quote != '\'' && quote != '"' {
                return None;
            }
            let inner = &value_region[1..];
            let end = inner.find(quote)?;
            return Some(&inner[..end]);
        }
    }
    None
}

/// Placeholder / example values that should not raise findings.
fn is_placeholder(value: &str) -> bool {
    let v = value.trim();
    if v.len() < 4 {
        return true;
    }
    let lower = v.to_ascii_lowercase();
    // Exact placeholder tokens only — do not substring-match "secret" inside real passwords.
    const EXACT: &[&str] = &[
        "password",
        "secret",
        "changeme",
        "your_password",
        "your-password",
        "your_api_key",
        "your-api-key",
        "xxx",
        "todo",
        "placeholder",
        "example",
        "redacted",
        "null",
        "none",
        "test",
        "dummy",
        "********",
        "xxxxxxxx",
    ];
    if EXACT.iter().any(|p| lower == *p) {
        return true;
    }
    // Env / template interpolations are not hardcoded secrets.
    if lower.contains("${")
        || lower.contains("{{")
        || lower.starts_with("process.env")
        || lower.starts_with("os.environ")
        || lower.starts_with("env.")
        || lower.starts_with("your_")
        || lower.starts_with("your-")
        || lower.starts_with('<') && lower.ends_with('>')
    {
        return true;
    }
    if v.chars().all(|c| c == '*' || c == 'x' || c == 'X') {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line};

    fn added_diff(path: &str, lines: &[&str]) -> Diff {
        let hunk_lines: Vec<Line> = lines
            .iter()
            .enumerate()
            .map(|(i, c)| Line {
                kind: LineKind::Added,
                content: (*c).to_string(),
                old_lineno: None,
                new_lineno: Some(i as u32 + 1),
            })
            .collect();
        let n = lines.len() as u32;
        Diff {
            files: vec![FileDiff {
                old_path: None,
                new_path: Some(path.into()),
                status: FileStatus::Added,
                binary: false,
                hunks: vec![Hunk {
                    old_start: 0,
                    old_count: 0,
                    new_start: 1,
                    new_count: n,
                    section: String::new(),
                    lines: hunk_lines,
                }],
            }],
        }
    }

    #[test]
    fn empty_diff_yields_no_findings() {
        assert!(scan_diff(&Diff::default()).is_empty());
    }

    /// Build fixture strings at runtime so GitHub secret scanning does not
    /// treat unit-test fixtures as real credentials in the repo history.
    fn fixture_stripe_live() -> String {
        format!("sk_live_{}", "A".repeat(24))
    }
    fn fixture_aws_akia() -> String {
        format!("AKIA{}", "B".repeat(16))
    }
    fn fixture_ghp() -> String {
        format!("ghp_{}", "c".repeat(36))
    }

    #[test]
    fn detects_stripe_live_and_aws_and_password() {
        let stripe = format!("STRIPE = '{}'", fixture_stripe_live());
        let aws = format!(r#"aws_key = "{}""#, fixture_aws_akia());
        let pass = r#"password = "hunter2secret""#.to_string();
        let lines = [stripe.as_str(), aws.as_str(), pass.as_str()];
        let diff = added_diff("cfg.py", &lines);
        let hits = scan_diff(&diff);
        assert!(
            hits.len() >= 3,
            "expected ≥3 secret hits, got {}: {:?}",
            hits.len(),
            hits.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
        assert!(hits.iter().all(|f| f.dimension == Dimension::Security));
        assert!(hits.iter().all(|f| f.severity == Severity::High));
        assert!(hits.iter().all(|f| f.confidence >= 0.9));
    }

    #[test]
    fn clean_code_produces_no_findings() {
        let diff = added_diff(
            "app.py",
            &[
                "import os",
                "api_key = os.environ['API_KEY']",
                "password = os.getenv('PASSWORD')",
                "print('hello')",
                "token = process.env.TOKEN",
            ],
        );
        let hits = scan_diff(&diff);
        assert!(
            hits.is_empty(),
            "clean code should not hit: {:?}",
            hits.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn placeholder_assignments_are_ignored() {
        assert!(match_added_line("a.rs", 1, r#"password = "changeme""#).is_none());
        assert!(match_added_line("a.rs", 1, r#"api_key = "YOUR_API_KEY""#).is_none());
        assert!(match_added_line("a.rs", 1, r#"secret_key = "xxxxxxxx""#).is_none());
    }

    #[test]
    fn context_and_deleted_lines_ignored() {
        let fake = fixture_stripe_live();
        let diff = Diff {
            files: vec![FileDiff {
                old_path: Some("a.rs".into()),
                new_path: Some("a.rs".into()),
                status: FileStatus::Modified,
                binary: false,
                hunks: vec![Hunk {
                    old_start: 1,
                    old_count: 2,
                    new_start: 1,
                    new_count: 1,
                    section: String::new(),
                    lines: vec![
                        Line {
                            kind: LineKind::Deleted,
                            content: fake.clone(),
                            old_lineno: Some(1),
                            new_lineno: None,
                        },
                        Line {
                            kind: LineKind::Context,
                            content: fake,
                            old_lineno: Some(2),
                            new_lineno: Some(1),
                        },
                    ],
                }],
            }],
        };
        assert!(scan_diff(&diff).is_empty());
    }

    #[test]
    fn private_key_block_detected() {
        let f = match_added_line("key.pem", 1, "-----BEGIN RSA PRIVATE KEY-----");
        assert!(f.is_some());
        assert!(f.unwrap().message.contains("Private key"));
    }

    #[test]
    fn github_pat_detected() {
        let line = format!("export TOKEN={}", fixture_ghp());
        let f = match_added_line("a.sh", 1, &line);
        assert!(f.is_some());
    }
}
