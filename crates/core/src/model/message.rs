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
