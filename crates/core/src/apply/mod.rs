//! 应用修复：把文件里 `[start_line, end_line]` 区间替换为建议代码。
//!
//! 纯函数、可测；不碰文件系统（IO 与人工确认在 CLI 层）。
//!
//! **安全要点**：行号可能漂移（模型外推 / relocate 兜底 / 文件已被改）。直接按行号替换会改错代码，
//! 故对外入口是 [`apply_fix`]——替换前先用 `existing_code` 锚点核对目标区间，不匹配就拒绝（fail-safe）。

/// 应用修复失败的原因。
#[derive(Debug, PartialEq, Eq)]
pub enum ApplyError {
    /// 行号非法 / 越界 / 倒置。
    OutOfRange,
    /// 目标区间已不包含锚点 `existing_code`（行号漂移 / 代码已变）——拒绝以免改错。
    AnchorMismatch,
}

/// 把若干行规范化为单串（折叠空白、丢空行），用于锚点比对——对缩进/换行/空白不敏感。
fn normalize_region(s: &str) -> String {
    s.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// **安全入口**：先用 `anchor`(existing_code) 核对 `[start_line,end_line]` 区间仍是要改的代码，
/// 通过才替换为 `replacement`。anchor 为空、行号越界、或区间已不含 anchor → 返回 `Err`，调用方据此跳过。
pub fn apply_fix(
    content: &str,
    start_line: u32,
    end_line: u32,
    anchor: &str,
    replacement: &str,
) -> Result<String, ApplyError> {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len() as u32;
    if start_line == 0 || end_line < start_line || start_line > n || end_line > n {
        return Err(ApplyError::OutOfRange);
    }
    let region = lines[(start_line - 1) as usize..end_line as usize].join("\n");
    let anchor_n = normalize_region(anchor);
    // anchor 必须非空、且其规范化内容确实出现在待替换区间内——否则行号已漂移，拒绝。
    if anchor_n.is_empty() || !normalize_region(&region).contains(&anchor_n) {
        return Err(ApplyError::AnchorMismatch);
    }
    // 区间已校验合法，splice 必成功。
    Ok(splice_lines(content, start_line, end_line, replacement).expect("range validated above"))
}

/// 把 `content` 里 1-based 闭区间 `[start_line, end_line]` 的行替换为 `replacement`。
/// 行号非法（0 / 越界 / 倒置）返回 `None`，调用方据此跳过。
///
/// 注意：按 `\n` 切分重组——会把 CRLF 归一为 LF，并保证结尾有换行。对代码修复够用。
pub fn splice_lines(
    content: &str,
    start_line: u32,
    end_line: u32,
    replacement: &str,
) -> Option<String> {
    if start_line == 0 || end_line < start_line {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len() as u32;
    if start_line > n || end_line > n {
        return None;
    }
    let s = (start_line - 1) as usize;
    let e = end_line as usize; // 切片用的 exclusive 上界

    let mut out = String::new();
    for l in &lines[..s] {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(replacement.trim_end_matches('\n'));
    out.push('\n');
    for l in &lines[e..] {
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "a();\nbad();\nc();\n";

    #[test]
    fn replaces_single_line() {
        let out = splice_lines(FILE, 2, 2, "good();").unwrap();
        assert_eq!(out, "a();\ngood();\nc();\n");
    }

    #[test]
    fn replaces_range_with_multiline() {
        let f = "x();\nl1();\nl2();\ny();\n";
        let out = splice_lines(f, 2, 3, "new1();\nnew2();\nnew3();").unwrap();
        assert_eq!(out, "x();\nnew1();\nnew2();\nnew3();\ny();\n");
    }

    #[test]
    fn replacement_trailing_newline_normalized() {
        let out = splice_lines(FILE, 2, 2, "good();\n").unwrap();
        assert_eq!(out, "a();\ngood();\nc();\n");
    }

    #[test]
    fn rejects_invalid_ranges() {
        assert!(splice_lines(FILE, 0, 0, "x").is_none());
        assert!(splice_lines(FILE, 2, 1, "x").is_none()); // 倒置
        assert!(splice_lines(FILE, 5, 5, "x").is_none()); // 越界
    }

    // ---- apply_fix：锚点校验（P0 安全） ----

    #[test]
    fn apply_fix_ok_when_anchor_matches() {
        // 锚点 = 第 2 行内容（仅缩进差异，规范化后相等），应通过并替换。
        let out = apply_fix(FILE, 2, 2, "    bad();", "good();").unwrap();
        assert_eq!(out, "a();\ngood();\nc();\n");
    }

    #[test]
    fn apply_fix_rejects_anchor_mismatch() {
        // 行号指向第 1 行(a();)，但锚点是 bad()——行号漂移，必须拒绝以免改错。
        assert_eq!(
            apply_fix(FILE, 1, 1, "bad();", "good();"),
            Err(ApplyError::AnchorMismatch)
        );
    }

    #[test]
    fn apply_fix_rejects_empty_anchor() {
        // 空锚点无法校验 → 拒绝（不能盲改）。
        assert_eq!(
            apply_fix(FILE, 2, 2, "   ", "good();"),
            Err(ApplyError::AnchorMismatch)
        );
    }

    #[test]
    fn apply_fix_rejects_out_of_range() {
        assert_eq!(
            apply_fix(FILE, 9, 9, "bad();", "x"),
            Err(ApplyError::OutOfRange)
        );
    }

    #[test]
    fn apply_fix_anchor_subset_of_region() {
        // 区间是 2 行，锚点只命中其中关键一行 → 仍算匹配（子串）。
        let f = "x();\nbad_a();\nbad_b();\ny();\n";
        let out = apply_fix(f, 2, 3, "bad_b();", "fixed();").unwrap();
        assert_eq!(out, "x();\nfixed();\ny();\n");
    }

    #[test]
    fn apply_fix_multiline_anchor_matches() {
        let f = "x();\nfoo();\nbar();\ny();\n";
        let out = apply_fix(f, 2, 3, "foo();\nbar();", "baz();").unwrap();
        assert_eq!(out, "x();\nbaz();\ny();\n");
    }

    #[test]
    fn apply_fix_inverted_range_rejected() {
        assert_eq!(
            apply_fix(FILE, 3, 2, "bad();", "good();"),
            Err(ApplyError::OutOfRange)
        );
    }

    #[test]
    fn splice_lines_preserves_lines_after_range() {
        let f = "a();\nb();\nc();\nd();\n";
        let out = splice_lines(f, 2, 3, "X();").unwrap();
        assert_eq!(out, "a();\nX();\nd();\n");
    }
}
