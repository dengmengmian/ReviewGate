//! 审查产品 CLI：`review` / `security` / `diff` / `findings` / `agent` / `tool` / `index`。

use clap::{Parser, Subcommand, ValueEnum};
use std::io::IsTerminal;

#[derive(Subcommand)]
pub(crate) enum FindingsCmd {
    /// List findings from the saved session (JSON array)
    List {
        /// open | resolved | all
        #[arg(long, default_value = "open")]
        status: String,
        /// Include low-confidence findings the gate folded away
        #[arg(long, default_value_t = false)]
        include_filtered: bool,
    },
    /// Print one finding in full (accepts a sequence number or an id prefix)
    Show {
        /// Sequence number (e.g. 3) or finding id / unique id prefix, from `findings list`
        id: String,
    },
    /// Mark a finding as handled in this session
    Resolve {
        /// Sequence number (e.g. 3) or finding id / unique id prefix
        id: String,
        /// Optional note recorded with the resolution
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum IndexCmd {
    /// Build/refresh the whole-repo definition index into .reviewgate/cache/symbols.json
    Build,
}

/// diff 范围选择（review 与 diff 共用）。
#[derive(Parser)]
pub(crate) struct DiffArgs {
    /// Review the changes introduced by a single commit
    #[arg(long)]
    commit: Option<String>,
    /// Range start (used with --to, from the merge-base)
    #[arg(long)]
    from: Option<String>,
    /// Range end (used with --from)
    #[arg(long)]
    to: Option<String>,
}

/// 把 commit/from/to 解析成 DiffMode（缺省工作区）。review 与 diff 共用。
pub(crate) fn resolve_mode(
    commit: &Option<String>,
    from: &Option<String>,
    to: &Option<String>,
) -> anyhow::Result<reviewgate_core::diff::DiffMode> {
    use reviewgate_core::diff::DiffMode;
    Ok(match (commit, from, to) {
        (Some(c), _, _) => DiffMode::Commit(c.clone()),
        (_, Some(f), Some(t)) => DiffMode::Range {
            from: f.clone(),
            to: t.clone(),
        },
        (_, Some(_), None) | (_, None, Some(_)) => {
            anyhow::bail!("--from and --to must be provided together")
        }
        _ => DiffMode::Workspace,
    })
}

/// 拉取当前 PR/MR 上已有的人类评审讨论，渲染成注入 prompt 的文本。
///
/// 取不到（非 PR 上下文 / 无 token / 平台暂不支持）时返回 `None` 并明说原因——
/// 它只是降噪上下文，不该让审查失败；但也不能默默当成"没人讨论过"。
async fn fetch_pr_discussion() -> Option<String> {
    use reviewgate_core::forge::{discussion, resolve_context_any};
    let Some(ctx) = resolve_context_any().await else {
        eprintln!("  [discussion] no PR/MR context detected; skipping (--with-pr-discussion)");
        return None;
    };
    let notes = match discussion::fetch(&ctx).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("  [discussion] fetch failed (ignored): {e}");
            return None;
        }
    };
    if notes.is_empty() {
        eprintln!("  [discussion] no existing human review comments on this PR/MR");
        return None;
    }
    let text = discussion::render_notes(&notes, discussion::MAX_DISCUSSION_CHARS);
    eprintln!(
        "  [discussion] injecting {} existing review comment(s) as context",
        notes.len()
    );
    Some(text)
}

