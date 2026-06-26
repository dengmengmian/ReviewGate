//! 审查编排：把 diff、Agent、工具、重定位串成一次完整审查。
//!
//! M1.10 跑单/多维度（顺序）。M2.2 会改为多维度并行，M2.3 加证伪 Judge，
//! M2.5 加闸口。CLI 只调用本模块，保持薄。

mod dedup;
mod rules;
mod units;

pub use dedup::dedupe;
pub use rules::{build_rules_section, build_rules_section_with_warnings};
pub use units::{plan_units, ReviewUnit};

use crate::agent::{
    build_user_prompt, run_agent_with_stats, AgentConfig, AgentExitReason, AgentStats,
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

/// 单文件/总行数上限，避免请求过大。
const MAX_FILE_LINES: usize = 500;
const MAX_TOTAL_LINES: usize = 2500;
const HUNK_CONTEXT_LINES: usize = 80;

/// 渲染指定文件子集的完整新版本（带行号，按上限截断）。`file_indices` 为 `diff.files` 下标。
async fn render_changed_files(
    diff: &Diff,
    file_indices: &[usize],
    repo_root: &Path,
    new_ref: &Option<String>,
) -> String {
    let mut out = String::new();
    let mut budget = MAX_TOTAL_LINES;
    for &fi in file_indices {
        let f = &diff.files[fi];
        let Some(path) = f.new_path.as_deref() else {
            continue; // 已删除文件跳过
        };
        if f.binary || budget == 0 {
            continue;
        }
        let content = match new_ref {
            Some(r) => diff::git::git(&["show", &format!("{r}:{path}")]).await.ok(),
            None => tokio::fs::read_to_string(repo_root.join(path)).await.ok(),
        };
        let Some(content) = content else { continue };
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();
        let selected = hunk_context_line_numbers(f, total);
        let selected = selected
            .into_iter()
            .take(MAX_FILE_LINES.min(budget))
            .collect::<Vec<_>>();
        budget -= selected.len();
        out.push_str(&format!("### {path}\n```\n"));
        let mut prev = None;
        for line_no in &selected {
            if prev.is_some_and(|p| *line_no > p + 1) {
                out.push_str("  ...\n");
            }
            let idx = line_no - 1;
            out.push_str(&format!("{:>5} {}\n", line_no, all_lines[idx]));
            prev = Some(*line_no);
        }
        if selected.len() < total {
            out.push_str(&format!(
                "…（共 {total} 行，已按 hunk 周边截取 {} 行，需要更多用 read_file）\n",
                selected.len()
            ));
        }
        out.push_str("```\n\n");
    }
    out
}

fn hunk_context_line_numbers(file: &crate::diff::FileDiff, total: usize) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    if file.hunks.is_empty() {
        return (1..=total).collect();
    }
    let mut ranges = Vec::new();
    for h in &file.hunks {
        let mut nums = h.lines.iter().filter_map(|l| l.new_lineno);
        let first = nums.next().unwrap_or(h.new_start).max(1) as usize;
        let last = h
            .lines
            .iter()
            .filter_map(|l| l.new_lineno)
            .max()
            .unwrap_or_else(|| h.new_start.saturating_add(h.new_count).saturating_sub(1))
            .max(1) as usize;
        let start = first.saturating_sub(HUNK_CONTEXT_LINES).max(1);
        let end = last.saturating_add(HUNK_CONTEXT_LINES).min(total);
        ranges.push((start, end));
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end + 1 {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
        .into_iter()
        .flat_map(|(start, end)| start..=end)
        .collect()
}

/// 该模式下「新版本」内容的来源 ref。
fn new_ref_for(mode: &DiffMode) -> Option<String> {
    match mode {
        DiffMode::Workspace => None,
        DiffMode::Commit(c) => Some(c.clone()),
        DiffMode::Range { to, .. } => Some(to.clone()),
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
    let samples = opts.samples.max(1);

    // 输入预算 → 把 diff 切成审查单元（正常 PR = 1 个单元，零退化）。
    let budget = cfg
        .active_provider()
        .map(|p| p.max_input_tokens())
        .unwrap_or(DEFAULT_MAX_INPUT_TOKENS) as usize;
    let units = plan_units(&diff, budget);
    if opts.verbose && units.len() > 1 {
        eprintln!(
            "  [units] diff 超输入预算（{budget} tok），切成 {} 个审查单元",
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
    let mut unit_prompts: Vec<Option<String>> = Vec::with_capacity(units.len());
    for (ui, unit) in units.iter().enumerate() {
        let full = build_unit_prompt(
            &diff,
            &unit.files,
            true,
            Path::new(&root),
            &new_ref,
            &rules_body,
        )
        .await;
        if estimate_tokens(&full) <= budget {
            unit_prompts.push(Some(full));
            continue;
        }
        let diff_only = build_unit_prompt(
            &diff,
            &unit.files,
            false,
            Path::new(&root),
            &new_ref,
            &rules_body,
        )
        .await;
        if estimate_tokens(&diff_only) <= budget {
            unit_prompts.push(Some(diff_only));
            continue;
        }
        // 单文件 diff 自身就超预算，无法再切 → 跳过并标记未审完（绝不静默放行）。
        incomplete = true;
        let label = unit
            .files
            .first()
            .map(|&i| diff.files[i].path().to_string())
            .unwrap_or_else(|| format!("unit{ui}"));
        eprintln!(
            "⚠ 文件 [{label}] diff 超输入预算（约 {} tok），已跳过未审",
            unit.est_tokens
        );
        warnings.push(ReviewWarning {
            dimension: format!("unit:{label}"),
            kind: "oversized",
            message: format!(
                "该文件 diff 超出输入预算（约 {} tok > {budget}），已跳过未审；请拆分改动或调大 max_input_tokens",
                unit.est_tokens
            ),
        });
        unit_prompts.push(None);
    }

    // fan-out：(单元 × 维度 × 样本) 并行。`labels` 与 `results` 一一对应，用于回填告警维度。
    let mut labels: Vec<Dimension> = Vec::new();
    let mut tasks = Vec::new();
    for prompt_opt in unit_prompts.iter() {
        let Some(prompt) = prompt_opt else { continue };
        for d in &dims {
            for _ in 0..samples {
                let mut agent_cfg = AgentConfig::for_dimension(*d);
                agent_cfg.verbose = opts.verbose;
                // 超时交给 Agent 内部"每轮检查、优雅收尾"，而非硬 cancel——保住已上报的发现。
                agent_cfg.timeout = opts.timeout;
                // 发送前预检预算：确定性避免撞 provider 的 context-length 上限。
                agent_cfg.max_input_tokens = Some(budget);
                let prompt = prompt.clone();
                let reg = &reg;
                let ctx = &ctx;
                labels.push(*d);
                tasks.push(async move {
                    run_agent_with_stats(client, reg, ctx, &agent_cfg, prompt).await
                });
            }
        }
    }
    let results = futures::future::join_all(tasks).await;

    // 每(单元×维度)容错：单个失败只记告警，不影响其它返回部分结果；未审完则标记 incomplete。
    let mut findings = Vec::new();
    let mut agent_stats = AgentStats::default();
    for (dim, r) in labels.iter().zip(results) {
        match r {
            Ok(run) => {
                agent_stats.llm_requests += run.stats.llm_requests;
                agent_stats.tool_calls += run.stats.tool_calls;
                agent_stats.findings_reported += run.stats.findings_reported;
                agent_stats.task_done_calls += run.stats.task_done_calls;
                agent_stats.loop_guarded += run.stats.loop_guarded;
                agent_stats.usage.add(&run.stats.usage);
                for (name, count) in run.stats.tool_counts {
                    *agent_stats.tool_counts.entry(name).or_default() += count;
                }
                match run.exit_reason {
                    AgentExitReason::TimedOut => {
                        incomplete = true;
                        warnings.push(ReviewWarning {
                            dimension: dim.as_str().to_string(),
                            kind: "timed_out",
                            message: "墙钟超时，该维度未审完（已保留其部分发现）".into(),
                        });
                    }
                    AgentExitReason::RequestFailed => {
                        incomplete = true;
                        warnings.push(ReviewWarning {
                            dimension: dim.as_str().to_string(),
                            kind: "incomplete",
                            message: "LLM 请求失败（可能上下文超限），该维度未审完".into(),
                        });
                    }
                    AgentExitReason::ContextOverflow => {
                        incomplete = true;
                        warnings.push(ReviewWarning {
                            dimension: dim.as_str().to_string(),
                            kind: "incomplete",
                            message: "上下文超输入预算，发送前预检提前收尾，该维度未审完".into(),
                        });
                    }
                    AgentExitReason::Completed | AgentExitReason::MaxRounds => {}
                }
                findings.extend(run.findings);
            }
            Err(e) => {
                incomplete = true;
                warnings.push(ReviewWarning {
                    dimension: dim.as_str().to_string(),
                    kind: "failed",
                    message: e.to_string(),
                });
                eprintln!("⚠ 维度 [{}] 审查失败（已跳过）：{e}", dim.as_str());
            }
        }
    }
    // 质量闸口不能把"未审完"误读成"通过"：未审完的维度/单元已保留其部分发现，但仍要醒目提示。
    if incomplete {
        eprintln!(
            "⚠ 本次审查未完整（超时/请求失败/上下文超限/超大文件跳过）：结果可能不完整。\
             如需完整结论请放宽 --timeout、调大 max_input_tokens 或拆分改动后重跑。"
        );
    }
    if opts.verbose {
        eprintln!(
            "  [agents] 汇总：LLM {} 次 · 工具 {} 次（{}）· 循环熔断 {} 次；{}",
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
        eprintln!("  [judge] 已跳过（--no-judge）");
    }

    if opts.verbose {
        let mut total_usage = agent_stats.usage.clone();
        total_usage.add(&judge_stats.usage);
        eprintln!(
            "  [review] 总计：LLM {} 次 · 工具 {} 次（Agent: {} / Judge: {}）；{}",
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

/// 构造单个审查单元的 user prompt：本单元各文件的 diff（+ 可选文件全文上下文）+ 业务规则正文。
async fn build_unit_prompt(
    diff: &Diff,
    file_indices: &[usize],
    include_ctx: bool,
    root: &Path,
    new_ref: &Option<String>,
    rules_body: &str,
) -> String {
    let mut diff_text = String::new();
    for &fi in file_indices {
        diff_text.push_str(&diff.files[fi].render_for_prompt());
        diff_text.push('\n');
    }
    let mut prompt = build_user_prompt(&diff_text);
    if include_ctx {
        let files_ctx = render_changed_files(diff, file_indices, root, new_ref).await;
        if !files_ctx.is_empty() {
            prompt.push_str("\n\n## 改动文件的完整新版本（已附上，无需再逐个读取）\n\n");
            prompt.push_str(&files_ctx);
        }
    }
    if !rules_body.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(rules_body);
    }
    prompt
}

/// 复合排序：未过滤优先 → 严重度降（High→Low）→ 置信度降。
fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.filtered
            .cmp(&b.filtered)
            .then(b.severity.cmp(&a.severity))
            .then(
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
}

/// 跨维度一致性加分：被 N(≥2) 个不同维度独立标记的发现，置信度按
/// 每多一个维度 +0.05、最多 +0.15 上调，并封顶 0.99（不冒充确定）。
fn boost_cross_dimension_agreement(findings: &mut [Finding]) {
    for f in findings.iter_mut() {
        if f.agreed_dimensions >= 2 {
            let bonus = (0.05 * (f.agreed_dimensions - 1) as f32).min(0.15);
            f.confidence = (f.confidence + bonus).min(0.99);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line, LineKind};
    use crate::model::{Dimension, Severity};

    fn finding(conf: f32, agreed: u8) -> Finding {
        Finding {
            dimension: Dimension::Logic,
            confidence: conf,
            severity: Severity::High,
            path: "a.rs".into(),
            start_line: 1,
            end_line: 1,
            message: "m".into(),
            existing_code: "x".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: crate::model::Reachability::default(),
            filtered: false,
            agreed_dimensions: agreed,
        }
    }

    #[test]
    fn sort_orders_unfiltered_severity_confidence() {
        let mut fs = vec![
            finding(0.95, 1), // 默认 High，未过滤
            {
                let mut f = finding(0.99, 1);
                f.filtered = true; // 高置信但被过滤 → 应排最后
                f
            },
            {
                let mut f = finding(0.70, 1);
                f.severity = Severity::Low; // 低危未过滤
                f
            },
        ];
        sort_findings(&mut fs);
        // 未过滤的 High(0.95) 第一，未过滤的 Low(0.70) 第二，被过滤的(0.99) 垫底。
        assert_eq!(fs[0].confidence, 0.95);
        assert_eq!(fs[1].confidence, 0.70);
        assert!(fs[2].filtered);
    }

    #[test]
    fn agreement_boost_scales_and_caps() {
        let mut fs = vec![
            finding(0.6, 1),  // 单维度：不加分
            finding(0.6, 2),  // +0.05
            finding(0.6, 4),  // +0.15（封顶）
            finding(0.95, 5), // 加分后封顶 0.99
        ];
        boost_cross_dimension_agreement(&mut fs);
        assert!((fs[0].confidence - 0.6).abs() < 1e-6);
        assert!((fs[1].confidence - 0.65).abs() < 1e-6);
        assert!((fs[2].confidence - 0.75).abs() < 1e-6);
        assert!((fs[3].confidence - 0.99).abs() < 1e-6);
    }

    #[tokio::test]
    async fn changed_file_context_is_hunk_window_not_file_prefix() {
        let root = std::env::temp_dir().join(format!("rg_hunk_window_{}", std::process::id()));
        let src = root.join("src");
        tokio::fs::create_dir_all(&src).await.unwrap();
        let content = (1..=220)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(src.join("large.rs"), content)
            .await
            .unwrap();

        let diff = Diff {
            files: vec![FileDiff {
                old_path: Some("src/large.rs".into()),
                new_path: Some("src/large.rs".into()),
                status: FileStatus::Modified,
                binary: false,
                hunks: vec![Hunk {
                    old_start: 149,
                    old_count: 3,
                    new_start: 149,
                    new_count: 3,
                    section: String::new(),
                    lines: vec![
                        Line {
                            kind: LineKind::Context,
                            content: "line 149".into(),
                            old_lineno: Some(149),
                            new_lineno: Some(149),
                        },
                        Line {
                            kind: LineKind::Added,
                            content: "line 150".into(),
                            old_lineno: None,
                            new_lineno: Some(150),
                        },
                    ],
                }],
            }],
        };

        let all: Vec<usize> = (0..diff.files.len()).collect();
        let rendered = render_changed_files(&diff, &all, &root, &None).await;

        assert!(rendered.contains("  150 line 150"));
        assert!(rendered.contains("   69 line 69"));
        assert!(!rendered.contains("    1 line 1"));
        tokio::fs::remove_dir_all(&root).await.ok();
    }
}
