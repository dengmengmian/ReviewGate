//! `reviewgate demo` —— 内置有毒样例，验证闸口会 BLOCK（不依赖业务仓库）。

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 故意有缺陷的 Python handler（SQL 拼接）。审查目标：security BLOCK。
pub const POISONED_HANDLER: &str = r#"# ReviewGate demo fixture — INTENTIONAL vulnerability for gate validation.
# Do not copy this pattern into production code.

def delete_user(conn, user_id: str) -> None:
    """Delete a user by id from an untrusted request parameter."""
    # BUG: user_id is interpolated into SQL (classic injection).
    query = f"DELETE FROM users WHERE id = '{user_id}'"
    conn.execute(query)


def get_user(conn, user_id: str):
    # BUG: same injection pattern on SELECT.
    return conn.execute(f"SELECT * FROM users WHERE id = {user_id}").fetchone()
"#;

/// 在 `root` 建一个最小 git 仓库，工作区含有毒改动（供 `DiffMode::Workspace` 审查）。
pub fn seed_demo_repo(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root).with_context(|| format!("mkdir {}", root.display()))?;

    git(root, &["init", "-q"])?;
    git(root, &["config", "user.email", "demo@reviewgate.local"])?;
    git(root, &["config", "user.name", "ReviewGate Demo"])?;
    // Detached default branch name across git versions.
    let _ = git(root, &["checkout", "-b", "main"]);

    // Clean baseline commit (no vuln), then introduce poison as uncommitted worktree change.
    let clean = "# clean baseline\n";
    let handler = root.join("handler.py");
    std::fs::write(&handler, clean).context("write clean handler.py")?;
    git(root, &["add", "handler.py"])?;
    git(root, &["commit", "-q", "-m", "demo: baseline"])?;

    std::fs::write(&handler, POISONED_HANDLER).context("write poisoned handler.py")?;

    // Sanity: workspace must show a diff.
    let status = Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(root)
        .output()
        .context("git diff")?;
    if !status.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
    let stat = String::from_utf8_lossy(&status.stdout);
    if !stat.contains("handler.py") {
        bail!("demo seed produced no workspace diff for handler.py: {stat}");
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to spawn git {:?}", args))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// 分配临时 demo 目录。
pub fn temp_demo_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("reviewgate-demo-{nanos}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_fixture_contains_sql_injection_shape() {
        assert!(POISONED_HANDLER.contains("DELETE FROM users"));
        assert!(
            POISONED_HANDLER.contains("f\"DELETE")
                || POISONED_HANDLER.contains("f'SELECT")
                || POISONED_HANDLER.contains("f\"SELECT")
        );
        assert!(POISONED_HANDLER.contains("user_id"));
    }

    #[test]
    fn seed_demo_repo_creates_workspace_diff() {
        let root = temp_demo_dir();
        seed_demo_repo(&root).expect("seed");
        let body = std::fs::read_to_string(root.join("handler.py")).unwrap();
        assert!(body.contains("DELETE FROM users"));
        let out = Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(out.status.success());
        let names = String::from_utf8_lossy(&out.stdout);
        assert!(
            names.contains("handler.py"),
            "expected handler.py in diff, got {names}"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
