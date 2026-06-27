//! 意图 / 技术评审：独立的**整体性** Agent，审「实现 vs 意图」。
//!
//! 结构化强制：把意图拆成 N 条验收标准（C1..CN），要求逐条 verdict；评审跑完后，
//! 未被覆盖的标准**兜底填 `Unknown`**，保证验收清单覆盖每一条（不再出现空清单）。
//! 仍从 diff 出发主动跨文件探索（调用方、契约、测试）——diff 是起点不是边界。

use crate::agent::{intent_system_prompt, run_agent_with_stats, AgentConfig, AgentExitReason};
use crate::diff::Diff;
use crate::llm::LlmClient;
use crate::model::{Dimension, Finding, IntentStatus, Reachability, Severity};
use crate::progress::Progress;
use crate::tool::{ToolContext, ToolRegistry};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct IntentReview {
    pub findings: Vec<Finding>,
    pub incomplete: bool,
}

/// 把意图文本拆成离散验收标准。优先识别列表项（`1.` / `1)` / `- ` / `* ` / `(a)` / `#N`）；
/// 无列表标记时退化为「按非空行拆」或「整体一条」。最多 12 条（控 prompt/清单规模）。
pub(super) fn parse_criteria(intent: &str) -> Vec<String> {
    const MAX: usize = 12;
    let mut out: Vec<String> = Vec::new();
    for line in intent.lines() {
        if let Some(rest) = strip_list_marker(line.trim()) {
            let rest = rest.trim();
            if !rest.is_empty() {
                out.push(rest.to_string());
            }
        }
    }
    if out.is_empty() {
        let lines: Vec<&str> = intent
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        if lines.len() > 1 {
            out = lines.into_iter().map(|s| s.to_string()).collect();
        } else {
            let whole = intent.trim();
            if !whole.is_empty() {
                out.push(whole.chars().take(300).collect());
            }
        }
    }
    out.truncate(MAX);
    out
}

/// 识别并剥离行首列表标记；不是列表项返回 None。
fn strip_list_marker(s: &str) -> Option<&str> {
    for m in ["- ", "* ", "• ", "· "] {
        if let Some(r) = s.strip_prefix(m) {
            return Some(r);
        }
    }
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let after = &s[digits..];
        if let Some(r) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            return Some(r);
        }
    }
    if let Some(rest) = s.strip_prefix('(') {
        if let Some(idx) = rest.find(") ") {
            if (1..=3).contains(&idx) {
                return Some(&rest[idx + 2..]);
            }
        }
    }
    None
}

/// 从评审上报的 `criterion` 字段里抽出标准编号 `C<n>` → 0-based 下标（1-based ID）。
fn criterion_index(s: &str) -> Option<usize> {
    let rest = s.trim_start();
    let rest = rest.strip_prefix('C').or_else(|| rest.strip_prefix('c'))?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| n - 1)
}

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
    let criteria = parse_criteria(intent);
    if criteria.is_empty() {
        return IntentReview {
            findings: Vec::new(),
            incomplete: false,
        };
    }

    let mut cfg = AgentConfig::for_dimension(Dimension::Intent);
    cfg.system_prompt = intent_system_prompt(); // 探索向系统提示，覆盖默认 shared（缺陷向）
    cfg.verbose = verbose;
    cfg.timeout = timeout;
    cfg.max_input_tokens = Some(budget);
    cfg.max_rounds = 16;
    cfg.progress = progress;

    let diff_body: String = diff
        .files
        .iter()
        .map(|f| f.render_for_prompt())
        .collect::<Vec<_>>()
        .join("\n");
    let criteria_list: String = criteria
        .iter()
        .enumerate()
        .map(|(i, c)| format!("C{}: {}", i + 1, c))
        .collect::<Vec<_>>()
        .join("\n");

    let user_prompt = format!(
        "## Acceptance criteria for this change\n\nReport **exactly one verdict per criterion** below. \
Set each verdict's `criterion` field to the criterion ID (e.g. `C2`).\n\n{criteria_list}\n\n\
## Original intent (for context)\n\n{intent}\n\n## The change (diff)\n\n{diff_body}\n\n\
Review implementation-vs-intent: start from the diff, use tools as needed to dig into other files, \
callers, contracts, and tests, and report a verdict (met/missing/deviation/breaking) for every Cn above.",
        intent = intent.trim(),
    );

    let (mut findings, incomplete) = match run_agent_with_stats(client, reg, ctx, &cfg, user_prompt)
        .await
    {
        Ok(run) => {
            if verbose {
                eprintln!(
                        "  [intent] intent review: {} LLM calls, {} tool calls, {} verdicts ({} criteria)",
                        run.stats.llm_requests,
                        run.stats.tool_calls,
                        run.findings.len(),
                        criteria.len()
                    );
            }
            let inc = matches!(
                run.exit_reason,
                AgentExitReason::TimedOut
                    | AgentExitReason::RequestFailed
                    | AgentExitReason::ContextOverflow
            );
            (run.findings, inc)
        }
        Err(e) => {
            eprintln!("! intent review failed (skipped): {e}");
            (Vec::new(), true)
        }
    };

    // 结构化强制：把上报的 verdict 归到对应标准；未覆盖的标准兜底填 Unknown，保证清单覆盖每条。
    let mut covered = vec![false; criteria.len()];
    for f in findings.iter_mut() {
        if let Some(idx) = f.criterion.as_deref().and_then(criterion_index) {
            if idx < criteria.len() {
                covered[idx] = true;
                f.criterion = Some(format!("C{}: {}", idx + 1, criteria[idx])); // ID → 完整文本展示
            }
        }
    }
    for (i, c) in criteria.iter().enumerate() {
        if !covered[i] {
            findings.push(Finding {
                dimension: Dimension::Intent,
                confidence: 0.0,
                severity: Severity::Low,
                path: String::new(),
                start_line: 0,
                end_line: 0,
                message: "Not assessed by the reviewer (ran out of budget/rounds, or skipped)."
                    .into(),
                existing_code: String::new(),
                evidence: String::new(),
                suggestion: None,
                suggestion_code: String::new(),
                reachability: Reachability::default(),
                filtered: true,
                agreed_dimensions: 1,
                criterion: Some(format!("C{}: {}", i + 1, c)),
                intent_status: Some(IntentStatus::Unknown),
            });
        }
    }

    // 有验收标准未被实际核对（兜底填了 Unknown）→ 视为未审完：绝不让"没真正核对"伪装成 PASS。
    let incomplete = incomplete || covered.iter().any(|&c| !c);

    IntentReview {
        findings,
        incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_criteria_handles_numbered_bulleted_terse() {
        let c = parse_criteria("## 验收\n1. first\n2. second\n3) third");
        assert_eq!(c, vec!["first", "second", "third"]);
        let c = parse_criteria("- a\n* b\n• c");
        assert_eq!(c, vec!["a", "b", "c"]);
        // 单行 terse commit message → 整体一条
        let c = parse_criteria("fix: support URL object as config.url");
        assert_eq!(c, vec!["fix: support URL object as config.url"]);
        // 全空 → 空
        assert!(parse_criteria("   \n  ").is_empty());
    }

    #[test]
    fn criterion_index_extracts_id() {
        assert_eq!(criterion_index("C2"), Some(1));
        assert_eq!(criterion_index("C10: blah"), Some(9));
        assert_eq!(criterion_index("c1"), Some(0));
        assert_eq!(criterion_index("missing dispatch"), None);
        assert_eq!(criterion_index("C0"), None); // 1-based，C0 非法
    }
}
