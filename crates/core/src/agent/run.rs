//! Agent tool-use 运行循环：调用 LLM → 执行工具 → 回灌结果 → 直到 `task_done`、自然结束或触达轮次上限。
//!
//! `report_finding` / `task_done` 是控制工具，由循环内部拦截处理（前者收集 Finding，后者终止）。

use super::control_tools::{
    parse_finding, parse_intent_finding, report_finding_def, report_intent_finding_def,
    task_done_def,
};
use super::{
    dimension_focus_block, AgentConfig, AgentExitReason, AgentRun, AgentStats, LOOP_GUARD_LIMIT,
};
use crate::llm::LlmClient;
use crate::model::{ContentBlock, Dimension, Finding, Message, Role, StopReason, ToolResult};
use crate::tool::{ToolContext, ToolRegistry};
use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;

/// 墙钟软着陆阈值：预算耗到该比例即切入收口轮，把剩余时间留给上报而不是继续探索。
/// 修复的失败模式：慢 provider 上探索烧满整个 timeout，一条都没上报就被硬超时标 incomplete。
const LANDING_FRACTION: f32 = 0.75;

/// 墙钟预算是否已消耗到「该收口」的程度。无 timeout（无预算可压）恒 false。
fn past_landing_threshold(elapsed: Duration, timeout: Option<Duration>) -> bool {
    timeout.is_some_and(|t| elapsed >= t.mul_f32(LANDING_FRACTION))
}

/// 跑一个维度 Agent，返回它上报的 Finding（行号尚未重定位）。
pub async fn run_agent(
    client: &dyn LlmClient,
    registry: &ToolRegistry,
    ctx: &ToolContext,
    cfg: &AgentConfig,
    user_prompt: String,
) -> Result<Vec<Finding>> {
    Ok(
        run_agent_with_stats(client, registry, ctx, cfg, user_prompt)
            .await?
            .findings,
    )
}

