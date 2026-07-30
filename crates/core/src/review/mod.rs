//! 审查编排：把 diff、Agent、工具、重定位串成一次完整审查。
//!
//! M1.10 跑单/多维度（顺序）。M2.2 会改为多维度并行，M2.3 加证伪 Judge，
//! M2.5 加闸口。CLI 只调用本模块，保持薄。

mod aggregate;
mod context;
pub mod cost;
pub mod coverage;
pub mod critical;
mod dedup;
mod incremental;
mod intent;
pub mod metrics;
mod prefetch;
pub mod profile;
mod rules;
mod secrets;
mod suppress;
mod units;

pub use cost::{
    estimate_from_units, estimate_review_cost, exceeds_budget, CostEstimate, TokenPrices,
};
pub use coverage::{build_coverage, refresh_unit_statuses, CoverageSnapshot};
pub use critical::{
    critical_incomplete_forces_fail, incomplete_advice, resolve_critical_globs, unfinished_paths,
};
pub use dedup::dedupe;
pub use metrics::RunMetrics;
pub use profile::RunProfile;
pub use rules::{build_rules_section, build_rules_section_with_warnings};
pub use secrets::{match_added_line as match_secret_line, scan_diff as scan_secrets};
pub use suppress::fingerprint;
pub use units::{plan_units, summarize_units, ReviewUnit, UnitJobSummary, UnitPlanSummary};

use aggregate::{boost_cross_dimension_agreement, sort_findings};
use context::{build_unit_prompt, new_ref_for};

use crate::agent::{
    dimension_focus_block_with_deep, run_agent_with_stats, shared_system_prompt, AgentConfig,
    AgentExitReason, AgentRun, AgentStats,
};
use crate::config::{Config, GateConfig, DEFAULT_MAX_INPUT_TOKENS};
use crate::diff::{self, Diff, DiffMode};
use crate::gate::{apply_gate, apply_incomplete_policy, GateDecision};
use crate::index::{CachingIndex, PersistentIndex, RepoIndex};
use crate::judge::{judge_all_with_stats_limited, JudgeStats};
use crate::llm::{build_client, estimate_tokens, LlmClient};
use crate::model::{Dimension, Finding, Usage};
use crate::relocate::relocate_all;
use crate::review::incremental::{review_signature, IncrementalCache};
use crate::review::suppress::{apply_suppression, load_ignore};
use crate::tool::{readonly_tools, ToolContext, ToolRegistry};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

/// Review depth profile. Standard is the multi-dimension quality gate; Deep is
/// security-only thorough review (`reviewgate security`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewProfile {
    /// Default multi-dimension gate: standard security checklist, samples=1.
    #[default]
    Standard,
    /// Security deep review: security-only, higher samples, sink-driven focus,
    /// deterministic secret precheck, incomplete never PASS.
    Deep,
}

impl ReviewProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewProfile::Standard => "standard",
            ReviewProfile::Deep => "deep",
        }
    }

    pub fn is_deep(self) -> bool {
        matches!(self, ReviewProfile::Deep)
    }
}

/// Default sample count for the deep security profile.
pub const DEEP_DEFAULT_SAMPLES: usize = 2;

/// 审查选项。
pub struct ReviewOptions {
    pub mode: DiffMode,
    pub dimensions: Vec<Dimension>,
    /// 是否运行证伪 Judge（默认 true）。
    pub judge: bool,
    /// 闸口阈值。
    pub gate: GateConfig,
    /// 是否打印每轮进度。
    pub verbose: bool,
    /// 单维度 Agent 墙钟上限（并行，故约等于审查阶段总耗时上限）。超时则跳过该维度、保留其余。
    pub timeout: Option<std::time::Duration>,
    /// 每个维度的采样次数（默认 1）。>1 时每维度并行跑多次、取**并集**，由 dedup 折叠重复、
    /// judge 过滤——以成本换取对 flaky 漏报（如 SSRF）的召回稳定性。
    pub samples: usize,
    /// 是否允许 `run_check` 沙箱执行（opt-in，默认 false）。开启后 logic 维度可真正运行
    /// 边界用例验证细微算法（如 off-by-one），代价是执行模型生成的自包含片段（见 LIMITATIONS）。
    pub exec_verify: bool,
    /// Judge 并发上限，避免候选过多时打满 provider 限流。
    pub judge_concurrency: usize,
    /// fan-out（单元×维度×样本）并发上限，避免大 PR 瞬时拉起几十路 LLM 流打满 provider 限流。
    pub fanout_concurrency: usize,
    /// 意图 / 参考文档（需求 / 设计 / 验收标准）。提供后由独立的整体性 Agent 做「实现 vs 意图」评审。
    /// None / 空 = 不做意图评审（零退化）。
    pub intent: Option<String>,
    /// 实时进度沉淀（CLI 据此单行渲染"在跑+干到哪了"）。None = 不记录。
    pub progress: Option<std::sync::Arc<crate::progress::Progress>>,
    /// 增量复审（opt-in，默认 false）：按文件缓存发现，只重审 hunk 变化的文件。
    /// 拿覆盖度换成本——见 LIMITATIONS。off 时零行为变化。
    pub incremental: bool,
    /// Standard vs deep security profile (does not add a new Dimension).
    pub profile: ReviewProfile,
    /// 运行姿态 gate/audit（影响默认采样等；与 Deep 正交）。
    pub run_profile: RunProfile,
    /// 跑前成本上限（USD）。需配置 price_per_mtok_* 才生效。
    pub max_cost_usd: Option<f64>,
    /// 跑前估算输入 token 上限；超过则拒绝开跑。
    pub max_est_input_tokens: Option<u64>,
    /// 仅估算成本后返回（不调 LLM）。由 CLI `--estimate-only` 使用。
    pub estimate_only: bool,
    /// 是否写入 `.reviewgate/cache/metrics.jsonl`（默认 true）。
    pub write_metrics: bool,
    /// Token 单价（USD / 百万 token），用于成本估算。
    pub token_prices: TokenPrices,
    /// 墙钟起点（毫秒指标）；None 则在 run 内自取。
    pub started: Option<std::time::Instant>,
}

