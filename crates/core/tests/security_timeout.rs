//! 饱和式 discovery 必须尊重 `--timeout` 作为**整体墙钟预算**，而不是每轮各给一份。
//!
//! 旧的 security 是并行跑固定采样，`--timeout` 天然就是总时长。饱和循环改成串行多轮后，
//! 若仍把 timeout 只传给单个 agent，用户设 `--timeout 200` 最坏会跑 6×200=1200s——
//! 与设定严重不符，且会把评测/CI 的时间预算彻底击穿。
//!
//! 超时停止属于「没审完」，必须标 incomplete：闸口的底线是不把没审完说成通过。
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
use std::time::Duration;

/// 每轮耗时 `delay`，且每轮都报**新**问题——永远不会自然饱和，只能靠预算或轮数上限刹车。
struct SlowMock {
    rounds: Arc<AtomicUsize>,
    delay: Duration,
}

#[async_trait]
impl LlmClient for SlowMock {
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
                    serde_json::json!({"real": true, "confidence": 0.95, "reason": "ok"}),
                )],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            });
        }
        tokio::time::sleep(self.delay).await;
        let n = self.rounds.fetch_add(1, Ordering::SeqCst) as u64;
        let line = 2 + n; // 每轮换一行 → 去重折不掉 → 池子持续增长
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

#[tokio::test]
async fn saturation_respects_timeout_as_a_total_budget_not_per_round() {
    let tmp = std::env::temp_dir().join(format!("rg_sec_timeout_{}", std::process::id()));
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
        "function f(input) {\n  const x2 = eval(input);\n  const x3 = eval(input);\n  \
         const x4 = eval(input);\n  const x5 = eval(input);\n  const x6 = eval(input);\n  \
         const x7 = eval(input);\n  return x2;\n}\n",
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let rounds = Arc::new(AtomicUsize::new(0));
    let mut review = ReviewOptions::workspace(vec![Dimension::Security]);
    review.judge = false;
    review.write_metrics = false;
    // 总预算 1s，每轮耗时 400ms → 最多跑得下 2~3 轮，绝不该跑满 8 轮。
    review.timeout = Some(Duration::from_millis(1000));
    let mut opts = SecurityOptions::new(review);
    opts.stop_after_no_new = 99; // 永不自然收敛
    opts.max_rounds = 8;

    let started = std::time::Instant::now();
    let outcome = run_security_with_client(
        &mock_config(),
        &opts,
        &SlowMock {
            rounds: rounds.clone(),
            delay: Duration::from_millis(400),
        },
    )
    .await
    .expect("超时应优雅收尾，不是报错");
    let elapsed = started.elapsed();

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let ran = rounds.load(Ordering::SeqCst);

    // 核心断言：总耗时必须受 timeout 约束，而不是 轮数 × timeout。
    // 给足宽裕量（3x）避免 CI 抖动误报，但 8 轮 × 400ms = 3.2s 会稳稳超过。
    assert!(
        elapsed < Duration::from_millis(3000),
        "总耗时必须受 --timeout 约束；实际跑了 {ran} 轮、耗时 {elapsed:?}"
    );
    assert!(
        ran < 8,
        "预算耗尽后必须停止，不该跑满 max_rounds=8；实际 {ran} 轮"
    );
    assert!(
        ran >= 1,
        "预算再紧也要至少跑完一轮，否则等于什么都没审；实际 {ran} 轮"
    );

    // 因预算耗尽而提前停 = 没审完，绝不能让闸口误读成通过。
    assert!(
        outcome.incomplete,
        "预算耗尽提前停属于未审完，必须标 incomplete（跑了 {ran} 轮）"
    );
}
