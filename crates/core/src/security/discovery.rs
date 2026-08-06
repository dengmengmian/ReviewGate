//! 饱和式 discovery：`reviewgate security` 的召回阶段。
//!
//! 与 review 线的固定 `samples` fan-out 不同，这里反复跑同一轮 fan-out，直到连续
//! 若干轮不再产出新发现（真饱和），或撞上轮数上限（成本兜底）。
//!
//! 「有没有新发现」用候选池去重后的规模变化来判定，复用 review 线的 `dedupe`，
//! 不引入第二套指纹。轮内去重只用于收敛判定；正式的行号校验与去重仍在后续阶段。

use crate::llm::LlmClient;
use crate::model::Dimension;
use crate::review::stages::{absorb_round, fanout_tasks, RunCtx};
use crate::review::{dedupe, ReviewWarning};
use crate::security::Saturation;
use futures::stream::StreamExt;

/// 跑饱和式 discovery，把发现累积进 `c.findings`。
///
/// 撞上轮数上限而非自然饱和时，标记 `incomplete`——闸口的底线是不把「可能没挖完」
/// 说成「通过」，所以这里必须让上限触顶可见，而不是静默返回。
pub(crate) async fn discover_saturating(
    c: &mut RunCtx<'_>,
    client: &dyn LlmClient,
    mut sat: Saturation,
) {
    let concurrency = c.opts.fanout_concurrency.max(1);
    let verbose = c.opts.verbose;

    // `--timeout` 是**整体**墙钟预算，不是每轮各给一份。串行多轮若各拿一份完整 timeout，
    // 用户设 200s 最坏会跑 6×200s，与设定严重不符。
    let started = std::time::Instant::now();
    let budget = c.opts.timeout;
    let remaining = |elapsed: std::time::Duration| -> Option<std::time::Duration> {
        budget.map(|b| b.saturating_sub(elapsed))
    };
    let mut budget_exhausted = false;

    while sat.should_continue() {
        // 每轮只给剩余预算，否则最后一轮会冲出总预算。
        let round_timeout = remaining(started.elapsed());
        let tasks = fanout_tasks(c, client, 1, round_timeout);
        let results = futures::stream::iter(tasks)
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await;
        let round_findings = absorb_round(c, results);

        // 并入候选池再去重：池子规模是否增长 = 这一轮有没有挖到新东西。
        let before = c.findings.len();
        c.findings.extend(round_findings);
        c.findings = dedupe(std::mem::take(&mut c.findings));
        let grew = c.findings.len() > before;
        sat.record(grew);

        if verbose {
            eprintln!(
                "  [saturation] round {}: pool {} -> {} ({})",
                sat.rounds_done(),
                before,
                c.findings.len(),
                if grew { "new findings" } else { "no new" }
            );
        }

        // 预算耗尽就收手。检查放在**跑完一轮之后**：预算再紧也要审完一轮，
        // 否则等于什么都没做还报了个「未审完」。
        if remaining(started.elapsed()).is_some_and(|r| r.is_zero()) {
            budget_exhausted = true;
            if verbose {
                eprintln!(
                    "  [saturation] wall-clock budget exhausted after {} round(s)",
                    sat.rounds_done()
                );
            }
            break;
        }
    }

    if budget_exhausted {
        // 预算耗尽 = 没审完。闸口的底线是不把没审完说成通过。
        c.incomplete = true;
        c.warnings.push(
            ReviewWarning::new(
                Dimension::Security.as_str(),
                "timed_out",
                format!(
                    "saturating discovery stopped after {} round(s): the --timeout wall-clock budget ran out before findings stopped growing; coverage may be partial",
                    sat.rounds_done()
                ),
            )
            .with_advice("Raise --timeout (0 = unlimited), or split the change and re-run"),
        );
    } else if sat.stopped_by_cap() {
        // 撞上限说明发现还在增长就被叫停了——可能还有没挖到的问题。
        c.incomplete = true;
        c.warnings.push(
            ReviewWarning::new(
                Dimension::Security.as_str(),
                "incomplete",
                format!(
                    "saturating discovery hit the {} round cap while still finding new issues; coverage may be partial",
                    sat.rounds_done()
                ),
            )
            .with_advice("Raise --max-rounds, or split the change and re-run"),
        );
    } else if verbose {
        eprintln!(
            "  [saturation] converged after {} round(s)",
            sat.rounds_done()
        );
    }
}