impl ReviewOptions {
    pub fn new(mode: DiffMode, dimensions: Vec<Dimension>) -> Self {
        Self {
            mode,
            dimensions,
            judge: true,
            gate: GateConfig::default(),
            verbose: false,
            timeout: None,
            samples: 1,
            exec_verify: false,
            judge_concurrency: 4,
            fanout_concurrency: 6,
            intent: None,
            progress: None,
            incremental: false,
            profile: ReviewProfile::Standard,
            run_profile: RunProfile::Gate,
            max_cost_usd: None,
            max_est_input_tokens: None,
            estimate_only: false,
            write_metrics: true,
            token_prices: TokenPrices::default(),
            started: None,
        }
    }

    pub fn workspace(dimensions: Vec<Dimension>) -> Self {
        Self::new(DiffMode::Workspace, dimensions)
    }

    /// Security-only deep review defaults used by `reviewgate security`.
    ///
    /// - dimensions = `[Security]` only
    /// - samples = [`DEEP_DEFAULT_SAMPLES`] (≥ standard's 1)
    /// - profile = Deep (deep focus, secret precheck, fail-incomplete hard)
    pub fn security_deep(mode: DiffMode) -> Self {
        let mut opts = Self::new(mode, vec![Dimension::Security]);
        opts.profile = ReviewProfile::Deep;
        opts.samples = DEEP_DEFAULT_SAMPLES;
        opts.gate.fail_on_incomplete = true;
        opts
    }
}

/// 维度/单元未审完的告警。让消费方不把"没审完"误读成"通过"。
#[derive(Debug, Clone, Serialize)]
pub struct ReviewWarning {
    pub dimension: String,
    /// `timed_out` | `failed` | `incomplete` | `oversized` | `rules_unavailable`
    pub kind: &'static str,
    pub message: String,
    /// 相关文件路径（oversized / 单元级告警尽量填全）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// 可操作建议（提高 timeout、拆 PR 等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advice: Option<String>,
}

impl ReviewWarning {
    pub fn new(
        dimension: impl Into<String>,
        kind: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            dimension: dimension.into(),
            kind,
            message: message.into(),
            paths: Vec::new(),
            advice: None,
        }
    }

    pub fn with_paths(mut self, paths: Vec<String>) -> Self {
        self.paths = paths;
        self
    }

    pub fn with_advice(mut self, advice: impl Into<String>) -> Self {
        self.advice = Some(advice.into());
        self
    }
}

/// 审查结果。
pub struct ReviewOutcome {
    pub findings: Vec<Finding>,
    pub files_changed: usize,
    pub decision: GateDecision,
    /// 未审完的维度/单元告警。非空表示结果可能不完整。
    pub warnings: Vec<ReviewWarning>,
    /// 是否有单元未审完（请求失败/上下文超限/超时/被跳过）。配合 `fail_on_incomplete` 决定是否阻止 PASS。
    pub incomplete: bool,
    /// 本次审查累计 token 用量（Agent + Judge）。
    pub usage: Usage,
    /// 跑前成本估算（若编排阶段已计算）。
    pub cost_estimate: Option<CostEstimate>,
    /// 关键路径 incomplete 是否触发强制失败策略。
    pub critical_incomplete: bool,
    /// 落盘/展示用的运行指标（可选）。
    pub run_metrics: Option<RunMetrics>,
    /// 多单元目录装箱计划（大 PR 合成报告）。
    pub unit_plan: Option<UnitPlanSummary>,
    /// 覆盖快照：covered / unfinished / oversized 路径。
    pub coverage: Option<CoverageSnapshot>,
}

impl Default for ReviewOutcome {
    fn default() -> Self {
        Self {
            findings: Vec::new(),
            files_changed: 0,
            decision: GateDecision::Pass,
            warnings: Vec::new(),
            incomplete: false,
            usage: Usage::default(),
            cost_estimate: None,
            critical_incomplete: false,
            run_metrics: None,
            unit_plan: None,
            coverage: None,
        }
    }
}

/// 执行一次审查。管线：多维并行 → 重定位 → 去重 → 证伪 Judge → 闸口。
/// 按配置构造 LLM 客户端后委托给 [`run_review_with_client`]。
pub async fn run_review(cfg: &Config, opts: &ReviewOptions) -> Result<ReviewOutcome> {
    let client = build_client(&cfg.active_provider_resolved()?)?;
    run_review_with_client(cfg, opts, &*client).await
}

