//! 审查上下文装配：渲染改动文件的 hunk 窗口、构造每个审查单元的 user prompt。

use crate::agent::build_user_prompt;
use crate::diff::{self, Diff, DiffMode};
use std::path::Path;

const MAX_FILE_LINES: usize = 500;
const MAX_TOTAL_LINES: usize = 2500;
const HUNK_CONTEXT_LINES: usize = 80;

/// 该模式下「新版本」内容的来源 ref。
pub(super) fn new_ref_for(mode: &DiffMode) -> Option<String> {
    match mode {
        DiffMode::Workspace => None,
        DiffMode::Commit(c) => Some(c.clone()),
        DiffMode::Range { to, .. } => Some(to.clone()),
    }
}

/// 渲染指定文件子集的完整新版本（带行号，按上限截断）。`file_indices` 为 `diff.files` 下标。
async fn render_changed_files(
    diff: &Diff,
    file_indices: &[usize],
    repo_root: &Path,
    new_ref: &Option<String>,
) -> String {
    let mut out = String::new();
    let mut budget = MAX_TOTAL_LINES;
    for &fi in file_indices {
        let f = &diff.files[fi];
        let Some(path) = f.new_path.as_deref() else {
            continue; // 已删除文件跳过
        };
        if f.binary || budget == 0 {
            continue;
        }
        let content = match new_ref {
            Some(r) => diff::git::git(&["show", &format!("{r}:{path}")]).await.ok(),
            None => tokio::fs::read_to_string(repo_root.join(path)).await.ok(),
        };
        let Some(content) = content else { continue };
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();
        let selected = hunk_context_line_numbers(f, total);
        let selected = selected
            .into_iter()
            .take(MAX_FILE_LINES.min(budget))
            .collect::<Vec<_>>();
        budget -= selected.len();
        out.push_str(&format!("### {path}\n```\n"));
        let mut prev = None;
        for line_no in &selected {
            if prev.is_some_and(|p| *line_no > p + 1) {
                out.push_str("  ...\n");
            }
            let idx = line_no - 1;
            out.push_str(&format!("{:>5} {}\n", line_no, all_lines[idx]));
            prev = Some(*line_no);
        }
        if selected.len() < total {
            out.push_str(&format!(
                "…（共 {total} 行，已按 hunk 周边截取 {} 行，需要更多用 read_file）\n",
                selected.len()
            ));
        }
        out.push_str("```\n\n");
    }
    out
}

fn hunk_context_line_numbers(file: &crate::diff::FileDiff, total: usize) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    if file.hunks.is_empty() {
        return (1..=total).collect();
    }
    let mut ranges = Vec::new();
    for h in &file.hunks {
        let mut nums = h.lines.iter().filter_map(|l| l.new_lineno);
        let first = nums.next().unwrap_or(h.new_start).max(1) as usize;
        let last = h
            .lines
            .iter()
            .filter_map(|l| l.new_lineno)
            .max()
            .unwrap_or_else(|| h.new_start.saturating_add(h.new_count).saturating_sub(1))
            .max(1) as usize;
        let start = first.saturating_sub(HUNK_CONTEXT_LINES).max(1);
        let end = last.saturating_add(HUNK_CONTEXT_LINES).min(total);
        ranges.push((start, end));
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end + 1 {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
        .into_iter()
        .flat_map(|(start, end)| start..=end)
        .collect()
}

/// 构造一个审查单元的 user prompt：单元内文件的 diff（+ 可选完整新版本上下文 + 项目规则）。
pub(super) async fn build_unit_prompt(
    diff: &Diff,
    file_indices: &[usize],
    include_ctx: bool,
    root: &Path,
    new_ref: &Option<String>,
    rules_body: &str,
) -> String {
    let mut diff_text = String::new();
    for &fi in file_indices {
        diff_text.push_str(&diff.files[fi].render_for_prompt());
        diff_text.push('\n');
    }
    let mut prompt = build_user_prompt(&diff_text);
    if include_ctx {
        let files_ctx = render_changed_files(diff, file_indices, root, new_ref).await;
        if !files_ctx.is_empty() {
            prompt.push_str("\n\n## Full new contents of the changed files (provided below; no need to read them one by one)\n\n");
            prompt.push_str(&files_ctx);
        }
    }
    if !rules_body.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(rules_body);
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line, LineKind};

    #[tokio::test]
    async fn changed_file_context_is_hunk_window_not_file_prefix() {
        let root = std::env::temp_dir().join(format!("rg_hunk_window_{}", std::process::id()));
        let src = root.join("src");
        tokio::fs::create_dir_all(&src).await.unwrap();
        let content = (1..=220)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(src.join("large.rs"), content)
            .await
            .unwrap();

        let diff = Diff {
            files: vec![FileDiff {
                old_path: Some("src/large.rs".into()),
                new_path: Some("src/large.rs".into()),
                status: FileStatus::Modified,
                binary: false,
                hunks: vec![Hunk {
                    old_start: 149,
                    old_count: 3,
                    new_start: 149,
                    new_count: 3,
                    section: String::new(),
                    lines: vec![
                        Line {
                            kind: LineKind::Context,
                            content: "line 149".into(),
                            old_lineno: Some(149),
                            new_lineno: Some(149),
                        },
                        Line {
                            kind: LineKind::Added,
                            content: "line 150".into(),
                            old_lineno: None,
                            new_lineno: Some(150),
                        },
                    ],
                }],
            }],
        };

        let all: Vec<usize> = (0..diff.files.len()).collect();
        let rendered = render_changed_files(&diff, &all, &root, &None).await;

        assert!(rendered.contains("  150 line 150"));
        assert!(rendered.contains("   69 line 69"));
        assert!(!rendered.contains("    1 line 1"));
        tokio::fs::remove_dir_all(&root).await.ok();
    }
}
