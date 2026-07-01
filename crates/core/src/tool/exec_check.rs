//! `run_check`：**opt-in** 沙箱执行自包含代码片段。
//!
//! 用途：让 logic 维度对怀疑有 off-by-one/边界错误的算法，**真正运行边界用例**验证（而非心算）。
//! 信任边界：**默认关闭**（`ctx.allow_exec == false` 时直接拒绝），仅 `--exec-verify` 开启。
//! 当前是弱隔离：临时目录 cwd + 清空环境(仅留 PATH) + 6s 超时 + stdin 关闭 + 输出截断 + 退出即清理。
//! 这不是 OS 级沙箱，不能阻止片段访问绝对路径或网络；仅在可信/隔离的 CI 环境中开启。
//! 仅支持 javascript(node)/python(python3) 的**自包含**片段。

use super::{Tool, ToolContext};
use crate::model::ToolDef;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

const EXEC_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_OUT: usize = 8 * 1024;

pub struct RunCheck;

/// 语言 → (可执行文件, 额外参数, 文件后缀)。仅自包含片段。
fn runtime(lang: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    match lang {
        "javascript" | "js" | "typescript" | "ts" => Some(("node", &[], "js")),
        "python" | "py" => Some(("python3", &["-I"], "py")),
        _ => None,
    }
}

#[async_trait]
impl Tool for RunCheck {
    fn name(&self) -> &str {
        "run_check"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "(Requires --exec-verify.) Run a self-contained snippet in a weakly isolated temporary directory to verify logic. \
Rewrite the suspected off-by-one or boundary-sensitive algorithm as a minimal runnable snippet, feed boundary inputs such as weekend, month end, 0, negative values, or boundary carries, \
and print actual outputs with console.log/print. Compare with expected semantics and report only confirmed bugs. \
Only javascript/python are supported; 6s timeout; this is not an OS-level sandbox and should be used only in trusted or isolated environments."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "language": { "type": "string", "enum": ["javascript", "python"] },
                    "code": { "type": "string", "description": "Self-contained runnable snippet that prints actual results with console.log/print" }
                },
                "required": ["language", "code"]
            }),
        }
    }

    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String> {
        if !ctx.allow_exec {
            return Ok("run_check is not enabled (requires --exec-verify). Use static case simulation instead.".into());
        }
        let lang = input.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let code = input.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let Some((bin, args, ext)) = runtime(lang) else {
            anyhow::bail!("run_check unsupported language: {lang} (only javascript/python)");
        };
        if code.trim().is_empty() {
            anyhow::bail!("run_check missing code");
        }
        run_sandboxed(bin, args, ext, code).await
    }
}

async fn run_sandboxed(bin: &str, args: &[&str], ext: &str, code: &str) -> Result<String> {
    use tokio::process::Command;
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("rg_check_{}_{}", std::process::id(), uniq));
    tokio::fs::create_dir_all(&dir).await?;
    let file = dir.join(format!("s.{ext}"));
    tokio::fs::write(&file, code).await?;

    let path_env = std::env::var("PATH").unwrap_or_default();
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .arg(&file)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", path_env)
        .stdin(std::process::Stdio::null())
        // 必须显式 piped：否则子进程继承父 fd，片段输出会泄漏到 ReviewGate 的
        // stdout（破坏 --format json），且 wait_with_output 拿不到、反给模型「(no output)」。
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let result = match cmd.spawn() {
        Ok(child) => match tokio::time::timeout(EXEC_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(o)) => {
                let so = String::from_utf8_lossy(&o.stdout);
                let se = String::from_utf8_lossy(&o.stderr);
                let mut s = String::new();
                if !so.trim().is_empty() {
                    s.push_str("stdout:\n");
                    s.push_str(&so);
                }
                if !se.trim().is_empty() {
                    s.push_str("\nstderr:\n");
                    s.push_str(&se);
                }
                if s.trim().is_empty() {
                    s = "(no output; remember to print actual results with console.log/print before comparing)".into();
                }
                cap(s)
            }
            Ok(Err(e)) => format!("execution error: {e}"),
            Err(_) => format!(
                "timed out (>{}s); snippet terminated",
                EXEC_TIMEOUT.as_secs()
            ),
        },
        Err(e) => format!("failed to start {bin} (runtime may be missing): {e}"),
    };

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(result)
}

fn cap(mut s: String) -> String {
    if s.len() > MAX_OUT {
        let mut c = MAX_OUT;
        while c > 0 && !s.is_char_boundary(c) {
            c -= 1;
        }
        s.truncate(c);
        s.push_str("\n... (output truncated)");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_mapping() {
        assert_eq!(runtime("javascript").map(|t| t.0), Some("node"));
        assert_eq!(runtime("ts").map(|t| t.0), Some("node"));
        assert_eq!(runtime("python").map(|t| t.0), Some("python3"));
        assert!(runtime("rust").is_none());
    }

    #[test]
    fn cap_truncates_on_boundary() {
        let big = "中".repeat(MAX_OUT);
        let out = cap(big);
        assert!(out.len() <= MAX_OUT + 64);
        assert!(out.contains("truncated"));
    }

    #[tokio::test]
    async fn disabled_by_default_refuses() {
        use crate::diff::Diff;
        use std::sync::Arc;
        // allow_exec 默认 false → 拒绝执行。
        let ctx = ToolContext::with_grep_index(Arc::new(Diff::default()), ".", None);
        let out = RunCheck
            .call(&json!({"language":"python","code":"print(1)"}), &ctx)
            .await
            .unwrap();
        assert!(out.contains("not enabled"));
    }

    // 回归：子进程 stdout/stderr 必须被捕获进返回值（喂给模型），
    // 而不是继承父进程 fd 泄漏到 ReviewGate 自己的 stdout（会破坏 --format json）。
    #[tokio::test]
    async fn captures_child_stdout_instead_of_leaking() {
        use crate::diff::Diff;
        use std::sync::Arc;
        let mut ctx = ToolContext::with_grep_index(Arc::new(Diff::default()), ".", None);
        ctx.allow_exec = true;
        let out = RunCheck
            .call(
                &json!({"language":"python","code":"print('RG_MARKER_7788')"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            out.contains("RG_MARKER_7788"),
            "child stdout must be captured into the tool result, got: {out:?}"
        );
    }
}