/// `--since-last-review` 的范围解析：以上次审查时的 HEAD 为基准，只审之后新增的改动
/// （新提交 + 未提交编辑）。
///
/// **失败即报错，绝不降级**：没有上次会话、上次没记 sha、sha 在本仓库找不到（rebase / force-push）
/// 时一律 bail。悄悄退回全量审查会让人以为"增量审过了"，退回更小范围则会漏审——两种都不能接受。
async fn resolve_since_last_review(
    commit: &Option<String>,
    from: &Option<String>,
    to: &Option<String>,
) -> anyhow::Result<reviewgate_core::diff::DiffMode> {
    use reviewgate_core::diff::{git, DiffMode};
    if commit.is_some() || from.is_some() || to.is_some() {
        anyhow::bail!("--since-last-review cannot be combined with --commit / --from / --to");
    }
    let root = git::repo_root().await?;
    let session = reviewgate_core::review::FindingSession::load(std::path::Path::new(&root))
        .map_err(|e| {
            anyhow::anyhow!("{e}\n  --since-last-review needs a previous review to start from")
        })?;
    let Some(sha) = session.head_sha.clone() else {
        anyhow::bail!(
            "the last review did not record a base commit — run a full `reviewgate review` once first"
        );
    };
    // 基准必须仍然可达：rebase / force-push 后旧 sha 会消失，此时增量范围没有意义。
    if git::git(&["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .await
        .is_err()
    {
        anyhow::bail!(
            "the previously reviewed commit {sha} is no longer in this repository (rebased or force-pushed?) — run a full `reviewgate review`"
        );
    }
    eprintln!("  [since] reviewing changes since {sha} (previous review)");
    Ok(DiffMode::Since(sha))
}

/// 解析意图文本：优先 `--intent`（文件路径，或 `-` 读 stdin）；否则 `--intent-from-commit` 用提交信息。
/// 这是「意图作为每次不同的输入」的入口——与常驻的 `business.rules` 正交。
pub(crate) fn resolve_intent(args: &ReviewArgs) -> anyhow::Result<Option<String>> {
    use anyhow::Context;
    let normalize = |s: String| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    if let Some(src) = &args.intent {
        let text = if src == "-" {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read intent from stdin")?;
            buf
        } else {
            std::fs::read_to_string(src)
                .with_context(|| format!("failed to read intent file: {src}"))?
        };
        return Ok(normalize(text));
    }
    if args.intent_from_commit {
        let Some(sha) = &args.commit else {
            anyhow::bail!("--intent-from-commit requires --commit");
        };
        let out = std::process::Command::new("git")
            .args(["log", "-1", "--format=%B", sha])
            .output()
            .context("failed to run git to read the commit message")?;
        if !out.status.success() {
            anyhow::bail!("failed to read commit message for {sha}");
        }
        return Ok(normalize(String::from_utf8_lossy(&out.stdout).to_string()));
    }
    Ok(None)
}

/// Output format. Invalid values are rejected at parse time (not silently coerced to text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    Text,
    Json,
}

/// Which verdict triggers a non-zero exit code. Invalid values are rejected at parse time,
/// so a typo (e.g. `--fail-on blcok`) can never silently disable the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FailOn {
    Block,
    Warn,
    Never,
}

#[derive(Parser)]
pub(crate) struct ReviewArgs {
    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
    /// Review dimensions: all, or a comma-separated list of security,perf,logic,style,ai_smell,business
    #[arg(long, default_value = "all")]
    dimensions: String,
    /// Review the changes introduced by a single commit
    #[arg(long)]
    commit: Option<String>,
    /// Range review start (used with --to, from the merge-base)
    #[arg(long)]
    from: Option<String>,
    /// Range review end (used with --from)
    #[arg(long)]
    to: Option<String>,
    /// Review only what changed since the last review (new commits + uncommitted edits). Errors out if there is no usable previous review.
    #[arg(long, default_value_t = false)]
    since_last_review: bool,
    /// Feed the PR/MR's existing reviewer discussion into the review as context, so already-raised points are not re-reported (needs PR context; GitHub only for now)
    #[arg(long, default_value_t = false)]
    with_pr_discussion: bool,
    /// Skip the counter-evidence judge (faster, but more false positives)
    #[arg(long)]
    no_judge: bool,
    /// Show filtered low-confidence findings
    #[arg(long)]
    show_filtered: bool,
    /// Which verdict triggers a non-zero exit code
    #[arg(long, value_enum, default_value = "block")]
    fail_on: FailOn,
    /// Post a summary comment on the GitHub PR (for GitHub Action)
    #[arg(long)]
    comment: bool,
    /// Print per-dimension, per-round progress to stderr
    #[arg(long, short)]
    verbose: bool,
    /// Per-dimension wall-clock timeout (seconds, 0=unlimited). On timeout, skip that dimension and keep the rest; useful as a CI fallback.
    #[arg(long, default_value = "0")]
    timeout: u64,
    /// Samples per dimension (default 1). >1 unions findings; same-dimension severity/confidence take the median. Multi-unit diffs pin this to 1.
    #[arg(long, default_value = "1")]
    samples: usize,
    /// Judge concurrency limit, to avoid provider rate limits when there are many candidates.
    #[arg(long, default_value = "4")]
    judge_concurrency: usize,
    /// Fan-out concurrency limit (units × dimensions × samples), to avoid provider rate limits on large PRs.
    #[arg(long, default_value = "6")]
    fanout_concurrency: usize,
    /// After per-finding y/N confirmation, apply suggestion_code to working-tree files (not applied when non-interactive).
    #[arg(long)]
    fix: bool,
    /// Apply all auto-applicable fixes without per-finding confirmation. Unlike --fix, works non-interactively (CI/scripts).
    #[arg(long)]
    fix_all: bool,
    /// With --fix/--fix-all, apply the fixes on a new git branch instead of the current one.
    /// Optionally name it; omit the value to auto-generate (reviewgate-fix-<timestamp>).
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    fix_branch: Option<String>,
    /// Enable run_check sandboxed execution (lets the logic dimension actually run edge cases to verify subtle algorithms).
    /// Runs model-generated self-contained JS/Python snippets — use only in trusted/CI sandbox environments. Off by default.
    #[arg(long)]
    exec_verify: bool,
    /// Path to an intent/reference doc (requirement/design/acceptance criteria); `-` reads stdin. When set, runs a separate "implementation vs intent" technical review.
    #[arg(long)]
    intent: Option<String>,
    /// Use this commit's message as the intent (only in --commit mode; --intent takes precedence if both are given).
    #[arg(long)]
    intent_from_commit: bool,
    /// Incremental review (opt-in): cache findings per file and only re-review files whose diff changed since the last run.
    /// Trades coverage for cost/latency on iterative PRs — see docs/LIMITATIONS.md. Cache lives in .reviewgate/cache/ (self-ignored).
    #[arg(long)]
    incremental: bool,
    /// Run profile: `gate` (default, precision) or `audit` (wider: samples≥2, includes style).
    #[arg(long, default_value = "gate")]
    profile: String,
    /// Abort before LLM if estimated USD exceeds this. Refuses to start when the provider has no price_per_mtok_* (the budget would be unchecked).
    #[arg(long)]
    max_cost: Option<f64>,
    /// Abort before LLM if estimated input tokens exceed this upper bound.
    #[arg(long)]
    max_input_tokens: Option<u64>,
    /// Only print the pre-run cost estimate and unit plan; do not call the model.
    #[arg(long)]
    estimate_only: bool,
    /// Do not append run metrics to `.reviewgate/cache/metrics.jsonl`.
    #[arg(long)]
    no_metrics: bool,
}

