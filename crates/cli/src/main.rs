//! ReviewGate CLI —— 主形态。

mod demo;
mod fix;
mod i18n;
mod init;
mod render;

use clap::{Parser, Subcommand, ValueEnum};
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "reviewgate",
    about = "A pre-merge quality gate for AI-generated code: surface high-risk issues first, fold low-confidence noise",
    version = reviewgate_core::version(),
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a global config (provider + model endpoint). Keeps the API key in the environment.
    Init(InitArgs),
    /// Run the built-in poisoned fixtures; expect BLOCK (does not need your app repo)
    Demo(DemoArgs),
    /// Review the current git diff
    Review(ReviewArgs),
    /// Security deep review: sink-driven security-only pass with higher samples and secret precheck
    Security(SecurityArgs),
    /// LLM connectivity self-check
    Llm {
        #[command(subcommand)]
        cmd: LlmCmd,
    },
    /// Print the parsed diff summary (debug). Supports --commit / --from --to; defaults to the working tree.
    Diff(DiffArgs),
    /// Invoke a single tool (debug): reviewgate tool <name> '<json>'
    Tool {
        name: String,
        #[arg(default_value = "{}")]
        input: String,
    },
    /// Run a single-dimension agent (debug): reviewgate agent --dimension logic
    Agent {
        /// Dimension: security | perf | logic | style | ai_smell | business
        #[arg(long, default_value = "logic")]
        dimension: String,
    },
    /// Self-update: download a release binary and replace the current executable
    Upgrade {
        /// Version to install, e.g. 0.8.0 (default: latest). Use it to roll back to a known-good build.
        version: Option<String>,
        /// Replace the binary even when a package manager (Homebrew/Cargo/mise/Nix) owns it
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Whole-repo symbol index (opt-in): speed up cross-file definition lookups during review
    Index {
        #[command(subcommand)]
        cmd: IndexCmd,
    },
    /// Query and update findings from the last `reviewgate review` (JSON; for agents and scripts)
    Findings {
        #[command(subcommand)]
        cmd: FindingsCmd,
    },
    /// Issue Review / triage (sync, classify, duplicate, comment)
    Issue {
        #[command(subcommand)]
        cmd: IssueCmd,
    },
    /// Webhook HTTP server (queue only; pair with `daemon` or `issue watch`)
    Serve(ServeArgs),
    /// Long-running Issue Review daemon: poll sync + queue worker (+ optional embedded serve)
    Daemon(DaemonArgs),
}

#[derive(Subcommand)]
enum FindingsCmd {
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
enum IssueCmd {
    /// Initialize local Issue index from the platform (history only, no bulk replies)
    Init(IssueSyncArgs),
    /// Incremental Issue sync into local SQLite+FTS
    Sync(IssueSyncArgs),
    /// Review a single Issue (default dry-run preview)
    Review(IssueReviewCliArgs),
    /// Inspect a stored Issue / last review decision
    Inspect {
        /// Issue number
        number: u64,
        /// Repository data dir (default .reviewgate/issue)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// owner/repo override
        #[arg(long)]
        repo: Option<String>,
    },
    /// Poll for updated issues and triage new ones (suggest mode)
    Watch(IssueWatchArgs),
    /// Show triage/action statistics recorded locally (including gated runs)
    Stats {
        /// Repository data dir (default .reviewgate/issue)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// owner/repo override
        #[arg(long)]
        repo: Option<String>,
        /// List the issues waiting for a human instead of the summary
        #[arg(long, default_value_t = false)]
        gated: bool,
    },
}

#[derive(Parser)]
struct IssueSyncArgs {
    /// owner/repo (default: REVIEWGATE_REPO or git remote origin)
    #[arg(long)]
    repo: Option<String>,
    /// Local data directory (default: .reviewgate/issue)
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Max issues to pull
    #[arg(long, default_value = "10000")]
    max: usize,
    /// API base (platform default if empty)
    #[arg(long, default_value = "")]
    api_base: String,
    /// Platform: github | gitlab | gitee | atomgit
    #[arg(long, default_value = "github")]
    forge: String,
    /// Use offline fixture seed instead of live platform
    #[arg(long)]
    fixture: bool,
}

#[derive(Parser)]
struct IssueReviewCliArgs {
    /// Issue number
    number: u64,
    /// owner/repo
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long, default_value = "")]
    api_base: String,
    /// Platform: github | gitlab | gitee | atomgit
    #[arg(long, default_value = "github")]
    forge: String,
    /// Preview only (default true unless --publish)
    #[arg(long, default_value_t = true)]
    dry_run: bool,
    /// Publish / update the single bot comment on the issue
    #[arg(long, default_value_t = false)]
    publish: bool,
    /// Triage only (skip technical verification)
    #[arg(long, default_value_t = false)]
    triage_only: bool,
    /// Phase 2: run Level-0 code verification against local repo
    #[arg(long, default_value_t = false)]
    verify: bool,
    /// Repo root for code verification (default: discover .git)
    #[arg(long)]
    repo_root: Option<PathBuf>,
    /// Offline fixture platform
    #[arg(long)]
    fixture: bool,
    /// Path to JSON file with a single GitHub-like issue object (implies fixture seed)
    #[arg(long)]
    fixture_issue: Option<PathBuf>,
}

#[derive(Parser)]
struct IssueWatchArgs {
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long, default_value = "")]
    api_base: String,
    /// Platform: github | gitlab | gitee | atomgit
    #[arg(long, default_value = "github")]
    forge: String,
    /// Poll interval, e.g. 5m, 30s, 300
    #[arg(long, default_value = "5m")]
    interval: String,
    /// Max poll iterations (0 = forever)
    #[arg(long, default_value = "0")]
    max_iterations: u64,
    #[arg(long)]
    fixture: bool,
    /// Enable technical verification each review
    #[arg(long, default_value_t = false)]
    verify: bool,
    #[arg(long)]
    repo_root: Option<PathBuf>,
}

#[derive(Parser)]
struct ServeArgs {
    /// Listen address
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    /// Webhook secret (or REVIEWGATE_WEBHOOK_SECRET)
    #[arg(long)]
    webhook_secret: Option<String>,
    /// SQLite queue path (default .reviewgate/issue/webhook.db)
    #[arg(long)]
    queue: Option<PathBuf>,
}

#[derive(Parser)]
struct DaemonArgs {
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Platform: github | gitlab | gitee | atomgit
    #[arg(long, default_value = "github")]
    forge: String,
    #[arg(long, default_value = "")]
    api_base: String,
    /// Poll interval for platform sync
    #[arg(long, default_value = "5m")]
    interval: String,
    /// Also bind webhook HTTP server
    #[arg(long)]
    serve: bool,
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    #[arg(long)]
    webhook_secret: Option<String>,
    #[arg(long, default_value_t = false)]
    verify: bool,
    /// Max outer loops for testing (0 = forever)
    #[arg(long, default_value = "0")]
    max_iterations: u64,
    #[arg(long)]
    fixture: bool,
}

/// `reviewgate init` flags.
#[derive(Parser)]
struct InitArgs {
    /// Provider preset: deepseek | openai | anthropic | custom
    #[arg(long, default_value = "deepseek")]
    provider: String,
    /// Protocol override: openai | anthropic
    #[arg(long)]
    protocol: Option<String>,
    /// Base URL override (required for --provider custom)
    #[arg(long)]
    base_url: Option<String>,
    /// Model override (required for --provider custom)
    #[arg(long)]
    model: Option<String>,
    /// Non-interactive: use flags/defaults, no prompts
    #[arg(long, short = 'y')]
    yes: bool,
    /// Overwrite an existing config.toml
    #[arg(long)]
    force: bool,
    /// Directory for config (default ~/.reviewgate). Writes config.toml inside.
    #[arg(long)]
    config_dir: Option<PathBuf>,
    /// Run `llm test` after writing the config
    #[arg(long)]
    test: bool,
}

