//! ReviewGate CLI —— 主形态。

mod fix;
mod i18n;
mod render;

use clap::{Parser, Subcommand, ValueEnum};
use std::io::IsTerminal;

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
    /// Review the current git diff
    Review(ReviewArgs),
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
    /// Self-update to the latest release (download the platform binary and replace the current executable)
    Upgrade,
}

#[derive(Subcommand)]
enum LlmCmd {
    /// Send one minimal request to the default provider to verify connectivity
    Test,
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

/// 分发子命令，返回进程退出码。只有 `review` 走闸口语义（0=放行 / 1=拦截）；
/// 其余成功即 0，错误统一冒泡到 `main` 记为 2。
async fn run(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Command::Review(args) => review(&args).await,
        Command::Llm { cmd } => match cmd {
            LlmCmd::Test => llm_test().await.map(|()| 0),
        },
        Command::Diff(args) => diff_summary(&args).await.map(|()| 0),
        Command::Tool { name, input } => tool_call(&name, &input).await.map(|()| 0),
        Command::Agent { dimension } => agent_run(&dimension).await.map(|()| 0),
        Command::Upgrade => upgrade().await.map(|()| 0),
    }
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

/// 自更新：下载最新 release 对应平台二进制，替换当前可执行文件。
async fn upgrade() -> anyhow::Result<()> {
    use anyhow::Context;
    let asset = release_asset(std::env::consts::OS, std::env::consts::ARCH)?;
    let url =
        format!("https://github.com/dengmengmian/ReviewGate/releases/latest/download/{asset}");
    eprintln!("Downloading latest release: {asset} ...");
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .with_context(|| format!("download failed: {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("download failed: HTTP {} ({url})", resp.status());
    }
    let bytes = resp.bytes().await?;

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
    let mut findings = run_agent(&*client, &reg, &ctx, &agent_cfg, user_prompt).await?;

    // M1.9 行号重定位。
    reviewgate_core::relocate::relocate_all(&mut findings, std::path::Path::new(&root), &None, &d)
        .await;

    println!("{}", serde_json::to_string_pretty(&findings)?);
    eprintln!("{} findings.", findings.len());
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

async fn review(args: &ReviewArgs) -> anyhow::Result<i32> {
    use reviewgate_core::config::Config;
    use reviewgate_core::review::{run_review, ReviewOptions};

    let dims = parse_dimensions(&args.dimensions)?;
    if args.fix_branch.is_some() && !(args.fix || args.fix_all) {
        anyhow::bail!("--fix-branch only applies with --fix or --fix-all");
    }
    let cfg = Config::load()?;
    let names: Vec<&str> = dims.iter().map(|d| d.as_str()).collect();
    let auto_business = (!cfg.business.rules.is_empty()
        || cfg.business.rules_dir.is_some()
        || cfg.business.skills_dir.is_some())
        && !dims.contains(&reviewgate_core::model::Dimension::Business);
    let effective_dims = dims.len() + usize::from(auto_business);
    let samples = args.samples.max(1);
    let agents = effective_dims * samples;

    let mode = resolve_mode(&args.commit, &args.from, &args.to)?;
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
        "ReviewGate {} {}{} {}",
        dim("reviewing"),
        names.join(", "),
        business,
        dim(&format!("· {agents} agents{samples_note}")),
    );

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
    opts.intent = resolve_intent(args)?;
    if opts.intent.is_some() {
        eprintln!("  + Intent review: intent loaded; running the implementation-vs-intent pass.");
    }

    // 实时进度：仅在终端、非 JSON、非 --verbose 时开。单行就地刷新，结束清行并给紧凑摘要；
    // JSON/管道/CI/verbose 下不渲染（避免污染输出/与详细日志打架）。
    let live =
        std::io::stderr().is_terminal() && args.format != OutputFormat::Json && !args.verbose;
    let progress = live.then(|| std::sync::Arc::new(reviewgate_core::progress::Progress::new()));
    opts.progress = progress.clone();
    let render = progress.clone().map(|p| {
        let t = i18n::Lang::detect();
        tokio::spawn(async move {
            const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            // 整行可见宽度上限，与文本渲染保持一致：超出会被终端折行，导致 \r\x1b[2K
            // 只清当前物理行、残留前面的折行 → 刷屏。把「整行」（含前后缀）压进预算即可。
            // 宽度按显示列算（CJK 记 2 列），否则中文文案会撑破预算。
            const LINE_WIDTH: usize = 60;
            let reviewing = t.reviewing();
            let start = std::time::Instant::now();
            let mut i = 0usize;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                let (n, last) = p.snapshot();
                let s = start.elapsed().as_secs();
                // 可见骨架：`⠋ {reviewing} · ` + last + ` · {n} calls · M:SS`。
                // 先给前后缀留位，剩下的预算分给 last，保证整行不超过 LINE_WIDTH。
                let suffix = format!(" · {} · {}:{:02}", t.calls(n), s / 60, s % 60);
                // 1(spinner)+1(空格)+reviewing+1(空格)+2("· ") 为前缀可见宽。
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
    let outcome = run_review(&cfg, &opts).await?;

    if let Some(h) = render {
        h.abort();
        let t = i18n::Lang::detect();
        let (n, _) = progress.as_ref().unwrap().snapshot();
        let s = started.elapsed().as_secs();
        // 清掉进度行，留一行紧凑完成摘要（细节收起）。
        eprint!("\r\x1b[2K");
        eprintln!(
            "\x1b[32m✓\x1b[0m {} \x1b[2m· {} · {}:{:02}\x1b[0m",
            t.review_complete(),
            t.tool_calls(n),
            s / 60,
            s % 60
        );
    }

    match args.format {
        OutputFormat::Json => println!("{}", render::render_json(&outcome)?),
        OutputFormat::Text => print!("{}", render::render_text(&outcome, args.show_filtered)),
    }

    // 可选：在 GitHub PR 上发摘要评论 + 行内 suggestion（作者一键应用，人把关）。
    if args.comment {
        if let Err(e) = reviewgate_core::github::post_summary(&outcome).await {
            eprintln!("failed to post summary comment: {e}");
        }
        if let Err(e) = reviewgate_core::github::post_inline_suggestions(&outcome).await {
            eprintln!("failed to post inline comments: {e}");
        }
    }

    // 可选：把 suggestion_code 应用到工作区文件（--fix 逐条确认；--fix-all 全部应用）。
    if args.fix || args.fix_all {
        let root = reviewgate_core::diff::git::repo_root().await?;
        fix::apply_fixes(
            &outcome.findings,
            std::path::Path::new(&root),
            args.fix_branch.as_deref(),
            args.fix_all,
        )?;
    }

    Ok(exit_code(
        outcome.decision,
        outcome.incomplete,
        cfg.gate.fail_on_incomplete,
        args.fail_on,
    ))
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

#[cfg(test)]
mod tests {
    use super::{exit_code, parse_dimensions, release_asset, FailOn};
    use reviewgate_core::gate::GateDecision;
    use reviewgate_core::model::Dimension;

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
        assert!(parse_dimensions("security,bogus").is_err());
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
}
