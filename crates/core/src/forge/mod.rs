//! 代码托管平台（forge）PR/MR 评论：GitHub / GitLab / AtomGit。
//!
//! 一条 Markdown **摘要评论**是核心闸口信号，三平台都支持；**行内 suggestion** 目前仅 GitHub。
//!
//! 平台识别：显式 `REVIEWGATE_FORGE`（github|gitlab|atomgit）优先；否则按 CI 环境自动识别
//! （GitHub Actions 的 `GITHUB_*`、GitLab CI 的 `CI_*`）。AtomGit 无公开 CI 约定，走显式变量。

use crate::gate::GateDecision;
use crate::model::{Finding, Severity};
use crate::review::ReviewOutcome;
use anyhow::{Context, Result};

/// 支持的代码托管平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    AtomGit,
}

impl Forge {
    /// 解析平台标识（大小写不敏感）。用于 `REVIEWGATE_FORGE`。
    pub fn parse(s: &str) -> Option<Forge> {
        match s.trim().to_ascii_lowercase().as_str() {
            "github" => Some(Forge::GitHub),
            "gitlab" => Some(Forge::GitLab),
            "atomgit" => Some(Forge::AtomGit),
            _ => None,
        }
    }

    /// 默认 API base。GitLab 无固定值——必须由 `CI_API_V4_URL` 提供（返回空串）。
    pub fn default_api_base(self) -> &'static str {
        match self {
            Forge::GitHub => "https://api.github.com",
            Forge::AtomGit => "https://api.atomgit.com/api/v5",
            Forge::GitLab => "",
        }
    }
}

/// 一次评论所需的解析后上下文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeContext {
    pub forge: Forge,
    /// API base（无尾斜杠）。
    pub api_base: String,
    /// GitHub/AtomGit 为 `owner/repo`；GitLab 为 project id（或 URL 编码的 path）。
    pub repo: String,
    /// PR / MR 号。
    pub number: u64,
    pub token: String,
}

/// 从环境解析评论上下文。返回 `None` = 非 CI/PR 上下文或缺关键信息（跳过评论，不报错）。
///
/// 入参 `get` 抽象 env 读取（便于单测）；`github_pr_hint` 是 GitHub 从 ref/event 推断的 PR 号
/// （其它平台忽略）。通用 `REVIEWGATE_FORGE/REPO/PR/MR/TOKEN/API_BASE` 可覆盖任意平台的自动识别。
pub fn resolve_context(
    get: impl Fn(&str) -> Option<String>,
    github_pr_hint: Option<u64>,
) -> Option<ForgeContext> {
    // 1) 平台识别：显式 REVIEWGATE_FORGE 优先，否则按 CI 环境自动识别。
    let forge = if let Some(f) = get("REVIEWGATE_FORGE").and_then(|s| Forge::parse(&s)) {
        f
    } else if get("GITHUB_ACTIONS").is_some() || get("GITHUB_REPOSITORY").is_some() {
        Forge::GitHub
    } else if get("GITLAB_CI").is_some() || get("CI_PROJECT_ID").is_some() {
        Forge::GitLab
    } else {
        return None;
    };

    // 2) token：通用 REVIEWGATE_TOKEN 覆盖平台默认。
    let token = get("REVIEWGATE_TOKEN").or_else(|| match forge {
        Forge::GitHub => get("GITHUB_TOKEN"),
        Forge::GitLab => get("GITLAB_TOKEN").or_else(|| get("CI_JOB_TOKEN")),
        Forge::AtomGit => get("ATOMGIT_TOKEN"),
    })?;

    // 3) repo / number / api_base：通用 REVIEWGATE_* 覆盖平台默认。
    let num = |s: String| s.parse::<u64>().ok();
    let (repo, number, api_base) = match forge {
        Forge::GitHub => {
            let repo = get("REVIEWGATE_REPO").or_else(|| get("GITHUB_REPOSITORY"))?;
            let number = get("REVIEWGATE_PR").and_then(num).or(github_pr_hint)?;
            let base = get("REVIEWGATE_API_BASE")
                .or_else(|| get("GITHUB_API_URL"))
                .unwrap_or_else(|| forge.default_api_base().to_string());
            (repo, number, base)
        }
        Forge::GitLab => {
            let repo = get("REVIEWGATE_REPO").or_else(|| get("CI_PROJECT_ID"))?;
            let number = get("REVIEWGATE_MR")
                .or_else(|| get("REVIEWGATE_PR"))
                .or_else(|| get("CI_MERGE_REQUEST_IID"))
                .and_then(num)?;
            let base = get("REVIEWGATE_API_BASE").or_else(|| get("CI_API_V4_URL"))?;
            (repo, number, base)
        }
        Forge::AtomGit => {
            let repo = get("REVIEWGATE_REPO").or_else(|| get("ATOMGIT_REPOSITORY"))?;
            let number = get("REVIEWGATE_PR")
                .or_else(|| get("ATOMGIT_PR"))
                .and_then(num)?;
            let base =
                get("REVIEWGATE_API_BASE").unwrap_or_else(|| forge.default_api_base().to_string());
            (repo, number, base)
        }
    };

    Some(ForgeContext {
        forge,
        api_base,
        repo,
        number,
        token,
    })
}