/// 同 [`run_review`]，但**注入** LLM 客户端——便于用 mock 做端到端编排测试（不联网）。
pub async fn run_review_with_client(
    cfg: &Config,
    opts: &ReviewOptions,
    client: &dyn LlmClient,
) -> Result<ReviewOutcome> {
    let root = diff::git::repo_root().await?;
    let diff: Arc<Diff> = Arc::new(diff::collect(&opts.mode).await?);
    let started = opts.started.unwrap_or_else(std::time::Instant::now);

    if diff.files.is_empty() {
        return Ok(ReviewOutcome {
            files_changed: 0,
            ..Default::default()
        });
    }

    let new_ref = new_ref_for(&opts.mode);
    let mut warnings: Vec<ReviewWarning> = Vec::new();
    let mut incomplete = false;

    // 项目规则正文：注入每个单元 prompt 的末尾（跨维度可缓存）。
    let rules_section = build_rules_section_with_warnings(&cfg.business, &diff, Path::new(&root));
    let rules_body = rules_section.body.clone();
    for message in rules_section.warnings {
        warnings.push(
            ReviewWarning::new(Dimension::Business.as_str(), "rules_unavailable", message)
                .with_advice("Fix rules_dir / path_rules globs or set builtin_path_rules"),
        );
    }

    // 配置了任一规则来源（inline / rules_dir / skills_dir）就自动并入 Business 维度。
    // Deep security stays security-only — do not pull in business rules dimension.
    let has_business_rules = !cfg.business.rules.is_empty()
        || cfg.business.rules_dir.is_some()
        || cfg.business.skills_dir.is_some();
    let mut dims = opts.dimensions.clone();
    if !opts.profile.is_deep() && has_business_rules && !dims.contains(&Dimension::Business) {
        dims.push(Dimension::Business);
    }
    let deep = opts.profile.is_deep();
    // 输入预算 → 把 diff 切成审查单元（正常 PR = 1 个单元，零退化）。
    let budget = cfg
        .active_provider()
        .map(|p| p.max_input_tokens())
        .unwrap_or(DEFAULT_MAX_INPUT_TOKENS) as usize;
    // 预留系统提示词 + 维度 focus 的固定开销：plan_units 只按 diff 计 token，但 Agent 预检会
    // 算上 system+focus。小/中预算下若不预留，切出的单元会在预检全部超预算（审不到任何东西）。
    let overhead = estimate_tokens(&shared_system_prompt())
        + dims
            .iter()
            .map(|d| estimate_tokens(&dimension_focus_block_with_deep(*d, deep)))
            .max()
            .unwrap_or(0)
        + 256;
    let plan_budget = budget.saturating_sub(overhead).max(512);
    let mut units = plan_units(&diff, plan_budget);
    let mut unit_plan = summarize_units(&diff, &units);
    // 多单元（大 PR）本就庞大：不再叠采样，避免 单元×维度×样本 的成本放大。
    // 多采样只在单单元（正常 PR）上用于提升 flaky 漏报（如 SSRF）的召回稳定性。
    let samples = if units.len() > 1 {
        1
    } else {
        opts.samples.max(1)
    };
    if units.len() > 1 {
        eprintln!(
            "  [units] large diff → {} review units (directory packing); samples forced to 1",
            units.len()
        );
        for job in &unit_plan.units {
            eprintln!(
                "    unit[{}] ~{} tok{}: {}",
                job.id,
                job.est_tokens,
                if job.oversized { " OVERSIZED" } else { "" },
                job.paths.join(", ")
            );
        }
    }

    // 跑前成本估算 + 预算守卫（estimate-only 在此返回）。
    let cost_estimate =
        estimate_from_units(&diff, &units, &dims, samples, opts.judge, opts.token_prices);
    eprintln!("  [cost] {}", cost_estimate.summary);
    if let Some(why) = exceeds_budget(&cost_estimate, opts.max_cost_usd, opts.max_est_input_tokens)
    {
        anyhow::bail!(
            "budget exceeded before review: {why}\n  estimate: {}\n  raise --max-cost / --max-input-tokens, narrow --dimensions, or split the PR",
            cost_estimate.summary
        );
    }
    if opts.estimate_only {
        let coverage = build_coverage(&diff, &unit_plan, &[], false);
        return Ok(ReviewOutcome {
            findings: Vec::new(),
            files_changed: diff.files.len(),
            decision: GateDecision::Pass,
            warnings: Vec::new(),
            incomplete: false,
            usage: Usage::default(),
            cost_estimate: Some(cost_estimate),
            critical_incomplete: false,
            run_metrics: None,
            unit_plan: Some(unit_plan),
            coverage: Some(coverage),
        });
    }

    // 增量复审（opt-in）：签名一致且文件 diff 逐字节不变的文件复用上轮发现，
    // 只把变化文件留给 fan-out。签名放在 units/samples 之后算（samples 受单元数影响）。
    // 命中项存 `incremental_reused`，闸口前并回；缓存在本轮重审文件评完后更新。
    let incremental = if opts.incremental {
        let sig = review_signature(
            &dims,
            client.model(),
            &rules_body,
            opts.judge,
            samples,
            opts.exec_verify,
            opts.profile.as_str(),
        );
        let cache = IncrementalCache::load(Path::new(&root));
        let (todo, reused) = incremental::partition(&diff, &sig, &cache);
        let todo_set: std::collections::HashSet<usize> = todo.iter().copied().collect();
        for u in &mut units {
            u.files.retain(|i| todo_set.contains(i));
        }
        units.retain(|u| !u.files.is_empty());
        if opts.verbose {
            eprintln!(
                "  [incremental] {} file(s) reused from cache, {} to review",
                diff.files.len() - todo.len(),
                todo.len()
            );
        }
        Some((todo, reused, cache, sig))
    } else {
        None
    };

    // 存在持久全仓索引（reviewgate index build 生成）则用之——find_definition 走完整查表；
    // 否则回退按需 TreeSitter（优雅降级，索引非必需）。
    let mut ctx = match RepoIndex::load(Path::new(&root)) {
        Some(repo_idx) => {
            if opts.verbose {
                eprintln!(
                    "  [index] using .reviewgate/cache/symbols.json ({} symbols)",
                    repo_idx.symbol_count()
                );
            }
            // 陈旧提示：仓库 HEAD 已变 → 索引可能过时。陈旧项已由位置校验安全回退按需，
            // 这里只是提醒重建以恢复"快+全"。
            let current_head = diff::git::git(&["rev-parse", "HEAD"])
                .await
                .ok()
                .map(|s| s.trim().to_string());
            if let (Some(built), Some(now)) = (repo_idx.built_at_head(), current_head.as_deref()) {
                if built != now {
                    eprintln!(
                        "  [index] symbols.json was built at an older HEAD; rerun `reviewgate index build` to refresh (stale entries safely fall back to on-demand lookup)."
                    );
                }
            }
            let index = Arc::new(CachingIndex::new(Arc::new(PersistentIndex::new(
                repo_idx,
                root.clone(),
            ))));
            ToolContext::new(diff.clone(), root.clone(), new_ref.clone(), index)
        }
        None => ToolContext::with_treesitter_index(diff.clone(), root.clone(), new_ref.clone()),
    };
    ctx.allow_exec = opts.exec_verify; // opt-in 沙箱执行（run_check）
    let mut reg = ToolRegistry::new();
    for t in readonly_tools() {
        reg.register(t);
    }

    // 为每个单元预构造 prompt：先带文件全文上下文；超预算则退化为 diff-only；仍超则跳过（未审完）。
    let unit_prompts = build_unit_prompts(
        &diff,
        &units,
        Path::new(&root),
        &new_ref,
        &rules_body,
        budget,
        overhead,
        &*ctx.index,
        &mut warnings,
        &mut incomplete,
    )
    .await;

    // fan-out：(单元 × 维度 × 样本) 并行。维度随每个 task 一起返回，以便 buffer_unordered
    // 乱序完成后仍能正确回填告警维度（不再依赖外部 labels 的下标对齐）。
    let mut tasks = Vec::new();
    for prompt_opt in unit_prompts.iter() {
        let Some(prompt) = prompt_opt else { continue };
        for d in &dims {
            for _ in 0..samples {
                let mut agent_cfg = AgentConfig::for_dimension(*d);
                agent_cfg.verbose = opts.verbose;
                agent_cfg.progress = opts.progress.clone();
                // 超时交给 Agent 内部"每轮检查、优雅收尾"，而非硬 cancel——保住已上报的发现。
                agent_cfg.timeout = opts.timeout;
                // 发送前预检预算：确定性避免撞 provider 的 context-length 上限。
                agent_cfg.max_input_tokens = Some(budget);
                if deep && *d == Dimension::Security {
                    agent_cfg.focus_override =
                        Some(dimension_focus_block_with_deep(Dimension::Security, true));
                }
                let prompt = Arc::clone(prompt);
                let reg = &reg;
                let ctx = &ctx;
                let dim = *d;
                tasks.push(async move {
                    let r = run_agent_with_stats(client, reg, ctx, &agent_cfg, prompt).await;
                    (dim, r)
                });
            }
        }
    }
    // 意图评审与 fan-out **并发**执行：意图 Agent 不依赖维度结果，故无需等 fan-out 完成再跑，
    // 否则总墙钟 ≈ fan-out + intent（翻倍）。并发后总耗时 ≈ max(fan-out, intent)。
    let intent_text = opts.intent.as_deref().filter(|s| !s.trim().is_empty());
    let intent_fut = async {
        match intent_text {
            Some(it) => Some(
                intent::run_intent_review(
                    client,
                    &reg,
                    &ctx,
                    &diff,
                    it,
                    budget,
                    opts.verbose,
                    opts.timeout,
                    opts.progress.clone(),
                )
                .await,
            ),
            None => None,
        }
    };
    // fan-out 用 buffer_unordered 限并发：大 PR 的 单元×维度×样本 可达数十，无上限并发会
    // 瞬时打满 provider 限流（与 judge 阶段保持一致的背压策略）。
    use futures::stream::StreamExt;
    let fanout_fut = futures::stream::iter(tasks)
        .buffer_unordered(opts.fanout_concurrency.max(1))
        .collect::<Vec<_>>();
    let (results, intent_outcome) = tokio::join!(fanout_fut, intent_fut);

    // 每(单元×维度)容错：单个失败只记告警，不影响其它返回部分结果；未审完则标记 incomplete。
    let (mut findings, agent_stats) =
        collect_agent_results(results, &mut warnings, &mut incomplete);

    // Deep profile: deterministic secret precheck (no LLM). Held aside and merged
    // **after** judge so insecure-by-construction hits cannot be false-negatived
    // by the counter-evidence stage (live: judge previously wiped sk_live_ findings).
    let secret_hits = if deep {
        let hits = secrets::scan_diff(&diff);
        if opts.verbose && !hits.is_empty() {
            eprintln!(
                "  [secrets] {} deterministic secret finding(s) (post-judge merge)",
                hits.len()
            );
        }
        hits
    } else {
        Vec::new()
    };
    // 质量闸口不能把"未审完"误读成"通过"：未审完的维度/单元已保留其部分发现，但仍要醒目提示。
    if incomplete {
        if warnings.iter().any(|w| w.kind == "auth_failed") {
            eprintln!(
                "! this review is incomplete because LLM authentication failed: \
                 fix the API key for the active provider (api_key in the config, or REVIEWGATE_API_KEY) and re-run."
            );
        } else {
            eprintln!(
                "! this review is incomplete (timeout/request failure/context overflow/oversized file skipped): the result may be partial. \
                 For a complete conclusion, raise --timeout, increase max_input_tokens, or split the change and re-run."
            );
        }
    }
    if opts.verbose {
        eprintln!(
            "  [agents] summary: {} LLM calls, {} tool calls ({}), {} loop-guards; {}",
            agent_stats.llm_requests,
            agent_stats.tool_calls,
            agent_stats.tool_summary(),
            agent_stats.loop_guarded,
            agent_stats.usage.summary()
        );
    }

    // 行号校验/兜底（模型多数已直接报标注行号）→ 跨维度去重。
    relocate_all(&mut findings, Path::new(&root), &new_ref, &diff).await;
    findings = dedupe(findings);

    // 意图 / 技术评审结果（已与 fan-out 并发跑完，见上）并入主结果：
    // 「问题类」verdict（missing/deviation/breaking/suggestion）过 Judge / 闸口；
    // 「已满足(met)」/「未核对(unknown)」是信息项——不判伪、不计入闸口，仅用于验收清单展示（闸口后再并入）。
    let mut intent_met: Vec<Finding> = Vec::new();
    if let Some(ir) = intent_outcome {
        if ir.incomplete {
            incomplete = true;
            warnings.push(
                ReviewWarning::new(
                    Dimension::Intent.as_str(),
                    "incomplete",
                    "intent review did not finish (timeout/context overflow); the result may be partial",
                )
                .with_advice("Raise --timeout or shorten the intent document"),
            );
        }
        for mut f in ir.findings {
            use crate::model::IntentStatus::{Met, Unknown};
            if matches!(f.intent_status, Some(Met) | Some(Unknown)) {
                f.filtered = true;
                intent_met.push(f);
            } else {
                findings.push(f);
            }
        }
    }

    // 证伪 Judge（可关）。
    let mut judge_stats = JudgeStats::default();
    if opts.judge && !findings.is_empty() {
        let judged = judge_all_with_stats_limited(
            client,
            &reg,
            &ctx,
            findings,
            opts.verbose,
            opts.judge_concurrency,
        )
        .await;
        findings = judged.0;
        judge_stats = judged.1;
    } else if opts.verbose && !opts.judge {
        eprintln!("  [judge] skipped (--no-judge)");
    }

    // Merge deterministic secret hits after judge; relocate then dedupe with agent findings.
    if !secret_hits.is_empty() {
        let mut secrets = secret_hits;
        relocate_all(&mut secrets, Path::new(&root), &new_ref, &diff).await;
        findings.extend(secrets);
        findings = dedupe(findings);
    }

    if opts.verbose {
        let mut total_usage = agent_stats.usage.clone();
        total_usage.add(&judge_stats.usage);
        eprintln!(
            "  [review] total: {} LLM calls, {} tool calls (agent: {} / judge: {}); {}",
            agent_stats.llm_requests + judge_stats.llm_requests,
            agent_stats.tool_calls + judge_stats.tool_calls,
            agent_stats.tool_calls,
            judge_stats.tool_calls,
            total_usage.summary()
        );
    }

    // 跨维度交叉印证加分：多个维度独立指向同一处 → 更可能是真问题。
    // 放在 Judge 之后（Judge 会重写置信度），让该信号能影响闸口与排序。
    boost_cross_dimension_agreement(&mut findings);

    // 增量复审收尾：把本轮重审文件的新发现写回缓存（判后-抑制前的终态），
    // 再并回上轮命中的缓存发现。缓存存取失败不影响审查（best-effort）。
    if let Some((todo, reused, mut cache, sig)) = incremental {
        incremental::store(&mut cache, &diff, &todo, &findings, &sig);
        if let Err(e) = cache.save(Path::new(&root)) {
            if opts.verbose {
                eprintln!("  [incremental] cache save failed (ignored): {e}");
            }
        }
        findings.extend(reused);
    }

    // 应用仓库 ignore 抑制：放在 judge+boost 之后、闸口之前。抑制项被拆出、
    // 闸口后再并回供 --show-filtered 展示——绝不让确认过的误报再次 BLOCK。
    // 抑制状态**不进增量缓存**（缓存的是判后-抑制前发现），故每轮按当前 ignore 重新判定，
    // 从 ignore 删除条目后即使文件未变、发现也会照常恢复。
    let ignored = load_ignore(Path::new(&root));
    let mut suppressed: Vec<Finding> = Vec::new();
    if !ignored.is_empty() {
        (findings, suppressed) = apply_suppression(findings, &ignored);
        if opts.verbose && !suppressed.is_empty() {
            eprintln!(
                "  [suppress] {} finding(s) matched .reviewgate/ignore",
                suppressed.len()
            );
        }
    }

    // 闸口：标记过滤项 + 判定。复合排序：未过滤优先 → 严重度降 → 置信度降。
    let mut decision = apply_gate(&mut findings, &opts.gate);
    // 未审完不变量：有单元未审完且 fail_on_incomplete 时，永不 PASS（至少 WARN；有 BLOCK 仍 BLOCK）。
    // Deep security always forces fail_on_incomplete (even if config turned it off).
    let fail_incomplete = opts.gate.fail_on_incomplete || deep;
    decision = apply_incomplete_policy(decision, incomplete, fail_incomplete);

    // 关键路径 incomplete：即便全局 fail_on_incomplete=false，触及 auth/payment 等路径仍强制非 PASS。
    let changed_paths: Vec<String> = diff.files.iter().map(|f| f.path().to_string()).collect();
    let critical_globs = resolve_critical_globs(&opts.gate.force_fail_incomplete_paths);
    let critical_incomplete =
        critical_incomplete_forces_fail(incomplete, &warnings, &changed_paths, &critical_globs);
    if critical_incomplete {
        decision = apply_incomplete_policy(decision, true, true);
        if opts.verbose || incomplete {
            eprintln!(
                "  [critical] incomplete review touches security-sensitive paths → force non-PASS"
            );
        }
    }

    // 已满足(met)的验收项和已抑制项在闸口之后并入：
    // 它们是信息项，只供验收清单 / --show-filtered 展示，不影响判定。
    findings.append(&mut intent_met);
    findings.append(&mut suppressed);
    sort_findings(&mut findings);

    let mut usage = agent_stats.usage.clone();
    usage.add(&judge_stats.usage);

    let kept = findings.iter().filter(|f| !f.filtered).count();
    let duration_ms = started.elapsed().as_millis() as u64;
    let run_metrics = RunMetrics::build(
        decision,
        incomplete,
        diff.files.len(),
        findings.len(),
        kept,
        warnings.len(),
        &usage,
        duration_ms,
        Some(opts.run_profile.as_str()),
        Some(&cost_estimate),
        critical_incomplete,
    );
    if opts.write_metrics {
        if let Err(e) = run_metrics.append_jsonl(Path::new(&root)) {
            if opts.verbose {
                eprintln!("  [metrics] write failed (ignored): {e}");
            }
        }
    }

    // 合成 unit/coverage 报告：刷新 unit 状态 + covered/unfinished 路径。
    refresh_unit_statuses(&mut unit_plan, &warnings);
    let coverage = build_coverage(&diff, &unit_plan, &warnings, incomplete);
    if coverage.should_surface() {
        eprintln!(
            "  [coverage] units={} covered={} unfinished={} oversized_skipped={}",
            unit_plan.unit_count,
            coverage.covered_paths.len(),
            coverage.unfinished_paths.len(),
            coverage.skipped_oversized_paths.len()
        );
    }

    Ok(ReviewOutcome {
        findings,
        files_changed: diff.files.len(),
        decision,
        warnings,
        incomplete,
        usage,
        cost_estimate: Some(cost_estimate),
        critical_incomplete,
        run_metrics: Some(run_metrics),
        unit_plan: Some(unit_plan),
        coverage: Some(coverage),
    })
}

