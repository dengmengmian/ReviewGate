//! 行号校验与兜底定位。
//!
//! 模型已能从代码左侧标注的行号直接报 `line_start`/`line_end`，本模块只做两件事：
//! 1. **校验**：模型给了行号时，用 `existing_code` 锚点在该行 ±窗口内廉价核对；
//!    符合即直接采信（不做全量扫描，省时）。明显不符才回退到全量定位纠正。
//! 2. **兜底**：模型没给行号（line_start=0）时，用 `existing_code` 把片段匹配回新文件。
//!
//! 全量定位（[`locate`]）对空白不敏感，并优先选与「新增行」重叠的匹配，
//! 避免片段在文件中多处出现时定位错误。

use crate::diff::Diff;
use crate::model::Finding;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// 规范化一行：折叠所有空白为单空格并去首尾。对缩进/空白差异不敏感。
fn normalize(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 在 `file_content` 中定位 `snippet`，返回 1-based `(start, end)`（含）。
/// `added_lines` 为该文件的新增行号集合，用于在多处匹配时择优。失败返回 None。
pub fn locate(snippet: &str, file_content: &str, added_lines: &HashSet<u32>) -> Option<(u32, u32)> {
    // 片段去掉首尾空行后逐行规范化。
    let snip: Vec<String> = {
        let mut v: Vec<String> = snippet.lines().map(normalize).collect();
        while v.first().map(|s| s.is_empty()).unwrap_or(false) {
            v.remove(0);
        }
        while v.last().map(|s| s.is_empty()).unwrap_or(false) {
            v.pop();
        }
        v
    };
    if snip.is_empty() {
        return None;
    }
    let file: Vec<String> = file_content.lines().map(normalize).collect();
    if file.len() < snip.len() {
        // 多行片段超过文件长度，仍可尝试单行兜底。
        return single_line_fallback(&snip, &file, added_lines);
    }

    // 收集所有精确（规范化）匹配，按与新增行的重叠度择优，平局取最靠前。
    let mut best: Option<((u32, u32), usize)> = None;
    for start in 0..=(file.len() - snip.len()) {
        if (0..snip.len()).all(|i| file[start + i] == snip[i]) {
            let s = start as u32 + 1;
            let e = (start + snip.len()) as u32;
            let overlap = (s..=e).filter(|n| added_lines.contains(n)).count();
            match &best {
                Some((_, bo)) if *bo >= overlap => {}
                _ => best = Some(((s, e), overlap)),
            }
        }
    }
    if let Some((range, _)) = best {
        return Some(range);
    }

    single_line_fallback(&snip, &file, added_lines)
}

/// 单行片段的子串兜底：片段（拼成一行）作为某文件行的子串。
fn single_line_fallback(
    snip: &[String],
    file: &[String],
    added_lines: &HashSet<u32>,
) -> Option<(u32, u32)> {
    if snip.len() != 1 || snip[0].is_empty() {
        return None;
    }
    let needle = &snip[0];
    let mut fallback: Option<u32> = None;
    for (i, line) in file.iter().enumerate() {
        if line.contains(needle) {
            let lineno = i as u32 + 1;
            if added_lines.contains(&lineno) {
                return Some((lineno, lineno)); // 命中新增行，最佳。
            }
            fallback.get_or_insert(lineno);
        }
    }
    fallback.map(|n| (n, n))
}

/// 校验窗口：模型给的行号允许与锚点实际位置相差的行数。
const ANCHOR_WINDOW: u32 = 3;

/// 锚点的首个有效行是否出现在 `claimed`(1-based) 行的 ±`window` 范围内。
/// 用于廉价校验模型给的行号；锚点为空时返回 true（不否定模型）。
fn anchor_matches_near(snippet: &str, file: &[String], claimed: u32, window: u32) -> bool {
    let Some(first) = snippet.lines().map(normalize).find(|l| !l.is_empty()) else {
        return true; // 无有效锚点行 → 无从否定，采信模型行号。
    };
    if claimed == 0 || file.is_empty() {
        return false;
    }
    let lo = (claimed.saturating_sub(window)).max(1) as usize;
    let hi = (claimed + window).min(file.len() as u32) as usize;
    (lo..=hi).any(|i| {
        file.get(i - 1)
            .map(|l| *l == first || l.contains(&first))
            .unwrap_or(false)
    })
}

/// 对一批 Finding 校验/补齐 `start_line`/`end_line`。
///
/// 模型给了行号：锚点核对通过即直接采信；明显不符才用 [`locate`] 纠正。
/// 模型没给行号：用 [`locate`] 兜底定位。
pub async fn relocate_all(
    findings: &mut [Finding],
    repo_root: &Path,
    new_ref: &Option<String>,
    diff: &Diff,
) {
    let by_path = diff.by_new_path();
    let mut cache: HashMap<String, Option<String>> = HashMap::new();

    for f in findings.iter_mut() {
        // 读新文件内容（带缓存）。
        if !cache.contains_key(&f.path) {
            let content = read_new_version(repo_root, new_ref, &f.path).await;
            cache.insert(f.path.clone(), content);
        }
        let Some(Some(content)) = cache.get(&f.path) else {
            continue; // 读不到文件：保留模型给的行号（或 0）。
        };

        // 该文件的新增行集合。
        let added: HashSet<u32> = by_path
            .get(f.path.as_str())
            .map(|fd| {
                fd.hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .filter(|l| l.kind == crate::diff::LineKind::Added)
                    .filter_map(|l| l.new_lineno)
                    .collect()
            })
            .unwrap_or_default();

        let line_count = content.lines().count() as u32;
        if f.start_line > 0 {
            // 模型给了行号：锚点核对通过则直接采信（快路径，不做全量扫描）。
            let file_lines: Vec<String> = content.lines().map(normalize).collect();
            if !f.existing_code.trim().is_empty()
                && !anchor_matches_near(&f.existing_code, &file_lines, f.start_line, ANCHOR_WINDOW)
            {
                // 明显不符：回退全量定位纠正；定位不到则保留模型行号（聊胜于 0）。
                if let Some((s, e)) = locate(&f.existing_code, content, &added) {
                    f.start_line = s;
                    f.end_line = e;
                }
            }
        } else if let Some((s, e)) = locate(&f.existing_code, content, &added) {
            // 模型没给行号：兜底定位。
            f.start_line = s;
            f.end_line = e;
        }

        // 越界保护：行号超出文件长度（短 diff 上模型常外推）且锚点又定位不到 → 置 0（未定位），
        // 不展示不存在的行号。
        if !line_in_range(f.start_line, line_count) || !line_in_range(f.end_line, line_count) {
            f.start_line = 0;
            f.end_line = 0;
        }
    }
}

/// 行号是否落在文件范围内（`1..=line_count`）；`0` 表示未定位，视为合法。
fn line_in_range(line: u32, line_count: u32) -> bool {
    line == 0 || line <= line_count
}

/// 读「新版本」文件内容。
async fn read_new_version(
    repo_root: &Path,
    new_ref: &Option<String>,
    path: &str,
) -> Option<String> {
    // path 来自 LLM 上报的 finding.path（不可信）→ 限制在仓库内，挡 `..`/绝对路径穿越。
    let full = crate::tool::confine_path(repo_root, path).ok()?;
    match new_ref {
        Some(r) => crate::diff::git::git(&["show", &format!("{r}:{path}")])
            .await
            .ok(),
        None => tokio::fs::read_to_string(full).await.ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str =
        "fn main() {\n    let x = 1;\n    let q = format!(\"id={}\", id);\n    run(q);\n}\n";

    fn added(range: std::ops::RangeInclusive<u32>) -> HashSet<u32> {
        range.collect()
    }

    #[test]
    fn locates_single_line_ignoring_indent() {
        let snippet = "let q = format!(\"id={}\", id);";
        let r = locate(snippet, FILE, &added(3..=3));
        assert_eq!(r, Some((3, 3)));
    }

    #[test]
    fn locates_multiline() {
        let snippet = "let q = format!(\"id={}\", id);\n    run(q);";
        let r = locate(snippet, FILE, &HashSet::new());
        assert_eq!(r, Some((3, 4)));
    }

    #[test]
    fn prefers_added_line_on_duplicate() {
        let file = "a();\nx();\na();\n";
        // "a();" 出现在第 1、3 行；新增的是第 3 行 → 应选 3。
        let r = locate("a();", file, &added(3..=3));
        assert_eq!(r, Some((3, 3)));
    }

    #[test]
    fn returns_none_when_absent() {
        assert_eq!(locate("nonexistent_token_xyz", FILE, &HashSet::new()), None);
    }

    #[test]
    fn anchor_validation_accepts_correct_and_nearby_lines() {
        let file: Vec<String> = FILE.lines().map(normalize).collect();
        // 锚点恰在第 3 行；声称第 3 行 → 通过。
        assert!(anchor_matches_near(
            "let q = format!(\"id={}\", id);",
            &file,
            3,
            3
        ));
        // 声称第 5 行，但窗口=3 覆盖到第 3 行 → 仍通过（容忍小偏差）。
        assert!(anchor_matches_near(
            "let q = format!(\"id={}\", id);",
            &file,
            5,
            3
        ));
    }

    #[test]
    fn anchor_validation_rejects_far_off_line() {
        let file: Vec<String> = FILE.lines().map(normalize).collect();
        // 锚点在第 3 行，却声称第 100 行 → 超出窗口 → 拒绝（触发回退定位）。
        assert!(!anchor_matches_near(
            "let q = format!(\"id={}\", id);",
            &file,
            100,
            3
        ));
    }

    #[test]
    fn line_range_guard() {
        // 文件 3 行：1..=3 合法，0 合法（未定位），4+ 越界。
        assert!(line_in_range(0, 3));
        assert!(line_in_range(1, 3));
        assert!(line_in_range(3, 3));
        assert!(!line_in_range(4, 3));
        assert!(!line_in_range(6, 3));
    }

    #[test]
    fn locate_empty_snippet_returns_none() {
        assert_eq!(locate("   ", FILE, &HashSet::new()), None);
    }

    #[test]
    fn locate_ignores_added_lines_when_not_provided() {
        let file = "a();\nx();\na();\n";
        let r = locate("a();", file, &HashSet::new());
        // 无 added_lines 时取最靠前匹配。
        assert_eq!(r, Some((1, 1)));
    }

    #[test]
    fn anchor_validation_trusts_model_when_anchor_empty() {
        let file: Vec<String> = FILE.lines().map(normalize).collect();
        // 锚点为空：无从否定 → 采信模型行号。
        assert!(anchor_matches_near("   ", &file, 2, 3));
    }
}
