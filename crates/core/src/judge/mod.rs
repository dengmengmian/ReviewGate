//! 证伪 Judge。
//!
//! 对每条 Finding 做一次独立验证：默认它**可能是误报**，用工具尝试反驳。
//! 只有证伪失败（问题确实成立）才保留，并以 Judge 给出的置信度覆盖原值。
//! 另有一份「硬排除清单」在调用 LLM 前先剔除明显的误报类别（省成本）。

mod prompt;

use crate::llm::LlmClient;
use crate::model::{Finding, Message, Reachability, StopReason, ToolDef, ToolResult, Usage};
use crate::tool::{ToolContext, ToolRegistry};
use futures::stream::{self, StreamExt};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Judge 轮次上限。默认期望首轮（带证据）即裁决；仅边界情形才用工具升级核实，
/// 故上限设小以截断长尾。
const MAX_ROUNDS: usize = 4;
const DEFAULT_CONCURRENCY: usize = 4;

/// 硬排除：明显应丢弃的发现（无需 LLM）。
pub fn hard_excluded(f: &Finding) -> bool {
    let p = f.path.to_lowercase();
    let is_test = p.contains("/test")
        || p.contains("__tests__")
        || p.ends_with("_test.go")
        || p.ends_with("_test.rs")
        || p.contains(".test.")
        || p.contains(".spec.")
        || p.contains("/tests/");
    // 测试文件里的「性能/规范」问题通常无意义。
    if is_test
        && matches!(
            f.dimension,
            crate::model::Dimension::Perf | crate::model::Dimension::Style
        )
    {
        return true;
    }
    false
}

/// Judge 裁决。
#[derive(Debug, Clone)]
pub struct Verdict {
    pub real: bool,
    pub confidence: f32,
    pub reason: String,
    /// 可达性评估：真问题但当前路径打不到 → `Latent`（闸口不阻断）。
    pub reachability: Reachability,
}

/// Judge 阶段统计，用于定位慢在哪里。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JudgeStats {
    pub candidates: usize,
    pub hard_excluded: usize,
    pub kept: usize,
    pub refuted: usize,
    pub failed_open: usize,
    pub llm_requests: usize,
    pub tool_calls: usize,
    pub tool_counts: BTreeMap<String, usize>,
    /// 累计 token 用量（含缓存命中）。
    pub usage: Usage,
}

impl JudgeStats {
    fn record_tool(&mut self, name: &str) {
        self.tool_calls += 1;
        *self.tool_counts.entry(name.to_string()).or_default() += 1;
    }

