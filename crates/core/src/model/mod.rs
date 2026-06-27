//! 核心数据模型：审查结果（Finding）、对话/工具消息、LLM 响应。

mod finding;
mod llm;
mod message;

pub use finding::{Dimension, Finding, IntentStatus, Reachability, Severity};
pub use llm::{LlmResponse, StopReason, ToolDef, Usage};
pub use message::{ContentBlock, Message, Role, ToolResult, ToolUse};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_roundtrip() {
        for d in Dimension::ALL {
            let json = serde_json::to_string(&d).unwrap();
            let back: Dimension = serde_json::from_str(&json).unwrap();
            assert_eq!(d, back);
            assert_eq!(json.trim_matches('"'), d.as_str());
        }
    }

    #[test]
    fn finding_serde_skips_none_suggestion() {
        let f = Finding {
            dimension: Dimension::Security,
            confidence: 0.91,
            severity: Severity::High,
            path: "src/auth.rs".into(),
            start_line: 12,
            end_line: 14,
            message: "SQL 注入风险".into(),
            existing_code: "format!(\"... {}\", input)".into(),
            evidence: "query 未参数化".into(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        // suggestion=None 被跳过；但 suggestion_code 始终存在（空串）。
        assert!(!json.contains("\"suggestion\":"));
        assert!(json.contains("\"suggestion_code\":\"\""));
        assert!(f.located());
    }

    #[test]
    fn message_helpers() {
        let m = Message::user("hello");
        assert_eq!(m.role, Role::User);
        assert_eq!(m.text(), "hello");

        let tr = ToolResult {
            tool_use_id: "t1".into(),
            content: "ok".into(),
            is_error: false,
        };
        let tm = Message::tool_results(vec![tr]);
        assert_eq!(tm.role, Role::Tool);
        assert_eq!(tm.content.len(), 1);
    }

    #[test]
    fn severity_orders_low_to_high() {
        assert!(Severity::Low < Severity::Med);
        assert!(Severity::Med < Severity::High);
    }
}
