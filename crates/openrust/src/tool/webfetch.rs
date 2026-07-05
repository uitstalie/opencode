//! WebFetch tool — HTTP GET with HTML-to-text conversion.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};
use serde_json::Value;

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
                "url": { "type": "string", "description": "URL to fetch" },
                "format": { "type": "string", "enum": ["text", "markdown", "html"] },
                "timeout": { "type": "integer", "description": "Timeout seconds (default 30, max 120)" }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let url = require_str!(p, "url");
        let timeout = p.u64_or("timeout", 30).min(120);

        let client = try_tool!(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .user_agent("openrust/0.0")
                .build(),
            |e| format!("Client error: {}", e)
        );

        let resp = try_tool!(client.get(url).send().await, |e| format!("Fetch: {}", e));
        if !resp.status().is_success() {
            return ToolResult::error(format!("HTTP {}", resp.status()));
        }
        let body = try_tool!(resp.text().await, |e| format!("Read: {}", e));
        let format = p.opt_str("format").unwrap_or("markdown");
        let content = if format == "html" {
            body.clone()
        } else {
            strip_html(&body)
        };

        let out = if content.len() > 100_000 {
            format!(
                "{}...\n\n[truncated at 100K / {} total]",
                &content[..100_000],
                content.len()
            )
        } else {
            content
        };
        ToolResult::text(out)
    }
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let lower = html.to_lowercase();
    for (i, ch) in html.char_indices() {
        if ch == '<' {
            let rest = &lower[i..];
            if rest.starts_with("<script") {
                in_script = true;
            } else if rest.starts_with("<style") {
                in_style = true;
            } else if rest.starts_with("</script") {
                in_script = false;
            } else if rest.starts_with("</style") {
                in_style = false;
            }
            in_tag = true;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            let end = i.min(lower.len() - 1);
            let ctx = &lower[i.saturating_sub(10)..=end];
            if ctx.contains("</p")
                || ctx.contains("</div")
                || ctx.contains("</h")
                || ctx.contains("<br")
            {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            continue;
        }
        if in_tag || in_script || in_style {
            continue;
        }
        if ch == '&' {
            let rest = &html[i..];
            if rest.starts_with("&amp;") {
                out.push('&');
                continue;
            }
            if rest.starts_with("&lt;") {
                out.push('<');
                continue;
            }
            if rest.starts_with("&gt;") {
                out.push('>');
                continue;
            }
            if rest.starts_with("&quot;") {
                out.push('"');
                continue;
            }
            if rest.starts_with("&#") {
                continue;
            }
        }
        out.push(ch);
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
    fn removes_script() {
        let t = strip_html("<script>alert(1)</script><p>Safe</p>");
        assert!(!t.contains("alert") && t.contains("Safe"));
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
        let _ = r; // Should not panic
    }
}
