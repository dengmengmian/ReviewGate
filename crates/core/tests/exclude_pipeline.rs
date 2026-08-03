//! 路径排除的端到端行为：被排除的文件不进 LLM，但必须出现在结果里（可核对，不静默）。
//!
//! 注：本测试切换进程 CWD 到临时仓库，故本文件**仅一个测试**避免 CWD 竞争。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, ExcludeConfig, GateConfig};
use reviewgate_core::diff::ExcludeReason;
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, LlmResponse, Message, StopReason, ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::{run_review_with_client, ReviewOptions};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// 记录每次请求里出现过的文件路径，用来验证被排除的文件确实没进 prompt。
struct RecordingLlm {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl LlmClient for RecordingLlm {
    async fn complete(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        let text = format!("{messages:?}");
        self.seen.lock().unwrap().push(text);
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

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git 可执行")
        .success();
    assert!(ok, "git {args:?} 失败");
}

#[tokio::test]
async fn excluded_files_are_skipped_and_reported() {
    let tmp = std::env::temp_dir().join(format!("rg_exclude_pipe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join("docs")).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 1;\n}\n").unwrap();
    std::fs::write(tmp.join("Cargo.lock"), "# lock v1\n").unwrap();
    std::fs::write(tmp.join("docs/guide.md"), "# guide\n").unwrap();
    // .reviewgateignore 先入库：它本身**不**被自动排除（改动它等于改动闸口范围，必须可审）。
    std::fs::write(tmp.join(".reviewgateignore"), "testdata/\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    // 三个文件都改：只有 app.js 该被审。
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 2;\n}\n").unwrap();
    std::fs::write(tmp.join("Cargo.lock"), "# lock v1\n# bumped\n").unwrap();
    std::fs::write(tmp.join("docs/guide.md"), "# guide\nmore\n").unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    let cfg = Config {
        provider: "mock".into(),
        providers: HashMap::new(),
        gate: GateConfig::default(),
        business: BusinessConfig::default(),
        issue_review: Default::default(),
        exclude: ExcludeConfig {
            patterns: vec!["docs/**".into()],
            builtin: true,
        },
        severity_labels: Vec::new(),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let client = RecordingLlm { seen: seen.clone() };
    let opts = ReviewOptions::workspace(vec![reviewgate_core::model::Dimension::Security]);
    let outcome = run_review_with_client(&cfg, &opts, &client).await;

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let outcome = outcome.expect("run_review_with_client 应成功");

    assert_eq!(outcome.files_changed, 1, "只有 app.js 应计入被审文件");

    let excluded = &outcome.excluded;
    assert_eq!(excluded.len(), 2, "应报告 2 个被排除文件：{excluded:?}");
    let lock = excluded
        .iter()
        .find(|e| e.path == "Cargo.lock")
        .expect("Cargo.lock 应被内置规则排除");
    assert_eq!(lock.reason, ExcludeReason::Builtin);
    let doc = excluded
        .iter()
        .find(|e| e.path == "docs/guide.md")
        .expect("docs/** 应被配置规则排除");
    assert_eq!(doc.reason, ExcludeReason::Config);
    // 覆盖快照描述的是"审了什么"，被排除的文件不能混进去冒充已覆盖。
    if let Some(cov) = &outcome.coverage {
        assert!(
            !cov.changed_paths.iter().any(|p| p == "Cargo.lock"),
            "被排除文件不该出现在 coverage.changed_paths：{:?}",
            cov.changed_paths
        );
        assert!(
            !cov.covered_paths.iter().any(|p| p == "docs/guide.md"),
            "被排除文件不该被标成已覆盖：{:?}",
            cov.covered_paths
        );
    }

    let seen = seen.lock().unwrap();
    let all = seen.join("\n");
    assert!(!all.is_empty(), "应至少发起一次 LLM 请求");
    assert!(all.contains("app.js"), "app.js 应进入 prompt");
    assert!(!all.contains("Cargo.lock"), "被排除文件不应进入 prompt");
    assert!(!all.contains("guide.md"), "被排除文件不应进入 prompt");
}
