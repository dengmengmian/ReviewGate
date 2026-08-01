//! Phase 2 技术验证。
//!
//! - **Level 0**：只读 `git grep` / `git log`，定位错误文案与相关提交。
//! - **Level 1（深挖）**：当像真 BUG 时，展开命中周围函数体、找调用方、读文件历史，
//!   把多行上下文交给 explain/LLM。不执行用户 shell。

use super::model::{IssueType, IssueVerdict, NormalizedIssue};
use crate::index::list_function_bodies;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InvestigationPlan {
    pub steps: Vec<String>,
    pub search_terms: Vec<String>,
    pub likely_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodeHit {
    pub path: String,
    pub line: u32,
    pub snippet: String,
    pub source: String,
}

/// Level 1：围绕一条 Level0 命中展开的代码上下文。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeepDigBlock {
    pub path: String,
    /// 原始 grep 锚点行。
    pub anchor_line: u32,
    /// 包围函数名（若解析到），否则是锚点附近抽到的标识符。
    pub symbol: Option<String>,
    /// `symbol` 是否真的是解析出来的包围函数。false 表示只是锚点附近的标识符——
    /// 措辞上不能说「函数 X 中」，那会把一个 var 声明说成函数体。
    #[serde(default)]
    pub symbol_is_fn: bool,
    pub start_line: u32,
    pub end_line: u32,
    /// 多行代码上下文（已截断）。
    pub context: String,
    /// 调用方 / 引用点。
    pub callers: Vec<CodeHit>,
    /// 该文件近期提交。
    pub file_commits: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TechnicalVerification {
    pub enabled: bool,
    pub skipped_reason: Option<String>,
    pub plan: InvestigationPlan,
    pub code_hits: Vec<CodeHit>,
    pub git_commits: Vec<String>,
    pub test_hits: Vec<CodeHit>,
    pub fix_prs: Vec<String>,
    pub evidence: Vec<String>,
    pub technical_verdict: IssueVerdict,
    pub confidence: f32,
    /// Level 1 深挖是否执行。
    #[serde(default)]
    pub deep_dig_ran: bool,
    /// Level 1 展开块（供 explain / 调试）。
    #[serde(default)]
    pub deep_dig: Vec<DeepDigBlock>,
}

/// 是否应进入代码验证。
/// `can_verify`：完整度模块放行（含「标题可检索」覆盖），优先于纯分数阈值。
#[allow(clippy::too_many_arguments)]
pub fn should_verify(
    issue_type: IssueType,
    spam_score: f32,
    completeness_score: f32,
    can_verify: bool,
    duplicate_confidence: f32,
    is_probable_dup: bool,
    config_enabled: bool,
    repo_accessible: bool,
) -> (bool, Option<String>) {
    if !config_enabled {
        return (false, Some("verification_disabled".into()));
    }
    if !repo_accessible {
        return (false, Some("repo_not_accessible".into()));
    }
    if spam_score >= 0.5 {
        return (false, Some("spam_gate".into()));
    }
    if is_probable_dup && duplicate_confidence >= 0.9 {
        return (false, Some("high_confidence_duplicate".into()));
    }
    if completeness_score < 0.6 && !can_verify {
        return (false, Some("completeness_below_threshold".into()));
    }
    let ok = matches!(
        issue_type,
        IssueType::Bug | IssueType::Security | IssueType::Performance | IssueType::Compatibility
    );
    if !ok {
        return (
            false,
            Some(format!("type_not_verifiable:{}", issue_type.as_str())),
        );
    }
    (true, None)
}

/// 根据 Issue 生成调查计划。
pub fn build_plan(n: &NormalizedIssue) -> InvestigationPlan {
    let mut terms = Vec::new();
    for e in &n.error_signatures {
        if e.len() >= 3 {
            terms.push(e.clone());
        }
    }
    for s in &n.stack_symbols {
        // 取最后一段符号
        let last = s.rsplit([' ', '/', ':']).find(|t| t.len() > 2).unwrap_or(s);
        if last.len() >= 3 && last.len() < 80 {
            terms.push(last.to_string());
        }
    }
    // 标题 + 正文高频技术词（含中文错误文案）
    let blob = format!("{}\n{}\n{}", n.title, n.symptom, n.body_clean);
    for t in [
        "retry",
        "reconnect",
        "timeout",
        "stream",
        "http",
        "sse",
        "openai",
        "deepseek",
        "provider",
        "gateway",
        "build_http_client",
        "chat_stream",
        "error decoding",
        "connection",
        "重试",
        "重连",
        "超时",
        "连接",
        "网络",
    ] {
        if blob.to_ascii_lowercase().contains(&t.to_ascii_lowercase()) || blob.contains(t) {
            terms.push(t.to_string());
        }
    }
    // 标题关键词（含 snake_case 工具名）
    for t in n.title.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if t.len() >= 4 && !STOP.contains(&t.to_ascii_lowercase().as_str()) {
            terms.push(t.to_string());
        }
    }
    // 明确实体加检索词
    let title_l = n.title.to_ascii_lowercase();
    for extra in [
        "request_user_input",
        "user_input",
        "dedup",
        "skills",
        "skill",
        "webui",
        "sync",
        "pair",
        "approval",
    ] {
        if title_l.contains(extra) || n.symptom.to_ascii_lowercase().contains(extra) {
            terms.push(extra.to_string());
        }
    }
    if n.title.contains("口令") || n.symptom.contains("口令") {
        terms.push("auth_token".into());
        terms.push("app_user".into());
        terms.push("WebuiToken".into());
        terms.push("/app".into());
        terms.push("pairing".into());
    }
    if n.title.to_ascii_lowercase().contains("skill")
        || n.symptom.to_ascii_lowercase().contains("skill")
        || n.title.contains("去重")
    {
        terms.push("registry".into());
        terms.push("use_skill".into());
        terms.push("list_skills".into());
        terms.push("frontmatter".into());
        terms.push("dedup".into());
    }
    let blob_l = format!("{} {}", n.title, n.symptom).to_ascii_lowercase();
    if blob_l.contains("auto")
        && (blob_l.contains("确认") || blob_l.contains("confirm") || blob_l.contains("approval"))
        || blob_l.contains("approval")
        || blob_l.contains("手动确认")
    {
        terms.push("approval_mode".into());
        terms.push("ApprovalMode".into());
        terms.push("approval".into());
        terms.push("auto".into());
    }
    if (blob_l.contains("sync") || n.title.contains("同步") || n.symptom.contains("同步"))
        && (blob_l.contains("delay")
            || n.symptom.contains("延迟")
            || n.symptom.contains("秒")
            || blob_l.contains("webui")
            || n.title.contains("WebUI"))
    {
        terms.push("ensure_headless_runtime".into());
        terms.push("InputAccepted".into());
        terms.push("live_api".into());
        terms.push("headless".into());
    }
    // 正文点名的源码路径优先进检索词（telemetry_cmd.rs / queue/mod.rs 等）
    let body_blob = format!("{} {}\n{}", n.title, n.symptom, n.body_clean);
    for p in extract_mentioned_source_paths(&body_blob) {
        if let Some(name) = std::path::Path::new(&p)
            .file_stem()
            .and_then(|s| s.to_str())
        {
            if name.len() >= 3 {
                terms.push(name.to_string());
            }
        }
        for part in p.split('/') {
            if part.len() >= 3 && part != "src" && part != "crates" && !part.ends_with(".rs") {
                terms.push(part.to_string());
            }
        }
        if p.contains("telemetry") {
            terms.push("telemetry".into());
            terms.push("telemetry_cmd".into());
        }
        if p.contains("queue") {
            terms.push("queue".into());
        }
    }
    if blob_l.contains("telemetry") || n.title.contains("telemetry") {
        terms.push("telemetry".into());
        terms.push("telemetry_cmd".into());
        terms.push("dump".into());
    }
    if blob_l.contains("payload")
        || blob_l.contains("20mb")
        || blob_l.contains("413")
        || n.symptom.contains("请求体")
        || n.title.contains("20MB")
    {
        terms.push("payload".into());
        terms.push("413".into());
        terms.push("too large".into());
        terms.push("max_body".into());
    }
    if blob_l.contains("websocket") || blob_l.contains("wecom") || n.title.contains("WeCom") {
        terms.push("websocket".into());
        terms.push("wecom".into());
        terms.push("aibot".into());
    }
    terms.sort();
    terms.dedup();
    terms.truncate(20);

    let mut steps = vec![
        "Search error signatures and stack symbols in repository".into(),
        "Search related source paths via git grep".into(),
        "Search tests mentioning the symptom".into(),
        "Inspect recent git history for related fixes".into(),
        "If bug-like: expand enclosing function, callers, file history (level1)".into(),
    ];
    if !n.reproduction_steps.is_empty() {
        steps.push("Map reproduction steps to code paths".into());
    }

    InvestigationPlan {
        steps,
        search_terms: terms,
        likely_paths: Vec::new(),
    }
}

const STOP: &[&str] = &[
    "when", "with", "from", "that", "this", "have", "does", "into", "after", "before", "error",
    "crash", "issue", "problem", "windows", "linux", "macos",
];

