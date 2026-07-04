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
    use serde_json::json;

    #[test]
    fn parse_finding_defaults_clamping_and_required_fields() {
        // 完整字段。
        let full = parse_finding(
            &json!({
                "path": "a.rs",
                "message": "m",
                "line_start": 10,
                "line_end": 12,
                "existing_code": "x",
                "severity": "high",
                "confidence": 0.9
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(full.start_line, 10);
        assert_eq!(full.end_line, 12);
        assert_eq!(full.confidence, 0.9);
        assert_eq!(full.severity, Severity::High);

        // 缺 line_start → 行号置 0。
        let no_line = parse_finding(
            &json!({
                "path": "a.rs", "message": "m", "existing_code": "x", "severity": "low"
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(no_line.start_line, 0);
        assert_eq!(no_line.end_line, 0);

        // 有 line_start 但缺 line_end → end=start。
        let no_end = parse_finding(
            &json!({
                "path": "a.rs", "message": "m", "existing_code": "x",
                "severity": "med", "line_start": 5
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(no_end.start_line, 5);
        assert_eq!(no_end.end_line, 5);

        // line_end < line_start 被兜底到 start。
        let inverted = parse_finding(
            &json!({
                "path": "a.rs", "message": "m", "existing_code": "x",
                "severity": "med", "line_start": 5, "line_end": 3
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(inverted.end_line, 5);

        // 置信度越界被 clamp。
        let high_conf = parse_finding(
            &json!({
                "path": "a.rs", "message": "m", "existing_code": "x",
                "severity": "med", "confidence": 1.5
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(high_conf.confidence, 1.0);
        let low_conf = parse_finding(
            &json!({
                "path": "a.rs", "message": "m", "existing_code": "x",
                "severity": "med", "confidence": -0.1
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(low_conf.confidence, 0.0);

        // 非法 severity 回退 Med。
        let bad_sev = parse_finding(
            &json!({
                "path": "a.rs", "message": "m", "existing_code": "x",
                "severity": "critical"
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(bad_sev.severity, Severity::Med);

        // 非法 severity 回退 Med（任何缺失/非法值都走 default 分支）。
        assert!(parse_finding(
            &json!({"path": "a.rs", "message": "m", "existing_code": "x"}),
            Dimension::Logic
        )
        .is_ok());
        let missing_sev = parse_finding(
            &json!({"path": "a.rs", "message": "m", "existing_code": "x"}),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(missing_sev.severity, Severity::Med);

        // 必填字段缺失（severity 不是必填，缺失时回退 Med）。
        assert!(parse_finding(
            &json!({"message": "m", "existing_code": "x", "severity": "med"}),
            Dimension::Logic
        )
        .is_err());
        assert!(parse_finding(
            &json!({"path": "a.rs", "existing_code": "x", "severity": "med"}),
            Dimension::Logic
        )
        .is_err());
        assert!(parse_finding(
            &json!({"path": "a.rs", "message": "m", "severity": "med"}),
            Dimension::Logic
        )
        .is_err());
    }

    #[test]
    fn parse_finding_optional_fields_propagate() {
        let input = json!({
            "path": "a.rs",
            "message": "m",
            "existing_code": "x",
            "severity": "med",
            "suggestion": "use Y",
            "suggestion_code": "Y",
            "evidence": "because Z"
        });
        let f = parse_finding(&input, Dimension::Logic).unwrap();
        assert_eq!(f.suggestion, Some("use Y".into()));
        assert_eq!(f.suggestion_code, "Y");
        assert_eq!(f.evidence, "because Z");
    }

    #[test]
    fn parse_finding_start_zero_forces_end_zero() {
        let f = parse_finding(
            &json!({
                "path": "a.rs",
                "message": "m",
                "existing_code": "x",
                "severity": "med",
                "line_start": 0,
                "line_end": 5
            }),
            Dimension::Logic,
        )
        .unwrap();
        assert_eq!(f.start_line, 0);
        assert_eq!(f.end_line, 0);
    }

    #[test]
    fn report_finding_def_has_required_fields() {
        let def = report_finding_def();
        assert_eq!(def.name, "report_finding");
        let schema = def.input_schema.as_object().unwrap();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "path"));
        assert!(required.iter().any(|v| v == "message"));
        assert!(required.iter().any(|v| v == "existing_code"));
    }

    #[test]
    fn task_done_def_has_name_and_summary() {
        let def = task_done_def();
        assert_eq!(def.name, "task_done");
        let schema = def.input_schema.as_object().unwrap();
        assert!(schema.contains_key("properties"));
    }

    #[test]
    fn report_intent_finding_def_requires_status_enum() {
        let def = report_intent_finding_def();
        assert_eq!(def.name, "report_intent_finding");
        let props = def.input_schema["properties"].as_object().unwrap();
        let status = props["status"].as_object().unwrap();
        let enums: Vec<&str> = status["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(enums.contains(&"met"));
        assert!(enums.contains(&"missing"));
        assert!(enums.contains(&"breaking"));
    }

    #[test]
    fn parse_intent_finding_status_cases_and_errors() {
        use crate::model::IntentStatus;
        let base = json!({"criterion": "c", "message": "m"});
        for (s, expected) in [
            ("met", IntentStatus::Met),
            ("missing", IntentStatus::Missing),
            ("deviation", IntentStatus::Deviation),
            ("breaking", IntentStatus::Breaking),
            ("suggestion", IntentStatus::Suggestion),
        ] {
            let mut input = base.clone();
            input["status"] = json!(s);
            let f = parse_intent_finding(&input).unwrap();
            assert_eq!(f.intent_status, Some(expected), "status={s}");
        }
        let mut bad = base.clone();
        bad["status"] = json!("unknown");
        assert!(parse_intent_finding(&bad).is_err());
    }
}