/// 跑一个维度 Agent，并返回运行统计。
pub async fn run_agent_with_stats(
    client: &dyn LlmClient,
    registry: &ToolRegistry,
    ctx: &ToolContext,
    cfg: &AgentConfig,
    user_prompt: String,
) -> Result<AgentRun> {
    // 意图维度用需求锚定的 report_intent_finding，其余维度用行锚的 report_finding。
    let intent_dim = cfg.dimension == Dimension::Intent;
    let report_name = if intent_dim {
        "report_intent_finding"
    } else {
        "report_finding"
    };
    let report_def = || {
        if intent_dim {
            report_intent_finding_def()
        } else {
            report_finding_def()
        }
    };
    let mut tools = registry.defs();
    tools.push(report_def());
    tools.push(task_done_def());
    // 最后一轮只给上报/结束工具，逼模型基于已有信息收口。
    let final_tools = vec![report_def(), task_done_def()];

    // 首条 user 消息分两块：
    //   块 0 = 共享大块（diff + 文件全文，维度无关）→ 由客户端挂缓存断点，跨维度/跨轮复用；
    //   块 1 = 本维度聚焦点（位于缓存断点之后，各维度不同，不破坏缓存）。
    let focus = dimension_focus_block(cfg.dimension);
    let first = Message {
        role: Role::User,
        content: if user_prompt.contains(&focus) {
            vec![ContentBlock::text(user_prompt)]
        } else {
            vec![ContentBlock::text(user_prompt), ContentBlock::text(focus)]
        },
    };
    let mut messages = vec![first];
    let mut findings = Vec::new();
    let mut stats = AgentStats::default();
    // 循环熔断计数：key = 工具名 + 入参，value = 相同调用次数。
    let mut call_counts: HashMap<String, usize> = HashMap::new();
    let start = std::time::Instant::now();
    // 默认 MaxRounds：循环自然走完即视为完成（末轮已强制收口）。各 break 点会覆盖。
    let mut exit_reason = AgentExitReason::MaxRounds;
    let mut error_detail: Option<String> = None;

    let dim = cfg.dimension.as_str();
    for round in 0..cfg.max_rounds {
        // 墙钟超时：每轮开始前检查，超时则优雅收尾——已 report_finding 的发现都保留，
        // 不像硬 cancel 那样丢工作（避免"超时即丢=误 PASS"）。
        if let Some(t) = cfg.timeout {
            if start.elapsed() >= t {
                exit_reason = AgentExitReason::TimedOut;
                if cfg.verbose {
                    eprintln!(
                        "  [{dim}] timed out after {}s; wrapping up early (kept {} findings)",
                        t.as_secs(),
                        findings.len()
                    );
                }
                break;
            }
        }
        // 发送前预检：估算本轮请求 token，超输入预算则确定性收尾（不撞 API 400）。
        if let Some(budget) = cfg.max_input_tokens {
            let est = estimate_request_tokens(&cfg.system_prompt, &messages);
            if est > budget {
                exit_reason = AgentExitReason::ContextOverflow;
                if cfg.verbose {
                    eprintln!(
                        "  [{dim}] round {} pre-check over budget (est {est} > {budget} tok); wrapping up early (kept {} findings)",
                        round + 1,
                        findings.len()
                    );
                }
                break;
            }
        }
        // 强制收口：还差 1 轮（轮次预算），或墙钟预算将尽（软着陆，见 LANDING_FRACTION）——
        // 停止调研，把剩余预算用来上报已确信的发现，而不是探索到硬超时一无所报。
        let time_landing = past_landing_threshold(start.elapsed(), cfg.timeout);
        let is_final = round + 1 >= cfg.max_rounds || time_landing;
        let round_tools = if is_final { &final_tools } else { &tools };
        if is_final {
            messages.push(Message::user(if time_landing {
                format!(
                    "The time budget for this review is nearly exhausted. Conclude now from the information you already have: call {report_name} for confirmed issues, or call task_done if there are no credible issues. Do not call additional investigation tools."
                )
            } else {
                format!(
                    "This is the final round. Conclude from the information you already have: call {report_name} for confirmed issues, or call task_done if there are no credible issues. Do not call additional investigation tools."
                )
            }));
        }
        if cfg.verbose {
            eprintln!(
                "  [{dim}] round {}: calling LLM...{}",
                round + 1,
                if is_final { " (forced wrap-up)" } else { "" }
            );
        }
        stats.llm_requests += 1;
        let remaining = cfg.timeout.and_then(|t| t.checked_sub(start.elapsed()));
        let resp = if let Some(remaining) = remaining {
            match tokio::time::timeout(
                remaining,
                client.complete(&cfg.system_prompt, &messages, round_tools),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => {
                    exit_reason = AgentExitReason::TimedOut;
                    if cfg.verbose {
                        eprintln!(
                            "  [{dim}] round {} request timed out; wrapping up early (kept {} findings)",
                            round + 1,
                            findings.len()
                        );
                    }
                    break;
                }
            }
        } else {
            client
                .complete(&cfg.system_prompt, &messages, round_tools)
                .await
        };
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                // 请求失败：如实归类（鉴权 vs 其它），保留已收集的发现，但标记未审完，不静默放行。
                exit_reason = crate::agent::classify_request_error(&e);
                error_detail = Some(truncate_detail(&e.to_string()));
                if cfg.verbose {
                    eprintln!(
                        "  [{dim}] round {} request failed; wrapping up early (kept {} findings): {e}",
                        round + 1,
                        findings.len()
                    );
                    eprintln!(
                        "  [{dim}] stats: {} LLM calls, {} tool calls ({})",
                        stats.llm_requests,
                        stats.tool_calls,
                        stats.tool_summary()
                    );
                }
                break;
            }
        };
        stats.usage.add(&resp.usage);
        messages.push(Message::assistant(resp.content.clone()));

        let tool_uses: Vec<_> = resp.tool_uses().into_iter().cloned().collect();
        if cfg.verbose && !tool_uses.is_empty() {
            let names: Vec<&str> = tool_uses.iter().map(|t| t.name.as_str()).collect();
            eprintln!(
                "  [{dim}] round {}: {} tool call(s): {}",
                round + 1,
                tool_uses.len(),
                names.join(", ")
            );
        }
        if tool_uses.is_empty() {
            // 没有工具调用：模型自然结束，收尾。
            if resp.stop_reason == StopReason::EndTurn || resp.stop_reason == StopReason::MaxTokens
            {
                exit_reason = AgentExitReason::Completed;
                break;
            }
            // 异常：给一次纠正提示。
            messages.push(Message::user(format!(
                "Please call {report_name} to report an issue, or call task_done if there are no issues."
            )));
            continue;
        }

        let mut results = Vec::new();
        let mut done = false;
        for tu in &tool_uses {
            stats.record_tool(&tu.name);
            if let Some(p) = &cfg.progress {
                p.record_tool(dim, &tu.name, &tool_target(&tu.input));
            }
            let (content, is_error) = match tu.name.as_str() {
                "report_finding" => match parse_finding(&tu.input, cfg.dimension) {
                    Ok(f) => {
                        findings.push(f);
                        ("Finding recorded.".to_string(), false)
                    }
                    Err(e) => (format!("Invalid report_finding arguments: {e}"), true),
                },
                "report_intent_finding" => match parse_intent_finding(&tu.input) {
                    Ok(f) => {
                        findings.push(f);
                        ("Verdict recorded.".to_string(), false)
                    }
                    Err(e) => (
                        format!("Invalid report_intent_finding arguments: {e}"),
                        true,
                    ),
                },
                "task_done" => {
                    done = true;
                    ("Review finished.".to_string(), false)
                }
                other => {
                    // 循环熔断：相同工具+参数调用达上限则短路，逼模型基于已有信息收口。
                    let key = format!("{other}\u{1}{}", tu.input);
                    let n = call_counts.entry(key).or_insert(0);
                    *n += 1;
                    if *n >= LOOP_GUARD_LIMIT {
                        stats.loop_guarded += 1;
                        if cfg.verbose {
                            eprintln!(
                                "  [{dim}] loop guard: {other} called {n}x with identical args; short-circuiting."
                            );
                        }
                        (
                            format!("You have called {other} with identical arguments {n} times. The result will not change. Conclude from existing information: report with report_finding or finish with task_done. Do not repeat this call."),
                            true,
                        )
                    } else {
                        match registry.dispatch(other, &tu.input, ctx).await {
                            Ok(s) => (s, false),
                            Err(e) => (format!("Tool error: {e}"), true),
                        }
                    }
                }
            };
            results.push(ToolResult {
                tool_use_id: tu.id.clone(),
                content,
                is_error,
            });
        }
        messages.push(Message::tool_results(results));
        if done {
            exit_reason = AgentExitReason::Completed;
            if cfg.verbose {
                eprintln!(
                    "  [{dim}] done, {} findings; {} LLM calls, {} tool calls ({}); {}",
                    findings.len(),
                    stats.llm_requests,
                    stats.tool_calls,
                    stats.tool_summary(),
                    stats.usage.summary()
                );
            }
            break;
        }
    }

    Ok(AgentRun {
        findings,
        stats,
        exit_reason,
        error_detail,
    })
}

