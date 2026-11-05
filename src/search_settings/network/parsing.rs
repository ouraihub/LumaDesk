//! Provider JSON and MCP response decoding.
use super::*;
pub(super) fn provider_error(provider: Provider, value: &serde_json::Value) -> bool {
    match provider {
        Provider::Bocha => value
            .get("code")
            .and_then(|v| v.as_i64())
            .is_some_and(|v| v != 200),
        Provider::Querit | Provider::QueritFetch => value
            .get("error_code")
            .and_then(|v| v.as_i64())
            .is_some_and(|v| v != 200),
        Provider::Firecrawl | Provider::FirecrawlFetch => {
            value.get("success") == Some(&serde_json::Value::Bool(false))
        }
        _ => value.get("error").is_some_and(|v| !v.is_null()),
    }
}

pub(super) fn text_field(value: &serde_json::Value, fields: &[&str]) -> String {
    fields
        .iter()
        .find_map(|field| match value.get(*field) {
            Some(serde_json::Value::String(text)) if !text.trim().is_empty() => Some(text.clone()),
            Some(serde_json::Value::Array(parts)) => Some(
                parts
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn parse_hits(
    provider: Provider,
    value: &serde_json::Value,
) -> Result<Vec<SearchHit>, String> {
    let path = match provider {
        Provider::Zhipu => "/search_result",
        Provider::Bocha => "/data/webPages/value",
        Provider::Querit => "/results/result",
        Provider::Brave => "/web/results",
        Provider::Parallel => "/results",
        Provider::JinaSearch => "/data",
        Provider::Firecrawl => "/data/web",
        _ => "/results",
    };
    let rows = value
        .pointer(path)
        .and_then(|v| v.as_array())
        .ok_or("搜索响应缺少结果列表")?;
    let hits = rows
        .iter()
        .filter_map(|row| {
            let url = text_field(row, &["url", "link"]);
            if url.is_empty() {
                return None;
            }
            Some(SearchHit {
                title: text_field(row, &["title", "name", "site_name"]).if_empty_then(&url),
                url,
                snippet: text_field(
                    row,
                    &[
                        "content",
                        "text",
                        "description",
                        "summary",
                        "snippet",
                        "excerpts",
                        "markdown",
                    ],
                ),
            })
        })
        .collect();
    Ok(hits)
}

pub(super) trait EmptyThen {
    fn if_empty_then(self, fallback: &str) -> String;
}
impl EmptyThen for String {
    fn if_empty_then(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_owned()
        } else {
            self
        }
    }
}

pub(super) fn parse_mcp(body: &str) -> Result<Vec<SearchHit>, String> {
    fn content(value: serde_json::Value, texts: &mut Vec<String>) -> Result<(), String> {
        if value.get("error").is_some_and(|error| !error.is_null())
            || value.pointer("/result/isError") == Some(&serde_json::Value::Bool(true))
        {
            return Err("MCP 搜索被供应商拒绝".into());
        }
        if let Some(parts) = value
            .pointer("/result/content")
            .and_then(|parts| parts.as_array())
        {
            texts.extend(parts.iter().filter_map(|part| {
                part.get("text")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            }));
        }
        Ok(())
    }
    let mut texts = Vec::new();
    if let Ok(value) = serde_json::from_str(body) {
        content(value, &mut texts)?;
    } else {
        let mut data = String::new();
        for line in body.lines().chain(std::iter::once("")) {
            if line.is_empty() {
                if !data.is_empty() {
                    let value = serde_json::from_str(data.trim())
                        .map_err(|_| "MCP 搜索事件不是有效 JSON".to_owned())?;
                    content(value, &mut texts)?;
                    data.clear();
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push_str(value.trim_start());
                data.push('\n');
            }
        }
    }
    if texts.is_empty() {
        return Err("MCP 搜索响应缺少结果".into());
    }
    let mut hits = Vec::new();
    let (mut title, mut url, mut snippet, mut reading) =
        (String::new(), String::new(), Vec::new(), false);
    for line in texts.join("\n\n").lines().chain(std::iter::once("Title:")) {
        if let Some(value) = line.strip_prefix("Title:") {
            if !url.is_empty() {
                hits.push(SearchHit {
                    title: if title.is_empty() {
                        url.clone()
                    } else {
                        std::mem::take(&mut title)
                    },
                    url: std::mem::take(&mut url),
                    snippet: snippet.join("\n").trim().to_owned(),
                });
            }
            title = value.trim().to_owned();
            snippet.clear();
            reading = false;
        } else if let Some(value) = line.strip_prefix("URL:") {
            url = value.trim().to_owned();
        } else if let Some(value) = line
            .strip_prefix("Text:")
            .or_else(|| line.strip_prefix("Highlights:"))
        {
            reading = true;
            snippet.push(value.trim_start().to_owned());
        } else if reading {
            snippet.push(line.to_owned());
        }
    }
    Ok(hits)
}