    pub fn tool_summary(&self) -> String {
        if self.tool_counts.is_empty() {
            return "无工具调用".into();
        }
        self.tool_counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 对一批 finding 逐条证伪（并行），返回保留下来的（已更新置信度）。
pub async fn judge_all(
    client: &dyn LlmClient,
    reg: &ToolRegistry,
    ctx: &ToolContext,
    findings: Vec<Finding>,
) -> Vec<Finding> {
    judge_all_with_stats(client, reg, ctx, findings, false)
        .await
        .0
}

/// 对一批 finding 逐条证伪，并返回统计。
pub async fn judge_all_with_stats(
    client: &dyn LlmClient,
    reg: &ToolRegistry,
    ctx: &ToolContext,
    findings: Vec<Finding>,
    verbose: bool,
) -> (Vec<Finding>, JudgeStats) {
    judge_all_with_stats_limited(client, reg, ctx, findings, verbose, DEFAULT_CONCURRENCY).await
}

/// 对一批 finding 逐条证伪，并限制并发，避免候选过多时打满 provider 限流。
pub async fn judge_all_with_stats_limited(
    client: &dyn LlmClient,
    reg: &ToolRegistry,
    ctx: &ToolContext,
    findings: Vec<Finding>,
    verbose: bool,
    max_concurrency: usize,
) -> (Vec<Finding>, JudgeStats) {
    let original_count = findings.len();
    // 先过硬排除。
    let candidates: Vec<Finding> = findings.into_iter().filter(|f| !hard_excluded(f)).collect();
    let mut stats = JudgeStats {
        candidates: candidates.len(),
        hard_excluded: original_count.saturating_sub(candidates.len()),
        ..JudgeStats::default()
    };

    if verbose {
        eprintln!(
            "  [judge] 开始证伪：候选 {} 条，硬排除 {} 条",
            stats.candidates, stats.hard_excluded
        );
    }

    let verdicts: Vec<(Finding, JudgeOne)> =
        stream::iter(candidates.into_iter().map(|f| async move {
            let one = judge_one_with_stats(client, reg, ctx, &f).await;
            (f, one)
        }))
        .buffer_unordered(max_concurrency.max(1))
        .collect()
        .await;
    let mut kept = Vec::new();
    for (mut f, one) in verdicts {
        stats.llm_requests += one.stats.llm_requests;
        stats.tool_calls += one.stats.tool_calls;
        stats.usage.add(&one.stats.usage);
        for (name, count) in one.stats.tool_counts {
            *stats.tool_counts.entry(name).or_default() += count;
        }

        let verdict = one.verdict;
        match verdict {
            Some(v) if v.real => {
                f.confidence = v.confidence;
                f.reachability = v.reachability;
                if !v.reason.is_empty() {
                    f.evidence = v.reason;
                }
                stats.kept += 1;
                kept.push(f);
            }
            Some(_) => {
                stats.refuted += 1;
            }
            None => {
                // Judge 失败：保守保留，但下调置信度。
                f.confidence = (f.confidence * 0.8).min(0.79);
                stats.failed_open += 1;
                kept.push(f);
            }
        }
    }
    if verbose {
        eprintln!(
            "  [judge] 完成：保留 {} 条，证伪 {} 条，失败保留 {} 条；LLM {} 次 · 工具 {} 次（{}）；{}",
            stats.kept + stats.failed_open,
            stats.refuted,
            stats.failed_open,
            stats.llm_requests,
            stats.tool_calls,
            stats.tool_summary(),
            stats.usage.summary()
        );
    }
    (kept, stats)
}

struct JudgeOne {
    verdict: Option<Verdict>,
    stats: JudgeStats,
}

/// 对单条 finding 证伪。
async fn judge_one_with_stats(
    client: &dyn LlmClient,
    reg: &ToolRegistry,
    ctx: &ToolContext,
    f: &Finding,
) -> JudgeOne {
    let mut tools = reg.defs();
    tools.push(verdict_def());

    let mut messages = vec![Message::user(prompt::user_prompt(f))];
    let mut stats = JudgeStats::default();

    for _ in 0..MAX_ROUNDS {
        stats.llm_requests += 1;
        let resp = match client.complete(prompt::SYSTEM, &messages, &tools).await {
            Ok(resp) => resp,
            Err(_) => {
                return JudgeOne {
                    verdict: None,
                    stats,
                };
            }
        };
        stats.usage.add(&resp.usage);
        messages.push(Message::assistant(resp.content.clone()));

        let tool_uses: Vec<_> = resp.tool_uses().into_iter().cloned().collect();
        if tool_uses.is_empty() {
            if resp.stop_reason == StopReason::EndTurn {
                return JudgeOne {
                    verdict: None,
                    stats,
                };
            }
            messages.push(Message::user(
                "Please verify with tools if needed, then call verdict with the final decision.",
            ));
            continue;
        }

        let mut results = Vec::new();
        for tu in &tool_uses {
            stats.record_tool(&tu.name);
            if tu.name == "verdict" {
                return JudgeOne {
                    verdict: parse_verdict(&tu.input),
                    stats,
                };
            }
            let (content, is_error) = match reg.dispatch(&tu.name, &tu.input, ctx).await {
                Ok(s) => (s, false),
                Err(e) => (format!("Tool error: {e}"), true),
            };
            results.push(ToolResult {
                tool_use_id: tu.id.clone(),
                content,
                is_error,
            });
        }
        messages.push(Message::tool_results(results));
    }
    JudgeOne {
        verdict: None,
        stats,
    }
}

fn verdict_def() -> ToolDef {
    ToolDef {
        name: "verdict".into(),
        description: "Give the final verdict for this finding.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "real": { "type": "boolean", "description": "Whether the issue is real and cannot be disproved" },
                "confidence": { "type": "number", "description": "Confidence from 0 to 1" },
                "reachability": {
                    "type": "string",
                    "enum": ["reachable", "latent", "unknown"],
                    "description": "Can this code path actually execute given the current callers/guards? 'reachable' = it can fire now; 'latent' = the code is correct-as-written but an upstream router/guard makes this branch or statement currently unreachable (a latent bug that fires only if someone later changes routing); 'unknown' = cannot determine. Default to 'reachable' unless you verified an upstream condition makes it unreachable."
                },
                "reason": { "type": "string", "description": "Concise evidence/reason in the requested output language. If reachability is 'latent', state the upstream condition that makes it unreachable." }
            },
            "required": ["real", "confidence"]
        }),
    }
}