/// 摘要评论的 POST 端点 URL。
pub fn summary_endpoint(forge: Forge, api_base: &str, repo: &str, number: u64) -> String {
    let base = api_base.trim_end_matches('/');
    match forge {
        // GitHub: PR 摘要走 issues 评论接口（PR 也是 issue）。
        Forge::GitHub => format!("{base}/repos/{repo}/issues/{number}/comments"),
        // GitLab: MR note。
        Forge::GitLab => format!("{base}/projects/{repo}/merge_requests/{number}/notes"),
        // AtomGit（Gitee v5 风格）。
        Forge::AtomGit => format!("{base}/repos/{repo}/pulls/{number}/comments"),
    }
}

/// 鉴权请求头 (name, value)。GitLab 用 `PRIVATE-TOKEN`；GitHub/AtomGit 用 `Authorization: Bearer`。
pub fn auth_header(forge: Forge, token: &str) -> (&'static str, String) {
    match forge {
        Forge::GitLab => ("PRIVATE-TOKEN", token.to_string()),
        Forge::GitHub | Forge::AtomGit => ("Authorization", format!("Bearer {token}")),
    }
}

/// 把审查结果渲染成 Markdown 摘要（平台中立，三平台通用）。
pub fn render_markdown(outcome: &ReviewOutcome) -> String {
    let badge = match outcome.decision {
        GateDecision::Pass => "✅ **PASS** — 放行",
        GateDecision::Warn => "⚠️ **WARN** — 有需关注的问题",
        GateDecision::Block => "🛑 **BLOCK** — 阻断合并",
    };
    let kept: Vec<&Finding> = outcome.findings.iter().filter(|f| !f.filtered).collect();
    let filtered = outcome.findings.len() - kept.len();

    let mut md = String::new();
    md.push_str("## 🚪 ReviewGate\n\n");
    md.push_str(&format!(
        "{badge}\n\n{} 个文件改动 · {} 条可信发现 · {} 条已过滤\n\n",
        outcome.files_changed,
        kept.len(),
        filtered
    ));

    if outcome.incomplete {
        md.push_str(
            "> 🟠 **审查未完整**：部分维度/单元因超时、请求失败、上下文超限或超大文件被跳过而**未审完** —— \
             结论不代表“无问题”。\n\n",
        );
        let unfinished = crate::review::unfinished_paths(&outcome.warnings);
        if !unfinished.is_empty() {
            md.push_str(&format!(
                "> **未覆盖路径:** {}\n\n",
                unfinished
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for a in crate::review::incomplete_advice(&outcome.warnings) {
            md.push_str(&format!("> - {a}\n"));
        }
        if !outcome.warnings.is_empty() {
            md.push('\n');
        }
    }
    if !outcome.warnings.is_empty() {
        let list: Vec<String> = outcome
            .warnings
            .iter()
            .map(|w| {
                let paths = if w.paths.is_empty() {
                    String::new()
                } else {
                    format!(" @ {}", w.paths.join(", "))
                };
                format!("`{}`（{}{}）", w.dimension, w.kind, paths)
            })
            .collect();
        md.push_str(&format!("> ⚠️ **Incomplete**: {}.\n\n", list.join(", ")));
    }
    if outcome.critical_incomplete {
        md.push_str(
            "> 🛑 **关键路径未审完**：触及 auth/payment/security 等敏感路径的 incomplete 已强制非 PASS。\n\n",
        );
    }

    if kept.is_empty() {
        md.push_str("No issues reached the display threshold.\n");
        return md;
    }

    md.push_str("| Severity | Dimension | Confidence | Location | Issue |\n");
    md.push_str("|---|---|---|---|---|\n");
    for f in &kept {
        let sev = match f.severity {
            Severity::High => "🔴 high",
            Severity::Med => "🟡 med",
            Severity::Low => "⚪ low",
        };
        let loc = if f.located() {
            format!("`{}:{}`", f.path, f.start_line)
        } else {
            format!("`{}`", f.path)
        };
        let msg = f.message.replace('|', "\\|").replace('\n', " ");
        md.push_str(&format!(
            "| {} | {} | {:.2} | {} | {} |\n",
            sev,
            f.dimension.as_str(),
            f.confidence,
            loc,
            msg
        ));
    }
    md.push_str("\n<sub>Generated by ReviewGate - parallel multi-agent + per-dimension experts + confidence filtering</sub>\n");
    md
}

/// 发一条摘要评论到检测到的 forge（GitHub/GitLab/AtomGit）。非 PR/MR 上下文则跳过。
pub async fn post_summary(outcome: &ReviewOutcome) -> Result<()> {
    let Some(ctx) = resolve_context(|k| std::env::var(k).ok(), detect_pr_number()) else {
        eprintln!("no forge PR/MR context detected; skipping summary comment.");
        return Ok(());
    };

    let body = render_markdown(outcome);
    let url = summary_endpoint(ctx.forge, &ctx.api_base, &ctx.repo, ctx.number);
    let (hname, hval) = auth_header(ctx.forge, &ctx.token);
    let client = crate::llm::http::shared_http_client()?;
    let resp = client
        .post(&url)
        .header(hname, hval)
        .header("User-Agent", "ReviewGate")
        .json(&serde_json::json!({ "body": body }))
        .send()
        .await
        .context("failed to send forge comment")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("forge comment returned {status}: {text}");
    }
    eprintln!(
        "Posted ReviewGate summary comment ({:?} #{}).",
        ctx.forge, ctx.number
    );
    Ok(())
}

/// 行内评论候选：已定位、未过滤，且 **high 或达到 block 置信度**（闸口级问题落到 PR 行上）。
/// 意图维度走验收清单，不发行内 suggestion。
pub fn inline_candidates<'a>(outcome: &'a ReviewOutcome, block_threshold: f32) -> Vec<&'a Finding> {
    use crate::model::Dimension;
    outcome
        .findings
        .iter()
        .filter(|f| {
            !f.filtered
                && f.start_line > 0
                && f.dimension != Dimension::Intent
                && (f.severity == Severity::High || f.confidence >= block_threshold)
        })
        .collect()
}

