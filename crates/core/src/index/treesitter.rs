//! TreeSitterIndex：基于 tree-sitter AST 的精确 CodeIndex（v1）。
//!
//! 混合策略：先用 `git grep -l` 做**文件级**快速预筛（拿到含该符号的候选文件），
//! 再对候选文件做 **AST 解析**精确分类——区分定义 / 调用 / 引用，且天然跳过
//! 注释与字符串里的同名文本（这是相对 GrepIndex 的核心精度提升）。
//! 不支持的语言回退到按行匹配。

use super::{CodeIndex, Lang, SymbolKind, SymbolLoc};
use crate::diff::git;
use anyhow::Result;
use async_trait::async_trait;
use tree_sitter::{Node, Parser};

const MAX_FILES: usize = 60;
const MAX_RESULTS: usize = 100;

#[derive(Clone, Copy)]
enum Mode {
    Definition,
    Caller,
    Reference,
}

/// 每种语言的 AST 节点配置。
struct LangSpec {
    def_kinds: &'static [&'static str],
    type_kinds: &'static [&'static str],
    var_kinds: &'static [&'static str],
    call_kinds: &'static [&'static str],
    call_fn_field: &'static str,
}

pub struct TreeSitterIndex;

impl TreeSitterIndex {
    pub fn new() -> Self {
        TreeSitterIndex
    }

    /// 候选文件：含该符号（词匹配）的文件列表。
    async fn candidate_files(&self, symbol: &str) -> Result<Vec<String>> {
        let (code, stdout) =
            git::git_lenient(&["grep", "-l", "-w", "-I", "--no-color", "-e", symbol]).await?;
        if code == 1 || stdout.trim().is_empty() {
            return Ok(Vec::new());
        }
        if code != 0 {
            anyhow::bail!("git grep -l exited with code {code}");
        }
        Ok(stdout
            .lines()
            .take(MAX_FILES)
            .map(|s| s.to_string())
            .collect())
    }

    async fn collect(&self, symbol: &str, mode: Mode) -> Result<Vec<SymbolLoc>> {
        if !is_identifier(symbol) {
            anyhow::bail!("invalid symbol name: {symbol}");
        }
        let mut out = Vec::new();
        for path in self.candidate_files(symbol).await? {
            if out.len() >= MAX_RESULTS {
                break;
            }
            let Ok(source) = tokio::fs::read_to_string(&path).await else {
                continue;
            };
            match lang_spec(&path) {
                Some((language, spec)) => {
                    scan_ast(&path, &source, language, &spec, symbol, mode, &mut out);
                }
                None => {
                    // 不支持的语言：回退到按行词匹配。
                    fallback_lines(&path, &source, symbol, &mut out);
                }
            }
        }
        out.truncate(MAX_RESULTS);
        Ok(out)
    }
}

impl Default for TreeSitterIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CodeIndex for TreeSitterIndex {
    async fn find_definition(&self, symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.collect(symbol, Mode::Definition).await
    }
    async fn find_callers(&self, symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.collect(symbol, Mode::Caller).await
    }
    async fn find_references(&self, symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.collect(symbol, Mode::Reference).await
    }
}

/// 一个函数定义及其函数体（供重复检测用）。
#[derive(Debug, Clone)]
pub struct FnBody {
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    /// 函数体文本（有 `body` 字段时取 body 块，否则取整个定义节点）。
    pub body_text: String,
}

/// 列出文件里所有**函数定义**（不含类型/常量）及其函数体文本。
/// 不支持的语言返回空。用于确定性重复函数检测。
pub fn list_function_bodies(path: &str, source: &str) -> Vec<FnBody> {
    let Some((language, spec)) = lang_spec(path) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let bytes = source.as_bytes();
    // 函数类定义 = def_kinds 去掉类型/变量类。
    let fn_kinds: Vec<&str> = spec
        .def_kinds
        .iter()
        .filter(|k| !spec.type_kinds.contains(k) && !spec.var_kinds.contains(k))
        .copied()
        .collect();

    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if fn_kinds.contains(&node.kind()) {
            if let Some(name) = def_name(&node, bytes) {
                // 取 body 块（捕获"改名但体相同"的复制）；无 body 字段则取整节点。
                let body_node = node.child_by_field_name("body").unwrap_or(node);
                out.push(FnBody {
                    name,
                    start_line: node.start_position().row as u32 + 1,
                    end_line: node.end_position().row as u32 + 1,
                    body_text: text(&body_node, bytes).to_string(),
                });
            }
        }
        for i in 0..node.child_count() {
            if let Some(c) = node.child(i as u32) {
                stack.push(c);
            }
        }
    }
    out
}