fn parse_verdict(input: &Value) -> Option<Verdict> {
    let real = input.get("real").and_then(|v| v.as_bool())?;
    let confidence = input
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f.clamp(0.0, 1.0) as f32)
        .unwrap_or(if real { 0.6 } else { 0.0 });
    let reason = input
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reachability = match input.get("reachability").and_then(|v| v.as_str()) {
        Some("latent") => Reachability::Latent,
        Some("reachable") => Reachability::Reachable,
        _ => Reachability::Unknown,
    };
    Some(Verdict {
        real,
        confidence,
        reason,
        reachability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Diff;
    use crate::model::{ContentBlock, Dimension, LlmResponse, Severity, ToolUse, Usage};
    use crate::tool::ToolContext;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 永远返回固定 verdict 的 mock（或永远报错）。
    struct VerdictMock {
        verdict: Option<(bool, f64)>, // None = 请求报错
    }

    #[async_trait::async_trait]
    impl LlmClient for VerdictMock {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[ToolDef],
        ) -> anyhow::Result<LlmResponse> {
            match self.verdict {
                Some((real, conf)) => Ok(LlmResponse {
                    content: vec![ContentBlock::ToolUse(ToolUse {
                        id: "v0".into(),
                        name: "verdict".into(),
                        input: json!({"real": real, "confidence": conf, "reason": "测试理由"}),
                    })],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                }),
                None => anyhow::bail!("judge 请求失败"),
            }
        }
        fn model(&self) -> &str {
            "mock"
        }
    }

    struct SlowCountingJudge {
        current: AtomicUsize,
        max_seen: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl LlmClient for SlowCountingJudge {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[ToolDef],
        ) -> anyhow::Result<LlmResponse> {
            let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            Ok(LlmResponse {
                content: vec![ContentBlock::ToolUse(ToolUse {
                    id: "v0".into(),
                    name: "verdict".into(),
                    input: json!({"real": true, "confidence": 0.9}),
                })],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            })
        }

        fn model(&self) -> &str {
            "slow-counting"
        }
    }

    fn finding(dim: Dimension, path: &str, conf: f32) -> Finding {
        Finding {
            dimension: dim,
            confidence: conf,
            severity: Severity::High,
            path: path.into(),
            start_line: 1,
            end_line: 1,
            message: "问题".into(),
            existing_code: "code".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: crate::model::Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::with_grep_index(Arc::new(Diff::default()), ".", None)
    }

    #[test]
    fn hard_exclude_drops_only_test_file_perf_style() {
        // 测试文件里的 perf/style 排除；security 不排除；src 里的都不排除。
        assert!(hard_excluded(&finding(
            Dimension::Perf,
            "src/foo_test.go",
            0.9
        )));
        assert!(hard_excluded(&finding(
            Dimension::Style,
            "pkg/__tests__/a.ts",
            0.9
        )));
        assert!(!hard_excluded(&finding(
            Dimension::Security,
            "src/foo_test.go",
            0.9
        )));
        assert!(!hard_excluded(&finding(
            Dimension::Perf,
            "src/main.rs",
            0.9
        )));
    }

    #[tokio::test]
    async fn keeps_real_finding_and_updates_confidence() {
        let client = VerdictMock {
            verdict: Some((true, 0.95)),
        };
        let reg = ToolRegistry::new();
        let kept = judge_all(
            &client,
            &reg,
            &ctx(),
            vec![finding(Dimension::Logic, "src/a.rs", 0.6)],
        )
        .await;
        assert_eq!(kept.len(), 1);
        assert!((kept[0].confidence - 0.95).abs() < 1e-6);
        assert_eq!(kept[0].evidence, "测试理由");
    }

    #[tokio::test]
    async fn stats_count_judge_llm_and_verdict_tool() {
        let client = VerdictMock {
            verdict: Some((true, 0.95)),
        };
        let reg = ToolRegistry::new();
        let (kept, stats) = judge_all_with_stats(
            &client,
            &reg,
            &ctx(),
            vec![
                finding(Dimension::Logic, "src/a.rs", 0.6),
                finding(Dimension::Perf, "src/foo_test.rs", 0.9),
            ],
            false,
        )
        .await;

        assert_eq!(kept.len(), 1);
        assert_eq!(stats.candidates, 1);
        assert_eq!(stats.hard_excluded, 1);
        assert_eq!(stats.kept, 1);
        assert_eq!(stats.llm_requests, 1);
        assert_eq!(stats.tool_calls, 1);
        assert_eq!(stats.tool_counts.get("verdict"), Some(&1));
    }

    /// 发出 real=true + reachability=latent 的 verdict。
    struct LatentMock;

    #[async_trait::async_trait]
    impl LlmClient for LatentMock {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[ToolDef],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::ToolUse(ToolUse {
                    id: "v0".into(),
                    name: "verdict".into(),
                    input: json!({
                        "real": true,
                        "confidence": 0.9,
                        "reachability": "latent",
                        "reason": "上游路由 guard 使该分支恒不可达"
                    }),
                })],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            })
        }
        fn model(&self) -> &str {
            "latent-mock"
        }
    }

    #[tokio::test]
    async fn latent_verdict_propagates_to_finding() {
        let client = LatentMock;
        let reg = ToolRegistry::new();
        let kept = judge_all(
            &client,
            &reg,
            &ctx(),
            vec![finding(Dimension::Logic, "src/a.rs", 0.6)],
        )
        .await;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].reachability, crate::model::Reachability::Latent);
    }

    #[tokio::test]
    async fn drops_refuted_finding() {
        let client = VerdictMock {
            verdict: Some((false, 0.9)),
        };
        let reg = ToolRegistry::new();
        let kept = judge_all(
            &client,
            &reg,
            &ctx(),
            vec![finding(Dimension::Logic, "src/a.rs", 0.9)],
        )
        .await;
        assert!(kept.is_empty());
    }

    #[tokio::test]
    async fn conservative_keep_on_judge_failure() {
        let client = VerdictMock { verdict: None }; // 请求报错
        let reg = ToolRegistry::new();
        let kept = judge_all(
            &client,
            &reg,
            &ctx(),
            vec![finding(Dimension::Logic, "src/a.rs", 0.9)],
        )
        .await;
        // 失败时保守保留，但置信度下调到 ≤ 0.79。
        assert_eq!(kept.len(), 1);
        assert!(kept[0].confidence <= 0.79);
    }

    #[tokio::test]
    async fn judge_respects_concurrency_limit() {
        let client = SlowCountingJudge {
            current: AtomicUsize::new(0),
            max_seen: AtomicUsize::new(0),
        };
        let reg = ToolRegistry::new();
        let fs = (0..6)
            .map(|i| finding(Dimension::Logic, &format!("src/{i}.rs"), 0.8))
            .collect();

        let (kept, stats) = judge_all_with_stats_limited(&client, &reg, &ctx(), fs, false, 2).await;

        assert_eq!(kept.len(), 6);
        assert_eq!(stats.llm_requests, 6);
        assert!(client.max_seen.load(Ordering::SeqCst) <= 2);
    }
}
