//! `--timeout` 必须是整次审查（含 Judge）的真预算，而不是只约束 Agent。
//!
//! 旧行为：Agent 跑完后 Judge 无墙钟，慢证伪可以把 `--timeout 50ms` 拖成数秒。
//! 本文件切 CWD，只保留一个测试。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig};
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, Dimension, LlmResponse, Message, StopReason, ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::{run_review_with_client, ReviewOptions};
use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

struct AgentFastJudgeSlow;

#[async_trait]
impl LlmClient for AgentFastJudgeSlow {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        if tools.iter().any(|t| t.name == "verdict") {
            tokio::time::sleep(Duration::from_secs(3)).await;
            return Ok(LlmResponse {
                content: vec![tool_use(
                    "verdict",
                    serde_json::json!({"real": true, "confidence": 0.95, "reason": "too late"}),
                )],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            });
        }
        Ok(LlmResponse {
            content: vec![
                tool_use(
                    "report_finding",
                    serde_json::json!({
                        "path": "app.js",
                        "line_start": 2,
                        "line_end": 2,
                        "existing_code": "return db.query(\"SELECT * FROM t WHERE id=\" + id);",
                        "message": "SQL injection",
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
        "fast-then-slow"
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
        .expect("git")
        .success();
    assert!(ok, "git {args:?} failed");
}

#[tokio::test]
async fn review_timeout_cuts_off_judge_and_marks_incomplete() {
    let tmp = std::env::temp_dir().join(format!("rg_timeout_budget_{}", std::process::id()));
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
        "function f(id) {\n  return db.query(\"SELECT * FROM t WHERE id=\" + id);\n}\n",
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let cfg = Config {
        provider: "mock".into(),
        providers: HashMap::new(),
        gate: GateConfig::default(),
        business: BusinessConfig::default(),
        issue_review: Default::default(),
        exclude: Default::default(),
        severity_labels: Vec::new(),
    };
    let mut opts = ReviewOptions::workspace(vec![Dimension::Security]);
    opts.write_metrics = false;
    opts.timeout = Some(Duration::from_millis(400));
    opts.started = Some(Instant::now());

    let t0 = Instant::now();
    let outcome = run_review_with_client(&cfg, &opts, &AgentFastJudgeSlow).await;
    let elapsed = t0.elapsed();

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let outcome = outcome.expect("review should return, not hang");
    assert!(
        elapsed < Duration::from_secs(2),
        "judge must not ignore --timeout; elapsed {elapsed:?}"
    );
    assert!(
        outcome.incomplete,
        "judge timeout must mark the review incomplete, not pretend it finished"
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.kind == "timed_out" || w.message.to_lowercase().contains("timeout")),
        "must surface a timeout warning: {:#?}",
        outcome.warnings
    );
}
