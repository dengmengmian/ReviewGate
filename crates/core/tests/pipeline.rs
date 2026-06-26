//! 端到端编排集成测试：用 mock LLM + 临时 git 仓库跑通整条
//! `diff 解析 → 多维 Agent → 行号校验 → 去重 → 证伪 Judge → 跨维加分 → 闸口`，不联网。
//!
//! 注：本测试会切换进程 CWD 到临时仓库，故保持本文件**仅一个测试**避免 CWD 竞争
//! （集成测试各文件是独立二进制，不影响 lib 单测）。

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

/// 脚本化 mock：
/// - judge 调用（tools 含 `verdict`）→ 返回 real=true（确认是真问题）。
/// - 维度 Agent 调用 → 一轮内 report_finding（指向第 2 行的 SQL 注入）+ task_done。
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
                        "existing_code": "return db.query(\"SELECT * FROM t WHERE id=\" + id);",
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
        gate: GateConfig::default(), // block ≥ 0.8
        business: BusinessConfig::default(),
    }
}

#[tokio::test]
async fn full_pipeline_blocks_real_finding() {
    // 1) 临时 git 仓库：提交干净 app.js，再改出带 SQL 注入的工作区改动。
    let tmp = std::env::temp_dir().join(format!("rg_pipeline_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 1;\n}\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    // 工作区改动：第 2 行变成 SQL 注入（与 mock 的 existing_code 对齐）。
    std::fs::write(
        tmp.join("app.js"),
        "function f(id) {\n  return db.query(\"SELECT * FROM t WHERE id=\" + id);\n}\n",
    )
    .unwrap();

    // 2) 切到该仓库跑全链路（仅本集成二进制单测，无 CWD 竞争）。
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let cfg = mock_config();
    let opts = ReviewOptions::workspace(reviewgate_core::model::Dimension::ALL.to_vec());
    let outcome = run_review_with_client(&cfg, &opts, &MockLlm).await;

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let outcome = outcome.expect("run_review_with_client 应成功");

    // 3) 断言整条编排：5 维度各报同一处 → 去重为 1；judge real=true 保留；置信度高 → BLOCK。
    assert_eq!(
        outcome.findings.len(),
        1,
        "5 维度同处发现应去重为 1，实际：{:#?}",
        outcome.findings
    );
    let f = &outcome.findings[0];
    assert_eq!(f.start_line, 2, "行号应校验/兜底到第 2 行");
    assert!(
        f.confidence >= 0.9,
        "judge 确认 + 跨维加分后应高置信，实际 {}",
        f.confidence
    );
    assert!(
        f.agreed_dimensions >= 2,
        "应记录多维度交叉印证，实际 {}",
        f.agreed_dimensions
    );
    assert_eq!(
        outcome.decision,
        GateDecision::Block,
        "高置信真问题应 BLOCK"
    );
    assert_eq!(outcome.files_changed, 1);
}
