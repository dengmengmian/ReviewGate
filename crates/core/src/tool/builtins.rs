//! 三个只读基础工具：read_file / code_search / find_file。

use super::{confine_path, Tool, ToolContext};
use crate::diff::git;
use crate::model::ToolDef;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

/// 单次结果的安全上限。
const MAX_FILE_LINES: usize = 400;
const MAX_SEARCH_MATCHES: usize = 100;
const MAX_FIND_RESULTS: usize = 100;

/// 读改动文件的「新版本」（带行号）。
pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description:
                "Read the file's new-version content with line numbers. Returns a limited number of lines; use start_line for pagination."
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to the repository root" },
                    "start_line": { "type": "integer", "description": "Start line, 1-based; defaults to 1" },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return; defaults to 400" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("read_file missing path")?;
        let start = input
            .get("start_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(MAX_FILE_LINES as u64)
            .min(MAX_FILE_LINES as u64) as usize;

        let content = match &ctx.new_ref {
            // git show 以仓库为根解析，天然受限于仓库内；但仍挡住绝对路径/穿越以早失败。
            Some(r) => {
                confine_path(&ctx.repo_root, path)?;
                git::git(&["show", &format!("{r}:{path}")]).await?
            }
            None => {
                let full = confine_path(&ctx.repo_root, path)?;
                tokio::fs::read_to_string(&full)
                    .await
                    .with_context(|| format!("failed to read file: {}", full.display()))?
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if start > total {
            return Ok(format!(
                "(file has {total} lines; start_line={start} is out of range)"
            ));
        }
        let end = (start - 1 + limit).min(total);
        let mut out = String::new();
        for (i, line) in lines[start - 1..end].iter().enumerate() {
            out.push_str(&format!("{:>6}\t{}\n", start + i, line));
        }
        if end < total {
            out.push_str(&format!(
                "... ({} more lines; continue with start_line={})\n",
                total - end,
                end + 1
            ));
        }
        Ok(out)
    }
}

/// 全仓搜索（git grep）。
pub struct CodeSearch;

#[async_trait]
impl Tool for CodeSearch {
    fn name(&self) -> &str {
        "code_search"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "Search the repository for a string or regex and return matching file:line:content entries. Use this to check whether an issue is handled elsewhere."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Search pattern" },
                    "regex": { "type": "boolean", "description": "Whether to treat pattern as regex; default false means literal search" },
                    "path": { "type": "string", "description": "Optional subpath to restrict the search" }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .context("code_search missing pattern")?;
        let regex = input
            .get("regex")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let path = input.get("path").and_then(|v| v.as_str());
        // 限定子路径同样不可越界（git grep 也会拒绝仓库外路径，这里早失败并给清晰错误）。
        if let Some(p) = path {
            confine_path(&ctx.repo_root, p)?;
        }

        let mut args: Vec<&str> = vec!["grep", "-n", "-I", "--no-color"];
        if regex {
            args.push("-E");
        } else {
            args.push("-F");
        }
        args.push("-e");
        args.push(pattern);
        if let Some(p) = path {
            args.push("--");
            args.push(p);
        }

        let (code, stdout) = git::git_lenient(&args).await?;
        if code == 1 || stdout.trim().is_empty() {
            return Ok("(no matches)".into());
        }
        if code != 0 {
            anyhow::bail!("git grep exited with code {code}");
        }
        let lines: Vec<&str> = stdout.lines().collect();
        let shown = lines.len().min(MAX_SEARCH_MATCHES);
        let mut out = lines[..shown].join("\n");
        if lines.len() > shown {
            out.push_str(&format!(
                "\n... ({} total matches; showing first {})",
                lines.len(),
                shown
            ));
        }
        Ok(out)
    }
}

/// 按文件名关键字找文件（git ls-files）。
pub struct FindFile;

#[async_trait]
impl Tool for FindFile {
    fn name(&self) -> &str {
        "find_file"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "Find repository file paths by filename keyword.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "keyword": { "type": "string", "description": "Keyword contained in the filename" }
                },
                "required": ["keyword"]
            }),
        }
    }

    async fn call(&self, input: &Value, _ctx: &ToolContext) -> Result<String> {
        let keyword = input
            .get("keyword")
            .and_then(|v| v.as_str())
            .context("find_file missing keyword")?;

        let (_, stdout) = git::git_lenient(&["ls-files"]).await?;
        let matches: Vec<&str> = stdout
            .lines()
            .filter(|l| l.contains(keyword))
            .take(MAX_FIND_RESULTS)
            .collect();
        if matches.is_empty() {
            Ok("(no matching files)".into())
        } else {
            Ok(matches.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Diff;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    static NEXT_TMP: AtomicUsize = AtomicUsize::new(0);

    /// 在临时目录建一个仓库根 + 一个文件，返回 (临时目录, ToolContext)。
    fn ctx_with_file() -> (std::path::PathBuf, ToolContext) {
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = NEXT_TMP.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "rg_builtins_{}_{}_{}",
            std::process::id(),
            uniq,
            seq
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.txt"), "line1\nline2\n").unwrap();
        // new_ref=None → 走磁盘读取路径（confine_path 生效的分支）。
        let ctx = ToolContext::with_grep_index(Arc::new(Diff::default()), dir.clone(), None);
        (dir, ctx)
    }

    #[tokio::test]
    async fn read_file_reads_in_repo_path() {
        let (dir, ctx) = ctx_with_file();
        let out = ReadFile
            .call(&json!({ "path": "ok.txt" }), &ctx)
            .await
            .unwrap();
        assert!(out.contains("line1") && out.contains("line2"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_file_rejects_absolute_and_traversal() {
        let (dir, ctx) = ctx_with_file();
        // 绝对路径：绝不能读到仓库外（如 /etc/passwd）。
        assert!(ReadFile
            .call(&json!({ "path": "/etc/passwd" }), &ctx)
            .await
            .is_err());
        // `..` 穿越同样拒绝。
        assert!(ReadFile
            .call(&json!({ "path": "../../../etc/passwd" }), &ctx)
            .await
            .is_err());
        // 含 `..` 的相对路径也拒绝。
        assert!(ReadFile
            .call(&json!({ "path": "sub/../../secret" }), &ctx)
            .await
            .is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn code_search_rejects_out_of_repo_subpath() {
        let (dir, ctx) = ctx_with_file();
        // 限定子路径越界应在调 git grep 前就被拒。
        assert!(CodeSearch
            .call(&json!({ "pattern": "x", "path": "../outside" }), &ctx)
            .await
            .is_err());
        assert!(CodeSearch
            .call(&json!({ "pattern": "x", "path": "/etc" }), &ctx)
            .await
            .is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