/// 为每个单元预构造 prompt：先带文件全文上下文；超预算则退化为 diff-only；
/// 仍超则跳过（oversized 告警 + 标记未审完，绝不静默放行）。返回与 `units` 对齐的 `Option<String>`。
///
/// 预取块（改动符号的调用点，本地计算）会附加在 prompt 末尾以省 Agent 的取数往返；
/// 它参与预算估算，若因它超预算则**退回无预取版本**——预取只加分，绝不把临界单元挤成 oversized。
#[allow(clippy::too_many_arguments)]
async fn build_unit_prompts(
    diff: &Diff,
    units: &[ReviewUnit],
    root: &Path,
    new_ref: &Option<String>,
    rules_body: &str,
    budget: usize,
    overhead: usize,
    index: &dyn crate::index::CodeIndex,
    warnings: &mut Vec<ReviewWarning>,
    incomplete: &mut bool,
) -> Vec<Option<Arc<String>>> {
    let mut unit_prompts: Vec<Option<Arc<String>>> = Vec::with_capacity(units.len());
    for (ui, unit) in units.iter().enumerate() {
        let prefetched = prefetch::render_prefetch(index, diff, &unit.files).await;
        let prefetch_tokens = if !prefetched.is_empty() {
            // "\n\n" ≈ 1 token (2 ASCII / 3 ceiling). 略高估，预算守卫方向安全。
            estimate_tokens(&prefetched) + 1
        } else {
            0
        };
        let full = build_unit_prompt(diff, &unit.files, true, root, new_ref, rules_body).await;
        let full_tokens = estimate_tokens(&full);
        if full_tokens + prefetch_tokens + overhead <= budget {
            let mut full_pf = full;
            if !prefetched.is_empty() {
                full_pf.push_str("\n\n");
                full_pf.push_str(&prefetched);
            }
            unit_prompts.push(Some(Arc::new(full_pf)));
            continue;
        }
        if full_tokens + overhead <= budget {
            unit_prompts.push(Some(Arc::new(full)));
            continue;
        }
        let diff_only =
            build_unit_prompt(diff, &unit.files, false, root, new_ref, rules_body).await;
        let diff_only_tokens = estimate_tokens(&diff_only);
        if diff_only_tokens + prefetch_tokens + overhead <= budget {
            let mut diff_only_pf = diff_only;
            if !prefetched.is_empty() {
                diff_only_pf.push_str("\n\n");
                diff_only_pf.push_str(&prefetched);
            }
            unit_prompts.push(Some(Arc::new(diff_only_pf)));
            continue;
        }
        if diff_only_tokens + overhead <= budget {
            unit_prompts.push(Some(Arc::new(diff_only)));
            continue;
        }
        // 单文件 diff 自身就超预算，无法再切 → 跳过并标记未审完（绝不静默放行）。
        *incomplete = true;
        let paths: Vec<String> = unit
            .files
            .iter()
            .map(|&i| diff.files[i].path().to_string())
            .collect();
        let label = paths
            .first()
            .cloned()
            .unwrap_or_else(|| format!("unit{ui}"));
        eprintln!(
            "! file [{label}] diff exceeds input budget (~{} tok); skipped (not reviewed)",
            unit.est_tokens
        );
        warnings.push(
            ReviewWarning::new(
                format!("unit:{label}"),
                "oversized",
                format!(
                    "this file's diff exceeds the input budget (~{} tok > {budget}); skipped (not reviewed)",
                    unit.est_tokens
                ),
            )
            .with_paths(paths)
            .with_advice("Split the change into a smaller PR or raise provider max_input_tokens"),
        );
        unit_prompts.push(None);
    }
    unit_prompts
}