/// Security deep-review args: same range/format/gate flags as `review`, fixed deep profile.
#[derive(Parser)]
pub(crate) struct SecurityArgs {
    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
    /// Review the changes introduced by a single commit
    #[arg(long)]
    commit: Option<String>,
    /// Range review start (used with --to, from the merge-base)
    #[arg(long)]
    from: Option<String>,
    /// Range review end (used with --from)
    #[arg(long)]
    to: Option<String>,
    /// Skip the counter-evidence judge (faster, but more false positives)
    #[arg(long)]
    no_judge: bool,
    /// Show filtered low-confidence findings
    #[arg(long)]
    show_filtered: bool,
    /// Which verdict triggers a non-zero exit code
    #[arg(long, value_enum, default_value = "block")]
    fail_on: FailOn,
    /// Post a summary comment on the GitHub PR (for GitHub Action)
    #[arg(long)]
    comment: bool,
    /// Print per-dimension, per-round progress to stderr
    #[arg(long, short)]
    verbose: bool,
    /// Per-dimension wall-clock timeout (seconds, 0=unlimited)
    #[arg(long, default_value = "0")]
    timeout: u64,
    /// Stop discovery after this many consecutive rounds with no new findings
    #[arg(long, default_value = "2")]
    stop_after_no_new: usize,
    /// Hard cap on discovery rounds; hitting it marks the review incomplete
    #[arg(long, default_value = "6")]
    max_rounds: usize,
    /// Judge concurrency limit
    #[arg(long, default_value = "4")]
    judge_concurrency: usize,
    /// Fan-out concurrency limit
    #[arg(long, default_value = "6")]
    fanout_concurrency: usize,
    /// After per-finding y/N confirmation, apply suggestion_code to working-tree files
    #[arg(long)]
    fix: bool,
    /// Apply all auto-applicable fixes without per-finding confirmation
    #[arg(long)]
    fix_all: bool,
    /// With --fix/--fix-all, apply the fixes on a new git branch
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    fix_branch: Option<String>,
    /// Incremental review (opt-in)
    #[arg(long)]
    incremental: bool,
}

pub(crate) async fn index_build() -> anyhow::Result<()> {
    use reviewgate_core::index::RepoIndex;
    let root = reviewgate_core::diff::git::repo_root().await?;
    let root = std::path::Path::new(&root);
    let idx = RepoIndex::build(root).await?;
    idx.save(root)?;
    eprintln!(
        "Indexed {} symbols ({} definitions) into .reviewgate/cache/symbols.json",
        idx.symbol_count(),
        idx.definition_count()
    );
    Ok(())
}

pub(crate) fn parse_dimension(s: &str) -> anyhow::Result<reviewgate_core::model::Dimension> {
    use reviewgate_core::model::Dimension::*;
    Ok(match s {
        "security" => Security,
        "perf" => Perf,
        "logic" => Logic,
        "style" => Style,
        "ai_smell" => AiSmell,
        "business" => Business,
        other => anyhow::bail!("unknown dimension: {other}"),
    })
}

pub(crate) fn parse_dimensions(s: &str) -> anyhow::Result<Vec<reviewgate_core::model::Dimension>> {
    use reviewgate_core::model::Dimension;
    if s.trim() == "all" {
        return Ok(Dimension::ALL.to_vec());
    }
    s.split(',').map(|p| parse_dimension(p.trim())).collect()
}

