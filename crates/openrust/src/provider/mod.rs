pub mod anthropic;
pub mod gemini;
pub mod openai_compat;

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
