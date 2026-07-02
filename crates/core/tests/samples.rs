//! 多采样（samples > 1）集成测试：单单元时每个维度跑多次，取并集后去重。
//!
//! 与 pipeline.rs 一样切换进程 CWD，故本文件仅一个测试。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig};
use reviewgate_core::gate::GateDecision;
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, Dimension, LlmResponse, Message, StopReason, ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::{run_review_with_client, ReviewOptions};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 每次调用都报同一处 SQL 注入，并统计调用次数。
struct SamplingMock {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LlmClient for SamplingMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        let is_judge = tools.iter().any(|t| t.name == "verdict");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let content = if is_judge {
            vec![tool_use(
                "verdict",
                serde_json::json!({"real": true, "confidence": 0.95, "reason": "confirmed"}),
            )]
        } else {
            vec![
                tool_use(
                    "report_finding",
                    serde_json::json!({
                        "path": "app.js",
                        "line_start": 2,
                        "line_end": 2,
                        "existing_code": "return db.query(\"SELECT * FROM t WHERE id=\" + id);",
                        "message": "SQL 注入",
                        "severity": "high",
                        "confidence": 0.9
                    }),
                ),
                tool_use("task_done", serde_json::json!({})),
            ]
        };
        Ok(LlmResponse {
            content,
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
    }
}

#[tokio::test]
async fn samples_union_and_dedupe() {
    let tmp = std::env::temp_dir().join(format!("rg_samples_{}", std::process::id()));
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

    let cfg = mock_config();
    let mut opts = ReviewOptions::workspace(Dimension::ALL.to_vec());
    opts.samples = 2;
    opts.judge_concurrency = 8;
    opts.fanout_concurrency = 12;
    let calls = Arc::new(AtomicUsize::new(0));
    let client = SamplingMock {
        calls: calls.clone(),
    };
    let outcome = run_review_with_client(&cfg, &opts, &client)
        .await
        .expect("samples review should run");

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    // 4 个默认维度 × 2 样本 = 8 次 Agent 调用，Judge 再对去重后的 1 条调用 1 次。
    assert!(
        outcome.findings.len() == 1,
        "各维度报同一处应去重为 1: {:?}",
        outcome.findings
    );
    assert_eq!(outcome.findings[0].start_line, 2);
    assert_eq!(outcome.decision, GateDecision::Block);
    // 4 个默认维度 × 2 样本 = 至少 8 次非 judge LLM 调用（Style 已移出默认集）。
    assert!(
        calls.load(Ordering::SeqCst) >= 8,
        "samples=2 应至少调用 8 次，实际 {}",
        calls.load(Ordering::SeqCst)
    );
}
