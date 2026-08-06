//! 质量回归护栏：饱和式 discovery 的召回**不得低于**旧的固定 2 轮采样。
//!
//! 真实 LLM 的召回是 flaky 的——同一处问题不是每轮都报得出来，这正是当初上多采样、
//! 现在换饱和策略要解决的问题。这里用确定性 mock 精确复刻该特性：5 处真实问题，
//! 每轮只报得出其中 2 处，按轮次轮转。
//!
//! 于是两种策略的召回可以被严格比较，且不依赖真实 LLM（无采样噪声、可进 CI）：
//! - 旧行为：固定跑 2 轮 → 只能拿到前两轮报出来的
//! - 新行为：跑到连续 2 轮无新发现 → 能把轮转覆盖到的都挖出来
//!
//! 注：本测试切换进程 CWD 到临时仓库，故本文件**仅一个测试**避免 CWD 竞争。

use async_trait::async_trait;
use reviewgate_core::config::{BusinessConfig, Config, GateConfig};
use reviewgate_core::llm::LlmClient;
use reviewgate_core::model::{
    ContentBlock, Dimension, LlmResponse, Message, StopReason, ToolDef, ToolUse, Usage,
};
use reviewgate_core::review::ReviewOptions;
use reviewgate_core::security::{run_security_with_client, SecurityOptions};
use std::collections::{BTreeSet, HashMap};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 文件里埋的真实问题总数（行号 2..=6）。
const TRUE_ISSUES: u64 = 5;
/// 每轮 LLM 报得出来的数量——模拟 flaky 召回。
const PER_ROUND: u64 = 2;

/// 每轮报 `PER_ROUND` 处问题，按轮次轮转，确定性可复现。
struct FlakyRecallMock {
    rounds: Arc<AtomicUsize>,
    /// 证伪 Judge 的调用次数——用来确认饱和累积的发现没有绕过证伪。
    verdicts: Arc<AtomicUsize>,
}

#[async_trait]
impl LlmClient for FlakyRecallMock {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        tools: &[ToolDef],
    ) -> anyhow::Result<LlmResponse> {
        if tools.iter().any(|t| t.name == "verdict") {
            self.verdicts.fetch_add(1, Ordering::SeqCst);
            return Ok(LlmResponse {
                content: vec![tool_use(
                    "verdict",
                    serde_json::json!({"real": true, "confidence": 0.95, "reason": "确认"}),
                )],
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            });
        }
        let n = self.rounds.fetch_add(1, Ordering::SeqCst) as u64;
        let mut content = Vec::new();
        for k in 0..PER_ROUND {
            // 轮转：第 n 轮报第 (n*PER_ROUND + k) mod TRUE_ISSUES 处问题。
            let issue = (n * PER_ROUND + k) % TRUE_ISSUES;
            let line = 2 + issue; // 文件里第 2..6 行各埋一处
            content.push(tool_use(
                "report_finding",
                serde_json::json!({
                    "path": "app.js",
                    "line_start": line,
                    "line_end": line,
                    "existing_code": format!("  const x{line} = eval(input);"),
                    "message": format!("eval 注入 #{line}"),
                    "severity": "high",
                    "confidence": 0.9
                }),
            ));
        }
        content.push(tool_use("task_done", serde_json::json!({})));
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
        id: format!("{name}_{}", input["line_start"].as_u64().unwrap_or(0)),
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
        issue_review: Default::default(),
        exclude: Default::default(),
        severity_labels: Vec::new(),
    }
}

fn opts(stop_after_no_new: usize, max_rounds: usize) -> SecurityOptions {
    opts_with_judge(stop_after_no_new, max_rounds, false)
}

fn opts_with_judge(stop_after_no_new: usize, max_rounds: usize, judge: bool) -> SecurityOptions {
    let mut review = ReviewOptions::workspace(vec![Dimension::Security]);
    review.judge = judge; // 测召回时关掉，免得证伪阶段掺进来
    review.write_metrics = false;
    let mut o = SecurityOptions::new(review);
    o.stop_after_no_new = stop_after_no_new;
    o.max_rounds = max_rounds;
    o
}

fn lines_of(outcome: &reviewgate_core::review::ReviewOutcome) -> BTreeSet<u32> {
    outcome.findings.iter().map(|f| f.start_line).collect()
}

