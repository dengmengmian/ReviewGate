//! 上下文检索工具：find_definition / find_callers / find_references。
//!
//! 这三个工具对 Agent 的签名固定，内部委托给 `ctx.index`（CodeIndex）。
//! v0 背后是 GrepIndex，v1 换 TreeSitterIndex 时本文件无需改动。

use super::{Tool, ToolContext};
use crate::index::{Lang, SymbolLoc};
use crate::model::ToolDef;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

fn symbol_arg<'a>(input: &'a Value, tool: &str) -> Result<&'a str> {
    input
        .get("symbol")
        .and_then(|v| v.as_str())
        .with_context(|| format!("{tool} missing symbol"))
}

fn lang_arg(input: &Value) -> Option<Lang> {
    input.get("lang").and_then(|v| v.as_str()).map(|s| match s {
        "rust" => Lang::Rust,
        "go" => Lang::Go,
        "typescript" | "ts" => Lang::TypeScript,
        "javascript" | "js" => Lang::JavaScript,
        "python" | "py" => Lang::Python,
        "java" => Lang::Java,
        "cpp" | "c" => Lang::Cpp,
        _ => Lang::Other,
    })
}

fn format_locs(locs: &[SymbolLoc]) -> String {
    if locs.is_empty() {
        return "(no results)".into();
    }
    locs.iter()
        .map(|l| format!("{}:{} [{}] {}", l.path, l.line, l.kind.as_str(), l.snippet))
        .collect::<Vec<_>>()
        .join("\n")
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "symbol": { "type": "string", "description": "Symbol name (identifier)" },
            "lang": { "type": "string", "description": "Optional language hint, such as rust/go/ts/py/java" }
        },
        "required": ["symbol"]
    })
}

/// 找符号定义。
pub struct FindDefinition;

#[async_trait]
impl Tool for FindDefinition {
    fn name(&self) -> &str {
        "find_definition"
    }
    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "Find the definition location of a symbol: function, type, or variable."
                .into(),
            input_schema: schema(),
        }
    }
    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String> {
        let symbol = symbol_arg(input, "find_definition")?;
        let locs = ctx.index.find_definition(symbol, lang_arg(input)).await?;
        Ok(format_locs(&locs))
    }
}

/// 找调用点。
pub struct FindCallers;

#[async_trait]
impl Tool for FindCallers {
    fn name(&self) -> &str {
        "find_callers"
    }
    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "Find call sites of a function or method to assess hot paths and impact."
                .into(),
            input_schema: schema(),
        }
    }
    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String> {
        let symbol = symbol_arg(input, "find_callers")?;
        let locs = ctx.index.find_callers(symbol, lang_arg(input)).await?;
        Ok(format_locs(&locs))
    }
}

/// 找全部引用。
pub struct FindReferences;

