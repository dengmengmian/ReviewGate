//! 意图 / 技术评审：独立的**整体性** Agent，审「实现 vs 意图」。
//!
//! 与按 (单元 × 维度) fan-out 的缺陷评审不同——它整体跑一次、从 diff 出发**主动跨文件探索**
//! （调用方、契约、相关模块、测试），判断实现是否完整、正确地满足传入的意图。diff 是起点不是边界。

use crate::agent::{intent_system_prompt, run_agent_with_stats, AgentConfig, AgentExitReason};
use crate::diff::Diff;
use crate::llm::LlmClient;
use crate::model::{Dimension, Finding};
use crate::progress::Progress;
use crate::tool::{ToolContext, ToolRegistry};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct IntentReview {
    pub findings: Vec<Finding>,
    pub incomplete: bool,
}

/// 整体审一次「实现 vs 意图」。`intent` = 本次改动的意图 / 需求 / 验收标准。
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_intent_review(
    client: &dyn LlmClient,
    reg: &ToolRegistry,
    ctx: &ToolContext,
    diff: &Diff,
    intent: &str,
    budget: usize,
    verbose: bool,
    timeout: Option<Duration>,
    progress: Option<Arc<Progress>>,
) -> IntentReview {
    let mut cfg = AgentConfig::for_dimension(Dimension::Intent);
    cfg.system_prompt = intent_system_prompt(); // 探索向系统提示，覆盖默认 shared（缺陷向）
    cfg.verbose = verbose;
    cfg.timeout = timeout;
    cfg.progress = progress;
    cfg.max_input_tokens = Some(budget);
    cfg.max_rounds = 16; // 更深的开放式探索

    let diff_body: String = diff
        .files
        .iter()
        .map(|f| f.render_for_prompt())
        .collect::<Vec<_>>()
        .join("\n");

    let user_prompt = format!(
        "## 本次改动的意图 / 需求 / 验收标准\n\n{intent}\n\n## 本次改动 (diff)\n\n{diff_body}\n\n\
请评审「实现 vs 意图」：从 diff 出发，按需用工具深入其它文件、调用方、契约与测试，\
判断是否完整、正确地实现了上述意图。",
        intent = intent.trim(),
    );

    match run_agent_with_stats(client, reg, ctx, &cfg, user_prompt).await {
        Ok(run) => {
            if verbose {
                eprintln!(
                    "  [intent] 意图评审：LLM {} 次 · 工具 {} 次 · 发现 {} 条",
                    run.stats.llm_requests,
                    run.stats.tool_calls,
                    run.findings.len()
                );
            }
            IntentReview {
                findings: run.findings,
                incomplete: matches!(
                    run.exit_reason,
                    AgentExitReason::TimedOut
                        | AgentExitReason::RequestFailed
                        | AgentExitReason::ContextOverflow
                ),
            }
        }
        Err(e) => {
            eprintln!("⚠ 意图评审失败（已跳过）：{e}");
            IntentReview {
                findings: Vec::new(),
                incomplete: true,
            }
        }
    }
}
