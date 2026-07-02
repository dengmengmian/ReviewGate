//! 审查编排：把 diff、Agent、工具、重定位串成一次完整审查。
//!
//! M1.10 跑单/多维度（顺序）。M2.2 会改为多维度并行，M2.3 加证伪 Judge，
//! M2.5 加闸口。CLI 只调用本模块，保持薄。

mod aggregate;
mod context;
mod dedup;
mod intent;
mod prefetch;
mod rules;
mod units;

pub use dedup::dedupe;
pub use rules::{build_rules_section, build_rules_section_with_warnings};
pub use units::{plan_units, ReviewUnit};

use aggregate::{boost_cross_dimension_agreement, sort_findings};
use context::{build_unit_prompt, new_ref_for};

use crate::agent::{
    dimension_focus_block, run_agent_with_stats, shared_system_prompt, AgentConfig,
    AgentExitReason, AgentRun, AgentStats,
};
use crate::config::{Config, GateConfig, DEFAULT_MAX_INPUT_TOKENS};
use crate::diff::{self, Diff, DiffMode};
use crate::gate::{apply_gate, GateDecision};
use crate::judge::{judge_all_with_stats_limited, JudgeStats};
use crate::llm::{build_client, estimate_tokens, LlmClient};
use crate::model::{Dimension, Finding, Usage};
use crate::relocate::relocate_all;
use crate::tool::{readonly_tools, ToolContext, ToolRegistry};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

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
        }
    }

    pub fn workspace(dimensions: Vec<Dimension>) -> Self {
        Self::new(DiffMode::Workspace, dimensions)
    }
}

/// 维度/单元未审完的告警。让消费方不把"没审完"误读成"通过"。
#[derive(Debug, Clone, Serialize)]
pub struct ReviewWarning {
    pub dimension: String,
    /// `timed_out` | `failed` | `incomplete` | `oversized` | `rules_unavailable`
    pub kind: &'static str,
    pub message: String,
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
    if diff.files.is_empty() {
        return Ok(ReviewOutcome {
            findings: Vec::new(),
            files_changed: 0,
            decision: GateDecision::Pass,
            warnings: Vec::new(),
            incomplete: false,
            usage: Usage::default(),
        });
    }

    let new_ref = new_ref_for(&opts.mode);
    let mut warnings: Vec<ReviewWarning> = Vec::new();
    let mut incomplete = false;

    // 项目规则正文：注入每个单元 prompt 的末尾（跨维度可缓存）。
    let rules_section = build_rules_section_with_warnings(&cfg.business, &diff, Path::new(&root));
    let rules_body = rules_section.body.clone();
    for message in rules_section.warnings {
        warnings.push(ReviewWarning {
            dimension: Dimension::Business.as_str().to_string(),
            kind: "rules_unavailable",
            message,
        });
    }

    // 配置了任一规则来源（inline / rules_dir / skills_dir）就自动并入 Business 维度。
    let has_business_rules = !cfg.business.rules.is_empty()
        || cfg.business.rules_dir.is_some()
        || cfg.business.skills_dir.is_some();
    let mut dims = opts.dimensions.clone();
    if has_business_rules && !dims.contains(&Dimension::Business) {
        dims.push(Dimension::Business);
    }
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
            .map(|d| estimate_tokens(&dimension_focus_block(*d)))
            .max()
            .unwrap_or(0)
        + 256;
    let plan_budget = budget.saturating_sub(overhead).max(512);
    let units = plan_units(&diff, plan_budget);
    // 多单元（大 PR）本就庞大：不再叠采样，避免 单元×维度×样本 的成本放大。
    // 多采样只在单单元（正常 PR）上用于提升 flaky 漏报（如 SSRF）的召回稳定性。
    let samples = if units.len() > 1 {
        1
    } else {
        opts.samples.max(1)
    };
    if opts.verbose && units.len() > 1 {
        eprintln!(
            "  [units] diff exceeds input budget ({budget} tok); split into {} review units; samples forced to 1 (cost control)",
            units.len()
        );
    }

    let mut ctx = ToolContext::with_treesitter_index(diff.clone(), root.clone(), new_ref.clone());
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
                let prompt = prompt.clone();
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
            warnings.push(ReviewWarning {
                dimension: Dimension::Intent.as_str().to_string(),
                kind: "incomplete",
                message: "intent review did not finish (timeout/context overflow); the result may be partial".into(),
            });
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

