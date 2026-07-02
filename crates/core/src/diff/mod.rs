//! diff 获取与解析。
//!
//! 支持三种范围模式（Workspace/Commit/Range），统一解析成 [`Diff`]。
//! Workspace 模式额外把未跟踪文件合成为「全新增」diff。

pub mod git;
mod model;
mod parse;

pub use model::{Diff, DiffMode, FileDiff, FileStatus, Hunk, Line, LineKind};

use anyhow::Result;

/// 统一 diff 上下文行数。
const CONTEXT: &str = "-U3";

/// 按模式收集改动。
pub async fn collect(mode: &DiffMode) -> Result<Diff> {
    let text = match mode {
        DiffMode::Workspace => git::git(&["diff", CONTEXT, "-M", "HEAD"]).await?,
        DiffMode::Commit(c) => git::git(&["show", CONTEXT, "-M", "--format=", c.as_str()]).await?,
        DiffMode::Range { from, to } => {
            let spec = format!("{from}...{to}");
            git::git(&["diff", CONTEXT, "-M", &spec]).await?
        }
    };

    let mut diff = Diff {
        files: parse::parse_unified(&text),
    };

    // Workspace 模式：补上未跟踪文件（合成全新增）。
    if matches!(mode, DiffMode::Workspace) {
        for path in git::untracked_files().await? {
            if let Some(fd) = synthesize_added(&path).await {
                diff.files.push(fd);
            }
        }
    }

    Ok(diff)
}

/// 把一个未跟踪文件合成为「全新增」FileDiff。
async fn synthesize_added(path: &str) -> Option<FileDiff> {
    let content = tokio::fs::read_to_string(path).await.ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len() as u32;
    let hunk_lines: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(i, c)| Line {
            kind: LineKind::Added,
            content: (*c).to_string(),
            old_lineno: None,
            new_lineno: Some(i as u32 + 1),
        })
        .collect();
    let hunks = if n == 0 {
        Vec::new()
    } else {
        vec![Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: n,
            section: String::new(),
            lines: hunk_lines,
        }]
    };
    Some(FileDiff {
        old_path: None,
        new_path: Some(path.to_string()),
        status: FileStatus::Added,
        binary: false,
        hunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn synthesize_added_maps_lines_and_status() {
        let tmp = std::env::temp_dir().join(format!("rg_synth_{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(&tmp, "line1\nline2\n").unwrap();

        let fd = synthesize_added(tmp.to_str().unwrap()).await.unwrap();
        assert_eq!(fd.status, FileStatus::Added);
        assert_eq!(fd.new_path.as_deref(), tmp.to_str());
        assert!(fd.old_path.is_none());
        assert!(!fd.binary);
        assert_eq!(fd.hunks.len(), 1);
        let h = &fd.hunks[0];
        assert_eq!(h.old_start, 0);
        assert_eq!(h.old_count, 0);
        assert_eq!(h.new_start, 1);
        assert_eq!(h.new_count, 2);
        assert_eq!(h.lines.len(), 2);
        assert_eq!(h.lines[0].content, "line1");
        assert_eq!(h.lines[0].kind, LineKind::Added);
        assert_eq!(h.lines[0].new_lineno, Some(1));
        assert_eq!(h.lines[1].new_lineno, Some(2));

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn synthesize_empty_file_has_no_hunks() {
        let tmp = std::env::temp_dir().join(format!("rg_synth_empty_{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(&tmp, "").unwrap();

        let fd = synthesize_added(tmp.to_str().unwrap()).await.unwrap();
        assert!(fd.hunks.is_empty());
        assert_eq!(fd.added_lines(), 0);

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn synthesize_missing_file_returns_none() {
        let path = "/tmp/reviewgate_definitely_missing_file_for_test.rs";
        assert!(synthesize_added(path).await.is_none());
    }
}
