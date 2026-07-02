//! 对话/工具消息模型。
//!
//! 这是 ReviewGate 内部的**协议无关**抽象。Anthropic 与 OpenAI 的线上格式不同，
//! 由各自的客户端（M1.3/M1.4）负责在这套类型与线上 JSON 之间转换。

use serde::{Deserialize, Serialize};

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// 工具执行结果（OpenAI 用 role=tool；Anthropic 用 user + tool_result 块）。
    Tool,
}

/// 内容块。一条消息由若干内容块组成。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// 纯文本。
    Text { text: String },
    /// 助手发起的工具调用。
    ToolUse(ToolUse),
    /// 工具调用的返回。
    ToolResult(ToolResult),
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
}

/// 助手发起的一次工具调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUse {
    /// 调用 id，用于把结果配回去。
    pub id: String,
    /// 工具名。
    pub name: String,
    /// 入参（JSON）。
    pub input: serde_json::Value,
}

/// 一次工具调用的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// 对应的 `ToolUse.id`。
    pub tool_use_id: String,
    /// 结果内容（纯文本/序列化后的）。
    pub content: String,
    /// 是否为错误结果。
    #[serde(default)]
    pub is_error: bool,
}

/// 一条消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// 用户文本消息。
    pub fn user(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentBlock::text(text)],
        }
    }

    /// 助手消息（通常直接来自 `LlmResponse.content`）。
    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    /// 把若干工具结果打包成一条 Tool 消息。
    pub fn tool_results(results: Vec<ToolResult>) -> Self {
        Message {
            role: Role::Tool,
            content: results.into_iter().map(ContentBlock::ToolResult).collect(),
        }
    }

    /// 取该消息里的纯文本（拼接所有 Text 块）。
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_serde_round_trip() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
        let back: Role = serde_json::from_str("\"user\"").unwrap();
        assert_eq!(back, Role::User);
    }

    #[test]
    fn content_block_text_helper() {
        let b = ContentBlock::text("hello");
        assert!(matches!(b, ContentBlock::Text { text } if text == "hello"));
    }

    #[test]
    fn message_user_and_assistant() {
        let m = Message::user("hi");
        assert_eq!(m.role, Role::User);
        assert_eq!(m.text(), "hi");

        let m = Message::assistant(vec![ContentBlock::text("a"), ContentBlock::text("b")]);
        assert_eq!(m.role, Role::Assistant);
        assert_eq!(m.text(), "ab");
    }

    #[test]
    fn message_tool_results_packs_blocks() {
        let results = vec![
            ToolResult {
                tool_use_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            },
            ToolResult {
                tool_use_id: "t2".into(),
                content: "err".into(),
                is_error: true,
            },
        ];
        let m = Message::tool_results(results);
        assert_eq!(m.role, Role::Tool);
        assert_eq!(m.content.len(), 2);
        assert!(matches!(&m.content[0], ContentBlock::ToolResult(r) if r.tool_use_id == "t1"));
    }

    #[test]
    fn message_text_ignores_nontext_blocks() {
        let m = Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::text("hello "),
                ContentBlock::ToolUse(ToolUse {
                    id: "x".into(),
                    name: "y".into(),
                    input: serde_json::json!({}),
                }),
                ContentBlock::text("world"),
            ],
        };
        assert_eq!(m.text(), "hello world");
    }
}
