//! 确定性上下文预取：round 1 之前，由 harness 对改动符号本地跑 `find_callers`，
//! 把结果作为**数据块**注入 unit prompt——省掉 Agent 前几轮最贵的 LLM 取数往返。
//!
//! 质量中性设计：只增加信息，不减少任何能力（工具照旧可用）；
//! 有严格上限防注意力稀释；六个维度共享同一块，prompt cache 摊薄 token 成本。

use crate::diff::{Diff, LineKind};
use crate::index::{CodeIndex, Lang};
use std::collections::BTreeSet;

/// 预取的符号数上限。
const MAX_SYMBOLS: usize = 10;
/// 每个符号列出的调用点上限。
const MAX_LOCS_PER_SYMBOL: usize = 6;

/// 从单元内的 hunk 抽取改动符号（段头外围符号 + 增删行上的定义关键字），带语言提示。
fn extract_changed_symbols(diff: &Diff, file_indices: &[usize]) -> Vec<(String, Lang)> {
    let mut set: BTreeSet<(String, String)> = BTreeSet::new(); // (symbol, path) 去重
    for &fi in file_indices {
        let Some(f) = diff.files.get(fi) else {
            continue;
        };
        let Some(path) = f.new_path.as_deref().or(f.old_path.as_deref()) else {
            continue;
        };
        if low_signal_file(path) {
            continue;
        }
        for h in &f.hunks {
            for sym in symbols_from_section(&h.section) {
                set.insert((sym, path.to_string()));
            }
            for l in &h.lines {
                if matches!(l.kind, LineKind::Added | LineKind::Deleted) {
                    for sym in def_symbols(&l.content) {
                        set.insert((sym, path.to_string()));
                    }
                }
            }
        }
    }
    set.into_iter()
        .take(MAX_SYMBOLS)
        .map(|(sym, path)| (sym, Lang::from_path(&path)))
        .collect()
}

/// 渲染预取块。无符号或全部查询空结果时返回空串（零退化）。
pub(super) async fn render_prefetch(
    index: &dyn CodeIndex,
    diff: &Diff,
    file_indices: &[usize],
) -> String {
    let symbols = extract_changed_symbols(diff, file_indices);
    if symbols.is_empty() {
        return String::new();
    }
    let mut body = String::new();
    for (sym, lang) in symbols {
        let Ok(locs) = index.find_callers(&sym, Some(lang)).await else {
            continue;
        };
        if locs.is_empty() {
            continue;
        }
        body.push_str(&format!("- `{sym}` call sites:\n"));
        for l in locs.iter().take(MAX_LOCS_PER_SYMBOL) {
            body.push_str(&format!("  {}:{} {}\n", l.path, l.line, l.snippet));
        }
        if locs.len() > MAX_LOCS_PER_SYMBOL {
            body.push_str(&format!(
                "  … {} more (use find_callers for the full list)\n",
                locs.len() - MAX_LOCS_PER_SYMBOL
            ));
        }
    }
    if body.is_empty() {
        return String::new();
    }
    format!(
        "## Prefetched call sites for changed symbols\n\n\
         Computed locally from the repository to save you lookup round-trips. \
         Possibly incomplete — verify with find_callers/find_references/read_file when it matters.\n\n{body}"
    )
}

/// 常见语言的「定义关键字 → 紧随其后的符号名」。
fn def_symbols(code: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "fn ",
        "struct ",
        "enum ",
        "trait ",
        "class ",
        "def ",
        "func ",
        "function ",
        "interface ",
        "type ",
    ];
    let mut out = Vec::new();
    for kw in KEYWORDS {
        if let Some(idx) = code.find(kw) {
            let rest = &code[idx + kw.len()..];
            if let Some(sym) = symbol_words(rest)
                .into_iter()
                .find(|s| looks_like_symbol(s))
            {
                out.push(sym);
            }
        }
    }
    out
}

/// 段头（`@@ ... @@ <section>`）里的候选符号：git 常把外围函数/类名放这里。
fn symbols_from_section(section: &str) -> Vec<String> {
    let defs = def_symbols(section);
    if !defs.is_empty() {
        return defs;
    }
    symbol_words(section)
        .into_iter()
        .filter(|s| looks_like_symbol(s))
        .take(2)
        .collect()
}

