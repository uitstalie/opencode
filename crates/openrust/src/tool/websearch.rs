//! WebSearch tool — DuckDuckGo HTML search.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};
use serde_json::Value;

pub struct WebSearchTool;

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "websearch"
    }
    fn description(&self) -> &'static str {
        "Search the web via DuckDuckGo HTML results. Returns titles, URLs, and snippets."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 10, max 20)" }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let query = require_str!(p, "query");
        let limit = p.u64_or("limit", 10).min(20) as usize;

        let client = try_tool!(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (compatible; openrust/0.0)")
                .build(),
            |e| format!("Client: {}", e)
        );

        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );
        let resp = try_tool!(client.get(&url).send().await, |e| format!("Search failed: {}", e));
        if !resp.status().is_success() {
            return ToolResult::error(format!("HTTP {} from search", resp.status().as_u16()));
        }
        let html = try_tool!(resp.text().await, |e| format!("Read: {}", e));

        let results = parse_results(&html, limit);
        if results.is_empty() {
            return ToolResult::text("No results found for this query.");
        }
        let out: Vec<String> = results
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}. {}\n   {}\n   {}", i + 1, r.title, r.url, r.snippet))
            .collect();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("query".to_string(), serde_json::json!(query));
        metadata.insert("results".to_string(), serde_json::json!(results.len()));
        ToolResult::Structured {
            content: out.join("\n\n"),
            metadata,
        }
    }
}

struct Sr {
    title: String,
    url: String,
    snippet: String,
}

fn parse_results(html: &str, limit: usize) -> Vec<Sr> {
    let mut results = Vec::new();
    for part in html.split("result__body").skip(1).take(limit) {
        let title = extract(part, "result__a\"", "</a>")
            .or_else(|| extract(part, "result__title\"", "</a>"))
            .unwrap_or("");
        let title = decode(strip_tags(title).trim());
        let url = extract(part, "result__url\"", "</a>")
            .or_else(|| extract(part, "href=\"", "\""))
            .unwrap_or("");
        let url = decode(strip_tags(url).trim());
        let snippet = extract(part, "result__snippet\"", "</a>")
            .or_else(|| extract(part, "result__snippet\">", "</"))
            .unwrap_or("");
        let snippet = decode(strip_tags(snippet).trim());
        if !title.is_empty() && !url.is_empty() {
            results.push(Sr {
                title,
                url,
                snippet,
            });
        }
    }
    results
}

fn extract<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let after = &s[s.find(start)? + start.len()..];
    Some(&after[..after.find(end)?])
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            continue;
        }
        if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strips() {
        assert_eq!(strip_tags("<b>Hi</b>"), "Hi");
    }
    #[test]
    fn decodes() {
        assert_eq!(decode("&amp;&lt;"), "&<");
    }
    #[test]
    fn extracts() {
        assert_eq!(
            extract(r#"<div class="title">Hello</div>"#, "title\">", "</div>"),
            Some("Hello")
        );
    }
}