    // 闸口：标记过滤项 + 判定。复合排序：未过滤优先 → 严重度降 → 置信度降。
    let mut decision = apply_gate(&mut findings, &opts.gate);
    // 未审完不变量：有单元未审完且 fail_on_incomplete 时，永不 PASS（至少 WARN；有 BLOCK 仍 BLOCK）。
    if incomplete && opts.gate.fail_on_incomplete && decision == GateDecision::Pass {
        decision = GateDecision::Warn;
    }
    // 已满足(met)的验收项在闸口之后并入：它们是信息项，只供验收清单展示，不影响判定。
    findings.append(&mut intent_met);
    sort_findings(&mut findings);

    let mut usage = agent_stats.usage.clone();
    usage.add(&judge_stats.usage);

    Ok(ReviewOutcome {
        findings,
        files_changed: diff.files.len(),
        decision,
        warnings,
        incomplete,
        usage,
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
) -> Vec<Option<String>> {
    let mut unit_prompts: Vec<Option<String>> = Vec::with_capacity(units.len());
    for (ui, unit) in units.iter().enumerate() {
        let prefetched = prefetch::render_prefetch(index, diff, &unit.files).await;
        let with_prefetch = |mut p: String| {
            if !prefetched.is_empty() {
                p.push_str("\n\n");
                p.push_str(&prefetched);
            }
            p
        };
        let full = build_unit_prompt(diff, &unit.files, true, root, new_ref, rules_body).await;
        let full_pf = with_prefetch(full.clone());
        if estimate_tokens(&full_pf) + overhead <= budget {
            unit_prompts.push(Some(full_pf));
            continue;
        }
        if estimate_tokens(&full) + overhead <= budget {
            unit_prompts.push(Some(full));
            continue;
        }
        let diff_only =
            build_unit_prompt(diff, &unit.files, false, root, new_ref, rules_body).await;
        let diff_only_pf = with_prefetch(diff_only.clone());
        if estimate_tokens(&diff_only_pf) + overhead <= budget {
            unit_prompts.push(Some(diff_only_pf));
            continue;
        }
        if estimate_tokens(&diff_only) + overhead <= budget {
            unit_prompts.push(Some(diff_only));
            continue;
        }
        // 单文件 diff 自身就超预算，无法再切 → 跳过并标记未审完（绝不静默放行）。
        *incomplete = true;
        let label = unit
            .files
            .first()
            .map(|&i| diff.files[i].path().to_string())
            .unwrap_or_else(|| format!("unit{ui}"));
        eprintln!(
            "! file [{label}] diff exceeds input budget (~{} tok); skipped (not reviewed)",
            unit.est_tokens
        );
        warnings.push(ReviewWarning {
            dimension: format!("unit:{label}"),
            kind: "oversized",
            message: format!(
                "this file's diff exceeds the input budget (~{} tok > {budget}); skipped (not reviewed); split the change or raise max_input_tokens",
                unit.est_tokens
            ),
        });
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
                warnings.push(ReviewWarning {
                    dimension: dim.as_str().to_string(),
                    kind: "failed",
                    message: e.to_string(),
                });
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
        AgentExitReason::TimedOut => Some(ReviewWarning {
            dimension: dim.as_str().to_string(),
            kind: "timed_out",
            message: "wall-clock timeout; this dimension did not finish (its partial findings are kept)".into(),
        }),
        AgentExitReason::AuthFailed => Some(ReviewWarning {
            dimension: dim.as_str().to_string(),
            kind: "auth_failed",
            message: format!(
                "LLM authentication failed — check the API key for the active provider (api_key in the config, or REVIEWGATE_API_KEY){}; this dimension did not finish",
                detail()
            ),
        }),
        AgentExitReason::RequestFailed => Some(ReviewWarning {
            dimension: dim.as_str().to_string(),
            kind: "incomplete",
            message: format!("LLM request failed{}; this dimension did not finish", detail()),
        }),
        AgentExitReason::ContextOverflow => Some(ReviewWarning {
            dimension: dim.as_str().to_string(),
            kind: "incomplete",
            message: "context exceeded the input budget; pre-send check wrapped up early; this dimension did not finish".into(),
        }),
        AgentExitReason::Completed | AgentExitReason::MaxRounds => None,
    }
}
