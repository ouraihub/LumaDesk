//! Page response decoding and plain-text extraction.
use super::*;

pub(super) fn parse_page(
    provider: Provider,
    value: &serde_json::Value,
    target: &str,
) -> Result<Page, String> {
    let data = if provider == Provider::QueritFetch {
        value
            .pointer("/results")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .or_else(|| {
                value
                    .pointer("/data")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
            })
            .unwrap_or(value)
    } else {
        value.get("data").unwrap_or(value)
    };
    let metadata = data.get("metadata").unwrap_or(data);
    let title = text_field(metadata, &["title"]).if_empty_then(&text_field(data, &["title"]));
    let body = text_field(data, &["markdown", "content", "text"]);
    if body.trim().is_empty() {
        return Err("网页响应缺少正文".into());
    }
    let url = text_field(metadata, &["sourceURL", "sourceUrl", "url", "link"])
        .if_empty_then(&text_field(data, &["url", "link"]))
        .if_empty_then(target);
    Ok(Page {
        title: title.if_empty_then(&url),
        url,
        body,
    })
}
pub(in crate::search_settings) fn html_text(raw: &str) -> (String, String) {
    let lower = raw.to_ascii_lowercase();
    let title = lower
        .find("<title>")
        .and_then(|a| {
            lower[a + 7..]
                .find("</title>")
                .map(|b| decode_entities(&raw[a + 7..a + 7 + b]))
        })
        .unwrap_or_default();
    let mut out = String::new();
    let mut pos = 0;
    while pos < raw.len() {
        if raw.as_bytes()[pos] == b'<' {
            let Some(end) = raw[pos..].find('>') else {
                break;
            };
            let end = pos + end;
            let tag = lower[pos + 1..end].trim();
            let skip = ["script", "style", "noscript", "template"]
                .into_iter()
                .find(|name| tag == *name || tag.split_whitespace().next() == Some(*name));
            if let Some(name) = skip {
                if let Some(close) = lower[end + 1..].find(&format!("</{name}")) {
                    let close = end + 1 + close;
                    if let Some(close_end) = lower[close..].find('>') {
                        pos = close + close_end + 1;
                        continue;
                    }
                }
                break;
            }
            out.push(' ');
            pos = end + 1;
        } else {
            let ch = raw[pos..].chars().next().unwrap_or(' ');
            out.push(ch);
            pos += ch.len_utf8();
        }
    }
    (
        title,
        decode_entities(&out)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        if let Some(end) = rest.find(';').filter(|n| *n <= 12) {
            let entity = &rest[1..end];
            let c = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|s| s.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(c) = c {
                out.push(c);
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}
