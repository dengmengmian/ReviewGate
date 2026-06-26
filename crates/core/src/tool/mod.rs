//! Agent 工具集：只读专用工具 + 结构化上报。
//!
//! 设计原则：不照搬通用编码 Agent（无写、无任意 shell）。工具刻意只读，
//! 加一个 `report_finding` 结构化上报（M1.8 引入）。这是质量闸口的安全边界。

mod builtins;
mod dup_tool;
mod exec_check;
mod index_tools;

pub use builtins::{CodeSearch, FindFile, ReadFile};
pub use dup_tool::FindDuplicateFunctions;
pub use exec_check::RunCheck;
pub use index_tools::{FindCallers, FindDefinition, FindReferences};

use crate::diff::Diff;
use crate::index::{CodeIndex, GrepIndex, TreeSitterIndex};
use crate::model::ToolDef;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// 工具执行上下文（只读）。
pub struct ToolContext {
    /// 本次审查范围的全部改动。
    pub diff: Arc<Diff>,
    /// 仓库根目录。
    pub repo_root: PathBuf,
    /// 「新版本」内容的来源 ref：
    /// `None` = 工作区（读磁盘）；`Some(ref)` = `git show ref:path`。
    pub new_ref: Option<String>,
    /// 代码上下文检索后端（v0 GrepIndex，v1 可换 TreeSitterIndex）。
    pub index: Arc<dyn CodeIndex>,
    /// 是否允许 `run_check` 沙箱执行（opt-in，默认 false，保留只读信任边界）。
    pub allow_exec: bool,
}

impl ToolContext {
    pub fn new(
        diff: Arc<Diff>,
        repo_root: impl Into<PathBuf>,
        new_ref: Option<String>,
        index: Arc<dyn CodeIndex>,
    ) -> Self {
        Self {
            diff,
            repo_root: repo_root.into(),
            new_ref,
            index,
            allow_exec: false,
        }
    }

    /// 用 GrepIndex 构造（v0 启发式）。
    pub fn with_grep_index(
        diff: Arc<Diff>,
        repo_root: impl Into<PathBuf>,
        new_ref: Option<String>,
    ) -> Self {
        Self::new(diff, repo_root, new_ref, Arc::new(GrepIndex::new()))
    }

    /// 用 TreeSitterIndex 构造（v1 AST 精确，不支持的语言内部回退按行匹配）。
    pub fn with_treesitter_index(
        diff: Arc<Diff>,
        repo_root: impl Into<PathBuf>,
        new_ref: Option<String>,
    ) -> Self {
        Self::new(diff, repo_root, new_ref, Arc::new(TreeSitterIndex::new()))
    }
}

/// 一个可被 Agent 调用的工具。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名（Agent 通过它调用）。
    fn name(&self) -> &str;
    /// 交给 LLM 的工具定义（含 JSON Schema）。
    fn def(&self) -> ToolDef;
    /// 执行；返回给模型看的文本结果。
    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String>;
}

/// 工具注册表：按名字派发。
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// 全部工具定义，交给 LLM。
    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools.values().map(|t| t.def()).collect()
    }

    /// 按名字执行。结果统一截断到 [`MAX_TOOL_RESULT_BYTES`]，防止单次工具结果撑爆上下文。
    pub async fn dispatch(&self, name: &str, input: &Value, ctx: &ToolContext) -> Result<String> {
        match self.tools.get(name) {
            Some(t) => Ok(cap_tool_result(t.call(input, ctx).await?)),
            None => anyhow::bail!("unknown tool: {name}"),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 把模型给的相对路径限制在仓库内，挡住绝对路径与 `..` 穿越。
///
/// 安全边界：工具入参里的 `path` 来自 LLM（不可信）。逐段检查并只接受仓库内的
/// 普通路径段——拒绝绝对路径、`..`、盘符前缀，防止 `read_file` 读到 `/etc/passwd`
/// 或仓库外的敏感文件。返回拼好的仓库内绝对路径。
pub fn confine_path(repo_root: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() {
        anyhow::bail!("path escapes repository: absolute paths are not allowed (`{rel}`)");
    }
    let mut out = repo_root.to_path_buf();
    for comp in p.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!("path escapes repository: `..` is not allowed ({rel})")
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("path escapes repository: {rel}")
            }
        }
    }
    Ok(out)
}

