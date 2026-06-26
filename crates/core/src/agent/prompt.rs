//! Agent 提示词装配。

#[cfg(test)]
use crate::language::detect_output_language_from;
use crate::language::output_language;
use crate::model::Dimension;

/// Dimension-specific focus text and checklist.
fn dimension_focus(d: Dimension) -> &'static str {
    match d {
        Dimension::Security => include_str!("../../prompts/dimensions/security.md").trim_end(),
        Dimension::Perf => include_str!("../../prompts/dimensions/perf.md").trim_end(),
        Dimension::Logic => include_str!("../../prompts/dimensions/logic.md").trim_end(),
        Dimension::Style => include_str!("../../prompts/dimensions/style.md").trim_end(),
        Dimension::Business => include_str!("../../prompts/dimensions/business.md").trim_end(),
        Dimension::AiSmell => include_str!("../../prompts/dimensions/ai_smell.md").trim_end(),
    }
}

/// Dimension-independent shared system prompt.
///
/// Keep this byte-identical across dimensions so prompt caching can reuse it. The
/// dimension-specific block is appended to the user message after the cacheable block.
pub fn shared_system_prompt() -> String {
    include_str!("../../prompts/shared_system.md")
        .trim_end()
        .to_string()
}

/// Dimension-specific focus block. It is placed after the shared user block.
pub fn dimension_focus_block(d: Dimension) -> String {
    format!(
        "## Review dimension\n\nYou are responsible for **only the `{dim}` dimension** in this run.\n\nFocus:\n{focus}\n\n\
Review only this dimension. Report confirmed issues with report_finding. Call task_done when finished.\n\n\
Output language: {lang}",
        dim = d.as_str(),
        focus = dimension_focus(d),
        lang = output_language(),
    )
}

/// Build the shared user prompt body: the review diff is dimension-independent and cacheable.
pub fn build_user_prompt(diff_text: &str) -> String {
    format!(
        "Please review the following code changes. Line numbers refer to the new file. `+` means added code; `-` means deleted code.\n\n\
Output language: {lang}\n\n{diff_text}",
        lang = output_language()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_templates_are_kept_as_markdown_files() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts");

        assert!(root.join("shared_system.md").is_file());
        for d in Dimension::ALL {
            assert!(
                root.join("dimensions")
                    .join(format!("{}.md", d.as_str()))
                    .is_file(),
                "missing prompt template for {}",
                d.as_str()
            );
        }
    }

    #[test]
    fn prompts_are_english_and_do_not_force_chinese() {
        assert!(shared_system_prompt().contains("You are ReviewGate"));
        assert!(!shared_system_prompt().contains("始终用中文"));
        assert!(dimension_focus_block(Dimension::Logic).contains("Review dimension"));
        assert!(build_user_prompt("diff").contains("Please review"));
    }

    #[test]
    fn output_language_is_detected_from_override_or_locale() {
        assert_eq!(
            detect_output_language_from([("REVIEWGATE_OUTPUT_LANGUAGE", "German")]),
            "German"
        );
        assert_eq!(
            detect_output_language_from([("LC_ALL", "zh_TW.UTF-8")]),
            "Chinese (Traditional)"
        );
        assert_eq!(
            detect_output_language_from([("LANG", "ja_JP.UTF-8")]),
            "Japanese"
        );
        assert_eq!(detect_output_language_from([("LANG", "C")]), "English");
    }
}
