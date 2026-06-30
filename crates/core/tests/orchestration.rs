//! 编排主路径的针对性集成测试（补 `run_review_with_client` 的闸口/incomplete 覆盖）。
//!
//! 与 `pipeline.rs` 互补：那条测「有真问题 → BLOCK」的正路；这里测两条此前没覆盖的路径：
//! 1) **无发现 → PASS**（且 incomplete=false，files_changed 正确）。
//! 2) **Agent 请求失败 → incomplete=true**（绝不把"没审完"洗成 PASS 放行）。
//!
//! 同一进程内 CWD 是全局的，故本文件仅一个测试、内部顺序跑多场景，避免并行 chdir 竞争。

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

/// 维度 Agent 一轮 task_done（零发现）；judge 不会被触发（无候选）。
struct NoFindingMock;

#[async_trait]
impl LlmClient for NoFindingMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::ToolUse(ToolUse {
                id: "task_done_0".into(),
                name: "task_done".into(),
                input: serde_json::json!({}),
            })],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
        })
    }
    fn model(&self) -> &str {
        "mock"
    }
}

/// 每次 LLM 调用都失败 → 每个维度 Agent 都 RequestFailed → 整体 incomplete。
struct FailingMock;

#[async_trait]
impl LlmClient for FailingMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("simulated provider error")
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

fn mock_config() -> Config {
    Config {
        provider: "mock".into(),
        providers: HashMap::new(),
        gate: GateConfig::default(),
        business: BusinessConfig::default(),
    }
}

#[tokio::test]
async fn orchestration_pass_and_incomplete_paths() {
    // 临时 git 仓库：提交干净文件，再改出一处工作区改动（内容本身无关，只需有 diff）。
    let tmp = std::env::temp_dir().join(format!("rg_orch_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 1;\n}\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 2;\n}\n").unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let cfg = mock_config();

    // 场景 1：零发现 → PASS，且未审完标记为 false，文件数正确。
    let opts = ReviewOptions::workspace(Dimension::ALL.to_vec());
    let pass = run_review_with_client(&cfg, &opts, &NoFindingMock)
        .await
        .expect("run should succeed");
    // 场景 2：每次请求失败 → 每维度 RequestFailed → incomplete=true、warnings 非空。
    let opts2 = ReviewOptions::workspace(Dimension::ALL.to_vec());
    let failed = run_review_with_client(&cfg, &opts2, &FailingMock)
        .await
        .expect("run should succeed even when every agent request fails");

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    // 场景 1 断言。
    assert!(pass.findings.is_empty(), "无 report_finding 应零发现");
    assert_eq!(pass.decision, GateDecision::Pass, "零发现应 PASS");
    assert!(!pass.incomplete, "全维度正常完成，不应标记未审完");
    assert_eq!(pass.files_changed, 1);

    // 场景 2 断言：核心不变量——请求全失败不能洗成"通过且完整"。
    assert!(failed.incomplete, "所有维度请求失败应标记 incomplete");
    assert!(!failed.warnings.is_empty(), "应记录每个失败维度的告警");
    assert!(failed.findings.is_empty(), "失败时无发现");
}
