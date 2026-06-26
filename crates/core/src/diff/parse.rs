//! unified diff 解析器。把 `git diff -U3` 文本解析成 `FileDiff`/`Hunk`/`Line`。

use super::model::{FileDiff, FileStatus, Hunk, Line, LineKind};

/// 解析 unified diff 文本。
pub fn parse_unified(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;

    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            // 收尾上一个文件。
            if let Some(f) = cur.take() {
                files.push(f);
            }
            let (a, b) = split_diff_git_paths(rest);
            cur = Some(FileDiff {
                old_path: a,
                new_path: b,
                status: FileStatus::Modified,
                binary: false,
                hunks: Vec::new(),
            });
            continue;
        }

        let Some(file) = cur.as_mut() else { continue };

        if raw.starts_with("new file mode") {
            file.status = FileStatus::Added;
        } else if raw.starts_with("deleted file mode") {
            file.status = FileStatus::Deleted;
        } else if let Some(p) = raw.strip_prefix("rename from ") {
            file.status = FileStatus::Renamed;
            file.old_path = Some(p.to_string());
        } else if let Some(p) = raw.strip_prefix("rename to ") {
            file.status = FileStatus::Renamed;
            file.new_path = Some(p.to_string());
        } else if raw.starts_with("Binary files ") {
            file.binary = true;
        } else if let Some(p) = raw.strip_prefix("--- ") {
            file.old_path = path_or_none(p);
            if file.old_path.is_none() {
                file.status = FileStatus::Added;
            }
        } else if let Some(p) = raw.strip_prefix("+++ ") {
            file.new_path = path_or_none(p);
            if file.new_path.is_none() {
                file.status = FileStatus::Deleted;
            }
        } else if raw.starts_with("@@") {
            if let Some((h, os, ns)) = parse_hunk_header(raw) {
                old_no = os;
                new_no = ns;
                file.hunks.push(h);
            }
        } else if !file.hunks.is_empty() {
            // hunk 体。
            let hunk = file.hunks.last_mut().unwrap();
            if let Some(c) = raw.strip_prefix('+') {
                hunk.lines.push(Line {
                    kind: LineKind::Added,
                    content: c.to_string(),
                    old_lineno: None,
                    new_lineno: Some(new_no),
                });
                new_no += 1;
            } else if let Some(c) = raw.strip_prefix('-') {
                hunk.lines.push(Line {
                    kind: LineKind::Deleted,
                    content: c.to_string(),
                    old_lineno: Some(old_no),
                    new_lineno: None,
                });
                old_no += 1;
            } else if let Some(c) = raw.strip_prefix(' ') {
                hunk.lines.push(Line {
                    kind: LineKind::Context,
                    content: c.to_string(),
                    old_lineno: Some(old_no),
                    new_lineno: Some(new_no),
                });
                old_no += 1;
                new_no += 1;
            } else if raw.starts_with('\\') {
                // "\ No newline at end of file" —— 忽略。
            }
        }
    }

    if let Some(f) = cur.take() {
        files.push(f);
    }
    files
}

/// 解析 `@@ -old_start,old_count +new_start,new_count @@ section`。
fn parse_hunk_header(line: &str) -> Option<(Hunk, u32, u32)> {
    // 形如 "@@ -a,b +c,d @@ rest"
    let rest = line.strip_prefix("@@ ")?;
    let close = rest.find(" @@")?;
    let ranges = &rest[..close];
    let section = rest[close + 3..].trim().to_string();

    let mut parts = ranges.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;

    Some((
        Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            section,
            lines: Vec::new(),
        },
        old_start,
        new_start,
    ))
}