/// 在本地仓库上跑 Level 0 验证。
pub fn verify_level0(
    repo_root: &Path,
    n: &NormalizedIssue,
    run_tests_search: bool,
) -> Result<TechnicalVerification> {
    let mut plan = build_plan(n);
    let mut code_hits = Vec::new();
    let mut test_hits = Vec::new();
    let mut evidence = Vec::new();
    let mut git_commits = Vec::new();

    for term in plan.search_terms.iter().take(10) {
        for hit in git_grep(repo_root, term, 12)? {
            if is_noise_path(&hit.path) {
                continue;
            }
            if is_test_path(&hit.path) {
                if run_tests_search {
                    test_hits.push(hit);
                }
            } else {
                if plan.likely_paths.len() < 12 && !plan.likely_paths.contains(&hit.path) {
                    plan.likely_paths.push(hit.path.clone());
                }
                code_hits.push(hit);
            }
        }
    }
    // 正文点名路径：磁盘上存在则注入锚点（grep 不一定命中模块自身文件名）
    let body_blob = format!("{} {}\n{}", n.title, n.symptom, n.body_clean);
    for mentioned in extract_mentioned_source_paths(&body_blob) {
        if let Some(hit) = hit_from_mentioned_path(repo_root, &mentioned) {
            if !code_hits
                .iter()
                .any(|h| h.path == hit.path && h.line == hit.line)
            {
                code_hits.push(hit);
            }
        }
    }
    // 源码优先、文档/配置靠后；同路径保留靠前命中
    code_hits.sort_by(|a, b| {
        path_rank(&a.path)
            .cmp(&path_rank(&b.path))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    {
        let mut seen = std::collections::HashSet::new();
        code_hits.retain(|h| seen.insert(format!("{}:{}", h.path, h.line)));
    }
    // 主题相关性过滤：丢掉 wire_dump / 无关路径噪声
    let before_rel = code_hits.len();
    code_hits = filter_relevant_hits(n, code_hits);
    if before_rel > code_hits.len() {
        evidence.push(format!(
            "hits_filtered_offtopic={}",
            before_rel - code_hits.len()
        ));
    }
    test_hits = filter_relevant_hits(n, test_hits);
    // likely_paths 必须跟过滤后的命中一致，避免 CLI/评论仍展示串题路径
    plan.likely_paths.clear();
    for h in &code_hits {
        if plan.likely_paths.len() >= 12 {
            break;
        }
        if !plan.likely_paths.contains(&h.path) {
            plan.likely_paths.push(h.path.clone());
        }
    }

    // git log --grep / S
    for term in plan.search_terms.iter().take(5) {
        for c in git_log_grep(repo_root, term, 5)? {
            if !git_commits.contains(&c) {
                git_commits.push(c);
            }
        }
    }

    // 关联 fix / close 关键词
    let mut fix_prs = Vec::new();
    for c in &git_commits {
        if let Some(pr) = extract_pr_ref(c) {
            fix_prs.push(pr);
        }
    }

    code_hits.truncate(30);
    test_hits.truncate(20);
    git_commits.truncate(15);

    if !code_hits.is_empty() {
        evidence.push(format!("code_hits={}", code_hits.len()));
    }
    if !test_hits.is_empty() {
        evidence.push(format!("test_hits={}", test_hits.len()));
    }
    if !git_commits.is_empty() {
        evidence.push(format!("related_commits={}", git_commits.len()));
    }
    if !fix_prs.is_empty() {
        evidence.push(format!("possible_fix_prs={}", fix_prs.join(",")));
    }

    let (mut technical_verdict, mut confidence) =
        score_verification(n, &code_hits, &test_hits, &git_commits);

    // Level 1：像真 BUG 时深挖函数体 / 调用方 / 文件历史
    let mut deep_dig = Vec::new();
    let mut deep_dig_ran = false;
    if should_deep_dig(technical_verdict, n, &code_hits) {
        deep_dig_ran = true;
        deep_dig = deepen_level1(repo_root, n, &code_hits);
        if !deep_dig.is_empty() {
            evidence.push(format!("deep_dig={}", deep_dig.len()));
            // 有展开上下文时略抬置信度（仍封顶）
            confidence = (confidence + 0.08).min(0.9);
            // 关联提交优先用「命中文件的 git log」（真实触碰该文件），
            // 关键词 log 仅保留与文件历史 sha 重叠的，避免串台噪声。
            git_commits = prefer_file_bound_commits(&deep_dig, &git_commits);
            // 同步刷新 fix_prs
            fix_prs.clear();
            for c in &git_commits {
                if let Some(pr) = extract_pr_ref(c) {
                    if !fix_prs.contains(&pr) {
                        fix_prs.push(pr);
                    }
                }
            }
            // 重写 evidence 里的 related_commits 计数
            evidence.retain(|e| !e.starts_with("related_commits="));
            if !git_commits.is_empty() {
                evidence.push(format!("related_commits={}", git_commits.len()));
            }
            evidence.push("commits_file_bound".into());

            // 仅在「错误签名命中 + 文件绑定 fix 提交」时才可升 ALREADY_FIXED
            if let Some((v, c)) = score_already_fixed_strict(n, &code_hits, &git_commits, true) {
                technical_verdict = v;
                confidence = c;
                evidence.push("already_fixed_file_bound".into());
            }
        }
    }

    if deep_dig_ran
        && !deep_dig.is_empty()
        && technical_verdict == IssueVerdict::Unverified
        && !n.error_signatures.is_empty()
    {
        technical_verdict = IssueVerdict::LikelyBug;
    }

    Ok(TechnicalVerification {
        enabled: true,
        skipped_reason: None,
        plan,
        code_hits,
        git_commits,
        test_hits,
        fix_prs,
        evidence,
        technical_verdict,
        confidence,
        deep_dig_ran,
        deep_dig,
    })
}

/// 深挖后的提交列表：文件历史优先；关键词命中仅当 sha 与文件历史重叠才保留。
fn prefer_file_bound_commits(deep: &[DeepDigBlock], keyword_commits: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen_sha = std::collections::HashSet::new();
    for d in deep {
        for c in &d.file_commits {
            let sha = commit_sha_prefix(c);
            if !sha.is_empty() && !seen_sha.insert(sha.to_string()) {
                continue;
            }
            if !out.contains(c) {
                out.push(c.clone());
            }
        }
    }
    if out.is_empty() {
        // 无文件历史时退回关键词（仍截断）
        return keyword_commits.iter().take(8).cloned().collect();
    }
    // 关键词 commit 仅当 short sha 已在文件历史上出现
    for c in keyword_commits {
        let sha = commit_sha_prefix(c);
        if sha.is_empty() {
            continue;
        }
        if seen_sha.contains(sha) && !out.contains(c) {
            out.push(c.clone());
        }
    }
    out.truncate(12);
    out
}

fn commit_sha_prefix(line: &str) -> &str {
    let token = line.split_whitespace().next().unwrap_or("");
    if token.len() >= 7 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        // 统一用 7 位前缀做集合键
        &token[..7.min(token.len())]
    } else {
        ""
    }
}

/// 是否进入 Level 1 深挖：像真缺陷 / 回归，或 Level0 已有错误签名命中源码。
pub fn should_deep_dig(verdict: IssueVerdict, n: &NormalizedIssue, code_hits: &[CodeHit]) -> bool {
    if code_hits.is_empty() {
        return false;
    }
    let has_src = code_hits.iter().any(|h| path_rank(&h.path) <= 2);
    if !has_src {
        return false;
    }
    matches!(
        verdict,
        IssueVerdict::LikelyBug
            | IssueVerdict::ConfirmedBug
            | IssueVerdict::Regression
            | IssueVerdict::Unverified
    ) && (!n.error_signatures.is_empty()
        || matches!(
            verdict,
            IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug | IssueVerdict::Regression
        ))
}

const DEEP_MAX_BLOCKS: usize = 4;
const DEEP_MAX_CONTEXT_CHARS: usize = 2800;
const DEEP_MAX_FN_LINES: u32 = 80;
const DEEP_WINDOW: u32 = 28;

/// Level 1：对优先命中展开函数体、调用方与文件历史。
pub fn deepen_level1(
    repo_root: &Path,
    n: &NormalizedIssue,
    code_hits: &[CodeHit],
) -> Vec<DeepDigBlock> {
    let mut ranked: Vec<&CodeHit> = code_hits
        .iter()
        .filter(|h| path_rank(&h.path) <= 2 && !is_noise_path(&h.path))
        .collect();
    ranked.sort_by(|a, b| {
        path_rank(&a.path)
            .cmp(&path_rank(&b.path))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });

    let mut blocks = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    for hit in ranked {
        if blocks.len() >= DEEP_MAX_BLOCKS {
            break;
        }
        // 同一文件优先保留第一个锚点，避免重复展开
        if !seen_paths.insert(hit.path.clone()) {
            continue;
        }
        if let Some(block) = dig_one_hit(repo_root, n, hit) {
            blocks.push(block);
        }
    }
    blocks
}

fn dig_one_hit(repo_root: &Path, n: &NormalizedIssue, hit: &CodeHit) -> Option<DeepDigBlock> {
    let full = repo_root.join(&hit.path);
    let source = std::fs::read_to_string(&full).ok()?;
    if source.is_empty() {
        return None;
    }

    let mut symbol = None;
    let mut symbol_is_fn = false;
    let (start_line, end_line, context) =
        if let Some(fb) = enclosing_function(&hit.path, &source, hit.line) {
            symbol = Some(fb.name.clone());
            symbol_is_fn = true;
            let (s, e, text) = slice_lines(&source, fb.start_line, fb.end_line, DEEP_MAX_FN_LINES);
            (s, e, text)
        } else {
            let start = hit.line.saturating_sub(DEEP_WINDOW).max(1);
            let end = hit.line.saturating_add(DEEP_WINDOW);
            let (s, e, text) = slice_lines(&source, start, end, DEEP_WINDOW * 2 + 1);
            // 尝试从锚点行抽标识符
            if let Some(id) = primary_ident_near_line(&source, hit.line) {
                symbol = Some(id);
            }
            (s, e, text)
        };

    let mut context = context;
    if context.chars().count() > DEEP_MAX_CONTEXT_CHARS {
        context = context.chars().take(DEEP_MAX_CONTEXT_CHARS).collect();
        context.push_str("\n…");
    }

    let mut notes = Vec::new();
    for sig in n.error_signatures.iter().take(4) {
        if !sig.is_empty() && context.contains(sig) {
            notes.push(format!(
                "error signature appears in expanded context: {sig}"
            ));
        }
    }
    if let Some(sym) = &symbol {
        notes.push(format!("enclosing/related symbol: {sym}"));
    }

    let mut callers = Vec::new();
    if let Some(sym) = &symbol {
        if is_plausible_symbol(sym) {
            for c in git_grep(repo_root, sym, 16).unwrap_or_default() {
                if c.path == hit.path && c.line >= start_line && c.line <= end_line {
                    continue; // 定义自身
                }
                if is_noise_path(&c.path) || is_test_path(&c.path) {
                    continue;
                }
                callers.push(CodeHit {
                    source: "caller_grep".into(),
                    ..c
                });
                if callers.len() >= 6 {
                    break;
                }
            }
        }
    }

    let file_commits = git_log_path(repo_root, &hit.path, 5).unwrap_or_default();
    if !file_commits.is_empty() {
        notes.push(format!(
            "file history: {} recent commit(s)",
            file_commits.len()
        ));
    }
    if !callers.is_empty() {
        notes.push(format!("callers/refs: {}", callers.len()));
    }

    Some(DeepDigBlock {
        path: hit.path.clone(),
        anchor_line: hit.line,
        symbol,
        symbol_is_fn,
        start_line,
        end_line,
        context,
        callers,
        file_commits,
        notes,
    })
}

struct EnclosingFn {
    name: String,
    start_line: u32,
    end_line: u32,
}

fn enclosing_function(path: &str, source: &str, line: u32) -> Option<EnclosingFn> {
    let bodies = list_function_bodies(path, source);
    bodies
        .into_iter()
        .filter(|b| b.start_line <= line && line <= b.end_line)
        .min_by_key(|b| b.end_line.saturating_sub(b.start_line))
        .map(|b| EnclosingFn {
            name: b.name,
            start_line: b.start_line,
            end_line: b.end_line,
        })
}

fn slice_lines(source: &str, start: u32, end: u32, max_lines: u32) -> (u32, u32, String) {
    let lines: Vec<&str> = source.lines().collect();
    let n = lines.len() as u32;
    let start = start.max(1).min(n.max(1));
    let mut end = end.max(start).min(n.max(1));
    if end.saturating_sub(start) + 1 > max_lines {
        end = start + max_lines - 1;
    }
    let mut out = String::new();
    for (i, line) in lines
        .iter()
        .enumerate()
        .skip((start - 1) as usize)
        .take((end - start + 1) as usize)
    {
        out.push_str(&format!("{:>4}| {}\n", i + 1, line));
    }
    (start, end.min(n.max(1)), out)
}

fn primary_ident_near_line(source: &str, line: u32) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let idx = (line.saturating_sub(1) as usize).min(lines.len() - 1);
    // 向上扫几行找 fn name
    let from = idx.saturating_sub(40);
    for i in (from..=idx).rev() {
        let l = lines[i].trim();
        let rest = [
            "pub async fn ",
            "pub(crate) fn ",
            "async fn ",
            "pub fn ",
            "fn ",
        ]
        .iter()
        .find_map(|p| l.strip_prefix(p));
        if let Some(rest) = rest {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if is_plausible_symbol(&name) {
                return Some(name);
            }
        }
    }
    // 锚点行里最长标识符
    let l = lines[idx];
    let mut best = String::new();
    let mut cur = String::new();
    for ch in l.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else {
            if cur.len() > best.len() && is_plausible_symbol(&cur) {
                best = cur.clone();
            }
            cur.clear();
        }
    }
    if cur.len() > best.len() && is_plausible_symbol(&cur) {
        best = cur;
    }
    if best.is_empty() {
        None
    } else {
        Some(best)
    }
}

