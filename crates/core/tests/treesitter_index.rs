//! Integration tests for TreeSitterIndex against a real git repository.

use reviewgate_core::index::{CodeIndex, SymbolKind, TreeSitterIndex};
use std::path::PathBuf;
use std::sync::Mutex;

static CWD_LOCK: Mutex<()> = Mutex::new(());

fn tmp_repo(prefix: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    dir
}

fn run(dir: &std::path::Path, cmd: &str) {
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .status()
        .expect(cmd);
    assert!(status.success(), "command failed: {cmd}");
}

/// Run an async closure with the current working directory temporarily set to `dir`.
// 进程级 cwd 不能并发修改：这里**有意**跨 await 持锁，把依赖 cwd 的测试串行化。
// 测试专用、无死锁风险（无嵌套加锁），故豁免 await_holding_lock。
#[allow(clippy::await_holding_lock)]
async fn with_cwd_async<Fut, R>(dir: &std::path::Path, f: impl FnOnce() -> Fut) -> R
where
    Fut: std::future::Future<Output = R>,
{
    let _guard = CWD_LOCK.lock().unwrap();
    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let r = f().await;
    std::env::set_current_dir(original).unwrap();
    r
}

#[tokio::test]
async fn finds_definition_callers_and_references() {
    let dir = tmp_repo("rg_tsidx");
    std::fs::write(
        dir.join("src.rs"),
        "// helper is useful\nfn helper() {}\nfn main() { helper(); }\n",
    )
    .unwrap();
    run(&dir, "git add src.rs && git commit -q -m init");

    let defs = with_cwd_async(&dir, || async {
        TreeSitterIndex::new().find_definition("helper", None).await
    })
    .await
    .unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].path, "src.rs");
    assert_eq!(defs[0].line, 2);
    assert_eq!(defs[0].kind, SymbolKind::Function);

    let callers = with_cwd_async(&dir, || async {
        TreeSitterIndex::new().find_callers("helper", None).await
    })
    .await
    .unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].line, 3);

    let refs = with_cwd_async(&dir, || async {
        TreeSitterIndex::new().find_references("helper", None).await
    })
    .await
    .unwrap();
    let lines: Vec<u32> = refs.iter().map(|r| r.line).collect();
    assert!(lines.contains(&2), "definition should be a reference");
    assert!(lines.contains(&3), "call site should be a reference");
    assert!(!lines.contains(&1), "comment should be ignored");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn invalid_symbol_returns_empty() {
    let dir = tmp_repo("rg_tsidx_bad");
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add a.rs && git commit -q -m init");

    let result = with_cwd_async(&dir, || async {
        TreeSitterIndex::new().find_definition("123bad", None).await
    })
    .await;
    assert!(result.is_err());

    std::fs::remove_dir_all(&dir).ok();
}
