//! 跑前成本估算：按单元 × 维度 × 采样粗算 token 上界，可选换算 USD。
//!
//! 启发式偏保守（宁可高估），仅用于预算守卫与用户预期，不替代 API 回传的真实 Usage。

use crate::diff::Diff;
use crate::model::Dimension;
use crate::review::units::{plan_units, ReviewUnit};
use serde::Serialize;

/// 一次审查的跑前成本估算。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CostEstimate {
    /// 审查单元数（大 PR 按目录装箱）。
    pub units: usize,
    /// 可审单元数（排除 oversized 跳过）。
    pub reviewable_units: usize,
    /// 维度数。
    pub dimensions: usize,
    /// 每维采样次数（多单元时强制 1）。
    pub samples: usize,
    /// fan-out Agent 数 ≈ reviewable_units × dimensions × samples。
    pub fanout_agents: usize,
    /// 估算输入 token 上界（含多轮工具放大）。
    pub est_input_tokens: u64,
    /// 估算输出 token 上界。
    pub est_output_tokens: u64,
    /// 可选 USD 上界（需配置单价）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_cost_usd: Option<f64>,
    /// 人类可读摘要一行。
    pub summary: String,
}

/// 输入/输出每百万 token 单价（USD）。
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenPrices {
    pub per_mtok_input: Option<f64>,
    pub per_mtok_output: Option<f64>,
}

/// 工具轮次放大：Agent 通常多轮读文件，输入按 2.5× 偏高估。
const ROUND_INPUT_MULTIPLIER: f64 = 2.5;
/// 每 Agent 粗估输出 token。
const OUTPUT_PER_AGENT: u64 = 900;
/// Judge 每条候选粗估输入/输出。
const JUDGE_INPUT_PER_CANDIDATE: u64 = 1200;
const JUDGE_OUTPUT_PER_CANDIDATE: u64 = 200;
/// 无真实候选数时，按 fanout 的一个比例预估 judge 候选。
const JUDGE_CANDIDATE_FRACTION: f64 = 0.35;

/// 估算审查成本。
///
/// `plan_budget` 与编排一致（已扣 system/focus 开销）。
/// `judge` 为是否跑证伪。
pub fn estimate_review_cost(
    diff: &Diff,
    plan_budget: usize,
    dimensions: &[Dimension],
    samples: usize,
    judge: bool,
    prices: TokenPrices,
) -> CostEstimate {
    let units = plan_units(diff, plan_budget);
    estimate_from_units(diff, &units, dimensions, samples, judge, prices)
}

/// 在已有 unit 规划上估算（避免重复 plan）。
pub fn estimate_from_units(
    diff: &Diff,
    units: &[ReviewUnit],
    dimensions: &[Dimension],
    samples: usize,
    judge: bool,
    prices: TokenPrices,
) -> CostEstimate {
    let dims = dimensions.len().max(1);
    // 多单元强制 samples=1（与 run_review 一致）。
    let samples = if units.len() > 1 { 1 } else { samples.max(1) };
    let reviewable: Vec<&ReviewUnit> = units.iter().filter(|u| !u.oversized).collect();
    let reviewable_n = reviewable.len();
    let fanout = reviewable_n.saturating_mul(dims).saturating_mul(samples);

    let mut unit_input = 0u64;
    for u in &reviewable {
        let tok = u.est_tokens as u64;
        // 每维每样本各读一遍 unit 上下文，再乘多轮。
        unit_input = unit_input.saturating_add(
            ((tok as f64) * dims as f64 * samples as f64 * ROUND_INPUT_MULTIPLIER) as u64,
        );
    }

    let mut est_output = (fanout as u64).saturating_mul(OUTPUT_PER_AGENT);

    let mut est_input = unit_input;
    if judge {
        let candidates = ((fanout as f64) * JUDGE_CANDIDATE_FRACTION).ceil() as u64;
        let candidates = candidates.max(1).min(50);
        est_input = est_input.saturating_add(candidates.saturating_mul(JUDGE_INPUT_PER_CANDIDATE));
        est_output =
            est_output.saturating_add(candidates.saturating_mul(JUDGE_OUTPUT_PER_CANDIDATE));
    }

    // 文件数为 0 时全 0。
    if diff.files.is_empty() {
        return CostEstimate {
            units: 0,
            reviewable_units: 0,
            dimensions: dims,
            samples,
            fanout_agents: 0,
            est_input_tokens: 0,
            est_output_tokens: 0,
            est_cost_usd: None,
            summary: "empty diff · $0".into(),
        };
    }

    let est_cost_usd = match (prices.per_mtok_input, prices.per_mtok_output) {
        (None, None) => None,
        (pi, po) => {
            let mut c = 0.0;
            if let Some(p) = pi {
                c += (est_input as f64 / 1_000_000.0) * p;
            }
            if let Some(p) = po {
                c += (est_output as f64 / 1_000_000.0) * p;
            }
            Some(c)
        }
    };

    let summary = match est_cost_usd {
        Some(usd) => format!(
            "~{est_input} in / ~{est_output} out tok · {fanout} agents · ${usd:.4} upper bound"
        ),
        None => format!("~{est_input} in / ~{est_output} out tok · {fanout} agents (set price_per_mtok_* for $)"),
    };

    CostEstimate {
        units: units.len(),
        reviewable_units: reviewable_n,
        dimensions: dims,
        samples,
        fanout_agents: fanout,
        est_input_tokens: est_input,
        est_output_tokens: est_output,
        est_cost_usd,
        summary,
    }
}