/// 解析单个文件并按 mode 收集匹配。
fn scan_ast(
    path: &str,
    source: &str,
    language: tree_sitter::Language,
    spec: &LangSpec,
    symbol: &str,
    mode: Mode,
    out: &mut Vec<SymbolLoc>,
) {
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        fallback_lines(path, source, symbol, out);
        return;
    }
    let Some(tree) = parser.parse(source, None) else {
        fallback_lines(path, source, symbol, out);
        return;
    };
    let lines: Vec<&str> = source.lines().collect();
    let bytes = source.as_bytes();

    // 手动遍历全部节点。
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        match mode {
            Mode::Reference => {
                if node.kind().ends_with("identifier") && text(&node, bytes) == symbol {
                    push(out, path, &node, SymbolKind::Reference, &lines);
                }
            }
            Mode::Caller => {
                if spec.call_kinds.contains(&node.kind()) {
                    if let Some(callee) = node.child_by_field_name(spec.call_fn_field) {
                        if last_ident(&callee, bytes).as_deref() == Some(symbol) {
                            push(out, path, &node, SymbolKind::Reference, &lines);
                        }
                    }
                }
            }
            Mode::Definition => {
                if spec.def_kinds.contains(&node.kind()) {
                    if let Some(name) = def_name(&node, bytes) {
                        if name == symbol {
                            let kind = if spec.type_kinds.contains(&node.kind()) {
                                SymbolKind::Type
                            } else if spec.var_kinds.contains(&node.kind()) {
                                SymbolKind::Variable
                            } else {
                                SymbolKind::Function
                            };
                            push(out, path, &node, kind, &lines);
                        }
                    }
                }
            }
        }
        let mut i = node.child_count();
        while i > 0 {
            i -= 1;
            if let Some(c) = node.child(i as u32) {
                stack.push(c);
            }
        }
    }
}

fn push(out: &mut Vec<SymbolLoc>, path: &str, node: &Node, kind: SymbolKind, lines: &[&str]) {
    let pos = node.start_position();
    let snippet = lines
        .get(pos.row)
        .map(|l| l.trim().to_string())
        .unwrap_or_default();
    out.push(SymbolLoc {
        path: path.to_string(),
        line: pos.row as u32 + 1,
        col: pos.column as u32 + 1,
        kind,
        snippet,
    });
}

fn text<'a>(node: &Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

/// 子树里最后一个 *identifier 叶子的文本（处理 a.b.c → c，ns::f → f）。
fn last_ident(node: &Node, bytes: &[u8]) -> Option<String> {
    let mut found: Option<String> = None;
    let mut stack = vec![*node];
    let mut ordered: Vec<(usize, usize, String)> = Vec::new();
    while let Some(n) = stack.pop() {
        if n.child_count() == 0 && n.kind().ends_with("identifier") {
            let p = n.start_position();
            ordered.push((p.row, p.column, text(&n, bytes).to_string()));
        }
        for i in 0..n.child_count() {
            if let Some(c) = n.child(i as u32) {
                stack.push(c);
            }
        }
    }
    ordered.sort_by_key(|(r, c, _)| (*r, *c));
    if let Some((_, _, s)) = ordered.last() {
        found = Some(s.clone());
    }
    found
}

