//! LLM 客户端共享 HTTP：统一的客户端构造 + 带退避抖动的重试。
//!
//! openai/anthropic 两个客户端原各有一份几乎相同的 `post_with_retry`——抽到这里去重，
//! 并加 **±25% 抖动**避免多维度并发同时失败时的"重试羊群"。

use anyhow::{Context, Result};
use std::time::Duration;

const MAX_ATTEMPTS: u64 = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// 统一构造 HTTP 客户端（连接/空闲/总超时集中在此调）。
pub fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(30)) // 防网关 LB 关闭空闲连接
        .build()
        .context("构造 HTTP 客户端失败")
}

/// POST JSON，对超时/连接失败/5xx 自动重试（最多 3 次，退避带抖动）；4xx 不重试。
/// `headers` 为各 provider 自带的鉴权头（如 `Authorization`/`x-api-key`）。
pub async fn post_json_with_retry(
    http: &reqwest::Client,
    endpoint: &str,
    headers: &[(&str, String)],
    body: &serde_json::Value,
) -> Result<String> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..MAX_ATTEMPTS {
        let mut req = http.post(endpoint).json(body);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    return Ok(text);
                }
                if status.is_server_error() {
                    last_err = Some(anyhow::anyhow!("LLM 返回 {status}：{text}"));
                } else {
                    anyhow::bail!("LLM 返回 {status}：{text}"); // 4xx：不重试
                }
            }
            Err(e) => last_err = Some(anyhow::anyhow!("LLM 请求发送失败：{e}")),
        }
        if attempt + 1 < MAX_ATTEMPTS {
            tokio::time::sleep(backoff_with_jitter(attempt)).await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM 请求失败")))
}

/// 退避：基础 `2*(attempt+1)` 秒，叠加 **±25% 抖动**（去同步并发重试）。
/// 抖动源用 SystemTime 纳秒做廉价伪随机——重试去同步不需要密码学随机。
fn backoff_with_jitter(attempt: u64) -> Duration {
    let base_ms: i64 = 2000 * (attempt as i64 + 1); // 2s, 4s
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let frac = (nanos % 1000) as i64 - 500; // [-500, 499]
    let jitter_ms = base_ms * frac / 2000; // ±25%
    Duration::from_millis((base_ms + jitter_ms).max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_within_jitter_bounds() {
        for attempt in 0..2u64 {
            let base = 2000 * (attempt + 1);
            let lo = base * 3 / 4; // -25%
            let hi = base * 5 / 4; // +25%
            for _ in 0..50 {
                let ms = backoff_with_jitter(attempt).as_millis() as u64;
                assert!(
                    ms >= lo && ms <= hi,
                    "attempt {attempt}: {ms} not in [{lo},{hi}]"
                );
            }
        }
    }
}