fn is_plausible_symbol(s: &str) -> bool {
    let b = s.as_bytes();
    if s.len() < 3 || s.len() > 64 {
        return false;
    }
    if !b[0].is_ascii_alphabetic() && b[0] != b'_' {
        return false;
    }
    // 过滤常见噪声
    !matches!(
        s,
        "self"
            | "super"
            | "crate"
            | "Self"
            | "true"
            | "false"
            | "return"
            | "match"
            | "error"
            | "Error"
            | "String"
            | "Result"
            | "Option"
            | "unwrap"
            | "expect"
            | "into"
            | "from"
            | "clone"
            | "format"
            | "println"
            | "eprintln"
            | "todo"
            | "unimplemented"
    )
}

fn git_log_path(repo: &Path, path: &str, limit: usize) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "log",
            "--oneline",
            &format!("-{limit}"),
            "--",
            path,
        ])
        .output();
    let Ok(out) = out else {
        return Ok(Vec::new());
    };
    if !out.status.success() && out.stdout.is_empty() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

fn score_verification(
    n: &NormalizedIssue,
    code: &[CodeHit],
    tests: &[CodeHit],
    commits: &[String],
) -> (IssueVerdict, f32) {
    let has_err = !n.error_signatures.is_empty();
    let path_match = !code.is_empty();
    let test_match = !tests.is_empty();
    let hist = !commits.is_empty();
    let strong_err_hit = code.iter().any(|h| hit_matches_error_sig(n, h));

    // 关键词 fix 日志 alone 不再直接 AlreadyFixed（易被串台提交误伤）
    let fix_lang = commits.iter().any(|c| is_fix_like_commit(c));

    if path_match && has_err && (test_match || strong_err_hit) {
        return (
            IssueVerdict::LikelyBug,
            if test_match { 0.82 } else { 0.78 },
        );
    }
    if path_match && has_err {
        return (IssueVerdict::LikelyBug, 0.72);
    }
    // 有相关代码 + 修复向提交：最多标回归嫌疑，不在此阶段 AlreadyFixed
    if fix_lang && path_match && strong_err_hit {
        return (IssueVerdict::Regression, 0.58);
    }
    if hist && !path_match {
        return (IssueVerdict::Unverified, 0.45);
    }
    if path_match {
        return (IssueVerdict::Unverified, 0.55);
    }
    (IssueVerdict::Unverified, 0.4)
}

/// 严格 AlreadyFixed：错误签名命中 + **与主题相关的**文件绑定 fix 提交 + 已深挖。
fn score_already_fixed_strict(
    n: &NormalizedIssue,
    code: &[CodeHit],
    file_bound_commits: &[String],
    deep_dig_ok: bool,
) -> Option<(IssueVerdict, f32)> {
    if !deep_dig_ok || n.error_signatures.is_empty() || code.is_empty() {
        return None;
    }
    if !code.iter().any(|h| hit_matches_error_sig(n, h)) {
        return None;
    }
    let relevant_fix: Vec<&String> = file_bound_commits
        .iter()
        .filter(|c| is_fix_like_commit(c) && fix_commit_matches_issue(n, c))
        .collect();
    if relevant_fix.is_empty() {
        return None;
    }
    // 有明确版本信息时更像「请升级」；否则标回归/疑似仍存在
    if n.environment
        .as_object()
        .map(|o| o.contains_key("app_version"))
        .unwrap_or(false)
    {
        Some((IssueVerdict::AlreadyFixed, 0.72))
    } else {
        Some((IssueVerdict::Regression, 0.62))
    }
}

fn is_fix_like_commit(c: &str) -> bool {
    let l = c.to_ascii_lowercase();
    l.contains("fix")
        || l.contains("close #")
        || l.contains("resolv")
        || l.contains("hotfix")
        || l.contains("bugfix")
}

/// fix 提交说明须与错误/症状主题有交集，避免「同文件其它 fix(auth)/fix(tls) oauth」误升 AlreadyFixed。
fn fix_commit_matches_issue(n: &NormalizedIssue, commit: &str) -> bool {
    let cl = commit.to_ascii_lowercase();
    let title = crate::issue::normalize::strip_campaign_noise(&n.title).to_ascii_lowercase();
    let symptom = n.symptom.to_ascii_lowercase();
    let blob = format!("{title} {symptom}");
    let networkish = blob_is_network_provider_issue(&blob, n);

    // 网络/provider 断连类：只认连接重置/重试/流式/超时类提交，排除 oauth/tls 登录
    if networkish {
        if cl.contains("oauth")
            || cl.contains("fix(auth)")
            || cl.contains("fix(tls)")
            || (cl.contains("tls") && cl.contains("atomgit") && !cl.contains("provider"))
        {
            return false;
        }
        return commit_is_connection_reset_class(&cl)
            || n.error_signatures.iter().any(|sig| {
                let s = sig.to_ascii_lowercase();
                s.len() >= 4 && cl.contains(&s)
            });
    }

    for sig in &n.error_signatures {
        let s = sig.to_ascii_lowercase();
        if s.len() >= 4 && cl.contains(&s) {
            return true;
        }
    }
    for key in [
        "stream",
        "disconnect",
        "timeout",
        "network",
        "connection",
        "reconnect",
        "网络",
        "连接",
        "重连",
        "中断",
        "sync",
        "webui",
        "skill",
        "dedup",
        "口令",
        "permission",
        "approval",
    ] {
        if issue_and_commit_share_key(&blob, &cl, key) {
            return true;
        }
    }
    false
}