/// 是否超过预算：`max_cost_usd` 或 `max_input_tokens`（任一超限即 true）。
pub fn exceeds_budget(
    est: &CostEstimate,
    max_cost_usd: Option<f64>,
    max_input_tokens: Option<u64>,
) -> Option<&'static str> {
    if let Some(max) = max_cost_usd {
        if let Some(usd) = est.est_cost_usd {
            if usd > max {
                return Some("estimated USD cost exceeds --max-cost");
            }
        }
    }
    if let Some(max) = max_input_tokens {
        if est.est_input_tokens > max {
            return Some("estimated input tokens exceed --max-input-tokens budget");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line, LineKind};

    fn mk_diff(n_files: usize, line_chars: usize) -> Diff {
        let mut files = Vec::new();
        for i in 0..n_files {
            let content = "x".repeat(line_chars);
            let hunk = Hunk {
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count: 1,
                section: String::new(),
                lines: vec![Line {
                    kind: LineKind::Added,
                    content: content.clone(),
                    old_lineno: None,
                    new_lineno: Some(1),
                }],
            };
            files.push(FileDiff {
                old_path: None,
                new_path: Some(format!("src/f{i}.rs")),
                status: FileStatus::Added,
                binary: false,
                hunks: vec![hunk],
            });
        }
        Diff { files }
    }

    #[test]
    fn empty_diff_zero_cost() {
        let d = Diff { files: vec![] };
        let e = estimate_review_cost(
            &d,
            100_000,
            &[Dimension::Security],
            1,
            true,
            TokenPrices::default(),
        );
        assert_eq!(e.fanout_agents, 0);
        assert_eq!(e.est_input_tokens, 0);
    }

    #[test]
    fn more_dimensions_increases_estimate() {
        let d = mk_diff(1, 300);
        let one = estimate_review_cost(
            &d,
            200_000,
            &[Dimension::Security],
            1,
            false,
            TokenPrices::default(),
        );
        let four = estimate_review_cost(
            &d,
            200_000,
            &Dimension::ALL,
            1,
            false,
            TokenPrices::default(),
        );
        assert!(four.est_input_tokens > one.est_input_tokens);
        assert_eq!(four.fanout_agents, one.fanout_agents * 4);
    }

    #[test]
    fn usd_when_prices_set() {
        let d = mk_diff(1, 300);
        let e = estimate_review_cost(
            &d,
            200_000,
            &[Dimension::Logic],
            1,
            false,
            TokenPrices {
                per_mtok_input: Some(1.0),
                per_mtok_output: Some(2.0),
            },
        );
        assert!(e.est_cost_usd.is_some());
        assert!(e.est_cost_usd.unwrap() > 0.0);
    }

    #[test]
    fn exceeds_budget_detects_token_and_usd() {
        let est = CostEstimate {
            units: 1,
            reviewable_units: 1,
            dimensions: 1,
            samples: 1,
            fanout_agents: 1,
            est_input_tokens: 50_000,
            est_output_tokens: 1000,
            est_cost_usd: Some(1.5),
            summary: String::new(),
        };
        assert!(exceeds_budget(&est, Some(1.0), None).is_some());
        assert!(exceeds_budget(&est, None, Some(10_000)).is_some());
        assert!(exceeds_budget(&est, Some(2.0), Some(100_000)).is_none());
    }
}