/// 截断错误详情，避免把整段服务端 JSON 灌进告警；保留足够定位的前缀。
fn truncate_detail(s: &str) -> String {
    const MAX: usize = 240;
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX {
        one_line
    } else {
        let head: String = one_line.chars().take(MAX).collect();
        format!("{head}…")
    }
}

/// 估算一次请求的输入 token（system + 所有消息的文本/工具入参/工具结果）。保守偏高。
fn estimate_request_tokens(system: &str, messages: &[Message]) -> usize {
    let mut total = crate::llm::estimate_tokens(system);
    for m in messages {
        for b in &m.content {
            let t = match b {
                ContentBlock::Text { text } => crate::llm::estimate_tokens(text),
                ContentBlock::ToolUse(u) => {
                    crate::llm::estimate_tokens(&u.name)
                        + crate::llm::estimate_tokens(&u.input.to_string())
                }
                ContentBlock::ToolResult(r) => crate::llm::estimate_tokens(&r.content),
            };
            total += t;
        }
    }
    total
}

/// 从工具入参里取一个简短「目标」用于进度展示（路径/查询/符号名…），截断到 48 字符。
fn tool_target(input: &serde_json::Value) -> String {
    for k in [
        "path",
        "file",
        "query",
        "pattern",
        "symbol",
        "name",
        "criterion",
    ] {
        if let Some(s) = input.get(k).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return s.chars().take(48).collect();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Diff;
    use crate::model::{Dimension, LlmResponse, Severity, ToolDef, Usage};
    use crate::tool::readonly_tools;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// 脚本化的 mock LLM：按队列返回预设响应/错误，并记录每次收到的工具数量。
    struct MockClient {
        queue: Mutex<VecDeque<Result<LlmResponse>>>,
        tool_counts: Mutex<Vec<usize>>,
        seen_messages: Mutex<Vec<Vec<Message>>>,
    }

    impl MockClient {
        fn new(responses: Vec<Result<LlmResponse>>) -> Self {
            MockClient {
                queue: Mutex::new(responses.into_iter().collect()),
                tool_counts: Mutex::new(Vec::new()),
                seen_messages: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for MockClient {
        async fn complete(
            &self,
            _system: &str,
            messages: &[Message],
            tools: &[ToolDef],
        ) -> Result<LlmResponse> {
            self.tool_counts.lock().unwrap().push(tools.len());
            self.seen_messages.lock().unwrap().push(messages.to_vec());
            self.queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(resp(vec![("task_done", json!({}))])))
        }
        fn model(&self) -> &str {
            "mock"
        }
    }

    struct SlowClient;

    #[async_trait::async_trait]
    impl LlmClient for SlowClient {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[ToolDef],
        ) -> Result<LlmResponse> {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(resp(vec![("task_done", json!({}))]))
        }

        fn model(&self) -> &str {
            "slow"
        }
    }

    /// 构造一个 tool_use 响应。
    fn resp(uses: Vec<(&str, serde_json::Value)>) -> LlmResponse {
        let content = uses
            .into_iter()
            .enumerate()
            .map(|(i, (name, input))| {
                ContentBlock::ToolUse(crate::model::ToolUse {
                    id: format!("t{i}"),
                    name: name.to_string(),
                    input,
                })
            })
            .collect();
        LlmResponse {
            content,
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        }
    }

    fn finding_input() -> serde_json::Value {
        json!({
            "path": "a.rs",
            "message": "空指针解引用",
            "line_start": 42,
            "line_end": 43,
            "existing_code": "x.unwrap()",
            "severity": "high",
            "confidence": 0.9
        })
    }

    fn ctx() -> ToolContext {
        ToolContext::with_grep_index(Arc::new(Diff::default()), ".", None)
    }

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        for t in readonly_tools() {
            r.register(t);
        }
        r
    }

    fn cfg(max_rounds: usize) -> AgentConfig {
        let mut c = AgentConfig::for_dimension(Dimension::Logic);
        c.max_rounds = max_rounds;
        c
    }

    #[tokio::test]
    async fn collects_finding_and_terminates_on_task_done() {
        let client = MockClient::new(vec![Ok(resp(vec![
            ("report_finding", finding_input()),
            ("task_done", json!({})),
        ]))]);
        let findings = run_agent(&client, &registry(), &ctx(), &cfg(5), "审查".into())
            .await
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].dimension, Dimension::Logic);
        assert_eq!(findings[0].severity, Severity::High);
        // 模型直接给的行号被解析填入。
        assert_eq!(findings[0].start_line, 42);
        assert_eq!(findings[0].end_line, 43);
    }

    #[tokio::test]
    async fn loop_guard_short_circuits_repeated_identical_calls() {
        // 连续 3 次相同参数的 code_search → 第 3 次触发熔断短路；随后 task_done。
        let call = || resp(vec![("code_search", json!({"pattern": "xyzzy_nope"}))]);
        let client = MockClient::new(vec![
            Ok(call()),
            Ok(call()),
            Ok(call()),
            Ok(resp(vec![("task_done", json!({}))])),
        ]);
        let run = run_agent_with_stats(&client, &registry(), &ctx(), &cfg(10), "审查".into())
            .await
            .unwrap();
        assert_eq!(run.stats.loop_guarded, 1, "第 3 次相同调用应被熔断");
    }

    #[tokio::test]
    async fn missing_line_defaults_to_zero_for_fallback() {
        // 不给 line_start → start/end 置 0，留给重定位用 existing_code 兜底。
        let input = json!({
            "path": "a.rs", "message": "m", "existing_code": "x.unwrap()", "severity": "low"
        });
        let client = MockClient::new(vec![Ok(resp(vec![
            ("report_finding", input),
            ("task_done", json!({})),
        ]))]);
        let findings = run_agent(&client, &registry(), &ctx(), &cfg(5), "审查".into())
            .await
            .unwrap();
        assert_eq!(findings[0].start_line, 0);
        assert_eq!(findings[0].end_line, 0);
    }

    #[tokio::test]
    async fn graceful_degradation_preserves_collected_findings() {
        // 第 1 轮报一个问题（不结束）；第 2 轮请求失败 → 应保留已收集的 1 条，且不报错。
        let client = MockClient::new(vec![
            Ok(resp(vec![("report_finding", finding_input())])),
            Err(anyhow::anyhow!("网络超时")),
        ]);
        let run = run_agent_with_stats(&client, &registry(), &ctx(), &cfg(5), "审查".into())
            .await
            .expect("优雅降级应返回 Ok 而非 Err");
        assert_eq!(run.findings.len(), 1);
        // 请求失败必须可区分（未审完），不能伪装成正常完成。
        assert_eq!(run.exit_reason, AgentExitReason::RequestFailed);
        assert!(run.incomplete());
    }

    #[tokio::test]
    async fn context_overflow_detected_before_request() {
        // 预算极小 → 首轮发送前预检即超预算 → ContextOverflow，且根本不发请求。
        let client = MockClient::new(vec![Ok(resp(vec![("task_done", json!({}))]))]);
        let mut c = cfg(5);
        c.max_input_tokens = Some(1); // 1 token 预算，必超
        let run = run_agent_with_stats(
            &client,
            &registry(),
            &ctx(),
            &c,
            "审查一段较长的内容".into(),
        )
        .await
        .unwrap();
        assert_eq!(run.exit_reason, AgentExitReason::ContextOverflow);
        assert!(run.incomplete());
        assert_eq!(run.stats.llm_requests, 0, "预检拦截，不应发出请求");
    }

    #[tokio::test]
    async fn normal_completion_is_not_incomplete() {
        let client = MockClient::new(vec![Ok(resp(vec![("task_done", json!({}))]))]);
        let run = run_agent_with_stats(&client, &registry(), &ctx(), &cfg(5), "审查".into())
            .await
            .unwrap();
        assert_eq!(run.exit_reason, AgentExitReason::Completed);
        assert!(!run.incomplete());
    }

    #[tokio::test]
    async fn stats_count_llm_rounds_and_tools() {
        let client = MockClient::new(vec![
            Ok(resp(vec![("report_finding", finding_input())])),
            Ok(resp(vec![
                ("code_search", json!({"pattern": "foo"})),
                ("task_done", json!({})),
            ])),
        ]);

        let run = run_agent_with_stats(&client, &registry(), &ctx(), &cfg(5), "审查".into())
            .await
            .unwrap();

        assert_eq!(run.stats.llm_requests, 2);
        assert_eq!(run.stats.tool_calls, 3);
        assert_eq!(run.stats.findings_reported, 1);
        assert_eq!(run.stats.task_done_calls, 1);
        assert_eq!(run.stats.tool_counts.get("report_finding"), Some(&1));
        assert_eq!(run.stats.tool_counts.get("code_search"), Some(&1));
        assert_eq!(run.stats.tool_counts.get("task_done"), Some(&1));
    }

    #[tokio::test]
    async fn final_round_forces_conclusion_with_reduced_tools() {
        // 第 1 轮报问题（不结束）；第 2 轮为强制收口轮，应只给 report_finding + task_done。
        let client = MockClient::new(vec![
            Ok(resp(vec![("report_finding", finding_input())])),
            Ok(resp(vec![("task_done", json!({}))])),
        ]);
        let _ = run_agent(&client, &registry(), &ctx(), &cfg(2), "审查".into())
            .await
            .unwrap();
        let counts = client.tool_counts.lock().unwrap().clone();
        assert_eq!(counts.len(), 2);
        // 非收口轮：全部只读工具 + report_finding + task_done。
        assert_eq!(counts[0], readonly_tools().len() + 2);
        // 收口轮：仅 report_finding + task_done = 2。
        assert_eq!(counts[1], 2);
    }

    #[tokio::test]
    async fn dimension_focus_is_not_duplicated_when_prompt_already_contains_it() {
        let client = MockClient::new(vec![Ok(resp(vec![("task_done", json!({}))]))]);
        let focus = dimension_focus_block(Dimension::Logic);
        let prompt = format!("共享 diff\n\n{focus}");

        let _ = run_agent(&client, &registry(), &ctx(), &cfg(5), prompt)
            .await
            .unwrap();

        let messages = client.seen_messages.lock().unwrap();
        let first = messages[0][0].text();
        assert_eq!(first.matches("## Review dimension").count(), 1);
    }

    #[tokio::test]
    async fn timeout_interrupts_in_flight_llm_request() {
        let mut c = cfg(5);
        c.timeout = Some(Duration::from_millis(30));

        let start = Instant::now();
        let run = run_agent_with_stats(&SlowClient, &registry(), &ctx(), &c, "审查".into())
            .await
            .unwrap();

        assert!(run.timed_out());
        assert_eq!(run.exit_reason, AgentExitReason::TimedOut);
        assert!(start.elapsed() < Duration::from_millis(150));
        assert_eq!(run.stats.llm_requests, 1);
    }

    #[test]
    fn landing_threshold_is_75_percent_of_timeout() {
        let t = Some(Duration::from_secs(100));
        assert!(!past_landing_threshold(Duration::from_secs(74), t));
        assert!(past_landing_threshold(Duration::from_secs(75), t));
        assert!(past_landing_threshold(Duration::from_secs(99), t));
        // 无 timeout → 永不触发（无预算可压）。
        assert!(!past_landing_threshold(Duration::from_secs(9999), None));
    }

    /// 每次调用 sleep 170ms 的"慢探索"客户端：拿到全量工具时继续探索（code_search，
    /// 参数随轮变化避开循环熔断）；一旦被切到收口工具集（无 code_search），上报 task_done。
    struct SlowExplorer {
        calls: Mutex<usize>,
        landing_seen: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl LlmClient for SlowExplorer {
        async fn complete(
            &self,
            _system: &str,
            messages: &[Message],
            tools: &[ToolDef],
        ) -> Result<LlmResponse> {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if let Some(last) = messages.last() {
                self.landing_seen.lock().unwrap().push(last.text());
            }
            if tools.iter().any(|t| t.name == "code_search") {
                Ok(resp(vec![(
                    "code_search",
                    json!({ "pattern": format!("probe_{}", *n) }),
                )]))
            } else {
                Ok(resp(vec![("task_done", json!({}))]))
            }
        }
        fn model(&self) -> &str {
            "slow-explorer"
        }
    }

    #[tokio::test]
    async fn wall_clock_landing_forces_wrapup_before_timeout() {
        // timeout=1000ms，阈值 75% = 750ms；每轮 ~100ms：r1..r7 正常探索，
        // r8@~750+ ≥阈值 → 收口轮（只剩上报工具 + 注入收口消息），且剩余 ~25% 预算足够收口调用完成。
        // 期望：客户端调 task_done → Completed（软着陆），而不是撞 1000ms 硬超时丢轮。
        let mut c = cfg(20);
        c.timeout = Some(Duration::from_millis(1000));
        let client = SlowExplorer {
            calls: Mutex::new(0),
            landing_seen: Mutex::new(Vec::new()),
        };

        let run = run_agent_with_stats(&client, &registry(), &ctx(), &c, "审查".into())
            .await
            .unwrap();

        assert_eq!(
            run.exit_reason,
            AgentExitReason::Completed,
            "应软着陆完成而非硬超时"
        );
        let seen = client.landing_seen.lock().unwrap();
        assert!(
            seen.iter().any(|m| m.contains("time budget")),
            "应收到临近墙钟的收口提示，got: {seen:?}"
        );
    }
}
