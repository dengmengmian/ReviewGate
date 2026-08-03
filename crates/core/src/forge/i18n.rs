//! PR/MR 摘要评论的本地化文案。
//!
//! 评论是**报告**，和终端报告同一性质，因此跟随 `output_language()`（未覆盖语言回退英文）。
//! 技术标识（维度名、severity、kind、路径、表头）保持英文，跨语言团队才对得上。

use crate::language::output_language;

/// 评论渲染语言。目前只区分中文与英文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdLang {
    En,
    Zh,
}

impl MdLang {
    /// 按 `REVIEWGATE_OUTPUT_LANGUAGE` / locale 探测。
    pub fn detect() -> Self {
        Self::from_language(&output_language())
    }

    pub fn from_language(lang: &str) -> Self {
        if lang.starts_with("Chinese") {
            MdLang::Zh
        } else {
            MdLang::En
        }
    }

    fn pick(self, en: &'static str, zh: &'static str) -> &'static str {
        match self {
            MdLang::En => en,
            MdLang::Zh => zh,
        }
    }

    pub fn badge_pass(self) -> &'static str {
        self.pick("✅ **PASS** — merge allowed", "✅ **PASS** — 放行")
    }
    pub fn badge_warn(self) -> &'static str {
        self.pick(
            "⚠️ **WARN** — issues worth attention",
            "⚠️ **WARN** — 有需关注的问题",
        )
    }
    pub fn badge_block(self) -> &'static str {
        self.pick("🛑 **BLOCK** — merge blocked", "🛑 **BLOCK** — 阻断合并")
    }

    /// 计数行：`N files changed · N credible findings · N filtered`。
    pub fn counts(self, files: usize, kept: usize, filtered: usize) -> String {
        match self {
            MdLang::En => {
                format!("{files} files changed · {kept} credible findings · {filtered} filtered")
            }
            MdLang::Zh => format!("{files} 个文件改动 · {kept} 条可信发现 · {filtered} 条已过滤"),
        }
    }

    /// 审查范围行。
    pub fn scope(self, scope: &str) -> String {
        match self {
            MdLang::En => format!("**Scope:** {scope}"),
            MdLang::Zh => format!("**审查范围：**{scope}"),
        }
    }
    pub fn excluded_label(self) -> &'static str {
        self.pick(
            "ℹ️ **Not reviewed (exclude rules):**",
            "ℹ️ **未送审（排除规则）:**",
        )
    }
    pub fn excluded_more(self, total: usize) -> String {
        match self {
            MdLang::En => format!(" … ({total} total)"),
            MdLang::Zh => format!(" …（共 {total} 个）"),
        }
    }

    pub fn incomplete_note(self) -> &'static str {
        self.pick(
            "> 🟠 **Review incomplete**: some dimensions/units were skipped (timeout, request failure, \
             context overflow, or oversized files) — this verdict does not mean \"no problems\".\n\n",
            "> 🟠 **审查未完整**：部分维度/单元因超时、请求失败、上下文超限或超大文件被跳过而**未审完** —— \
             结论不代表“无问题”。\n\n",
        )
    }
    pub fn uncovered_paths(self) -> &'static str {
        self.pick("> **Uncovered paths:**", "> **未覆盖路径:**")
    }
    pub fn incomplete_list(self) -> &'static str {
        self.pick("> ⚠️ **Incomplete**:", "> ⚠️ **未审完**：")
    }
    pub fn critical_incomplete(self) -> &'static str {
        self.pick(
            "> 🛑 **Critical paths incomplete**: the review touched auth/payment/security paths \
             without finishing, so the verdict was forced to non-PASS.\n\n",
            "> 🛑 **关键路径未审完**：触及 auth/payment/security 等敏感路径的 incomplete 已强制非 PASS。\n\n",
        )
    }
    pub fn no_issues(self) -> &'static str {
        self.pick(
            "No issues reached the display threshold.\n",
            "没有达到展示阈值的问题。\n",
        )
    }
    /// 已在 PR 讨论里被提过、因此折叠的发现数。
    pub fn already_discussed(self, n: usize) -> String {
        match self {
            MdLang::En => format!(
                "> 💬 {n} finding(s) folded: already raised in this PR's existing review discussion."
            ),
            MdLang::Zh => format!("> 💬 已折叠 {n} 条：PR 现有评审讨论里已经提过。"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chinese_and_falls_back_to_english() {
        assert_eq!(MdLang::from_language("Chinese (Simplified)"), MdLang::Zh);
        assert_eq!(MdLang::from_language("English"), MdLang::En);
        assert_eq!(MdLang::from_language("Français"), MdLang::En);
    }

    #[test]
    fn both_languages_are_non_empty() {
        for l in [MdLang::En, MdLang::Zh] {
            assert!(!l.badge_pass().is_empty());
            assert!(!l.badge_block().is_empty());
            assert!(!l.counts(1, 2, 3).is_empty());
            assert!(!l.incomplete_note().is_empty());
            assert!(!l.no_issues().is_empty());
        }
    }
}
