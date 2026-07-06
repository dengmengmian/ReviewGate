//! 端到端：增量复审。第一轮审查 + 落缓存；第二轮 diff 逐字节不变 → 全部命中缓存，
//! **零 LLM 调用**却复现同样的 BLOCK 与发现。用 mock LLM + 临时 git 仓库，不联网。
//!
//! 注：本测试切换进程 CWD 到临时仓库，故本文件**仅一个测试**避免 CWD 竞争。

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

/// 计数所有 complete 调用（agent + judge）。判定 mock 同 pipeline.rs。
struct CountingMock {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LlmClient for CountingMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let is_judge = tools.iter().any(|t| t.name == "verdict");
        let content = if is_judge {
            vec![tool_use(
                "verdict",
                serde_json::json!({"real": true, "confidence": 0.95, "reason": "确认"}),
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

async fn run(calls: Arc<AtomicUsize>) -> reviewgate_core::review::ReviewOutcome {
    let cfg = mock_config();
    let mut opts = ReviewOptions::workspace(Dimension::ALL.to_vec());
    opts.incremental = true;
    run_review_with_client(&cfg, &opts, &CountingMock { calls })
        .await
        .expect("run_review_with_client 应成功")
}

#[tokio::test]
async fn second_run_reuses_cache_with_zero_llm_calls() {
    let tmp = std::env::temp_dir().join(format!("rg_incremental_{}", std::process::id()));
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

    // 第一轮：无缓存，真实审查，落缓存。
    let calls = Arc::new(AtomicUsize::new(0));
    let first = run(calls.clone()).await;
    let first_calls = calls.load(Ordering::SeqCst);

    // 第二轮：diff 逐字节不变 → 全部命中缓存。
    calls.store(0, Ordering::SeqCst);
    let second = run(calls.clone()).await;
    let second_calls = calls.load(Ordering::SeqCst);

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(first_calls > 0, "第一轮应有真实 LLM 调用");
    assert_eq!(
        second_calls, 0,
        "第二轮 diff 未变应全命中缓存、零 LLM 调用，实际 {second_calls}"
    );
    // 结论一致：仍 BLOCK、发现集合相同。
    assert_eq!(first.decision, GateDecision::Block);
    assert_eq!(second.decision, GateDecision::Block, "复用缓存后判定应不变");
    assert_eq!(
        second.findings.len(),
        first.findings.len(),
        "复用缓存后发现数应不变"
    );
    assert_eq!(second.findings[0].path, "app.js");
}