/// 定义节点的名字。
fn def_name(node: &Node, bytes: &[u8]) -> Option<String> {
    // rust/python/go/js 等：有 "name" 字段。
    if let Some(n) = node.child_by_field_name("name") {
        // 限定名/析构名取最后一个 identifier 分量。
        return last_ident(&n, bytes).or_else(|| Some(text(&n, bytes).to_string()));
    }
    // c/c++：沿 declarator 链下钻到名字。
    let mut cur = node.child_by_field_name("declarator")?;
    for _ in 0..8 {
        let k = cur.kind();
        if k == "identifier" || k == "field_identifier" {
            return Some(text(&cur, bytes).to_string());
        }
        if k == "destructor_name" || k == "operator_name" || k == "qualified_identifier" {
            return last_ident(&cur, bytes);
        }
        cur = cur.child_by_field_name("declarator")?;
    }
    None
}

/// 不支持语言的按行回退（词匹配，含注释/字符串——退化但有结果）。
fn fallback_lines(path: &str, source: &str, symbol: &str, out: &mut Vec<SymbolLoc>) {
    for (i, line) in source.lines().enumerate() {
        if word_in_line(line, symbol) {
            out.push(SymbolLoc {
                path: path.to_string(),
                line: i as u32 + 1,
                col: 1,
                kind: SymbolKind::Reference,
                snippet: line.trim().to_string(),
            });
        }
    }
}

