//! Bounded search and page-reading execution.
use super::*;
use crate::settings_task::{self as task, Failure};
use futures_util::StreamExt;
use lumakit_ai::search::{self, SearchHit, SearchResult};
use lumakit_observability::TraceContext;
use reqwest::Url;
mod policy;
#[cfg(test)]
pub(super) use policy::normalize_rule;
pub(super) use policy::{blocked, preferences_from_draft, validate_url};
mod parsing;
use parsing::*;
mod page;
pub(super) use page::html_text;
use page::parse_page;
async fn send(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    trace: &TraceContext,
    deadline: tokio::time::Instant,
    limit_ms: u64,
) -> Result<(u16, String, String), Failure> {
    task::http(
        trace,
        "settings.search.http",
        cancel,
        deadline,
        limit_ms,
        async {
            let response = request
                .send()
                .await
                .map_err(|e| Failure::request(e, 10_000, limit_ms))?;
            let status = response.status().as_u16();
            if !(200..=299).contains(&status) {
                return Err(task::status(status));
            }
            read_limited(response, cancel, limit_ms).await
        },
    )
    .await
}
#[cfg(test)]
mod tests;

fn process_hits(
    query: String,
    provider: Provider,
    mut hits: Vec<SearchHit>,
    preferences: &SearchPreferences,
) -> SearchResult {
    hits.retain(|hit| Url::parse(&hit.url).is_ok_and(|url| !blocked(&url, &preferences.blacklist)));
    let mut policy = preferences.clone();
    policy.blacklist.clear();
    search::process_result(
        SearchResult {
            query,
            provider: provider.id().into(),
            hits,
        },
        &policy,
    )
}

fn auth(
    request: reqwest::RequestBuilder,
    config: &Config,
    key: Option<&str>,
) -> reqwest::RequestBuilder {
    if config.provider == Provider::Searxng {
        return if config.username.is_empty() && key.is_none() {
            request
        } else {
            request.basic_auth(&config.username, key)
        };
    }
    let Some(key) = key else {
        return request;
    };
    let (name, value) = match config.provider {
        Provider::Brave => ("x-subscription-token", key.to_owned()),
        Provider::Exa | Provider::ExaMcp | Provider::Parallel => ("x-api-key", key.to_owned()),
        _ => ("authorization", format!("Bearer {key}")),
    };
    match reqwest::header::HeaderValue::from_str(&value) {
        Ok(mut value) => {
            value.set_sensitive(true);
            request.header(name, value)
        }
        Err(_) => request.header(name, value),
    }
}

fn endpoint_for(config: &Config, search: bool) -> Result<Url, String> {
    let mut endpoint = validate_url(&config.endpoint)?;
    if endpoint.path() != "/" && !endpoint.path().is_empty() {
        return Ok(endpoint);
    }
    let path = if search {
        match config.provider {
            Provider::Tavily | Provider::Exa | Provider::Searxng => "/search",
            Provider::Bocha => "/v1/web-search",
            Provider::Querit | Provider::Parallel => "/v1/search",
            Provider::Brave => "/res/v1/web/search",
            Provider::JinaSearch => "",
            Provider::Firecrawl => "/v2/search",
            Provider::Zhipu | Provider::ExaMcp => "",
            Provider::Direct
            | Provider::Jina
            | Provider::QueritFetch
            | Provider::FirecrawlFetch => "",
        }
    } else {
        match config.provider {
            Provider::Jina => "",
            Provider::QueritFetch => "/v1/contents",
            Provider::FirecrawlFetch => "/v2/scrape",
            _ => "",
        }
    };
    endpoint.set_path(path);
    Ok(endpoint)
}

fn encoded(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 15) as usize] as char);
        }
    }
    out
}

