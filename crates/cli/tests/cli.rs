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
    assert!(stdout.contains("init"), "help should list init: {stdout}");
    assert!(stdout.contains("demo"), "help should list demo: {stdout}");
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
        .args(["init", "--yes", "--provider", "deepseek", "--config-dir"])
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
    assert!(
        cfg_path.is_file(),
        "config.toml must exist at {}",
        cfg_path.display()
    );
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
    // 饱和式 discovery 的两个旋钮取代了固定 --samples：轮数由「还挖不挖得到新东西」决定。
    assert!(
        stdout.contains("--stop-after-no-new") && stdout.contains("--max-rounds"),
        "security help should expose the saturation knobs: {stdout}"
    );
    // 连描述文本里的 "samples" 也不该留：轮数已由饱和策略决定，任何采样措辞都是误导。
    assert!(
        !lower.contains("samples"),
        "security no longer samples; neither the flag nor the wording should survive: {stdout}"
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

/// 写一个最小的发现会话文件（模拟 `reviewgate review` 的落盘产物）。
fn seed_session(dir: &std::path::Path, id_code: &str) {
    let cache = dir.join(".reviewgate").join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let session = serde_json::json!({
        "version": 2,
        "run_id": "1700000000",
        "created_at": "2023-11-14T22:13:20Z",
        "decision": "block",
        "files_changed": 1,
        "incomplete": false,
        "records": [{
            "seq": 1,
            "id": "abc123def456",
            "status": "open",
            "finding": {
                "dimension": "security",
                "confidence": 0.9,
                "severity": "high",
                "path": "src.rs",
                "start_line": 1,
                "end_line": 1,
                "message": "SQL injection",
                "existing_code": id_code,
                "evidence": "",
                "suggestion_code": "",
                "filtered": false,
                "agreed_dimensions": 1
            }
        }]
    });
    std::fs::write(
        cache.join("findings.json"),
        serde_json::to_string_pretty(&session).unwrap(),
    )
    .unwrap();
}

#[test]
fn cli_findings_list_show_resolve_roundtrip() {
    let dir = temp_dir("rg-findings");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("src.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add src.rs && git commit -q -m init");
    seed_session(&dir, "let q = format!(\"{}\", id);");

    // list：默认只给 open，且必须带上 incomplete/decision 供 agent 判断。
    let out = bin()
        .args(["findings", "list"])
        .current_dir(&dir)
        .output()
        .expect("findings list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "findings list failed: {stdout}");
    assert!(stdout.contains("\"count\": 1"), "{stdout}");
    assert!(stdout.contains("\"incomplete\": false"), "{stdout}");
    assert!(stdout.contains("abc123def456"), "{stdout}");

    // show：短序号可寻址（agent/人对话里好引用）。
    let out = bin()
        .args(["findings", "show", "1"])
        .current_dir(&dir)
        .output()
        .expect("findings show");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "findings show failed: {stdout}");
    assert!(stdout.contains("SQL injection"), "{stdout}");

    // resolve：写回会话，之后 open 列表清空、resolved 列表有 1 条。
    let out = bin()
        .args(["findings", "resolve", "abc123", "--note", "fixed"])
        .current_dir(&dir)
        .output()
        .expect("findings resolve");
    assert!(
        out.status.success(),
        "findings resolve failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = bin()
        .args(["findings", "list"])
        .current_dir(&dir)
        .output()
        .expect("findings list after resolve");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"count\": 0"),
        "resolve 后不应再列出：{stdout}"
    );

    let out = bin()
        .args(["findings", "list", "--status", "resolved"])
        .current_dir(&dir)
        .output()
        .expect("findings list resolved");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"count\": 1"), "{stdout}");
    assert!(stdout.contains("fixed"), "备注应落盘：{stdout}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_findings_without_session_fails_with_guidance() {
    let dir = temp_dir("rg-findings-empty");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("src.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add src.rs && git commit -q -m init");

    let out = bin()
        .args(["findings", "list"])
        .current_dir(&dir)
        .output()
        .expect("findings list");
    assert!(!out.status.success(), "没有会话时不能伪装成空结果");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("reviewgate review"),
        "错误应指出下一步：{stderr}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_since_last_review_refuses_without_a_previous_review() {
    let dir = temp_dir("rg-since-none");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add a.rs && git commit -q -m init");

    // 测试必须自带配置：否则在没有 ~/.reviewgate/config.toml 的干净机器上，
    // 程序会先因「找不到配置」退出，根本走不到这里要验的分支。
    let config = dir.join("reviewgate.toml");
    std::fs::write(
        &config,
        "provider = \"p\"\n[providers.p]\nprotocol = \"openai\"\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-test-not-used\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let out = bin()
        .args(["review", "--since-last-review"])
        .current_dir(&dir)
        .env("REVIEWGATE_CONFIG", &config)
        .output()
        .expect("review --since-last-review");
    assert!(!out.status.success(), "没有上次审查时必须报错而非全量重审");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("previous review"),
        "错误要说明缺的是什么：{stderr}"
    );
}