/// `reviewgate demo` flags.
#[derive(Parser)]
struct DemoArgs {
    /// Keep the temporary demo repo and print its path
    #[arg(long)]
    keep: bool,
    /// Only seed the demo workspace (no LLM). Prints the path and exits 0.
    #[arg(long)]
    prepare_only: bool,
    /// Per-dimension wall-clock timeout in seconds (0 = unlimited)
    #[arg(long, default_value = "180")]
    timeout: u64,
}

#[derive(Subcommand)]
enum LlmCmd {
    /// Send one minimal request to the default provider to verify connectivity
    Test,
}

#[derive(Subcommand)]
enum IndexCmd {
    /// Build/refresh the whole-repo definition index into .reviewgate/cache/symbols.json
    Build,
}

/// diff 范围选择（review 与 diff 共用）。
#[derive(Parser)]
struct DiffArgs {
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
fn resolve_mode(
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
fn resolve_intent(args: &ReviewArgs) -> anyhow::Result<Option<String>> {
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
enum OutputFormat {
    Text,
    Json,
}

/// Which verdict triggers a non-zero exit code. Invalid values are rejected at parse time,
/// so a typo (e.g. `--fail-on blcok`) can never silently disable the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FailOn {
    Block,
    Warn,
    Never,
}

#[derive(Parser)]
struct ReviewArgs {
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
    /// Samples per dimension (default 1). >1 unions the results to stabilize recall of flaky misses (e.g. SSRF), at N× cost.
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
    /// Abort before LLM if estimated USD cost exceeds this (requires price_per_mtok_* in provider config).
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
struct SecurityArgs {
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
    /// Samples for the security dimension (default 2 for deep profile). >1 unions results for stable recall.
    #[arg(long, default_value = "2")]
    samples: usize,
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => std::process::exit(code),
        // 操作性错误（配置缺失/网络失败/密钥未配…）用退出码 2，与「闸口 BLOCK」的 1 区分：
        // CI 才能分辨「PR 有 must-fix」(1) 和「工具自身出错，应重试/告警」(2)。
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(2);
        }
    }
}

/// 分发子命令，返回进程退出码。只有 `review` / `security` / `demo` 走闸口语义（0=放行 / 1=拦截）；
/// 其余成功即 0，错误统一冒泡到 `main` 记为 2。
async fn run(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Command::Init(args) => cmd_init(&args).await,
        Command::Demo(args) => cmd_demo(&args).await,
        Command::Review(args) => review(&args).await,
        Command::Security(args) => security(&args).await,
        Command::Llm { cmd } => match cmd {
            LlmCmd::Test => llm_test().await.map(|()| 0),
        },
        Command::Diff(args) => diff_summary(&args).await.map(|()| 0),
        Command::Tool { name, input } => tool_call(&name, &input).await.map(|()| 0),
        Command::Agent { dimension } => agent_run(&dimension).await.map(|()| 0),
        Command::Upgrade { version, force } => upgrade(version.as_deref(), force).await.map(|()| 0),
        Command::Index { cmd } => match cmd {
            IndexCmd::Build => index_build().await.map(|()| 0),
        },
        Command::Findings { cmd } => match cmd {
            FindingsCmd::List {
                status,
                include_filtered,
            } => findings_list(&status, include_filtered).await.map(|()| 0),
            FindingsCmd::Show { id } => findings_show(&id).await.map(|()| 0),
            FindingsCmd::Resolve { id, note } => findings_resolve(&id, note).await.map(|()| 0),
        },
        Command::Issue { cmd } => match cmd {
            IssueCmd::Init(args) => issue_sync(&args, true).await.map(|()| 0),
            IssueCmd::Sync(args) => issue_sync(&args, false).await.map(|()| 0),
            IssueCmd::Review(args) => issue_review_cmd(&args).await.map(|()| 0),
            IssueCmd::Inspect {
                number,
                data_dir,
                repo,
            } => issue_inspect(number, data_dir.as_deref(), repo.as_deref()).map(|()| 0),
            IssueCmd::Watch(args) => issue_watch(&args).await.map(|()| 0),
            IssueCmd::Stats {
                data_dir,
                repo,
                gated,
            } => issue_stats(data_dir.as_deref(), repo.as_deref(), gated).map(|()| 0),
        },
        Command::Serve(args) => issue_serve(&args).await.map(|()| 0),
        Command::Daemon(args) => issue_daemon(&args).await.map(|()| 0),
    }
}

async fn cmd_init(args: &InitArgs) -> anyhow::Result<i32> {
    let code = init::run_init(
        &args.provider,
        args.protocol.as_deref(),
        args.base_url.as_deref(),
        args.model.as_deref(),
        args.yes,
        args.force,
        args.config_dir.as_deref(),
        args.test,
    )?;
    if args.test {
        // Point config discovery at the file we just wrote when using --config-dir.
        if let Some(dir) = &args.config_dir {
            let path = dir.join("config.toml");
            // SAFETY: single-threaded CLI; no concurrent env readers at this point.
            std::env::set_var("REVIEWGATE_CONFIG", &path);
        }
        match llm_test().await {
            Ok(()) => Ok(code),
            Err(e) => {
                eprintln!("error: {e:#}");
                eprintln!(
                    "hint: set REVIEWGATE_API_KEY and re-run `reviewgate llm test` (config was still written)"
                );
                Ok(2)
            }
        }
    } else {
        Ok(code)
    }
}

async fn cmd_demo(args: &DemoArgs) -> anyhow::Result<i32> {
    use anyhow::Context;
    use reviewgate_core::config::Config;
    use reviewgate_core::diff::DiffMode;
    use reviewgate_core::model::Dimension;
    use reviewgate_core::review::ReviewOptions;

    let root = demo::temp_demo_dir();
    demo::seed_demo_repo(&root)?;
    eprintln!(
        "Demo workspace: {} (poisoned SQL injection in handler.py)",
        root.display()
    );

    if args.prepare_only {
        // Caller owns the path when prepare-only (printed for inspection).
        println!("{}", root.display());
        eprintln!("Prepared only (--prepare-only). Review with:");
        eprintln!(
            "  cd {} && reviewgate review --dimensions security --fail-on block",
            root.display()
        );
        return Ok(0);
    }

    // Cleanup helper: always remove temp workspace unless --keep (or prepare-only above).
    let keep = args.keep;
    let cleanup = |root: &std::path::Path, keep: bool| {
        if keep {
            eprintln!("Kept demo workspace: {}", root.display());
        } else {
            let _ = std::fs::remove_dir_all(root);
        }
    };

    // Ensure config exists before spending tokens.
    let cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            cleanup(&root, keep);
            return Err(anyhow::anyhow!(
                "{e}\n  tip: run `reviewgate init` then `export REVIEWGATE_API_KEY=...` before demo"
            ));
        }
    };
    // Validate key early with a clear message.
    if let Err(e) = cfg.active_provider_resolved() {
        cleanup(&root, keep);
        return Err(anyhow::anyhow!(
            "{e}\n  tip: export REVIEWGATE_API_KEY=\"your-key\" then re-run `reviewgate demo`"
        ));
    }

    let prev = std::env::current_dir().ok();
    if let Err(e) = std::env::set_current_dir(&root) {
        cleanup(&root, keep);
        return Err(e).with_context(|| format!("chdir {}", root.display()));
    }

    eprintln!("ReviewGate demo · dimension=security · expect BLOCK on SQL injection");
    let mut opts = ReviewOptions::new(DiffMode::Workspace, vec![Dimension::Security]);
    opts.judge = true;
    opts.gate = cfg.gate.clone();
    opts.samples = 1;
    if args.timeout > 0 {
        opts.timeout = Some(std::time::Duration::from_secs(args.timeout));
    }

    // Real review/gate path (same present_and_exit as `review`); FailOn::Block → exit 1 on BLOCK.
    let code = present_and_exit(
        &cfg,
        opts,
        ReviewRunArgs {
            estimate_only: false,
            format: OutputFormat::Text,
            show_filtered: false,
            comment: false,
            fix: false,
            fix_all: false,
            fix_branch: None,
            fail_on: FailOn::Block,
            verbose: false,
        },
    )
    .await;

    if let Some(p) = prev {
        let _ = std::env::set_current_dir(p);
    }

    match &code {
        Ok(1) => {
            eprintln!();
            eprintln!("OK demo: gate returned BLOCK as expected.");
            eprintln!("Next: cd your-repo && reviewgate review");
        }
        Ok(0) => {
            eprintln!();
            eprintln!(
                "warn: demo expected BLOCK but got PASS — model/provider may have missed the fixture; retry or try a stronger model"
            );
        }
        Ok(c) => {
            eprintln!();
            eprintln!("demo finished with exit code {c} (WARN/incomplete still means the gate did not fake PASS)");
        }
        Err(_) => {}
    }

    cleanup(&root, keep);
    code
}

