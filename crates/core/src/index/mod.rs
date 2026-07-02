//! 代码上下文检索（CodeIndex）。
//!
//! 关键设计：`find_definition` / `find_callers` / `find_references` 对 Agent 的
//! 工具签名**固定不变**，背后委托给可替换的 [`CodeIndex`]。
//!
//! - v0 [`GrepIndex`]：基于 `git grep` 的启发式，即装即用。
//! - v1 `TreeSitterIndex`（未来）：tree-sitter AST 精确解析 + 缓存，
//!   升级时只替换注入的实现，Agent 与工具层零改动。

mod cache;
mod grep;
mod treesitter;

pub use cache::CachingIndex;
pub use grep::GrepIndex;
pub use treesitter::{list_function_bodies, FnBody, TreeSitterIndex};

use anyhow::Result;
use async_trait::async_trait;

/// 符号类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Type,
    Variable,
    /// 引用/调用点等非定义位置。
    Reference,
    Other,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Type => "type",
            SymbolKind::Variable => "variable",
            SymbolKind::Reference => "reference",
            SymbolKind::Other => "other",
        }
    }
}

/// 语言（用于将来 tree-sitter 选 grammar；v0 仅作提示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Go,
    TypeScript,
    JavaScript,
    Python,
    Java,
    Cpp,
    Other,
}

impl Lang {
    /// 由文件扩展名推断。
    pub fn from_path(path: &str) -> Lang {
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => Lang::Rust,
            "go" => Lang::Go,
            "ts" | "tsx" => Lang::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
            "py" => Lang::Python,
            "java" => Lang::Java,
            "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" => Lang::Cpp,
            _ => Lang::Other,
        }
    }
}

/// 一个符号位置。
#[derive(Debug, Clone)]
pub struct SymbolLoc {
    pub path: String,
    /// 1-based 行号。
    pub line: u32,
    /// 1-based 列号（best-effort）。
    pub col: u32,
    pub kind: SymbolKind,
    /// 该行内容（已 trim）。
    pub snippet: String,
}

/// 上下文检索后端。工具委托给它，实现可替换。
#[async_trait]
pub trait CodeIndex: Send + Sync {
    /// 找符号定义位置。
    async fn find_definition(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>>;
    /// 找函数/方法的调用点。
    async fn find_callers(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>>;
    /// 找符号的所有引用。
    async fn find_references(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_from_path_maps_extensions() {
        assert!(matches!(Lang::from_path("src/main.rs"), Lang::Rust));
        assert!(matches!(Lang::from_path("app.go"), Lang::Go));
        assert!(matches!(Lang::from_path("component.tsx"), Lang::TypeScript));
        assert!(matches!(Lang::from_path("script.jsx"), Lang::JavaScript));
        assert!(matches!(Lang::from_path("main.py"), Lang::Python));
        assert!(matches!(Lang::from_path("Bean.java"), Lang::Java));
        assert!(matches!(Lang::from_path("lib.cpp"), Lang::Cpp));
        assert!(matches!(Lang::from_path("lib.cxx"), Lang::Cpp));
        assert!(matches!(Lang::from_path("header.h"), Lang::Cpp));
        assert!(matches!(Lang::from_path("Dockerfile"), Lang::Other));
    }

    #[test]
    fn symbol_kind_as_str() {
        assert_eq!(SymbolKind::Function.as_str(), "function");
        assert_eq!(SymbolKind::Type.as_str(), "type");
        assert_eq!(SymbolKind::Variable.as_str(), "variable");
        assert_eq!(SymbolKind::Reference.as_str(), "reference");
        assert_eq!(SymbolKind::Other.as_str(), "other");
    }
}
