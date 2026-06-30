//! LLM 客户端共享 HTTP：统一的客户端构造 + 带退避抖动的重试。
//!
//! openai/anthropic 两个客户端原各有一份几乎相同的 `post_with_retry`——抽到这里去重，
//! 并加 **±25% 抖动**避免多维度并发同时失败时的"重试羊群"。

use anyhow::{Context, Result};
use std::sync::OnceLock;
use std::time::Duration;

const MAX_ATTEMPTS: u64 = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// 进程级共享客户端：`reqwest::Client` 内部即 `Arc`，clone 共享同一连接池。
/// 多个 provider 实例/多次 GitHub 评论复用它，免去重复建池与 TLS 握手开销。
static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 取共享 HTTP 客户端（首次构造，之后复用）。连接/空闲/总超时集中在此调。
pub fn shared_http_client() -> Result<reqwest::Client> {
    if let Some(c) = SHARED_CLIENT.get() {
        return Ok(c.clone());
    }
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(30)) // 防网关 LB 关闭空闲连接
        .build()
        .context("failed to build HTTP client")?;
    // 竞态下别的线程可能先 set 成功——那就用已存在的那个，丢弃本地新建的。
    Ok(SHARED_CLIENT.get_or_init(|| client).clone())
}

/// POST JSON，对超时/连接失败/5xx/429/408 自动重试（最多 3 次，退避带抖动）；其它 4xx 不重试。
/// 限流（429）/请求超时（408）是**瞬时**错误：直接 bail 会让整个审查单元误判成 incomplete，
/// 故并入重试分支，并优先采纳服务端 `Retry-After` 给的等待时长。
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
        // Retry-After（若服务端给了）覆盖默认退避——读 body 会消费 resp，故先取出。
        let mut retry_after: Option<Duration> = None;
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                retry_after = parse_retry_after(&resp);
                let text = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    return Ok(text);
                }
                if is_retryable_status(status) {
                    last_err = Some(anyhow::anyhow!("LLM returned {status}: {text}"));
                } else {
                    anyhow::bail!("LLM returned {status}: {text}"); // 其它 4xx：不重试
                }
            }
            Err(e) => last_err = Some(anyhow::anyhow!("failed to send LLM request: {e}")),
        }
        if attempt + 1 < MAX_ATTEMPTS {
            let wait = retry_after.unwrap_or_else(|| backoff_with_jitter(attempt));
            tokio::time::sleep(wait).await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM request failed")))
}

/// 可重试的状态码：5xx（服务端故障）+ 429（限流）+ 408（请求超时）。
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
}

/// 解析 `Retry-After` 头：仅支持「秒数」形式（HTTP-date 形式较罕见，忽略走默认退避）。
/// 上限 60s——防止服务端给出夸张值把单次审查卡死。
fn parse_retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.min(60)))
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
    fn retryable_covers_5xx_429_408_only() {
        use reqwest::StatusCode;
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS)); // 429
        assert!(is_retryable_status(StatusCode::REQUEST_TIMEOUT)); // 408
                                                                   // 其它 4xx 不重试（鉴权/请求错误，重试无意义）。
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
    }

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
