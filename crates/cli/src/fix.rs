//! `--fix`：逐条把 suggestion_code 应用到工作区文件——**每条都人工 y/N 确认**。
//! 非终端（CI/管道）不应用，只提示。

use reviewgate_core::apply::{apply_fix, ApplyError};
use reviewgate_core::model::Finding;
use reviewgate_core::tool::confine_path;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

fn confined_fix_path(repo_root: &Path, path: &str) -> anyhow::Result<PathBuf> {
    confine_path(repo_root, path)
}

/// 交互式应用修复。仅处理：已定位（start_line>0）、有 suggestion_code、未被过滤的发现。
pub fn apply_fixes(findings: &[Finding], repo_root: &Path) -> anyhow::Result<()> {
    let fixable: Vec<&Finding> = findings
        .iter()
        .filter(|f| {
            f.start_line > 0
                && f.end_line >= f.start_line
                && !f.suggestion_code.trim().is_empty()
                && !f.filtered
        })
        .collect();

    if fixable.is_empty() {
        eprintln!("没有可一键应用的修复（需 suggestion_code + 已定位行号）。");
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        eprintln!(
            "--fix 需逐条人工确认；当前非终端（CI/管道），已跳过应用。修复代码见上方 diff 或 JSON 的 suggestion_code。"
        );
        return Ok(());
    }

    eprintln!("\n— 逐条确认应用修复（最终由你决定）—");
    let mut by_path: BTreeMap<&str, Vec<&Finding>> = BTreeMap::new();
    for f in &fixable {
        by_path.entry(f.path.as_str()).or_default().push(f);
    }

    let mut applied = 0usize;
    for (path, mut items) in by_path {
        let full = match confined_fix_path(repo_root, path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("跳过 {path}（路径越界：{e}）");
                continue;
            }
        };
        let Ok(mut content) = std::fs::read_to_string(&full) else {
            eprintln!("跳过 {path}（读不到文件）");
            continue;
        };
        // 自底向上应用：先改下面的行，上面的行号不受影响。
        items.sort_by_key(|f| std::cmp::Reverse(f.start_line));

        let mut file_changed = false;
        for f in items {
            println!(
                "\n{}:{}-{}  [{} · {} · conf {:.2}]",
                path,
                f.start_line,
                f.end_line,
                f.dimension.as_str(),
                f.severity.as_str(),
                f.confidence
            );
            for l in f.existing_code.lines() {
                println!("  \x1b[91m- {}\x1b[0m", l.trim_end());
            }
            for l in f.suggestion_code.lines() {
                println!("  \x1b[92m+ {}\x1b[0m", l.trim_end());
            }
            print!("应用此修复? [y/N] ");
            io::stdout().flush().ok();
            let mut ans = String::new();
            io::stdin().read_line(&mut ans).ok();
            if ans.trim().eq_ignore_ascii_case("y") {
                // 安全：替换前用 existing_code 锚点核对目标行未漂移，不匹配则拒绝以免改错代码。
                match apply_fix(
                    &content,
                    f.start_line,
                    f.end_line,
                    &f.existing_code,
                    &f.suggestion_code,
                ) {
                    Ok(nc) => {
                        content = nc;
                        file_changed = true;
                        applied += 1;
                        println!("✓ 已应用");
                    }
                    Err(ApplyError::OutOfRange) => eprintln!("✗ 行号超出文件范围，跳过"),
                    Err(ApplyError::AnchorMismatch) => {
                        eprintln!("✗ 该处代码与发现时不一致（行号可能已漂移），跳过以免改错")
                    }
                }
            } else {
                println!("跳过");
            }
        }
        if file_changed {
            if let Err(e) = std::fs::write(&full, content) {
                eprintln!("✗ 写入 {path} 失败：{e}");
            }
        }
    }
    eprintln!("\n共应用 {applied} 处修复。建议改完再跑一遍 `reviewgate review` 复核。");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confined_fix_path_rejects_escape_paths() {
        let root = Path::new("/repo");
        assert_eq!(
            confined_fix_path(root, "src/main.rs").unwrap(),
            PathBuf::from("/repo/src/main.rs")
        );
        assert!(confined_fix_path(root, "../secret").is_err());
        assert!(confined_fix_path(root, "/etc/passwd").is_err());
    }
}
