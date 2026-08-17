//! Issue 分诊产品 CLI：`issue` / `serve` / `daemon`。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::review_cmd::OutputFormat;

#[derive(Subcommand)]
pub(crate) enum IssueCmd {
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
pub(crate) struct IssueSyncArgs {
    /// owner/repo (default: REVIEWGATE_REPO or git remote origin)
    #[arg(long)]
    repo: Option<String>,
    /// Local data directory (default: .reviewgate/issue)
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Max issues to pull (default: `[issue_review.sync] max_history_issues`)
    #[arg(long)]
    max: Option<usize>,
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
pub(crate) struct IssueReviewCliArgs {
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
    /// Run Level-0 code verification against a local checkout: match the reported error to a
    /// source line, expand the enclosing function, and look for prior fixes to that file.
    /// Measured on 1020 real cli/cli issues, this adds ~9 points of discriminative power over
    /// classification alone (see docs/LIMITATIONS.md #11) — treat it as evidence for the reply,
    /// not as the primary basis for deciding whether something is a real bug.
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
    /// Print the raw decision trail (types, scores, reasons) instead of just the summary
    #[arg(long, default_value_t = false)]
    verbose: bool,
    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
    /// Do not call the model (no classification fallback, no narrative rewrite).
    #[arg(long, default_value_t = false)]
    no_llm: bool,
}

#[derive(Parser)]
pub(crate) struct IssueWatchArgs {
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
    /// Max issues synced and triaged per poll. The rest stay queued for the next round —
    /// keeps the first run against a large backlog from burning the platform's API quota in one go.
    #[arg(long, default_value = "20")]
    max_issues_per_run: usize,
    /// Skip the sync step and only triage what is already indexed. Useful when re-measuring
    /// after a rule change: syncing walks the whole repository looking for new issues, which
    /// costs API quota and minutes without affecting the triage result.
    #[arg(long, default_value_t = false)]
    no_sync: bool,
    /// Ask the model to classify the issues the rules are unsure about (low confidence, or a
    /// near-tie between two types). Off by default: watch is otherwise a zero-model-cost mode.
    /// Measured on real data, 61% of misclassifications fall in the low-confidence band and a
    /// further 39% are near-ties, so this is where the accuracy is.
    #[arg(long, default_value_t = false)]
    llm: bool,
    #[arg(long)]
    fixture: bool,
    /// Run Level-0 code verification against a local checkout: match the reported error to a
    /// source line, expand the enclosing function, and look for prior fixes to that file.
    /// Measured on 1020 real cli/cli issues, this adds ~9 points of discriminative power over
    /// classification alone (see docs/LIMITATIONS.md #11) — treat it as evidence for the reply,
    /// not as the primary basis for deciding whether something is a real bug.
    #[arg(long, default_value_t = false)]
    verify: bool,
    #[arg(long)]
    repo_root: Option<PathBuf>,
    /// Post/update the bot comment when `[issue_review] mode = "publish"`.
    #[arg(long, default_value_t = false)]
    publish: bool,
    /// Re-triage stored issues even if content/comment hashes are unchanged (eval only).
    #[arg(long, default_value_t = false)]
    force_retriage: bool,
}

#[derive(Parser)]
pub(crate) struct ServeArgs {
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
pub(crate) struct DaemonArgs {
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
    /// Run Level-0 code verification against a local checkout: match the reported error to a
    /// source line, expand the enclosing function, and look for prior fixes to that file.
    /// Measured on 1020 real cli/cli issues, this adds ~9 points of discriminative power over
    /// classification alone (see docs/LIMITATIONS.md #11) — treat it as evidence for the reply,
    /// not as the primary basis for deciding whether something is a real bug.
    #[arg(long, default_value_t = false)]
    verify: bool,
    /// Max outer loops for testing (0 = forever)
    #[arg(long, default_value = "0")]
    max_iterations: u64,
    /// Max issues synced and triaged per poll. The rest stay queued for the next round.
    #[arg(long, default_value = "20")]
    max_issues_per_run: usize,
    #[arg(long)]
    fixture: bool,
    /// Post/update the bot comment when `[issue_review] mode = "publish"`.
    #[arg(long, default_value_t = false)]
    publish: bool,
    /// Same as `issue watch --llm`.
    #[arg(long, default_value_t = false)]
    llm: bool,
    /// Re-triage stored issues even if content/comment hashes are unchanged (eval only).
    #[arg(long, default_value_t = false)]
    force_retriage: bool,
}

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

fn refuse_publish_unless_mode_allows(
    cfg: &reviewgate_core::issue::IssueReviewConfig,
) -> anyhow::Result<()> {
    if !cfg.actions.publish {
        anyhow::bail!(
            "--publish requires [issue_review] mode = \"publish\" (current mode is suggest; nothing was posted)"
        );
    }
    Ok(())
}

async fn maybe_publish(
    store: &reviewgate_core::issue::IssueStore,
    platform: &dyn reviewgate_core::issue::IssuePlatform,
    out: &reviewgate_core::issue::ReviewOutput,
    want_publish: bool,
) -> anyhow::Result<Option<reviewgate_core::issue::PublishResult>> {
    if out.skipped.is_some() {
        return Ok(None);
    }
    if !want_publish || !out.planned.has_writes() {
        return Ok(None);
    }
    Ok(Some(
        reviewgate_core::issue::publish_decision(store, platform, out).await?,
    ))
}

fn pick_issue_numbers(
    store: &reviewgate_core::issue::IssueStore,
    budget: usize,
    force_retriage: bool,
) -> anyhow::Result<Vec<u64>> {
    if force_retriage {
        let mut nums = store.list_issue_numbers()?;
        nums.truncate(budget);
        Ok(nums)
    } else {
        store.issues_due_for_triage(budget)
    }
}

pub(crate) fn require_issue_review_enabled() -> anyhow::Result<()> {
    match reviewgate_core::config::Config::load() {
        Ok(c) if !c.issue_review.enabled => anyhow::bail!(
            "[issue_review] enabled = false; set it to true to run issue init/sync/review/watch/serve/daemon"
        ),
        _ => Ok(()),
    }
}

fn issue_sync_overlap() -> std::time::Duration {
    let raw = reviewgate_core::config::Config::load()
        .ok()
        .map(|c| c.issue_review.sync.overlap)
        .unwrap_or_else(|| "5m".into());
    let secs = reviewgate_core::issue::parse_duration_secs(&raw).unwrap_or(300);
    std::time::Duration::from_secs(secs)
}

fn issue_max_history() -> usize {
    reviewgate_core::config::Config::load()
        .ok()
        .map(|c| c.issue_review.sync.max_history_issues)
        .filter(|&n| n > 0)
        .unwrap_or(10_000)
}

fn issue_review_cfg_from_file() -> reviewgate_core::issue::IssueReviewConfig {
    use reviewgate_core::issue::{ActionPolicy, IssueReviewConfig, MentionConfig};
    let mut cfg = IssueReviewConfig::default();
    if let Ok(file) = reviewgate_core::config::Config::load() {
        cfg.enabled = file.issue_review.enabled;
        cfg.overlap = file.issue_review.sync.overlap.clone();
        cfg.max_history_issues = file.issue_review.sync.max_history_issues;
        cfg.vector_enabled = file.issue_review.vector.enabled;
        cfg.candidate_limit = file.issue_review.duplicate.candidate_limit;
        cfg.min_similarity = file.issue_review.duplicate.min_similarity;
        cfg.actions = ActionPolicy {
            // `mode` 终于接上了：只有显式写 publish 才放行写操作，
            // 其余取值（含默认的 suggest 和任何笔误）一律视为只分析不发言。
            publish: file
                .issue_review
                .mode
                .trim()
                .eq_ignore_ascii_case("publish"),
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

pub(crate) async fn issue_sync(args: &IssueSyncArgs, is_init: bool) -> anyhow::Result<()> {
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
    let max = args.max.unwrap_or_else(issue_max_history);

    let synced = if args.fixture {
        let platform = FixturePlatform::new();
        seed_demo_fixture(&platform)?;
        sync_from_platform(&store, &platform, max, None, true).await?
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
        let since_arg =
            reviewgate_core::issue::since_with_overlap(since.as_deref(), issue_sync_overlap());
        sync_from_platform(&store, platform.as_ref(), max, since_arg.as_deref(), true).await?
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

pub(crate) async fn issue_review_cmd(args: &IssueReviewCliArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::model::{RawIssue, RawLabel, RawUser};
    use reviewgate_core::issue::{
        publish_decision, resolve_repo_root, review_issue_with_llm, FixturePlatform, IssuePlatform,
        IssueStore, LocalEmbedder,
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
    if args.publish {
        refuse_publish_unless_mode_allows(&cfg)?;
    }
    cfg.verify_enabled = args.verify && !args.triage_only;
    cfg.repo_root = args.repo_root.clone().or_else(|| resolve_repo_root(None));
    let emb = LocalEmbedder;
    let forge = resolve_forge(&args.forge)?;
    // 有配置时用 LLM 写面向用户的说明（证据仍来自本地检索）
    let llm_box = if args.no_llm {
        None
    } else {
        reviewgate_core::config::Config::load()
            .ok()
            .and_then(|c| c.active_provider_resolved().ok())
            .and_then(|p| build_client(&p).ok())
    };
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

    // JSON 在动作执行后再打印，`published` 才是真实结果；文本先打，便于边看边决定。
    let json_mode = matches!(args.format, OutputFormat::Json);
    if !json_mode {
        print!(
            "{}",
            crate::render::render_issue_review(&out, args.verbose, args.publish)
        );
    }

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
            if json_mode {
                // JSON 模式下这行会污染输出，结果已在 envelope 的 published/planned 里
            } else {
                println!(
                    "published: comment_id={} created={} updated={} close={} labels={:?}",
                    pub_out.comment_id,
                    pub_out.created,
                    pub_out.updated,
                    out.planned.close,
                    out.planned.labels_to_add
                );
            }
        }
    }
    if json_mode {
        println!(
            "{}",
            crate::render::render_issue_review_json(&out, args.publish)?
        );
    }
    let _ = args.dry_run;
    Ok(())
}

/// Local triage statistics. The point of this view is the gated count: in long-running
/// modes a low-confidence issue is skipped silently, and without this nobody notices.
pub(crate) fn issue_stats(
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

pub(crate) fn issue_inspect(
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

pub(crate) async fn issue_watch(args: &IssueWatchArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::{
        resolve_repo_root, review_issue_with_llm, sync_from_platform, FixturePlatform, IssueStore,
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
    if args.publish {
        refuse_publish_unless_mode_allows(&cfg)?;
    }
    cfg.verify_enabled = args.verify;
    cfg.repo_root = args.repo_root.clone().or_else(|| resolve_repo_root(None));
    let emb = LocalEmbedder;
    let forge = resolve_forge(&args.forge)?;
    // 只有 --llm 才建客户端：不开就完全没有模型开销，watch 保持零成本。
    let llm: Option<Box<dyn reviewgate_core::llm::LlmClient>> = if args.llm {
        let c = reviewgate_core::config::Config::load()
            .ok()
            .and_then(|c| c.active_provider_resolved().ok())
            .and_then(|p| reviewgate_core::llm::build_client(&p).ok());
        if c.is_none() {
            eprintln!("--llm 指定了但没有可用的 provider 配置；退回纯规则分类");
        }
        c
    } else {
        None
    };

    let fixture_platform = if args.fixture {
        let p = FixturePlatform::new();
        seed_demo_fixture(&p)?;
        Some(p)
    } else {
        None
    };

    let mut iter = 0u64;
    loop {
        iter += 1;
        eprintln!(
            "issue watch: iteration {iter} repo={repo} forge={}",
            forge.as_str()
        );
        let budget = args.max_issues_per_run.max(1);
        let mut synced_n = 0usize;
        let mut triaged = 0usize;
        let mut skipped_unchanged = 0usize;
        let mut published_n = 0usize;
        let mut publish_failed = 0usize;
        let mut gated = 0usize;

        let live;
        let platform: &dyn reviewgate_core::issue::IssuePlatform = if let Some(p) =
            fixture_platform.as_ref()
        {
            if !args.no_sync {
                let synced = sync_from_platform(&store, p, budget, None, true).await?;
                synced_n = synced.len();
                eprintln!("synced {synced_n} fixture issues");
            }
            p
        } else {
            live = build_live_platform(forge, &args.api_base, &repo)?;
            if args.no_sync {
                eprintln!("--no-sync: 跳过同步，只分诊已入库的");
            } else {
                let since = store.get_sync_cursor()?;
                let since_arg = reviewgate_core::issue::since_with_overlap(
                    since.as_deref(),
                    issue_sync_overlap(),
                );
                let synced =
                    sync_from_platform(&store, live.as_ref(), budget, since_arg.as_deref(), true)
                        .await?;
                synced_n = synced.len();
                eprintln!("synced {synced_n} issues");
            }
            if let Ok(Some(c)) = store.get_sync_cursor() {
                eprintln!("sync cursor (ISO8601): {c}");
            }
            live.as_ref()
        };

        let due = pick_issue_numbers(&store, budget, args.force_retriage)?;
        let candidates = if args.force_retriage {
            due
        } else {
            let all = store.list_issue_numbers()?;
            skipped_unchanged = all.len().saturating_sub(due.len());
            due
        };
        for num in candidates {
            match review_issue_with_llm(&store, platform, num, &cfg, &emb, llm.as_deref()).await {
                Ok(out) => {
                    if out.skipped.is_some() {
                        continue;
                    }
                    triaged += 1;
                    if out.decision.confidence < cfg.actions.min_confidence {
                        gated += 1;
                    }
                    let pub_note = match maybe_publish(&store, platform, &out, args.publish).await {
                        Ok(Some(p)) if p.skipped_truncated => {
                            publish_failed += 1;
                            String::new()
                        }
                        Ok(Some(p)) if p.created => {
                            published_n += 1;
                            " published=created".into()
                        }
                        Ok(Some(p)) if p.updated => {
                            published_n += 1;
                            " published=updated".into()
                        }
                        Ok(_) => String::new(),
                        Err(e) => {
                            publish_failed += 1;
                            eprintln!("  #{num} publish failed: {e:#}");
                            String::new()
                        }
                    };
                    eprintln!(
                        "  #{} → {} ({:.0}%) type={} dup={} tech={}{pub_note}",
                        num,
                        out.decision.verdict.as_str(),
                        out.decision.confidence * 100.0,
                        out.decision.primary_type.as_str(),
                        out.decision.duplicate_status.as_str(),
                        out.decision.technical_verdict.as_str()
                    );
                }
                Err(e) => eprintln!("  #{num} review failed: {e:#}"),
            }
        }
        if skipped_unchanged > 0 {
            eprintln!("  skipped_unchanged={skipped_unchanged}");
        }
        eprintln!(
            "watch round: synced={synced_n} triaged={triaged} skipped_unchanged={skipped_unchanged} published={published_n} publish_failed={publish_failed} gated={gated}"
        );
        let backlog = store.issues_due_for_triage(budget + 1)?.len();
        if backlog > 0 {
            eprintln!(
                "  {backlog}{} issue(s) still waiting; they are picked up next round (--max-issues-per-run={budget}).",
                if backlog > budget { "+" } else { "" }
            );
        }
        if args.max_iterations > 0 && iter >= args.max_iterations {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    Ok(())
}

pub(crate) async fn issue_serve(args: &ServeArgs) -> anyhow::Result<()> {
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
        bot_logins: vec![
            "reviewgate[bot]".into(),
            "reviewgate-bot".into(),
            "github-actions[bot]".into(),
        ],
    };
    run_webhook_server(cfg, None).await
}

pub(crate) async fn issue_daemon(args: &DaemonArgs) -> anyhow::Result<()> {
    use reviewgate_core::issue::{
        drain_queue_once, payload_needs_full_review, resolve_repo_root, review_issue_with_llm,
        run_webhook_server, sync_from_platform, EventQueue, FixturePlatform, IssueStore,
        LocalEmbedder, ServeConfig,
    };

    let data_dir = issue_data_dir(args.data_dir.as_ref());
    let repo = if args.fixture {
        resolve_repo_slug(args.repo.as_deref()).unwrap_or_else(|_| "fixture/demo".into())
    } else {
        resolve_repo_slug(args.repo.as_deref())?
    };
    let store = IssueStore::open(&data_dir, &repo)?;
    let mut cfg = issue_review_cfg_from_file();
    if args.publish {
        refuse_publish_unless_mode_allows(&cfg)?;
    }
    cfg.verify_enabled = args.verify;
    cfg.repo_root = resolve_repo_root(None);
    let emb = LocalEmbedder;
    let forge = resolve_forge(&args.forge)?;
    let interval = parse_interval_secs(&args.interval)?;
    let queue_path = data_dir.join("webhook.db");
    let queue = EventQueue::open(&queue_path)?;
    let llm: Option<Box<dyn reviewgate_core::llm::LlmClient>> = if args.llm {
        reviewgate_core::config::Config::load()
            .ok()
            .and_then(|c| c.active_provider_resolved().ok())
            .and_then(|p| reviewgate_core::llm::build_client(&p).ok())
    } else {
        None
    };
    let fixture_platform = if args.fixture {
        let p = FixturePlatform::new();
        seed_demo_fixture(&p)?;
        Some(p)
    } else {
        None
    };
    let bot_logins = vec![
        "reviewgate[bot]".to_string(),
        "reviewgate-bot".to_string(),
        "github-actions[bot]".to_string(),
    ];

    if args.serve {
        // 与 `reviewgate serve` 保持同一契约：缺 secret 就报错退出。
        // 回退到源码里写死的常量等于把 webhook 签名校验作废——任何读过源码的人
        // 都能伪造事件，驱动 Issue 的评论/打标签/关闭/指派。
        let secret = args
            .webhook_secret
            .clone()
            .or_else(|| std::env::var("REVIEWGATE_WEBHOOK_SECRET").ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--serve needs a webhook secret: pass --webhook-secret or set REVIEWGATE_WEBHOOK_SECRET"
                )
            })?;
        let scfg = ServeConfig {
            listen: args.listen.clone(),
            webhook_secret: secret,
            queue_path: queue_path.clone(),
            bot_logins: bot_logins.clone(),
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
            let want_publish = args.publish;
            let bots = bot_logins.clone();
            let fixture_platform = fixture_platform.as_ref();
            let llm_ref = llm.as_deref();
            async move {
                let bots: Vec<&str> = bots.iter().map(|s| s.as_str()).collect();
                if !payload_needs_full_review(&d.event_type, &d.payload, &bots) {
                    return Ok(());
                }
                let Some(num) = d.issue_number else {
                    return Ok(());
                };
                if !d.event_type.is_empty() {
                    eprintln!(
                        "  queue {} {} #{} ({})",
                        d.event_type, d.action, num, d.delivery_id
                    );
                }
                if let Some(platform) = fixture_platform {
                    let out =
                        review_issue_with_llm(store, platform, num, cfg, emb, llm_ref).await?;
                    let _ = maybe_publish(store, platform, &out, want_publish).await?;
                } else {
                    let platform = build_live_platform(forge, &api_base, &repo)?;
                    let out =
                        review_issue_with_llm(store, platform.as_ref(), num, cfg, emb, llm_ref)
                            .await?;
                    let _ = maybe_publish(store, platform.as_ref(), &out, want_publish).await?;
                }
                Ok(())
            }
        })
        .await?;
        if nq > 0 {
            eprintln!("  processed {nq} queue deliveries");
        }

        // 2) poll sync + triage
        let budget = args.max_issues_per_run.max(1);
        let live;
        let platform: &dyn reviewgate_core::issue::IssuePlatform = if let Some(p) =
            fixture_platform.as_ref()
        {
            sync_from_platform(&store, p, budget, None, true).await?;
            p
        } else {
            live = build_live_platform(forge, &args.api_base, &repo)?;
            let since = store.get_sync_cursor()?;
            let since_arg =
                reviewgate_core::issue::since_with_overlap(since.as_deref(), issue_sync_overlap());
            sync_from_platform(&store, live.as_ref(), budget, since_arg.as_deref(), true).await?;
            live.as_ref()
        };
        for num in pick_issue_numbers(&store, budget, args.force_retriage)? {
            match review_issue_with_llm(&store, platform, num, &cfg, &emb, llm.as_deref()).await {
                Ok(out) => {
                    if out.skipped.is_some() {
                        continue;
                    }
                    match maybe_publish(&store, platform, &out, args.publish).await {
                        Ok(Some(p)) if p.created => {
                            eprintln!(
                                "  poll #{num} → {} published=created",
                                out.decision.verdict.as_str()
                            )
                        }
                        Ok(Some(p)) if p.updated => {
                            eprintln!(
                                "  poll #{num} → {} published=updated",
                                out.decision.verdict.as_str()
                            )
                        }
                        Ok(_) => eprintln!("  poll #{} → {}", num, out.decision.verdict.as_str()),
                        Err(e) => eprintln!("  poll #{num} publish failed: {e:#}"),
                    }
                }
                Err(e) => eprintln!("  poll #{num} failed: {e:#}"),
            }
        }
        let backlog = store.issues_due_for_triage(budget + 1)?.len();
        if backlog > 0 {
            eprintln!("  {backlog} issue(s) still waiting; picked up next round.");
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
