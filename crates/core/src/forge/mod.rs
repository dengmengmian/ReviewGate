//! 代码托管平台（forge）PR/MR 评论：GitHub / GitLab / AtomGit。
//!
//! 一条 Markdown **摘要评论**是核心闸口信号，三平台都支持，且**就地更新同一条**
//! （靠 [`SUMMARY_MARKER`]），不会每次 push 追加。
//!
//! **行内评论**支持 GitHub（reviews 接口一次性批量提交）与 GitLab（discussions 逐条提交，
//! 平台无批量接口）；AtomGit 的行内接口未经验证，只发摘要。行内评论正文埋了发现指纹，
//! 已发过的下一轮自动跳过。
//!
//! 平台识别：显式 `REVIEWGATE_FORGE`（github|gitlab|atomgit）优先；否则按 CI 环境自动识别
//! （GitHub Actions 的 `GITHUB_*`、GitLab CI 的 `CI_*`）。AtomGit 无公开 CI 约定，走显式变量。

pub mod discussion;
pub mod i18n;
pub mod local;

use i18n::MdLang;

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

/// 摘要评论的 POST 端点 URL。列出已有评论用同一个 URL（GET）。
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

/// 摘要评论**就地更新**的 (HTTP method, URL)。
///
/// `None` = 该平台没有经过验证的更新接口，调用方退回追加一条新评论。AtomGit 属此类——
/// 不猜 API 形状，宁可多一条评论也不发到错误的端点。
pub fn summary_update_endpoint(
    forge: Forge,
    api_base: &str,
    repo: &str,
    number: u64,
    comment_id: u64,
) -> Option<(&'static str, String)> {
    let base = api_base.trim_end_matches('/');
    match forge {
        // GitHub 的 issue 评论按评论 id 更新，不带 issue 号。
        Forge::GitHub => Some((
            "PATCH",
            format!("{base}/repos/{repo}/issues/comments/{comment_id}"),
        )),
        Forge::GitLab => Some((
            "PUT",
            format!("{base}/projects/{repo}/merge_requests/{number}/notes/{comment_id}"),
        )),
        Forge::AtomGit => None,
    }
}

/// 鉴权请求头 (name, value)。GitLab 用 `PRIVATE-TOKEN`；GitHub/AtomGit 用 `Authorization: Bearer`。
pub fn auth_header(forge: Forge, token: &str) -> (&'static str, String) {
    match forge {
        Forge::GitLab => ("PRIVATE-TOKEN", token.to_string()),
        Forge::GitHub | Forge::AtomGit => ("Authorization", format!("Bearer {token}")),
    }
}

/// 摘要评论的隐藏标记：下一轮据此找回同一条评论就地更新，而不是每次 push 追加一条。
/// 三平台的 Markdown 都会把 HTML 注释渲染为不可见。
pub const SUMMARY_MARKER: &str = "<!-- reviewgate:summary -->";

/// 从评论列表 JSON 里找出**我们自己最后一条**摘要评论的 id。
///
/// GitHub 的 issue 评论与 GitLab 的 MR note 列表都是 `[{id, body}]`，同一个解析器够用。
/// 解析失败/找不到都返回 `None`——调用方退回追加一条新评论，不会因此中断。
pub fn find_marked_comment_id(json: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct Item {
        id: u64,
        #[serde(default)]
        body: Option<String>,
    }
    let items: Vec<Item> = serde_json::from_str(json).ok()?;
    items
        .into_iter()
        .rfind(|c| {
            c.body
                .as_deref()
                .is_some_and(|b| b.contains(SUMMARY_MARKER))
        })
        .map(|c| c.id)
}

/// 把审查结果渲染成 Markdown 摘要（平台中立，三平台通用）。
/// 散文跟随 `output_language()`；维度/severity/路径等技术标识保持英文。
pub fn render_markdown(outcome: &ReviewOutcome) -> String {
    render_markdown_lang(outcome, MdLang::detect())
}

