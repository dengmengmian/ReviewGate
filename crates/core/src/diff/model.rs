//! diff 数据结构。

use std::collections::HashMap;

/// 审查范围模式。
#[derive(Debug, Clone)]
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
            out.push_str("(二进制文件，略)\n");
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
