//! 端到端：`reviewgate security` 的饱和式 discovery。
//!
//! 两个场景用同一个临时仓库串行跑：
//! 1. 每轮报同一个问题 → 去重后池子不再增长 → 连续 2 轮空转即收敛（共 3 轮）。
//! 2. 每轮报新问题 → 池子一直增长 → 撞轮数上限 → 必须标记 incomplete，绝不假装审完。
//!
//! 用 mock LLM + 临时 git 仓库，不联网。
//!
//! 注：本测试切换进程 CWD 到临时仓库，故本文件**仅一个测试**避免 CWD 竞争。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig};
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, Dimension, LlmResponse, Message, StopReason, ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::ReviewOptions;
use reviewgate_core::security::{run_security_with_client, SecurityOptions};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 只计 **agent** 轮次（judge 调用带 `verdict` 工具，不计）。
/// `distinct` = true 时每轮报不同行，模拟「一直挖到新东西」。
struct RoundMock {
    agent_calls: Arc<AtomicUsize>,
    distinct: bool,
}

#[async_trait]
impl LlmClient for RoundMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        if tools.iter().any(|t| t.name == "verdict") {
            return Ok(LlmResponse {
                content: vec![tool_use(
                    "verdict",
                    serde_json::json!({"real": true, "confidence": 0.95, "reason": "确认"}),
                )],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            });
        }
        let n = self.agent_calls.fetch_add(1, Ordering::SeqCst);
        // distinct：每轮换一行 → dedupe 折不掉 → 候选池持续增长。
        let line = if self.distinct { 2 + n as u64 } else { 2 };
        Ok(LlmResponse {
            content: vec![
                tool_use(
                    "report_finding",
                    serde_json::json!({
                        "path": "app.js",
                        "line_start": line,
                        "line_end": line,
                        "existing_code": format!("  const x{line} = eval(input);"),
                        "message": format!("eval 注入 #{line}"),
                        "severity": "high",
                        "confidence": 0.9
                    }),
                ),
                tool_use("task_done", serde_json::json!({})),
            ],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        })
    }
    fn model(&self) -> &str {
        "mock"
    }
}

fn tool_use(name: &str, input: serde_json::Value) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: format!("{name}_0"),
        name: name.into(),
        input,
    })
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git 可执行")
        .success();
    assert!(ok, "git {args:?} 失败");
}

fn mock_config() -> Config {
    Config {
        provider: "mock".into(),
        providers: HashMap::new(),
        gate: GateConfig::default(),
        business: BusinessConfig::default(),
        issue_review: Default::default(),
        exclude: Default::default(),
        severity_labels: Vec::new(),
    }
}

fn security_opts(stop_after_no_new: usize, max_rounds: usize) -> SecurityOptions {
    let mut review = ReviewOptions::workspace(vec![Dimension::Security]);
    review.judge = false; // 隔离饱和循环行为，避免证伪阶段干扰计数
    review.write_metrics = false;
    let mut opts = SecurityOptions::new(review);
    opts.stop_after_no_new = stop_after_no_new;
    opts.max_rounds = max_rounds;
    opts
}

#[tokio::test]
async fn saturating_discovery_converges_and_flags_the_round_cap() {
    let tmp = std::env::temp_dir().join(format!("rg_sec_sat_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 1;\n}\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    std::fs::write(
        tmp.join("app.js"),
        "function f(input) {\n  const x2 = eval(input);\n  const x3 = eval(input);\n  const x4 = eval(input);\n  return x2;\n}\n",
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    // --- 场景 1：重复报同一个问题 → 应在「1 轮有新 + 2 轮空转」后收敛 ---
    let calls = Arc::new(AtomicUsize::new(0));
    let converged = run_security_with_client(
        &mock_config(),
        &security_opts(2, 10),
        &RoundMock {
            agent_calls: calls.clone(),
            distinct: false,
        },
    )
    .await
    .expect("饱和 discovery 应成功");
    let converged_rounds = calls.load(Ordering::SeqCst);

    // --- 场景 2：每轮都有新问题 → 应跑满上限并标记 incomplete ---
    let calls2 = Arc::new(AtomicUsize::new(0));
    let capped = run_security_with_client(
        &mock_config(),
        &security_opts(2, 3),
        &RoundMock {
            agent_calls: calls2.clone(),
            distinct: true,
        },
    )
    .await
    .expect("撞上限也应正常返回");
    let capped_rounds = calls2.load(Ordering::SeqCst);

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    // 场景 1：第 1 轮进池，第 2、3 轮去重后池子不增长 → 连续 2 轮空转 → 停。
    assert_eq!(
        converged_rounds, 3,
        "重复发现应在 1 轮有新 + 2 轮空转后收敛，实际跑了 {converged_rounds} 轮"
    );
    assert!(
        !converged.incomplete,
        "自然饱和不是未审完，不该标 incomplete"
    );
    assert_eq!(
        converged.findings.len(),
        1,
        "同一处问题重复报应被去重为 1 条"
    );

    // 场景 2：一直有新发现 → 跑满 max_rounds=3，且必须暴露覆盖可能不全。
    assert_eq!(
        capped_rounds, 3,
        "每轮都有新发现时应跑满 max_rounds=3，实际 {capped_rounds} 轮"
    );
    assert!(
        capped.incomplete,
        "撞轮数上限说明可能还没挖完，必须标 incomplete 而不是假装审完"
    );
    assert!(
        capped
            .warnings
            .iter()
            .any(|w| w.kind == "incomplete" && w.message.contains("round cap")),
        "撞上限要留下可读的告警，实际 warnings: {:?}",
        capped
            .warnings
            .iter()
            .map(|w| &w.message)
            .collect::<Vec<_>>()
    );
    assert_eq!(capped.findings.len(), 3, "3 轮各报一处新问题应保留 3 条");

    // 成本估算必须跟着轮数上界走。饱和轮数跑前未知，若按固定采样估算就会低估，
    // 让 --max-cost 放过实际会超支的运行；闸口工具宁可保守拦下，也不该给人成本惊喜。
    // 两次运行的 diff 完全相同，唯一变量是 max_rounds（10 vs 3）。
    let est_wide = converged.cost_estimate.as_ref().expect("应有成本估算");
    let est_narrow = capped.cost_estimate.as_ref().expect("应有成本估算");
    assert!(
        est_wide.est_input_tokens > est_narrow.est_input_tokens,
        "max_rounds=10 的估算应高于 max_rounds=3（上界语义），实际 {} vs {}",
        est_wide.est_input_tokens,
        est_narrow.est_input_tokens
    );
    assert_eq!(
        est_narrow.samples, 3,
        "security 线的估算基数应是轮数上界 max_rounds，而不是固定采样"
    );
}