fn blob_is_network_provider_issue(blob: &str, n: &NormalizedIssue) -> bool {
    blob.contains("网络")
        || blob.contains("decoding")
        || blob.contains("断连")
        || blob.contains("重连")
        || blob.contains("中断")
        || token_has(blob, "connection")
        || token_has(blob, "network")
        || n.error_signatures.iter().any(|s| {
            s.contains("网络")
                || s.contains("decoding")
                || s.contains("connection")
                || s.contains("重连")
        })
}

fn commit_is_connection_reset_class(cl: &str) -> bool {
    cl.contains("disconnect")
        || cl.contains("reconnect")
        || cl.contains("connection reset")
        || cl.contains("连接重置")
        || cl.contains("连接中断")
        || cl.contains("重连")
        || cl.contains("中断")
        || cl.contains("stale")
        || cl.contains("keep-alive")
        || cl.contains("keepalive")
        || cl.contains("badrecordmac")
        || cl.contains("timedout")
        || cl.contains("timed out")
        || cl.contains("os error 10054")
        || cl.contains("os error 110")
        || (cl.contains("retry")
            && (cl.contains("reset")
                || cl.contains("timeout")
                || cl.contains("transient")
                || cl.contains("transport")
                || cl.contains("rate limit")
                || cl.contains("连接")))
}

fn issue_and_commit_share_key(issue_blob: &str, commit_l: &str, key: &str) -> bool {
    let in_issue = if !key.is_ascii() {
        issue_blob.contains(key)
    } else {
        token_has(issue_blob, key)
    };
    let in_commit = if !key.is_ascii() {
        commit_l.contains(key)
    } else {
        token_has(commit_l, key)
    };
    in_issue && in_commit
}

fn token_has(hay: &str, needle: &str) -> bool {
    hay.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|t| t == needle)
}

fn hit_matches_error_sig(n: &NormalizedIssue, h: &CodeHit) -> bool {
    n.error_signatures
        .iter()
        .any(|s| error_sig_matches_text(s, &h.snippet))
}

/// 错误签名匹配：禁止 `error:` 命中 `is_error`，禁止极短泛标记乱匹配。
pub(crate) fn error_sig_matches_text(sig: &str, text: &str) -> bool {
    if sig.is_empty() {
        return false;
    }
    let s = sig.to_ascii_lowercase();
    let t = text.to_ascii_lowercase();
    // 泛标记：只认独立 `error:` / `exception:`，不认 is_error: / has_error:
    if s == "error:" || s == "exception:" {
        let needle = if s == "error:" {
            "error:"
        } else {
            "exception:"
        };
        return contains_as_sig_prefix(&t, needle);
    }
    // 短 ASCII 词：词边界
    if s.is_ascii() && s.len() <= 6 && !s.contains(' ') {
        let bare = s.trim_end_matches(':');
        return token_has(&t, bare) || t.contains(&s);
    }
    t.contains(&s) || text.contains(sig)
}

fn snip_has_network_transport_sig(snip: &str) -> bool {
    snip.contains("网络")
        || snip.contains("decoding")
        || snip.contains("connection reset")
        || snip.contains("重连")
        || snip.contains("中断")
        || snip.contains("disconnect")
        || snip.contains("timeout")
        || snip.contains("badrecordmac")
        || snip.contains("远端")
        || snip.contains("error decoding")
}