fn symbol_words(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(|w| w.trim_matches('_').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// 过滤太短的词与常见关键字/内建类型。
fn looks_like_symbol(s: &str) -> bool {
    if s.len() < 3 {
        return false;
    }
    !matches!(
        s,
        "pub"
            | "fn"
            | "let"
            | "mut"
            | "const"
            | "static"
            | "self"
            | "Self"
            | "Some"
            | "None"
            | "true"
            | "false"
            | "return"
            | "String"
            | "Option"
            | "Result"
            | "Vec"
            | "use"
            | "mod"
            | "impl"
            | "def"
            | "func"
            | "class"
            | "type"
            | "int"
            | "void"
            | "char"
            | "export"
    ) && s.chars().any(|c| c.is_ascii_alphabetic())
}

/// 锁文件等低信号路径。
fn low_signal_file(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    l.ends_with(".lock")
        || l.ends_with("package-lock.json")
        || l.ends_with("pnpm-lock.yaml")
        || l.ends_with("yarn.lock")
        || l.ends_with("go.sum")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line, LineKind};
    use crate::index::{SymbolKind, SymbolLoc};
    use anyhow::Result;
    use async_trait::async_trait;

    fn diff_with(path: &str, section: &str, changed: &[(LineKind, &str)]) -> Diff {
        let lines = changed
            .iter()
            .map(|(k, c)| Line {
                kind: *k,
                content: (*c).to_string(),
                old_lineno: None,
                new_lineno: Some(1),
            })
            .collect();
        Diff {
            files: vec![FileDiff {
                old_path: Some(path.into()),
                new_path: Some(path.into()),
                status: FileStatus::Modified,
                binary: false,
                hunks: vec![Hunk {
                    old_start: 1,
                    old_count: 1,
                    new_start: 1,
                    new_count: 1,
                    section: section.into(),
                    lines,
                }],
            }],
        }
    }

    /// 假索引：为任意符号返回固定的调用点列表。
    struct FakeIndex {
        locs_per_symbol: usize,
    }

    #[async_trait]
    impl CodeIndex for FakeIndex {
        async fn find_definition(&self, _s: &str, _l: Option<Lang>) -> Result<Vec<SymbolLoc>> {
            Ok(Vec::new())
        }
        async fn find_callers(&self, s: &str, _l: Option<Lang>) -> Result<Vec<SymbolLoc>> {
            Ok((0..self.locs_per_symbol)
                .map(|i| SymbolLoc {
                    path: format!("src/caller{i}.rs"),
                    line: 10 + i as u32,
                    col: 1,
                    kind: SymbolKind::Reference,
                    snippet: format!("{s}(arg{i})"),
                })
                .collect())
        }
        async fn find_references(&self, _s: &str, _l: Option<Lang>) -> Result<Vec<SymbolLoc>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn extracts_symbols_from_section_and_def_lines() {
        let d = diff_with(
            "src/auth.rs",
            "fn outer_handler(req: Request)",
            &[(LineKind::Added, "pub fn validate_token(t: &str) -> bool {")],
        );
        let syms: Vec<String> = extract_changed_symbols(&d, &[0])
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert!(syms.contains(&"outer_handler".to_string()), "{syms:?}");
        assert!(syms.contains(&"validate_token".to_string()), "{syms:?}");
    }

    #[tokio::test]
    async fn renders_call_sites_with_caps() {
        let d = diff_with(
            "src/auth.rs",
            "",
            &[(LineKind::Added, "pub fn validate_token(t: &str) -> bool {")],
        );
        // 10 个调用点 > 上限 6 → 截断并提示还有更多。
        let out = render_prefetch(
            &FakeIndex {
                locs_per_symbol: 10,
            },
            &d,
            &[0],
        )
        .await;
        assert!(out.contains("Prefetched call sites"), "{out}");
        assert!(out.contains("`validate_token` call sites"), "{out}");
        assert!(out.contains("src/caller0.rs:10"), "{out}");
        assert!(out.contains("4 more"), "截断提示：{out}");
        assert_eq!(
            out.matches("src/caller").count(),
            MAX_LOCS_PER_SYMBOL,
            "每符号至多 {MAX_LOCS_PER_SYMBOL} 条：{out}"
        );
    }

    #[tokio::test]
    async fn empty_when_no_symbols_or_no_call_sites() {
        // lockfile → 无符号 → 空。
        let d = diff_with("Cargo.lock", "", &[(LineKind::Added, "name = \"serde\"")]);
        assert_eq!(
            render_prefetch(&FakeIndex { locs_per_symbol: 5 }, &d, &[0]).await,
            ""
        );
        // 有符号但索引查不到调用点 → 空（零噪声）。
        let d2 = diff_with("a.rs", "", &[(LineKind::Added, "fn lonely_fn() {}")]);
        assert_eq!(
            render_prefetch(&FakeIndex { locs_per_symbol: 0 }, &d2, &[0]).await,
            ""
        );
    }
}
