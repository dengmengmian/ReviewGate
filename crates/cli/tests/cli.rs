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

/// Run `git ...` command(s) in `dir`, panicking on failure. `&&`-chained commands are supported.
/// Calls `git` directly (no `bash -c`) so it works on Windows CI, where the bash dependency is flaky.
/// Only git commands with whitespace-separable args are supported (sufficient for these tests).
fn run(dir: &std::path::Path, cmd: &str) {
    for seg in cmd.split("&&") {
        let parts: Vec<&str> = seg.split_whitespace().collect();
        assert_eq!(
            parts.first().copied(),
            Some("git"),
            "run() only supports git commands: {seg}"
        );
        let status = Command::new("git")
            .args(&parts[1..])
            .current_dir(dir)
            .status()
            .unwrap_or_else(|e| panic!("failed to spawn git for `{seg}`: {e}"));
        assert!(status.success(), "command failed: {seg}");
    }
}

#[test]
fn cli_review_help_lists_phase2_flags() {
    let out = bin()
        .args(["review", "--help"])
        .output()
        .expect("review --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--profile",
        "--max-cost",
        "--max-input-tokens",
        "--estimate-only",
        "--no-metrics",
    ] {
        assert!(
            stdout.contains(flag),
            "review --help should list {flag}: {stdout}"
        );
    }
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
    assert!(
        stdout.contains("security"),
        "help should list the security deep-review command: {stdout}"
    );
    assert!(
        stdout.contains("init"),
        "help should list init: {stdout}"
    );
    assert!(
        stdout.contains("demo"),
        "help should list demo: {stdout}"
    );
}

#[test]
fn cli_init_help_lists_noninteractive_flags() {
    let out = bin()
        .args(["init", "--help"])
        .output()
        .expect("run reviewgate init --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--provider",
        "--protocol",
        "--base-url",
        "--model",
        "--yes",
        "--force",
        "--config-dir",
    ] {
        assert!(
            stdout.contains(flag),
            "init --help should list {flag}: {stdout}"
        );
    }
}

#[test]
fn cli_demo_help_lists_prepare_only() {
    let out = bin()
        .args(["demo", "--help"])
        .output()
        .expect("run reviewgate demo --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--prepare-only"),
        "demo --help should list --prepare-only: {stdout}"
    );
    assert!(
        stdout.contains("--keep"),
        "demo --help should list --keep: {stdout}"
    );
}

