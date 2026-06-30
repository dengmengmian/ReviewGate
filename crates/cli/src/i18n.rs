//! 报告渲染的本地化文案。跟随 `output_language()`，未覆盖语言回退英文。
//!
//! 仅覆盖「报告正文/标题/状态词」这类散文型 chrome；命令名（`reviewgate review …`）、
//! 维度/严重度标识、token 计量行属技术标识，保持英文。

/// 渲染语言。目前只区分中文与英文（其余语言回退英文）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// 按 `REVIEWGATE_OUTPUT_LANGUAGE` / locale 探测；中文（简/繁）→ Zh，其余 → En。
    pub fn detect() -> Self {
        Self::from_language(&reviewgate_core::language::output_language())
    }

    pub fn from_language(lang: &str) -> Self {
        if lang.starts_with("Chinese") {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    fn pick(self, en: &'static str, zh: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::Zh => zh,
        }
    }

    // ── 顶部计数行 ──
    pub fn no_changes(self) -> &'static str {
        self.pick(
            "No changes detected.\n\
             ReviewGate reviews the working tree by default (git diff HEAD), and there is no uncommitted diff right now.\n\
             To review committed work, point it at a commit or a range, e.g.:\n  \
               reviewgate review --commit HEAD           review the latest commit\n  \
               reviewgate review --from main --to HEAD   review this branch against main",
            "未检测到变更。\n\
             ReviewGate 默认审查工作区改动（git diff HEAD），当前没有未提交的 diff。\n\
             如需审查已提交内容，可指定提交或范围，例如：\n  \
               reviewgate review --commit HEAD           审查最近一次提交\n  \
               reviewgate review --from main --to HEAD   审查当前分支相对 main 的改动",
        )
    }
    pub fn files(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} files"),
            Lang::Zh => format!("{n} 个文件"),
        }
    }
    pub fn must_fix(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} must-fix"),
            Lang::Zh => format!("{n} 必须修复"),
        }
    }
    pub fn warn(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} warn"),
            Lang::Zh => format!("{n} 警告"),
        }
    }
    pub fn hidden(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} hidden"),
            Lang::Zh => format!("{n} 隐藏"),
        }
    }

    // ── 状态词 ──
    pub fn gate_label(self, pass: GateLabel) -> &'static str {
        match pass {
            GateLabel::Pass => self.pick("PASS", "通过"),
            GateLabel::Warn => self.pick("WARN", "警告"),
            GateLabel::Block => self.pick("BLOCK", "拦截"),
        }
    }

    // ── 未完成 / 不完整 ──
    pub fn incomplete_note(self) -> &'static str {
        self.pick(
            "Incomplete: some dimensions/units did not finish (timeout, request failure, context overflow, or oversized file skipped) — this result does not mean \"no issues\".",
            "不完整：部分维度/单元未跑完（超时、请求失败、上下文溢出，或跳过了过大的文件）——此结果不代表「没有问题」。",
        )
    }
    pub fn sec_incomplete_review(self) -> &'static str {
        self.pick("INCOMPLETE REVIEW", "评审未完成")
    }
    pub fn dims_not_finished(self) -> &'static str {
        self.pick(
            "The following dimensions did not finish:",
            "以下维度未跑完：",
        )
    }
    pub fn result_may_incomplete(self) -> &'static str {
        self.pick(
            "Result may be incomplete. Re-run with:",
            "结果可能不完整，可重新运行：",
        )
    }

    // ── 区块标题 ──
    pub fn sec_must_fix(self) -> &'static str {
        self.pick("MUST FIX", "必须修复")
    }
    pub fn sec_warnings(self) -> &'static str {
        self.pick("WARNINGS", "警告")
    }
    pub fn sec_not_shown(self) -> &'static str {
        self.pick("NOT SHOWN", "未显示")
    }
    pub fn sec_next_steps(self) -> &'static str {
        self.pick("NEXT STEPS", "后续步骤")
    }
    pub fn sec_intent_checklist(self) -> &'static str {
        self.pick("INTENT / ACCEPTANCE CHECKLIST", "意图 / 验收清单")
    }

    // ── 正文 ──
    pub fn no_actionable(self) -> &'static str {
        self.pick("No actionable issues found.", "未发现需处理的问题。")
    }
    pub fn low_conf_listed(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} low-confidence findings:"),
            Lang::Zh => format!("{n} 项低置信发现："),
        }
    }
    pub fn low_conf_hidden(self, n: usize) -> String {
        match self {
            Lang::En => format!(
                "{n} low-confidence findings hidden. Run with --show-filtered to inspect them."
            ),
            Lang::Zh => format!("{n} 项低置信发现已隐藏，加 --show-filtered 查看。"),
        }
    }
    pub fn next_patches(self) -> &'static str {
        self.pick(
            "Some findings include suggested patches. Apply manually, or run:",
            "部分发现附带补丁建议。可手动应用，或运行：",
        )
    }
    pub fn next_no_action(self) -> &'static str {
        self.pick("No action required.", "无需处理。")
    }
    pub fn next_fix_rerun(self) -> &'static str {
        self.pick(
            "Fix the findings above, then re-run:",
            "修复以上问题后，重新运行：",
        )
    }
    pub fn debug_slow_prefix(self) -> &'static str {
        self.pick("Debug slow reviews:  ", "调试慢评审：  ")
    }

    // ── 验收清单 ──
    pub fn unspecified(self) -> &'static str {
        self.pick("(unspecified)", "（未指定）")
    }
    pub fn intent_met(self) -> &'static str {
        self.pick("✓ met", "✓ 满足")
    }
    pub fn intent_missing(self) -> &'static str {
        self.pick("✗ missing", "✗ 缺失")
    }
    pub fn intent_breaking(self) -> &'static str {
        self.pick("✗ breaking", "✗ 破坏")
    }
    pub fn intent_deviation(self) -> &'static str {
        self.pick("⚠ deviation", "⚠ 偏差")
    }
    pub fn intent_suggestion(self) -> &'static str {
        self.pick("• suggestion", "• 建议")
    }
    pub fn intent_not_assessed(self) -> &'static str {
        self.pick("? not assessed", "? 未评估")
    }

    // ── 单条发现 ──
    pub fn confirmed_by(self, n: u8) -> String {
        match self {
            Lang::En => format!("confirmed by {n} dimensions"),
            Lang::Zh => format!("{n} 个维度共同确认"),
        }
    }
    // ── 实时进度 ──
    pub fn reviewing(self) -> &'static str {
        self.pick("Reviewing", "评审中")
    }
    pub fn calls(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} calls"),
            Lang::Zh => format!("{n} 次调用"),
        }
    }
    pub fn review_complete(self) -> &'static str {
        self.pick("Review complete", "评审完成")
    }
    pub fn tool_calls(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} tool calls"),
            Lang::Zh => format!("{n} 次工具调用"),
        }
    }

    pub fn patch(self) -> &'static str {
        self.pick("Patch", "补丁")
    }
    pub fn current(self) -> &'static str {
        self.pick("Current", "当前")
    }
    pub fn fix(self) -> &'static str {
        self.pick("Fix", "修复")
    }
}

/// 闸口判定的本地化键（避免在 i18n 模块直接依赖 core 的枚举形状）。
#[derive(Clone, Copy)]
pub enum GateLabel {
    Pass,
    Warn,
    Block,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chinese_variants_else_english() {
        assert_eq!(Lang::from_language("Chinese (Simplified)"), Lang::Zh);
        assert_eq!(Lang::from_language("Chinese (Traditional)"), Lang::Zh);
        assert_eq!(Lang::from_language("English"), Lang::En);
        assert_eq!(Lang::from_language("German"), Lang::En);
    }

    #[test]
    fn formats_track_language() {
        assert_eq!(Lang::En.must_fix(2), "2 must-fix");
        assert_eq!(Lang::Zh.must_fix(2), "2 必须修复");
        assert_eq!(Lang::En.sec_next_steps(), "NEXT STEPS");
        assert_eq!(Lang::Zh.sec_next_steps(), "后续步骤");
    }
}
