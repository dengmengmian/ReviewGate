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

/// 解析 `--fix-branch` 的分支名：显式给名就用它，留空则按时间戳自动生成。
fn resolve_branch_name(explicit: &str) -> String {
    let e = explicit.trim();
    if !e.is_empty() {
        return e.to_string();
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("reviewgate-fix-{secs}")
}

/// 从当前 HEAD 新建分支并切过去（保留工作区改动）。失败即返回错误，绝不带着
/// 未落地的意图继续在原分支上应用修复。
fn create_and_switch_branch(repo_root: &Path, name: &str) -> anyhow::Result<()> {
    let out = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["checkout", "-b", name])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run git checkout -b: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git checkout -b {name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// 交互式应用修复。仅处理：已定位（start_line>0）、有 suggestion_code、未被过滤的发现。
///
/// `fix_branch`：`Some("")` 表示新建自动命名的分支，`Some(name)` 用指定名，`None` 改当前分支。
/// 分支仅在「确有可应用的修复且处于交互终端」时才创建，避免留下空分支。
pub fn apply_fixes(
    findings: &[Finding],
    repo_root: &Path,
    fix_branch: Option<&str>,
) -> anyhow::Result<()> {
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
        eprintln!("No auto-applicable fixes (need suggestion_code + a located line range).");
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        eprintln!(
            "--fix needs interactive confirmation; not a terminal (CI/pipe), so application was skipped. See the diff above or suggestion_code in the JSON."
        );
        return Ok(());
    }

    // 可选：先在新分支上应用，保持原分支干净。仅在确有可应用修复时才建（见上方 early return）。
    if let Some(explicit) = fix_branch {
        let name = resolve_branch_name(explicit);
        create_and_switch_branch(repo_root, &name)?;
        eprintln!(
            "Created and switched to branch '{name}'. Fixes apply here; your original branch stays untouched."
        );
    }

    eprintln!("\n- Confirm each fix before applying (you decide) -");
    let mut by_path: BTreeMap<&str, Vec<&Finding>> = BTreeMap::new();
    for f in &fixable {
        by_path.entry(f.path.as_str()).or_default().push(f);
    }

    let mut applied = 0usize;
    for (path, mut items) in by_path {
        let full = match confined_fix_path(repo_root, path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skip {path} (path outside repo: {e})");
                continue;
            }
        };
        let Ok(mut content) = std::fs::read_to_string(&full) else {
            eprintln!("skip {path} (cannot read file)");
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
            print!("Apply this fix? [y/N] ");
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
                        println!("OK applied");
                    }
                    Err(ApplyError::OutOfRange) => eprintln!("x line out of range, skipped"),
                    Err(ApplyError::AnchorMismatch) => {
                        eprintln!("x code here differs from when it was found (lines may have drifted); skipped to avoid a wrong edit")
                    }
                }
            } else {
                println!("skipped");
            }
        }
        if file_changed {
            if let Err(e) = std::fs::write(&full, content) {
                eprintln!("x failed to write {path}: {e}");
            }
        }
    }
    eprintln!("\nApplied {applied} fix(es). Re-run `reviewgate review` afterward to re-check.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_branch_name_uses_explicit_or_generates() {
        assert_eq!(resolve_branch_name("my-fixes"), "my-fixes");
        assert_eq!(resolve_branch_name("  spaced  "), "spaced");
        let gen = resolve_branch_name("");
        assert!(gen.starts_with("reviewgate-fix-"), "got {gen}");
        assert!(
            gen.len() > "reviewgate-fix-".len(),
            "should append a timestamp"
        );
    }

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
