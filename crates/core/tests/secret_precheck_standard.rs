//! 标准 `review`（非 deep security）也必须跑确定性密钥预检。
//!
//! 密钥不能依赖模型方差：mock 故意不报任何问题，预检仍应 BLOCK。
//! 本文件切 CWD，只保留一个测试。

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

struct SilentLlm;

#[async_trait]
impl LlmClient for SilentLlm {
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
                    serde_json::json!({"real": false, "confidence": 0.9, "reason": "mock refute"}),
                )],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            });
        }
        Ok(LlmResponse {
            content: vec![tool_use("task_done", serde_json::json!({}))],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        })
    }
    fn model(&self) -> &str {
        "silent-mock"
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
async fn standard_review_blocks_hardcoded_secret_without_llm() {
    let tmp = std::env::temp_dir().join(format!("rg_secret_std_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("cfg.py"), "API = None\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    // 运行时拼 fixture，避免仓库历史被当成真密钥。
    let stripe = format!("STRIPE = 'sk_live_{}'\n", "A".repeat(24));
    std::fs::write(tmp.join("cfg.py"), stripe).unwrap();

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
    let outcome = run_review_with_client(&cfg, &opts, &SilentLlm).await;

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let outcome = outcome.expect("review should run");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.dimension == Dimension::Security
                && f.evidence.contains("deterministic secret")),
        "standard review must keep the secret precheck hit: {:#?}",
        outcome.findings
    );
    assert_eq!(
        outcome.decision,
        GateDecision::Block,
        "hardcoded live key must BLOCK even when the model reports nothing"
    );
}
