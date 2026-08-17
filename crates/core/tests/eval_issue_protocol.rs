//! Issue 评测协议必须可在本机无 token 下自检：脚本能解释自己、groundtruth 能对假库出数。

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

#[test]
fn eval_issue_groundtruth_self_test() {
    let script = repo_root().join("scripts/eval-issue-groundtruth.py");
    let out = Command::new(python())
        .arg(&script)
        .arg("--self-test")
        .output()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", python()));
    assert!(
        out.status.success(),
        "eval-issue-groundtruth.py --self-test failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[cfg(unix)]
fn eval_issue_triage_help_documents_protocol_flags() {
    let script = repo_root().join("scripts/eval-issue-triage.sh");
    let out = Command::new("bash")
        .arg(&script)
        .arg("--help")
        .output()
        .expect("bash eval-issue-triage.sh --help");
    assert!(
        out.status.success(),
        "help must not require GITHUB_TOKEN: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for needle in [
        "--force-retriage",
        "--llm",
        "--no-sync",
        "不发布",
        "--publish",
    ] {
        assert!(
            text.contains(needle),
            "eval-issue-triage.sh --help must mention {needle}:\n{text}"
        );
    }
}

#[test]
#[cfg(unix)]
fn eval_issue_triage_refuses_publish() {
    let script = repo_root().join("scripts/eval-issue-triage.sh");
    let out = Command::new("bash")
        .arg(&script)
        .args(["owner/repo", "--publish"])
        .output()
        .expect("bash eval-issue-triage.sh --publish");
    assert!(
        !out.status.success(),
        "eval must refuse --publish: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("禁止") || err.contains("--publish"),
        "must say why publish is refused: {err}"
    );
}
