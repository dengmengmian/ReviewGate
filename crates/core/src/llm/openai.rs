//! OpenAI 兼容客户端（`/chat/completions`）。
//!
//! 覆盖 DeepSeek / Kimi / GLM / 通义 等所有 OpenAI 兼容端点。负责在 ReviewGate
//! 内部消息模型与线上 JSON 之间双向转换。

use super::LlmClient;
use crate::config::ProviderConfig;
use crate::model::{ContentBlock, LlmResponse, Message, Role, StopReason, ToolDef, ToolUse, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct OpenAiClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        let http = super::http::build_http_client()?;
        let endpoint = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
        Ok(Self {
            http,
            endpoint,
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        })
    }

    /// 把内部消息（含系统提示）转换为线上 messages 数组。
    fn to_wire_messages(&self, system: &str, messages: &[Message]) -> Vec<Value> {
        let mut wire = Vec::new();
        if !system.is_empty() {
            wire.push(json!({ "role": "system", "content": system }));
        }
        for m in messages {
            match m.role {
                Role::System => {
                    wire.push(json!({ "role": "system", "content": m.text() }));
                }
                Role::User => {
                    wire.push(json!({ "role": "user", "content": m.text() }));
                }
                Role::Assistant => {
                    let mut text = String::new();
                    let mut tool_calls = Vec::new();
                    for b in &m.content {
                        match b {
                            ContentBlock::Text { text: t } => text.push_str(t),
                            ContentBlock::ToolUse(tu) => {
                                tool_calls.push(json!({
                                    "id": tu.id,
                                    "type": "function",
                                    "function": {
                                        "name": tu.name,
                                        "arguments": tu.input.to_string(),
                                    }
                                }));
                            }
                            ContentBlock::ToolResult(_) => {}
                        }
                    }
                    let mut msg = json!({ "role": "assistant" });
                    if tool_calls.is_empty() {
                        msg["content"] = json!(text);
                    } else {
                        // content 可为 null（仅工具调用）。
                        msg["content"] = if text.is_empty() {
                            Value::Null
                        } else {
                            json!(text)
                        };
                        msg["tool_calls"] = json!(tool_calls);
                    }
                    wire.push(msg);
                }
                Role::Tool => {
                    // 每个工具结果是一条独立的 tool 消息。
                    for b in &m.content {
                        if let ContentBlock::ToolResult(tr) = b {
                            wire.push(json!({
                                "role": "tool",
                                "tool_call_id": tr.tool_use_id,
                                "content": tr.content,
                            }));
                        }
                    }
                }
            }
        }
        wire
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse> {
        let mut body = json!({
            "model": self.model,
            "messages": self.to_wire_messages(system, messages),
        });
        if !tools.is_empty() {
            let wire_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(wire_tools);
            body["tool_choice"] = json!("auto");
        }

        let headers = [("Authorization", format!("Bearer {}", self.api_key))];
        let text =
            super::http::post_json_with_retry(&self.http, &self.endpoint, &headers, &body).await?;

        let parsed: ChatResponse = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse LLM response: {text}"))?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .context("LLM response has no choices")?;

        let mut content = Vec::new();
        if let Some(t) = choice.message.content {
            if !t.is_empty() {
                content.push(ContentBlock::text(t));
            }
        }
        for tc in choice.message.tool_calls.unwrap_or_default() {
            let input: Value = serde_json::from_str(&tc.function.arguments)
                .unwrap_or(Value::String(tc.function.arguments.clone()));
            content.push(ContentBlock::ToolUse(ToolUse {
                id: tc.id,
                name: tc.function.name,
                input,
            }));
        }

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some(other) => StopReason::Other(other.to_string()),
            None => StopReason::EndTurn,
        };

        let usage = parsed
            .usage
            .map(|u| {
                // prompt_tokens 含缓存命中；拆出缓存读取部分（DeepSeek/GLM 的 prompt_cache_hit_tokens）。
                let cache_read = u.prompt_cache_hit_tokens.min(u.prompt_tokens);
                Usage {
                    input_tokens: u.prompt_tokens - cache_read,
                    output_tokens: u.completion_tokens,
                    cache_read_input_tokens: cache_read,
                    cache_creation_input_tokens: 0,
                }
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

// ---------- 线上响应结构 ----------

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: WireMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Deserialize)]
struct WireToolCall {
    id: String,
    function: WireFunction,
}

#[derive(Deserialize)]
struct WireFunction {
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    /// 提示缓存命中 token（DeepSeek/GLM 等 OpenAI 兼容端点提供；缺省 0）。
    #[serde(default)]
    prompt_cache_hit_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;
    use crate::model::{Message, ToolResult, ToolUse};

    fn client() -> OpenAiClient {
        OpenAiClient::new(&ProviderConfig {
            protocol: Protocol::Openai,
            base_url: "https://x/v1".into(),
            api_key: "k".into(),
            model: "m".into(),
            max_input_tokens: None,
        })
        .unwrap()
    }

    #[test]
    fn wire_messages_map_roles_and_tool_calls() {
        let c = client();
        let msgs = vec![
            Message::user("审查"),
            Message::assistant(vec![ContentBlock::ToolUse(ToolUse {
                id: "t1".into(),
                name: "read_file".into(),
                input: json!({"path": "a.rs"}),
            })]),
            Message::tool_results(vec![ToolResult {
                tool_use_id: "t1".into(),
                content: "body".into(),
                is_error: false,
            }]),
        ];
        let w = c.to_wire_messages("你是审查助手", &msgs);
        // system 在最前。
        assert_eq!(w[0]["role"], "system");
        assert_eq!(w[1]["role"], "user");
        // assistant 仅工具调用 → content 为 null，tool_calls 有值。
        assert_eq!(w[2]["role"], "assistant");
        assert!(w[2]["content"].is_null());
        assert_eq!(w[2]["tool_calls"][0]["function"]["name"], "read_file");
        // arguments 是 JSON 字符串。
        assert!(w[2]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap()
            .contains("a.rs"));
        // 工具结果 → 独立的 role=tool 消息，带 tool_call_id。
        assert_eq!(w[3]["role"], "tool");
        assert_eq!(w[3]["tool_call_id"], "t1");
        assert_eq!(w[3]["content"], "body");
    }

    #[test]
    fn empty_system_omitted() {
        let c = client();
        let w = c.to_wire_messages("", &[Message::user("hi")]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0]["role"], "user");
    }

    #[test]
    fn parses_response_with_tool_call_and_cache_usage() {
        let body = r#"{
            "choices": [{"message": {"content": null, "tool_calls": [
                {"id": "c1", "function": {"name": "task_done", "arguments": "{}"}}
            ]}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 50, "prompt_cache_hit_tokens": 800}
        }"#;
        let parsed: ChatResponse = serde_json::from_str(body).unwrap();
        let u = parsed.usage.unwrap();
        // 缓存读取从 prompt_tokens 拆出。
        let cache_read = u.prompt_cache_hit_tokens.min(u.prompt_tokens);
        assert_eq!(cache_read, 800);
        assert_eq!(u.prompt_tokens - cache_read, 200);
        // finish_reason → ToolUse。
        let choice = parsed.choices.into_iter().next().unwrap();
        assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            choice.message.tool_calls.unwrap()[0].function.name,
            "task_done"
        );
    }
}