fn word_in_line(line: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = line[from..].find(word) {
        let s = from + rel;
        let e = s + word.len();
        let before_ok = s == 0 || !is_word_char(line.as_bytes()[s - 1]);
        let after_ok = e >= line.len() || !is_word_char(line.as_bytes()[e]);
        if before_ok && after_ok {
            return true;
        }
        from = e;
    }
    false
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 路径 → (tree-sitter Language, 节点配置)。不支持返回 None。
fn lang_spec(path: &str) -> Option<(tree_sitter::Language, LangSpec)> {
    match Lang::from_path(path) {
        Lang::Rust => Some((
            tree_sitter_rust::LANGUAGE.into(),
            LangSpec {
                def_kinds: &[
                    "function_item",
                    "struct_item",
                    "enum_item",
                    "trait_item",
                    "type_item",
                    "const_item",
                    "static_item",
                    "mod_item",
                    "macro_definition",
                ],
                type_kinds: &["struct_item", "enum_item", "trait_item", "type_item"],
                var_kinds: &["const_item", "static_item"],
                call_kinds: &["call_expression", "macro_invocation"],
                call_fn_field: "function",
            },
        )),
        Lang::Cpp => Some((
            tree_sitter_cpp::LANGUAGE.into(),
            LangSpec {
                def_kinds: &[
                    "function_definition",
                    "struct_specifier",
                    "class_specifier",
                    "enum_specifier",
                ],
                type_kinds: &["struct_specifier", "class_specifier", "enum_specifier"],
                var_kinds: &[],
                call_kinds: &["call_expression"],
                call_fn_field: "function",
            },
        )),
        Lang::Python => Some((
            tree_sitter_python::LANGUAGE.into(),
            LangSpec {
                def_kinds: &["function_definition", "class_definition"],
                type_kinds: &["class_definition"],
                var_kinds: &[],
                call_kinds: &["call"],
                call_fn_field: "function",
            },
        )),
        Lang::Go => Some((
            tree_sitter_go::LANGUAGE.into(),
            LangSpec {
                def_kinds: &[
                    "function_declaration",
                    "method_declaration",
                    "type_declaration",
                ],
                type_kinds: &["type_declaration"],
                var_kinds: &[],
                call_kinds: &["call_expression"],
                call_fn_field: "function",
            },
        )),
        Lang::Java => Some((
            tree_sitter_java::LANGUAGE.into(),
            LangSpec {
                def_kinds: &[
                    "method_declaration",
                    "constructor_declaration",
                    "class_declaration",
                    "interface_declaration",
                    "enum_declaration",
                    "record_declaration",
                    "annotation_type_declaration",
                ],
                type_kinds: &[
                    "class_declaration",
                    "interface_declaration",
                    "enum_declaration",
                    "record_declaration",
                    "annotation_type_declaration",
                ],
                var_kinds: &[],
                call_kinds: &["method_invocation"],
                call_fn_field: "name",
            },
        )),
        Lang::JavaScript | Lang::TypeScript => Some((
            tree_sitter_javascript::LANGUAGE.into(),
            LangSpec {
                def_kinds: &[
                    "function_declaration",
                    "class_declaration",
                    "method_definition",
                    "generator_function_declaration",
                ],
                type_kinds: &["class_declaration"],
                var_kinds: &[],
                call_kinds: &["call_expression", "new_expression"],
                call_fn_field: "function",
            },
        )),
        _ => None,
    }
}

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

    fn scan(path: &str, src: &str, sym: &str, mode: Mode) -> Vec<SymbolLoc> {
        let (lang, spec) = lang_spec(path).expect("supported lang");
        let mut out = Vec::new();
        scan_ast(path, src, lang, &spec, sym, mode, &mut out);
        out
    }

    const RUST: &str = "// login is great\nfn login(id: u32) {\n    let s = \"login string\";\n    audit(id);\n    login(id);\n}\nstruct User {}\n";

    #[test]
    fn rust_definition_precise() {
        let d = scan("a.rs", RUST, "login", Mode::Definition);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 2);
        assert_eq!(d[0].kind, SymbolKind::Function);
    }

    #[test]
    fn rust_callers_skip_comment_and_string() {
        // "login" 出现在注释(1)、定义(2)、字符串(3)、调用(5)。caller 只应命中第 5 行。
        let c = scan("a.rs", RUST, "login", Mode::Caller);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].line, 5);
    }

    #[test]
    fn rust_references_skip_comment_and_string() {
        // 引用应命中定义(2)与调用(5) 的 identifier，但不命中注释(1)/字符串(3)。
        let r = scan("a.rs", RUST, "login", Mode::Reference);
        let lines: Vec<u32> = r.iter().map(|l| l.line).collect();
        assert!(lines.contains(&2));
        assert!(lines.contains(&5));
        assert!(!lines.contains(&1), "不应命中注释");
        assert!(!lines.contains(&3), "不应命中字符串");
    }

    #[test]
    fn rust_struct_is_type() {
        let d = scan("a.rs", RUST, "User", Mode::Definition);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, SymbolKind::Type);
    }

    #[test]
    fn cpp_destructor_and_call() {
        let src =
            "struct MemPool {\n  ~MemPool();\n};\nMemPool::~MemPool() {\n  releasePool(id_);\n}\n";
        let callers = scan("a.cpp", src, "releasePool", Mode::Caller);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].line, 5);
    }

    #[test]
    fn python_def_and_call_skip_string() {
        let src = "def greet(n):\n    msg = \"greet please\"\n    return greet(n)\n";
        let defs = scan("a.py", src, "greet", Mode::Definition);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].line, 1);
        let callers = scan("a.py", src, "greet", Mode::Caller);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].line, 3);
    }

    // "login" 出现在注释(1)、方法定义(3)、字符串(4)、调用(6)。
    const JAVA: &str = "// login is great\nclass User {\n    void login(int id) {\n        String s = \"login string\";\n        audit(id);\n        login(id);\n    }\n}\n";

    #[test]
    fn java_definition_precise() {
        let d = scan("a.java", JAVA, "login", Mode::Definition);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 3);
        assert_eq!(d[0].kind, SymbolKind::Function);
    }

    #[test]
    fn java_class_is_type() {
        let d = scan("a.java", JAVA, "User", Mode::Definition);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, SymbolKind::Type);
    }

    #[test]
    fn java_callers_skip_comment_and_string() {
        let c = scan("a.java", JAVA, "login", Mode::Caller);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].line, 6);
    }

    #[test]
    fn java_references_skip_comment_and_string() {
        let r = scan("a.java", JAVA, "login", Mode::Reference);
        let lines: Vec<u32> = r.iter().map(|l| l.line).collect();
        assert!(lines.contains(&3), "命中定义名");
        assert!(lines.contains(&6), "命中调用");
        assert!(!lines.contains(&1), "不应命中注释");
        assert!(!lines.contains(&4), "不应命中字符串");
    }
}
