//! 饱和式采样控制：`reviewgate security` 用「连续若干轮无新发现才停」替代固定 samples。
//!
//! 固定 samples 的问题是两头不讨好：小 diff 白跑第二遍，大 diff 两遍远不够。
//! 饱和策略让轮数跟着 diff 的实际复杂度走，并用 `max_rounds` 兜住长尾成本。
//!
//! 「有没有新发现」由调用方判定——跑完一轮后把结果并入候选池再 `dedupe`，
//! 池子是否增长就是本轮的 `grew`。这样完全复用现有去重语义，不引入第二套指纹。

/// 一轮 discovery 之后的收敛状态机。
#[derive(Debug, Clone)]
pub struct Saturation {
    /// 连续多少轮无新发现就停。规范化后 ≥ 1。
    stop_after_no_new: usize,
    /// 轮数硬上限，兜住成本长尾。规范化后 ≥ 1。
    max_rounds: usize,
    rounds_done: usize,
    dry_streak: usize,
}

impl Saturation {
    /// 两个参数都规范化到至少 1：一轮都不跑的配置没有意义，
    /// 与其在调用点静默返回空结果，不如在这里兜成「至少跑一轮」。
    pub fn new(stop_after_no_new: usize, max_rounds: usize) -> Self {
        Self {
            stop_after_no_new: stop_after_no_new.max(1),
            max_rounds: max_rounds.max(1),
            rounds_done: 0,
            dry_streak: 0,
        }
    }

    /// 是否还应再跑一轮 discovery。
    pub fn should_continue(&self) -> bool {
        self.rounds_done < self.max_rounds && self.dry_streak < self.stop_after_no_new
    }

    /// 记录一轮结果。`grew` = 本轮并入并去重后候选池是否增长。
    pub fn record(&mut self, grew: bool) {
        self.rounds_done += 1;
        if grew {
            self.dry_streak = 0;
        } else {
            self.dry_streak += 1;
        }
    }

    /// 已完成轮数。
    pub fn rounds_done(&self) -> usize {
        self.rounds_done
    }

    /// 是否因撞上 `max_rounds` 而停（而非真正饱和）。
    ///
    /// 撞上限说明「可能还有没挖到的问题」，调用方据此标记 incomplete——
    /// 闸口的底线是不把「没审完」说成「通过」。
    pub fn stopped_by_cap(&self) -> bool {
        self.rounds_done >= self.max_rounds && self.dry_streak < self.stop_after_no_new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_at_least_one_round() {
        let s = Saturation::new(2, 10);
        assert!(s.should_continue(), "首轮之前必须允许跑");
        assert_eq!(s.rounds_done(), 0);
    }

    #[test]
    fn stops_after_consecutive_dry_rounds() {
        let mut s = Saturation::new(2, 10);
        s.record(true); // 第 1 轮有新发现
        assert!(s.should_continue());
        s.record(false); // 第 2 轮空
        assert!(s.should_continue(), "只空了 1 轮，未达 stop_after_no_new=2");
        s.record(false); // 第 3 轮空 → 连续 2 轮
        assert!(!s.should_continue(), "连续 2 轮无新发现应停");
        assert_eq!(s.rounds_done(), 3);
        assert!(!s.stopped_by_cap(), "这是真饱和，不是撞上限");
    }

    #[test]
    fn new_finding_resets_dry_streak() {
        let mut s = Saturation::new(2, 10);
        s.record(false);
        s.record(true); // 打断连续空轮
        assert!(s.should_continue(), "有新发现应重置空轮计数");
        s.record(false);
        assert!(s.should_continue());
        s.record(false);
        assert!(!s.should_continue());
    }

    #[test]
    fn stops_at_max_rounds_and_flags_cap() {
        let mut s = Saturation::new(3, 4);
        for _ in 0..4 {
            s.record(true); // 每轮都有新发现，永远不会饱和
        }
        assert!(!s.should_continue(), "达到 max_rounds 必须停");
        assert_eq!(s.rounds_done(), 4);
        assert!(s.stopped_by_cap(), "撞上限应可被识别，供 incomplete 标记");
    }

    #[test]
    fn degenerate_params_still_run_one_round() {
        let mut s = Saturation::new(0, 0);
        assert!(s.should_continue(), "参数为 0 也要至少跑一轮");
        s.record(true);
        assert!(!s.should_continue());
        assert_eq!(s.rounds_done(), 1);
    }
}