pub(crate) async fn agent_run(dimension: &str) -> anyhow::Result<()> {
    use reviewgate_core::agent::{build_user_prompt, run_agent, AgentConfig};
    use reviewgate_core::config::Config;
    use reviewgate_core::diff::{self, DiffMode};
    use reviewgate_core::llm::build_client;
    use reviewgate_core::tool::{readonly_tools, ToolContext, ToolRegistry};
    use std::sync::Arc;

    let dim = parse_dimension(dimension)?;
    let cfg = Config::load()?;
    let client = build_client(&cfg.active_provider_resolved()?)?;

    let root = diff::git::repo_root().await?;
    let d = Arc::new(diff::collect(&DiffMode::Workspace).await?);
    if d.files.is_empty() {
        eprintln!("{}", crate::i18n::Lang::detect().no_changes());
        return Ok(());
    }
    // 只传共享大块；维度聚焦块由 run_agent 注入（见 review 路径说明）。
    let user_prompt = build_user_prompt(&d.render_for_prompt());

    let ctx = ToolContext::with_grep_index(d.clone(), root.clone(), None);
    let mut reg = ToolRegistry::new();
    for t in readonly_tools() {
        reg.register(t);
    }

    let agent_cfg = AgentConfig::for_dimension(dim);
    eprintln!(
        "Running dimension [{}] with model {} ...",
        dim,
        client.model()
    );
    let mut findings = run_agent(&*client, &reg, &ctx, &agent_cfg, Arc::new(user_prompt)).await?;

    // M1.9 行号重定位。
    reviewgate_core::relocate::relocate_all(&mut findings, std::path::Path::new(&root), &None, &d)
        .await;

    println!("{}", serde_json::to_string_pretty(&findings)?);
    eprintln!("{} findings.", findings.len());
    Ok(())
}

// ───────────────────────── 发现会话（agent 增量消费） ─────────────────────────

