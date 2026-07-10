//! WebFetch tool — HTTP GET with HTML-to-text conversion.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};
use serde_json::Value;
use std::collections::HashMap;

/// Max allowed fetch timeout in seconds.
const MAX_TIMEOUT: u64 = 120;

/// Max response body size in bytes.
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Content types we accept as text.
const TEXT_CONTENT_TYPES: &[&str] = &[
    "text/", "application/json", "application/xml", "application/xhtml",
];

pub struct WebFetchTool;

#[async_trait::async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "webfetch"
    }
    fn description(&self) -> &'static str {
        "Fetch content from a URL, returning text, markdown, or raw HTML. Markdown is the default."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch (http/https only)" },
                "format": { "type": "string", "enum": ["text", "markdown", "html"] },
                "timeout": { "type": "integer", "description": "Timeout seconds (default 30, max 120)" }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let url = require_str!(p, "url");
        let timeout = p.u64_or("timeout", 30).min(MAX_TIMEOUT);

        if let Err(msg) = validate_url(url) {
            return ToolResult::error(msg);
        }

        let client = try_tool!(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .user_agent("openrust/0.0")
                .redirect(reqwest::redirect::Policy::none())
                .build(),
            |e| format!("Client error: {}", e)
        );

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("Cloudflare") || err_str.contains("403") || err_str.contains("1020") {
                    return ToolResult::error(format!(
                        "Blocked by Cloudflare: {}. Try again or use a different source.",
                        url
                    ));
                }
                return ToolResult::error(format!("Fetch failed: {}", err_str));
            }
        };

        let status = resp.status();
        if status.is_redirection() {
            return ToolResult::error(format!(
                "Redirect not followed (SSRF protection): {} → {}. Use the direct URL if trusted.",
                url,
                resp.headers().get("location")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("?")
            ));
        }
        if !status.is_success() {
            return ToolResult::error(format!(
                "HTTP {} fetching {}",
                status.as_u16(),
                url
            ));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let is_text = TEXT_CONTENT_TYPES
            .iter()
            .any(|prefix| content_type.to_lowercase().starts_with(prefix));

        if !content_type.is_empty() && !is_text {
            return ToolResult::error(format!(
                "Non-text content type '{}' not supported for {}. Use bash with curl for binary downloads.",
                content_type,
                url
            ));
        }

        let body = try_tool!(resp.text().await, |e| format!("Read body: {}", e));

        if body.len() > MAX_BODY_SIZE {
            return ToolResult::error(format!(
                "Response too large: {:.1} KB (max {:.1} KB)",
                body.len() as f64 / 1024.0,
                MAX_BODY_SIZE as f64 / 1024.0
            ));
        }

        let format = p.opt_str("format").unwrap_or("markdown");
        let content = if format == "html" {
            body.clone()
        } else {
            strip_html(&body)
        };

        let (out, truncated) = if content.len() > 100_000 {
            let preview = format!(
                "{}...\n\n[truncated at 100K / {} total]",
                crate::tool::truncate_str(&content, 100_000),
                content.len()
            );
            (preview, true)
        } else {
            (content, false)
        };

        let mut metadata = HashMap::new();
        metadata.insert("url".to_string(), serde_json::json!(url));
        metadata.insert("status".to_string(), serde_json::json!(status.as_u16()));
        metadata.insert("size".to_string(), serde_json::json!(body.len()));
        metadata.insert("truncated".to_string(), serde_json::json!(truncated));
        ToolResult::Structured { content: out, metadata }
    }
}

