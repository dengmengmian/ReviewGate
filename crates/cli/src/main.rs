//! ReviewGate CLI —— 主形态。
//!
//! 子命令按产品拆开：[`review_cmd`] 是审查闸口，[`issue_cmd`] 是 Issue 分诊。
//! 本文件只做参数总表、分发，以及 init / demo / llm / upgrade。

mod demo;
mod fix;
mod i18n;
mod init;
mod issue_cmd;
mod render;
mod review_cmd;

use clap::{Parser, Subcommand};
use issue_cmd::{
    issue_daemon, issue_inspect, issue_review_cmd, issue_serve, issue_stats, issue_sync,
    issue_watch, require_issue_review_enabled, DaemonArgs, IssueCmd, ServeArgs,
};
use review_cmd::{
    agent_run, diff_summary, findings_list, findings_resolve, findings_show, index_build,
    present_and_exit, review, security, tool_call, DiffArgs, FailOn, FindingsCmd, IndexCmd, Line,
    OutputFormat, ReviewArgs, ReviewRunArgs, SecurityArgs,
};
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
    /// Security deep review: sink-driven security-only pass with saturating recall and secret precheck
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
            IssueCmd::Init(args) => {
                require_issue_review_enabled()?;
                issue_sync(&args, true).await.map(|()| 0)
            }
            IssueCmd::Sync(args) => {
                require_issue_review_enabled()?;
                issue_sync(&args, false).await.map(|()| 0)
            }
            IssueCmd::Review(args) => {
                require_issue_review_enabled()?;
                issue_review_cmd(&args).await.map(|()| 0)
            }
            IssueCmd::Inspect {
                number,
                data_dir,
                repo,
            } => issue_inspect(number, data_dir.as_deref(), repo.as_deref()).map(|()| 0),
            IssueCmd::Watch(args) => {
                require_issue_review_enabled()?;
                issue_watch(&args).await.map(|()| 0)
            }
            IssueCmd::Stats {
                data_dir,
                repo,
                gated,
            } => issue_stats(data_dir.as_deref(), repo.as_deref(), gated).map(|()| 0),
        },
        Command::Serve(args) => {
            require_issue_review_enabled()?;
            issue_serve(&args).await.map(|()| 0)
        }
        Command::Daemon(args) => {
            require_issue_review_enabled()?;
            issue_daemon(&args).await.map(|()| 0)
        }
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
        Line::Review,
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
    use super::{
        detect_install_manager, release_asset, release_base_url, verify_release_checksum,
        InstallManager,
    };

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
        assert_eq!(detect_install_manager("/usr/local/bin/reviewgate"), None);
        assert_eq!(detect_install_manager("/Users/me/bin/reviewgate"), None);
        assert_eq!(
            detect_install_manager("C:\\Users\\me\\.cargo\\bin\\reviewgate.exe"),
            Some(InstallManager::Cargo)
        );
    }

    #[test]
    fn release_base_url_pins_version_with_v_prefix() {
        assert!(release_base_url(None).ends_with("/latest/download"));
        assert!(release_base_url(Some("0.8.0")).ends_with("/download/v0.8.0"));
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
        verify_release_checksum(bytes, &sums, "reviewgate-linux-x64").unwrap();
    }

    #[test]
    fn release_checksum_rejects_missing_or_mismatched_asset() {
        let bytes = b"reviewgate-test-binary";
        let bad = "0".repeat(64);
        let err = verify_release_checksum(
            bytes,
            &format!("{bad}  reviewgate-linux-x64\n"),
            "reviewgate-linux-x64",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("checksum mismatch"), "{err}");

        let err = verify_release_checksum(bytes, "", "reviewgate-linux-x64")
            .unwrap_err()
            .to_string();
        assert!(err.contains("checksum not found"), "{err}");
    }
}
