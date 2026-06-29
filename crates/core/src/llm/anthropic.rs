//! Anthropic 客户端（`/v1/messages`）。
//!
//! 负责内部消息模型与 Anthropic 线上 JSON 的双向转换，并对 system 与最后一个
//! 工具定义加 `cache_control: ephemeral`（prompt caching，省 token）。

use super::LlmClient;
use crate::config::ProviderConfig;
use crate::model::{ContentBlock, LlmResponse, Message, Role, StopReason, ToolDef, ToolUse, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 8192;

pub struct AnthropicClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        let http = super::http::shared_http_client()?;
        let endpoint = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
        Ok(Self {
            http,
            endpoint,
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        })
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse> {
        let mut wire_messages = to_wire_messages(messages);
        mark_shared_cache_breakpoint(&mut wire_messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "messages": wire_messages,
        });

        if !system.is_empty() {
            // 数组形式，便于挂 cache_control。
            body["system"] = json!([{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" }
            }]);
        }

        if !tools.is_empty() {
            let mut wire_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect();
            // 最后一个工具挂 cache_control，缓存整段工具定义。
            if let Some(last) = wire_tools.last_mut() {
                last["cache_control"] = json!({ "type": "ephemeral" });
            }
            body["tools"] = json!(wire_tools);
            body["tool_choice"] = json!({ "type": "auto" });
        }

        let headers = [
            ("x-api-key", self.api_key.clone()),
            ("anthropic-version", ANTHROPIC_VERSION.to_string()),
        ];
        let text =
            super::http::post_json_with_retry(&self.http, &self.endpoint, &headers, &body).await?;

        let parsed: MessagesResponse = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse LLM response: {text}"))?;

        let mut content = Vec::new();
        for block in parsed.content {
            match block {
                WireContent::Text { text } if !text.is_empty() => {
                    content.push(ContentBlock::text(text));
                }
                WireContent::Text { .. } => {}
                WireContent::ToolUse { id, name, input } => {
                    content.push(ContentBlock::ToolUse(ToolUse { id, name, input }));
                }
                WireContent::Other => {}
            }
        }

        let stop_reason = match parsed.stop_reason.as_deref() {
            Some("end_turn") | Some("stop_sequence") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some(other) => StopReason::Other(other.to_string()),
            None => StopReason::EndTurn,
        };

        let usage = parsed
            .usage
            .map(|u| Usage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_read_input_tokens: u.cache_read_input_tokens,
                cache_creation_input_tokens: u.cache_creation_input_tokens,
            })
            .unwrap_or_default();

        Ok(LlmResponse {
            content,
            stop_reason,
            usage,
        })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

/// 给首条 user 消息的第一个块挂 `cache_control`。
///
/// 这是 review 的共享大块（diff + 文件全文），跨维度、跨轮都相同，可命中缓存。
/// 低于最小缓存阈值时 Anthropic 会自动忽略（judge 等小请求零副作用）。
fn mark_shared_cache_breakpoint(wire_messages: &mut [Value]) {
    if let Some(first_block) = wire_messages
        .iter_mut()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_array_mut())
        .and_then(|blocks| blocks.first_mut())
    {
        first_block["cache_control"] = json!({ "type": "ephemeral" });
    }
}

/// 内部消息 → Anthropic messages 数组。tool 结果归入 user 角色。
fn to_wire_messages(messages: &[Message]) -> Vec<Value> {
    let mut wire = Vec::new();
    for m in messages {
        match m.role {
            Role::User | Role::System => {
                let blocks: Vec<Value> = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } if !text.is_empty() => {
                            Some(json!({ "type": "text", "text": text }))
                        }
                        _ => None,
                    })
                    .collect();
                if !blocks.is_empty() {
                    wire.push(json!({ "role": "user", "content": blocks }));
                }
            }
            Role::Assistant => {
                let blocks: Vec<Value> = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } if !text.is_empty() => {
                            Some(json!({ "type": "text", "text": text }))
                        }
                        ContentBlock::ToolUse(tu) => Some(json!({
                            "type": "tool_use",
                            "id": tu.id,
                            "name": tu.name,
                            "input": tu.input,
                        })),
                        _ => None,
                    })
                    .collect();
                if !blocks.is_empty() {
                    wire.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            Role::Tool => {
                // 工具结果作为 user 角色的 tool_result 块。
                let blocks: Vec<Value> = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolResult(tr) => Some(json!({
                            "type": "tool_result",
                            "tool_use_id": tr.tool_use_id,
                            "content": tr.content,
                            "is_error": tr.is_error,
                        })),
                        _ => None,
                    })
                    .collect();
                if !blocks.is_empty() {
                    wire.push(json!({ "role": "user", "content": blocks }));
                }
            }
        }
    }
    wire
}

// ---------- 线上响应结构 ----------

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<WireContent>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolResult;

    #[test]
    fn maps_roles_and_tool_results() {
        let messages = vec![
            Message::user("审查这段代码"),
            Message::assistant(vec![ContentBlock::ToolUse(ToolUse {
                id: "t1".into(),
                name: "read_file".into(),
                input: json!({"path": "a.rs"}),
            })]),
            Message::tool_results(vec![ToolResult {
                tool_use_id: "t1".into(),
                content: "file body".into(),
                is_error: false,
            }]),
        ];
        let wire = to_wire_messages(&messages);
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[0]["role"], "user");
        assert_eq!(wire[1]["role"], "assistant");
        assert_eq!(wire[1]["content"][0]["type"], "tool_use");
        // 工具结果归 user。
        assert_eq!(wire[2]["role"], "user");
        assert_eq!(wire[2]["content"][0]["type"], "tool_result");
        assert_eq!(wire[2]["content"][0]["tool_use_id"], "t1");
    }

    #[test]
    fn caches_first_user_block_only() {
        let mut wire = to_wire_messages(&[
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::text("共享大块：diff + 文件"),
                    ContentBlock::text("维度聚焦点"),
                ],
            },
            Message::assistant(vec![ContentBlock::text("ok")]),
        ]);
        mark_shared_cache_breakpoint(&mut wire);
        // 仅首条 user 消息的块 0 挂缓存；块 1（维度聚焦点）不挂，故跨维度可复用前缀。
        assert_eq!(wire[0]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(wire[0]["content"][1].get("cache_control").is_none());
        assert!(wire[1]["content"][0].get("cache_control").is_none());
    }
}