fn validate_url(url_str: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;
    let scheme = parsed.scheme();

    // Block non-http schemes
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme '{}'. Only http/https allowed.", scheme));
    }

    // Block internal/localhost IPs
    if let Some(host) = parsed.host_str() {
        let h = host.to_lowercase();
        if h == "localhost" || h == "127.0.0.1" || h == "[::1]"
            || h.starts_with("192.168.")
            || h.starts_with("10.")
            || h.starts_with("169.254.")
            || h == "0.0.0.0"
            || h.starts_with("172.16.")
            || h.starts_with("172.17.")
            || h.starts_with("172.18.")
            || h.starts_with("172.19.")
            || h.starts_with("172.20.")
            || h.starts_with("172.21.")
            || h.starts_with("172.22.")
            || h.starts_with("172.23.")
            || h.starts_with("172.24.")
            || h.starts_with("172.25.")
            || h.starts_with("172.26.")
            || h.starts_with("172.27.")
            || h.starts_with("172.28.")
            || h.starts_with("172.29.")
            || h.starts_with("172.30.")
            || h.starts_with("172.31.")
        {
            return Err(format!(
                "Internal/private address '{}' is not allowed.",
                host
            ));
        }
    }

    Ok(())
}

fn decode_html_entity(s: &str) -> Option<(char, usize)> {
    if s.starts_with("&amp;") { return Some(('&', 5)); }
    if s.starts_with("&lt;") { return Some(('<', 4)); }
    if s.starts_with("&gt;") { return Some(('>', 4)); }
    if s.starts_with("&quot;") { return Some(('"', 6)); }
    if s.starts_with("&apos;") { return Some(('\'', 6)); }
    if s.starts_with("&#") {
        let after = &s[2..];
        let end = after.find(';')?;
        let num_str = &after[..end];
        let code = if let Some(hex) = num_str.strip_prefix('x').or_else(|| num_str.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num_str.parse::<u32>().ok()?
        };
        let ch = char::from_u32(code)?;
        return Some((ch, 2 + end + 1));
    }
    None
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let lower = html.to_lowercase();
    let len = html.len();
    let mut i = 0usize;

    while i < len {
        let rest = &html[i..];
        let lower_rest = &lower[i..];
        let ch = rest.chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();

        if ch == '<' {
            if lower_rest.starts_with("<script") {
                in_script = true;
            } else if lower_rest.starts_with("<style") {
                in_style = true;
            } else if lower_rest.starts_with("</script") {
                in_script = false;
            } else if lower_rest.starts_with("</style") {
                in_style = false;
            }
            in_tag = true;
            i += ch_len;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            let end = i.min(lower.len().saturating_sub(1));
            let ctx = &lower[i.saturating_sub(10)..=end];
            if (ctx.contains("</p")
                || ctx.contains("</div")
                || ctx.contains("</h")
                || ctx.contains("<br"))
                && !out.ends_with('\n')
            {
                out.push('\n');
            }
            i += ch_len;
            continue;
        }
        if in_tag || in_script || in_style {
            i += ch_len;
            continue;
        }
        if ch == '&' {
            if let Some((decoded, entity_len)) = decode_html_entity(rest) {
                out.push(decoded);
                i += entity_len;
                continue;
            }
        }
        out.push(ch);
        i += ch_len;
    }
    out.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags() {
        let t = strip_html("<html><body><p>Hello</p><p>World</p></body></html>");
        assert!(t.contains("Hello") && t.contains("World"));
    }

    #[test]
    fn decodes_numeric_entities() {
        let t = strip_html("<p>It&#39;s a test &#x27;ok&#x27;</p>");
        assert!(t.contains("It's a test 'ok'"));
    }

    #[test]
    fn removes_script() {
        let t = strip_html("<script>alert(1)</script><p>Safe</p>");
        assert!(!t.contains("alert") && t.contains("Safe"));
    }

    #[test]
    fn validates_https_urls() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path").is_ok());
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("not-a-url").is_err());
    }

    #[test]
    fn rejects_private_addresses() {
        assert!(validate_url("http://localhost:8080").is_err());
        assert!(validate_url("http://127.0.0.1").is_err());
        assert!(validate_url("http://192.168.1.1").is_err());
    }

    #[tokio::test]
    async fn bad_url_graceful() {
        let c = ToolContext::new(std::env::current_dir().unwrap());
        let r = WebFetchTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "url": "not-a-url-!!!", "timeout": 3
                })),
                &c,
            )
            .await;
        assert!(matches!(r, ToolResult::Error(_)));
    }
}
