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

/// 应用修复。仅处理：已定位（start_line>0）、有 suggestion_code、未被过滤的发现。
///
/// `fix_branch`：`Some("")` 表示新建自动命名的分支，`Some(name)` 用指定名，`None` 改当前分支。
/// 分支仅在「确有可应用的修复」时才创建，避免留下空分支。
/// `assume_yes`（`--fix-all`）：跳过逐条 y/N，直接全部应用，且**不要求交互终端**（供 CI/脚本批量用）；
/// 为 false 时是逐条确认的交互模式，需要终端。
pub fn apply_fixes(
    findings: &[Finding],
    repo_root: &Path,
    fix_branch: Option<&str>,
    assume_yes: bool,
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
    if !assume_yes && !io::stdin().is_terminal() {
        eprintln!(
            "--fix needs interactive confirmation; not a terminal (CI/pipe), so application was skipped. Use --fix-all to apply without prompts, or see the diff above / suggestion_code in the JSON."
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

    if assume_yes {
        eprintln!(
            "\n- Applying all {} auto-applicable fix(es) without confirmation (--fix-all) -",
            fixable.len()
        );
    } else {
        eprintln!("\n- Confirm each fix before applying (you decide) -");
    }
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
            let approved = if assume_yes {
                true
            } else {
                print!("Apply this fix? [y/N] ");
                io::stdout().flush().ok();
                let mut ans = String::new();
                io::stdin().read_line(&mut ans).ok();
                ans.trim().eq_ignore_ascii_case("y")
            };
            if approved {
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

    fn mk_finding(path: &str, line: u32, existing: &str, suggestion: &str) -> Finding {
        use reviewgate_core::model::{Dimension, Reachability, Severity};
        Finding {
            dimension: Dimension::Logic,
            confidence: 0.9,
            severity: Severity::High,
            path: path.into(),
            start_line: line,
            end_line: line,
            message: "test".into(),
            existing_code: existing.into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: suggestion.into(),
            reachability: Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    // --fix-all（assume_yes=true）非交互直接落地：无需 stdin，文件应被改写。
    #[test]
    fn fix_all_applies_without_prompt() {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("rg_fixall_{}_{}", std::process::id(), secs));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.txt");
        std::fs::write(&file, "line1\nBAD\nline3\n").unwrap();

        let findings = vec![mk_finding("t.txt", 2, "BAD", "GOOD")];
        apply_fixes(&findings, &dir, None, true).unwrap();

        let got = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, "line1\nGOOD\nline3\n", "fix-all should apply the fix");
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