/// 当前 UNIX 秒（会话时间戳）。系统时钟异常时退化为 0，不影响功能。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 把本次审查结果写成发现会话。
async fn save_findings_session(
    outcome: &reviewgate_core::review::ReviewOutcome,
) -> anyhow::Result<()> {
    use reviewgate_core::review::FindingSession;
    let root = reviewgate_core::diff::git::repo_root().await?;
    // 记下本次审查的基准 commit，供下次 `--since-last-review` 只审新增部分。
    // 取不到就存 None——下次会明确拒绝增量，而不是拿一个猜的基准少审。
    let head_sha = reviewgate_core::diff::git::git(&["rev-parse", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    FindingSession::new(
        &outcome.findings,
        &outcome.decision.as_str().to_lowercase(),
        outcome.files_changed,
        outcome.incomplete,
        now_secs(),
        head_sha,
    )
    .save(std::path::Path::new(&root))
}

fn parse_finding_status(s: &str) -> anyhow::Result<Option<reviewgate_core::review::FindingStatus>> {
    use reviewgate_core::review::FindingStatus;
    match s.trim().to_ascii_lowercase().as_str() {
        "open" => Ok(Some(FindingStatus::Open)),
        "resolved" => Ok(Some(FindingStatus::Resolved)),
        "all" => Ok(None),
        other => anyhow::bail!("unknown --status `{other}` (use open | resolved | all)"),
    }
}

async fn load_session(
) -> anyhow::Result<(std::path::PathBuf, reviewgate_core::review::FindingSession)> {
    let root = std::path::PathBuf::from(reviewgate_core::diff::git::repo_root().await?);
    let session = reviewgate_core::review::FindingSession::load(&root)?;
    Ok((root, session))
}

/// `reviewgate findings list`：JSON 输出。**始终**带上 incomplete/decision——
/// 只给一个空数组会让 agent 把"没审完"读成"没问题"。
pub(crate) async fn findings_list(status: &str, include_filtered: bool) -> anyhow::Result<()> {
    let want = parse_finding_status(status)?;
    let (_, session) = load_session().await?;
    let selected = session.select(want, include_filtered);
    let out = serde_json::json!({
        "run_id": session.run_id,
        "created_at": session.created_at,
        "decision": session.decision,
        "files_changed": session.files_changed,
        "incomplete": session.incomplete,
        "count": selected.len(),
        "findings": selected,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

pub(crate) async fn findings_show(id: &str) -> anyhow::Result<()> {
    let (_, session) = load_session().await?;
    println!("{}", serde_json::to_string_pretty(session.find(id)?)?);
    Ok(())
}

pub(crate) async fn findings_resolve(id: &str, note: Option<String>) -> anyhow::Result<()> {
    let (root, mut session) = load_session().await?;
    let record = session.resolve(id, note, now_secs())?.clone();
    session.save(&root)?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

pub(crate) async fn tool_call(name: &str, input: &str) -> anyhow::Result<()> {
    use reviewgate_core::diff::{self, DiffMode};
    use reviewgate_core::tool::{readonly_tools, ToolContext, ToolRegistry};
    use std::sync::Arc;

    let root = diff::git::repo_root().await?;
    let d = Arc::new(diff::collect(&DiffMode::Workspace).await?);
    let ctx = ToolContext::with_treesitter_index(d, root, None);

    let mut reg = ToolRegistry::new();
    for t in readonly_tools() {
        reg.register(t);
    }
    let args: serde_json::Value = serde_json::from_str(input)?;
    let result = reg.dispatch(name, &args, &ctx).await?;
    println!("{result}");
    Ok(())
}

pub(crate) fn validate_review_args(args: &ReviewArgs) -> anyhow::Result<()> {
    // `--fix-branch` 只在真正要应用修复时才有意义。
    if args.fix_branch.is_some() && !(args.fix || args.fix_all) {
        anyhow::bail!("--fix-branch only applies with --fix or --fix-all");
    }
    Ok(())
}

fn validate_security_args(args: &SecurityArgs) -> anyhow::Result<()> {
    if args.fix_branch.is_some() && !(args.fix || args.fix_all) {
        anyhow::bail!("--fix-branch only applies with --fix or --fix-all");
    }
    Ok(())
}

/// Shared post-review presentation + exit code (used by `review` and `security`).
pub(crate) struct ReviewRunArgs {
    /// 仅估算、未真正审查。为 true 时**不写发现会话**——什么都没审就推进增量基准，
    /// 会让下一次 `--since-last-review` 跳过一整段从未审过的改动。
    pub(crate) estimate_only: bool,
    pub(crate) format: OutputFormat,
    pub(crate) show_filtered: bool,
    pub(crate) comment: bool,
    pub(crate) fix: bool,
    pub(crate) fix_all: bool,
    pub(crate) fix_branch: Option<String>,
    pub(crate) fail_on: FailOn,
    pub(crate) verbose: bool,
}

/// 走哪条编排管线。两条线共用进度渲染、输出、退出码语义，但阶段序列不同：
/// review 是固定采样的多维闸口，security 是饱和式召回的安全深审。
pub(crate) enum Line {
    Review,
    Security {
        stop_after_no_new: usize,
        max_rounds: usize,
    },
}

pub(crate) async fn present_and_exit(
    cfg: &reviewgate_core::config::Config,
    opts: reviewgate_core::review::ReviewOptions,
    line: Line,
    run: ReviewRunArgs,
) -> anyhow::Result<i32> {
    use reviewgate_core::review::run_review;
    use reviewgate_core::security::{run_security, SecurityOptions};

    let live = std::io::stderr().is_terminal() && run.format != OutputFormat::Json && !run.verbose;
    let progress = live.then(|| std::sync::Arc::new(reviewgate_core::progress::Progress::new()));
    let mut opts = opts;
    opts.progress = progress.clone();
    let render = progress.clone().map(|p| {
        let t = crate::i18n::Lang::detect();
        tokio::spawn(async move {
            const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            const LINE_WIDTH: usize = 60;
            let reviewing = t.reviewing();
            let start = std::time::Instant::now();
            let mut i = 0usize;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                let (n, last) = p.snapshot();
                let s = start.elapsed().as_secs();
                let suffix = format!(" · {} · {}:{:02}", t.calls(n), s / 60, s % 60);
                let fixed = 5 + crate::render::display_width(reviewing) + crate::render::display_width(&suffix);
                let budget = LINE_WIDTH.saturating_sub(fixed);
                let last = crate::render::truncate_to_width(&last, budget);
                eprint!(
                    "\r\x1b[2K\x1b[36m{}\x1b[0m {reviewing} \x1b[2m·\x1b[0m {last}\x1b[2m{suffix}\x1b[0m",
                    FRAMES[i % FRAMES.len()],
                );
                let _ = std::io::Write::flush(&mut std::io::stderr());
                i += 1;
            }
        })
    });

    // 退出码语义要用到，但 opts 可能被 move 进 SecurityOptions，故先取出。
    let deep = opts.profile.is_deep();
    let started = std::time::Instant::now();
    let outcome = match line {
        Line::Review => run_review(cfg, &opts).await?,
        Line::Security {
            stop_after_no_new,
            max_rounds,
        } => {
            let mut so = SecurityOptions::new(opts);
            so.stop_after_no_new = stop_after_no_new;
            so.max_rounds = max_rounds;
            run_security(cfg, &so).await?
        }
    };

    if let Some(h) = render {
        h.abort();
        let t = crate::i18n::Lang::detect();
        let (n, _) = progress.as_ref().unwrap().snapshot();
        let s = started.elapsed().as_secs();
        eprint!("\r\x1b[2K");
        eprintln!(
            "\x1b[32m✓\x1b[0m {} \x1b[2m· {} · {}:{:02}\x1b[0m",
            t.review_complete(),
            t.tool_calls(n),
            s / 60,
            s % 60
        );
    }

    match run.format {
        OutputFormat::Json => println!("{}", crate::render::render_json(&outcome)?),
        OutputFormat::Text => {
            // 团队自定义的严重度标签/配色（配置写错在审查开始时就已报错，这里必然可解析）。
            let labels = reviewgate_core::config::SeverityLabels::resolve(&cfg.severity_labels)?;
            print!(
                "{}",
                crate::render::render_text_with_labels(&outcome, run.show_filtered, labels)
            )
        }
    }

    // 落盘发现会话，供 `reviewgate findings list/show/resolve` 增量消费（agent 修复循环）。
    // 写失败只提示，不影响审查结论——它是便利设施，不是闸口的一部分。
    if !run.estimate_only {
        if let Err(e) = save_findings_session(&outcome).await {
            eprintln!("  [session] write failed (ignored): {e}");
        }
    }

    if run.comment {
        if let Err(e) = reviewgate_core::forge::post_summary(&outcome).await {
            eprintln!("failed to post summary comment: {e}");
        }
        if let Err(e) =
            reviewgate_core::forge::post_inline_suggestions(&outcome, cfg.gate.block_threshold)
                .await
        {
            eprintln!("failed to post inline comments: {e}");
        }
    }

    if run.fix || run.fix_all {
        let root = reviewgate_core::diff::git::repo_root().await?;
        crate::fix::apply_fixes(
            &outcome.findings,
            std::path::Path::new(&root),
            run.fix_branch.as_deref(),
            run.fix_all,
        )?;
    }

    // Deep profile / critical-path incomplete always treat incomplete as non-PASS for exit semantics.
    let fail_incomplete = cfg.gate.fail_on_incomplete || deep || outcome.critical_incomplete;
    let incomplete_for_exit = outcome.incomplete || outcome.critical_incomplete;
    Ok(exit_code(
        outcome.decision,
        incomplete_for_exit,
        fail_incomplete,
        run.fail_on,
    ))
}

pub(crate) async fn review(args: &ReviewArgs) -> anyhow::Result<i32> {
    use reviewgate_core::config::Config;
    use reviewgate_core::review::{ReviewOptions, RunProfile, TokenPrices};

    validate_review_args(args)?;
    let run_profile = RunProfile::parse(&args.profile).ok_or_else(|| {
        anyhow::anyhow!("unknown --profile `{}` (use gate or audit)", args.profile)
    })?;

    // dimensions: "all" defers to profile; explicit list wins.
    let user_all = args.dimensions.trim() == "all";
    let dims = if user_all {
        run_profile.dimensions(true, None)
    } else {
        parse_dimensions(&args.dimensions)?
    };

    let cfg = Config::load()?;
    let names: Vec<&str> = dims.iter().map(|d| d.as_str()).collect();
    let auto_business = (!cfg.business.rules.is_empty()
        || cfg.business.rules_dir.is_some()
        || cfg.business.skills_dir.is_some())
        && !dims.contains(&reviewgate_core::model::Dimension::Business);
    let effective_dims = dims.len() + usize::from(auto_business);
    // samples: CLI flag if user raised it; else profile default (audit≥2).
    let samples = if args.samples > 1 {
        args.samples
    } else {
        run_profile.default_samples().max(args.samples.max(1))
    };
    let agents = effective_dims * samples;

    let mode = if args.since_last_review {
        resolve_since_last_review(&args.commit, &args.from, &args.to).await?
    } else {
        resolve_mode(&args.commit, &args.from, &args.to)?
    };
    let etty = std::io::stderr().is_terminal();
    let dim = |s: &str| {
        if etty {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let business = if auto_business { " + business" } else { "" };
    let samples_note = if samples > 1 {
        format!(" · samples={samples}")
    } else {
        String::new()
    };
    eprintln!(
        "ReviewGate {} [{}] {}{} {}",
        dim("reviewing"),
        run_profile.as_str(),
        names.join(", "),
        business,
        dim(&format!("· {agents} agents{samples_note}")),
    );

    let prices = cfg
        .active_provider()
        .map(|p| TokenPrices {
            per_mtok_input: p.price_per_mtok_input,
            per_mtok_output: p.price_per_mtok_output,
        })
        .unwrap_or_default();

    let mut opts = ReviewOptions::new(mode, dims);
    opts.judge = !args.no_judge;
    opts.gate = cfg.gate.clone();
    opts.verbose = args.verbose;
    if args.timeout > 0 {
        opts.timeout = Some(std::time::Duration::from_secs(args.timeout));
    }
    opts.samples = samples;
    opts.judge_concurrency = args.judge_concurrency.max(1);
    opts.fanout_concurrency = args.fanout_concurrency.max(1);
    opts.exec_verify = args.exec_verify;
    opts.incremental = args.incremental;
    opts.run_profile = run_profile;
    opts.max_cost_usd = args.max_cost;
    opts.max_est_input_tokens = args.max_input_tokens;
    opts.estimate_only = args.estimate_only;
    opts.write_metrics = !args.no_metrics;
    opts.token_prices = prices;
    opts.started = Some(std::time::Instant::now());
    opts.intent = resolve_intent(args)?;
    if opts.intent.is_some() {
        eprintln!("  + Intent review: intent loaded; running the implementation-vs-intent pass.");
    }
    if args.with_pr_discussion {
        opts.pr_discussion = fetch_pr_discussion().await;
    }

    if args.estimate_only {
        // Still go through present_and_exit so JSON/text can show the estimate.
        let code = present_and_exit(
            &cfg,
            opts,
            Line::Review,
            ReviewRunArgs {
                estimate_only: true,
                format: args.format,
                show_filtered: args.show_filtered,
                comment: false,
                fix: false,
                fix_all: false,
                fix_branch: None,
                fail_on: FailOn::Never,
                verbose: args.verbose,
            },
        )
        .await?;
        return Ok(code);
    }

    present_and_exit(
        &cfg,
        opts,
        Line::Review,
        ReviewRunArgs {
            estimate_only: false,
            format: args.format,
            show_filtered: args.show_filtered || run_profile == RunProfile::Audit,
            comment: args.comment,
            fix: args.fix,
            fix_all: args.fix_all,
            fix_branch: args.fix_branch.clone(),
            fail_on: args.fail_on,
            verbose: args.verbose,
        },
    )
    .await
}

/// Security deep review: same engine as `review`, deep profile defaults.
pub(crate) async fn security(args: &SecurityArgs) -> anyhow::Result<i32> {
    use reviewgate_core::config::Config;
    use reviewgate_core::review::ReviewOptions;

    validate_security_args(args)?;
    let cfg = Config::load()?;
    let mode = resolve_mode(&args.commit, &args.from, &args.to)?;

    let etty = std::io::stderr().is_terminal();
    let dim = |s: &str| {
        if etty {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    eprintln!(
        "ReviewGate {} security {} {}",
        dim("deep review"),
        dim("· sink inventory + secret precheck"),
        dim(&format!(
            "· saturating discovery (stop after {} idle rounds, max {})",
            args.stop_after_no_new.max(1),
            args.max_rounds.max(1)
        )),
    );

    let mut opts = ReviewOptions::security_deep(mode);
    opts.judge = !args.no_judge;
    opts.gate = cfg.gate.clone();
    opts.gate.fail_on_incomplete = true; // deep never treats incomplete as PASS
    opts.verbose = args.verbose;
    if args.timeout > 0 {
        opts.timeout = Some(std::time::Duration::from_secs(args.timeout));
    }
    opts.judge_concurrency = args.judge_concurrency.max(1);
    opts.fanout_concurrency = args.fanout_concurrency.max(1);
    opts.incremental = args.incremental;

    present_and_exit(
        &cfg,
        opts,
        Line::Security {
            stop_after_no_new: args.stop_after_no_new,
            max_rounds: args.max_rounds,
        },
        ReviewRunArgs {
            estimate_only: false,
            format: args.format,
            show_filtered: args.show_filtered,
            comment: args.comment,
            fix: args.fix,
            fix_all: args.fix_all,
            fix_branch: args.fix_branch.clone(),
            fail_on: args.fail_on,
            verbose: args.verbose,
        },
    )
    .await
}

/// CI 闸口退出码语义（纯函数，便于单测覆盖各组合）。
/// 未审完 + `fail_on_incomplete`：无论 `--fail-on` 取值一律非 0——杜绝"漏审却放行"。
pub(crate) fn exit_code(
    decision: reviewgate_core::gate::GateDecision,
    incomplete: bool,
    fail_on_incomplete: bool,
    fail_on: FailOn,
) -> i32 {
    use reviewgate_core::gate::GateDecision;
    if incomplete && fail_on_incomplete {
        return 1;
    }
    match (decision, fail_on) {
        (GateDecision::Block, FailOn::Block) | (GateDecision::Block, FailOn::Warn) => 1,
        (GateDecision::Warn, FailOn::Warn) => 1,
        _ => 0,
    }
}

pub(crate) async fn diff_summary(args: &DiffArgs) -> anyhow::Result<()> {
    use reviewgate_core::diff;

    let mode = resolve_mode(&args.commit, &args.from, &args.to)?;
    let d = diff::collect(&mode).await?;
    println!("Files changed: {}", d.files.len());
    for f in &d.files {
        println!(
            "  [{:?}{}] {}  (+{} -{}, {} hunks)",
            f.status,
            if f.binary { ",binary" } else { "" },
            f.path(),
            f.added_lines(),
            f.deleted_lines(),
            f.hunks.len(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        exit_code, parse_dimensions, resolve_intent, resolve_mode, validate_review_args, FailOn,
        OutputFormat, ReviewArgs,
    };
    use reviewgate_core::gate::GateDecision;
    use reviewgate_core::model::Dimension;

    fn review_args() -> ReviewArgs {
        ReviewArgs {
            format: OutputFormat::Text,
            dimensions: "all".into(),
            commit: None,
            from: None,
            to: None,
            no_judge: false,
            show_filtered: false,
            fail_on: FailOn::Block,
            comment: false,
            verbose: false,
            timeout: 0,
            samples: 1,
            judge_concurrency: 4,
            fanout_concurrency: 6,
            fix: false,
            fix_all: false,
            fix_branch: None,
            exec_verify: false,
            intent: None,
            since_last_review: false,
            with_pr_discussion: false,
            intent_from_commit: false,
            incremental: false,
            profile: "gate".into(),
            max_cost: None,
            max_input_tokens: None,
            estimate_only: false,
            no_metrics: false,
        }
    }

    #[test]
    fn resolve_mode_workspace_commit_range() {
        use reviewgate_core::diff::DiffMode;
        assert!(matches!(
            resolve_mode(&None, &None, &None).unwrap(),
            DiffMode::Workspace
        ));
        assert_eq!(
            resolve_mode(&Some("abc".into()), &None, &None).unwrap(),
            DiffMode::Commit("abc".into())
        );
        assert_eq!(
            resolve_mode(&None, &Some("main".into()), &Some("HEAD".into())).unwrap(),
            DiffMode::Range {
                from: "main".into(),
                to: "HEAD".into()
            }
        );
        assert!(resolve_mode(&None, &Some("main".into()), &None).is_err());
        assert!(resolve_mode(&None, &None, &Some("HEAD".into())).is_err());
    }

    #[test]
    fn resolve_intent_from_file_or_none() {
        let tmp = std::env::temp_dir().join(format!("rg_intent_{}", std::process::id()));
        std::fs::write(&tmp, "意图说明\n").unwrap();

        let mut args = review_args();
        args.intent = Some(tmp.to_str().unwrap().into());
        assert_eq!(resolve_intent(&args).unwrap(), Some("意图说明".into()));

        let args = review_args();
        assert_eq!(resolve_intent(&args).unwrap(), None);

        let mut args = review_args();
        args.intent = Some(tmp.to_str().unwrap().into());
        std::fs::write(&tmp, "   \n").unwrap();
        assert_eq!(resolve_intent(&args).unwrap(), None);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn exit_code_gate_and_fail_on_matrix() {
        assert_eq!(
            exit_code(GateDecision::Block, false, false, FailOn::Block),
            1
        );
        assert_eq!(
            exit_code(GateDecision::Block, false, false, FailOn::Warn),
            1
        );
        assert_eq!(
            exit_code(GateDecision::Block, false, false, FailOn::Never),
            0
        );
        assert_eq!(exit_code(GateDecision::Warn, false, false, FailOn::Warn), 1);
        assert_eq!(
            exit_code(GateDecision::Warn, false, false, FailOn::Block),
            0
        );
        assert_eq!(exit_code(GateDecision::Pass, false, false, FailOn::Warn), 0);
    }

    #[test]
    fn exit_code_incomplete_overrides_when_configured() {
        assert_eq!(exit_code(GateDecision::Pass, true, true, FailOn::Never), 1);
        assert_eq!(exit_code(GateDecision::Warn, true, true, FailOn::Block), 1);
        assert_eq!(exit_code(GateDecision::Pass, true, false, FailOn::Block), 0);
    }

    #[test]
    fn parse_dimensions_all_and_list_and_invalid() {
        assert_eq!(parse_dimensions("all").unwrap(), Dimension::ALL.to_vec());
        let list = parse_dimensions("security,logic").unwrap();
        assert_eq!(list, vec![Dimension::Security, Dimension::Logic]);
        let with_business = parse_dimensions("security,business").unwrap();
        assert!(with_business.contains(&Dimension::Business));
        assert!(parse_dimensions("security,bogus").is_err());
    }

    #[test]
    fn parse_dimension_rejects_unknown_and_empty() {
        assert!(super::parse_dimension("unknown").is_err());
        assert!(super::parse_dimension("").is_err());
        assert_eq!(
            super::parse_dimension("security").unwrap(),
            Dimension::Security
        );
    }

    #[test]
    fn fix_branch_requires_fix_or_fix_all() {
        let mut args = review_args();
        args.fix_branch = Some("".into());
        assert!(validate_review_args(&args).is_err());

        args.fix = true;
        assert!(validate_review_args(&args).is_ok());

        args.fix = false;
        args.fix_all = true;
        assert!(validate_review_args(&args).is_ok());
    }

    #[test]
    fn parse_dimensions_trims_whitespace() {
        assert_eq!(
            parse_dimensions(" security , logic , ai_smell ").unwrap(),
            vec![Dimension::Security, Dimension::Logic, Dimension::AiSmell]
        );
    }

    #[test]
    fn resolve_intent_from_commit_requires_commit() {
        let mut args = review_args();
        args.intent_from_commit = true;
        assert!(resolve_intent(&args).is_err());
    }

    #[test]
    fn resolve_intent_stdin_dash() {
        let mut args = review_args();
        args.intent = Some("-".into());
        assert_eq!(args.intent.as_deref(), Some("-"));
    }

    #[test]
    fn exit_code_fail_on_never_allows_block_and_warn() {
        assert_eq!(
            exit_code(GateDecision::Block, false, false, FailOn::Never),
            0
        );
        assert_eq!(
            exit_code(GateDecision::Warn, false, false, FailOn::Never),
            0
        );
    }
}