/// `error:` 不得匹配 `is_error:` / `has_error:`（前缀须为非标识符字符）。
fn contains_as_sig_prefix(hay: &str, needle: &str) -> bool {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return false;
    }
    let mut i = 0usize;
    while i + n.len() <= h.len() {
        if &h[i..i + n.len()] == n {
            let ok_before = i == 0 || !(h[i - 1].is_ascii_alphanumeric() || h[i - 1] == b'_');
            if ok_before {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// ASCII 用词边界；CJK 用 contains。
fn ascii_or_cjk_token_match(path: &str, snip: &str, tok: &str) -> bool {
    if !tok.is_ascii() {
        return path.contains(tok) || snip.contains(tok);
    }
    let t = tok.to_ascii_lowercase();
    // 通用 noise token 不参与加分
    if matches!(
        t.as_str(),
        "error" | "erro" | "fail" | "failed" | "issue" | "bug" | "test" | "true" | "false"
    ) {
        return false;
    }
    token_has(path, &t) || token_has(snip, &t)
}

/// 口令 / App 远程配对相关路径（非 git remote / labels / tool-call pairing）。
pub(crate) fn path_looks_like_pairing(path: &str, snip: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let s = snip.to_ascii_lowercase();
    // 拒绝：git remote、labels、工具结果 pairing 修复、无关 repair
    if p.contains("atomgit/remote")
        || p.contains("atomgit/models")
        || p.contains("compaction")
        || s.contains("label list")
        || s.contains("git remote url")
        || s.contains("coerce a json value into a label")
        || s.contains("tool-call")
        || s.contains("tool call")
        || s.contains("result pairing")
        || (s.contains("pairing repair") && !s.contains("口令") && !contains_app_cmd(&s))
        || (s.contains("repairing") && s.contains("control character"))
    {
        return false;
    }
    // 明确实现文件
    if p.contains("auth_token") || p.contains("webui_token") {
        return true;
    }
    if p.contains("device") && (p.contains("pair") || p.contains("auth") || p.contains("app")) {
        return true;
    }
    // daemon 中 WebuiToken / app 远程
    if p.contains("daemon")
        && (contains_app_cmd(&s)
            || s.contains("app_user_id")
            || s.contains("app_user")
            || s.contains("口令")
            || s.contains("device code")
            || s.contains("webui_token")
            || s.contains("app 远程")
            || s.contains("远程访问")
            || s.contains("struct webuitoken"))
    {
        return true;
    }
    // clix/tuix 必须真是 /app 命令语义（不能用 append 误匹配 /app）
    if (p.contains("clix") || p.contains("tuix"))
        && (contains_app_cmd(&s)
            || s.contains("口令")
            || s.contains("app_user")
            || s.contains("webui_token")
            || s.contains("device code"))
    {
        return true;
    }
    // snip 明确口令/设备码
    if s.contains("口令") || s.contains("device code") || s.contains("pair token") {
        return true;
    }
    if contains_app_cmd(&s) && (s.contains("token") || s.contains("remote") || s.contains("口令"))
    {
        return true;
    }
    false
}

/// 匹配 `/app` 命令边界，避免 `persona/append` 等误命中。
fn contains_app_cmd(s: &str) -> bool {
    // `/app` 后不能是字母（append 会假阳）
    let b = s.as_bytes();
    let needle = b"/app";
    let mut i = 0;
    while i + 4 <= b.len() {
        if &b[i..i + 4] == needle {
            let next_ok = i + 4 >= b.len() || !b[i + 4].is_ascii_alphabetic();
            let prev_ok = i == 0 || !b[i - 1].is_ascii_alphanumeric();
            // 允许 ` /app `、`"/app"`、`` `/app` ``
            if next_ok {
                let _ = prev_ok;
                return true;
            }
        }
        i += 1;
    }
    // 独立词 app 模式过宽，不用
    false
}

/// 丢掉与 Issue 主题无关的命中（wire dump 示例、站点静态页等）。
pub fn filter_relevant_hits(n: &NormalizedIssue, hits: Vec<CodeHit>) -> Vec<CodeHit> {
    let mut scored: Vec<(i32, CodeHit)> = hits
        .into_iter()
        .filter(|h| !is_noise_path(&h.path))
        .map(|h| (hit_relevance_score(n, &h), h))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    // 若全被滤掉但原先有命中，保留分数最高的噪声外路径会在 filter 前处理；
    // 这里允许空结果 → 避免答非所问。
    scored.into_iter().map(|(_, h)| h).collect()
}

pub(crate) fn hit_relevance_score(n: &NormalizedIssue, h: &CodeHit) -> i32 {
    let mut score = 0i32;
    let path = h.path.to_ascii_lowercase();
    let snip = h.snippet.to_ascii_lowercase();
    let title = crate::issue::normalize::strip_campaign_noise(&n.title).to_ascii_lowercase();
    let symptom = n.symptom.to_ascii_lowercase();

    if hit_matches_error_sig(n, h) {
        score += 50;
    }
    for sig in &n.error_signatures {
        let s = sig.to_ascii_lowercase();
        if path.contains(&s) || snip.contains(&s) {
            score += 20;
        }
    }
    // 标题实体词（长度≥4）：ASCII 走词边界，避免 Error⊂is_error / Error⊂DecryptError
    for tok in title.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if tok.len() < 4 {
            continue;
        }
        if ascii_or_cjk_token_match(&path, &snip, tok) {
            score += 8;
        }
    }
    for tok in symptom.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if tok.chars().count() < 4 {
            continue;
        }
        if ascii_or_cjk_token_match(&path, &snip, tok) {
            score += 4;
        }
    }
    // 模块线索（含中文实体）
    let issue_blob = format!("{title} {symptom} {}", n.body_clean.to_ascii_lowercase());
    // 正文点名路径：最高优先
    let mentioned =
        extract_mentioned_source_paths(&format!("{} {}\n{}", n.title, n.symptom, n.body_clean));
    for mp in &mentioned {
        let mp_l = mp.to_ascii_lowercase();
        if path.contains(&mp_l)
            || path.ends_with(mp_l.rsplit('/').next().unwrap_or(""))
            || std::path::Path::new(&path)
                .file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| mp_l.ends_with(f) || mp_l.contains(&f.replace(".rs", "")))
        {
            score += 40;
        }
    }
    for clue in [
        "skill",
        "dedup",
        "webui",
        "sync",
        "retry",
        "oauth",
        "stream",
        "provider",
        "tui",
        "daemon",
        "atomgit",
        "request_user",
        "approval",
        "memory",
        "acp",
        "telemetry",
        "payload",
        "websocket",
        "wecom",
        "queue",
        "口令",
        "配对",
        "去重",
        "延迟",
        "同步",
    ] {
        let clue_l = clue.to_ascii_lowercase();
        if (issue_blob.contains(clue) || issue_blob.contains(&clue_l))
            && (path.contains(&clue_l)
                || snip.contains(&clue_l)
                || path.contains(clue)
                || snip.contains(clue)
                || (clue == "口令" && path_looks_like_pairing(&path, &snip))
                || (clue == "同步" && (path.contains("sync") || path.contains("webui")))
                || (clue == "延迟"
                    && (path.contains("sync") || path.contains("webui") || path.contains("event"))))
        {
            score += 12;
        }
    }
    // 主题硬互斥：非该主题时直接清零
    // 口令/配对优先：正文里「与网络报错无关」不得把 issue 当成 provider 断连
    let about_pair = issue_blob.contains("口令")
        || issue_blob.contains("配对")
        || issue_blob.contains("pair")
        || (issue_blob.contains("app") && issue_blob.contains("连接"));
    let about_network = !about_pair
        && (issue_blob.contains("网络")
            || issue_blob.contains("connect")
            || issue_blob.contains("decoding")
            || issue_blob.contains("断连")
            || issue_blob.contains("重连")
            || n.error_signatures
                .iter()
                .any(|s| s.contains("网络") || s.contains("decoding") || s.contains("connection")));
    let about_input = issue_blob.contains("request_user_input")
        || issue_blob.contains("user_input")
        || issue_blob.contains("手动确认");
    let about_skill =
        issue_blob.contains("skill") || issue_blob.contains("去重") || issue_blob.contains("dedup");
    let about_sync_latency = (issue_blob.contains("sync") || issue_blob.contains("同步"))
        && (issue_blob.contains("延迟")
            || issue_blob.contains("delay")
            || issue_blob.contains("秒")
            || issue_blob.contains("webui")
            || issue_blob.contains("inputaccepted")
            || issue_blob.contains("headless"));
    let about_auto_confirm = (issue_blob.contains("auto") || issue_blob.contains("自动"))
        && (issue_blob.contains("确认")
            || issue_blob.contains("confirm")
            || issue_blob.contains("approval")
            || issue_blob.contains("手动"))
        || issue_blob.contains("approval_mode")
        || issue_blob.contains("approvalmode");

    if about_pair {
        // 硬门槛：非 pairing 实现一律丢弃（不要只靠前面加分）
        if !path_looks_like_pairing(&path, &snip) {
            return 0;
        }
        score = score.max(20);
    }
    if about_input
        && path.contains("provider")
        && !path.contains("tool")
        && !path.contains("approval")
        && !snip.contains("request_user")
        && !snip.contains("user_input")
    {
        return 0;
    }
    if about_auto_confirm {
        // Auto/确认 → approval_mode，禁止 bash askpass 冒充
        let good = path.contains("approval")
            || snip.contains("approval")
            || snip.contains("approvalmode")
            || snip.contains("approval_mode")
            || (path.contains("daemon")
                && (snip.contains("auto") || snip.contains("confirm") || snip.contains("mode")))
            || (path.contains("live")
                && (snip.contains("approval")
                    || snip.contains("auto")
                    || snip.contains("confirm")));
        let bad_bash = path.contains("bash")
            && (snip.contains("askpass")
                || snip.contains("password")
                || snip.contains("prompt that can't appear")
                || snip.contains("webui/headless"));
        if bad_bash || !good {
            return 0;
        }
        score = score.max(25);
    }
    if about_skill {
        // 必须落到 skills 实现，禁止 Cargo.toml 注释充数
        if path.ends_with("cargo.toml") || path.ends_with("Cargo.toml") {
            return 0;
        } else if path.contains("/skills/")
            || path.contains("\\skills\\")
            || path.contains("skill.rs")
            || path.contains("use_skill")
            || path.contains("registry")
            || snip.contains("dedup")
            || snip.contains("同名")
        {
            score = score.max(25);
        } else if !path.contains("skill") && !snip.contains("skill") {
            return 0;
        } else {
            // 含 skill 但非 skills/ 实现：降权
            score = score.min(5);
        }
    }
    if about_sync_latency {
        // 延迟同步 → 只认 live_api / InputAccepted / ensure_headless_runtime 延迟路径
        // 禁止：provider 切换脚注、tuix commands 里 phone/WebUI usage broadcast 注释
        let good = path.contains("live_api")
            || path.contains("native_live")
            || path.contains("live_hub")
            || snip.contains("ensure_headless")
            || snip.contains("inputaccepted")
            || snip.contains("input accepted")
            || (path.contains("daemon")
                && (snip.contains("headless")
                    || snip.contains("inputaccepted")
                    || snip.contains("live_api")
                    || (snip.contains("sync") && snip.contains("delay"))))
            || (path.contains("webui")
                && (snip.contains("inputaccepted")
                    || snip.contains("ensure_headless")
                    || (snip.contains("sync") && (snip.contains("delay") || snip.contains("秒")))));
        let bad = (path.contains("provider") && !snip.contains("sync"))
            || snip.contains("wrong provider")
            || snip.contains("provider-override")
            || path.contains("cli/src/main.rs")
            || path.contains("atomcode-cli")
            || (path.contains("commands.rs")
                && (snip.contains("usage")
                    || snip.contains("phone/webui")
                    || snip.contains("merely because")
                    || (snip.contains("broadcast")
                        && !snip.contains("inputaccepted")
                        && !snip.contains("ensure_headless"))))
            || (path.contains("bash") && snip.contains("askpass"));
        if bad || !good {
            return 0;
        }
        score = score.max(25);
    }
    // 鉴权主题：仅当 issue 明确谈 oauth/login/token 恢复时，oauth 路径才可进
    let about_auth = issue_blob.contains("oauth")
        || issue_blob.contains("login")
        || issue_blob.contains("登录")
        || issue_blob.contains("鉴权")
        || (issue_blob.contains("token")
            && (issue_blob.contains("refresh")
                || issue_blob.contains("auth")
                || issue_blob.contains("atomgit")))
        || issue_blob.contains("start_login")
        || about_pair;
    // 非鉴权 issue：oauth / gateway_crypto / TLS login 一律丢（禁止 error: 泛匹配抬权）
    if !about_auth
        && (path.contains("oauth")
            || path.contains("gateway_crypto")
            || (path.contains("/auth/")
                && !path.contains("provider")
                && !path.contains("auth_token")
                && !path_looks_like_pairing(&path, &snip)))
    {
        return 0;
    }
    // 非 hooks 主题：cc_hooks 测试钩子不进
    let about_hooks = issue_blob.contains("hook")
        || issue_blob.contains("cc_hooks")
        || issue_blob.contains("post_tool");
    if !about_hooks && path.contains("cc_hooks") {
        return 0;
    }
    // 非 codeintel/LSP 主题：codeintel 不进
    let about_codeintel = issue_blob.contains("codeintel")
        || issue_blob.contains("lsp")
        || issue_blob.contains("language server")
        || issue_blob.contains("补全")
        || issue_blob.contains("goto");
    if !about_codeintel && path.contains("codeintel") {
        return 0;
    }

    // 网络断连类：只认 provider/传输层
    if about_network {
        let snip_network = snip_has_network_transport_sig(&snip);
        if path.contains("oauth")
            || path.contains("gateway_crypto")
            || (path.contains("/auth/") && !path.contains("provider"))
            || path.contains("cc_hooks")
            || path.contains("codeintel")
            || path.contains("/hooks/")
            || (path.contains("plugin") && !snip_network)
        {
            return 0;
        }
        let path_transport = path.contains("provider")
            || path.contains("retry")
            || path.contains("openai")
            || path.contains("http")
            || path.contains("stream")
            || path.contains("reqwest")
            || path.contains("transport");
        if !path_transport && !snip_network {
            return 0;
        }
    }
    if !about_network
        && (path.contains("retry") || snip.contains("网络连接中断"))
        && !about_pair
        && !issue_blob.contains("payload")
        && !issue_blob.contains("413")
        && !issue_blob.contains("retry")
        && !issue_blob.contains("重试")
    {
        score = score.min(2);
    }

    // 强噪声路径即使误入也压到 0
    if path.contains("wire_dump")
        || path.contains("referral.html")
        || path.ends_with(".gitignore")
        || path.contains("create_tag_release")
        || path.ends_with("site/index.html")
        || path.contains("/site/")
    {
        score = 0;
    }
    // Cargo.toml 包描述里的 OAuth/marketing 文案：非依赖/cargo 主题直接丢
    if path.ends_with("cargo.toml") {
        let about_cargo = issue_blob.contains("cargo")
            || issue_blob.contains("依赖")
            || issue_blob.contains("dependency")
            || issue_blob.contains("crate");
        if !about_cargo
            && (snip.contains("description")
                || snip.contains("oauth login")
                || snip.contains("leaf crate")
                || snip.contains("infallible")
                || snip.contains("rustls"))
        {
            return 0;
        }
    }
    if (snip.contains("wire_dump") || (snip.contains("deepseek-v4") && snip.contains("json!({")))
        && !about_network
        && !issue_blob.contains("provider")
        && !issue_blob.contains("wire")
    {
        score = 0;
    }
    score
}

