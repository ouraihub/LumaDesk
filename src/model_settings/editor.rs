//! Provider, endpoint and credential draft editing.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;
impl ModelSettings {
    pub(super) fn set_key_enabled(&mut self, p: usize, e: usize, id: &str, enabled: bool) {
        if self.job.is_some() {
            return;
        }
        let (trace, root) = TraceContext::root("settings.keys.enabled", "ui");
        let _entered = root.span().clone().entered();
        let index = self.draft.providers[p].endpoints[e]
            .keys
            .iter()
            .position(|key| key.id == id);
        let result = if let Some(index) = index {
            let before = self.draft.providers[p].endpoints[e].keys[index].enabled;
            self.draft.providers[p].endpoints[e].keys[index].enabled = enabled;
            let result = self.persist_keys(&trace, p, e);
            if result.is_err() {
                self.draft.providers[p].endpoints[e].keys[index].enabled = before;
            }
            result
        } else {
            Err(Failure::failed("SETTINGS_TARGET_MISSING", "密钥已不存在"))
        };
        self.report_task(&trace, &result, "密钥启用状态已保存");
        task::end(root, &result);
    }
    pub(super) fn key_target(&self) -> Option<(usize, usize)> {
        if let Some(EditorDialog::Keys { provider, endpoint }) = &self.editor {
            let p = self
                .draft
                .providers
                .iter()
                .position(|p| &p.id == provider)?;
            let e = self.draft.providers[p]
                .endpoints
                .iter()
                .position(|e| &e.id == endpoint)?;
            Some((p, e))
        } else if self.editor.is_none() {
            self.indices().and_then(|(p, e)| e.map(|e| (p, e)))
        } else {
            None
        }
    }

    pub(super) fn submit_key(&mut self) -> bool {
        if self.job.is_some() {
            return false;
        }
        let Some((p, e)) = self.key_target() else {
            return false;
        };
        let id = self.draft.providers[p].endpoints[e].id.clone();
        let Some(draft) = self.key_drafts.get_mut(&id) else {
            return false;
        };
        if draft.value.trim().is_empty() && draft.editing.is_none() {
            return false;
        }
        draft.submitted = true;
        self.save_key(p, e);
        !self.error
    }
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
        let (trace, root) = TraceContext::root("settings.keys.reveal", "ui");
        let _entered = root.span().clone().entered();
        let endpoint = &self.draft.providers[p].endpoints[e];
        let result = endpoint
            .keys
            .iter()
            .find(|key| key.id == key_id)
            .ok_or_else(|| Failure::failed("SETTINGS_TARGET_MISSING", "密钥已不存在"))
            .and_then(|key| {
                task::sync(
                    &trace,
                    "settings.keys.secret.open",
                    "SETTINGS_SECRET_FAILED",
                    || key.open().map_err(|_| "无法解密密钥；原凭据未修改".into()),
                )
            });
        let draft = self.key_drafts.entry(endpoint.id.clone()).or_default();
        draft.revealed = None;
        draft.revealed_key = None;
        self.report_task(&trace, &result, "已显示密钥");
        let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
        if let Ok(secret) = result {
            let draft = self
                .key_drafts
                .entry(self.draft.providers[p].endpoints[e].id.clone())
                .or_default();
            draft.revealed = Some(secret);
            draft.revealed_key = Some(key_id.into());
        }
        task::end(root, &outcome);
    }

    pub(super) fn save_key(&mut self, p: usize, e: usize) {
        let (trace, root) = TraceContext::root("settings.keys.save", "ui");
        let _entered = root.span().clone().entered();
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let original_keys = self.draft.providers[p].endpoints[e].keys.clone();
        let original_selection = self.draft.providers[p].endpoints[e].selected_key.clone();
        let result = (|| {
            let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
            let value = Zeroizing::new(draft.value.trim().to_owned());
            if !value.is_empty() && (value.len() > 4096 || value.chars().any(char::is_control)) {
                return Err(Failure::failed(
                    "SETTINGS_INPUT_INVALID",
                    "密钥不能含控制字符，且不能超过 4096 字节",
                ));
            }
            let editing = draft.editing.clone();
            let label = normalize_key_label(&draft.label)
                .map_err(|message| Failure::failed("SETTINGS_INPUT_INVALID", message))?
                .map(str::to_owned);
            let endpoint = &mut self.draft.providers[p].endpoints[e];
            if let Some(id) = editing {
                let key = endpoint
                    .keys
                    .iter_mut()
                    .find(|key| key.id == id)
                    .ok_or_else(|| {
                        Failure::failed("SETTINGS_TARGET_MISSING", "待修改密钥已不存在")
                    })?;
                if !value.is_empty() {
                    task::sync(
                        &trace,
                        "settings.keys.secret.replace",
                        "SETTINGS_SECRET_FAILED",
                        || {
                            key.replace(&value)
                                .map_err(|_| "无法加密密钥；原凭据未修改".into())
                        },
                    )?;
                }
                key.label = label;
            } else {
                if value.is_empty() {
                    return Err(Failure::failed(
                        "SETTINGS_INPUT_INVALID",
                        "密钥不能为空，原凭据未修改",
                    ));
                }
                if endpoint.keys.len() >= MAX_KEYS {
                    return Err(Failure::failed(
                        "SETTINGS_INPUT_INVALID",
                        "每条线路最多保存 16 个密钥",
                    ));
                }
                let mut key = task::sync(
                    &trace,
                    "settings.keys.secret.create",
                    "SETTINGS_SECRET_FAILED",
                    || SavedKey::new(&value).map_err(|_| "无法加密密钥；原凭据未修改".into()),
                )?;
                key.label = label;
                if endpoint.selected_key.is_none() {
                    endpoint.selected_key = Some(key.id.clone());
                }
                endpoint.keys.push(key);
            }
            self.persist_keys(&trace, p, e)
        })();
        if self.report_task(&trace, &result, "密钥已加密保存") {
            let draft = self.key_drafts.get_mut(&endpoint_id).unwrap();
            draft.value.zeroize();
            draft.label.clear();
            draft.editing = None;
            draft.revealed = None;
            draft.revealed_key = None;
        } else {
            self.draft.providers[p].endpoints[e].keys = original_keys;
            self.draft.providers[p].endpoints[e].selected_key = original_selection;
        }
        task::end(root, &result);
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