/// 单次工具结果的字节上限。审查时改动文件全文已前置注入，工具结果只需补充少量
/// 跨文件上下文，32 KiB 足矣；超出则截断并提示模型缩小范围，避免拖慢与撑爆上下文。
pub const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;

/// 把工具结果截断到 [`MAX_TOOL_RESULT_BYTES`]（按 UTF-8 字符边界）。
fn cap_tool_result(mut s: String) -> String {
    if s.len() <= MAX_TOOL_RESULT_BYTES {
        return s;
    }
    let mut cut = MAX_TOOL_RESULT_BYTES;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s.push_str(&format!(
        "\n... (result too long; truncated to {} KiB. Use a more precise query or range if more context is needed)",
        MAX_TOOL_RESULT_BYTES / 1024
    ));
    s
}

/// 只读上下文检索工具集：基础三件 + 符号检索三件。
pub fn readonly_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFile),
        Box::new(CodeSearch),
        Box::new(FindFile),
        Box::new(FindDefinition),
        Box::new(FindCallers),
        Box::new(FindReferences),
        Box::new(FindDuplicateFunctions),
        // run_check：默认 inert（ctx.allow_exec=false 时拒绝执行），仅 --exec-verify 开启。
        Box::new(RunCheck),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_result_unchanged() {
        let s = "短结果".to_string();
        assert_eq!(cap_tool_result(s.clone()), s);
    }

    #[test]
    fn confine_path_blocks_traversal_and_absolute() {
        let root = Path::new("/repo");
        // 合法的仓库内路径：接受并拼到 root 下。
        assert_eq!(
            confine_path(root, "src/main.rs").unwrap(),
            Path::new("/repo/src/main.rs")
        );
        assert_eq!(
            confine_path(root, "./a/b.rs").unwrap(),
            Path::new("/repo/a/b.rs")
        );
        // 穿越 / 绝对路径 / 嵌套 .. 全部拒绝。
        assert!(confine_path(root, "../etc/passwd").is_err());
        assert!(confine_path(root, "a/../../etc/passwd").is_err());
        assert!(confine_path(root, "/etc/passwd").is_err());
        assert!(confine_path(root, "/absolute").is_err());
    }

    #[test]
    fn confine_accepts_in_repo_paths() {
        let root = Path::new("/repo");
        assert_eq!(
            confine_path(root, "src/main.rs").unwrap(),
            PathBuf::from("/repo/src/main.rs")
        );
        // 前导 ./ 与多级正常段都接受。
        assert_eq!(
            confine_path(root, "./a/b/c.rs").unwrap(),
            PathBuf::from("/repo/a/b/c.rs")
        );
    }

    #[test]
    fn confine_rejects_absolute_and_traversal() {
        let root = Path::new("/repo");
        assert!(confine_path(root, "/etc/passwd").is_err());
        assert!(confine_path(root, "../../../etc/passwd").is_err());
        assert!(confine_path(root, "src/../../secret").is_err());
    }

    #[test]
    fn long_result_truncated_on_char_boundary() {
        // 多字节字符填充到超限，确保截断不切坏 UTF-8。
        let big = "中".repeat(MAX_TOOL_RESULT_BYTES); // 每个 3 字节，远超上限
        let out = cap_tool_result(big);
        assert!(out.len() <= MAX_TOOL_RESULT_BYTES + 128); // 上限 + 提示语
        assert!(out.contains("truncated"));
        // 仍是合法 UTF-8（String 本身保证），且没有 panic。
    }
}
