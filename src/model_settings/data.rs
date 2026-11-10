//! Model catalog validation and projection.
use super::*;
pub(super) fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => "OpenAI Chat",
        Protocol::OpenAiResponses => "OpenAI Responses",
        Protocol::AnthropicMessages => "Anthropic Messages",
    }
}
pub(super) fn unknown_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        streaming: false,
        tools: false,
        vision: false,
    }
}
pub(super) fn validate_model_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() || id.len() > 512 || id != id.trim() || id.chars().any(char::is_control)
    {
        Err("模型 ID 不能为空、含控制字符或首尾空白，长度最多 512 字节".into())
    } else {
        Ok(())
    }
}
pub(super) fn model_url(base: &str, protocol: Protocol) -> Result<String, String> {
    let base = validate_url(base)?;
    let mut url = reqwest::Url::parse(&base).map_err(|_| "地址无效".to_owned())?;
    let path = url.path().trim_end_matches('/');
    let path = if protocol == Protocol::AnthropicMessages
        && !path.split('/').any(|segment| segment == "v1")
    {
        format!("{path}/v1/models")
    } else {
        format!("{path}/models")
    };
    url.set_path(&path);
    Ok(url.to_string())
}
pub(super) fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 128 && !name.chars().any(char::is_control)
}
pub(super) fn project(file: &SettingsFile) -> Result<(), String> {
    let mut ids = HashSet::new();
    if file.providers.len() > 128 {
        return Err("最多保存 128 个服务商".into());
    }
    for provider in &file.providers {
        if provider.id.is_empty()
            || provider.id.len() > 128
            || !ids.insert(provider.id.clone())
            || !valid_name(&provider.name)
            || provider.endpoints.len() > 64
        {
            return Err("服务商名称应为 1–128 字符且不含控制字符；每个服务商最多 64 条线路，配置标识必须唯一".into());
        }
        for endpoint in &provider.endpoints {
            if endpoint.id.is_empty()
                || endpoint.id.len() > 128
                || !ids.insert(endpoint.id.clone())
                || !valid_name(&endpoint.name)
                || endpoint.models.len() > 10000
            {
                return Err("线路名称应为 1–128 字符且不含控制字符；每条线路最多 10000 个模型，配置标识必须唯一".into());
            }
            validate_url(&endpoint.base_url)?;
            if endpoint.keys.len() > MAX_KEYS {
                return Err("每条线路最多保存 16 个密钥".into());
            }
            for key in &endpoint.keys {
                if key.id.is_empty() || !ids.insert(key.id.clone()) || key.ciphertext.is_empty() {
                    return Err("密钥元数据无效；原配置未覆盖".into());
                }
            }
            let mut model_ids = HashSet::new();
            for model in &endpoint.models {
                validate_model_id(&model.id)?;
                model.settings.validate(endpoint.protocol)?;
                if !model_ids.insert(&model.id) {
                    return Err("同一线路不能重复添加模型 ID".into());
                }
            }
        }
    }
    Ok(())
}

pub(super) fn primary_key(endpoint: &EndpointEntry) -> Option<&SavedKey> {
    endpoint
        .selected_key
        .as_ref()
        .and_then(|id| {
            endpoint
                .keys
                .iter()
                .find(|key| key.enabled && &key.id == id)
        })
        .or_else(|| endpoint.keys.iter().find(|key| key.enabled))
        .or_else(|| endpoint.keys.first())
}

pub(super) fn model_options(file: &SettingsFile) -> Vec<ModelOption> {
    model_options_for(file, ModelType::Chat)
}
pub(super) fn model_options_for(file: &SettingsFile, kind: ModelType) -> Vec<ModelOption> {
    file.providers
        .iter()
        .filter(|p| p.enabled)
        .flat_map(|p| {
            p.endpoints.iter().flat_map(move |e| {
                e.models
                    .iter()
                    .filter(move |m| m.settings.model_type == kind)
                    .map(move |m| ModelOption {
                        model: lumakit_ai::ModelRef {
                            provider: p.id.clone(),
                            endpoint: e.id.clone(),
                            model: m.id.clone(),
                            protocol: e.protocol,
                        },
                        name: m.settings.display_name(&m.id).to_owned(),
                        provider: p.name.clone(),
                        endpoint: e.name.clone(),
                        settings: m.effective_settings(e.protocol),
                        has_key: e
                            .keys
                            .iter()
                            .any(|key| key.enabled && !key.ciphertext.is_empty()),
                    })
            })
        })
        .collect()
}
pub(super) fn repair_model_selection(endpoint: &mut EndpointEntry) {
    if !endpoint
        .models
        .iter()
        .any(|m| Some(&m.id) == endpoint.selected_model.as_ref())
    {
        endpoint.selected_model = endpoint.models.first().map(|m| m.id.clone());
    }
}
pub(super) fn merge_models(
    models: &mut Vec<ModelEntry>,
    discovered: &[String],
    selected: &HashSet<String>,
) {
    models.retain(|m| !discovered.contains(&m.id) || selected.contains(&m.id));
    for id in discovered {
        if selected.contains(id) && !models.iter().any(|m| &m.id == id) {
            models.push(ModelEntry {
                id: id.clone(),
                capabilities: unknown_capabilities(),
                manual: false,
                settings: Default::default(),
            });
        }
    }
}
pub(super) fn parse_models(body: &[u8]) -> Result<Vec<String>, String> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "响应不是有效 JSON；原模型未修改".to_owned())?;
    let rows = json
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "响应缺少 data 模型列表；原模型未修改".to_owned())?;
    if rows.len() > 10000 {
        return Err("模型目录超过 10000 条限制，原模型未修改".into());
    }
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for row in rows {
        let id = row
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "模型列表含无效 ID；原模型未修改".to_owned())?;
        validate_model_id(id)?;
        if seen.insert(id.to_owned()) {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}
pub(super) fn model_page(
    body: &[u8],
    protocol: Protocol,
) -> Result<(Vec<String>, Option<String>), String> {
    let ids = parse_models(body)?;
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "模型目录 JSON 无效".to_owned())?;
    let has_more = match json.get("has_more") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| "模型目录分页标记无效；原模型未修改".to_owned())?,
    };
    if !has_more {
        return Ok((ids, None));
    }
    if protocol != Protocol::AnthropicMessages {
        return Err("此协议返回了不支持的分页格式；原模型未修改".into());
    }
    if ids.is_empty() {
        return Err("模型目录分页为空但仍标记有下一页；原模型未修改".into());
    }
    let cursor = json
        .get("last_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "模型目录分页缺少 last_id；原模型未修改".to_owned())?;
    validate_model_id(cursor)?;
    Ok((ids, Some(cursor.to_owned())))
}