#[async_trait]
impl Tool for FindReferences {
    fn name(&self) -> &str {
        "find_references"
    }
    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "Find all reference locations for a symbol.".into(),
            input_schema: schema(),
        }
    }
    async fn call(&self, input: &Value, ctx: &ToolContext) -> Result<String> {
        let symbol = symbol_arg(input, "find_references")?;
        let locs = ctx.index.find_references(symbol, lang_arg(input)).await?;
        Ok(format_locs(&locs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Diff;
    use crate::index::{CodeIndex, SymbolKind};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct FakeIndex {
        defs: Vec<SymbolLoc>,
        callers: Vec<SymbolLoc>,
        refs: Vec<SymbolLoc>,
    }

    #[async_trait]
    impl CodeIndex for FakeIndex {
        async fn find_definition(
            &self,
            _symbol: &str,
            _lang: Option<Lang>,
        ) -> Result<Vec<SymbolLoc>> {
            Ok(self.defs.clone())
        }
        async fn find_callers(&self, _symbol: &str, _lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
            Ok(self.callers.clone())
        }
        async fn find_references(
            &self,
            _symbol: &str,
            _lang: Option<Lang>,
        ) -> Result<Vec<SymbolLoc>> {
            Ok(self.refs.clone())
        }
    }

    fn ctx_with_index(index: Arc<dyn CodeIndex>) -> ToolContext {
        ToolContext::new(Arc::new(Diff::default()), ".", None, index)
    }

    fn sample_loc() -> SymbolLoc {
        SymbolLoc {
            path: "src/a.rs".into(),
            line: 12,
            col: 4,
            kind: SymbolKind::Function,
            snippet: "fn foo() {}".into(),
        }
    }

    #[tokio::test]
    async fn find_definition_call_formats_result() {
        let ctx = ctx_with_index(Arc::new(FakeIndex {
            defs: vec![sample_loc()],
            callers: vec![],
            refs: vec![],
        }));
        let out = FindDefinition
            .call(&json!({"symbol": "foo", "lang": "rust"}), &ctx)
            .await
            .unwrap();
        assert!(out.contains("src/a.rs:12 [function] fn foo() {}"));
    }

    #[tokio::test]
    async fn find_callers_call_formats_result() {
        let mut loc = sample_loc();
        loc.kind = SymbolKind::Reference;
        loc.snippet = "foo();".into();
        let ctx = ctx_with_index(Arc::new(FakeIndex {
            defs: vec![],
            callers: vec![loc],
            refs: vec![],
        }));
        let out = FindCallers
            .call(&json!({"symbol": "foo"}), &ctx)
            .await
            .unwrap();
        assert!(out.contains("src/a.rs:12 [reference] foo();"));
    }

    #[tokio::test]
    async fn find_references_call_no_results() {
        let ctx = ctx_with_index(Arc::new(FakeIndex {
            defs: vec![],
            callers: vec![],
            refs: vec![],
        }));
        let out = FindReferences
            .call(&json!({"symbol": "foo"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out, "(no results)");
    }

    #[tokio::test]
    async fn index_tool_missing_symbol_errors() {
        let ctx = ctx_with_index(Arc::new(FakeIndex {
            defs: vec![],
            callers: vec![],
            refs: vec![],
        }));
        let err = FindDefinition
            .call(&json!({"lang": "rust"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("find_definition missing symbol"));
    }

    #[test]
    fn symbol_arg_extracts_or_errors() {
        assert_eq!(symbol_arg(&json!({"symbol": "foo"}), "t").unwrap(), "foo");
        // 缺 symbol → 报错（带工具名）。
        let err = symbol_arg(&json!({}), "find_callers").unwrap_err();
        assert!(err.to_string().contains("find_callers"));
    }

    #[test]
    fn lang_arg_maps_known_and_aliases() {
        assert_eq!(lang_arg(&json!({"lang": "rust"})), Some(Lang::Rust));
        assert_eq!(lang_arg(&json!({"lang": "ts"})), Some(Lang::TypeScript));
        assert_eq!(lang_arg(&json!({"lang": "py"})), Some(Lang::Python));
        assert_eq!(lang_arg(&json!({"lang": "c"})), Some(Lang::Cpp));
        // 未知语言 → Other（不报错）。
        assert_eq!(lang_arg(&json!({"lang": "cobol"})), Some(Lang::Other));
        // 没给 lang → None（让 index 自行推断）。
        assert_eq!(lang_arg(&json!({})), None);
    }

    #[test]
    fn symbol_arg_error_includes_tool_name() {
        let err = symbol_arg(&json!({"lang": "rust"}), "find_definition").unwrap_err();
        assert!(err.to_string().contains("find_definition"));
    }

    #[test]
    fn format_locs_empty_and_nonempty() {
        assert_eq!(format_locs(&[]), "(no results)");
        let locs = vec![
            SymbolLoc {
                path: "src/a.rs".into(),
                line: 12,
                col: 4,
                kind: SymbolKind::Function,
                snippet: "fn foo() {}".into(),
            },
            SymbolLoc {
                path: "src/b.rs".into(),
                line: 3,
                col: 1,
                kind: SymbolKind::Reference,
                snippet: "foo();".into(),
            },
        ];
        let out = format_locs(&locs);
        assert_eq!(
            out,
            "src/a.rs:12 [function] fn foo() {}\nsrc/b.rs:3 [reference] foo();"
        );
    }

    #[test]
    fn symbol_arg_rejects_non_string_symbol() {
        let err = symbol_arg(&json!({"symbol": 123}), "find_definition").unwrap_err();
        assert!(err.to_string().contains("find_definition missing symbol"));
    }

    #[test]
    fn format_locs_with_empty_snippet() {
        let locs = vec![SymbolLoc {
            path: "src/a.rs".into(),
            line: 1,
            col: 1,
            kind: SymbolKind::Other,
            snippet: "".into(),
        }];
        assert_eq!(format_locs(&locs), "src/a.rs:1 [other] ");
    }
}