/// 同 [`render_markdown`]，语言可注入（测试用，避免依赖进程 locale）。
pub fn render_markdown_lang(outcome: &ReviewOutcome, t: MdLang) -> String {
    let badge = match outcome.decision {
        GateDecision::Pass => t.badge_pass(),
        GateDecision::Warn => t.badge_warn(),
        GateDecision::Block => t.badge_block(),
    };
    let kept: Vec<&Finding> = outcome.findings.iter().filter(|f| !f.filtered).collect();
    let filtered = outcome.findings.len() - kept.len();

    let mut md = String::new();
    md.push_str(SUMMARY_MARKER);
    md.push('\n');
    md.push_str("## 🚪 ReviewGate\n\n");
    md.push_str(&format!(
        "{badge}\n\n{}\n\n",
        t.counts(outcome.files_changed, kept.len(), filtered)
    ));
    if !outcome.scope.is_empty() {
        md.push_str(&format!("{}\n\n", t.scope(&outcome.scope)));
    }

    if !outcome.excluded.is_empty() {
        // 少审了哪些文件必须写进 PR 评论——否则 reviewer 看到 PASS 会以为全审过了。
        let list = outcome
            .excluded
            .iter()
            .take(10)
            .map(|e| format!("`{}` ({})", e.path, e.reason.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let more = if outcome.excluded.len() > 10 {
            t.excluded_more(outcome.excluded.len())
        } else {
            String::new()
        };
        md.push_str(&format!("> {} {list}{more}\n\n", t.excluded_label()));
    }

    if outcome.incomplete {
        md.push_str(t.incomplete_note());
        let unfinished = crate::review::unfinished_paths(&outcome.warnings);
        if !unfinished.is_empty() {
            md.push_str(&format!(
                "{} {}\n\n",
                t.uncovered_paths(),
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
                format!("`{}` ({}{})", w.dimension, w.kind, paths)
            })
            .collect();
        md.push_str(&format!("{} {}.\n\n", t.incomplete_list(), list.join(", ")));
    }
    if outcome.critical_incomplete {
        md.push_str(t.critical_incomplete());
    }

    if kept.is_empty() {
        md.push_str(t.no_issues());
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

/// 解析评论上下文：**CI 环境变量优先**（行为与之前完全一致），解析不出时回退到本地
/// 已认证的 `gh` / `glab` CLI。这样本地跑 `--comment` 不必再单独配一份 token。
pub async fn resolve_context_any() -> Option<ForgeContext> {
    if let Some(ctx) = resolve_context(|k| std::env::var(k).ok(), detect_pr_number()) {
        return Some(ctx);
    }
    local::resolve_context_from_cli().await
}

/// 发/更新摘要评论到检测到的 forge（GitHub/GitLab/AtomGit）。非 PR/MR 上下文则跳过。
///
/// **就地更新**：找得到上一轮带 [`SUMMARY_MARKER`] 的评论就改它，PR 上永远只有一条
/// ReviewGate 摘要；找不到（首轮、拉取失败、AtomGit 无更新接口）才追加一条新的。
pub async fn post_summary(outcome: &ReviewOutcome) -> Result<()> {
    let Some(ctx) = resolve_context_any().await else {
        eprintln!(
            "no forge PR/MR context detected; skipping summary comment.\n  \
             In CI set REVIEWGATE_TOKEN (+ repo/PR vars); locally make sure `gh`/`glab` is \
             authenticated and the current branch has an open PR/MR."
        );
        return Ok(());
    };

    let body = render_markdown(outcome);
    let url = summary_endpoint(ctx.forge, &ctx.api_base, &ctx.repo, ctx.number);
    let (hname, hval) = auth_header(ctx.forge, &ctx.token);
    let client = crate::llm::http::shared_http_client()?;

    // 上一轮的摘要评论。拉取失败不是错误——退回追加，最坏是多一条评论。
    // 没有可用更新接口的平台（AtomGit）直接跳过这次拉取，不做无用请求。
    let can_update =
        summary_update_endpoint(ctx.forge, &ctx.api_base, &ctx.repo, ctx.number, 0).is_some();
    let existing = if !can_update {
        None
    } else {
        match fetch_comment_bodies(&ctx, &url).await {
            // 从最后一页往回找：要的是**最新**那条摘要。
            Ok(pages) => pages.iter().rev().find_map(|p| find_marked_comment_id(p)),
            Err(e) => {
                eprintln!("  [forge] could not list existing comments ({e}); posting a new one.");
                None
            }
        }
    };
    let update = existing.and_then(|id| {
        summary_update_endpoint(ctx.forge, &ctx.api_base, &ctx.repo, ctx.number, id)
    });

    let req = match &update {
        Some(("PUT", u)) => client.put(u),
        Some((_, u)) => client.patch(u),
        None => client.post(&url),
    };
    let resp = req
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
        "{} ReviewGate summary comment ({:?} #{}).",
        if update.is_some() {
            "Updated"
        } else {
            "Posted"
        },
        ctx.forge,
        ctx.number
    );
    Ok(())
}

/// 分页拉取一个评论列表端点的原始 JSON（每页一个字符串）。
///
/// 长 PR 的评论可能超过一页，摘要评论恰恰在最后一页——不翻页就会每次都当成"首轮"
/// 再追加一条。上限 10 页（1000 条）兜底，避免异常仓库把 CI 拖住。
async fn fetch_comment_bodies(ctx: &ForgeContext, url: &str) -> Result<Vec<String>> {
    const PER_PAGE: usize = 100;
    const MAX_PAGES: usize = 10;
    let (hname, hval) = auth_header(ctx.forge, &ctx.token);
    let client = crate::llm::http::shared_http_client()?;
    let mut pages = Vec::new();
    for page in 1..=MAX_PAGES {
        let resp = client
            .get(url)
            .header(hname, hval.clone())
            .header("User-Agent", "ReviewGate")
            .query(&[
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ])
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("HTTP {}", resp.status());
        }
        let text = resp.text().await?;
        let count = serde_json::from_str::<Vec<serde_json::Value>>(&text)
            .map(|v| v.len())
            .unwrap_or(0);
        pages.push(text);
        if count < PER_PAGE {
            break;
        }
    }
    Ok(pages)
}

/// 一条已经映射到 diff 锚点、可直接序列化成各平台请求体的行内评论。
///
/// 平台中立：GitHub 只用 `path`/`line`/`start_line`；GitLab 的 position 还需要
/// `old_path`（重命名）与 `old_line`（上下文行两侧都得给，否则解析不出位置）。
#[derive(Debug, Clone)]
pub struct InlineComment<'a> {
    pub finding: &'a Finding,
    /// 新侧路径。
    pub path: String,
    /// 基线侧路径；未重命名时与 `path` 相同。
    pub old_path: String,
    /// 锚点行（新文件行号）。多行发现锚在末行。
    pub line: u32,
    /// 上下文行在旧文件的行号；新增行为 `None`。
    pub old_line: Option<u32>,
    /// 多行发现的起始行；单行为 `None`。
    pub start_line: Option<u32>,
}

/// 行内评论候选：已定位、未过滤，且 **high 或达到 block 置信度**（闸口级问题落到 PR 行上）。
/// 意图维度走验收清单，不发行内 suggestion。
pub fn inline_candidates(outcome: &ReviewOutcome, block_threshold: f32) -> Vec<&Finding> {
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

/// 把候选发现映射到 diff 锚点上。
///
/// **锚不住的直接丢掉**：行内评论是批量提交的，一条位置非法就会让整批被平台拒绝，
/// 结果是一条行内评论都发不出去。被丢掉的发现仍然完整列在摘要评论的表格里。
///
/// `anchors` 为 `None` 表示不知道 diff 形状（未走完整审查管线）——此时不做校验，
/// 按旧行为原样放行，两侧路径视为同名、无上下文行号。
pub fn map_inline<'a>(
    candidates: &[&'a Finding],
    anchors: Option<&crate::diff::DiffAnchors>,
) -> Vec<InlineComment<'a>> {
    candidates
        .iter()
        .filter_map(|f| {
            let line = f.end_line.max(f.start_line);
            let start_line = (f.end_line > f.start_line).then_some(f.start_line);
            let Some(anchors) = anchors else {
                return Some(InlineComment {
                    finding: f,
                    path: f.path.clone(),
                    old_path: f.path.clone(),
                    line,
                    old_line: None,
                    start_line,
                });
            };
            let file = anchors.get(&f.path)?;
            // 多行发现：两端都必须在 diff 上，否则平台会拒绝整个区间。
            if let Some(s) = start_line {
                if !file.lines.contains_key(&s) {
                    return None;
                }
            }
            let old_line = *file.lines.get(&line)?;
            Some(InlineComment {
                finding: f,
                path: f.path.clone(),
                old_path: file.old_path.clone(),
                line,
                old_line,
                start_line,
            })
        })
        .collect()
}

/// 行内评论指纹标记：下一轮据此认出"这条我已经发过了"，避免每次 push 重复刷同一条。
fn fp_marker(fingerprint: &str) -> String {
    format!("<!-- reviewgate:fp:{fingerprint} -->")
}

/// 一条行内评论的正文（含 ` ```suggestion ` 块与幂等指纹）。
pub fn inline_body(f: &Finding) -> String {
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
    body.push_str(&format!(
        "\n\n{}",
        fp_marker(&crate::review::fingerprint(f))
    ));
    body
}

/// 从已有评论列表 JSON 里收集 ReviewGate 发过的行内评论指纹。
///
/// 只认自己埋的标记，不做任何文本相似度匹配——相似度误判等于给闸口开后门。
pub fn posted_fingerprints(json: &str) -> std::collections::HashSet<String> {
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        body: Option<String>,
    }
    const PREFIX: &str = "<!-- reviewgate:fp:";
    serde_json::from_str::<Vec<Item>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            let body = c.body?;
            let rest = body.split(PREFIX).nth(1)?;
            Some(rest.split_once(" -->")?.0.trim().to_string())
        })
        .collect()
}

