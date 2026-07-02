//! `find_duplicate_functions`：确定性重复函数检测。
//!
//! 思路（业内"确定性候选 + LLM 判断"惯例）：用 tree-sitter 列出**改动文件**里的函数，
//! 把函数体规范化（折叠空白）后按相等分组，给出**精确重复**候选；是否真有维护风险
//! 交给 Agent/Judge 判断。**diff-scoped**：只扫改动文件，故每个候选必然涉及本次改动。

use super::{Tool, ToolContext};
use crate::index::list_function_bodies;
use crate::model::ToolDef;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 规范化后函数体的最小长度——低于此值视为样板/小函数，不参与（降噪）。
const MIN_BODY_CHARS: usize = 120;
/// 最多返回的重复组数。
const MAX_GROUPS: usize = 20;

/// 规范化函数体：折叠所有空白为单空格并去首尾。对缩进/格式差异不敏感。
fn normalize_body(body: &str) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 一个函数出现位置。
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
struct Loc {
    path: String,
    name: String,
    line: u32,
}

/// 把 (path, source) 列表里的函数按规范化体分组，返回有 ≥2 个成员的精确重复组。
/// 纯函数，便于单测。
fn duplicate_groups(files: &[(String, String)]) -> Vec<Vec<Loc>> {
    let mut by_body: HashMap<String, Vec<Loc>> = HashMap::new();
    // 稳定顺序：按首次出现的规范化体记录顺序。
    let mut order: Vec<String> = Vec::new();
    for (path, source) in files {
        for f in list_function_bodies(path, source) {
            let norm = normalize_body(&f.body_text);
            if norm.len() < MIN_BODY_CHARS {
                continue;
            }
            let loc = Loc {
                path: path.clone(),
                name: f.name,
                line: f.start_line,
            };
            if !by_body.contains_key(&norm) {
                order.push(norm.clone());
            }
            by_body.entry(norm).or_default().push(loc);
        }
    }
    order
        .into_iter()
        .filter_map(|k| {
            let v = by_body.remove(&k)?;
            // 同名同位置（同一函数被多次列出）不算重复；按 (path,line) 去重后仍 ≥2 才算。
            let mut uniq = v;
            uniq.dedup_by(|a, b| a.path == b.path && a.line == b.line);
            (uniq.len() >= 2).then_some(uniq)
        })
        .take(MAX_GROUPS)
        .collect()
}

/// 检测改动文件内/间的重复函数。
pub struct FindDuplicateFunctions;

