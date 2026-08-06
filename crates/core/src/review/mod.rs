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
pub mod session;
/// 阶段函数与运行上下文。`crate::security` 复用同一组阶段组装自己的序列，
/// 故对 crate 内可见；不对外暴露（阶段边界是内部实现细节）。
pub(crate) mod stages;
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
pub use session::{FindingRecord, FindingSession, FindingStatus};
pub use suppress::fingerprint;
pub use units::{plan_units, summarize_units, ReviewUnit, UnitJobSummary, UnitPlanSummary};

use aggregate::{boost_cross_dimension_agreement, sort_findings};
use context::{build_unit_prompt, new_ref_for};

use crate::agent::{
    dimension_focus_block_with_deep, run_agent_with_stats, shared_system_prompt, AgentConfig,
    AgentExitReason, AgentRun, AgentStats,
};
use crate::config::{Config, GateConfig, DEFAULT_MAX_INPUT_TOKENS};
use crate::diff::{self, Diff, DiffMode, ExcludedFile, Excluder};
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
    /// Security deep review: security-only, sink-driven focus, deterministic
    /// secret precheck, incomplete never PASS. Round count comes from the
    /// saturating discovery loop in [`crate::security`], not from sampling.
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
    /// PR/MR 上**已有的评审讨论**（人类 reviewer 已经提过的点）。注入 prompt 作为上下文，
    /// 让模型不要把别人已经指出的问题当新发现重复报一遍。
    ///
    /// 只做**上下文注入**，不做自动折叠——按文本相似度去隐藏发现，等于给闸口开一个
    /// 「有人评论过就不算问题」的后门。None = 不注入（零退化）。
    pub pr_discussion: Option<String>,
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
            pr_discussion: None,
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
    /// - profile = Deep (sink-driven focus, secret precheck, fail-incomplete hard)
    ///
    /// 不设 `samples`：security 线的轮数由饱和式 discovery 决定
    /// （见 [`crate::security`]），固定采样在那条线上没有意义。
    pub fn security_deep(mode: DiffMode) -> Self {
        let mut opts = Self::new(mode, vec![Dimension::Security]);
        opts.profile = ReviewProfile::Deep;
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
    /// 被排除规则挡在 LLM 之前的文件（带原因）。永远如实回传——闸口不能悄悄少审。
    pub excluded: Vec<ExcludedFile>,
    /// 本次审查的范围描述（如 `working tree vs HEAD`、`main...HEAD`、`since last review (…)`）。
    /// PASS 只对这个范围成立。
    pub scope: String,
    /// 本次 diff 上可挂 PR 行内评论的锚点。`None` = 未知（不做校验，保持旧行为），
    /// 与 `Some(空)`（确实无处可挂）语义不同。
    pub diff_anchors: Option<crate::diff::DiffAnchors>,
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
            excluded: Vec::new(),
            scope: String::new(),
            diff_anchors: None,
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
    // 准备阶段：diff 采集 / 排除 / 规则装配 / 单元规划 / 成本守卫 / 工具上下文 / prompt 预构造。
    // 空 diff 与 --estimate-only 在这里直接产出终态结果。
    let mut c = match stages::prepare(cfg, opts, client, None).await? {
        stages::Prepared::Early(outcome) => return Ok(*outcome),
        stages::Prepared::Ready(c) => *c,
    };

    stages::discover_and_intent(&mut c, client).await;
    stages::secrets(&mut c);
    stages::report_incomplete(&c);
    stages::relocate_dedupe(&mut c).await;
    stages::merge_intent(&mut c);
    stages::judge(&mut c, client).await;
    stages::boost(&mut c);
    stages::store_incremental(&mut c);
    stages::suppress(&mut c);
    stages::gate(&mut c);
    Ok(stages::finalize(c))
}

/// 把不可信文本围进带哨兵的围栏，让模型能清楚看到"数据到哪里结束"。
/// 同时剥掉内容里出现的哨兵串，防止伪造闭合围栏后接着写指令。
fn fence_untrusted(text: &str) -> String {
    const FENCE: &str = "===== UNTRUSTED PR DISCUSSION =====";
    const END: &str = "===== END UNTRUSTED PR DISCUSSION =====";
    let cleaned = text.replace(FENCE, "[fence]").replace(END, "[fence]");
    format!("{FENCE}\n{cleaned}\n{END}")
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
    fn security_deep_defaults_are_security_only_and_never_pass_when_incomplete() {
        let deep = ReviewOptions::security_deep(DiffMode::Workspace);
        let standard = ReviewOptions::workspace(Dimension::ALL.to_vec());

        assert_eq!(deep.profile, ReviewProfile::Deep);
        assert!(deep.profile.is_deep());
        assert_eq!(deep.dimensions, vec![Dimension::Security]);
        assert!(deep.gate.fail_on_incomplete);
        // 轮数由饱和式 discovery 决定，不再靠固定采样。
        assert_eq!(deep.samples, standard.samples);

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