/// 滤掉上一轮已经发过的发现（指纹不含行号，代码没改就一直命中）。
pub fn drop_already_posted<'a>(
    candidates: Vec<&'a Finding>,
    posted: &std::collections::HashSet<String>,
) -> Vec<&'a Finding> {
    candidates
        .into_iter()
        .filter(|f| !posted.contains(&crate::review::fingerprint(f)))
        .collect()
}

/// GitLab position 需要的三个 sha。缺一个都定位不了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLabDiffRefs {
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
}

/// 从 `GET /projects/:id/merge_requests/:iid` 的响应里取 `diff_refs`。
pub fn parse_gitlab_diff_refs(json: &str) -> Option<GitLabDiffRefs> {
    #[derive(serde::Deserialize)]
    struct Refs {
        base_sha: String,
        start_sha: String,
        head_sha: String,
    }
    #[derive(serde::Deserialize)]
    struct Mr {
        diff_refs: Option<Refs>,
    }
    let r = serde_json::from_str::<Mr>(json).ok()?.diff_refs?;
    Some(GitLabDiffRefs {
        base_sha: r.base_sha,
        start_sha: r.start_sha,
        head_sha: r.head_sha,
    })
}

/// GitHub 创建 review 的请求体：**一次请求带上全部行内评论**。
///
/// 逐条发会产生 N 个请求、N 封通知，且没有原子性。`event` 固定 `COMMENT`——
/// 闸口结论由 exit code 与摘要评论表达，不擅自替人按下 request-changes。
/// `COMMENT` 事件的 body 不能为空，故这里给一句指回摘要的短说明。
pub fn build_review_payload(commit_id: &str, comments: &[InlineComment]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = comments
        .iter()
        .map(|c| {
            let mut o = serde_json::json!({
                "path": c.path,
                "line": c.line,
                "side": "RIGHT",
                "body": inline_body(c.finding),
            });
            if let Some(s) = c.start_line {
                o["start_line"] = serde_json::json!(s);
                o["start_side"] = serde_json::json!("RIGHT");
            }
            o
        })
        .collect();
    serde_json::json!({
        "commit_id": commit_id,
        "body": format!(
            "**ReviewGate** anchored {} gate-level finding(s) inline. Full report in the summary comment.",
            items.len()
        ),
        "event": "COMMENT",
        "comments": items,
    })
}

