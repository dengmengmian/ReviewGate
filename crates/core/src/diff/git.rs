//! git 子进程封装（异步）。

use anyhow::{Context, Result};
use tokio::process::Command;

/// 跑一条 git 命令，返回 stdout（utf-8 有损）。失败带 stderr。
pub async fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to run git {:?}", args))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "git {:?} 退出码 {:?}：{}",
            args,
            out.status.code(),
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 跑一条 git 命令，不因非零退出码报错。返回 (退出码, stdout)。
/// 用于 `git grep`（无匹配时退出码为 1，并非错误）。
pub async fn git_lenient(args: &[&str]) -> Result<(i32, String)> {
    let out = Command::new("git")
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to run git {:?}", args))?;
    let code = out.status.code().unwrap_or(-1);
    Ok((code, String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// 仓库根目录。
pub async fn repo_root() -> Result<String> {
    Ok(git(&["rev-parse", "--show-toplevel"])
        .await?
        .trim()
        .to_string())
}

/// 未跟踪（且未被 .gitignore 排除）的文件列表。
pub async fn untracked_files() -> Result<Vec<String>> {
    let out = git(&["ls-files", "--others", "--exclude-standard"]).await?;
    Ok(out.lines().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn repo_root_returns_non_empty_absolute_path() {
        let root = repo_root()
            .await
            .expect("repo_root should succeed in git tree");
        assert!(!root.is_empty());
        assert!(std::path::Path::new(&root).is_absolute());
    }

    #[tokio::test]
    async fn rev_parse_head_succeeds() {
        let head = git(&["rev-parse", "HEAD"])
            .await
            .expect("git rev-parse HEAD");
        assert_eq!(head.trim().len(), 40);
    }

    #[tokio::test]
    async fn git_lenient_returns_nonzero_for_invalid_ref() {
        // `git_lenient` must not panic on non-zero exits; it should surface the code.
        let (code, out) = git_lenient(&["rev-parse", "--verify", "__no_such_ref__/HEAD"])
            .await
            .expect("git_lenient should not error on non-zero exit");
        assert_ne!(code, 0);
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn untracked_files_returns_vec_without_panic() {
        let files = untracked_files().await.expect("untracked_files should run");
        // The workspace is git-controlled; this should not fail.
        assert!(files.iter().all(|f| !f.contains('\n')));
    }
}