/// 在 PR 上为闸口级发现发**行内 review 评论**，带 ` ```suggestion ` 块（一键应用，人把关）。
/// **目前仅 GitHub**——GitLab/AtomGit 的行内定位差异较大，暂只在摘要评论里给出发现。
/// best-effort：逐条独立提交，单条失败不影响其它。
///
/// `block_threshold` 应来自闸口配置（`gate.block_threshold`），与 PASS/BLOCK 判定一致。
pub async fn post_inline_suggestions(outcome: &ReviewOutcome, block_threshold: f32) -> Result<()> {
    let Some(ctx) = resolve_context(|k| std::env::var(k).ok(), detect_pr_number()) else {
        return Ok(());
    };
    if ctx.forge != Forge::GitHub {
        eprintln!("inline suggestions are GitHub-only for now; skipping.");
        return Ok(());
    }
    let Some(commit_id) = detect_head_sha() else {
        eprintln!("could not get the PR head commit sha; skipping inline comments.");
        return Ok(());
    };

    let candidates = inline_candidates(outcome, block_threshold);
    if candidates.is_empty() {
        eprintln!("No high/BLOCK findings with line anchors; skipping inline comments.");
        return Ok(());
    }

    let url = format!(
        "{}/repos/{}/pulls/{}/comments",
        ctx.api_base.trim_end_matches('/'),
        ctx.repo,
        ctx.number
    );
    let client = crate::llm::http::shared_http_client()?;
    let mut posted = 0usize;
    for f in candidates {
        let mut body = format!(
            "**[{} · {} · {:.0}%] ReviewGate**\n\n{}",
            f.dimension.as_str(),
            f.severity.as_str(),
            f.confidence * 100.0,
            f.message
        );
        if !f.suggestion_code.trim().is_empty() {
            body.push_str(&format!(
                "\n\n```suggestion\n{}\n```",
                f.suggestion_code.trim_end_matches('\n')
            ));
        } else if let Some(s) = f.suggestion.as_deref().filter(|s| !s.trim().is_empty()) {
            body.push_str(&format!("\n\n**Fix hint:** {s}"));
        }
        let mut payload = serde_json::json!({
            "body": body,
            "commit_id": commit_id,
            "path": f.path,
            "line": f.end_line.max(f.start_line),
            "side": "RIGHT",
        });
        if f.end_line > f.start_line {
            payload["start_line"] = serde_json::json!(f.start_line);
            payload["start_side"] = serde_json::json!("RIGHT");
        }

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", ctx.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "ReviewGate")
            .json(&payload)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => posted += 1,
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                eprintln!(
                    "行内评论失败 {}:{} → {status}：{}",
                    f.path,
                    f.start_line,
                    text.chars().take(160).collect::<String>()
                );
            }
            Err(e) => eprintln!(
                "inline comment request failed {}:{} -> {e}",
                f.path, f.start_line
            ),
        }
    }
    eprintln!("Posted {posted} inline suggestion comment(s) (high/BLOCK only).");
    Ok(())
}

