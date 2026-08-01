//! 拉取 PR/MR 上**已有的人类评审讨论**，作为审查上下文注入 prompt。
//!
//! 目的：reviewer 已经指出过的点，机器不该再当新发现刷一遍——那是噪音，也是 review bot
//! 最招人烦的地方。
//!
//! **只做上下文注入，不做自动折叠**：不按文本相似度去隐藏任何发现。相似度匹配一旦误判，
//! 就等于"有人评论过 → 这个问题不算数"，那是给闸口开后门。要不要重复报由模型带着完整
//! 上下文判断，且被要求显式说明"这点此前已被提出"。

use super::{Forge, ForgeContext};
use anyhow::{Context, Result};
use serde::Deserialize;

/// 注入 prompt 的讨论文本上限（字符）。PR 讨论可能非常长，必须有界，
/// 否则挤占 diff 的 token 预算、把单元推成 oversized。
pub const MAX_DISCUSSION_CHARS: usize = 6000;

/// 单条讨论内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// 评论者 login（机器人已过滤）。
    pub author: String,
    /// 行内评论所在文件（顶层评论为 None）。
    pub path: Option<String>,
    /// 行内评论所在行（顶层评论为 None）。
    pub line: Option<u32>,
    pub body: String,
}

#[derive(Deserialize)]
struct GhComment {
    #[serde(default)]
    user: Option<GhUser>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    original_line: Option<u32>,
}

#[derive(Deserialize)]
struct GhUser {
    #[serde(default)]
    login: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// 解析 GitHub 评论数组（review comments 或 issue comments 通用）。
///
/// 过滤掉机器人（`user.type == "Bot"`、login 以 `[bot]` 结尾）和 ReviewGate 自己的评论——
/// 否则上一轮自己的输出会被当成"已有讨论"喂回去，形成自我强化。
pub fn parse_github_comments(json: &str) -> Result<Vec<Note>> {
    let raw: Vec<GhComment> =
        serde_json::from_str(json).context("failed to parse forge comments JSON")?;
    Ok(raw
        .into_iter()
        .filter_map(|c| {
            let body = c.body.unwrap_or_default();
            let body = body.trim();
            if body.is_empty() || body.contains("🚪 ReviewGate") {
                return None;
            }
            let user = c.user.unwrap_or(GhUser {
                login: None,
                kind: None,
            });
            let login = user.login.unwrap_or_default();
            let is_bot = user.kind.as_deref() == Some("Bot") || login.ends_with("[bot]");
            if is_bot {
                return None;
            }
            Some(Note {
                author: login,
                path: c.path,
                line: c.line.or(c.original_line),
                body: body.to_string(),
            })
        })
        .collect())
}

/// 把讨论渲染成注入 prompt 的紧凑文本。超过 [`MAX_DISCUSSION_CHARS`] 时**保留最新的**
/// （PR 后期的讨论更贴近当前代码），并显式标注截断了多少条——不假装全给了。
pub fn render_notes(notes: &[Note], max_chars: usize) -> String {
    let mut rendered: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut dropped = 0usize;
    for n in notes.iter().rev() {
        let loc = match (&n.path, n.line) {
            (Some(p), Some(l)) => format!("{p}:{l}"),
            (Some(p), None) => p.clone(),
            _ => "(general)".to_string(),
        };
        // 单条也要截断：一条超长评论不该吃掉整个预算。
        let body: String = n.body.chars().take(600).collect();
        let one = format!("- **{}** on `{loc}`: {}", n.author, body.replace('\n', " "));
        if used + one.len() > max_chars {
            dropped += 1;
            continue;
        }
        used += one.len();
        rendered.push(one);
    }
    rendered.reverse();
    if dropped > 0 {
        rendered.push(format!(
            "- _({dropped} older comment(s) omitted to stay within the context budget)_"
        ));
    }
    rendered.join("\n")
}

/// 拉取该 PR/MR 的行内评审评论 + 顶层评论。
///
/// 目前只实现 GitHub（`gh`/token 两条路径都走同一 API）。其它平台返回空——
/// 宁可不注入，也不猜 API 形状。网络/权限失败同样返回空并提示，绝不因此中断审查。
pub async fn fetch(ctx: &ForgeContext) -> Result<Vec<Note>> {
    if ctx.forge != Forge::GitHub {
        return Ok(Vec::new());
    }
    let base = ctx.api_base.trim_end_matches('/');
    let review_url = format!("{base}/repos/{}/pulls/{}/comments", ctx.repo, ctx.number);
    let issue_url = format!("{base}/repos/{}/issues/{}/comments", ctx.repo, ctx.number);

    let mut notes = Vec::new();
    for url in [review_url, issue_url] {
        match fetch_one(ctx, &url).await {
            Ok(mut n) => notes.append(&mut n),
            Err(e) => eprintln!("  [discussion] {url} failed (ignored): {e}"),
        }
    }
    Ok(notes)
}

async fn fetch_one(ctx: &ForgeContext, url: &str) -> Result<Vec<Note>> {
    let (hname, hval) = super::auth_header(ctx.forge, &ctx.token);
    let client = crate::llm::http::shared_http_client()?;
    let resp = client
        .get(url)
        .header(hname, hval)
        .header("User-Agent", "ReviewGate")
        .query(&[("per_page", "100")])
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    parse_github_comments(&resp.text().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inline_and_top_level_comments() {
        let json = r#"[
          {"user":{"login":"alice","type":"User"},"body":"这里没做空值检查","path":"src/a.rs","line":42},
          {"user":{"login":"bob","type":"User"},"body":"整体看起来不错"}
        ]"#;
        let notes = parse_github_comments(json).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(notes[0].line, Some(42));
        assert_eq!(notes[1].path, None);
    }

    #[test]
    fn drops_bots_and_reviewgates_own_comments() {
        let json = r#"[
          {"user":{"login":"dependabot[bot]","type":"Bot"},"body":"bump dep"},
          {"user":{"login":"ci","type":"Bot"},"body":"build ok"},
          {"user":{"login":"rg","type":"User"},"body":"🚪 ReviewGate 上一轮结论"},
          {"user":{"login":"carol","type":"User"},"body":"真人意见"}
        ]"#;
        let notes = parse_github_comments(json).unwrap();
        assert_eq!(
            notes.len(),
            1,
            "只应留下真人非 ReviewGate 的评论: {notes:?}"
        );
        assert_eq!(notes[0].author, "carol");
    }

    #[test]
    fn empty_bodies_are_skipped() {
        let json = r#"[{"user":{"login":"a","type":"User"},"body":"   "}]"#;
        assert!(parse_github_comments(json).unwrap().is_empty());
    }

    #[test]
    fn render_keeps_newest_and_discloses_truncation() {
        let notes: Vec<Note> = (0..50)
            .map(|i| Note {
                author: format!("u{i}"),
                path: Some("a.rs".into()),
                line: Some(i as u32),
                body: "x".repeat(100),
            })
            .collect();
        let out = render_notes(&notes, 600);
        assert!(out.contains("u49"), "应保留最新的评论：{out}");
        assert!(!out.contains("**u0**"), "最旧的应被裁掉：{out}");
        assert!(out.contains("omitted"), "截断必须明说：{out}");
        assert!(out.len() < 1500);
    }

    #[test]
    fn render_empty_is_empty() {
        assert!(render_notes(&[], MAX_DISCUSSION_CHARS).is_empty());
    }
}