async fn read_limited(
    response: reqwest::Response,
    cancel: &CancellationToken,
    limit_ms: u64,
) -> Result<(u16, String, String), Failure> {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err("响应超过 2 MiB".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::select! { _ = cancel.cancelled() => return Err(Failure::Cancelled), c = stream.next() => c }
    {
        let chunk = chunk.map_err(|e| Failure::request(e, 10_000, limit_ms))?;
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err("响应超过 2 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(bytes).map_err(|_| "仅支持 UTF-8 响应".to_owned())?;
    Ok((status, content_type, body))
}

/// The `attempt`-th key to try: the preferred key first, then the rest in list order, wrapping.
fn nth_key(config: &Config, attempt: usize) -> Option<&SavedKey> {
    let first = config
        .selected_key
        .as_ref()
        .and_then(|id| config.keys.iter().position(|key| &key.id == id))
        .unwrap_or(0);
    let count = config.keys.len();
    (count > 0).then(|| &config.keys[(first + attempt) % count])
}

pub(super) async fn search_query(
    config: Config,
    query: String,
    preferences: SearchPreferences,
    cancel: CancellationToken,
    trace: &TraceContext,
) -> Result<SearchResult, Failure> {
    let deadline = task::deadline(15_000);
    if config.provider.is_fetch() {
        return Err("此服务商不提供搜索".into());
    }
    if config.provider.requires_key() && config.keys.is_empty() {
        return Err(Failure::failed(
            "SETTINGS_KEY_NOT_SAVED",
            "请先填写并保存搜索密钥",
        ));
    }
    let endpoint = endpoint_for(&config, true)
        .map_err(|message| Failure::failed("SETTINGS_INVALID_ENDPOINT", message))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| Failure::failed("SETTINGS_CLIENT_FAILED", "无法创建搜索客户端"))?;
    let max = preferences.max_results.clamp(1, 50);
    let count = max.min(20).to_string();
    let attempts = config.keys.len().clamp(1, 16);
    for index in 0..attempts {
        let secret = nth_key(&config, index)
            .map(SavedKey::open)
            .transpose()
            .map_err(|message| Failure::failed("SETTINGS_SECRET_FAILED", message))?;
        let key = secret.as_ref().map(|value| value.as_str());
        let mut target = endpoint.clone();
        if config.provider == Provider::JinaSearch
            && (target.path().is_empty() || target.path() == "/")
        {
            target.set_path(&format!("/{}", encoded(&query)));
        }
        let request = match config.provider {
            Provider::Zhipu => auth(client.post(target).json(&serde_json::json!({"search_query": query, "search_engine": "search_std", "search_intent": false})), &config, key),
            Provider::Tavily => auth(client.post(target).json(&serde_json::json!({"query": query, "max_results": 50})), &config, key),
            Provider::Searxng => auth(client.get(target).query(&[("q", query.as_str()), ("format", "json"), ("language", "auto")]), &config, key),
            Provider::Exa => auth(client.post(target).json(&serde_json::json!({"query": query, "numResults": 50, "contents": {"text": true}})), &config, key),
            Provider::ExaMcp => auth(client.post(target).header(reqwest::header::ACCEPT, "application/json, text/event-stream").json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"web_search_exa","arguments":{"query":query,"type":"auto","numResults":max,"livecrawl":"fallback"}}})), &config, key),
            Provider::Bocha => auth(client.post(target).json(&serde_json::json!({"query": query, "count": max, "exclude": "", "summary": true})), &config, key),
            Provider::Querit => auth(client.post(target).json(&serde_json::json!({"query": query, "count": max})), &config, key),
            Provider::Brave => auth(client.get(target).query(&[("q", query.as_str()), ("count", count.as_str())]), &config, key),
            Provider::Parallel => auth(client.post(target).json(&serde_json::json!({"objective": query, "search_queries": [query], "advanced_settings": {"max_results": max}})), &config, key),
            Provider::JinaSearch => auth(client.get(target).header(reqwest::header::ACCEPT, "application/json"), &config, key),
            Provider::Firecrawl => auth(client.post(target).json(&serde_json::json!({"query": query, "limit": max, "scrapeOptions": {"formats": ["markdown"]}})), &config, key),
            Provider::Direct | Provider::Jina | Provider::QueritFetch | Provider::FirecrawlFetch => return Err("此服务商不提供搜索".into()),
        };
        let response = send(request, &cancel, trace, deadline, 15_000).await;
        let (_, _content_type, body) = match response {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.code(),
                    "SETTINGS_HTTP_UNAUTHORIZED" | "SETTINGS_HTTP_RATE_LIMITED"
                ) && index + 1 < attempts =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if config.provider == Provider::ExaMcp {
            let hits = parse_mcp(&body)?;
            return Ok(process_hits(query, config.provider, hits, &preferences));
        }
        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| "搜索响应不是有效 JSON".to_owned())?;
        if provider_error(config.provider, &value) {
            return Err("供应商返回业务错误；未轮换密钥".into());
        }
        let hits = if config.provider == Provider::Searxng {
            parse_hits(Provider::Searxng, &value)?
        } else {
            parse_hits(config.provider, &value)?
        };
        return Ok(process_hits(query, config.provider, hits, &preferences));
    }
    Err("没有可用搜索密钥".into())
}