/// PR head commit sha：优先 event payload 的 `pull_request.head.sha`，回退 `GITHUB_SHA`。
fn detect_head_sha() -> Option<String> {
    if let Ok(path) = std::env::var("GITHUB_EVENT_PATH") {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(sha) = v
                    .get("pull_request")
                    .and_then(|pr| pr.get("head"))
                    .and_then(|h| h.get("sha"))
                    .and_then(|s| s.as_str())
                {
                    return Some(sha.to_string());
                }
            }
        }
    }
    std::env::var("GITHUB_SHA").ok().filter(|s| !s.is_empty())
}

/// 从 GitHub Action 环境推断 PR 号（其它平台的号在 `resolve_context` 里读各自 env）。
fn detect_pr_number() -> Option<u64> {
    if let Ok(r) = std::env::var("GITHUB_REF") {
        if let Some(n) = parse_pr_ref(&r) {
            return Some(n);
        }
    }
    if let Ok(path) = std::env::var("GITHUB_EVENT_PATH") {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Some(n) = parse_pr_event(&text) {
                return Some(n);
            }
        }
    }
    None
}

/// 从 `refs/pull/<N>/merge` 形式的 ref 抽 PR 号。
fn parse_pr_ref(r: &str) -> Option<u64> {
    r.strip_prefix("refs/pull/")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

/// 从 GitHub event payload JSON 抽 PR 号（`pull_request.number` 或顶层 `number`）。
fn parse_pr_event(text: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("pull_request")
        .and_then(|pr| pr.get("number"))
        .and_then(|n| n.as_u64())
        .or_else(|| v.get("number").and_then(|n| n.as_u64()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_forge_ci() {
        assert_eq!(Forge::parse("github"), Some(Forge::GitHub));
        assert_eq!(Forge::parse("GitLab"), Some(Forge::GitLab));
        assert_eq!(Forge::parse(" atomgit "), Some(Forge::AtomGit));
        assert_eq!(Forge::parse("bitbucket"), None);
    }

    #[test]
    fn endpoints_per_forge() {
        assert_eq!(
            summary_endpoint(Forge::GitHub, "https://api.github.com", "o/r", 7),
            "https://api.github.com/repos/o/r/issues/7/comments"
        );
        assert_eq!(
            summary_endpoint(Forge::GitLab, "https://gitlab.com/api/v4/", "42", 9),
            "https://gitlab.com/api/v4/projects/42/merge_requests/9/notes"
        );
        assert_eq!(
            summary_endpoint(Forge::AtomGit, "https://api.atomgit.com/api/v5", "o/r", 3),
            "https://api.atomgit.com/api/v5/repos/o/r/pulls/3/comments"
        );
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn resolve_none_without_forge_env() {
        assert!(resolve_context(env(&[]), None).is_none());
    }

    #[test]
    fn resolve_github_auto() {
        let ctx = resolve_context(
            env(&[("GITHUB_REPOSITORY", "o/r"), ("GITHUB_TOKEN", "gh")]),
            Some(7),
        )
        .expect("应识别 GitHub");
        assert_eq!(ctx.forge, Forge::GitHub);
        assert_eq!(ctx.repo, "o/r");
        assert_eq!(ctx.number, 7);
        assert_eq!(ctx.token, "gh");
        assert_eq!(ctx.api_base, "https://api.github.com");
    }

    #[test]
    fn resolve_github_none_without_pr() {
        // 无 PR 号（非 PR 上下文）→ None，跳过评论。
        assert!(resolve_context(
            env(&[("GITHUB_REPOSITORY", "o/r"), ("GITHUB_TOKEN", "gh")]),
            None
        )
        .is_none());
    }

    #[test]
    fn resolve_gitlab_auto_from_ci() {
        let ctx = resolve_context(
            env(&[
                ("GITLAB_CI", "true"),
                ("CI_PROJECT_ID", "42"),
                ("CI_MERGE_REQUEST_IID", "9"),
                ("CI_API_V4_URL", "https://gitlab.com/api/v4"),
                ("GITLAB_TOKEN", "gl"),
            ]),
            None,
        )
        .expect("应识别 GitLab");
        assert_eq!(ctx.forge, Forge::GitLab);
        assert_eq!(ctx.repo, "42");
        assert_eq!(ctx.number, 9);
        assert_eq!(ctx.token, "gl");
        assert_eq!(ctx.api_base, "https://gitlab.com/api/v4");
    }

    #[test]
    fn resolve_atomgit_explicit() {
        let ctx = resolve_context(
            env(&[
                ("REVIEWGATE_FORGE", "atomgit"),
                ("REVIEWGATE_REPO", "o/r"),
                ("REVIEWGATE_PR", "5"),
                ("REVIEWGATE_TOKEN", "at"),
            ]),
            None,
        )
        .expect("应识别 AtomGit");
        assert_eq!(ctx.forge, Forge::AtomGit);
        assert_eq!(ctx.repo, "o/r");
        assert_eq!(ctx.number, 5);
        assert_eq!(ctx.token, "at");
        assert_eq!(ctx.api_base, "https://api.atomgit.com/api/v5");
    }

    #[test]
    fn reviewgate_token_overrides_platform_token() {
        let ctx = resolve_context(
            env(&[
                ("GITHUB_REPOSITORY", "o/r"),
                ("GITHUB_TOKEN", "gh"),
                ("REVIEWGATE_TOKEN", "override"),
            ]),
            Some(1),
        )
        .unwrap();
        assert_eq!(ctx.token, "override");
    }

    #[test]
    fn auth_header_per_forge() {
        assert_eq!(
            auth_header(Forge::GitHub, "tok"),
            ("Authorization", "Bearer tok".to_string())
        );
        assert_eq!(
            auth_header(Forge::AtomGit, "tok"),
            ("Authorization", "Bearer tok".to_string())
        );
        assert_eq!(
            auth_header(Forge::GitLab, "tok"),
            ("PRIVATE-TOKEN", "tok".to_string())
        );
    }

    use crate::model::{Dimension, Reachability};

    #[test]
    fn parse_pr_number_from_ref_and_event() {
        assert_eq!(parse_pr_ref("refs/pull/38060/merge"), Some(38060));
        assert_eq!(parse_pr_ref("refs/pull/12/head"), Some(12));
        assert_eq!(parse_pr_ref("refs/heads/main"), None);
        assert_eq!(parse_pr_event(r#"{"pull_request":{"number":7}}"#), Some(7));
        assert_eq!(parse_pr_event(r#"{"number":9}"#), Some(9));
        assert_eq!(parse_pr_event(r#"{"foo":1}"#), None);
        assert_eq!(parse_pr_event("not json"), None);
        assert_eq!(parse_pr_event(r#"{"pull_request":{"number":"abc"}}"#), None);
    }

    fn base_finding() -> Finding {
        Finding {
            dimension: Dimension::Security,
            confidence: 0.95,
            severity: Severity::High,
            path: "a.rs".into(),
            start_line: 3,
            end_line: 3,
            message: "SQL 注入".into(),
            existing_code: "x".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    fn outcome_with(findings: Vec<Finding>, d: GateDecision, incomplete: bool) -> ReviewOutcome {
        ReviewOutcome {
            files_changed: 1,
            decision: d,
            incomplete,
            findings,
            ..Default::default()
        }
    }

    #[test]
    fn markdown_summary_pass_no_issues() {
        let md = render_markdown(&outcome_with(vec![], GateDecision::Pass, false));
        assert!(md.contains("PASS"));
        assert!(md.contains("No issues reached the display threshold"));
    }

    #[test]
    fn markdown_summary_warn_incomplete_and_warnings() {
        let mut f = base_finding();
        f.filtered = true;
        let mut o = outcome_with(vec![f], GateDecision::Warn, true);
        o.files_changed = 2;
        o.warnings = vec![crate::review::ReviewWarning::new(
            "logic",
            "timed_out",
            "timeout",
        )];
        let md = render_markdown(&o);
        assert!(md.contains("WARN"));
        assert!(md.contains("审查未完整"));
        assert!(md.contains("timed_out"));
        assert!(md.contains("1 条已过滤"));
    }

    #[test]
    fn inline_candidates_only_high_or_block_confidence() {
        let high = base_finding(); // high + 0.95
        let mut med = base_finding();
        med.severity = Severity::Med;
        med.confidence = 0.6;
        med.start_line = 5;
        let mut low_located = base_finding();
        low_located.severity = Severity::Low;
        low_located.confidence = 0.5;
        let mut unlocated = base_finding();
        unlocated.start_line = 0;
        let o = outcome_with(
            vec![high, med, low_located, unlocated],
            GateDecision::Block,
            false,
        );
        let c = inline_candidates(&o, 0.8);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].severity, Severity::High);
        // med with confidence >= block threshold
        let mut med_block = base_finding();
        med_block.severity = Severity::Med;
        med_block.confidence = 0.85;
        let o2 = outcome_with(vec![med_block], GateDecision::Block, false);
        assert_eq!(inline_candidates(&o2, 0.8).len(), 1);
    }

    #[test]
    fn markdown_summary_has_decision_and_escapes_pipe() {
        let mut f = base_finding();
        f.message = "SQL 注入 | 危险".into();
        let md = render_markdown(&outcome_with(vec![f], GateDecision::Block, false));
        assert!(md.contains("BLOCK"));
        assert!(md.contains("a.rs:3"));
        assert!(md.contains("\\|"));
    }

    #[test]
    fn render_markdown_block_with_multiple_findings() {
        let mut f1 = base_finding();
        f1.message = "SQL injection\nwith newline".into();
        let mut f2 = base_finding();
        f2.severity = Severity::Med;
        f2.confidence = 0.7;
        f2.message = "inefficient clone".into();
        f2.path = "b.rs".into();
        f2.start_line = 10;
        f2.end_line = 12;
        let md = render_markdown(&outcome_with(vec![f1, f2], GateDecision::Block, false));
        assert!(md.contains("🛑 **BLOCK"));
        assert!(md.contains("🔴 high"));
        assert!(md.contains("🟡 med"));
        assert!(md.contains("SQL injection with newline"));
        assert!(md.contains("b.rs:10"));
    }

    #[test]
    fn detect_head_sha_from_event_and_env() {
        let tmp = std::env::temp_dir().join(format!("rg_event_{}", std::process::id()));
        std::fs::write(&tmp, r#"{"pull_request":{"head":{"sha":"abc123"}}}"#).unwrap();
        std::env::set_var("GITHUB_EVENT_PATH", tmp.to_str().unwrap());
        assert_eq!(detect_head_sha(), Some("abc123".into()));

        std::env::remove_var("GITHUB_EVENT_PATH");
        std::env::set_var("GITHUB_SHA", "def456");
        assert_eq!(detect_head_sha(), Some("def456".into()));
        std::env::remove_var("GITHUB_SHA");
    }
}
