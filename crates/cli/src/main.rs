//! ReviewGate CLI —— 主形态。

mod fix;
mod render;

use clap::{Parser, Subcommand};
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
        /// Dimension: security | perf | logic | style | ai_smell
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

#[derive(Parser)]
struct ReviewArgs {
    /// Output format: text | json
    #[arg(long, default_value = "text")]
    format: String,
    /// Review dimensions: all, or a comma-separated list of security,perf,logic,style,ai_smell
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
    /// Which verdict triggers a non-zero exit code: block | warn | never
    #[arg(long, default_value = "block")]
    fail_on: String,
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
    /// After per-finding y/N confirmation, apply suggestion_code to working-tree files (not applied when non-interactive).
    #[arg(long)]
    fix: bool,
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
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Review(args) => {
            let code = review(&args).await?;
            std::process::exit(code);
        }
        Command::Llm { cmd } => match cmd {
            LlmCmd::Test => llm_test().await?,
        },
        Command::Diff(args) => diff_summary(&args).await?,
        Command::Tool { name, input } => tool_call(&name, &input).await?,
        Command::Agent { dimension } => agent_run(&dimension).await?,
        Command::Upgrade => upgrade().await?,
    }
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
        eprintln!("No changes detected.");
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
    use reviewgate_core::gate::GateDecision;
    use reviewgate_core::review::{run_review, ReviewOptions};

    let dims = parse_dimensions(&args.dimensions)?;
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
    eprintln!(
        "ReviewGate reviewing ({} base dimensions: {}{}; samples={}; {} agents)...",
        dims.len(),
        names.join(", "),
        if auto_business {
            "; +business (auto)"
        } else {
            ""
        },
        samples,
        agents
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
    opts.exec_verify = args.exec_verify;
    opts.intent = resolve_intent(args)?;
    if opts.intent.is_some() {
        eprintln!("  + Intent review: intent loaded; running the implementation-vs-intent pass.");
    }

    // 实时进度：仅在终端、非 JSON、非 --verbose 时开。单行就地刷新，结束清行并给紧凑摘要；
    // JSON/管道/CI/verbose 下不渲染（避免污染输出/与详细日志打架）。
    let live = std::io::stderr().is_terminal() && args.format != "json" && !args.verbose;
    let progress = live.then(|| std::sync::Arc::new(reviewgate_core::progress::Progress::new()));
    opts.progress = progress.clone();
    let render = progress.clone().map(|p| {
        tokio::spawn(async move {
            const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let start = std::time::Instant::now();
            let mut i = 0usize;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                let (n, last) = p.snapshot();
                let last: String = last.chars().take(60).collect();
                let s = start.elapsed().as_secs();
                eprint!(
                    "\r\x1b[2K{} Reviewing - {n} tool calls - {last} - {}:{:02}",
                    FRAMES[i % FRAMES.len()],
                    s / 60,
                    s % 60
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
        let (n, _) = progress.as_ref().unwrap().snapshot();
        let s = started.elapsed().as_secs();
        // 清掉进度行，留一行紧凑完成摘要（细节收起）。
        eprint!("\r\x1b[2K");
        eprintln!(
            "OK Review complete - {n} tool calls - {}:{:02}",
            s / 60,
            s % 60
        );
    }

    match args.format.as_str() {
        "json" => println!("{}", render::render_json(&outcome)?),
        _ => print!("{}", render::render_text(&outcome, args.show_filtered)),
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

    // 可选：逐条确认后把 suggestion_code 应用到工作区文件。
    if args.fix {
        let root = reviewgate_core::diff::git::repo_root().await?;
        fix::apply_fixes(&outcome.findings, std::path::Path::new(&root))?;
    }

    // 退出码语义（供 CI 闸口）。
    // 未审完 + fail_on_incomplete：无论 --fail-on 取值，一律非 0——杜绝"漏审却放行"。
    if outcome.incomplete && cfg.gate.fail_on_incomplete {
        return Ok(1);
    }
    let code = match (outcome.decision, args.fail_on.as_str()) {
        (GateDecision::Block, "block") | (GateDecision::Block, "warn") => 1,
        (GateDecision::Warn, "warn") => 1,
        _ => 0,
    };
    Ok(code)
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
    println!(
        "Stop reason: {:?}  Usage: in={} out={}",
        resp.stop_reason, resp.usage.input_tokens, resp.usage.output_tokens
    );
    println!("LLM connectivity OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::release_asset;

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