#[test]
fn cli_since_last_review_rejects_conflicting_range_flags() {
    let dir = temp_dir("rg-since-conflict");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add a.rs && git commit -q -m init");

    // 测试必须自带配置：否则在没有 ~/.reviewgate/config.toml 的干净机器上，
    // 程序会先因「找不到配置」退出，根本走不到这里要验的分支。
    let config = dir.join("reviewgate.toml");
    std::fs::write(
        &config,
        "provider = \"p\"\n[providers.p]\nprotocol = \"openai\"\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-test-not-used\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let out = bin()
        .args(["review", "--since-last-review", "--commit", "HEAD"])
        .current_dir(&dir)
        .env("REVIEWGATE_CONFIG", &config)
        .output()
        .expect("review with conflicting flags");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot be combined"), "{stderr}");
}

#[test]
fn cli_since_last_review_rejects_unreachable_base_commit() {
    let dir = temp_dir("rg-since-gone");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add a.rs && git commit -q -m init");

    // 会话里的基准 sha 在本仓库不存在（模拟 rebase / force-push）。
    let cache = dir.join(".reviewgate").join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let session = serde_json::json!({
        "version": 2,
        "run_id": "1700000000",
        "created_at": "2023-11-14T22:13:20Z",
        "decision": "pass",
        "head_sha": "0123456789abcdef0123456789abcdef01234567",
        "files_changed": 1,
        "incomplete": false,
        "records": []
    });
    std::fs::write(
        cache.join("findings.json"),
        serde_json::to_string_pretty(&session).unwrap(),
    )
    .unwrap();

    // 测试必须自带配置：否则在没有 ~/.reviewgate/config.toml 的干净机器上，
    // 程序会先因「找不到配置」退出，根本走不到这里要验的分支。
    let config = dir.join("reviewgate.toml");
    std::fs::write(
        &config,
        "provider = \"p\"\n[providers.p]\nprotocol = \"openai\"\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-test-not-used\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let out = bin()
        .args(["review", "--since-last-review"])
        .current_dir(&dir)
        .env("REVIEWGATE_CONFIG", &config)
        .output()
        .expect("review --since-last-review");
    assert!(
        !out.status.success(),
        "基准不可达时必须报错，不能悄悄改范围"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no longer in this repository"), "{stderr}");
}

#[test]
fn cli_estimate_only_does_not_move_the_incremental_baseline() {
    // --estimate-only 不调用 LLM、什么都没审。若它覆盖了会话里的基准 commit，
    // 下一次 --since-last-review 就会从新基准开始，把从未审过的改动整段跳过。
    let dir = temp_dir("rg-estimate-baseline");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.rs"), "fn a() {}\n").unwrap();
    run(&dir, "git add a.rs && git commit -q -m c1");

    // 播一个会话，基准 = 第一个 commit。
    let cache = dir.join(".reviewgate").join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join(".gitignore"), "*\n").unwrap();
    let base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&dir)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let session = serde_json::json!({
        "version": 2, "run_id": "1", "created_at": "2023-11-14T22:13:20Z",
        "decision": "pass", "head_sha": base, "files_changed": 1,
        "incomplete": false, "records": []
    });
    std::fs::write(
        cache.join("findings.json"),
        serde_json::to_string_pretty(&session).unwrap(),
    )
    .unwrap();

    // 之后再提交一次，然后只做估算。
    std::fs::write(dir.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    run(&dir, "git commit -q -am c2");

    let config = dir.join("reviewgate.toml");
    std::fs::write(
        &config,
        "provider = \"p\"\n[providers.p]\nprotocol = \"openai\"\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-test-not-used\"\nmodel = \"m\"\n",
    )
    .unwrap();

    let out = bin()
        .args([
            "review",
            "--estimate-only",
            "--format",
            "json",
            "--no-metrics",
        ])
        .current_dir(&dir)
        .env("REVIEWGATE_CONFIG", &config)
        .output()
        .expect("estimate-only review");
    assert!(
        out.status.success(),
        "estimate-only should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cache.join("findings.json")).unwrap())
            .unwrap();
    assert_eq!(
        after["head_sha"].as_str(),
        Some(base.as_str()),
        "估算没有审任何东西，不得推进增量基准"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn cli_daemon_serve_refuses_the_publicly_known_default_secret() {
    // `serve` 在缺 secret 时是报错退出的；`daemon --serve` 起的是同一个 webhook 服务，
    // 不能悄悄回退到源码里公开的常量——那等于把签名校验作废，任何人都能伪造 webhook。
    let dir = temp_dir("rg-daemon-secret");
    run(&dir, "git init -q");
    run(&dir, "git config user.email test@example.com");
    run(&dir, "git config user.name Test");
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    run(&dir, "git add a.rs && git commit -q -m init");

    let out = bin()
        .args([
            "daemon",
            "--serve",
            "--fixture",
            "--repo",
            "acme/demo",
            "--max-iterations",
            "1",
        ])
        .current_dir(&dir)
        .env_remove("REVIEWGATE_WEBHOOK_SECRET")
        .output()
        .expect("run daemon --serve");

    assert!(
        !out.status.success(),
        "缺 webhook secret 时必须失败退出，而不是用公开常量启动服务"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("webhook-secret") || stderr.contains("REVIEWGATE_WEBHOOK_SECRET"),
        "错误要指出该配什么：{stderr}"
    );
}
