//! GrepIndex：基于 `git grep` 的启发式 CodeIndex（v0）。
//!
//! 用一次词匹配搜索取得全部引用，再在 Rust 侧按「定义关键字」「调用形态」
//! 筛分定义/调用。避免依赖各平台 git grep 的正则边界差异，稳健且零额外依赖。
//! v1 将由 tree-sitter 精确实现替换，本类型的对外行为即 CodeIndex 契约。

use super::{CodeIndex, Lang, SymbolKind, SymbolLoc};
use crate::diff::git;
use anyhow::Result;
use async_trait::async_trait;

const MAX_RESULTS: usize = 100;

/// 可作为「定义」前缀的关键字（跨语言并集）。
const DEF_KEYWORDS: &[&str] = &[
    "fn",
    "func",
    "fun",
    "def",
    "function",
    "class",
    "struct",
    "enum",
    "interface",
    "trait",
    "type",
    "const",
    "val",
    "var",
    "let",
    "module",
    "namespace",
    "impl",
];

pub struct GrepIndex;

impl GrepIndex {
    pub fn new() -> Self {
        GrepIndex
    }

    /// 该符号的全部词匹配引用。
    async fn references_raw(&self, symbol: &str) -> Result<Vec<SymbolLoc>> {
        if !is_identifier(symbol) {
            anyhow::bail!("符号名非法：{symbol}");
        }
        let (code, stdout) =
            git::git_lenient(&["grep", "-nw", "-I", "--no-color", "-e", symbol]).await?;
        if code == 1 || stdout.trim().is_empty() {
            return Ok(Vec::new());
        }
        if code != 0 {
            anyhow::bail!("git grep 退出码 {code}");
        }
        let mut locs = Vec::new();
        for line in stdout.lines().take(MAX_RESULTS) {
            if let Some(loc) = parse_grep_line(line, symbol) {
                locs.push(loc);
            }
        }
        Ok(locs)
    }
}

impl Default for GrepIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CodeIndex for GrepIndex {
    async fn find_definition(&self, symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        let mut out = Vec::new();
        for mut loc in self.references_raw(symbol).await? {
            if let Some(kind) = definition_kind(&loc.snippet, symbol) {
                loc.kind = kind;
                out.push(loc);
            }
        }
        Ok(out)
    }

    async fn find_callers(&self, symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        let mut out = Vec::new();
        for mut loc in self.references_raw(symbol).await? {
            // 是调用形态、且不是定义点（定义行 `fn foo(` 也含 `foo(`）。
            if is_call_site(&loc.snippet, symbol) && definition_kind(&loc.snippet, symbol).is_none()
            {
                loc.kind = SymbolKind::Reference;
                out.push(loc);
            }
        }
        Ok(out)
    }

    async fn find_references(&self, symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        Ok(self
            .references_raw(symbol)
            .await?
            .into_iter()
            .map(|mut l| {
                l.kind = SymbolKind::Reference;
                l
            })
            .collect())
    }
}

/// 解析 `path:line:content` 一行。
fn parse_grep_line(line: &str, symbol: &str) -> Option<SymbolLoc> {
    let mut it = line.splitn(3, ':');
    let path = it.next()?.to_string();
    let lineno: u32 = it.next()?.parse().ok()?;
    let content = it.next().unwrap_or("");
    let col = content.find(symbol).map(|b| b as u32 + 1).unwrap_or(1);
    Some(SymbolLoc {
        path,
        line: lineno,
        col,
        kind: SymbolKind::Other,
        snippet: content.trim().to_string(),
    })
}

/// 该行是否像 `symbol` 的定义；是则返回推断的类别。
fn definition_kind(content: &str, symbol: &str) -> Option<SymbolKind> {
    let sym_pos = content.find(symbol)?;
    let before = &content[..sym_pos];
    // 找紧邻 symbol 之前的关键字 token。
    let last_kw = before
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .rfind(|t| !t.is_empty());
    let kw = last_kw?;
    if !DEF_KEYWORDS.contains(&kw) {
        return None;
    }
    let kind = match kw {
        "fn" | "func" | "fun" | "def" | "function" => SymbolKind::Function,
        "class" | "struct" | "enum" | "interface" | "trait" | "type" => SymbolKind::Type,
        "const" | "val" | "var" | "let" => SymbolKind::Variable,
        _ => SymbolKind::Other,
    };
    Some(kind)
}

/// 该行是否像对 `symbol` 的调用（symbol 紧跟 `(`）。
fn is_call_site(content: &str, symbol: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = content[from..].find(symbol) {
        let pos = from + rel;
        let after = content[pos + symbol.len()..].trim_start();
        if after.starts_with('(') {
            return true;
        }
        from = pos + symbol.len();
    }
    false
}

/// 仅允许标识符（字母/数字/下划线，且不以数字开头）。
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_definition_keywords() {
        assert_eq!(
            definition_kind("fn login(id: u32) {", "login"),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            definition_kind("struct User {", "User"),
            Some(SymbolKind::Type)
        );
        assert_eq!(definition_kind("    run(login)", "login"), None);
    }

    #[test]
    fn detects_call_site() {
        assert!(is_call_site("    let q = login(id);", "login"));
        assert!(is_call_site("audit (user)", "audit"));
        assert!(!is_call_site("let login = 1;", "login"));
    }

    #[test]
    fn validates_identifier() {
        assert!(is_identifier("foo_bar"));
        assert!(is_identifier("_x"));
        assert!(!is_identifier("9x"));
        assert!(!is_identifier("a.b"));
        assert!(!is_identifier(""));
    }
}