/// 正文点名路径 → 若仓库内存在则读前若干实质行作为锚点。
fn hit_from_mentioned_path(repo_root: &Path, mentioned: &str) -> Option<CodeHit> {
    let rel = mentioned.trim().trim_start_matches("./");
    // 允许 issue 写错 crate 名：按文件名回落搜索
    let candidates = [
        repo_root.join(rel),
        // 常见：atomcode-core/queue → atomcode-telemetry/queue
        repo_root.join(rel.replace("atomcode-core", "atomcode-telemetry")),
        repo_root.join(rel.replace("atomcode-core", "atomcode-cli")),
    ];
    let mut path_buf = None;
    for c in &candidates {
        if c.is_file() {
            path_buf = Some(c.clone());
            break;
        }
    }
    // 仅文件名时：find 一层 crates/*/src/<name>
    if path_buf.is_none() {
        let name = std::path::Path::new(rel)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(rel);
        if name.ends_with(".rs") {
            let walk = [
                repo_root
                    .join("crates")
                    .join("atomcode-cli")
                    .join("src")
                    .join(name),
                repo_root
                    .join("crates")
                    .join("atomcode-telemetry")
                    .join("src")
                    .join("queue")
                    .join(name),
                repo_root
                    .join("crates")
                    .join("atomcode-telemetry")
                    .join("src")
                    .join(name),
            ];
            for c in walk {
                if c.is_file() {
                    path_buf = Some(c);
                    break;
                }
            }
        }
    }
    let full = path_buf?;
    let rel_out = full
        .strip_prefix(repo_root)
        .unwrap_or(full.as_path())
        .to_string_lossy()
        .replace('\\', "/");
    let text = std::fs::read_to_string(&full).ok()?;
    // 挑第一个实质函数/结构或含 read/dump/queue 的行
    let mut best: Option<(u32, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let ln = (i + 1) as u32;
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("/*") || t.starts_with("use ") {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        let hot = lower.contains("fn dump")
            || lower.contains("fn status")
            || lower.contains("read_to_string")
            || lower.contains("queue")
            || lower.contains("read_dir")
            || lower.contains("struct queue")
            || lower.contains("payload")
            || lower.contains("websocket");
        if hot || best.is_none() {
            best = Some((ln, t.to_string()));
            if hot {
                break;
            }
        }
        if ln > 80 && best.is_some() {
            break;
        }
    }
    let (line, snip) = best?;
    Some(CodeHit {
        path: rel_out,
        line,
        snippet: snip.chars().take(160).collect(),
        source: "body_path".into(),
    })
}

/// 从 Issue 正文抽取 `crates/.../*.rs` 或 `` `path.rs` `` 点名路径。
pub(crate) fn extract_mentioned_source_paths(blob: &str) -> Vec<String> {
    let mut out = Vec::new();
    // backticks
    for part in blob.split('`') {
        let p = part.trim();
        if looks_like_source_path(p) {
            out.push(p.trim_matches(|c: char| c == '"' || c == '\'').to_string());
        }
    }
    // bare tokens containing .rs
    for tok in blob.split_whitespace() {
        let t = tok.trim_matches(|c: char| {
            !c.is_ascii_alphanumeric() && c != '/' && c != '_' && c != '-' && c != '.'
        });
        if looks_like_source_path(t) {
            out.push(t.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn looks_like_source_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    if l.len() < 6 || l.len() > 200 {
        return false;
    }
    if !(l.ends_with(".rs")
        || l.ends_with(".go")
        || l.ends_with(".ts")
        || l.ends_with(".tsx")
        || l.ends_with(".py"))
    {
        return false;
    }
    l.contains('/') || l.contains("telemetry") || l.contains("queue") || l.contains("mod.rs")
}

fn is_test_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains("test") || l.contains("spec") || l.contains("__tests__") || l.contains("/tests/")
}

fn is_noise_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.ends_with(".md")
        || l.ends_with(".lock")
        || l.contains("changelog")
        || l.contains("license")
        || l.contains("/docs/")
        || l.contains("assets/setup-seeds")
        || l.contains("node_modules")
        || l.contains("/target/")
        || l.ends_with(".gitignore")
        || l.contains("site/referral")
        || l.contains("create_tag_release")
        || l.ends_with(".mcp.json.example")
}

/// 数字越小优先级越高。
fn path_rank(p: &str) -> u8 {
    let l = p.to_ascii_lowercase();
    if l.contains("/provider/") || l.contains("retry") || l.contains("http") || l.contains("stream")
    {
        return 0;
    }
    if l.contains("/src/") || l.starts_with("crates/") || l.starts_with("src/") {
        return 1;
    }
    if l.ends_with(".rs") || l.ends_with(".go") || l.ends_with(".ts") || l.ends_with(".py") {
        return 2;
    }
    if l.ends_with(".toml") || l.ends_with(".yaml") || l.ends_with(".yml") {
        return 3;
    }
    5
}

fn git_grep(repo: &Path, term: &str, limit: usize) -> Result<Vec<CodeHit>> {
    if term.trim().is_empty() {
        return Ok(Vec::new());
    }
    let out = Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "grep",
            "-nI",
            "--max-count",
            &limit.to_string(),
            "-e",
            term,
            "--",
            ".",
            ":!target",
            ":!node_modules",
            ":!.git",
        ])
        .output();
    let Ok(out) = out else {
        return Ok(Vec::new());
    };
    if !out.status.success() && out.stdout.is_empty() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut hits = Vec::new();
    for line in text.lines().take(limit) {
        // path:line:snippet
        let mut parts = line.splitn(3, ':');
        let path = parts.next().unwrap_or("").to_string();
        let ln: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let snippet = parts.next().unwrap_or("").trim().to_string();
        if path.is_empty() {
            continue;
        }
        hits.push(CodeHit {
            path,
            line: ln,
            snippet: snippet.chars().take(160).collect(),
            source: "git_grep".into(),
        });
    }
    Ok(hits)
}

fn git_log_grep(repo: &Path, term: &str, limit: usize) -> Result<Vec<String>> {
    if term.trim().is_empty() {
        return Ok(Vec::new());
    }
    let out = Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "log",
            "--oneline",
            &format!("-{limit}"),
            "--grep",
            term,
            "-i",
        ])
        .output();
    let Ok(out) = out else {
        return Ok(Vec::new());
    };
    if !out.status.success() {
        // also try pickaxe
        let out2 = Command::new("git")
            .args([
                "-C",
                &repo.to_string_lossy(),
                "log",
                "--oneline",
                &format!("-{limit}"),
                "-S",
                term,
            ])
            .output();
        if let Ok(o) = out2 {
            let t = String::from_utf8_lossy(&o.stdout);
            return Ok(t
                .lines()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect());
        }
        return Ok(Vec::new());
    }
    let t = String::from_utf8_lossy(&out.stdout);
    Ok(t.lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

fn extract_pr_ref(line: &str) -> Option<String> {
    // 仅识别常见修复语义中的 PR：fix/close/resolve #123 或 (#123)
    let lower = line.to_ascii_lowercase();
    let looks_fix = lower.contains("fix")
        || lower.contains("close")
        || lower.contains("resolv")
        || lower.contains("merge");
    if !looks_fix {
        return None;
    }
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            // 至少 2 位，避免 #1 噪声
            if j >= i + 3 {
                return Some(line[i..j].to_string());
            }
        }
    }
    None
}

