//! Saved-endpoint model detection, key rotation and pagination.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;

pub(super) fn detect_models(
    endpoint: &EndpointEntry,
    cancel: CancellationToken,
    trace: &TraceContext,
) -> Result<Vec<String>, Failure> {
    if !endpoint.keys.is_empty() && !endpoint.keys.iter().any(|key| key.enabled) {
        return Err(Failure::failed(
            "SETTINGS_NO_ENABLED_KEY",
            "此线路没有已启用的密钥；请先启用至少一个密钥再检测",
        ));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::failed("SETTINGS_RUNTIME_FAILED", "无法启动网络任务"))?;
    runtime.block_on(async {
        let url = model_url(&endpoint.base_url, endpoint.protocol)
            .map_err(|message| Failure::failed("SETTINGS_INVALID_ENDPOINT", message))?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| Failure::failed("SETTINGS_CLIENT_FAILED", "无法初始化 HTTPS 客户端"))?;
        let deadline = task::deadline(45_000);
        let first = endpoint
            .selected_key
            .as_ref()
            .and_then(|id| {
                endpoint
                    .keys
                    .iter()
                    .position(|key| key.enabled && &key.id == id)
            })
            .or_else(|| endpoint.keys.iter().position(|key| key.enabled))
            .unwrap_or(0);
        let keys = endpoint
            .keys
            .iter()
            .cycle()
            .skip(first)
            .take(endpoint.keys.len())
            .filter(|key| key.enabled)
            .map(Some)
            .chain(endpoint.keys.is_empty().then_some(None));
        let mut last_status = 401;
        'keys: for key in keys {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            let secret = key
                .map(SavedKey::open)
                .transpose()
                .map_err(|message| Failure::failed("SETTINGS_SECRET_FAILED", message))?;
            let mut ids = Vec::new();
            let mut seen = HashSet::new();
            let mut cursors = HashSet::new();
            let mut after_id: Option<String> = None;
            for _ in 0..10000 {
                let mut page_url = reqwest::Url::parse(&url).map_err(|_| {
                    Failure::failed("SETTINGS_INVALID_ENDPOINT", "模型目录地址无效")
                })?;
                if let Some(cursor) = &after_id {
                    page_url.query_pairs_mut().append_pair("after_id", cursor);
                }
                let mut request = client.get(page_url);
                if endpoint.protocol == Protocol::AnthropicMessages {
                    request = request.header("anthropic-version", "2023-06-01");
                }
                if let Some(secret) = &secret {
                    request = if endpoint.protocol == Protocol::AnthropicMessages {
                        request.header("x-api-key", secret.as_str())
                    } else {
                        request.bearer_auth(secret.as_str())
                    };
                }
                let page = task::http(
                    trace,
                    "settings.models.http",
                    &cancel,
                    deadline,
                    45_000,
                    async {
                        let response = request
                            .send()
                            .await
                            .map_err(|e| Failure::request(e, 8_000, 20_000))?;
                        let status = response.status().as_u16();
                        if !response.status().is_success() {
                            return Err(task::status(status));
                        }
                        if response
                            .content_length()
                            .is_some_and(|n| n > MAX_BODY as u64)
                        {
                            return Err("模型目录超过 2 MiB 限制".into());
                        }
                        let mut stream = response.bytes_stream();
                        let mut body = Vec::new();
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk.map_err(|e| Failure::request(e, 8_000, 20_000))?;
                            if body.len() + chunk.len() > MAX_BODY {
                                return Err("模型目录超过 2 MiB 限制".into());
                            }
                            body.extend_from_slice(&chunk);
                        }
                        model_page(&body, endpoint.protocol).map_err(Failure::from)
                    },
                )
                .await;
                let (page, next) = match page {
                    Ok(page) => page,
                    Err(error)
                        if matches!(
                            error.code(),
                            "SETTINGS_HTTP_UNAUTHORIZED" | "SETTINGS_HTTP_RATE_LIMITED"
                        ) =>
                    {
                        if let Failure::Http(status) = error {
                            last_status = status;
                        }
                        continue 'keys;
                    }
                    Err(error) => return Err(error),
                };
                for id in page {
                    if seen.insert(id.clone()) {
                        ids.push(id);
                    }
                }
                if ids.len() > 10000 {
                    return Err("模型目录总数超过 10000 条限制；原模型未修改".into());
                }
                let Some(next) = next else {
                    return Ok(ids);
                };
                if !cursors.insert(next.clone()) {
                    return Err("模型目录分页游标重复；原模型未修改".into());
                }
                after_id = Some(next);
            }
            return Err("模型目录分页次数超过限制；原模型未修改".into());
        }
        if endpoint.keys.is_empty() {
            return Err(Failure::failed(
                "SETTINGS_KEY_NOT_SAVED",
                format!(
                    "此线路还没有已保存的密钥，服务返回 HTTP {last_status}；请先添加 API 密钥再检测"
                ),
            ));
        }
        Err(task::status(last_status))
    })
}
