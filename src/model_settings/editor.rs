//! Provider, endpoint and credential draft editing.
use super::*;
impl ModelSettings {
    pub(super) fn open_endpoint(&mut self, p: usize, e: Option<usize>) {
        let endpoint = e.map(|e| &self.draft.providers[p].endpoints[e]);
        self.editor = Some(EditorDialog::Endpoint {
            provider: self.draft.providers[p].id.clone(),
            id: endpoint.map(|e| e.id.clone()),
            name: endpoint.map_or_else(|| "新线路".into(), |e| e.name.clone()),
            url: endpoint.map_or_else(String::new, |e| e.base_url.clone()),
            protocol: endpoint.map_or(Protocol::OpenAiChat, |e| e.protocol),
        });
    }

    pub(super) fn reveal_key(&mut self, p: usize, e: usize, key_id: &str) {
        let endpoint = &self.draft.providers[p].endpoints[e];
        let result = endpoint
            .keys
            .iter()
            .find(|key| key.id == key_id)
            .ok_or_else(|| "密钥已不存在".to_owned())
            .and_then(SavedKey::open);
        let endpoint_id = endpoint.id.clone();
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
        draft.revealed = None;
        draft.revealed_key = None;
        match result {
            Ok(secret) => {
                let draft = self.key_drafts.entry(endpoint_id).or_default();
                draft.revealed = Some(secret);
                draft.revealed_key = Some(key_id.into());
            }
            Err(error) => {
                self.report(Err(error), "");
            }
        }
    }

    pub(super) fn save_key(&mut self, p: usize, e: usize) {
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
        let value = Zeroizing::new(draft.value.trim().to_owned());
        let editing = draft.editing.clone();
        let original_keys = self.draft.providers[p].endpoints[e].keys.clone();
        let original_selection = self.draft.providers[p].endpoints[e].selected_key.clone();
        let endpoint = &mut self.draft.providers[p].endpoints[e];
        let result = if value.is_empty() {
            Err("密钥不能为空，原凭据未修改".into())
        } else if let Some(id) = editing {
            endpoint
                .keys
                .iter_mut()
                .find(|key| key.id == id)
                .ok_or_else(|| "待修改密钥已不存在".to_owned())
                .and_then(|key| key.replace(&value))
        } else if endpoint.keys.len() >= MAX_KEYS {
            Err("每条线路最多保存 16 个密钥".into())
        } else {
            SavedKey::new(&value).map(|key| {
                if endpoint.selected_key.is_none() {
                    endpoint.selected_key = Some(key.id.clone());
                }
                endpoint.keys.push(key);
            })
        };
        match result {
            Ok(()) => {
                let result = self.persist_keys(p, e);
                if self.report(result, "密钥已加密保存") {
                    let draft = self.key_drafts.get_mut(&endpoint_id).unwrap();
                    draft.value.zeroize();
                    draft.editing = None;
                    draft.revealed = None;
                    draft.revealed_key = None;
                } else {
                    self.draft.providers[p].endpoints[e].keys = original_keys;
                    self.draft.providers[p].endpoints[e].selected_key = original_selection;
                }
            }
            Err(error) => {
                self.report(Err(error), "");
            }
        }
    }

    pub(super) fn apply_editor(&mut self, editor: &EditorDialog) -> Result<(), String> {
        match editor {
            EditorDialog::Provider { id, name } => {
                if !valid_name(name) {
                    return Err("名称应为 1–128 字符且不含控制字符".into());
                }
                if let Some(id) = id {
                    self.draft
                        .providers
                        .iter_mut()
                        .find(|p| p.id == *id)
                        .ok_or("服务商已不存在")?
                        .name = name.trim().into();
                } else {
                    let id = new_id()?;
                    self.draft.providers.push(ProviderEntry {
                        id: id.clone(),
                        name: name.trim().into(),
                        enabled: false,
                        endpoints: Vec::new(),
                    });
                    self.provider = Some(id);
                    self.endpoint = None;
                }
            }
            EditorDialog::Endpoint {
                provider,
                id,
                name,
                url,
                protocol,
            } => {
                if !valid_name(name) {
                    return Err("线路名称无效".into());
                }
                let url = validate_url(url)?;
                let p = self
                    .draft
                    .providers
                    .iter_mut()
                    .find(|p| p.id == *provider)
                    .ok_or("服务商已不存在")?;
                if let Some(id) = id {
                    let endpoint = p
                        .endpoints
                        .iter_mut()
                        .find(|e| e.id == *id)
                        .ok_or("线路已不存在")?;
                    endpoint.name = name.trim().into();
                    endpoint.base_url = url;
                    endpoint.protocol = *protocol;
                } else {
                    let id = new_id()?;
                    p.endpoints.push(EndpointEntry {
                        id: id.clone(),
                        name: name.trim().into(),
                        base_url: url,
                        protocol: *protocol,
                        keys: Vec::new(),
                        models: Vec::new(),
                        selected_key: None,
                        selected_model: None,
                    });
                    self.endpoint = Some(id);
                }
            }
            EditorDialog::Model {
                provider,
                endpoint,
                original,
                value,
                ..
            } => {
                let mut value = value.clone();
                value.manual = true;
                if original.is_none() {
                    value.id = value.id.trim().to_owned();
                }
                validate_model_id(&value.id)?;
                let endpoint = self
                    .draft
                    .providers
                    .iter_mut()
                    .find(|p| p.id == *provider)
                    .and_then(|p| p.endpoints.iter_mut().find(|e| e.id == *endpoint))
                    .ok_or("线路已不存在")?;
                value.settings.validate(endpoint.protocol)?;
                if original.as_ref().is_some_and(|id| id != &value.id) {
                    return Err("已保存模型的 ID 不可修改；请新增模型".into());
                }
                if endpoint
                    .models
                    .iter()
                    .any(|model| model.id == value.id && Some(&model.id) != original.as_ref())
                {
                    return Err("该模型 ID 已存在".into());
                }
                for field in [&mut value.settings.name, &mut value.settings.group_name] {
                    *field = field
                        .take()
                        .map(|text| text.trim().to_owned())
                        .filter(|text| !text.is_empty());
                }
                if let Some(original) = original {
                    let model = endpoint
                        .models
                        .iter_mut()
                        .find(|m| m.id == *original)
                        .ok_or("模型已不存在")?;
                    if endpoint.selected_model.as_ref() == Some(original) {
                        endpoint.selected_model = Some(value.id.clone());
                    }
                    *model = value;
                } else {
                    endpoint.models.push(value);
                }
                repair_model_selection(endpoint);
            }
            EditorDialog::Keys { .. } => {}
        }
        Ok(())
    }
}