#[tokio::test]
async fn saturating_recall_is_never_worse_than_the_old_fixed_two_samples() {
    let tmp = std::env::temp_dir().join(format!("rg_sec_recall_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);
    std::fs::write(tmp.join("app.js"), "function f() {\n  return 1;\n}\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "base"]);
    // 5 处真实问题，分别在第 2..6 行。
    std::fs::write(
        tmp.join("app.js"),
        "function f(input) {\n  const x2 = eval(input);\n  const x3 = eval(input);\n  \
         const x4 = eval(input);\n  const x5 = eval(input);\n  const x6 = eval(input);\n  \
         return x2;\n}\n",
    )
    .unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    // --- 旧行为：固定跑满 2 轮（stop_after_no_new 设大，保证不提前收敛）---
    let old_rounds = Arc::new(AtomicUsize::new(0));
    let old = run_security_with_client(
        &mock_config(),
        &opts(99, 2),
        &FlakyRecallMock {
            rounds: old_rounds.clone(),
            verdicts: Arc::new(AtomicUsize::new(0)),
        },
    )
    .await
    .expect("固定 2 轮应成功");

    // --- 新行为：饱和，连续 2 轮无新发现才停 ---
    let new_rounds = Arc::new(AtomicUsize::new(0));
    let new = run_security_with_client(
        &mock_config(),
        &opts(2, 6),
        &FlakyRecallMock {
            rounds: new_rounds.clone(),
            verdicts: Arc::new(AtomicUsize::new(0)),
        },
    )
    .await
    .expect("饱和应成功");

    // --- 精度防线：饱和累积的发现必须**全部**过证伪 Judge，一条都不能绕过 ---
    // 多跑轮次会带进更多候选，若有任何一条能绕过证伪直达闸口，精度就被这次改动拖低了。
    let judged_rounds = Arc::new(AtomicUsize::new(0));
    let judged_verdicts = Arc::new(AtomicUsize::new(0));
    let judged = run_security_with_client(
        &mock_config(),
        &opts_with_judge(2, 6, true),
        &FlakyRecallMock {
            rounds: judged_rounds.clone(),
            verdicts: judged_verdicts.clone(),
        },
    )
    .await
    .expect("开启 judge 的饱和应成功");

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let old_lines = lines_of(&old);
    let new_lines = lines_of(&new);

    // 核心质量断言：饱和召回是旧行为的超集——绝不能丢掉旧策略能找到的问题。
    assert!(
        old_lines.is_subset(&new_lines),
        "饱和召回必须覆盖旧固定采样找到的全部问题；旧={old_lines:?} 新={new_lines:?}"
    );

    // 旧策略 2 轮 × 每轮 2 处 = 只能覆盖 4 处，漏掉 1 处。
    assert_eq!(
        old_lines.len(),
        4,
        "固定 2 轮在 flaky 召回下只能拿到 4/5，实际 {old_lines:?}"
    );
    // 饱和会一直挖到连续 2 轮无新，5 处全拿到。
    assert_eq!(
        new_lines.len(),
        TRUE_ISSUES as usize,
        "饱和应挖满 5/5，实际 {new_lines:?}（跑了 {} 轮）",
        new_rounds.load(Ordering::SeqCst)
    );

    // 召回提升不能以「永远跑满上限」为代价——真饱和了就该停，且不标 incomplete。
    assert!(
        new_rounds.load(Ordering::SeqCst) < 6,
        "应在撞上限前自然收敛，实际跑满了 {} 轮",
        new_rounds.load(Ordering::SeqCst)
    );
    assert!(
        !new.incomplete,
        "自然收敛不该标 incomplete（那会让闸口无谓地不给 PASS）"
    );

    // 精度防线：证伪调用次数必须等于去重后的发现数——每条都被独立证伪过，没有漏网的。
    let judged_lines = lines_of(&judged);
    assert_eq!(
        judged_lines.len(),
        TRUE_ISSUES as usize,
        "开 judge 后召回不该变化，实际 {judged_lines:?}"
    );
    assert_eq!(
        judged_verdicts.load(Ordering::SeqCst),
        judged_lines.len(),
        "饱和累积的每条发现都必须过一次证伪 Judge——多跑轮次不能让候选绕过精度防线"
    );
}