/// GitLab 创建行内 discussion 的请求体。
///
/// GitLab 没有批量接口，只能逐条 POST；位置由 `position` 对象表达，三个 sha 与两侧
/// 路径都必须给。多行发现锚在末行——`line_range` 需要 SHA1 line_code，为此引一个
/// 哈希依赖不划算，区间信息已经写在正文里。
pub fn build_gitlab_discussion_payload(
    comment: &InlineComment,
    refs: &GitLabDiffRefs,
) -> serde_json::Value {
    let mut position = serde_json::json!({
        "position_type": "text",
        "base_sha": refs.base_sha,
        "start_sha": refs.start_sha,
        "head_sha": refs.head_sha,
        "old_path": comment.old_path,
        "new_path": comment.path,
        "new_line": comment.line,
    });
    // 上下文行两侧行号都要给；新增行只有新侧，带上 old_line 反而定位失败。
    if let Some(old) = comment.old_line {
        position["old_line"] = serde_json::json!(old);
    }
    serde_json::json!({
        "body": inline_body(comment.finding),
        "position": position,
    })
}

/// 在 PR/MR 上为闸口级发现发**行内评论**，带 ` ```suggestion ` 块（一键应用，人把关）。
///
/// GitHub 走 reviews 接口一次性提交；GitLab 走 discussions 逐条提交（平台无批量接口）；
/// AtomGit 的行内接口未经验证，只在摘要评论里给出发现。
/// 已经发过的（指纹命中）跳过，不会每次 push 重复刷。
///
/// `block_threshold` 应来自闸口配置（`gate.block_threshold`），与 PASS/BLOCK 判定一致。
pub async fn post_inline_suggestions(outcome: &ReviewOutcome, block_threshold: f32) -> Result<()> {
    let Some(ctx) = resolve_context_any().await else {
        return Ok(());
    };
    if ctx.forge == Forge::AtomGit {
        eprintln!("inline comments are not supported on AtomGit yet; skipping.");
        return Ok(());
    }

    let candidates = inline_candidates(outcome, block_threshold);
    if candidates.is_empty() {
        eprintln!("No high/BLOCK findings with line anchors; skipping inline comments.");
        return Ok(());
    }

    // 已发过的跳过。拉取失败不阻断——最坏是重复一条评论，比漏发闸口级发现好。
    let list_url = inline_list_endpoint(&ctx);
    let posted = match fetch_comment_bodies(&ctx, &list_url).await {
        Ok(pages) => pages.iter().flat_map(|p| posted_fingerprints(p)).collect(),
        Err(e) => {
            eprintln!("  [forge] could not list existing inline comments ({e}); may repost.");
            std::collections::HashSet::new()
        }
    };
    let total = candidates.len();
    let candidates = drop_already_posted(candidates, &posted);
    let skipped_dup = total - candidates.len();

    let mapped = map_inline(&candidates, outcome.diff_anchors.as_ref());
    let unanchored = candidates.len() - mapped.len();
    if unanchored > 0 {
        eprintln!(
            "  [forge] {unanchored} finding(s) could not be anchored to the reviewed diff; \
             they stay in the summary table only."
        );
    }
    if mapped.is_empty() {
        eprintln!("No inline comments to post ({skipped_dup} already posted).");
        return Ok(());
    }

    let posted_count = match ctx.forge {
        Forge::GitHub => post_github_review(&ctx, &mapped).await?,
        Forge::GitLab => post_gitlab_discussions(&ctx, &mapped).await?,
        Forge::AtomGit => 0,
    };
    eprintln!(
        "Posted {posted_count} inline comment(s) (high/BLOCK only; {skipped_dup} already posted)."
    );
    Ok(())
}

