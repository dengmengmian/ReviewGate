//! diff 采集集成测试：Workspace / Commit / Range 三模式 + 未跟踪文件（真实 git 仓库）。
//!
//! 单文件单测——会切换进程 CWD 到临时仓库，故只放一个测试避免与并行测试竞争
//! （集成测试各文件是独立二进制，不影响 lib 单测）。

use reviewgate_core::diff::{self, DiffMode};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git 可执行");
    assert!(
        out.status.success(),
        "git {args:?} 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn diff_modes_workspace_commit_range_and_untracked() {
    let tmp = std::env::temp_dir().join(format!("rg_diffmodes_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q"]);
    git(&tmp, &["config", "user.email", "t@t.co"]);
    git(&tmp, &["config", "user.name", "t"]);

    // c1：初始；c2：改 a.txt 第 2 行（+1 −1）。
    std::fs::write(tmp.join("a.txt"), "l1\nl2\nl3\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "c1"]);
    let sha1 = git(&tmp, &["rev-parse", "HEAD"]);
    std::fs::write(tmp.join("a.txt"), "l1\nMODIFIED\nl3\n").unwrap();
    git(&tmp, &["add", "-A"]);
    git(&tmp, &["commit", "-qm", "c2"]);
    let sha2 = git(&tmp, &["rev-parse", "HEAD"]);

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();

    // Commit 模式：审 c2 引入的改动。
    let d = diff::collect(&DiffMode::Commit(sha2.clone()))
        .await
        .unwrap();
    assert_eq!(d.files.len(), 1, "c2 只改了 a.txt");
    assert_eq!(d.files[0].new_path.as_deref(), Some("a.txt"));
    assert_eq!(d.files[0].added_lines(), 1);
    assert_eq!(d.files[0].deleted_lines(), 1);
    assert!(!d.files[0].hunks.is_empty(), "应解析出 hunk");

    // Range 模式：sha1...sha2 = PR 引入的改动。
    let d = diff::collect(&DiffMode::Range {
        from: sha1.clone(),
        to: sha2.clone(),
    })
    .await
    .unwrap();
    assert_eq!(d.files.len(), 1);
    assert!(d.files[0].added_lines() >= 1);

    // Workspace 模式：工作区改 a.txt（未提交）+ 新建未跟踪 b.txt → 两者都应采到。
    std::fs::write(tmp.join("a.txt"), "l1\nMODIFIED\nl3\nl4-uncommitted\n").unwrap();
    std::fs::write(tmp.join("b.txt"), "new untracked file\n").unwrap();
    let d = diff::collect(&DiffMode::Workspace).await.unwrap();
    let paths: Vec<&str> = d
        .files
        .iter()
        .filter_map(|f| f.new_path.as_deref())
        .collect();
    assert!(paths.contains(&"a.txt"), "工作区改动应含 a.txt：{paths:?}");
    assert!(paths.contains(&"b.txt"), "未跟踪文件应被采到：{paths:?}");

    std::env::set_current_dir(&prev).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
}