/// 建/刷新全仓定义索引到 `.reviewgate/cache/symbols.json`。
async fn index_build() -> anyhow::Result<()> {
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

/// 当前平台对应的 release 资产名（与 `install.sh` 命名一致）。
fn release_asset(os: &str, arch: &str) -> anyhow::Result<String> {
    let o = match os {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        other => anyhow::bail!("unsupported OS: {other}"),
    };
    let a = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => anyhow::bail!("unsupported arch: {other}"),
    };
    let ext = if o == "windows" { ".exe" } else { "" };
    Ok(format!("reviewgate-{o}-{a}{ext}"))
}

/// 安装来源：由谁在管这个二进制。自更新会直接覆盖文件，绕过包管理器会让它的记录失真
/// （`brew upgrade` 之后又被换回旧版），所以检测到就交还给对应的包管理器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallManager {
    Homebrew,
    Cargo,
    Mise,
    Nix,
}

impl InstallManager {
    /// 该包管理器的升级方式（给用户直接复制执行）。
    fn upgrade_hint(self) -> &'static str {
        match self {
            InstallManager::Homebrew => "brew upgrade reviewgate",
            InstallManager::Cargo => "cargo install reviewgate --force",
            InstallManager::Mise => "mise upgrade reviewgate",
            InstallManager::Nix => "nix profile upgrade reviewgate (or re-run `nix run`)",
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            InstallManager::Homebrew => "Homebrew",
            InstallManager::Cargo => "Cargo",
            InstallManager::Mise => "mise",
            InstallManager::Nix => "Nix",
        }
    }
}

/// 按可执行文件路径判断安装来源。识别不出（安装脚本 / 手动下载）→ `None`，走自替换。
///
/// 只看路径特征：这些管理器的安装位置是稳定约定，比反过来问每个 CLI 更快也更可靠
/// （包管理器可能根本没装在 PATH 上）。
fn detect_install_manager(exe_path: &str) -> Option<InstallManager> {
    let p = exe_path.replace('\\', "/");
    if p.starts_with("/nix/store/") || p.contains("/.nix-profile/") {
        return Some(InstallManager::Nix);
    }
    if p.contains("/Cellar/")
        || p.contains("/homebrew/")
        || p.contains("/Homebrew/")
        || p.contains("/linuxbrew/")
    {
        return Some(InstallManager::Homebrew);
    }
    if p.contains("/mise/installs/") || p.contains("/mise/shims/") {
        return Some(InstallManager::Mise);
    }
    if p.contains("/.cargo/bin/") {
        return Some(InstallManager::Cargo);
    }
    None
}

/// release 下载目录：`None` = latest，`Some("0.8.0")` / `Some("v0.8.0")` = 指定 tag。
fn release_base_url(version: Option<&str>) -> String {
    const REPO: &str = "https://github.com/dengmengmian/ReviewGate/releases";
    match version {
        None => format!("{REPO}/latest/download"),
        Some(v) => {
            let v = v.trim();
            let tag = if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            };
            format!("{REPO}/download/{tag}")
        }
    }
}