#[test]
fn cli_init_writes_config_noninteractive() {
    let dir = temp_dir("rg-init-cli");
    let out = bin()
        .args([
            "init",
            "--yes",
            "--provider",
            "deepseek",
            "--config-dir",
        ])
        .arg(&dir)
        .output()
        .expect("run reviewgate init");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "init should succeed. stdout={stdout} stderr={stderr}"
    );
    let cfg_path = dir.join("config.toml");
    assert!(cfg_path.is_file(), "config.toml must exist at {}", cfg_path.display());
    let cfg = std::fs::read_to_string(&cfg_path).expect("config written");
    assert!(cfg.contains("provider = \"deepseek\""));
    assert!(cfg.contains("protocol = \"openai\""));
    assert!(cfg.contains("api.deepseek.com"));
    assert!(cfg.contains("model = "));
    assert!(stderr.contains("REVIEWGATE_API_KEY") || stdout.contains("REVIEWGATE_API_KEY"));
    // No live secret and no active api_key assignment.
    assert!(!cfg.contains("sk-"));
    for line in cfg.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        assert!(
            !t.starts_with("api_key"),
            "init must not embed api_key: {t}"
        );
    }
    // Second write without --force must fail.
    let out2 = bin()
        .args(["init", "--yes", "--config-dir"])
        .arg(&dir)
        .output()
        .expect("init again");
    assert!(
        !out2.status.success(),
        "second init without --force should fail"
    );
    // --force overwrites successfully.
    let out3 = bin()
        .args([
            "init",
            "--yes",
            "--force",
            "--provider",
            "openai",
            "--config-dir",
        ])
        .arg(&dir)
        .output()
        .expect("init --force");
    assert!(
        out3.status.success(),
        "init --force should succeed: {}",
        String::from_utf8_lossy(&out3.stderr)
    );
    let cfg2 = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        cfg2.contains("provider = \"openai\""),
        "force overwrite should switch provider: {cfg2}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_init_custom_provider_requires_url_and_model() {
    let dir = temp_dir("rg-init-custom");
    let bad = bin()
        .args(["init", "--yes", "--provider", "custom", "--config-dir"])
        .arg(&dir)
        .output()
        .expect("init custom incomplete");
    assert!(
        !bad.status.success(),
        "custom without base-url/model must fail"
    );
    let ok = bin()
        .args([
            "init",
            "--yes",
            "--provider",
            "custom",
            "--base-url",
            "http://127.0.0.1:9999/v1",
            "--model",
            "local-model",
            "--protocol",
            "openai",
            "--config-dir",
        ])
        .arg(&dir)
        .output()
        .expect("init custom full");
    assert!(
        ok.status.success(),
        "custom init should succeed: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(cfg.contains("provider = \"custom\""));
    assert!(cfg.contains("http://127.0.0.1:9999/v1"));
    assert!(cfg.contains("local-model"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_demo_prepare_only_seeds_workspace() {
    let out = bin()
        .args(["demo", "--prepare-only"])
        .output()
        .expect("run reviewgate demo --prepare-only");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "prepare_only should succeed. stdout={stdout} stderr={stderr}"
    );
    let path = stdout.lines().next().unwrap_or("").trim();
    assert!(
        !path.is_empty(),
        "prepare-only must print demo path on stdout"
    );
    let root = std::path::Path::new(path);
    assert!(
        root.join("handler.py").is_file(),
        "expected handler.py under {path}"
    );
    let body = std::fs::read_to_string(root.join("handler.py")).unwrap();
    assert!(
        body.contains("DELETE FROM users") && body.contains("user_id"),
        "fixture must be SQL-injection-shaped: {body}"
    );
    // Real git workspace diff (same path full demo will review).
    let diff = Command::new("git")
        .args(["diff", "--name-only"])
        .current_dir(root)
        .output()
        .expect("git diff in demo workspace");
    assert!(diff.status.success(), "git diff should work in demo repo");
    let names = String::from_utf8_lossy(&diff.stdout);
    assert!(
        names.contains("handler.py"),
        "demo workspace must have handler.py in git diff: {names}"
    );
    std::fs::remove_dir_all(path).ok();
}

/// Full demo reuses the shipped review pipeline: without config it must fail closed
/// (exit 2), not silently PASS — proves wiring reaches Config::load / gate path.
#[test]
fn cli_demo_full_without_config_fails_closed() {
    let dir = temp_dir("rg-demo-noconfig");
    // Isolate from any user ~/.reviewgate/config.toml and project reviewgate.toml.
    let out = bin()
        .args(["demo"])
        .env_remove("REVIEWGATE_API_KEY")
        .env_remove("REVIEWGATE_CONFIG")
        .env("HOME", &dir)
        .env("USERPROFILE", &dir)
        .current_dir(&dir)
        .output()
        .expect("run reviewgate demo without config");
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(
        code, 0,
        "demo without config must not succeed/PASS: stderr={stderr}"
    );
    assert!(
        code == 2 || !out.status.success(),
        "expected tool error exit, got {code}: {stderr}"
    );
    assert!(
        stderr.contains("reviewgate.toml")
            || stderr.contains("not found")
            || stderr.contains("init")
            || stderr.contains("API key")
            || stderr.contains("config"),
        "error should point at missing config/key: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_security_help_describes_deep_review() {
    let out = bin()
        .args(["security", "--help"])
        .output()
        .expect("run reviewgate security --help");
    assert!(out.status.success(), "security --help should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lower = stdout.to_ascii_lowercase();
    assert!(
        lower.contains("security")
            && (lower.contains("deep")
                || lower.contains("sink")
                || lower.contains("secret")
                || lower.contains("review")),
        "security help should describe security-focused review: {stdout}"
    );
    // Core range flags must be available (same as review).
    assert!(
        stdout.contains("--commit") && stdout.contains("--from") && stdout.contains("--to"),
        "security help should accept range flags: {stdout}"
    );
    assert!(
        stdout.contains("--samples"),
        "security help should expose samples: {stdout}"
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
