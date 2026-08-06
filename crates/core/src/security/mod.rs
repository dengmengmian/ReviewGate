//! `reviewgate security` 的独立编排线。
//!
//! 与 [`crate::review`] 共用底座（diff 采集、工具层、Agent 循环、证伪 Judge、闸口），
//! 但**相位编排自己走**：饱和式 discovery 替代固定采样，不跑意图评审，不做跨维度加分
//! （单维时本就无效果）。两条线的差异体现在阶段序列本身，而不是同一个函数里的分支。

mod discovery;
pub mod saturation;

pub use saturation::Saturation;

use crate::config::Config;
use crate::llm::{build_client, LlmClient};
use crate::review::stages::{self, Prepared};
use crate::review::{ReviewOptions, ReviewOutcome};
use anyhow::Result;

/// 饱和式 discovery 的默认参数：连续 2 轮无新发现即停，最多 6 轮。
///
/// 2 轮空转就收手，是因为再多跑主要是重复付钱；6 轮上限对应「每轮都还在挖到东西」
/// 的坏情况，撞上限会标记 incomplete 而不是假装审完了。
pub const DEFAULT_STOP_AFTER_NO_NEW: usize = 2;
pub const DEFAULT_MAX_ROUNDS: usize = 6;

/// 安全深审入口：自建 LLM 客户端。
pub async fn run_security(cfg: &Config, opts: &SecurityOptions) -> Result<ReviewOutcome> {
    let client = build_client(&cfg.active_provider_resolved()?)?;
    run_security_with_client(cfg, opts, &*client).await
}

/// 安全深审的编排参数：复用审查选项，另加饱和式 discovery 的两个旋钮。
pub struct SecurityOptions {
    pub review: ReviewOptions,
    /// 连续多少轮无新发现即停。
    pub stop_after_no_new: usize,
    /// 轮数硬上限；撞上限标记 incomplete。
    pub max_rounds: usize,
}

impl SecurityOptions {
    pub fn new(review: ReviewOptions) -> Self {
        Self {
            review,
            stop_after_no_new: DEFAULT_STOP_AFTER_NO_NEW,
            max_rounds: DEFAULT_MAX_ROUNDS,
        }
    }
}

/// 同 [`run_security`]，但**注入** LLM 客户端——便于用 mock 做端到端编排测试（不联网）。
///
/// 阶段序列与 review 线的差异：
/// - discovery 用饱和循环而非固定 samples
/// - 不跑意图评审（与安全无关）
/// - 不做跨维度交叉印证加分（单维时天然无效果）
pub async fn run_security_with_client(
    cfg: &Config,
    opts: &SecurityOptions,
    client: &dyn LlmClient,
) -> Result<ReviewOutcome> {
    // 估算基数用轮数上界：饱和轮数跑前未知，低估会让预算守卫形同虚设。
    let mut c =
        match stages::prepare(cfg, &opts.review, client, Some(opts.max_rounds.max(1))).await? {
            Prepared::Early(outcome) => return Ok(*outcome),
            Prepared::Ready(c) => *c,
        };

    let sat = Saturation::new(opts.stop_after_no_new, opts.max_rounds);
    discovery::discover_saturating(&mut c, client, sat).await;
    stages::secrets(&mut c);
    stages::report_incomplete(&c);
    stages::relocate_dedupe(&mut c).await;
    stages::judge(&mut c, client).await;
    stages::store_incremental(&mut c);
    stages::suppress(&mut c);
    stages::gate(&mut c);
    Ok(stages::finalize(c))
}