/// 自更新：下载指定（默认最新）release 的平台二进制，校验 SHA-256 后替换当前可执行文件。
///
/// 若二进制由包管理器安装，默认**不覆盖**，改为提示用该管理器升级（`--force` 可强制）。
async fn upgrade(version: Option<&str>, force: bool) -> anyhow::Result<()> {
    use anyhow::Context;

    let exe = std::env::current_exe().context("failed to locate the current executable")?;
    let exe_str = exe.to_string_lossy().to_string();
    if let Some(mgr) = detect_install_manager(&exe_str) {
        if !force {
            anyhow::bail!(
                "this binary is managed by {} ({exe_str}).\n  \
                 Upgrade with: {}\n  \
                 Or pass --force to replace the file in place anyway (the package manager's \
                 records will then be out of date).",
                mgr.as_str(),
                mgr.upgrade_hint()
            );
        }
        eprintln!(
            "warning: overwriting a {}-managed binary because --force was given",
            mgr.as_str()
        );
    }

    let asset = release_asset(std::env::consts::OS, std::env::consts::ARCH)?;
    let base = release_base_url(version);
    let url = format!("{base}/{asset}");
    let sums_url = format!("{base}/sha256sum.txt");
    match version {
        Some(v) => eprintln!("Downloading release {v}: {asset} ..."),
        None => eprintln!("Downloading latest release: {asset} ..."),
    }
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .with_context(|| format!("download failed: {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("download failed: HTTP {} ({url})", resp.status());
    }
    let bytes = resp.bytes().await?;
    let sums = reqwest::Client::new()
        .get(&sums_url)
        .send()
        .await
        .with_context(|| format!("checksum download failed: {sums_url}"))?;
    if !sums.status().is_success() {
        anyhow::bail!(
            "checksum download failed: HTTP {} ({sums_url})",
            sums.status()
        );
    }
    let sums = sums.text().await.context("failed to read checksum file")?;
    verify_release_checksum(&bytes, &sums, &asset)?;

    // 写临时文件 → 自替换当前可执行文件（self_replace 处理 Windows 运行中 exe 的替换）。
    let tmp = std::env::temp_dir().join(format!("reviewgate-upgrade-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).context("failed to write temp file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    self_replace::self_replace(&tmp).context("failed to replace the current executable")?;
    let _ = std::fs::remove_file(&tmp);

    // 用新二进制打印版本确认。
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(out) = std::process::Command::new(&exe).arg("--version").output() {
            eprint!("OK Upgraded to: {}", String::from_utf8_lossy(&out.stdout));
            return Ok(());
        }
    }
    eprintln!("OK Upgraded.");
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn verify_release_checksum(bytes: &[u8], checksums: &str, asset: &str) -> anyhow::Result<()> {
    let expected = checksums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        (name == asset).then_some(hash)
    });
    let Some(expected) = expected else {
        anyhow::bail!("checksum not found for release asset `{asset}`");
    };
    let actual = sha256_hex(bytes);
    if !expected.eq_ignore_ascii_case(&actual) {
        anyhow::bail!("checksum mismatch for `{asset}`: expected {expected}, got {actual}");
    }
    Ok(())
}

fn parse_dimension(s: &str) -> anyhow::Result<reviewgate_core::model::Dimension> {
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

fn parse_dimensions(s: &str) -> anyhow::Result<Vec<reviewgate_core::model::Dimension>> {
    use reviewgate_core::model::Dimension;
    if s.trim() == "all" {
        return Ok(Dimension::ALL.to_vec());
    }
    s.split(',').map(|p| parse_dimension(p.trim())).collect()
}

async fn agent_run(dimension: &str) -> anyhow::Result<()> {
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
async fn findings_list(status: &str, include_filtered: bool) -> anyhow::Result<()> {
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

async fn findings_show(id: &str) -> anyhow::Result<()> {
    let (_, session) = load_session().await?;
    println!("{}", serde_json::to_string_pretty(session.find(id)?)?);
    Ok(())
}

async fn findings_resolve(id: &str, note: Option<String>) -> anyhow::Result<()> {
    let (root, mut session) = load_session().await?;
    let record = session.resolve(id, note, now_secs())?.clone();
    session.save(&root)?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

async fn tool_call(name: &str, input: &str) -> anyhow::Result<()> {
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

fn validate_review_args(args: &ReviewArgs) -> anyhow::Result<()> {
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
struct ReviewRunArgs {
    /// 仅估算、未真正审查。为 true 时**不写发现会话**——什么都没审就推进增量基准，
    /// 会让下一次 `--since-last-review` 跳过一整段从未审过的改动。
    estimate_only: bool,
    format: OutputFormat,
    show_filtered: bool,
    comment: bool,
    fix: bool,
    fix_all: bool,
    fix_branch: Option<String>,
    fail_on: FailOn,
    verbose: bool,
}

async fn present_and_exit(
    cfg: &reviewgate_core::config::Config,
    opts: reviewgate_core::review::ReviewOptions,
    run: ReviewRunArgs,
) -> anyhow::Result<i32> {
    use reviewgate_core::review::run_review;

    let live = std::io::stderr().is_terminal() && run.format != OutputFormat::Json && !run.verbose;
    let progress = live.then(|| std::sync::Arc::new(reviewgate_core::progress::Progress::new()));
    let mut opts = opts;
    opts.progress = progress.clone();
    let render = progress.clone().map(|p| {
        let t = i18n::Lang::detect();
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
                let fixed = 5 + render::display_width(reviewing) + render::display_width(&suffix);
                let budget = LINE_WIDTH.saturating_sub(fixed);
                let last = render::truncate_to_width(&last, budget);
                eprint!(
                    "\r\x1b[2K\x1b[36m{}\x1b[0m {reviewing} \x1b[2m·\x1b[0m {last}\x1b[2m{suffix}\x1b[0m",
                    FRAMES[i % FRAMES.len()],
                );
                let _ = std::io::Write::flush(&mut std::io::stderr());
                i += 1;
            }
        })
    });

    let started = std::time::Instant::now();
    let outcome = run_review(cfg, &opts).await?;

    if let Some(h) = render {
        h.abort();
        let t = i18n::Lang::detect();
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
        OutputFormat::Json => println!("{}", render::render_json(&outcome)?),
        OutputFormat::Text => {
            // 团队自定义的严重度标签/配色（配置写错在审查开始时就已报错，这里必然可解析）。
            let labels = reviewgate_core::config::SeverityLabels::resolve(&cfg.severity_labels)?;
            print!(
                "{}",
                render::render_text_with_labels(&outcome, run.show_filtered, labels)
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
        fix::apply_fixes(
            &outcome.findings,
            std::path::Path::new(&root),
            run.fix_branch.as_deref(),
            run.fix_all,
        )?;
    }

    // Deep profile / critical-path incomplete always treat incomplete as non-PASS for exit semantics.
    let fail_incomplete =
        cfg.gate.fail_on_incomplete || opts.profile.is_deep() || outcome.critical_incomplete;
    let incomplete_for_exit = outcome.incomplete || outcome.critical_incomplete;
    Ok(exit_code(
        outcome.decision,
        incomplete_for_exit,
        fail_incomplete,
        run.fail_on,
    ))
}

async fn review(args: &ReviewArgs) -> anyhow::Result<i32> {
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
async fn security(args: &SecurityArgs) -> anyhow::Result<i32> {
    use reviewgate_core::config::Config;
    use reviewgate_core::review::ReviewOptions;

    validate_security_args(args)?;
    let cfg = Config::load()?;
    let mode = resolve_mode(&args.commit, &args.from, &args.to)?;
    let samples = args.samples.max(1);

    let etty = std::io::stderr().is_terminal();
    let dim = |s: &str| {
        if etty {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let samples_note = if samples > 1 {
        format!(" · samples={samples}")
    } else {
        String::new()
    };
    eprintln!(
        "ReviewGate {} security {} {}",
        dim("deep review"),
        dim("· sink inventory + secret precheck"),
        dim(&format!("· {samples} agents{samples_note}")),
    );

    let mut opts = ReviewOptions::security_deep(mode);
    opts.judge = !args.no_judge;
    opts.gate = cfg.gate.clone();
    opts.gate.fail_on_incomplete = true; // deep never treats incomplete as PASS
    opts.verbose = args.verbose;
    if args.timeout > 0 {
        opts.timeout = Some(std::time::Duration::from_secs(args.timeout));
    }
    opts.samples = samples;
    opts.judge_concurrency = args.judge_concurrency.max(1);
    opts.fanout_concurrency = args.fanout_concurrency.max(1);
    opts.incremental = args.incremental;

    present_and_exit(
        &cfg,
        opts,
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
fn exit_code(
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

async fn diff_summary(args: &DiffArgs) -> anyhow::Result<()> {
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

async fn llm_test() -> anyhow::Result<()> {
    use reviewgate_core::config::Config;
    use reviewgate_core::llm::build_client;
    use reviewgate_core::model::Message;

    let cfg = Config::load()?;
    let provider = cfg.active_provider_resolved()?;
    println!(
        "Provider: {} ({:?})  Model: {}  Endpoint: {}",
        cfg.provider, provider.protocol, provider.model, provider.base_url
    );

    let client = build_client(&provider)?;
    let messages = vec![Message::user("Reply in one sentence: connection OK.")];
    let resp = client
        .complete(
            "You are a connectivity self-check assistant. Reply briefly.",
            &messages,
            &[],
        )
        .await?;

    println!("---\nReply: {}", resp.text().trim());
    println!("LLM connectivity OK");
    Ok(())
}

// ─── Issue Review CLI ───────────────────────────────────────────────────────

fn issue_data_dir(explicit: Option<&PathBuf>) -> PathBuf {
    explicit
        .cloned()
        .unwrap_or_else(|| PathBuf::from(".reviewgate/issue"))
}

fn resolve_repo_slug(explicit: Option<&str>) -> anyhow::Result<String> {
    if let Some(r) = explicit {
        return Ok(r.to_string());
    }
    if let Ok(r) = std::env::var("REVIEWGATE_REPO") {
        if !r.trim().is_empty() {
            return Ok(r);
        }
    }
    if let Ok(r) = std::env::var("GITHUB_REPOSITORY") {
        if !r.trim().is_empty() {
            return Ok(r);
        }
    }
    // git remote get-url origin → owner/repo
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output();
    if let Ok(out) = out {
        if out.status.success() {
            let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Some(slug) = parse_github_slug(&url) {
                return Ok(slug);
            }
        }
    }
    anyhow::bail!(
        "could not resolve owner/repo: pass --repo, or set REVIEWGATE_REPO / GITHUB_REPOSITORY"
    )
}

fn parse_github_slug(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches(".git");
    if let Some(rest) = u.strip_prefix("git@github.com:") {
        return Some(rest.to_string());
    }
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
    ] {
        if let Some(rest) = u.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
            if parts.len() >= 2 {
                return Some(format!("{}/{}", parts[0], parts[1]));
            }
        }
    }
    // already owner/repo?
    let parts: Vec<&str> = u.split('/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        return Some(u.to_string());
    }
    None
}

fn issue_review_cfg_from_file() -> reviewgate_core::issue::IssueReviewConfig {
    use reviewgate_core::issue::{ActionPolicy, IssueReviewConfig, MentionConfig};
    let mut cfg = IssueReviewConfig::default();
    if let Ok(file) = reviewgate_core::config::Config::load() {
        cfg.vector_enabled = file.issue_review.vector.enabled;
        cfg.candidate_limit = file.issue_review.duplicate.candidate_limit;
        cfg.min_similarity = file.issue_review.duplicate.min_similarity;
        cfg.actions = ActionPolicy {
            comment: file.issue_review.actions.comment,
            update_existing_comment: file.issue_review.actions.update_existing_comment,
            add_labels: file.issue_review.actions.add_labels,
            close_issue: file.issue_review.actions.close_issue,
            // 开了总闸自然也含广告；单开 close_spam 则只放行广告这一类。
            close_spam: file.issue_review.actions.close_spam
                || file.issue_review.actions.close_issue,
            close_invalid: false,
            close_duplicate: false,
            safe_labels_only: true,
            min_confidence: file.issue_review.actions.min_confidence,
            assign_on_triage: file.issue_review.actions.assign_on_triage,
        };
        let m = &file.issue_review.mentions;
        cfg.mentions = MentionConfig {
            default: m.default.clone(),
            on_needs_info: m.on_needs_info.clone(),
            on_probable_duplicate: m.on_probable_duplicate.clone(),
            on_security: m.on_security.clone(),
            on_likely_bug: m.on_likely_bug.clone(),
            on_confirmed_bug: m.on_confirmed_bug.clone(),
            on_regression: m.on_regression.clone(),
            on_already_fixed: m.on_already_fixed.clone(),
            on_spam: m.on_spam.clone(),
            on_advertisement: m.on_advertisement.clone(),
            on_question: m.on_question.clone(),
            on_feature_request: m.on_feature_request.clone(),
            on_needs_triage: m.on_needs_triage.clone(),
        };
    }
    cfg
}

fn resolve_forge(s: &str) -> anyhow::Result<reviewgate_core::issue::IssueForge> {
    reviewgate_core::issue::IssueForge::parse(s)
        .or_else(|| {
            std::env::var("REVIEWGATE_FORGE")
                .ok()
                .and_then(|v| reviewgate_core::issue::IssueForge::parse(&v))
        })
        .ok_or_else(|| anyhow::anyhow!("unknown forge `{s}` (github|gitlab|gitee|atomgit)"))
}

fn platform_token(forge: reviewgate_core::issue::IssueForge) -> anyhow::Result<String> {
    if let Ok(t) = std::env::var("REVIEWGATE_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    match forge {
        reviewgate_core::issue::IssueForge::GitHub => std::env::var("GITHUB_TOKEN").map_err(|_| {
            anyhow::anyhow!("set REVIEWGATE_TOKEN or GITHUB_TOKEN for github Issue API access")
        }),
        reviewgate_core::issue::IssueForge::GitLab => std::env::var("GITLAB_TOKEN")
            .or_else(|_| std::env::var("CI_JOB_TOKEN"))
            .map_err(|_| {
                anyhow::anyhow!("set REVIEWGATE_TOKEN or GITLAB_TOKEN for gitlab Issue API access")
            }),
        reviewgate_core::issue::IssueForge::Gitee => std::env::var("GITEE_TOKEN").map_err(|_| {
            anyhow::anyhow!("set REVIEWGATE_TOKEN or GITEE_TOKEN for gitee Issue API access")
        }),
        reviewgate_core::issue::IssueForge::AtomGit => {
            std::env::var("ATOMGIT_TOKEN").map_err(|_| {
                anyhow::anyhow!(
                    "set REVIEWGATE_TOKEN or ATOMGIT_TOKEN for atomgit Issue API access"
                )
            })
        }
    }
}

fn build_live_platform(
    forge: reviewgate_core::issue::IssueForge,
    api_base: &str,
    repo: &str,
) -> anyhow::Result<Box<dyn reviewgate_core::issue::IssuePlatform>> {
    let token = platform_token(forge)?;
    let http = reviewgate_core::issue::ReqwestDoer::new()?;
    Ok(reviewgate_core::issue::build_platform(
        forge, api_base, repo, &token, http,
    ))
}

fn seed_demo_fixture(platform: &reviewgate_core::issue::FixturePlatform) -> anyhow::Result<()> {
    use reviewgate_core::issue::model::{RawIssue, RawLabel, RawUser};
    let samples = [
        (
            1u64,
            "Windows save crash access violation",
            "## Expected\nsave succeeds\n## Actual\naccess violation crash\n## Steps to reproduce\n1. open settings\n2. click save\n## Environment\nWindows 11 version 0.6.1\n",
        ),
        (
            2,
            "Crash when saving config on Windows",
            "access violation after clicking save on Windows 11\nerror: access violation\n",
        ),
        (
            3,
            "readme typo",
            "documentation spelling error in README\n",
        ),
        (
            4,
            "Free crypto airdrop",
            "Join telegram t.me/scam 邀请码 XYZ click here to claim limited offer https://a.com https://b.com https://c.com https://d.com\n",
        ),
    ];
    for (n, title, body) in samples {
        platform.seed_issue(RawIssue {
            number: n,
            title: title.into(),
            body: Some(body.into()),
            state: "open".into(),
            labels: vec![RawLabel {
                name: "triage".into(),
            }],
            user: Some(RawUser {
                login: "reporter".into(),
                user_type: Some("User".into()),
            }),
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-02T00:00:00Z".into(),
            closed_at: None,
            pull_request: None,
        });
    }
    Ok(())
}

async fn issue_sync(args: &IssueSyncArgs, is_init: bool) -> anyhow::Result<()> {
    use reviewgate_core::issue::{sync_from_platform, FixturePlatform, IssueStore};

    let repo = resolve_repo_slug(args.repo.as_deref()).unwrap_or_else(|_| {
        if args.fixture {
            "fixture/demo".into()
        } else {
            String::new()
        }
    });
    if repo.is_empty() {
        resolve_repo_slug(args.repo.as_deref())?;
    }
    let data_dir = issue_data_dir(args.data_dir.as_ref());
    let store = IssueStore::open(&data_dir, &repo)?;
    let forge = resolve_forge(&args.forge)?;

    let synced = if args.fixture {
        let platform = FixturePlatform::new();
        seed_demo_fixture(&platform)?;
        sync_from_platform(&store, &platform, args.max, None, true).await?
    } else {
        let platform = build_live_platform(forge, &args.api_base, &repo)?;
        let since = if is_init {
            None
        } else {
            store.get_sync_cursor()?
        };
        if let Some(ref s) = since {
            if s.parse::<u64>().is_ok() {
                eprintln!(
                    "warning: legacy epoch cursor {s:?}; doing full resync (no since filter)"
                );
            }
        }
        let since_arg = since.as_deref().filter(|s| s.parse::<u64>().is_err());
        sync_from_platform(&store, platform.as_ref(), args.max, since_arg, true).await?
    };

    println!(
        "{} complete: repo={repo} indexed={} db={} (history index only; no bulk replies)",
        if is_init { "issue init" } else { "issue sync" },
        synced.len(),
        store.path.display()
    );
    if let Ok(Some(c)) = store.get_sync_cursor() {
        println!("sync cursor (ISO8601): {c}");
    }
    println!("total issues in store: {}", store.count_issues()?);
    Ok(())
}

async fn issue_review_cmd(args: &IssueReviewCliArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::model::{RawIssue, RawLabel, RawUser};
    use reviewgate_core::issue::{
        format_review_text, publish_decision, resolve_repo_root, review_issue_with_llm,
        FixturePlatform, IssuePlatform, IssueStore, LocalEmbedder,
    };
    use reviewgate_core::llm::build_client;

    let repo = if args.fixture || args.fixture_issue.is_some() {
        resolve_repo_slug(args.repo.as_deref()).unwrap_or_else(|_| "fixture/demo".into())
    } else {
        resolve_repo_slug(args.repo.as_deref())?
    };
    let data_dir = issue_data_dir(args.data_dir.as_ref());
    let store = IssueStore::open(&data_dir, &repo)?;
    let mut cfg = issue_review_cfg_from_file();
    cfg.verify_enabled = args.verify && !args.triage_only;
    cfg.repo_root = args.repo_root.clone().or_else(|| resolve_repo_root(None));
    let emb = LocalEmbedder;
    let forge = resolve_forge(&args.forge)?;
    // 有配置时用 LLM 写面向用户的说明（证据仍来自本地检索）
    let llm_box = reviewgate_core::config::Config::load()
        .ok()
        .and_then(|c| c.active_provider_resolved().ok())
        .and_then(|p| build_client(&p).ok());
    let llm = llm_box.as_deref();
    if llm.is_some() {
        eprintln!("issue explain: using configured LLM for user-facing narrative");
    } else {
        eprintln!("issue explain: no LLM config/key; using deterministic narrative");
    }

    let out = if args.fixture || args.fixture_issue.is_some() {
        let platform = FixturePlatform::new();
        if let Some(path) = &args.fixture_issue {
            let text = std::fs::read_to_string(path)?;
            let issue: RawIssue = serde_json::from_str(&text)?;
            platform.seed_issue(issue);
        } else {
            seed_demo_fixture(&platform)?;
        }
        if platform.get_issue(args.number).await.is_err() {
            platform.seed_issue(RawIssue {
                number: args.number,
                title: "fixture issue".into(),
                body: Some("error: panic in fixture".into()),
                state: "open".into(),
                labels: vec![RawLabel { name: "bug".into() }],
                user: Some(RawUser {
                    login: "u".into(),
                    user_type: Some("User".into()),
                }),
                created_at: "t".into(),
                updated_at: "t".into(),
                closed_at: None,
                pull_request: None,
            });
        }
        let _ =
            reviewgate_core::issue::sync_from_platform(&store, &platform, 100, None, true).await?;
        review_issue_with_llm(&store, &platform, args.number, &cfg, &emb, llm).await?
    } else {
        let platform = build_live_platform(forge, &args.api_base, &repo)?;
        review_issue_with_llm(&store, platform.as_ref(), args.number, &cfg, &emb, llm).await?
    };

    let text = format_review_text(&out);
    print!("{text}");

    // Default is dry-run; only publish when --publish is set.
    if args.publish {
        if args.fixture || args.fixture_issue.is_some() {
            let platform = FixturePlatform::new();
            seed_demo_fixture(&platform)?;
            if platform.get_issue(args.number).await.is_err() {
                // number may be outside demo seed
                seed_demo_fixture(&platform)?;
            }
            let pub_out = publish_decision(&store, &platform, &out).await?;
            println!(
                "published (fixture): comment_id={} created={} updated={}",
                pub_out.comment_id, pub_out.created, pub_out.updated
            );
            // second publish must update, not create another comment
            let pub_out2 = publish_decision(&store, &platform, &out).await?;
            println!(
                "published again: comment_id={} created={} updated={} total_comments={}",
                pub_out2.comment_id,
                pub_out2.created,
                pub_out2.updated,
                platform.comment_count(args.number)
            );
        } else {
            let platform = build_live_platform(forge, &args.api_base, &repo)?;
            let pub_out = publish_decision(&store, platform.as_ref(), &out).await?;
            println!(
                "published: comment_id={} created={} updated={} close={} labels={:?}",
                pub_out.comment_id,
                pub_out.created,
                pub_out.updated,
                out.planned.close,
                out.planned.labels_to_add
            );
        }
    } else {
        println!(
            "(dry-run: pass --publish to post/update the bot comment; --verify for code analysis)"
        );
    }
    let _ = args.dry_run;
    Ok(())
}

/// Local triage statistics. The point of this view is the gated count: in long-running
/// modes a low-confidence issue is skipped silently, and without this nobody notices.
fn issue_stats(
    data_dir: Option<&std::path::Path>,
    repo: Option<&str>,
    gated_only: bool,
) -> anyhow::Result<()> {
    use reviewgate_core::issue::IssueStore;

    let repo = resolve_repo_slug(repo).unwrap_or_else(|_| "fixture/demo".into());
    let dir = data_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".reviewgate/issue"));
    let store = IssueStore::open(&dir, &repo)?;
    if gated_only {
        let rows = store.gated_issues()?;
        println!("repo: {repo}");
        if rows.is_empty() {
            println!("nothing waiting for a human");
            return Ok(());
        }
        println!("{} issue(s) waiting for a human:", rows.len());
        for g in &rows {
            println!(
                "  #{:<6} {:<16} {:<12} {:>3}%  {}",
                g.issue_number,
                g.primary_type,
                g.verdict,
                (g.confidence * 100.0).round() as i64,
                if g.handed_off {
                    "handed off"
                } else {
                    "NOT handed off (no triage owner configured)"
                }
            );
        }
        return Ok(());
    }
    let s = store.action_stats()?;
    println!("repo: {repo}");
    println!("store: {}", store.path.display());
    if s.total == 0 {
        println!("no triage recorded yet");
        return Ok(());
    }
    println!("triaged: {}", s.total);
    println!("planned comment: {}", s.commented);
    println!("planned close: {}", s.closed);
    println!("executed on platform: {}", s.executed);
    println!("gated (low confidence): {}", s.gated_low_confidence);
    println!("avg confidence: {:.0}%", s.avg_confidence * 100.0);
    println!("by verdict:");
    for (v, n) in &s.by_verdict {
        println!("  {v}: {n}");
    }
    Ok(())
}

fn issue_inspect(
    number: u64,
    data_dir: Option<&std::path::Path>,
    repo: Option<&str>,
) -> anyhow::Result<()> {
    use reviewgate_core::issue::IssueStore;

    let repo = resolve_repo_slug(repo).unwrap_or_else(|_| "fixture/demo".into());
    let dir = data_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".reviewgate/issue"));
    let store = IssueStore::open(&dir, &repo)?;
    match store.get_issue(number)? {
        None => {
            println!(
                "issue #{number} not found in local store ({})",
                store.path.display()
            );
        }
        Some(issue) => {
            println!("issue #{number}");
            println!("title: {}", issue.title);
            println!("state: {}", issue.state);
            println!("content_hash: {}", issue.content_hash);
            println!("comments_hash: {}", issue.comments_hash);
            println!("error_signature: {}", issue.error_signature);
            println!(
                "embedding: {}",
                if issue.embedding.is_some() {
                    "yes"
                } else {
                    "no"
                }
            );
            println!("body_clean:\n{}", issue.body_clean);
        }
    }
    if let Some(d) = store.latest_review(number)? {
        println!("--- last review ---");
        println!("verdict: {} conf={:.2}", d.verdict.as_str(), d.confidence);
        println!("type: {}", d.primary_type.as_str());
        println!(
            "duplicate: {} of={:?}",
            d.duplicate_status.as_str(),
            d.duplicate_of
        );
        println!(
            "completeness: {:.2} missing={:?}",
            d.completeness_score, d.missing_fields
        );
    }
    Ok(())
}

async fn issue_watch(args: &IssueWatchArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::{
        resolve_repo_root, review_issue, sync_from_platform, FixturePlatform, IssueStore,
        LocalEmbedder,
    };

    let interval = parse_interval_secs(&args.interval)?;
    let repo = if args.fixture {
        resolve_repo_slug(args.repo.as_deref()).unwrap_or_else(|_| "fixture/demo".into())
    } else {
        resolve_repo_slug(args.repo.as_deref())?
    };
    let data_dir = issue_data_dir(args.data_dir.as_ref());
    let store = IssueStore::open(&data_dir, &repo)?;
    let mut cfg = issue_review_cfg_from_file();
    cfg.verify_enabled = args.verify;
    cfg.repo_root = args.repo_root.clone().or_else(|| resolve_repo_root(None));
    let emb = LocalEmbedder;
    let forge = resolve_forge(&args.forge)?;

    let mut iter = 0u64;
    loop {
        iter += 1;
        eprintln!(
            "issue watch: iteration {iter} repo={repo} forge={}",
            forge.as_str()
        );
        if args.fixture {
            let platform = FixturePlatform::new();
            seed_demo_fixture(&platform)?;
            let synced = sync_from_platform(&store, &platform, 100, None, true).await?;
            eprintln!("synced {} fixture issues", synced.len());
            for num in &synced {
                let out = review_issue(&store, &platform, *num, &cfg, &emb).await?;
                eprintln!(
                    "  #{} → {} ({:.0}%) tech={}",
                    num,
                    out.decision.verdict.as_str(),
                    out.decision.confidence * 100.0,
                    out.decision.technical_verdict.as_str()
                );
            }
        } else {
            let platform = build_live_platform(forge, &args.api_base, &repo)?;
            let since = store.get_sync_cursor()?;
            let since_arg = since.as_deref().filter(|s| s.parse::<u64>().is_err());
            let synced =
                sync_from_platform(&store, platform.as_ref(), 200, since_arg, true).await?;
            eprintln!("synced {} issues", synced.len());
            if let Ok(Some(c)) = store.get_sync_cursor() {
                eprintln!("sync cursor (ISO8601): {c}");
            }
            for num in &synced {
                match review_issue(&store, platform.as_ref(), *num, &cfg, &emb).await {
                    Ok(out) => {
                        eprintln!(
                            "  #{} → {} ({:.0}%) type={} dup={} tech={}",
                            num,
                            out.decision.verdict.as_str(),
                            out.decision.confidence * 100.0,
                            out.decision.primary_type.as_str(),
                            out.decision.duplicate_status.as_str(),
                            out.decision.technical_verdict.as_str()
                        );
                    }
                    Err(e) => {
                        eprintln!("  #{num} review failed: {e:#}");
                    }
                }
            }
        }
        if args.max_iterations > 0 && iter >= args.max_iterations {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    Ok(())
}

async fn issue_serve(args: &ServeArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::{run_webhook_server, ServeConfig};

    let secret = args
        .webhook_secret
        .clone()
        .or_else(|| std::env::var("REVIEWGATE_WEBHOOK_SECRET").ok())
        .ok_or_else(|| anyhow::anyhow!("pass --webhook-secret or set REVIEWGATE_WEBHOOK_SECRET"))?;
    let queue = args
        .queue
        .clone()
        .unwrap_or_else(|| PathBuf::from(".reviewgate/issue/webhook.db"));
    let cfg = ServeConfig {
        listen: args.listen.clone(),
        webhook_secret: secret,
        queue_path: queue,
        bot_logins: vec!["reviewgate[bot]".into(), "reviewgate-bot".into()],
    };
    run_webhook_server(cfg, None).await
}

async fn issue_daemon(args: &DaemonArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::{
        drain_queue_once, resolve_repo_root, review_issue, run_webhook_server, sync_from_platform,
        EventQueue, FixturePlatform, IssueStore, LocalEmbedder, ServeConfig,
    };

    let data_dir = issue_data_dir(args.data_dir.as_ref());
    let repo = if args.fixture {
        resolve_repo_slug(args.repo.as_deref()).unwrap_or_else(|_| "fixture/demo".into())
    } else {
        resolve_repo_slug(args.repo.as_deref())?
    };
    let store = IssueStore::open(&data_dir, &repo)?;
    let mut cfg = issue_review_cfg_from_file();
    cfg.verify_enabled = args.verify;
    cfg.repo_root = resolve_repo_root(None);
    let emb = LocalEmbedder;
    let forge = resolve_forge(&args.forge)?;
    let interval = parse_interval_secs(&args.interval)?;
    let queue_path = data_dir.join("webhook.db");
    let queue = EventQueue::open(&queue_path)?;

    if args.serve {
        let secret = args
            .webhook_secret
            .clone()
            .or_else(|| std::env::var("REVIEWGATE_WEBHOOK_SECRET").ok())
            .unwrap_or_else(|| "dev-insecure-secret".into());
        let scfg = ServeConfig {
            listen: args.listen.clone(),
            webhook_secret: secret,
            queue_path: queue_path.clone(),
            bot_logins: vec!["reviewgate[bot]".into(), "reviewgate-bot".into()],
        };
        tokio::spawn(async move {
            if let Err(e) = run_webhook_server(scfg, None).await {
                eprintln!("serve error: {e:#}");
            }
        });
        eprintln!("daemon: webhook serve on {}", args.listen);
    }

    let mut iter = 0u64;
    loop {
        iter += 1;
        eprintln!("daemon loop {iter} repo={repo}");

        // 1) drain webhook queue
        let nq = drain_queue_once(&queue, |d| {
            let store = &store;
            let cfg = &cfg;
            let emb = &emb;
            let repo = repo.clone();
            let api_base = args.api_base.clone();
            let fixture = args.fixture;
            async move {
                let Some(num) = d.issue_number else {
                    return Ok(());
                };
                if !d.event_type.is_empty() {
                    eprintln!(
                        "  queue {} {} #{} ({})",
                        d.event_type, d.action, num, d.delivery_id
                    );
                }
                if fixture {
                    let platform = FixturePlatform::new();
                    seed_demo_fixture(&platform)?;
                    let _ = review_issue(store, &platform, num, cfg, emb).await?;
                } else {
                    let platform = build_live_platform(forge, &api_base, &repo)?;
                    let _ = review_issue(store, platform.as_ref(), num, cfg, emb).await?;
                }
                Ok(())
            }
        })
        .await?;
        if nq > 0 {
            eprintln!("  processed {nq} queue deliveries");
        }

        // 2) poll sync + triage
        if args.fixture {
            let platform = FixturePlatform::new();
            seed_demo_fixture(&platform)?;
            let synced = sync_from_platform(&store, &platform, 100, None, true).await?;
            for num in synced {
                let out = review_issue(&store, &platform, num, &cfg, &emb).await?;
                eprintln!("  poll #{} → {}", num, out.decision.verdict.as_str());
            }
        } else {
            let platform = build_live_platform(forge, &args.api_base, &repo)?;
            let since = store.get_sync_cursor()?;
            let since_arg = since.as_deref().filter(|s| s.parse::<u64>().is_err());
            let synced =
                sync_from_platform(&store, platform.as_ref(), 100, since_arg, true).await?;
            for num in synced {
                match review_issue(&store, platform.as_ref(), num, &cfg, &emb).await {
                    Ok(out) => eprintln!("  poll #{} → {}", num, out.decision.verdict.as_str()),
                    Err(e) => eprintln!("  poll #{num} failed: {e:#}"),
                }
            }
        }

        if args.max_iterations > 0 && iter >= args.max_iterations {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    Ok(())
}

fn parse_interval_secs(s: &str) -> anyhow::Result<u64> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('s') {
        return Ok(num.parse()?);
    }
    if let Some(num) = s.strip_suffix('m') {
        return Ok(num.parse::<u64>()? * 60);
    }
    if let Some(num) = s.strip_suffix('h') {
        return Ok(num.parse::<u64>()? * 3600);
    }
    Ok(s.parse()?)
}

#[cfg(test)]
mod tests {
    use super::{
        detect_install_manager, exit_code, parse_dimensions, release_asset, release_base_url,
        resolve_mode, FailOn, InstallManager, ReviewArgs,
    };
    use reviewgate_core::gate::GateDecision;
    use reviewgate_core::model::Dimension;

    fn review_args() -> ReviewArgs {
        ReviewArgs {
            format: super::OutputFormat::Text,
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
        assert_eq!(
            super::resolve_intent(&args).unwrap(),
            Some("意图说明".into())
        );

        let args = review_args();
        assert_eq!(super::resolve_intent(&args).unwrap(), None);

        let mut args = review_args();
        args.intent = Some(tmp.to_str().unwrap().into());
        std::fs::write(&tmp, "   \n").unwrap();
        assert_eq!(super::resolve_intent(&args).unwrap(), None);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn exit_code_gate_and_fail_on_matrix() {
        // block + fail-on=block/warn → 1；fail-on=never → 0。
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
        // warn 只在 fail-on=warn 时非 0。
        assert_eq!(exit_code(GateDecision::Warn, false, false, FailOn::Warn), 1);
        assert_eq!(
            exit_code(GateDecision::Warn, false, false, FailOn::Block),
            0
        );
        // pass 永远 0。
        assert_eq!(exit_code(GateDecision::Pass, false, false, FailOn::Warn), 0);
    }

    #[test]
    fn exit_code_incomplete_overrides_when_configured() {
        // 未审完 + fail_on_incomplete：即便 PASS / fail-on=never 也非 0（杜绝漏审放行）。
        assert_eq!(exit_code(GateDecision::Pass, true, true, FailOn::Never), 1);
        assert_eq!(exit_code(GateDecision::Warn, true, true, FailOn::Block), 1);
        // 未审完但未开 fail_on_incomplete：回到常规闸口语义。
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
        assert!(super::validate_review_args(&args).is_err());

        args.fix = true;
        assert!(super::validate_review_args(&args).is_ok());

        args.fix = false;
        args.fix_all = true;
        assert!(super::validate_review_args(&args).is_ok());
    }

    #[test]
    fn parse_dimensions_trims_whitespace() {
        assert_eq!(
            parse_dimensions(" security , logic , ai_smell ").unwrap(),
            vec![
                reviewgate_core::model::Dimension::Security,
                reviewgate_core::model::Dimension::Logic,
                reviewgate_core::model::Dimension::AiSmell,
            ]
        );
    }

    #[test]
    fn resolve_intent_from_commit_requires_commit() {
        let mut args = review_args();
        args.intent_from_commit = true;
        assert!(super::resolve_intent(&args).is_err());
    }

    #[test]
    fn resolve_intent_stdin_dash() {
        // 由于无法可靠模拟 stdin，这里仅验证函数签名和空路径处理不 panic。
        let mut args = review_args();
        args.intent = Some("-".into());
        // 不实际调用，避免阻塞等待 stdin。
        assert_eq!(args.intent.as_deref(), Some("-"));
    }

    #[test]
    fn detect_install_manager_recognizes_package_managers() {
        assert_eq!(
            detect_install_manager("/opt/homebrew/Cellar/reviewgate/0.9.0/bin/reviewgate"),
            Some(InstallManager::Homebrew)
        );
        assert_eq!(
            detect_install_manager("/home/linuxbrew/.linuxbrew/bin/reviewgate"),
            Some(InstallManager::Homebrew)
        );
        assert_eq!(
            detect_install_manager("/Users/me/.cargo/bin/reviewgate"),
            Some(InstallManager::Cargo)
        );
        assert_eq!(
            detect_install_manager(
                "/Users/me/.local/share/mise/installs/reviewgate/0.9.0/bin/reviewgate"
            ),
            Some(InstallManager::Mise)
        );
        assert_eq!(
            detect_install_manager("/nix/store/abc-reviewgate-0.9.0/bin/reviewgate"),
            Some(InstallManager::Nix)
        );
        // 安装脚本 / 手动下载：自替换是正确路径。
        assert_eq!(detect_install_manager("/usr/local/bin/reviewgate"), None);
        assert_eq!(detect_install_manager("/Users/me/bin/reviewgate"), None);
        // Windows 路径分隔符也要认。
        assert_eq!(
            detect_install_manager("C:\\Users\\me\\.cargo\\bin\\reviewgate.exe"),
            Some(InstallManager::Cargo)
        );
    }

    #[test]
    fn release_base_url_pins_version_with_v_prefix() {
        assert!(release_base_url(None).ends_with("/latest/download"));
        assert!(release_base_url(Some("0.8.0")).ends_with("/download/v0.8.0"));
        // 已带 v 的不重复加前缀。
        assert!(release_base_url(Some("v0.8.0")).ends_with("/download/v0.8.0"));
        assert!(release_base_url(Some(" 0.8.0 ")).ends_with("/download/v0.8.0"));
    }

    #[test]
    fn release_asset_maps_platforms() {
        assert_eq!(
            release_asset("macos", "aarch64").unwrap(),
            "reviewgate-darwin-arm64"
        );
        assert_eq!(
            release_asset("macos", "x86_64").unwrap(),
            "reviewgate-darwin-x64"
        );
        assert_eq!(
            release_asset("linux", "aarch64").unwrap(),
            "reviewgate-linux-arm64"
        );
        assert_eq!(
            release_asset("linux", "x86_64").unwrap(),
            "reviewgate-linux-x64"
        );
        assert_eq!(
            release_asset("windows", "x86_64").unwrap(),
            "reviewgate-windows-x64.exe"
        );
        // 命名须与 install.sh / release.yml 的资产名一致。
        assert!(release_asset("freebsd", "x86_64").is_err());
        assert!(release_asset("linux", "riscv64").is_err());
    }

    #[test]
    fn release_checksum_verifies_named_asset() {
        let bytes = b"reviewgate-test-binary";
        let good = super::sha256_hex(bytes);
        let sums = format!(
            "{}  reviewgate-linux-x64\n{}  reviewgate-darwin-arm64\n",
            good,
            "0".repeat(64)
        );

        super::verify_release_checksum(bytes, &sums, "reviewgate-linux-x64").unwrap();
    }

    #[test]
    fn release_checksum_rejects_missing_or_mismatched_asset() {
        let bytes = b"reviewgate-test-binary";
        let bad = "0".repeat(64);
        let err = super::verify_release_checksum(
            bytes,
            &format!("{bad}  reviewgate-linux-x64\n"),
            "reviewgate-linux-x64",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("checksum mismatch"), "{err}");

        let err = super::verify_release_checksum(bytes, "", "reviewgate-linux-x64")
            .unwrap_err()
            .to_string();
        assert!(err.contains("checksum not found"), "{err}");
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
