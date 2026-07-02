//! CLI binary smoke tests.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reviewgate"))
}

/// Create a temporary directory with a unique name inside `/tmp`.
fn temp_dir(prefix: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Run a shell command in the given directory, panicking on failure.
fn run(dir: &std::path::Path, cmd: &str) {
    let status = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .status()
        .expect(cmd);
    assert!(status.success(), "command failed: {cmd}");
}

#[test]
fn cli_help_shows_usage_and_subcommands() {
    let out = bin().arg("--help").output().expect("run reviewgate --help");
    assert!(out.status.success(), "help should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("quality gate") || stdout.contains("reviewgate"),
        "help should describe the tool"
    );
    assert!(
        stdout.contains("review") || stdout.contains("<COMMAND>"),
        "help should list commands"
    );
}

#[test]
fn cli_version_matches_cargo_version() {
    let out = bin()
        .arg("--version")
        .output()
        .expect("run reviewgate --version");
    assert!(out.status.success(), "version should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(stdout.starts_with("reviewgate "));
}

#[test]
fn cli_review_requires_input() {
    // Running `reviewgate review` with no diff/input should fail fast with a usage error.
    let out = bin()
        .args(["review", "--no-confirm"])
        .output()
        .expect("run reviewgate review");
    assert!(!out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("required") || combined.contains("error") || combined.contains("Usage"),
        "expected usage error, got: {combined}"
    );
}

#[test]
fn cli_diff_reports_workspace_changes() {
    let dir = temp_dir("rg-diff-test");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
    run(&dir, "git add a.txt && git commit -q -m init");
    std::fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();

    let out = bin()
        .arg("diff")
        .current_dir(&dir)
        .output()
        .expect("run reviewgate diff");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "diff should succeed. stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("Files changed: 1"),
        "expected one file, got: {stdout}"
    );
    assert!(stdout.contains("a.txt"), "expected a.txt, got: {stdout}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_diff_commit_mode_reports_that_commit() {
    let dir = temp_dir("rg-diff-commit");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
    run(&dir, "git add a.txt && git commit -q -m init");
    std::fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();
    run(&dir, "git add a.txt && git commit -q -m second");

    let out = bin()
        .args(["diff", "--commit", "HEAD"])
        .current_dir(&dir)
        .output()
        .expect("run reviewgate diff --commit");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "diff --commit should succeed. stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("Files changed: 1"),
        "expected one file, got: {stdout}"
    );
    assert!(stdout.contains("a.txt"), "expected a.txt, got: {stdout}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_diff_range_mode_reports_range() {
    let dir = temp_dir("rg-diff-range");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
    run(&dir, "git add a.txt && git commit -q -m init");
    std::fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();
    run(&dir, "git add a.txt && git commit -q -m second");

    let out = bin()
        .args(["diff", "--from", "HEAD~1", "--to", "HEAD"])
        .current_dir(&dir)
        .output()
        .expect("run reviewgate diff --from --to");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "diff range should succeed. stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("Files changed: 1"),
        "expected one file, got: {stdout}"
    );
    assert!(stdout.contains("a.txt"), "expected a.txt, got: {stdout}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_tool_find_file_locates_file() {
    let dir = temp_dir("rg-tool-test");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add . && git commit -q -m init");

    let input = r#"{"keyword":"lib"}"#;
    let out = bin()
        .args(["tool", "find_file", input])
        .current_dir(&dir)
        .output()
        .expect("run reviewgate tool");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "tool should succeed. stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("src/lib.rs") || stdout.contains("lib.rs"),
        "expected lib.rs in output, got: {stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_tool_code_search_finds_pattern() {
    let dir = temp_dir("rg-tool-search");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(
        dir.join("src.rs"),
        "fn hello() {}\nfn main() { hello(); }\n",
    )
    .unwrap();
    run(&dir, "git add src.rs && git commit -q -m init");

    let input = r#"{"pattern":"hello"}"#;
    let out = bin()
        .args(["tool", "code_search", input])
        .current_dir(&dir)
        .output()
        .expect("run reviewgate tool code_search");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "tool code_search should succeed. stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("src.rs") && stdout.contains("hello"),
        "expected matches, got: {stdout}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
