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
use std::time::Instant;

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

    /// 同 [`complete`]，HTTP 重试不得越过 `deadline`。默认忽略截止、走 [`complete`]。
    async fn complete_until(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
        _deadline: Option<Instant>,
    ) -> Result<LlmResponse> {
        self.complete(system, messages, tools).await
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;

    fn cfg(protocol: crate::config::Protocol) -> ProviderConfig {
        ProviderConfig {
            protocol,
            base_url: "https://api.example.com".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            max_input_tokens: None,
            price_per_mtok_input: None,
            price_per_mtok_output: None,
        }
    }

    #[test]
    fn build_client_openai() {
        let client = build_client(&cfg(crate::config::Protocol::Openai)).unwrap();
        assert_eq!(client.model(), "m");
    }

    #[test]
    fn build_client_anthropic() {
        let client = build_client(&cfg(crate::config::Protocol::Anthropic)).unwrap();
        assert_eq!(client.model(), "m");
    }
}