/// 已有行内评论的列表端点（用于指纹去重）。
fn inline_list_endpoint(ctx: &ForgeContext) -> String {
    let base = ctx.api_base.trim_end_matches('/');
    match ctx.forge {
        Forge::GitHub => format!("{base}/repos/{}/pulls/{}/comments", ctx.repo, ctx.number),
        // GitLab 的 notes 列表是扁平的，行内 discussion 的 note 也在里面。
        _ => format!(
            "{base}/projects/{}/merge_requests/{}/notes",
            ctx.repo, ctx.number
        ),
    }
}

/// GitHub：一次请求提交整批行内评论。
async fn post_github_review(ctx: &ForgeContext, mapped: &[InlineComment<'_>]) -> Result<usize> {
    let Some(commit_id) = detect_head_sha() else {
        eprintln!("could not get the PR head commit sha; skipping inline comments.");
        return Ok(0);
    };
    let url = format!(
        "{}/repos/{}/pulls/{}/reviews",
        ctx.api_base.trim_end_matches('/'),
        ctx.repo,
        ctx.number
    );
    let client = crate::llm::http::shared_http_client()?;
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", ctx.token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "ReviewGate")
        .json(&build_review_payload(&commit_id, mapped))
        .send()
        .await
        .context("failed to create the GitHub review")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // 整批被拒时说清楚代价：这些发现仍在摘要表格里，不是丢了。
        anyhow::bail!(
            "GitHub review returned {status}: {}\n  \
             {} inline comment(s) were not posted; the findings remain in the summary comment.",
            text.chars().take(300).collect::<String>(),
            mapped.len()
        );
    }
    Ok(mapped.len())
}

