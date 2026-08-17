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
use std::time::{Duration, Instant};

/// Judge 轮次上限。默认期望首轮（带证据）即裁决；仅边界情形才用工具升级核实，
/// 故上限设小以截断长尾。
const MAX_ROUNDS: usize = 4;
const DEFAULT_CONCURRENCY: usize = 4;

/// 确定性证伪：被引代码里有会改结论的控制条件，叙述却完全没提。
///
/// 金标准 gin#4709：叙述是「TempDir 可写所以写入必成功、测试必挂」，
/// 同一段代码却有 `mode = 0o644` / `Chmod`。推理链错了，不是真问题。
pub fn causal_gap_refutes(f: &Finding) -> bool {
    if !code_has_file_mode_control(&f.existing_code) {
        return false;
    }
    let story = format!("{}\n{}", f.message, f.evidence);
    if story_mentions_file_mode(&story) {
        return false;
    }
    story_claims_unconditional_write_success(&story)
}

fn code_has_file_mode_control(code: &str) -> bool {
    let c = code.to_ascii_lowercase();
    c.contains("chmod")
        || c.contains("filemode")
        || c.contains("0o644")
        || c.contains("0o755")
        || c.contains("0o444")
        || c.contains("0o000")
}

fn story_mentions_file_mode(story: &str) -> bool {
    let s = story.to_ascii_lowercase();
    s.contains("chmod")
        || s.contains("filemode")
        || s.contains("0o644")
        || s.contains("0o755")
        || s.contains("0o444")
        || s.contains("0o000")
        || s.contains("no x")
        || s.contains("execute bit")
        || s.contains("mode=")
        || s.contains("mode =")
}

fn story_claims_unconditional_write_success(story: &str) -> bool {
    let s = story.to_ascii_lowercase();
    let success = s.contains("将成功")
        || s.contains("都会成功")
        || s.contains("will succeed")
        || s.contains("必然失败")
        || s.contains("必挂")
        || s.contains("must fail")
        || s.contains("will fail")
        || s.contains("一定失败");
    let writeish = s.contains("tempdir")
        || s.contains("可写")
        || s.contains("writable")
        || s.contains("mkdirall")
        || s.contains("saveuploadedfile")
        || s.contains("create");
    success && writeish
}

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
    /// 因 `--timeout` 预算耗尽而未完成证伪的条数（计入 fail-open）。
    pub timed_out: usize,
    /// 确定性因果缺口证伪（未调 LLM）。
    pub causal_gap: usize,
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
    judge_all_with_deadline(client, reg, ctx, findings, verbose, max_concurrency, None).await
}

