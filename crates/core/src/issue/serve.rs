//! 最小 Webhook HTTP 服务：验签 → 入队 → 202（不在请求内跑模型）。

use super::queue::EventQueue;
use super::webhook::{
    parse_github_event, parse_gitlab_event, verify_github_signature, verify_gitlab_token,
};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct ServeConfig {
    pub listen: String,
    pub webhook_secret: String,
    pub queue_path: PathBuf,
    pub bot_logins: Vec<String>,
}

/// 启动 HTTP 服务直到取消。返回接受的连接数（测试可用 max_accepts 限制）。
pub async fn run_webhook_server(cfg: ServeConfig, max_accepts: Option<u64>) -> Result<()> {
    let queue = Arc::new(Mutex::new(EventQueue::open(&cfg.queue_path)?));
    let listener = TcpListener::bind(&cfg.listen)
        .await
        .with_context(|| format!("bind {}", cfg.listen))?;
    eprintln!("reviewgate serve listening on http://{}", cfg.listen);
    let mut n = 0u64;
    loop {
        if let Some(max) = max_accepts {
            if n >= max {
                break;
            }
        }
        let (mut socket, _) = listener.accept().await?;
        n += 1;
        let cfg = cfg.clone();
        let queue = queue.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(&mut socket, &cfg, &queue).await {
                eprintln!("webhook connection error: {e:#}");
            }
        });
    }
    Ok(())
}

const MAX_WEBHOOK_BODY: usize = 1024 * 1024;

async fn handle_connection(
    socket: &mut tokio::net::TcpStream,
    cfg: &ServeConfig,
    queue: &Arc<Mutex<EventQueue>>,
) -> Result<()> {
    let (headers, body) = match read_http_request(socket).await {
        Ok(v) => v,
        Err(e) if format!("{e:#}").starts_with("payload_too_large") => {
            eprintln!("webhook 413: {e:#}");
            write_response(socket, 413, "text/plain", b"payload too large").await?;
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let method = headers
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("");
    let path = headers
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("/");

    if method == "GET" && (path == "/health" || path == "/healthz") {
        write_response(socket, 200, "text/plain", b"ok").await?;
        return Ok(());
    }

    if method != "POST" || !path.starts_with("/webhook") {
        write_response(socket, 404, "text/plain", b"not found").await?;
        return Ok(());
    }

    let get_header = |name: &str| -> Option<String> {
        for line in headers.lines().skip(1) {
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case(name) {
                    return Some(v.trim().to_string());
                }
            }
        }
        None
    };

    let body_str = String::from_utf8_lossy(&body).to_string();

    // GitHub path: /webhook or /webhook/github
    let is_gitlab = path.contains("gitlab")
        || get_header("X-Gitlab-Event").is_some()
        || get_header("X-Gitlab-Token").is_some();

    let parsed = if is_gitlab {
        if let Err(e) =
            verify_gitlab_token(&cfg.webhook_secret, get_header("X-Gitlab-Token").as_deref())
        {
            write_response(socket, 401, "text/plain", e.to_string().as_bytes()).await?;
            return Ok(());
        }
        let delivery = get_header("X-Gitlab-Event-UUID").unwrap_or_default();
        let bots: Vec<&str> = cfg.bot_logins.iter().map(|s| s.as_str()).collect();
        parse_gitlab_event(&body_str, &delivery, &bots)?
    } else {
        let sig = get_header("X-Hub-Signature-256").unwrap_or_default();
        if let Err(e) = verify_github_signature(&cfg.webhook_secret, &body, &sig) {
            write_response(socket, 401, "text/plain", e.to_string().as_bytes()).await?;
            return Ok(());
        }
        let delivery = get_header("X-GitHub-Delivery").unwrap_or_default();
        let event = get_header("X-GitHub-Event").unwrap_or_else(|| "unknown".into());
        let bots: Vec<&str> = cfg.bot_logins.iter().map(|s| s.as_str()).collect();
        parse_github_event(&event, &delivery, &body_str, &bots)?
    };

    if parsed.is_bot_loop {
        write_response(
            socket,
            202,
            "application/json",
            br#"{"status":"ignored_bot"}"#,
        )
        .await?;
        return Ok(());
    }

    let inserted = {
        let q = queue.lock().await;
        q.enqueue(
            &parsed.delivery_id,
            &parsed.event_type,
            &parsed.action,
            &parsed.repo_id,
            parsed.issue_number,
            &body_str,
        )?
    };

    let resp = serde_json::json!({
        "status": if inserted { "queued" } else { "duplicate" },
        "delivery_id": parsed.delivery_id,
        "needs_full_review": parsed.needs_full_review,
    });
    write_response(socket, 202, "application/json", resp.to_string().as_bytes()).await?;
    Ok(())
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Ok((headers, body)) = split_http(&buf) {
            let method = headers
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("");
            if method == "GET" {
                return Ok((headers, Vec::new()));
            }
            match content_length(&headers) {
                Some(len) if len > MAX_WEBHOOK_BODY => {
                    anyhow::bail!("payload_too_large:{len}");
                }
                Some(len) if body.len() >= len => {
                    return Ok((headers, body[..len].to_vec()));
                }
                None => anyhow::bail!("payload_too_large:missing_content_length"),
                _ => {}
            }
        }
        if buf.len() > MAX_WEBHOOK_BODY + 16 * 1024 {
            anyhow::bail!("payload_too_large:{}", buf.len());
        }
    }
    let (headers, body) = split_http(&buf)?;
    if let Some(len) = content_length(&headers) {
        if len > MAX_WEBHOOK_BODY {
            anyhow::bail!("payload_too_large:{len}");
        }
        if body.len() >= len {
            return Ok((headers, body[..len].to_vec()));
        }
    }
    anyhow::bail!("payload_too_large:incomplete")
}

