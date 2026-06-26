//! GitHub PR 摘要评论（供 GitHub Action 使用）。
//!
//! 读取 Action 环境变量（GITHUB_TOKEN / GITHUB_REPOSITORY / PR 号 / head sha），
//! 发两类评论：一条 Markdown **摘要评论**，以及每条已定位发现的**行内 review 评论**
//! （带 ` ```suggestion ` 块，作者在 PR 点「Commit suggestion」一键应用——人把关）。

use crate::gate::GateDecision;
use crate::model::{Finding, Severity};
use crate::review::ReviewOutcome;
use anyhow::{Context, Result};

/// 把审查结果渲染成 Markdown 摘要。
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

    // 未审完告警：放在最前，0 发现时也要显示——避免"没审完"被当成"通过"。
    if outcome.incomplete {
        md.push_str(
            "> 🟠 **审查未完整**：部分维度/单元因超时、请求失败、上下文超限或超大文件被跳过而**未审完** —— \
             结论不代表“无问题”。请放宽 --timeout、调大 `max_input_tokens` 或拆分改动后重跑。\n\n",
        );
    }
    if !outcome.warnings.is_empty() {
        let list: Vec<String> = outcome
            .warnings
            .iter()
            .map(|w| format!("`{}`（{}）", w.dimension, w.kind))
            .collect();
        md.push_str(&format!("> ⚠️ **未审完明细**：{}。\n\n", list.join("、")));
    }

    if kept.is_empty() {
        md.push_str("未发现达到展示阈值的问题。\n");
        return md;
    }

    md.push_str("| 严重度 | 维度 | 置信度 | 位置 | 问题 |\n");
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
    md.push_str("\n<sub>由 ReviewGate 自动生成 · 多 Agent 并行 + 分维度专家 + 置信度过滤</sub>\n");
    md
}

/// 若处于 GitHub Action PR 上下文，发一条摘要评论。非 PR 上下文则跳过。
pub async fn post_summary(outcome: &ReviewOutcome) -> Result<()> {
    let token = std::env::var("GITHUB_TOKEN").context("缺少 GITHUB_TOKEN")?;
    let repo = std::env::var("GITHUB_REPOSITORY").context("缺少 GITHUB_REPOSITORY")?;
    let Some(pr) = detect_pr_number() else {
        eprintln!("非 PR 上下文，跳过评论。");
        return Ok(());
    };

    let body = render_markdown(outcome);
    let url = format!("https://api.github.com/repos/{repo}/issues/{pr}/comments");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "ReviewGate")
        .json(&serde_json::json!({ "body": body }))
        .send()
        .await
        .context("发送 GitHub 评论失败")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("GitHub 评论返回 {status}：{text}");
    }
    eprintln!("已在 PR #{pr} 发布 ReviewGate 摘要评论。");
    Ok(())
}

/// 在 PR 上为每条已定位发现发一条**行内 review 评论**，带 ` ```suggestion ` 块——
/// 作者在 GitHub UI 点「Commit suggestion」即可一键应用（**人把关**）。
/// best-effort：逐条独立提交，单条失败（如行不在 diff 内）不影响其它。
pub async fn post_inline_suggestions(outcome: &ReviewOutcome) -> Result<()> {
    let token = std::env::var("GITHUB_TOKEN").context("缺少 GITHUB_TOKEN")?;
    let repo = std::env::var("GITHUB_REPOSITORY").context("缺少 GITHUB_REPOSITORY")?;
    let Some(pr) = detect_pr_number() else {
        eprintln!("非 PR 上下文，跳过行内评论。");
        return Ok(());
    };
    let Some(commit_id) = detect_head_sha() else {
        eprintln!("拿不到 PR head commit sha，跳过行内评论。");
        return Ok(());
    };

    let url = format!("https://api.github.com/repos/{repo}/pulls/{pr}/comments");
    let client = reqwest::Client::new();
    let mut posted = 0usize;
    for f in outcome
        .findings
        .iter()
        .filter(|f| !f.filtered && f.start_line > 0)
    {
        let mut body = format!(
            "**[{} · {}] ReviewGate**\n\n{}",
            f.dimension.as_str(),
            f.severity.as_str(),
            f.message
        );
        // 有修复代码 → 附 GitHub 原生 suggestion 块（一键应用）。
        if !f.suggestion_code.trim().is_empty() {
            body.push_str(&format!(
                "\n\n```suggestion\n{}\n```",
                f.suggestion_code.trim_end_matches('\n')
            ));
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
            .header("Authorization", format!("Bearer {token}"))
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
            Err(e) => eprintln!("行内评论请求失败 {}:{} → {e}", f.path, f.start_line),
        }
    }
    eprintln!("已发布 {posted} 条行内 suggestion 评论。");
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

/// 从 Action 环境推断 PR 号。
fn detect_pr_number() -> Option<u64> {
    // refs/pull/<N>/merge
    if let Ok(r) = std::env::var("GITHUB_REF") {
        if let Some(n) = parse_pr_ref(&r) {
            return Some(n);
        }
    }
    // event payload
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
    use crate::gate::GateDecision;
    use crate::model::{Dimension, Finding, Severity};

    #[test]
    fn parse_pr_number_from_ref_and_event() {
        assert_eq!(parse_pr_ref("refs/pull/38060/merge"), Some(38060));
        assert_eq!(parse_pr_ref("refs/pull/12/head"), Some(12));
        assert_eq!(parse_pr_ref("refs/heads/main"), None);
        assert_eq!(parse_pr_event(r#"{"pull_request":{"number":7}}"#), Some(7));
        assert_eq!(parse_pr_event(r#"{"number":9}"#), Some(9));
        assert_eq!(parse_pr_event(r#"{"foo":1}"#), None);
        assert_eq!(parse_pr_event("not json"), None);
    }

    #[test]
    fn markdown_summary_has_decision_and_rows() {
        let outcome = ReviewOutcome {
            files_changed: 1,
            decision: GateDecision::Block,
            warnings: vec![],
            incomplete: false,
            usage: Default::default(),
            findings: vec![Finding {
                dimension: Dimension::Security,
                confidence: 0.95,
                severity: Severity::High,
                path: "a.rs".into(),
                start_line: 3,
                end_line: 3,
                message: "SQL 注入 | 危险".into(),
                existing_code: "x".into(),
                evidence: String::new(),
                suggestion: None,
                suggestion_code: String::new(),
                reachability: crate::model::Reachability::default(),
                filtered: false,
                agreed_dimensions: 1,
            }],
        };
        let md = render_markdown(&outcome);
        assert!(md.contains("BLOCK"));
        assert!(md.contains("a.rs:3"));
        // message 里的 | 被转义，不破坏表格。
        assert!(md.contains("\\|"));
    }
}
