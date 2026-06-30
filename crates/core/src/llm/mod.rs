//! LLM 客户端：协议无关的 `LlmClient` trait + 各协议实现。

mod anthropic;
pub(crate) mod http;
mod openai;
mod tokens;

pub use anthropic::AnthropicClient;
pub use openai::OpenAiClient;
pub use tokens::estimate_tokens;

use crate::config::{Protocol, ProviderConfig};
use crate::model::{LlmResponse, Message, ToolDef};
use anyhow::Result;
use async_trait::async_trait;

/// 统一的 LLM 客户端接口。Agent 循环只依赖这个 trait。
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// 发起一次补全。`system` 为系统提示；`messages` 为对话历史；`tools` 可为空。
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<LlmResponse>;

    /// 当前模型名（用于日志）。
    fn model(&self) -> &str;
}

/// 按提供方配置构造对应客户端。
pub fn build_client(cfg: &ProviderConfig) -> Result<Box<dyn LlmClient>> {
    match cfg.protocol {
        Protocol::Openai => Ok(Box::new(OpenAiClient::new(cfg)?)),
        Protocol::Anthropic => Ok(Box::new(AnthropicClient::new(cfg)?)),
    }
}
