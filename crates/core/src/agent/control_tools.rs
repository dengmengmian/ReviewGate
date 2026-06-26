//! 控制工具定义 + `report_finding` 解析。
//!
//! `report_finding` / `task_done` 是 Agent 循环内部拦截处理的控制工具；
//! [`parse_finding`] 把 `report_finding` 入参构造成 [`Finding`]。与 run 循环分离，便于单独演进。

use crate::model::{Dimension, Finding, Severity, ToolDef};
use anyhow::Result;
use serde_json::{json, Value};

pub(super) fn report_finding_def() -> ToolDef {
    ToolDef {
        name: "report_finding".into(),
        description: "Report one review finding. line_start/line_end must be copied directly from the new-file line numbers shown beside the code. existing_code must be a real snippet currently present at that location and is used as an anchor for validation and fallback relocation."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path relative to the repository root" },
                "message": { "type": "string", "description": "Issue description in the requested output language" },
                "line_start": { "type": "integer", "description": "Issue start line copied from the shown new-file line number" },
                "line_end": { "type": "integer", "description": "Issue end line, inclusive; same as line_start for single-line issues" },
                "existing_code": { "type": "string", "description": "Anchor snippet: real code currently present at that location" },
                "severity": { "type": "string", "enum": ["high", "med", "low"] },
                "confidence": { "type": "number", "description": "Confidence from 0 to 1" },
                "suggestion": { "type": "string", "description": "Optional textual fix suggestion in the requested output language" },
                "suggestion_code": { "type": "string", "description": "Optional replacement code after the fix, used to show a diff" },
                "evidence": { "type": "string", "description": "Optional supporting evidence in the requested output language" }
            },
            "required": ["path", "message", "line_start", "existing_code", "severity"]
        }),
    }
}

pub(super) fn task_done_def() -> ToolDef {
    ToolDef {
        name: "task_done".into(),
        description: "Call when this dimension review is complete, even if there are no findings."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "Optional summary" }
            }
        }),
    }
}

/// 从 report_finding 入参构造 Finding。行号优先取模型报出的 line_start/line_end，
/// 缺失或非法时置 0，由重定位用 existing_code 兜底。
pub(super) fn parse_finding(input: &Value, dimension: Dimension) -> Result<Finding> {
    let get_str = |k: &str| input.get(k).and_then(|v| v.as_str());
    let path = get_str("path")
        .ok_or_else(|| anyhow::anyhow!("missing path"))?
        .to_string();
    let message = get_str("message")
        .ok_or_else(|| anyhow::anyhow!("missing message"))?
        .to_string();
    let existing_code = get_str("existing_code")
        .ok_or_else(|| anyhow::anyhow!("missing existing_code"))?
        .to_string();
    let severity = match get_str("severity") {
        Some("high") => Severity::High,
        Some("low") => Severity::Low,
        _ => Severity::Med,
    };
    let confidence = input
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f.clamp(0.0, 1.0) as f32)
        .unwrap_or(0.6);
    let suggestion = get_str("suggestion").map(|s| s.to_string());
    let suggestion_code = get_str("suggestion_code").unwrap_or("").to_string();
    let evidence = get_str("evidence").unwrap_or("").to_string();

    // 模型直接报新文件行号（取自标注）。缺失/非法则置 0，由重定位用 existing_code 兜底。
    let get_line = |k: &str| input.get(k).and_then(|v| v.as_u64()).map(|n| n as u32);
    let start_line = get_line("line_start").unwrap_or(0);
    let end_line = if start_line == 0 {
        0
    } else {
        get_line("line_end").unwrap_or(start_line).max(start_line)
    };

    Ok(Finding {
        dimension,
        confidence,
        severity,
        path,
        start_line,
        end_line,
        message,
        existing_code,
        evidence,
        suggestion,
        suggestion_code,
        reachability: crate::model::Reachability::default(),
        filtered: false,
        agreed_dimensions: 1,
    })
}