/// 同 [`judge_all_with_stats_limited`]，并遵守整次审查剩下的墙钟预算。
///
/// `timeout = Some(0)` 或预算已耗尽：不再调 LLM，fail-open 并记 `timed_out`。
pub async fn judge_all_with_deadline(
    client: &dyn LlmClient,
    reg: &ToolRegistry,
    ctx: &ToolContext,
    findings: Vec<Finding>,
    verbose: bool,
    max_concurrency: usize,
    timeout: Option<Duration>,
) -> (Vec<Finding>, JudgeStats) {
    let original_count = findings.len();
    // 先过硬排除。
    let candidates: Vec<Finding> = findings.into_iter().filter(|f| !hard_excluded(f)).collect();
    let mut stats = JudgeStats {
        candidates: candidates.len(),
        hard_excluded: original_count.saturating_sub(candidates.len()),
        ..JudgeStats::default()
    };

    let mut to_judge = Vec::new();
    for f in candidates {
        if causal_gap_refutes(&f) {
            stats.refuted += 1;
            stats.causal_gap += 1;
            if verbose {
                eprintln!(
                    "  [judge] causal-gap refute {} (file-mode control unmentioned in story)",
                    f.path
                );
            }
        } else {
            to_judge.push(f);
        }
    }

    if verbose {
        eprintln!(
            "  [judge] 开始证伪：候选 {} 条，硬排除 {} 条，因果缺口 {} 条",
            stats.candidates, stats.hard_excluded, stats.causal_gap
        );
    }

    let budget_start = Instant::now();
    let verdicts: Vec<(Finding, JudgeOne)> =
        stream::iter(to_judge.into_iter().map(|f| async move {
            let remaining = timeout.map(|t| t.saturating_sub(budget_start.elapsed()));
            if remaining.is_some_and(|r| r.is_zero()) {
                return (
                    f,
                    JudgeOne {
                        verdict: None,
                        stats: JudgeStats::default(),
                        timed_out: true,
                    },
                );
            }
            let work = judge_one_with_stats(client, reg, ctx, &f);
            let one = if let Some(r) = remaining {
                match tokio::time::timeout(r, work).await {
                    Ok(one) => one,
                    Err(_) => JudgeOne {
                        verdict: None,
                        stats: JudgeStats::default(),
                        timed_out: true,
                    },
                }
            } else {
                work.await
            };
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
        if one.timed_out {
            stats.timed_out += 1;
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
                // Judge 失败 / 超时：保守保留，但下调置信度。
                f.confidence = (f.confidence * 0.8).min(0.79);
                stats.failed_open += 1;
                kept.push(f);
            }
        }
    }
    if verbose {
        eprintln!(
            "  [judge] 完成：保留 {} 条，证伪 {} 条，失败保留 {} 条，超时 {} 条；LLM {} 次 · 工具 {} 次（{}）；{}",
            stats.kept + stats.failed_open,
            stats.refuted,
            stats.failed_open,
            stats.timed_out,
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
    timed_out: bool,
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
                    timed_out: false,
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
                    timed_out: false,
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
                    timed_out: false,
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
        timed_out: false,
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
                None => anyhow::bail!("judge request failed"),
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
    async fn empty_findings_returns_empty_and_zero_stats() {
        let client = VerdictMock {
            verdict: Some((true, 0.9)),
        };
        let reg = ToolRegistry::new();
        let (kept, stats) = judge_all_with_stats(&client, &reg, &ctx(), vec![], false).await;
        assert!(kept.is_empty());
        assert_eq!(stats.candidates, 0);
        assert_eq!(stats.hard_excluded, 0);
        assert_eq!(stats.llm_requests, 0);
    }

    #[tokio::test]
    async fn malformed_verdict_conservatively_keeps_with_damped_confidence() {
        struct BadVerdictMock;
        #[async_trait::async_trait]
        impl LlmClient for BadVerdictMock {
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
                        input: json!({"real": true}), // 缺少 confidence
                    })],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                })
            }
            fn model(&self) -> &str {
                "bad"
            }
        }
        let reg = ToolRegistry::new();
        let (kept, _) = judge_all_with_stats(
            &BadVerdictMock,
            &reg,
            &ctx(),
            vec![finding(Dimension::Logic, "src/a.rs", 0.9)],
            false,
        )
        .await;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].confidence, 0.6);
    }

    #[tokio::test]
    async fn parse_verdict_reachability_and_confidence_defaults() {
        // real=false 且缺 confidence → 0
        let v = parse_verdict(&json!({"real": false})).unwrap();
        assert!(!v.real);
        assert_eq!(v.confidence, 0.0);
        assert_eq!(v.reachability, Reachability::Unknown);

        // 非法 reachability 回退 Unknown
        let v = parse_verdict(&json!({"real": true, "confidence": 0.8, "reachability": "bogus"}))
            .unwrap();
        assert!(v.real);
        assert_eq!(v.reachability, Reachability::Unknown);

        // confidence 越界被截断
        let v = parse_verdict(&json!({"real": true, "confidence": 1.5})).unwrap();
        assert_eq!(v.confidence, 1.0);
        let v = parse_verdict(&json!({"real": true, "confidence": -0.2})).unwrap();
        assert_eq!(v.confidence, 0.0);
    }

    #[test]
    fn parse_verdict_missing_real_returns_none() {
        assert!(parse_verdict(&json!({"confidence": 0.9})).is_none());
        assert!(parse_verdict(&json!({})).is_none());
    }

    #[test]
    fn hard_excluded_non_test_files_and_security_dimension() {
        assert!(!hard_excluded(&finding(
            Dimension::Perf,
            "src/main.rs",
            0.9
        )));
        assert!(!hard_excluded(&finding(
            Dimension::Security,
            "src/foo_test.go",
            0.9
        )));
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

    fn gin4709_false_block() -> Finding {
        // 2026-08-04 实跑金标准：叙述是「TempDir 可写 → 写入必成功 → 测试必挂」，
        // 被引代码里却有 mode=0o644 / Chmod，叙述完全没提。
        Finding {
            dimension: Dimension::Logic,
            confidence: 0.98,
            severity: Severity::High,
            path: "context_test.go".into(),
            start_line: 258,
            end_line: 275,
            message: "AI 机械式重构导致测试语义丢失：改用 t.TempDir() 后目录干净且可写，\
SaveUploadedFile 将成功执行，但测试仍断言 require.Error，导致该测试必然失败。"
                .into(),
            existing_code: r#"
var mode fs.FileMode = 0o644
dst := filepath.Join(t.TempDir(), "test", "permission_test")
require.Error(t, c.SaveUploadedFile(f, dst, mode))
// SaveUploadedFile: os.MkdirAll(dir, mode); os.Chmod(dir, mode)
"#
            .into(),
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

    #[test]
    fn causal_gap_refutes_gin4709_and_spares_explained_mode() {
        assert!(
            causal_gap_refutes(&gin4709_false_block()),
            "gin#4709 gold must be a deterministic causal-gap refute"
        );
        let mut explained = gin4709_false_block();
        explained
            .message
            .push_str(" 但同一段代码用 mode=0o644 并 Chmod，无 x 位，非 root 下写入仍会失败。");
        assert!(
            !causal_gap_refutes(&explained),
            "if the story already names the mode control, do not auto-refute"
        );
        assert!(
            !causal_gap_refutes(&finding(Dimension::Security, "app.js", 0.9)),
            "ordinary findings without a mode control must not be refuted"
        );
    }

    #[tokio::test]
    async fn gin4709_is_dropped_even_when_llm_says_real() {
        let client = VerdictMock {
            verdict: Some((true, 0.99)),
        };
        let reg = ToolRegistry::new();
        let (kept, stats) =
            judge_all_with_stats(&client, &reg, &ctx(), vec![gin4709_false_block()], false).await;
        assert!(
            kept.is_empty(),
            "causal-gap must override a confident LLM real=true: {kept:#?}"
        );
        assert!(
            stats.causal_gap >= 1 && stats.refuted >= 1,
            "stats must record the deterministic refute: {stats:?}"
        );
        assert_eq!(
            stats.llm_requests, 0,
            "do not spend an LLM call on a causal gap"
        );
    }

    #[tokio::test]
    async fn judge_timeout_fail_opens_and_counts() {
        let client = SlowCountingJudge {
            current: AtomicUsize::new(0),
            max_seen: AtomicUsize::new(0),
        };
        let reg = ToolRegistry::new();
        let (kept, stats) = judge_all_with_deadline(
            &client,
            &reg,
            &ctx(),
            vec![finding(Dimension::Logic, "src/a.rs", 0.9)],
            false,
            1,
            Some(std::time::Duration::from_millis(5)),
        )
        .await;
        assert_eq!(kept.len(), 1);
        assert!(kept[0].confidence <= 0.79);
        assert!(
            stats.timed_out >= 1,
            "timeout must be counted, got {stats:?}"
        );
    }
}
