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
        criterion: None,
        intent_status: None,
    })
}

/// 意图评审专用的需求锚定上报工具。与 `report_finding` 不同：以**验收标准**为锚，
/// 位置可选（缺失类发现没有可锚的行），并带 status 表达「满足/缺失/不符/破坏/建议」。
pub(super) fn report_intent_finding_def() -> ToolDef {
    ToolDef {
        name: "report_intent_finding".into(),
        description: "Report one intent/technical-review verdict, anchored to an acceptance criterion (not to a diff line). Use this instead of report_finding for intent review. Report one verdict per acceptance criterion: status=met when satisfied, or missing/deviation/breaking when not; plus optional suggestion-level concerns. file/line are optional — a 'missing' item often has no anchor."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "criterion": { "type": "string", "description": "The acceptance criterion / intent point this verdict is about (quote or paraphrase it)" },
                "status": { "type": "string", "enum": ["met", "missing", "deviation", "breaking", "suggestion"], "description": "Verdict relative to the criterion" },
                "message": { "type": "string", "description": "Explanation in the requested output language. For 'met', a one-line justification is enough." },
                "confidence": { "type": "number", "description": "Confidence from 0 to 1" },
                "file": { "type": "string", "description": "Optional most-relevant file (repo-relative)" },
                "line_start": { "type": "integer", "description": "Optional anchor line in the file" },
                "existing_code": { "type": "string", "description": "Optional anchor snippet if a concrete location applies" },
                "suggestion": { "type": "string", "description": "Optional fix/approach suggestion in the requested output language" }
            },
            "required": ["criterion", "status", "message"]
        }),
    }
}

/// 从 report_intent_finding 入参构造 Finding（dimension = Intent，需求锚定，行号/路径可选）。
pub(super) fn parse_intent_finding(input: &Value) -> Result<Finding> {
    use crate::model::IntentStatus;
    let get_str = |k: &str| input.get(k).and_then(|v| v.as_str());
    let criterion = get_str("criterion")
        .ok_or_else(|| anyhow::anyhow!("missing criterion"))?
        .to_string();
    let message = get_str("message")
        .ok_or_else(|| anyhow::anyhow!("missing message"))?
        .to_string();
    let status = match get_str("status") {
        Some("met") => IntentStatus::Met,
        Some("missing") => IntentStatus::Missing,
        Some("deviation") => IntentStatus::Deviation,
        Some("breaking") => IntentStatus::Breaking,
        Some("suggestion") => IntentStatus::Suggestion,
        other => anyhow::bail!("invalid status: {other:?}"),
    };
    // 严重度由 status 推导：缺失/破坏 = High，不符 = Med，建议/已满足 = Low。
    let severity = match status {
        IntentStatus::Missing | IntentStatus::Breaking => Severity::High,
        IntentStatus::Deviation => Severity::Med,
        IntentStatus::Suggestion | IntentStatus::Met | IntentStatus::Unknown => Severity::Low,
    };
    let confidence = input
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f.clamp(0.0, 1.0) as f32)
        .unwrap_or(0.6);
    let path = get_str("file").unwrap_or("").to_string();
    let start_line = input
        .get("line_start")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);

    Ok(Finding {
        dimension: Dimension::Intent,
        confidence,
        severity,
        path,
        start_line,
        end_line: start_line,
        message,
        existing_code: get_str("existing_code").unwrap_or("").to_string(),
        evidence: String::new(),
        suggestion: get_str("suggestion").map(|s| s.to_string()),
        suggestion_code: String::new(),
        reachability: crate::model::Reachability::default(),
        filtered: false,
        agreed_dimensions: 1,
        criterion: Some(criterion),
        intent_status: Some(status),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::IntentStatus;
    use serde_json::json;

    #[test]
    fn intent_finding_maps_status_severity_and_optional_anchor() {
        // missing → High,无 anchor（缺失类发现没有行号）
        let f = parse_intent_finding(&json!({
            "criterion": "dispatch 必须处理 URL 对象",
            "status": "missing",
            "message": "dispatchRequest 未处理 URL 对象",
            "confidence": 0.8
        }))
        .unwrap();
        assert_eq!(f.dimension, Dimension::Intent);
        assert_eq!(f.intent_status, Some(IntentStatus::Missing));
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.criterion.as_deref(), Some("dispatch 必须处理 URL 对象"));
        assert_eq!(f.start_line, 0);
        assert!(f.path.is_empty());

        // met → Low,可带可选 anchor
        let m = parse_intent_finding(&json!({
            "criterion": "c", "status": "met", "message": "ok",
            "file": "a.js", "line_start": 5
        }))
        .unwrap();
        assert_eq!(m.intent_status, Some(IntentStatus::Met));
        assert_eq!(m.severity, Severity::Low);
        assert_eq!(m.path, "a.js");
        assert_eq!(m.start_line, 5);

        // suggestion → Low；deviation → Med
        assert_eq!(
            parse_intent_finding(&json!({"criterion":"c","status":"suggestion","message":"m"}))
                .unwrap()
                .severity,
            Severity::Low
        );
        assert_eq!(
            parse_intent_finding(&json!({"criterion":"c","status":"deviation","message":"m"}))
                .unwrap()
                .severity,
            Severity::Med
        );

        // 非法/缺字段 → 报错
        assert!(
            parse_intent_finding(&json!({"criterion":"c","status":"bogus","message":"m"})).is_err()
        );
        assert!(parse_intent_finding(&json!({"status":"met","message":"m"})).is_err());
    }
}