pub(super) async fn fetch_page(
    config: Config,
    target: String,
    prefs: SearchPreferences,
    cancel: CancellationToken,
    trace: &TraceContext,
) -> Result<Page, Failure> {
    let deadline = task::deadline(20_000);
    let url = validate_url(&target)
        .map_err(|message| Failure::failed("SETTINGS_INVALID_ENDPOINT", message))?;
    if blocked(&url, &prefs.blacklist) {
        return Err(Failure::failed(
            "SETTINGS_TARGET_BLOCKED",
            "目标地址命中黑名单，未发起请求",
        ));
    }
    if config.provider == Provider::Direct && !config.keys.is_empty() {
        return Err("直接获取不发送 API 密钥".into());
    }
    if config.provider.requires_key() && config.keys.is_empty() {
        return Err(Failure::failed(
            "SETTINGS_KEY_NOT_SAVED",
            "请先填写并保存读取密钥",
        ));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| Failure::failed("SETTINGS_CLIENT_FAILED", "无法创建读取客户端"))?;
    if config.provider == Provider::Direct {
        let (_, content_type, body) = send(
            client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "text/plain, text/html;q=0.8"),
            &cancel,
            trace,
            deadline,
            20_000,
        )
        .await?;
        if !content_type.starts_with("text/")
            && !content_type.contains("xhtml")
            && !body.trim_start().starts_with('<')
        {
            return Err("读取仅支持文本或 HTML 响应".into());
        }
        let (title, body) = if content_type.contains("html") || body.trim_start().starts_with('<') {
            html_text(&body)
        } else {
            (String::new(), body)
        };
        return Ok(Page {
            title: title.if_empty_then(&target),
            url: target,
            body: body.chars().take(prefs.cutoff).collect(),
        });
    }
    let endpoint = endpoint_for(&config, false)
        .map_err(|message| Failure::failed("SETTINGS_INVALID_ENDPOINT", message))?;
    let attempts = config.keys.len().clamp(1, 16);
    for index in 0..attempts {
        let secret = nth_key(&config, index)
            .map(SavedKey::open)
            .transpose()
            .map_err(|message| Failure::failed("SETTINGS_SECRET_FAILED", message))?;
        let key = secret.as_ref().map(|value| value.as_str());
        let request = match config.provider {
            Provider::Jina => auth(
                client
                    .get(
                        Url::parse(&format!(
                            "{}/{}",
                            endpoint.as_str().trim_end_matches('/'),
                            target
                        ))
                        .map_err(|_| "读取地址无效".to_owned())?,
                    )
                    .header("X-Return-Format", "text"),
                &config,
                key,
            ),
            Provider::QueritFetch => auth(
                client.post(endpoint.clone()).json(
                    &serde_json::json!({"urls":[target],"format":"markdown","extrasMeta":true}),
                ),
                &config,
                key,
            ),
            Provider::FirecrawlFetch => auth(
                client
                    .post(endpoint.clone())
                    .json(&serde_json::json!({"url":target,"formats":["markdown"]})),
                &config,
                key,
            ),
            _ => return Err("此服务商不提供读取".into()),
        };
        let response = send(request, &cancel, trace, deadline, 20_000).await;
        let (_, content_type, body) = match response {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.code(),
                    "SETTINGS_HTTP_UNAUTHORIZED" | "SETTINGS_HTTP_RATE_LIMITED"
                ) && index + 1 < attempts =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if config.provider == Provider::Jina {
            let title = body
                .lines()
                .find_map(|line| line.strip_prefix("Title: "))
                .unwrap_or("")
                .to_owned();
            return Ok(Page {
                title: title.if_empty_then(&target),
                url: target,
                body: body.chars().take(prefs.cutoff).collect(),
            });
        }
        if !content_type.contains("json") && !body.trim_start().starts_with('{') {
            return Err("读取响应不是有效 JSON".into());
        }
        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| "读取响应不是有效 JSON".to_owned())?;
        if provider_error(config.provider, &value) {
            return Err("供应商返回业务错误；未轮换密钥".into());
        }
        let mut page = parse_page(config.provider, &value, &target)?;
        page.body = page.body.chars().take(prefs.cutoff).collect();
        return Ok(page);
    }
    Err("没有可用读取密钥".into())
}
