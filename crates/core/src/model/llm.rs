//! LLM 工具定义与响应模型（协议无关）。

use super::message::{ContentBlock, ToolUse};
use serde::{Deserialize, Serialize};

/// 交给 LLM 的工具定义（function/tool schema）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// 工具名（Agent 通过它调用）。
    pub name: String,
    /// 给模型看的说明。
    pub description: String,
    /// 入参的 JSON Schema。
    pub input_schema: serde_json::Value,
}

/// 本轮停止原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 模型自然结束本轮。
    EndTurn,
    /// 模型请求调用工具，需执行后回灌再继续。
    ToolUse,
    /// 触达 max_tokens。
    MaxTokens,
    /// 其它（携带原始字符串）。
    Other(String),
}

/// token 用量。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// 未命中缓存的输入 token。
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// 命中缓存读取的输入 token（Anthropic `cache_read_input_tokens`）。
    #[serde(default)]
    pub cache_read_input_tokens: u32,
    /// 写入缓存的输入 token（Anthropic `cache_creation_input_tokens`）。
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
}

impl Usage {
    /// 累加另一次调用的用量。
    pub fn add(&mut self, o: &Usage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_input_tokens += o.cache_read_input_tokens;
        self.cache_creation_input_tokens += o.cache_creation_input_tokens;
    }

    /// 总输入 token（未命中 + 缓存读取 + 缓存写入）。
    pub fn total_input(&self) -> u32 {
        self.input_tokens + self.cache_read_input_tokens + self.cache_creation_input_tokens
    }

    /// 缓存读取占总输入的百分比（0–100）。
    pub fn cache_hit_pct(&self) -> u32 {
        let total = self.total_input();
        if total == 0 {
            0
        } else {
            (self.cache_read_input_tokens as u64 * 100 / total as u64) as u32
        }
    }

    /// 人话摘要：用于 verbose 输出。
    pub fn summary(&self) -> String {
        format!(
            "输入 {} tok（缓存命中 {}/{} = {}%）· 输出 {} tok",
            self.total_input(),
            self.cache_read_input_tokens,
            self.total_input(),
            self.cache_hit_pct(),
            self.output_tokens
        )
    }
}

/// 一次 LLM 调用的响应（已归一化为内部模型）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// 助手返回的内容块（文本 + 工具调用）。
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    #[serde(default)]
    pub usage: Usage,
}

impl LlmResponse {
    /// 抽出本轮所有工具调用。
    pub fn tool_uses(&self) -> Vec<&ToolUse> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse(tu) => Some(tu),
                _ => None,
            })
            .collect()
    }

    /// 拼接本轮所有文本块。
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
    use super::Usage;

    #[test]
    fn usage_accumulates_and_computes_cache_hit() {
        let mut acc = Usage::default();
        acc.add(&Usage {
            input_tokens: 200,
            output_tokens: 50,
            cache_read_input_tokens: 800,
            cache_creation_input_tokens: 0,
        });
        acc.add(&Usage {
            input_tokens: 0,
            output_tokens: 30,
            cache_read_input_tokens: 1000,
            cache_creation_input_tokens: 0,
        });
        // 总输入 = 200 + 1800 = 2000；缓存读取 1800 → 90%。
        assert_eq!(acc.total_input(), 2000);
        assert_eq!(acc.cache_hit_pct(), 90);
        assert_eq!(acc.output_tokens, 80);
    }

    #[test]
    fn cache_hit_pct_zero_when_empty() {
        assert_eq!(Usage::default().cache_hit_pct(), 0);
    }
}