fn content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        if let Some(v) = line
            .split_once(':')
            .filter(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
            .map(|(_, v)| v.trim())
        {
            return v.parse().ok();
        }
    }
    None
}

fn split_http(raw: &[u8]) -> Result<(String, &[u8])> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("invalid http request")?;
    let headers = String::from_utf8_lossy(&raw[..sep]).to_string();
    let body = &raw[sep + 4..];
    // Content-Length truncate if partial
    let mut content_len = body.len();
    for line in headers.lines() {
        if let Some(v) = line
            .split_once(':')
            .filter(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
            .map(|(_, v)| v.trim())
        {
            if let Ok(n) = v.parse::<usize>() {
                content_len = n.min(body.len());
            }
        }
    }
    Ok((headers, &body[..content_len]))
}

async fn write_response(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    ctype: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.flush().await?;
    Ok(())
}

/// Worker：从队列 claim 并回调处理函数。
pub async fn drain_queue_once<F, Fut>(queue: &EventQueue, mut handler: F) -> Result<usize>
where
    F: FnMut(super::queue::WebhookDelivery) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut n = 0usize;
    while let Some(d) = queue.claim_next()? {
        match handler(d.clone()).await {
            Ok(()) => {
                queue.mark_completed(&d.delivery_id)?;
                n += 1;
            }
            Err(e) => {
                let retry = d.attempts < 5;
                queue.mark_failed(&d.delivery_id, &format!("{e:#}"), retry)?;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::time::Duration;

    #[tokio::test]
    async fn serve_accepts_signed_webhook() {
        let dir = std::env::temp_dir().join(format!(
            "rg-webhook-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let qpath = dir.join("q.db");
        let secret = "testsecret";
        let cfg = ServeConfig {
            listen: "127.0.0.1:0".into(),
            webhook_secret: secret.into(),
            queue_path: qpath.clone(),
            bot_logins: vec!["reviewgate[bot]".into()],
        };
        // bind ephemeral: fix by binding first
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let mut cfg2 = cfg.clone();
        cfg2.listen = addr.to_string();
        let server = tokio::spawn(async move {
            run_webhook_server(cfg2, Some(1)).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let body = br#"{"action":"opened","issue":{"number":9},"repository":{"full_name":"a/b"},"sender":{"login":"u","type":"User"}}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        let req = format!(
            "POST /webhook HTTP/1.1\r\nHost: localhost\r\nX-GitHub-Event: issues\r\nX-GitHub-Delivery: del-test-1\r\nX-Hub-Signature-256: {sig}\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(req.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).await.unwrap();
        let resp_s = String::from_utf8_lossy(&resp);
        assert!(resp_s.contains("202"), "{resp_s}");
        assert!(
            resp_s.contains("queued") || resp_s.contains("duplicate") || resp_s.contains("status"),
            "{resp_s}"
        );
        let _ = server.await;

        let q = EventQueue::open(&qpath).unwrap();
        // pending or processing/completed depending on timing — at least one row
        let pending = q.count_by_status("pending").unwrap();
        let processing = q.count_by_status("processing").unwrap();
        let completed = q.count_by_status("completed").unwrap();
        assert!(
            pending + processing + completed >= 1,
            "expected delivery enqueued"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
