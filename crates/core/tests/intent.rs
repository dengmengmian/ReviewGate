//! 意图评审集成测试：验证 `--intent` 会触发独立的 intent Agent，
//! 并把缺失类 verdict 并入闸口判定。
//!
//! 与 pipeline.rs 一样切换进程 CWD，故本文件仅一个测试。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig};
use reviewgate_core::gate::GateDecision;
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, LlmResponse, Message, StopReason, ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::{run_review_with_client, ReviewOptions};
use std::collections::HashMap;
use std::process::Command;

struct IntentMock;

#[async_trait]
impl LlmClient for IntentMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        let is_judge = tools.iter().any(|t| t.name == "verdict");
        let is_intent = tools.iter().any(|t| t.name == "report_intent_finding");
        let content = if is_judge {
            vec![tool_use(
                "verdict",
                serde_json::json!({"real": true, "confidence": 0.9, "reason": "confirmed"}),
            )]
        } else if is_intent {
            vec![
                tool_use(
                    "report_intent_finding",
                    serde_json::json!({
                        "criterion": "C1",
                        "status": "missing",
                        "message": "dispatchRequest does not handle URL objects",
                        "confidence": 0.85
                    }),
                ),
                tool_use("task_done", serde_json::json!({})),
            ]
        } else {
            vec![tool_use("task_done", serde_json::json!({}))]
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
async fn intent_review_produces_missing_verdict() {
    let tmp = std::env::temp_dir().join(format!("rg_intent_{}", std::process::id()));
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
        "function f(id) {\n  return db.query(id);\n}\n",
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let cfg = mock_config();
    let mut opts = ReviewOptions::workspace(reviewgate_core::model::Dimension::ALL.to_vec());
    opts.intent = Some("- dispatchRequest must handle URL objects".into());
    let outcome = run_review_with_client(&cfg, &opts, &IntentMock)
        .await
        .expect("intent review should run");

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    // Intent 发现应被保留并进入闸口。
    let intent_findings: Vec<_> = outcome
        .findings
        .iter()
        .filter(|f| f.dimension == reviewgate_core::model::Dimension::Intent)
        .collect();
    assert!(
        !intent_findings.is_empty(),
        "应有 intent 发现: {:?}",
        outcome.findings
    );
    assert!(
        intent_findings
            .iter()
            .any(|f| { f.intent_status == Some(reviewgate_core::model::IntentStatus::Missing) }),
        "应有 missing verdict: {:?}",
        intent_findings
    );
    assert_eq!(outcome.decision, GateDecision::Block);
}
