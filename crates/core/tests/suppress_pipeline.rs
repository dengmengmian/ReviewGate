//! 端到端：`.reviewgate/ignore` 命中的高危误报被抑制——闸口由 BLOCK 降为 PASS，
//! 发现仍以 filtered 保留（可 `--show-filtered` 展开）。用 mock LLM + 临时 git 仓库，不联网。
//!
//! 注：本测试会切换进程 CWD 到临时仓库，故本文件**仅一个测试**避免 CWD 竞争。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig};
use reviewgate_core::gate::GateDecision;
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, Dimension, Finding, LlmResponse, Message, Reachability, Severity, StopReason,
    ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::{fingerprint, run_review_with_client, ReviewOptions};
use std::collections::HashMap;
use std::process::Command;

const INJECTION_CODE: &str = "return db.query(\"SELECT * FROM t WHERE id=\" + id);";

/// 与 pipeline.rs 同款脚本化 mock：judge 确认 real；各维度报同一处 SQL 注入。
struct MockLlm;

#[async_trait]
impl LlmClient for MockLlm {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        let is_judge = tools.iter().any(|t| t.name == "verdict");
        let content = if is_judge {
            vec![tool_use(
                "verdict",
                serde_json::json!({"real": true, "confidence": 0.95, "reason": "确认注入"}),
            )]
        } else {
            vec![
                tool_use(
                    "report_finding",
                    serde_json::json!({
                        "path": "app.js",
                        "line_start": 2,
                        "line_end": 2,
                        "existing_code": INJECTION_CODE,
                        "message": "SQL 注入：用户输入拼接进查询",
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

/// 去重后 best 维度为 dims 首个（Security），existing_code 即上报的注入行——
/// 据此复算该发现的指纹，写进 ignore。
fn expected_fingerprint() -> String {
    let f = Finding {
        dimension: Dimension::Security,
        confidence: 0.9,
        severity: Severity::High,
        path: "app.js".into(),
        start_line: 2,
        end_line: 2,
        message: String::new(),
        existing_code: INJECTION_CODE.into(),
        evidence: String::new(),
        suggestion: None,
        suggestion_code: String::new(),
        reachability: Reachability::Unknown,
        filtered: false,
        agreed_dimensions: 1,
        criterion: None,
        intent_status: None,
    };
    fingerprint(&f)
}

#[tokio::test]
async fn ignore_file_suppresses_block_to_pass() {
    let tmp = std::env::temp_dir().join(format!("rg_suppress_{}", std::process::id()));
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

    // 关键：把该发现的指纹写进 .reviewgate/ignore（团队确认为误报）。
    std::fs::create_dir_all(tmp.join(".reviewgate")).unwrap();
    std::fs::write(
        tmp.join(".reviewgate").join("ignore"),
        format!("# 已确认非注入\n{}\n", expected_fingerprint()),
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();
    let cfg = mock_config();
    let opts = ReviewOptions::workspace(Dimension::ALL.to_vec());
    let outcome = run_review_with_client(&cfg, &opts, &MockLlm).await;
    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let outcome = outcome.expect("run_review_with_client 应成功");

    // 被抑制 → 闸口不再 BLOCK（无其它发现即 PASS）。
    assert_eq!(
        outcome.decision,
        GateDecision::Pass,
        "命中 ignore 的高危误报不应再 BLOCK，findings={:#?}",
        outcome.findings
    );
    // 发现未被静默丢弃：仍在结果里、标记为 filtered（可展开审计）。
    assert_eq!(outcome.findings.len(), 1, "被抑制项应保留而非删除");
    assert!(outcome.findings[0].filtered, "被抑制项应标 filtered");
}