/// 解析 `start,count` 或 `start`（count 默认 1）。
fn parse_range(s: &str) -> Option<(u32, u32)> {
    let mut it = s.split(',');
    let start: u32 = it.next()?.parse().ok()?;
    let count: u32 = match it.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

/// `--- a/foo` / `+++ b/foo` 的路径；`/dev/null` 返回 None。
fn path_or_none(s: &str) -> Option<String> {
    // 去掉可能的 tab 元信息后缀。
    let s = s.split('\t').next().unwrap_or(s).trim();
    if s == "/dev/null" {
        return None;
    }
    let stripped = s
        .strip_prefix("a/")
        .or_else(|| s.strip_prefix("b/"))
        .unwrap_or(s);
    Some(stripped.to_string())
}

/// 从 `a/foo b/foo` 粗解析双路径（仅作兜底，权威以 ---/+++ 为准）。
fn split_diff_git_paths(rest: &str) -> (Option<String>, Option<String>) {
    if let Some(idx) = rest.find(" b/") {
        let a = rest[..idx].trim();
        let b = &rest[idx + 1..];
        return (path_or_none(a), path_or_none(b));
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/auth.rs b/src/auth.rs\nindex 111..222 100644\n--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -10,3 +10,4 @@ fn login() {\n let user = get();\n-    let q = format!(\"select * where id={}\", id);\n+    let q = sqlx::query(\"select * where id=$1\");\n+    audit(user);\n ok()\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n";

    #[test]
    fn parses_two_files() {
        let files = parse_unified(SAMPLE);
        assert_eq!(files.len(), 2);

        let auth = &files[0];
        assert_eq!(auth.new_path.as_deref(), Some("src/auth.rs"));
        assert_eq!(auth.status, FileStatus::Modified);
        assert_eq!(auth.added_lines(), 2);
        assert_eq!(auth.deleted_lines(), 1);

        // 行号正确：新增的 audit(user) 应在新文件第 12 行附近。
        let added: Vec<_> = auth.hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .collect();
        assert_eq!(added[0].new_lineno, Some(11));
        assert_eq!(added[1].new_lineno, Some(12));

        let newf = &files[1];
        assert_eq!(newf.new_path.as_deref(), Some("new.txt"));
        assert_eq!(newf.status, FileStatus::Added);
        assert_eq!(newf.added_lines(), 2);
        assert_eq!(newf.hunks[0].lines[0].new_lineno, Some(1));
    }

    #[test]
    fn hunk_header_without_count() {
        let files =
            parse_unified("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -5 +5 @@\n-old\n+new\n");
        let h = &files[0].hunks[0];
        assert_eq!(h.old_start, 5);
        assert_eq!(h.old_count, 1);
        assert_eq!(h.new_start, 5);
    }

    #[test]
    fn parses_deleted_file() {
        let d = "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-line one\n-line two\n";
        let files = parse_unified(d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Deleted);
        assert_eq!(files[0].old_path.as_deref(), Some("gone.rs"));
        assert_eq!(files[0].new_path, None);
        assert_eq!(files[0].deleted_lines(), 2);
        assert_eq!(files[0].added_lines(), 0);
    }

    #[test]
    fn parses_renamed_file() {
        let d = "diff --git a/old/name.rs b/new/name.rs\nsimilarity index 90%\nrename from old/name.rs\nrename to new/name.rs\n--- a/old/name.rs\n+++ b/new/name.rs\n@@ -1 +1 @@\n-fn a() {}\n+fn b() {}\n";
        let files = parse_unified(d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].old_path.as_deref(), Some("old/name.rs"));
        assert_eq!(files[0].new_path.as_deref(), Some("new/name.rs"));
    }

    /// 健壮性：畸形/对抗性 diff 不得 panic（解析器是最暴露在不可信输入下的组件）。
    #[test]
    fn malformed_input_does_not_panic() {
        let cases = [
            "",                                 // 空
            "@@",                               // 截断的 hunk 头
            "@@ -1 +1",                         // 缺 " @@" 收尾
            "@@ garbage @@\n+x\n",              // 范围非法
            "+orphan line before any header\n", // 体行先于任何 @@ 头
            " ctx line with no file\n-del\n+add\n",
            "diff --git a/x b/x\n@@ -1,1 +1,1 @@\n中文行\n😀 emoji\n", // 多字节
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n",                         // 头后无体
            "@@ -1,2 +1,2 @@ section with spaces and @@ inside\n x\n", // 头里再现 @@
            "Binary files a/x and b/x differ\n",
        ];
        for c in cases {
            // 只要不 panic 即通过；顺便确认返回的是合法结构。
            let files = parse_unified(c);
            for f in &files {
                let _ = (f.added_lines(), f.deleted_lines(), f.hunks.len());
            }
        }
    }
}