#[async_trait]
impl Tool for FindDuplicateFunctions {
    fn name(&self) -> &str {
        "find_duplicate_functions"
    }

    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name().into(),
            description: "Deterministically detect duplicate functions within or across changed files, where normalized function bodies are exactly identical. \
Returns locations for each group so you can decide whether it is a real maintainability risk, such as copy-paste that was not adapted or logic that should stay consistent but may drift. \
Small functions and boilerplate are filtered automatically. No input is required."
                .into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn call(&self, _input: &Value, ctx: &ToolContext) -> Result<String> {
        // 读改动文件的新版本（diff-scoped）。
        let mut files: Vec<(String, String)> = Vec::new();
        for f in &ctx.diff.files {
            let Some(path) = f.new_path.as_deref() else {
                continue;
            };
            if f.binary {
                continue;
            }
            let content = match &ctx.new_ref {
                Some(r) => crate::diff::git::git(&["show", &format!("{r}:{path}")])
                    .await
                    .ok(),
                None => tokio::fs::read_to_string(ctx.repo_root.join(path))
                    .await
                    .ok(),
            };
            if let Some(c) = content {
                files.push((path.to_string(), c));
            }
        }

        let groups = duplicate_groups(&files);
        if groups.is_empty() {
            return Ok(
                "(no exact duplicate functions found within or across changed files)".into(),
            );
        }
        let mut out = format!(
            "Found {} duplicate function groups with identical normalized bodies:\n",
            groups.len()
        );
        for (i, g) in groups.iter().enumerate() {
            out.push_str(&format!("\n[Group {}]\n", i + 1));
            for loc in g {
                out.push_str(&format!("  - {}:{} {}()\n", loc.path, loc.line, loc.name));
            }
        }
        out.push_str("\nJudge each group before reporting: report only when variables, boundaries, or error handling were not adapted after copying, or when two locations should stay consistent and are likely to drift. Do not report pure boilerplate or test helpers.");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize_body("  a\n   b\t c "), "a b c");
    }

    #[test]
    fn detects_identical_bodies_across_files_ignoring_name_and_indent() {
        // 同一函数体（仅函数名与缩进不同）出现在两个文件 → 一组重复。
        let body = "{\n    let total = base_price * quantity + shipping_fee;\n    if total < 0 { return Err(\"negative total not allowed\"); }\n    audit_log.record(user_id, total);\n    Ok(total)\n}";
        let a = format!("fn compute_a(base_price: i64, quantity: i64) -> Result<i64> {body}");
        let b = format!("fn compute_b(base_price: i64, quantity: i64) -> Result<i64> {body}");
        let groups = duplicate_groups(&[("a.rs".into(), a), ("b.rs".into(), b)]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn small_bodies_filtered_as_boilerplate() {
        let a = "fn id(x: i32) -> i32 { x }".to_string();
        let b = "fn id2(x: i32) -> i32 { x }".to_string();
        // 函数体太短（< MIN_BODY_CHARS）→ 不算重复。
        assert!(duplicate_groups(&[("a.rs".into(), a), ("b.rs".into(), b)]).is_empty());
    }

    #[test]
    fn same_loc_deduped_within_file() {
        // 同一函数体在同一文件同一行只算一次（防御 tree-sitter 重复列出）。
        let body = "{\n    let total = base_price * quantity + shipping_fee + discount;\n    if total < 0 { return Err(\"negative total not allowed in this module\"); }\n    audit_log.record(user_id, total, created_at);\n    let rounded = (total as f64 * tax_rate).round() as i64;\n    Ok(rounded)\n}";
        let src = format!("fn a() {body}\nfn b() {body}");
        let groups = duplicate_groups(&[("a.rs".into(), src)]);
        // a 与 b 是两个不同位置，应归为一组；若去重正确则仍是 1 组 2 成员。
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn max_groups_limit() {
        let mut files = Vec::new();
        // 造 25 组不同的重复函数，每组在 2 个文件中出现；应被限制为 MAX_GROUPS=20。
        for g in 0..25 {
            let body = format!(
                "{{\n    let v{} = base_price * quantity + shipping_fee + discount;\n    let y = (v{} as f64 * tax_rate).round() as i64;\n    if y < 0 {{ return Err(\"negative total not allowed\"); }}\n    audit_log.record(user_id, y, {});\n    Ok(y)\n}}",
                g, g, g
            );
            for side in ["a", "b"] {
                files.push((
                    format!("f{}_{}.rs", g, side),
                    format!("fn f{}_{}() {body}", g, side),
                ));
            }
        }
        let groups = duplicate_groups(&files);
        assert_eq!(groups.len(), MAX_GROUPS);
    }

    #[test]
    fn distinct_bodies_not_grouped() {
        // 两个体都足够长（> 阈值），仅运算符不同 → 不应被分到同一组。
        let big = |op: &str| {
            format!(
                "fn f() {{\n    let total = base_price {op} quantity + shipping_fee;\n    if total < 0 {{ return Err(\"negative total not allowed\"); }}\n    audit_log.record(user_id, total);\n    Ok(total)\n}}"
            )
        };
        let groups = duplicate_groups(&[("a.rs".into(), big("*")), ("b.rs".into(), big("+"))]);
        assert!(groups.is_empty());
    }
}
