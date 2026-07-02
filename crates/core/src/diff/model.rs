//! diff 数据结构。

use std::collections::HashMap;

/// 审查范围模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffMode {
    /// 工作区相对 HEAD 的改动（含暂存）+ 未跟踪文件。默认。
    Workspace,
    /// 单个 commit 引入的改动。
    Commit(String),
    /// `from...to` 自 merge-base 起 `to` 的改动。
    Range { from: String, to: String },
}

/// 文件改动状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// diff 中一行的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Deleted,
}

/// diff 中的一行。
#[derive(Debug, Clone)]
pub struct Line {
    pub kind: LineKind,
    /// 行内容（不含前导 +/-/空格 与换行）。
    pub content: String,
    /// 旧文件行号（1-based）；新增行为 None。
    pub old_lineno: Option<u32>,
    /// 新文件行号（1-based）；删除行为 None。
    pub new_lineno: Option<u32>,
}

/// 一个 hunk。
#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// `@@ ... @@` 之后的区段标题（函数上下文）。
    pub section: String,
    pub lines: Vec<Line>,
}

/// 单个文件的改动。
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// 展示用路径：优先新路径。
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("<unknown>")
    }

    /// 新增行数。
    pub fn added_lines(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Added)
            .count()
    }

    /// 删除行数。
    pub fn deleted_lines(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Deleted)
            .count()
    }
}

/// 一次审查范围内的全部改动。
#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

impl Diff {
    /// 以新路径为 key 建索引，供工具按文件查 diff。
    pub fn by_new_path(&self) -> HashMap<&str, &FileDiff> {
        self.files
            .iter()
            .filter_map(|f| f.new_path.as_deref().map(|p| (p, f)))
            .collect()
    }

    /// 渲染成给 LLM 看的文本（带新文件行号、+/- 标记）。
    pub fn render_for_prompt(&self) -> String {
        let mut out = String::new();
        for f in &self.files {
            out.push_str(&f.render_for_prompt());
            out.push('\n');
        }
        out
    }
}

impl FileDiff {
    /// 渲染单个文件改动。
    pub fn render_for_prompt(&self) -> String {
        let mut out = format!("### {}  [{:?}]\n", self.path(), self.status);
        if self.binary {
            out.push_str("(binary file, omitted)\n");
            return out;
        }
        for h in &self.hunks {
            if h.section.is_empty() {
                out.push_str(&format!(
                    "@@ -{},{} +{},{} @@\n",
                    h.old_start, h.old_count, h.new_start, h.new_count
                ));
            } else {
                out.push_str(&format!(
                    "@@ -{},{} +{},{} @@ {}\n",
                    h.old_start, h.old_count, h.new_start, h.new_count, h.section
                ));
            }
            for l in &h.lines {
                let (mark, no) = match l.kind {
                    LineKind::Added => ('+', l.new_lineno),
                    LineKind::Deleted => ('-', None),
                    LineKind::Context => (' ', l.new_lineno),
                };
                match no {
                    Some(n) => out.push_str(&format!("{:>6} {}{}\n", n, mark, l.content)),
                    None => out.push_str(&format!("       {}{}\n", mark, l.content)),
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, content: &str, old: Option<u32>, new: Option<u32>) -> Line {
        Line {
            kind,
            content: content.into(),
            old_lineno: old,
            new_lineno: new,
        }
    }

    fn file_diff(new_path: &str, hunks: Vec<Hunk>) -> FileDiff {
        FileDiff {
            old_path: None,
            new_path: Some(new_path.into()),
            status: FileStatus::Modified,
            binary: false,
            hunks,
        }
    }

    #[test]
    fn path_prefers_new_then_old_then_unknown() {
        let mut f = file_diff("a.rs", vec![]);
        assert_eq!(f.path(), "a.rs");
        f.new_path = None;
        f.old_path = Some("old.rs".into());
        assert_eq!(f.path(), "old.rs");
        f.old_path = None;
        assert_eq!(f.path(), "<unknown>");
    }

    #[test]
    fn added_and_deleted_line_counts() {
        let hunk = Hunk {
            old_start: 1,
            old_count: 2,
            new_start: 1,
            new_count: 3,
            section: String::new(),
            lines: vec![
                line(LineKind::Deleted, "a", Some(1), None),
                line(LineKind::Added, "b", None, Some(1)),
                line(LineKind::Added, "c", None, Some(2)),
                line(LineKind::Context, "d", Some(2), Some(3)),
            ],
        };
        let f = file_diff("x.rs", vec![hunk]);
        assert_eq!(f.added_lines(), 2);
        assert_eq!(f.deleted_lines(), 1);
    }

    #[test]
    fn by_new_path_indexes_only_new_paths() {
        let a = FileDiff {
            old_path: Some("old.rs".into()),
            new_path: Some("a.rs".into()),
            status: FileStatus::Renamed,
            binary: false,
            hunks: vec![],
        };
        let b = FileDiff {
            old_path: Some("b.rs".into()),
            new_path: None,
            status: FileStatus::Deleted,
            binary: false,
            hunks: vec![],
        };
        let diff = Diff { files: vec![a, b] };
        let idx = diff.by_new_path();
        assert_eq!(idx.len(), 1);
        assert!(idx.contains_key("a.rs"));
        assert!(!idx.contains_key("b.rs"));
    }

    #[test]
    fn render_for_prompt_includes_status_and_line_numbers() {
        let f = file_diff(
            "src/main.rs",
            vec![Hunk {
                old_start: 1,
                old_count: 1,
                new_start: 1,
                new_count: 2,
                section: "fn main".into(),
                lines: vec![
                    line(LineKind::Context, "fn main() {", Some(1), Some(1)),
                    line(LineKind::Added, "    println!();", None, Some(2)),
                ],
            }],
        );
        let out = f.render_for_prompt();
        assert!(out.contains("### src/main.rs  [Modified]"));
        assert!(out.contains("@@ -1,1 +1,2 @@ fn main"));
        assert!(
            out.contains("     1  fn main() {"),
            "actual context line: {out:?}"
        );
        assert!(
            out.contains("     2 +    println!();"),
            "actual added line: {out:?}"
        );
    }

    #[test]
    fn render_binary_file_omits_content() {
        let f = FileDiff {
            old_path: None,
            new_path: Some("img.png".into()),
            status: FileStatus::Added,
            binary: true,
            hunks: vec![],
        };
        let out = f.render_for_prompt();
        assert!(out.contains("img.png"));
        assert!(out.contains("(binary file, omitted)"));
    }

    #[test]
    fn render_deleted_lines_have_no_number() {
        let f = file_diff(
            "src/x.rs",
            vec![Hunk {
                old_start: 5,
                old_count: 1,
                new_start: 5,
                new_count: 0,
                section: String::new(),
                lines: vec![line(LineKind::Deleted, "removed", Some(5), None)],
            }],
        );
        let out = f.render_for_prompt();
        assert!(out.contains("       -removed"));
        assert!(!out.contains("     5 -removed"));
    }

    #[test]
    fn render_diff_joins_files_with_blank_line() {
        let diff = Diff {
            files: vec![file_diff("a.rs", vec![]), file_diff("b.rs", vec![])],
        };
        let out = diff.render_for_prompt();
        let parts: Vec<_> = out.split("\n").collect();
        // Two headers plus a trailing empty line.
        assert!(parts.iter().any(|s| s.contains("a.rs")));
        assert!(parts.iter().any(|s| s.contains("b.rs")));
    }
}