/// 解析仓库根：cwd 或显式路径，需含 .git 或源码。
pub fn resolve_repo_root(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if p.is_dir() {
            return Some(p.to_path_buf());
        }
    }
    let cwd = std::env::current_dir().ok()?;
    // walk up for .git
    let mut cur = cwd.as_path();
    loop {
        if cur.join(".git").exists() {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::normalize::normalize_issue;

    #[test]
    fn should_verify_gates() {
        let (ok, _) = should_verify(IssueType::Bug, 0.0, 0.8, true, 0.0, false, true, true);
        assert!(ok);
        let (no, r) = should_verify(IssueType::Bug, 0.9, 0.8, true, 0.0, false, true, true);
        assert!(!no);
        assert!(r.unwrap().contains("spam"));
        let (no2, _) = should_verify(IssueType::Question, 0.0, 0.9, true, 0.0, false, true, true);
        assert!(!no2);
        // 完整度分数低但 can_verify（可检索标题）仍放行
        let (ok2, _) = should_verify(IssueType::Bug, 0.0, 0.4, true, 0.0, false, true, true);
        assert!(ok2);
        let (no3, r3) = should_verify(IssueType::Bug, 0.0, 0.4, false, 0.0, false, true, true);
        assert!(!no3);
        assert!(r3.unwrap().contains("completeness"));
    }

    #[test]
    fn filter_drops_offtopic_wire_dump_for_pairing_bug() {
        let n = normalize_issue(
            "GitCode App 中无法使用口令连接新电脑",
            "口令连接失败，与网络报错无关的配对问题",
        );
        let hits = vec![
            CodeHit {
                path: "crates/atomcode-capabilities/src/provider/mod.rs".into(),
                line: 192,
                snippet: r#"let body = json!({"model": "deepseek-v4"}); wire_dump_to(dir.path(), "deepseek-v4", &body);"#.into(),
                source: "git_grep".into(),
            },
            CodeHit {
                path: "crates/atomcode-capabilities/src/atomgit/remote.rs".into(),
                line: 1,
                snippet: "//! Parse a git remote URL into an AtomGit/GitCode API push target.".into(),
                source: "git_grep".into(),
            },
            CodeHit {
                path: "crates/atomcode-capabilities/src/atomgit/models.rs".into(),
                line: 26,
                snippet: "/// Coerce a JSON value into a label list".into(),
                source: "git_grep".into(),
            },
            CodeHit {
                path: "crates/atomcode-daemon/src/auth_token.rs".into(),
                line: 142,
                snippet: "/// 仅 `/app` 模式下启用（`state.app_user_id` 非空）".into(),
                source: "git_grep".into(),
            },
        ];
        let filtered = filter_relevant_hits(&n, hits);
        assert!(
            filtered.iter().all(|h| !h.snippet.contains("wire_dump")),
            "wire dump should drop: {filtered:?}"
        );
        assert!(
            filtered
                .iter()
                .all(|h| !h.path.contains("remote.rs") && !h.path.contains("models.rs")),
            "reject git-remote/labels for 口令: {filtered:?}"
        );
        assert!(
            filtered.iter().any(|h| h.path.contains("auth_token")),
            "keep /app auth_token hit: {filtered:?}"
        );
        // compaction tool-call pairing 不得冒充设备配对
        let bad = filter_relevant_hits(
            &n,
            vec![CodeHit {
                path: "crates/atomcode-capabilities/src/compaction.rs".into(),
                line: 5,
                snippet: "tool-call/result pairing repair".into(),
                source: "g".into(),
            }],
        );
        assert!(
            bad.is_empty(),
            "tool-call pairing must not match 口令: {bad:?}"
        );
        // clix stdin 注释不得因 App 字样留下
        let clix = filter_relevant_hits(
            &n,
            vec![CodeHit {
                path: "crates/atomcode-clix/src/main.rs".into(),
                line: 176,
                snippet: "// `-` means stdin; only ONE of diff/task/persona/append may read it."
                    .into(),
                source: "g".into(),
            }],
        );
        assert!(
            !path_looks_like_pairing(
                "crates/atomcode-clix/src/main.rs",
                "// `-` means stdin; only ONE of diff/task/persona/append may read it."
            ),
            "path_looks_like_pairing must be false for clix stdin"
        );
        assert_eq!(
            hit_relevance_score(
                &n,
                &CodeHit {
                    path: "crates/atomcode-clix/src/main.rs".into(),
                    line: 176,
                    snippet:
                        "// `-` means stdin; only ONE of diff/task/persona/append may read it."
                            .into(),
                    source: "g".into(),
                }
            ),
            0,
            "relevance score must be 0"
        );
        assert!(clix.is_empty(), "clix stdin must drop for 口令: {clix:?}");
    }

    #[test]
    fn body_mentioned_path_is_injected_as_hit() {
        let root = std::env::temp_dir().join(format!(
            "rg-body-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("crates/atomcode-cli/src/telemetry_cmd.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "use std::io;\n\npub fn dump(atomcode_dir: &std::path::Path) -> io::Result<()> {\n    let qdir = atomcode_dir.join(\"telemetry/queue\");\n    let _ = std::fs::read_to_string(&qdir.join(\"seg\"))?;\n    Ok(())\n}\n",
        )
        .unwrap();
        let hit = hit_from_mentioned_path(&root, "crates/atomcode-cli/src/telemetry_cmd.rs")
            .expect("inject hit");
        assert!(hit.path.contains("telemetry_cmd.rs"), "{hit:?}");
        assert!(
            hit.snippet.contains("dump")
                || hit.snippet.contains("queue")
                || hit.snippet.contains("read"),
            "{hit:?}"
        );
        let q = root.join("crates/atomcode-telemetry/src/queue/mod.rs");
        std::fs::create_dir_all(q.parent().unwrap()).unwrap();
        std::fs::write(&q, "pub struct Queue {}\n").unwrap();
        let hit2 = hit_from_mentioned_path(&root, "crates/atomcode-core/src/queue/mod.rs")
            .expect("fallback queue path");
        assert!(hit2.path.contains("queue"), "{hit2:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filter_non_auth_issue_drops_oauth_even_with_error_colon() {
        // #669 / #880 / #710 类：非 oauth 主题，error: 不得抬 oauth.rs
        let telemetry = normalize_issue(
            "[Bug] telemetry status/dump 全量读取队列导致 OOM",
            "相关代码位于 `crates/atomcode-cli/src/telemetry_cmd.rs` 和 `crates/atomcode-core/src/queue/mod.rs`。一次性读取整个队列文件到内存。",
        );
        let payload = normalize_issue(
            "请求体超出 20MB 限制时应主动防护",
            "API error (413 Payload Too Large) 请求体超过 20MB 限制",
        );
        let wecom = normalize_issue(
            "WeCom WebSocket errcode 846609",
            "gateway fails to send response via WeCom WebSocket aibot not subscribed",
        );
        let oauth_hit = CodeHit {
            path: "crates/atomcode-auth/src/oauth.rs".into(),
            line: 570,
            snippet: "TLS 1.2 fallback also failed after initial error: {first:#}".into(),
            source: "g".into(),
        };
        let hooks_hit = CodeHit {
            path: "crates/atomcode-capabilities/src/cc_hooks.rs".into(),
            line: 1350,
            snippet: "is_error: false,".into(),
            source: "g".into(),
        };
        let tele_hit = CodeHit {
            path: "crates/atomcode-cli/src/telemetry_cmd.rs".into(),
            line: 40,
            snippet: "fn dump_queue() { /* read whole queue file */ }".into(),
            source: "g".into(),
        };
        let queue_hit = CodeHit {
            path: "crates/atomcode-core/src/queue/mod.rs".into(),
            line: 10,
            snippet: "pub struct Queue { /* segments */ }".into(),
            source: "g".into(),
        };
        for (n, label) in [
            (&telemetry, "telemetry"),
            (&payload, "payload"),
            (&wecom, "wecom"),
        ] {
            assert_eq!(
                hit_relevance_score(n, &oauth_hit),
                0,
                "{label}: oauth must score 0"
            );
            assert_eq!(
                hit_relevance_score(n, &hooks_hit),
                0,
                "{label}: cc_hooks must score 0"
            );
        }
        let f = filter_relevant_hits(
            &telemetry,
            vec![oauth_hit.clone(), hooks_hit, tele_hit, queue_hit],
        );
        assert!(
            f.iter()
                .all(|h| !h.path.contains("oauth") && !h.path.contains("cc_hooks")),
            "{f:?}"
        );
        assert!(
            f.iter()
                .any(|h| h.path.contains("telemetry_cmd") || h.path.contains("queue")),
            "body-named paths must win: {f:?}"
        );
        // 正文点名路径抽取
        let paths = extract_mentioned_source_paths(
            "见 `crates/atomcode-cli/src/telemetry_cmd.rs` 与 crates/atomcode-core/src/queue/mod.rs",
        );
        assert!(
            paths.iter().any(|p| p.contains("telemetry_cmd")),
            "{paths:?}"
        );
    }

    #[test]
    fn filter_network_drops_cc_hooks_codeintel_and_error_token_false_positive() {
        let n = normalize_issue(
            "频繁的出现 [Error: 网络连接中断:远端关闭或重置了连接",
            "error decoding response body / 网络连接中断",
        );
        // 确保 error_signatures 含 error: 也不抬 is_error
        assert!(
            n.error_signatures
                .iter()
                .any(|s| s.contains("网络") || s.contains("decoding")),
            "{:?}",
            n.error_signatures
        );
        let hits = vec![
            CodeHit {
                path: "crates/atomcode-capabilities/src/cc_hooks.rs".into(),
                line: 1350,
                snippet: "is_error: false,".into(),
                source: "g".into(),
            },
            CodeHit {
                path: "crates/atomcode-capabilities/src/codeintel/lsp/client.rs".into(),
                line: 10,
                snippet: "fn connect() { /* lsp */ }".into(),
                source: "g".into(),
            },
            CodeHit {
                path: "crates/atomcode-capabilities/src/provider/retry.rs".into(),
                line: 188,
                snippet: "\"网络连接中断:远端关闭或重置了连接\"".into(),
                source: "g".into(),
            },
        ];
        // Error⊂is_error：单独计分应为 0
        let hook_score = hit_relevance_score(&n, &hits[0]);
        assert_eq!(
            hook_score, 0,
            "cc_hooks is_error must score 0, got {hook_score}"
        );
        let lsp_score = hit_relevance_score(&n, &hits[1]);
        assert_eq!(lsp_score, 0, "codeintel must score 0, got {lsp_score}");
        let f = filter_relevant_hits(&n, hits);
        assert!(
            f.iter()
                .all(|h| !h.path.contains("cc_hooks") && !h.path.contains("codeintel")),
            "{f:?}"
        );
        assert!(f.iter().any(|h| h.path.contains("retry")), "{f:?}");
        // 直接断言 error: 不匹配 is_error
        assert!(!error_sig_matches_text("error:", "is_error: false,"));
        assert!(error_sig_matches_text("error:", "Error: 网络连接中断"));
    }

    #[test]
    fn filter_auto_confirm_prefers_approval_not_bash_askpass() {
        let n = normalize_issue(
            "[Bug] webui的Auto模式在Deepseek模型时有时还要手动确认",
            "Auto 模式下偶现还要手动确认",
        );
        let hits = vec![
            CodeHit {
                path: "crates/atomcode-capabilities/src/tools/bash.rs".into(),
                line: 52,
                snippet: "// actually wired (Unix interactive TUI); off elsewhere (webui/headless/"
                    .into(),
                source: "g".into(),
            },
            CodeHit {
                path: "crates/atomcode-daemon/src/approval_mode.rs".into(),
                line: 10,
                snippet: "pub enum ApprovalMode { Auto, Manual }".into(),
                source: "g".into(),
            },
        ];
        let f = filter_relevant_hits(&n, hits);
        assert!(f.iter().all(|h| !h.path.contains("bash")), "{f:?}");
        assert!(f.iter().any(|h| h.path.contains("approval_mode")), "{f:?}");
    }

    #[test]
    fn filter_sync_latency_prefers_live_api_not_provider_footer() {
        let n = normalize_issue(
            "[Bug] WebUI sync 模式下消息延迟 2~10 秒才同步到 TUI",
            "ensure_headless_runtime 之前 InputAccepted 广播延迟",
        );
        let hits = vec![
            CodeHit {
                path: "crates/atomcode-cli/src/main.rs".into(),
                line: 2169,
                snippet: "synchronized WebUI tabs expose and reload the wrong provider".into(),
                source: "g".into(),
            },
            CodeHit {
                path: "crates/atomcode-tuix/src/event_loop/commands.rs".into(),
                line: 839,
                snippet:
                    "// conversation output. Do not broadcast them to phone/WebUI merely because"
                        .into(),
                source: "g".into(),
            },
            CodeHit {
                path: "crates/atomcode-daemon/src/live_api.rs".into(),
                line: 100,
                snippet: "fn ensure_headless_runtime() { /* InputAccepted broadcast */ }".into(),
                source: "g".into(),
            },
        ];
        let f = filter_relevant_hits(&n, hits);
        assert!(
            f.iter().all(|h| !h.snippet.contains("wrong provider")),
            "{f:?}"
        );
        assert!(
            f.iter().all(|h| !h.path.contains("commands.rs")),
            "tuix usage broadcast must drop: {f:?}"
        );
        assert!(
            f.iter().all(|h| !h.path.contains("main.rs")),
            "cli provider footer must drop: {f:?}"
        );
        assert!(f.iter().any(|h| h.path.contains("live_api")), "{f:?}");
    }

    #[test]
    fn filter_skill_dedup_prefers_skills_impl_not_cargo_toml() {
        let n = normalize_issue(
            "bug: 同名 skill 去重逻辑误删 — /skills dedup-skill",
            "两个目录同 name frontmatter 时 /skills 为空",
        );
        let hits = vec![
            CodeHit {
                path: "crates/atomcode-capabilities/Cargo.toml".into(),
                line: 174,
                snippet: "# Skills: markdown/frontmatter skill loader + use_skill".into(),
                source: "git_grep".into(),
            },
            CodeHit {
                path: "crates/atomcode-capabilities/src/skills/registry.rs".into(),
                line: 80,
                snippet: "fn dedup_by_name(skills: Vec<Skill>) -> Vec<Skill>".into(),
                source: "git_grep".into(),
            },
        ];
        let filtered = filter_relevant_hits(&n, hits);
        assert!(
            filtered.iter().all(|h| !h.path.contains("Cargo.toml")),
            "Cargo.toml must drop: {filtered:?}"
        );
        assert!(
            filtered.iter().any(|h| h.path.contains("skills/registry")),
            "keep skills registry: {filtered:?}"
        );
    }

    #[test]
    fn already_fixed_requires_error_hit_and_file_bound_fix() {
        let n = normalize_issue(
            "网络连接中断",
            "error: 网络连接中断\n## Environment\nversion 1.2.3\n",
        );
        let code = vec![CodeHit {
            path: "src/retry.rs".into(),
            line: 10,
            snippet: "网络连接中断:远端关闭".into(),
            source: "git_grep".into(),
        }];
        // 无 fix 提交 → None
        assert!(score_already_fixed_strict(&n, &code, &["abc hello".into()], true).is_none());
        // 有 fix 但未深挖 → None
        assert!(
            score_already_fixed_strict(&n, &code, &["abc fix retry disconnect".into()], false)
                .is_none()
        );
        // 有 fix + 深挖 + 错误命中 + 版本 + 连接重置类提交 → AlreadyFixed
        let v = score_already_fixed_strict(
            &n,
            &code,
            &["7182883f fix(v2/provider): 连接重置加固重连".into()],
            true,
        );
        assert!(matches!(v, Some((IssueVerdict::AlreadyFixed, _))), "{v:?}");
        // oauth/tls 登录修复不得抬 AlreadyFixed
        assert!(
            score_already_fixed_strict(
                &n,
                &code,
                &["522c6f2a fix(tls): retry AtomGit connections with TLS 1.2".into()],
                true,
            )
            .is_none(),
            "tls oauth fix must not mark already_fixed"
        );
        assert!(
            score_already_fixed_strict(
                &n,
                &code,
                &["fcf0b5b6 fix(auth): harden concurrent token recovery".into()],
                true,
            )
            .is_none(),
            "unrelated fix(auth) must not mark already_fixed"
        );
        // 关键词 fix alone + 无 error 命中 snippet → score_verification 不 AlreadyFixed
        let weak = score_verification(
            &normalize_issue("random", "no special error here just bug"),
            &[CodeHit {
                path: "src/foo.rs".into(),
                line: 1,
                snippet: "fn foo() {}".into(),
                source: "g".into(),
            }],
            &[],
            &["deadbeef fix something".into()],
        );
        assert_ne!(weak.0, IssueVerdict::AlreadyFixed, "{weak:?}");
    }

    #[test]
    fn plan_extracts_terms() {
        let n = normalize_issue(
            "access violation on save",
            "error: access violation\n## Steps\n1. save\nfn save_config crashed",
        );
        let p = build_plan(&n);
        assert!(!p.search_terms.is_empty());
        assert!(!p.steps.is_empty());
    }

    #[test]
    fn level0_runs_on_this_repo() {
        let root = resolve_repo_root(None).expect("repo root");
        let n = normalize_issue(
            "IssueStore FTS query fails",
            "error in issues_fts MATCH\n## Actual\nquery fails\n## Expected\nhits\n",
        );
        let v = verify_level0(&root, &n, true).unwrap();
        assert!(v.enabled);
        // This repo should have issue store code mentioning fts or issues
        assert!(
            !v.code_hits.is_empty() || !v.plan.search_terms.is_empty(),
            "expected some search activity: {v:?}"
        );
    }

    #[test]
    fn bug_like_issue_triggers_level1_deep_dig() {
        let root = std::env::temp_dir().join(format!(
            "rg_issue_deep_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        // provider 调用 retry 里的错误包装
        std::fs::write(
            root.join("src/retry.rs"),
            r#"
pub fn wrap_network_error(err: &str) -> String {
    // UNIQUE_DEEP_DIG_MARKER: network connection reset by peer
    if err.contains("connection reset") {
        return format!("网络连接中断:远端关闭或重置了连接 ({err})");
    }
    err.to_string()
}

pub fn retry_loop(mut n: u32) -> Result<(), String> {
    while n > 0 {
        n -= 1;
        if n == 0 {
            return Err(wrap_network_error("connection reset by peer"));
        }
    }
    Ok(())
}
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/provider.rs"),
            r#"
use crate::retry::retry_loop;

pub async fn chat_stream() -> Result<(), String> {
    retry_loop(3)?;
    Ok(())
}
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub mod retry;\npub mod provider;\n",
        )
        .unwrap();
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "t@t.com"]);
        run_git(&root, &["config", "user.name", "t"]);
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", "add retry network error handling"]);

        let n = normalize_issue(
            "网络连接中断 when streaming",
            "error: 网络连接中断:远端关闭或重置了连接\n## Actual\nstream fails\n## Expected\nworks\n",
        );
        let v = verify_level0(&root, &n, true).unwrap();
        assert!(v.enabled, "{v:?}");
        assert!(
            !v.code_hits.is_empty(),
            "level0 should hit retry error string: terms={:?} hits={:?}",
            v.plan.search_terms,
            v.code_hits
        );
        assert!(
            v.deep_dig_ran,
            "bug-like with code hits must run level1: {v:?}"
        );
        assert!(
            !v.deep_dig.is_empty(),
            "level1 must produce deep dig blocks: {v:?}"
        );
        let dig = &v.deep_dig[0];
        // 多行上下文，不是 160 字单行 snippet
        assert!(
            dig.context.lines().count() >= 3,
            "expected multi-line expanded context, got: {}",
            dig.context
        );
        assert!(
            dig.context.contains("UNIQUE_DEEP_DIG_MARKER")
                || dig.context.contains("wrap_network_error")
                || dig.context.contains("网络连接中断"),
            "context should include expanded function body: {}",
            dig.context
        );
        assert!(
            dig.symbol.as_deref() == Some("wrap_network_error")
                || dig.symbol.as_deref() == Some("retry_loop")
                || dig.context.contains("fn "),
            "should resolve enclosing symbol: {:?}",
            dig.symbol
        );
        // 至少有文件历史或调用方之一
        assert!(
            !dig.file_commits.is_empty() || !dig.callers.is_empty() || !dig.notes.is_empty(),
            "deep dig should record history/callers/notes: {dig:?}"
        );
        assert!(
            v.evidence.iter().any(|e| e.starts_with("deep_dig=")),
            "evidence should mention deep_dig: {:?}",
            v.evidence
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn should_deep_dig_skips_when_no_code_hits() {
        let n = normalize_issue("x", "no real error here");
        assert!(!should_deep_dig(IssueVerdict::LikelyBug, &n, &[]));
    }

    fn run_git(root: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(
            st.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&st.stderr)
        );
    }
}
