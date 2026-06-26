//! 端到端：上下文不足/超大改动时**绝不静默放行**。
//!
//! 把 `max_input_tokens` 压到极小，模拟"diff 远超输入窗口"。预期：所有审查单元因超预算被跳过，
//! `incomplete=true`，且闸口在 `fail_on_incomplete=true`（默认）下**不得 PASS**（降级为 WARN）。
//!
//! 注：与 pipeline.rs 同理，本测试切换进程 CWD，故本文件**仅一个测试**。各集成测试文件是独立二进制。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig, ProviderConfig};
use reviewgate_core::gate::GateDecision;
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{LlmResponse, Message, StopReason, ToolDef, Usage};
use reviewgate_core::review::{run_review_with_client, ReviewOptions};
use std::collections::HashMap;
use std::process::Command;

/// 永远 task_done 的 mock——本测试根本不该走到它（单元应在派发前被跳过）。
struct NeverCalledLlm;

#[async_trait]
impl LlmClient for NeverCalledLlm {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![reviewgate_core::model::ContentBlock::ToolUse(
                reviewgate_core::model::ToolUse {
                    id: "t0".into(),
                    name: "task_done".into(),
                    input: serde_json::json!({}),
                },
            )],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        })
    }
    fn model(&self) -> &str {
        "mock"
    }
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

/// provider 带极小输入预算，逼审查单元超预算。
fn tiny_budget_config() -> Config {
    let mut providers = HashMap::new();
    providers.insert(
        "mock".to_string(),
        ProviderConfig {
            protocol: Default::default(),
            base_url: "x".into(),
            api_key: "x".into(),
            model: "m".into(),
            max_input_tokens: Some(1), // 1 token 预算：任何 diff 都超
        },
    );
    Config {
        provider: "mock".into(),
        providers,
        gate: GateConfig::default(), // fail_on_incomplete 默认 true
        business: BusinessConfig::default(),
    }
}

#[tokio::test]
async fn oversized_diff_never_silently_passes() {
    let tmp = std::env::temp_dir().join(format!("rg_incomplete_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 1;\n}\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    // 工作区改动（内容不重要，只要 diff 非空、超 1 token 预算）。
    std::fs::write(
        tmp.join("app.js"),
        "function f(id) {\n  return db.query(id);\n  // changed\n}\n",
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let cfg = tiny_budget_config();
    let opts = ReviewOptions::workspace(reviewgate_core::model::Dimension::ALL.to_vec());
    let outcome = run_review_with_client(&cfg, &opts, &NeverCalledLlm).await;

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let outcome = outcome.expect("run_review_with_client 应成功（优雅降级，不报错）");

    // 核心保证：未审完被显式标记。
    assert!(outcome.incomplete, "超预算应标记 incomplete");
    // 绝不静默 PASS：默认 fail_on_incomplete=true，0 发现也至少 WARN。
    assert_ne!(
        outcome.decision,
        GateDecision::Pass,
        "未审完绝不能 PASS，实际 {:?}",
        outcome.decision
    );
    assert_eq!(outcome.decision, GateDecision::Warn);
    // 留痕：有 oversized 告警。
    assert!(
        outcome.warnings.iter().any(|w| w.kind == "oversized"),
        "应有 oversized 告警，实际 {:?}",
        outcome.warnings
    );
}
