//! WebSearch tool — multi-engine web search (Bing, DuckDuckGo) with
//! automatic fallback when an engine fails or returns nothing (e.g. the
//! DuckDuckGo HTML endpoint silently rate-limits to empty results).

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};
use serde_json::Value;

pub struct WebSearchTool;

#[derive(Clone, Copy)]
enum Engine {
    Bing,
    DuckDuckGo,
}

impl Engine {
    fn name(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::DuckDuckGo => "duckduckgo",
        }
    }

    fn url(self, query: &str) -> String {
        match self {
            Self::Bing => format!("https://www.bing.com/search?q={}", urlencoding::encode(query)),
            Self::DuckDuckGo => format!(
                "https://html.duckduckgo.com/html/?q={}",
                urlencoding::encode(query)
            ),
        }
    }

    fn parse(self, html: &str, limit: usize) -> Vec<Sr> {
        match self {
            Self::Bing => parse_bing(html, limit),
            Self::DuckDuckGo => parse_ddg(html, limit),
        }
    }
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "websearch"
    }
    fn description(&self) -> &'static str {
        "Search the web. Returns titles, URLs, and snippets. The optional engine parameter \
         picks a backend (auto/bing/duckduckgo); auto falls back to the next engine when \
         one fails or returns nothing."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 10, max 20)" },
                "engine": { "type": "string", "description": "Search backend: auto (default, fallback chain), bing, or duckduckgo" }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let query = require_str!(p, "query");
        let limit = p.u64_or("limit", 10).min(20) as usize;

        // Engine preference: tool parameter > config default > auto chain.
        let config_engine = crate::core::config::Config::load(&ctx.cwd)
            .ok()
            .and_then(|c| c.search_engine);
        let preferred = p
            .opt_str("engine")
            .map(|s| s.to_string())
            .or(config_engine)
            .unwrap_or_else(|| "auto".to_string());
        let engines: Vec<Engine> = match preferred.to_ascii_lowercase().as_str() {
            "bing" => vec![Engine::Bing],
            "duckduckgo" | "ddg" => vec![Engine::DuckDuckGo],
            _ => vec![Engine::Bing, Engine::DuckDuckGo],
        };

        let client = try_tool!(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (compatible; openrust/0.0)")
                .build(),
            |e| format!("Client: {}", e)
        );

        let mut attempts: Vec<String> = Vec::new();
        for engine in engines {
            let resp = client.get(engine.url(query)).send().await;
            let html = match resp {
                Ok(r) if r.status().is_success() => match r.text().await {
                    Ok(t) => t,
                    Err(e) => {
                        attempts.push(format!("{}: read failed ({e})", engine.name()));
                        continue;
                    }
                },
                Ok(r) => {
                    attempts.push(format!("{}: HTTP {}", engine.name(), r.status().as_u16()));
                    continue;
                }
                Err(e) => {
                    attempts.push(format!("{}: request failed ({e})", engine.name()));
                    continue;
                }
            };

            let results = engine.parse(&html, limit);
            if results.is_empty() {
                // Empty here often means silent rate-limiting, not a genuine
                // no-hit — try the next engine before giving up.
                attempts.push(format!("{}: no results (possible rate-limit)", engine.name()));
                continue;
            }

            let out: Vec<String> = results
                .iter()
                .enumerate()
                .map(|(i, r)| format!("{}. {}\n   {}\n   {}", i + 1, r.title, r.url, r.snippet))
                .collect();
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("query".to_string(), serde_json::json!(query));
            metadata.insert("results".to_string(), serde_json::json!(results.len()));
            metadata.insert("engine".to_string(), serde_json::json!(engine.name()));
            if !attempts.is_empty() {
                metadata.insert("fallback".to_string(), serde_json::json!(attempts.join("; ")));
            }
            return ToolResult::Structured {
                content: format!("[engine: {}]\n{}", engine.name(), out.join("\n\n")),
                metadata,
            };
        }

        ToolResult::text(format!(
            "No results found for this query. (attempts: {})",
            attempts.join("; ")
        ))
    }
}

struct Sr {
    title: String,
    url: String,
    snippet: String,
}

/// Parse DuckDuckGo HTML endpoint results.
fn parse_ddg(html: &str, limit: usize) -> Vec<Sr> {
    let mut results = Vec::new();
    for part in html.split("result__body").skip(1).take(limit) {
        let title = extract(part, "result__a\"", "</a>")
            .or_else(|| extract(part, "result__title\"", "</a>"))
            .map(tag_text)
            .unwrap_or("");
        let title = decode(strip_tags(title).trim());
        let url = extract(part, "result__url\"", "</a>")
            .map(tag_text)
            .or_else(|| extract(part, "href=\"", "\""))
            .unwrap_or("");
        let url = decode(strip_tags(url).trim());
        let snippet = extract(part, "result__snippet\"", "</a>")
            .or_else(|| extract(part, "result__snippet\">", "</"))
            .map(tag_text)
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

/// Drop the tag-attribute head of an extracted fragment, keeping the text
/// after the first `>` (extract slices from a class-name match, so the raw
/// fragment still carries ` href="...">` etc.).
fn tag_text(raw: &str) -> &str {
    match raw.split_once('>') {
        Some((_, text)) => text,
        None => raw,
    }
}

/// Parse Bing results page: each organic hit is `<li class="b_algo">` with
/// `<h2><a href="URL">TITLE</a></h2>` and a caption `<p>…</p>`.
fn parse_bing(html: &str, limit: usize) -> Vec<Sr> {
    let mut results = Vec::new();
    for part in html.split("<li class=\"b_algo\"").skip(1).take(limit) {
        let head = extract(part, "<h2><a ", "</a>").unwrap_or("");
        let url = extract(head, "href=\"", "\"").unwrap_or("");
        let title = head.split_once('>').map(|(_, t)| t).unwrap_or("");
        let title = decode(strip_tags(title).trim());
        let url = decode(url.trim());
        let snippet = extract(part, "<p>", "</p>")
            .or_else(|| extract(part, "<p ", "</p>").and_then(|s| {
                s.split_once('>').map(|(_, text)| text)
            }))
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

    #[test]
    fn parses_bing_results() {
        let html = r#"<ol id="b_results">
<li class="b_algo"><h2><a href="https://example.com/a" target="_blank">Example Title</a></h2>
<div class="b_caption"><p>First snippet here.</p></div></li>
<li class="b_algo"><h2><a href="https://example.com/b">Second &amp; Title</a></h2>
<div class="b_caption"><p class="b_lineclamp2">Second snippet.</p></div></li>
</ol>"#;
        let results = parse_bing(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Title");
        assert_eq!(results[0].url, "https://example.com/a");
        assert_eq!(results[0].snippet, "First snippet here.");
        assert_eq!(results[1].title, "Second & Title");
        assert_eq!(results[1].snippet, "Second snippet.");
    }

    #[test]
    fn parses_ddg_results() {
        let html = r#"<div class="result__body">
<a class="result__a" href="https://ddg.example/x">DDG Title</a>
<a class="result__url" href="https://ddg.example/x">ddg.example</a>
<a class="result__snippet">DDG snippet text</a>
</div>"#;
        let results = parse_ddg(html, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "DDG Title");
        assert_eq!(results[0].snippet, "DDG snippet text");
    }
}