/// 汇总 fan-out 结果：聚合各 Agent 统计、按退出原因回填告警与未审完标记、收集 findings。
fn collect_agent_results(
    results: Vec<(Dimension, Result<AgentRun>)>,
    warnings: &mut Vec<ReviewWarning>,
    incomplete: &mut bool,
) -> (Vec<Finding>, AgentStats) {
    let mut findings = Vec::new();
    let mut agent_stats = AgentStats::default();
    for (dim, r) in results {
        match r {
            Ok(run) => {
                if let Some(w) = warning_for_exit(dim, &run) {
                    *incomplete = true;
                    warnings.push(w);
                }
                agent_stats.llm_requests += run.stats.llm_requests;
                agent_stats.tool_calls += run.stats.tool_calls;
                agent_stats.findings_reported += run.stats.findings_reported;
                agent_stats.task_done_calls += run.stats.task_done_calls;
                agent_stats.loop_guarded += run.stats.loop_guarded;
                agent_stats.usage.add(&run.stats.usage);
                for (name, count) in run.stats.tool_counts {
                    *agent_stats.tool_counts.entry(name).or_default() += count;
                }
                findings.extend(run.findings);
            }
            Err(e) => {
                *incomplete = true;
                warnings.push(
                    ReviewWarning::new(dim.as_str(), "failed", e.to_string())
                        .with_advice("Re-run with -v; check network/provider limits"),
                );
                eprintln!(
                    "! dimension [{}] review failed (skipped): {e}",
                    dim.as_str()
                );
            }
        }
    }
    (findings, agent_stats)
}

