//! ReviewGate CLI —— 主形态。

mod fix;
mod render;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "reviewgate",
    about = "面向 AI Coding 时代的质量闸口：多 Agent 并行审查 + 分维度专家 + 置信度过滤",
    version = reviewgate_core::version(),
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 审查当前 git diff
    Review(ReviewArgs),
    /// LLM 连通性自检
    Llm {
        #[command(subcommand)]
        cmd: LlmCmd,
    },
    /// 打印解析后的 diff 摘要（调试用）。支持 --commit / --from --to，缺省为工作区。
    Diff(DiffArgs),
    /// 调用单个工具（调试用）：reviewgate tool <name> '<json>'
    Tool {
        name: String,
        #[arg(default_value = "{}")]
        input: String,
    },
    /// 跑单维度 Agent（调试用）：reviewgate agent --dimension logic
    Agent {
        /// 维度：security | perf | logic | style | ai_smell
        #[arg(long, default_value = "logic")]
        dimension: String,
    },
}

#[derive(Subcommand)]
enum LlmCmd {
    /// 向默认提供方发一次最小请求，验证连通性
    Test,
}

/// diff 范围选择（review 与 diff 共用）。
#[derive(Parser)]
struct DiffArgs {
    /// 审查单个 commit 引入的改动
    #[arg(long)]
    commit: Option<String>,
    /// 范围起点（与 --to 配合，自 merge-base 起）
    #[arg(long)]
    from: Option<String>,
    /// 范围终点（与 --from 配合）
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
            anyhow::bail!("--from 与 --to 必须同时提供")
        }
        _ => DiffMode::Workspace,
    })
}

#[derive(Parser)]
struct ReviewArgs {
    /// 输出格式：text | json
    #[arg(long, default_value = "text")]
    format: String,
    /// 审查维度：all 或逗号分隔 security,perf,logic,style,ai_smell
    #[arg(long, default_value = "all")]
    dimensions: String,
    /// 审查单个 commit 引入的改动
    #[arg(long)]
    commit: Option<String>,
    /// 范围审查起点（与 --to 配合，自 merge-base 起）
    #[arg(long)]
    from: Option<String>,
    /// 范围审查终点（与 --from 配合）
    #[arg(long)]
    to: Option<String>,
    /// 跳过证伪 Judge（更快，但误报更多）
    #[arg(long)]
    no_judge: bool,
    /// 展开被过滤的低置信项
    #[arg(long)]
    show_filtered: bool,
    /// 何种判定导致非 0 退出码：block | warn | never
    #[arg(long, default_value = "block")]
    fail_on: String,
    /// 在 GitHub PR 上发布摘要评论（用于 GitHub Action）
    #[arg(long)]
    comment: bool,
    /// 打印每个维度每轮的进度到 stderr
    #[arg(long, short)]
    verbose: bool,
    /// 单维度墙钟超时（秒，0=不限）。超时跳过该维度、保留其余，适合 CI 兜底。
    #[arg(long, default_value = "0")]
    timeout: u64,
    /// 每维度采样次数（默认 1）。>1 取并集提升对 flaky 漏报（如 SSRF）的召回稳定性，成本 ×N。
    #[arg(long, default_value = "1")]
    samples: usize,
    /// Judge 并发上限，避免候选过多时触发 provider 限流。
    #[arg(long, default_value = "4")]
    judge_concurrency: usize,
    /// 逐条 y/N 确认后，把 suggestion_code 应用到工作区文件（非终端不应用）。
    #[arg(long)]
    fix: bool,
    /// 开启 run_check 沙箱执行（logic 维度可真正运行边界用例验证细微算法）。
    /// 会执行模型生成的自包含 JS/Python 片段——仅在可信/CI 沙箱环境使用。默认关闭。
    #[arg(long)]
    exec_verify: bool,
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
        other => anyhow::bail!("未知维度：{other}"),
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
        eprintln!("没有检测到改动。");
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
    eprintln!("跑维度 [{}]，模型 {} …", dim, client.model());
    let mut findings = run_agent(&*client, &reg, &ctx, &agent_cfg, user_prompt).await?;

    // M1.9 行号重定位。
    reviewgate_core::relocate::relocate_all(&mut findings, std::path::Path::new(&root), &None, &d)
        .await;

    println!("{}", serde_json::to_string_pretty(&findings)?);
    eprintln!("共 {} 条发现。", findings.len());
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
        "ReviewGate 审查中（基础维度 {} 个：{}{}；samples={}；实际 Agent {} 个）…",
        dims.len(),
        names.join(", "),
        if auto_business {
            "；自动加入 business"
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
    let outcome = run_review(&cfg, &opts).await?;

    match args.format.as_str() {
        "json" => println!("{}", render::render_json(&outcome)?),
        _ => print!("{}", render::render_text(&outcome, args.show_filtered)),
    }

    // 可选：在 GitHub PR 上发摘要评论 + 行内 suggestion（作者一键应用，人把关）。
    if args.comment {
        if let Err(e) = reviewgate_core::github::post_summary(&outcome).await {
            eprintln!("发布摘要评论失败：{e}");
        }
        if let Err(e) = reviewgate_core::github::post_inline_suggestions(&outcome).await {
            eprintln!("发布行内评论失败：{e}");
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
    println!("改动文件数：{}", d.files.len());
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
        "提供方：{}（{:?}）  模型：{}  端点：{}",
        cfg.provider, provider.protocol, provider.model, provider.base_url
    );

    let client = build_client(&provider)?;
    let messages = vec![Message::user("用一句话回复：连接正常。")];
    let resp = client
        .complete("你是连通性自检助手，请简短回复。", &messages, &[])
        .await?;

    println!("---\n回复：{}", resp.text().trim());
    println!(
        "停止原因：{:?}  用量：in={} out={}",
        resp.stop_reason, resp.usage.input_tokens, resp.usage.output_tokens
    );
    println!("✅ LLM 连通正常");
    Ok(())
}
