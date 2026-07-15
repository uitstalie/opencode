pub mod anthropic;
pub mod gemini;
pub mod openai_compat;

use reqwest::Client;
use serde_json::Value;
use std::future::Future;

/// Recursively merge `source` into `target`. Nested objects combine; scalars replace.
pub(crate) fn merge_options_into(target: &mut Value, source: &Value) {
    if let (Some(target_map), Some(source_map)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in source_map {
            match target_map.get_mut(key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    merge_options_into(existing, value);
                }
                _ => {
                    target_map.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

/// Normalize user-facing option keys to wire-format keys.
///
/// Applies:
/// - CamelCase → snake_case conversion (`promptCacheKey` → `prompt_cache_key`)
/// - Works recursively on nested objects and arrays.
///
/// Users write semantic names in config (`promptCacheKey`, `reasoningEffort`);
/// this ensures they land as provider-native wire fields after merging.
pub(crate) fn normalize_options(opts: &Value) -> Value {
    match opts {
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            for (k, v) in map {
                result.insert(camel_to_snake(k), normalize_options(v));
            }
            Value::Object(result)
        }
        Value::Array(arr) => Value::Array(
            arr.iter().map(normalize_options).collect(),
        ),
        _ => opts.clone(),
    }
}

fn camel_to_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 && !result.ends_with('_') {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Build a shared HTTP client with sane defaults for LLM API calls:
/// - TCP keep-alive every 30s (prevents NAT/firewall from dropping idle connections)
/// - HTTP/2 PING every 30s / timeout 10s (keeps H2 connections alive through proxies)
/// - 8s connect timeout (fail fast on DNS/TCP issues)
/// - 300s total timeout (LLM streaming can take minutes)
pub(crate) fn build_http_client() -> reqwest::Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .connect_timeout(std::time::Duration::from_secs(8))
        .read_timeout(std::time::Duration::from_secs(60))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .http2_keep_alive_interval(std::time::Duration::from_secs(30))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub(crate) async fn retry_with_backoff<F, Fut>(
    max_retries: u32,
    f: F,
) -> anyhow::Result<reqwest::Response>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let mut attempt = 0;
    loop {
        let result = f().await;
        match result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }
                let should_retry = status.as_u16() == 429
                    || status.as_u16() == 408
                    || status.as_u16() >= 500;
                if attempt < max_retries && should_retry {
                    attempt += 1;
                    let delay_ms = response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|secs| secs * 1000)
                        .unwrap_or_else(|| {
                            let base = 1000u64 * 2u64.pow(attempt);
                            let jitter = (rand_seed() % 300).max(50);
                            base + jitter
                        });
                    tracing::warn!(
                        "Provider returned {}, retrying in {}ms (attempt {}/{})",
                        status,
                        delay_ms,
                        attempt,
                        max_retries
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }
                let text = response.text().await.unwrap_or_default();
                tracing::error!("Provider returned {}: {}", status, text);
                return Err(anyhow::anyhow!("HTTP {}: {}", status, text));
            }
            Err(e) => {
                if attempt < max_retries && (e.is_connect() || e.is_timeout()) {
                    attempt += 1;
                    let base = 1000u64 * 2u64.pow(attempt);
                    let jitter = (rand_seed() % 300).max(50);
                    let delay_ms = base + jitter;
                    tracing::warn!(
                        "Connection error, retrying in {}ms (attempt {}/{}): {}",
                        delay_ms,
                        attempt,
                        max_retries,
                        e
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }
                return Err(anyhow::anyhow!("Request failed: {}", e));
            }
        }
    }
}

fn rand_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
}

/// Append raw bytes to a byte buffer and extract all complete UTF-8 text,
/// leaving any trailing incomplete multi-byte sequence in the buffer.
///
/// This prevents mojibake when a TCP chunk splits a multi-byte character
/// (e.g. emoji) at an arbitrary byte boundary.
fn drain_complete_utf8(buf: &mut Vec<u8>, incoming: &[u8]) -> String {
    buf.extend_from_slice(incoming);
    let mut cut = buf.len();
    while cut > 0 {
        if std::str::from_utf8(&buf[..cut]).is_ok() {
            break;
        }
        cut -= 1;
    }
    let trailing = buf[cut..].to_vec();
    let text = std::str::from_utf8(&buf[..cut])
        .unwrap_or_default()
        .to_string();
    buf.clear();
    buf.extend_from_slice(&trailing);
    text
}

/// Parse an HTTP response body as an SSE stream, yielding each `data:`
/// payload as a decoded `String`.
///
/// Handles:
/// - Byte-level buffering so multi-byte UTF-8 characters split across TCP
///   chunks are reassembled correctly (no mojibake on emoji/CJK).
/// - Line buffering so SSE events split across chunks are reassembled.
/// - Skipping non-`data:` lines (comments, `event:`, `id:`, etc.).
/// - Stopping on the `[DONE]` sentinel.
///
/// Each provider only needs to JSON-parse the yielded strings.
pub(crate) fn sse_data_lines(
    response: reqwest::Response,
) -> impl futures::Stream<Item = anyhow::Result<String>> + Send {
    use futures::StreamExt;

    async_stream::stream! {
        let mut stream = response.bytes_stream();
        let mut byte_buf: Vec<u8> = Vec::new();
        let mut line_buf = String::new();
        let mut data_lines: Vec<String> = Vec::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    yield Err(anyhow::anyhow!("Stream error: {}", e));
                    return;
                }
            };
            line_buf.push_str(&drain_complete_utf8(&mut byte_buf, &chunk));

            while let Some(pos) = line_buf.find('\n') {
                let line = line_buf[..pos].trim().to_string();
                line_buf = line_buf[pos + 1..].to_string();
                if line.is_empty() {
                    if !data_lines.is_empty() {
                        let data = data_lines.join("\n");
                        data_lines.clear();
                        if data == "[DONE]" {
                            return;
                        }
                        tracing::trace!(chars = data.len(), "SSE data event received");
                        yield Ok(data);
                    }
                    continue;
                }
                if line.starts_with(':') {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:") else {
                    if data_lines.is_empty() && line.starts_with('{') {
                        tracing::trace!(chars = line.len(), "JSON response event received");
                        yield Ok(line);
                    }
                    continue;
                };
                data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_string());
            }
        }

        // Drain any remaining partial line (no trailing newline). Some relays
        // return a complete JSON response instead of SSE despite stream=true.
        let remaining = line_buf.trim_end_matches(['\r', '\n']);
        if !remaining.is_empty() {
            if let Some(data) = remaining.strip_prefix("data:") {
                data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_string());
            } else if data_lines.is_empty() {
                yield Ok(remaining.to_string());
            }
        }
        if !data_lines.is_empty() {
            let data = data_lines.join("\n");
            if data != "[DONE]" && !data.is_empty() {
                yield Ok(data);
            }
        }
    }
}