/// 非正常退出原因 → 告警（返回 Some 即应标记未审完）。正常完成/走满轮次返回 None。
fn warning_for_exit(dim: Dimension, run: &AgentRun) -> Option<ReviewWarning> {
    let detail = || {
        run.error_detail
            .as_deref()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default()
    };
    match run.exit_reason {
        AgentExitReason::TimedOut => Some(
            ReviewWarning::new(
                dim.as_str(),
                "timed_out",
                "wall-clock timeout; this dimension did not finish (its partial findings are kept)",
            )
            .with_advice("Raise --timeout (e.g. --timeout 300) and re-run"),
        ),
        AgentExitReason::AuthFailed => Some(
            ReviewWarning::new(
                dim.as_str(),
                "auth_failed",
                format!(
                    "LLM authentication failed — check the API key for the active provider (api_key in the config, or REVIEWGATE_API_KEY){}; this dimension did not finish",
                    detail()
                ),
            )
            .with_advice("Set REVIEWGATE_API_KEY or providers.*.api_key and re-run"),
        ),
        AgentExitReason::RequestFailed => Some(
            ReviewWarning::new(
                dim.as_str(),
                "incomplete",
                format!("LLM request failed{}; this dimension did not finish", detail()),
            )
            .with_advice("Re-run with -v; check provider rate limits / network"),
        ),
        AgentExitReason::ContextOverflow => Some(
            ReviewWarning::new(
                dim.as_str(),
                "incomplete",
                "context exceeded the input budget; pre-send check wrapped up early; this dimension did not finish",
            )
            .with_advice("Split the PR or raise provider max_input_tokens"),
        ),
        AgentExitReason::Completed | AgentExitReason::MaxRounds => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentExitReason, AgentRun, AgentStats};
    use crate::model::{Dimension, Finding, Reachability, Severity, Usage};

    fn finding(dim: Dimension) -> Finding {
        Finding {
            dimension: dim,
            confidence: 0.9,
            severity: Severity::High,
            path: "a.rs".into(),
            start_line: 1,
            end_line: 1,
            message: "m".into(),
            existing_code: "x".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Reachability::Unknown,
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    fn run_with(findings: Vec<Finding>, reason: AgentExitReason) -> AgentRun {
        let stats = AgentStats {
            llm_requests: 1,
            findings_reported: findings.len(),
            ..Default::default()
        };
        AgentRun {
            findings,
            stats,
            exit_reason: reason,
            error_detail: None,
        }
    }

    #[test]
    fn collect_aggregates_findings_and_stats() {
        let mut warnings = Vec::new();
        let mut incomplete = false;
        let results = vec![
            (
                Dimension::Security,
                Ok(run_with(
                    vec![finding(Dimension::Security)],
                    AgentExitReason::Completed,
                )),
            ),
            (
                Dimension::Logic,
                Ok(run_with(
                    vec![finding(Dimension::Logic)],
                    AgentExitReason::Completed,
                )),
            ),
        ];
        let (findings, stats) = collect_agent_results(results, &mut warnings, &mut incomplete);
        assert_eq!(findings.len(), 2);
        assert_eq!(stats.llm_requests, 2);
        assert_eq!(stats.findings_reported, 2);
        assert!(warnings.is_empty());
        assert!(!incomplete);
    }

    #[test]
    fn collect_marks_incomplete_on_timeout_and_failed() {
        let mut warnings = Vec::new();
        let mut incomplete = false;
        let results = vec![
            (
                Dimension::Security,
                Ok(run_with(vec![], AgentExitReason::TimedOut)),
            ),
            (Dimension::Logic, Err(anyhow::anyhow!("boom"))),
        ];
        let (findings, _) = collect_agent_results(results, &mut warnings, &mut incomplete);
        assert!(findings.is_empty());
        assert!(incomplete);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|w| w.kind == "timed_out"));
        assert!(warnings.iter().any(|w| w.kind == "failed"));
    }

    #[test]
    fn collect_keeps_partial_findings_on_failure() {
        let f = finding(Dimension::Security);
        let mut warnings = Vec::new();
        let mut incomplete = false;
        let results = vec![(
            Dimension::Security,
            Ok(run_with(vec![f.clone()], AgentExitReason::TimedOut)),
        )];
        let (findings, _) = collect_agent_results(results, &mut warnings, &mut incomplete);
        assert_eq!(findings.len(), 1);
        assert!(incomplete);
    }

    #[test]
    fn warning_for_exit_maps_reasons() {
        let completed = AgentRun {
            findings: vec![],
            stats: AgentStats::default(),
            exit_reason: AgentExitReason::Completed,
            error_detail: None,
        };
        assert!(warning_for_exit(Dimension::Security, &completed).is_none());

        let timed = AgentRun {
            findings: vec![],
            stats: AgentStats::default(),
            exit_reason: AgentExitReason::TimedOut,
            error_detail: None,
        };
        let w = warning_for_exit(Dimension::Perf, &timed).unwrap();
        assert_eq!(w.kind, "timed_out");
        assert_eq!(w.dimension, "perf");

        let auth = AgentRun {
            findings: vec![],
            stats: AgentStats::default(),
            exit_reason: AgentExitReason::AuthFailed,
            error_detail: Some("401".into()),
        };
        let w = warning_for_exit(Dimension::Security, &auth).unwrap();
        assert_eq!(w.kind, "auth_failed");
        assert!(w.message.contains("401"));

        let ctx = AgentRun {
            findings: vec![],
            stats: AgentStats::default(),
            exit_reason: AgentExitReason::ContextOverflow,
            error_detail: None,
        };
        assert_eq!(
            warning_for_exit(Dimension::Logic, &ctx).unwrap().kind,
            "incomplete"
        );
    }

    #[test]
    fn warning_for_exit_max_rounds_returns_none() {
        let run = AgentRun {
            findings: vec![],
            stats: AgentStats::default(),
            exit_reason: AgentExitReason::MaxRounds,
            error_detail: None,
        };
        assert!(warning_for_exit(Dimension::Logic, &run).is_none());
    }

    #[test]
    fn collect_agent_results_sums_loop_guarded_and_error_detail() {
        let mut run = run_with(
            vec![finding(Dimension::Security)],
            AgentExitReason::RequestFailed,
        );
        run.stats.loop_guarded = 3;
        run.error_detail = Some("timeout".into());

        let mut warnings = Vec::new();
        let mut incomplete = false;
        let (_, stats) = collect_agent_results(
            vec![(Dimension::Security, Ok(run))],
            &mut warnings,
            &mut incomplete,
        );
        assert!(incomplete);
        assert_eq!(stats.loop_guarded, 3);
        assert!(warnings.iter().any(|w| w.message.contains("timeout")));
    }

    #[test]
    fn review_options_workspace_and_commit() {
        let ws = ReviewOptions::workspace(vec![Dimension::Logic]);
        assert!(matches!(ws.mode, DiffMode::Workspace));

        let commit = ReviewOptions::new(DiffMode::Commit("abc".into()), vec![Dimension::Security]);
        assert!(matches!(commit.mode, DiffMode::Commit(s) if s == "abc"));
    }

    #[test]
    fn review_options_defaults() {
        let opts = ReviewOptions::new(DiffMode::Commit("abc".into()), Dimension::ALL.to_vec());
        assert!(opts.judge);
        assert_eq!(opts.samples, 1);
        assert!(!opts.exec_verify);
        assert_eq!(opts.judge_concurrency, 4);
        assert_eq!(opts.fanout_concurrency, 6);
        assert!(opts.intent.is_none());
        assert!(!opts.verbose);
        assert_eq!(opts.profile, ReviewProfile::Standard);

        let ws = ReviewOptions::workspace(Dimension::ALL.to_vec());
        assert!(matches!(ws.mode, DiffMode::Workspace));
        assert_eq!(ws.profile, ReviewProfile::Standard);
    }

    #[test]
    fn security_deep_defaults_are_security_only_with_higher_samples() {
        let deep = ReviewOptions::security_deep(DiffMode::Workspace);
        let standard = ReviewOptions::workspace(Dimension::ALL.to_vec());

        assert_eq!(deep.profile, ReviewProfile::Deep);
        assert!(deep.profile.is_deep());
        assert_eq!(deep.dimensions, vec![Dimension::Security]);
        assert_eq!(deep.samples, DEEP_DEFAULT_SAMPLES);
        assert!(deep.samples > standard.samples);
        assert!(deep.gate.fail_on_incomplete);

        // Standard multi-dim path is unchanged: four defect dims, samples=1, standard profile.
        assert_eq!(standard.dimensions, Dimension::ALL.to_vec());
        assert_eq!(standard.samples, 1);
        assert!(!standard.profile.is_deep());
        assert_eq!(ReviewProfile::Deep.as_str(), "deep");
        assert_eq!(ReviewProfile::Standard.as_str(), "standard");
    }

    #[test]
    fn deep_incomplete_policy_never_passes_empty_incomplete_outcome() {
        // Simulate deep incomplete with zero findings after gate → must not PASS.
        let decision = apply_gate(&mut [], &GateConfig::default());
        assert_eq!(decision, GateDecision::Pass);
        let deep_forced = apply_incomplete_policy(decision, true, true);
        assert_eq!(deep_forced, GateDecision::Warn);

        // Complete empty findings remain PASS.
        assert_eq!(
            apply_incomplete_policy(GateDecision::Pass, false, true),
            GateDecision::Pass
        );
    }

    #[test]
    fn agent_stats_aggregate_usage_and_tools() {
        let mut run1 = run_with(
            vec![finding(Dimension::Security)],
            AgentExitReason::Completed,
        );
        run1.stats.usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 0,
        };
        run1.stats.tool_counts.insert("read_file".into(), 1);
        let mut run2 = run_with(vec![finding(Dimension::Logic)], AgentExitReason::Completed);
        run2.stats.usage = Usage {
            input_tokens: 8,
            output_tokens: 4,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        };
        run2.stats.tool_counts.insert("read_file".into(), 2);

        let mut warnings = Vec::new();
        let mut incomplete = false;
        let (_, stats) = collect_agent_results(
            vec![
                (Dimension::Security, Ok(run1)),
                (Dimension::Logic, Ok(run2)),
            ],
            &mut warnings,
            &mut incomplete,
        );

        assert_eq!(stats.usage.input_tokens, 18);
        assert_eq!(stats.usage.output_tokens, 9);
        assert_eq!(stats.usage.cache_read_input_tokens, 2);
        assert_eq!(stats.tool_counts["read_file"], 3);
    }
}