/// GitLab：逐条 POST discussion（平台没有批量接口），单条失败不影响其它。
async fn post_gitlab_discussions(
    ctx: &ForgeContext,
    mapped: &[InlineComment<'_>],
) -> Result<usize> {
    let base = ctx.api_base.trim_end_matches('/');
    let client = crate::llm::http::shared_http_client()?;
    let (hname, hval) = auth_header(ctx.forge, &ctx.token);

    // position 必须带 MR 的三个 sha，只能从 MR 详情拿——取不到就不发，绝不猜。
    let mr_url = format!("{base}/projects/{}/merge_requests/{}", ctx.repo, ctx.number);
    let refs = match client
        .get(&mr_url)
        .header(hname, hval.clone())
        .header("User-Agent", "ReviewGate")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            parse_gitlab_diff_refs(&r.text().await.unwrap_or_default())
        }
        Ok(r) => {
            eprintln!("  [forge] GET merge request returned {}", r.status());
            None
        }
        Err(e) => {
            eprintln!("  [forge] GET merge request failed: {e}");
            None
        }
    };
    let Some(refs) = refs else {
        eprintln!("could not read the MR diff_refs; skipping inline comments.");
        return Ok(0);
    };

    let url = format!(
        "{base}/projects/{}/merge_requests/{}/discussions",
        ctx.repo, ctx.number
    );
    let mut posted = 0usize;
    for c in mapped {
        let resp = client
            .post(&url)
            .header(hname, hval.clone())
            .header("User-Agent", "ReviewGate")
            .json(&build_gitlab_discussion_payload(c, &refs))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => posted += 1,
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                eprintln!(
                    "inline comment failed {}:{} -> {status}: {}",
                    c.path,
                    c.line,
                    text.chars().take(160).collect::<String>()
                );
            }
            Err(e) => eprintln!("inline comment request failed {}:{} -> {e}", c.path, c.line),
        }
    }
    Ok(posted)
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
    fn markdown_states_the_reviewed_scope() {
        // 增量审查的 PASS 只对某个范围成立；PR 评论必须写清楚审的是哪一段。
        let mut o = outcome_with(vec![], GateDecision::Pass, false);
        o.scope = "since last review (abc123def456) incl. working tree".into();
        let md = render_markdown_lang(&o, MdLang::En);
        assert!(md.contains("Scope:"), "{md}");
        assert!(md.contains("abc123def456"), "{md}");
        let zh = render_markdown_lang(&o, MdLang::Zh);
        assert!(zh.contains("审查范围"), "{zh}");
    }

    #[test]
    fn markdown_lists_excluded_files_so_a_pass_is_not_misread() {
        let mut o = outcome_with(vec![], GateDecision::Pass, false);
        o.excluded = vec![crate::diff::ExcludedFile {
            path: "vendor/dep.go".into(),
            reason: crate::diff::ExcludeReason::Builtin,
        }];
        let md = render_markdown_lang(&o, MdLang::En);
        assert!(md.contains("Not reviewed"), "{md}");
        assert!(md.contains("vendor/dep.go"), "{md}");
    }

    #[test]
    fn markdown_summary_pass_no_issues() {
        let md = render_markdown_lang(&outcome_with(vec![], GateDecision::Pass, false), MdLang::En);
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
        let md = render_markdown_lang(&o, MdLang::En);
        assert!(md.contains("WARN"));
        assert!(md.contains("Review incomplete"));
        assert!(md.contains("timed_out"));
        assert!(md.contains("1 filtered"));

        // 同一结果的中文版：散文换语言，技术标识不变。
        let zh = render_markdown_lang(&o, MdLang::Zh);
        assert!(zh.contains("审查未完整"));
        assert!(zh.contains("1 条已过滤"));
        assert!(zh.contains("timed_out"), "维度/kind 等技术标识保持英文");
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
        let md = render_markdown_lang(
            &outcome_with(vec![f], GateDecision::Block, false),
            MdLang::En,
        );
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
        let md = render_markdown_lang(
            &outcome_with(vec![f1, f2], GateDecision::Block, false),
            MdLang::En,
        );
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

    // ---- 摘要评论 upsert（同一条评论就地更新，而不是每次 push 刷一条）----

    #[test]
    fn markdown_carries_the_upsert_marker() {
        let md = render_markdown_lang(&outcome_with(vec![], GateDecision::Pass, false), MdLang::En);
        assert!(md.contains(SUMMARY_MARKER), "{md}");
        // 自我识别串必须保留：discussion.rs 靠它把自己上一轮的输出挡在 prompt 之外。
        assert!(md.contains("🚪 ReviewGate"), "{md}");
    }

    #[test]
    fn find_marked_comment_picks_our_latest_summary() {
        let json = format!(
            r#"[
              {{"id": 1, "body": "looks good to me"}},
              {{"id": 2, "body": "{SUMMARY_MARKER}\n## 🚪 ReviewGate\n旧的一轮"}},
              {{"id": 3, "body": "另一个人的评论"}},
              {{"id": 4, "body": "{SUMMARY_MARKER}\n## 🚪 ReviewGate\n新的一轮"}}
            ]"#
        );
        assert_eq!(find_marked_comment_id(&json), Some(4));
    }

    #[test]
    fn find_marked_comment_none_when_absent_or_unparsable() {
        assert_eq!(find_marked_comment_id(r#"[{"id":1,"body":"hi"}]"#), None);
        assert_eq!(find_marked_comment_id("not json"), None);
    }

    #[test]
    fn summary_update_endpoint_per_forge() {
        assert_eq!(
            summary_update_endpoint(Forge::GitHub, "https://api.github.com", "o/r", 7, 99),
            Some((
                "PATCH",
                "https://api.github.com/repos/o/r/issues/comments/99".into()
            ))
        );
        assert_eq!(
            summary_update_endpoint(Forge::GitLab, "https://gitlab.com/api/v4/", "42", 9, 99),
            Some((
                "PUT",
                "https://gitlab.com/api/v4/projects/42/merge_requests/9/notes/99".into()
            ))
        );
        // AtomGit 的评论更新 API 未经验证——不猜，退回追加一条新评论。
        assert_eq!(
            summary_update_endpoint(
                Forge::AtomGit,
                "https://api.atomgit.com/api/v5",
                "o/r",
                3,
                99
            ),
            None
        );
    }

    // ---- 行内评论：先映射到 diff 锚点，再一次性批量提交 ----

    fn file_anchors(old_path: &str, lines: &[(u32, Option<u32>)]) -> crate::diff::FileAnchors {
        crate::diff::FileAnchors {
            old_path: old_path.to_string(),
            lines: lines.iter().copied().collect(),
        }
    }

    fn anchors(pairs: &[(&str, crate::diff::FileAnchors)]) -> crate::diff::DiffAnchors {
        pairs
            .iter()
            .map(|(p, a)| (p.to_string(), a.clone()))
            .collect()
    }

    #[test]
    fn map_inline_drops_findings_not_anchored_on_the_diff() {
        let on_diff = base_finding(); // a.rs:3
        let mut off_diff = base_finding();
        off_diff.start_line = 900;
        off_diff.end_line = 900;
        let mut other_file = base_finding();
        other_file.path = "b.rs".into();
        let a = anchors(&[(
            "a.rs",
            file_anchors("a.rs", &[(1, Some(1)), (2, None), (3, None)]),
        )]);
        let mapped = map_inline(&[&on_diff, &off_diff, &other_file], Some(&a));
        assert_eq!(mapped.len(), 1, "只保留落在 diff 行上的发现");
        assert_eq!(mapped[0].line, 3);
    }

    #[test]
    fn map_inline_requires_both_ends_of_a_range_on_the_diff() {
        let mut f = base_finding();
        f.start_line = 3;
        f.end_line = 7; // 7 不在 diff 上 → GitHub 会 422，整批被拒
        let a = anchors(&[("a.rs", file_anchors("a.rs", &[(3, None)]))]);
        assert!(map_inline(&[&f], Some(&a)).is_empty());
    }

    #[test]
    fn map_inline_carries_rename_and_context_line_for_gitlab_positions() {
        let f = base_finding(); // a.rs:3
        let a = anchors(&[(
            "a.rs",
            // 重命名：旧侧叫 old_a.rs；第 3 行是上下文行，旧侧行号 9。
            file_anchors("old_a.rs", &[(3, Some(9))]),
        )]);
        let mapped = map_inline(&[&f], Some(&a));
        assert_eq!(mapped[0].path, "a.rs");
        assert_eq!(mapped[0].old_path, "old_a.rs");
        assert_eq!(mapped[0].old_line, Some(9));
    }

    #[test]
    fn map_inline_passes_everything_through_when_anchors_are_unknown() {
        // anchors=None 表示"不知道 diff 长什么样"，不是"没有可评论的行"——保持旧行为。
        let f = base_finding();
        let mapped = map_inline(&[&f], None);
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].old_path, "a.rs", "未知重命名信息时两侧同名");
        assert_eq!(mapped[0].old_line, None);
    }

    #[test]
    fn review_payload_batches_every_comment_into_one_review() {
        let f1 = base_finding();
        let mut f2 = base_finding();
        f2.path = "b.rs".into();
        f2.start_line = 10;
        f2.end_line = 12;
        f2.suggestion_code = "let x = 1;".into();
        let mapped = map_inline(&[&f1, &f2], None);
        let payload = build_review_payload("sha123", &mapped);

        assert_eq!(payload["commit_id"], "sha123");
        assert_eq!(payload["event"], "COMMENT");
        // COMMENT 事件的 body 不能为空，否则 GitHub 422。
        assert!(!payload["body"].as_str().unwrap().is_empty());

        let comments = payload["comments"].as_array().expect("comments 数组");
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0]["path"], "a.rs");
        assert_eq!(comments[0]["line"], 3);
        assert_eq!(comments[0]["side"], "RIGHT");
        assert!(
            comments[0].get("start_line").is_none(),
            "单行不带 start_line"
        );
        assert_eq!(comments[1]["line"], 12);
        assert_eq!(comments[1]["start_line"], 10);
        assert_eq!(comments[1]["start_side"], "RIGHT");
        assert!(comments[1]["body"]
            .as_str()
            .unwrap()
            .contains("```suggestion"));
    }

    // ---- GitLab 行内：discussions + position ----

    fn refs() -> GitLabDiffRefs {
        GitLabDiffRefs {
            base_sha: "base1".into(),
            start_sha: "start1".into(),
            head_sha: "head1".into(),
        }
    }

    #[test]
    fn gitlab_position_carries_three_shas_and_both_paths() {
        let f = base_finding();
        let a = anchors(&[("a.rs", file_anchors("old_a.rs", &[(3, None)]))]);
        let mapped = map_inline(&[&f], Some(&a));
        let payload = build_gitlab_discussion_payload(&mapped[0], &refs());
        let pos = &payload["position"];
        assert_eq!(pos["position_type"], "text");
        assert_eq!(pos["base_sha"], "base1");
        assert_eq!(pos["start_sha"], "start1");
        assert_eq!(pos["head_sha"], "head1");
        assert_eq!(pos["new_path"], "a.rs");
        assert_eq!(pos["old_path"], "old_a.rs");
        assert_eq!(pos["new_line"], 3);
        assert!(
            pos.get("old_line").is_none(),
            "新增行只有新侧行号，带上 old_line 会定位失败"
        );
        assert!(!payload["body"].as_str().unwrap().is_empty());
    }

    #[test]
    fn gitlab_position_includes_old_line_for_context_lines() {
        // 上下文行必须两侧行号都给，否则 GitLab 无法解析 position。
        let f = base_finding();
        let a = anchors(&[("a.rs", file_anchors("a.rs", &[(3, Some(9))]))]);
        let mapped = map_inline(&[&f], Some(&a));
        let payload = build_gitlab_discussion_payload(&mapped[0], &refs());
        assert_eq!(payload["position"]["old_line"], 9);
        assert_eq!(payload["position"]["new_line"], 3);
    }

    #[test]
    fn gitlab_anchors_a_multi_line_finding_at_its_end_line() {
        let mut f = base_finding();
        f.start_line = 10;
        f.end_line = 12;
        let mapped = map_inline(&[&f], None);
        let payload = build_gitlab_discussion_payload(&mapped[0], &refs());
        assert_eq!(payload["position"]["new_line"], 12);
    }

    #[test]
    fn gitlab_diff_refs_parsed_from_merge_request() {
        let json = r#"{"diff_refs":{"base_sha":"b","start_sha":"s","head_sha":"h"}}"#;
        let r = parse_gitlab_diff_refs(json).expect("应解析出 diff_refs");
        assert_eq!(r.base_sha, "b");
        assert_eq!(r.start_sha, "s");
        assert_eq!(r.head_sha, "h");
        assert!(parse_gitlab_diff_refs(r#"{"iid":1}"#).is_none());
    }

    // ---- 幂等：同一处发现不该每次 push 重发一遍 ----

    #[test]
    fn inline_body_embeds_a_stable_fingerprint() {
        let f = base_finding();
        let body = inline_body(&f);
        let fp = crate::review::fingerprint(&f);
        assert!(body.contains(&fp_marker(&fp)), "{body}");
        // 行号漂移不改变指纹——下一轮同一处问题仍能识别为"已发过"。
        let mut moved = base_finding();
        moved.start_line = 42;
        moved.end_line = 42;
        assert!(inline_body(&moved).contains(&fp_marker(&fp)));
    }

    #[test]
    fn already_posted_fingerprints_are_not_re_posted() {
        let f = base_finding();
        let fp = crate::review::fingerprint(&f);
        let json = format!(
            r#"[{{"body": "旧评论 {}"}}, {{"body": "无关"}}]"#,
            fp_marker(&fp)
        );
        let posted = posted_fingerprints(&json);
        assert!(posted.contains(&fp));

        let fresh = drop_already_posted(vec![&f], &posted);
        assert!(fresh.is_empty(), "同一处发现不该每次 push 重复发一遍");
        assert_eq!(
            drop_already_posted(vec![&f], &std::collections::HashSet::new()).len(),
            1
        );
    }
}
