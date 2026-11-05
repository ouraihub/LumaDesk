//! 设置草稿与已保存目录分开，聊天不直接读取设置控件。
use lumakit_support::secrets::{SavedKey, new_id};
use gpui_kit::{App, Context, Entity, Window};
mod view;
mod catalog_refresh;
use futures_util::StreamExt;
use lumakit_ai::{AiConnection, Protocol};
use lumakit_ai::providers::ModelCapabilities;
use lumakit_ai::catalog::{EndpointEntry, ModelEntry, ProviderEntry, ModelCatalog, validate_url};
use lumakit_ai::catalog_store::CatalogStore;
use lumakit_ai::model_config::ModelType;
use lumakit_ai::chat::ModelOption;
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, path::Path, sync::mpsc, thread::JoinHandle, time::Duration};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

#[cfg(test)]
const VERSION: u32 = 1;
const MAX_KEYS: usize = 16;
const MAX_BODY: usize = 2 * 1024 * 1024;

const PRESETS: [(&str, &str, &str, &str); 3] = [
    ("preset-qiniu", "七牛云", "https://api.qnaigc.com", "https://s.qiniu.com/BfyeU3"),
    ("preset-siliconflow", "硅基流动", "https://api.siliconflow.cn/v1", "https://cloud.siliconflow.cn/i/ieP7r6Bx"),
    ("preset-fenno", "Fenno", "https://api.fenno.ai/v1", "https://api.fenno.ai/s/vWbM"),
];

#[derive(Clone, Default, Serialize, Deserialize)]
struct SettingsFile {
    providers: Vec<ProviderEntry>,
    #[serde(default)]
    presets_initialized: bool,
}

impl SettingsFile {
    fn seed_presets(&mut self) {
        if self.presets_initialized { return; }
        for (id, name, base_url, _) in PRESETS {
            if self.providers.iter().any(|p| p.id == id || p.name.eq_ignore_ascii_case(name)) { continue; }
            self.providers.push(ProviderEntry {
                id: id.into(), name: name.into(), enabled: false,
                endpoints: vec![EndpointEntry {
                    id: format!("{id}-main"), name: "默认线路".into(), base_url: base_url.into(),
                    protocol: Protocol::OpenAiChat, keys: Vec::new(), models: Vec::new(),
                    selected_key: None, selected_model: None,
                }],
            });
        }
        // 删除预设后重启不应重新出现；不能用“列表为空”判断是否首次初始化。
        self.presets_initialized = true;
    }
}

#[derive(Default)]
struct KeyDraft { value: Zeroizing<String>, editing: Option<String>, revealed: Option<Zeroizing<String>>, revealed_key: Option<String> }
#[derive(Clone)]
struct DetectConfig { provider: String, endpoint: EndpointEntry }
struct Job {
    cancel: CancellationToken,
    thread: JoinHandle<()>,
    rx: mpsc::Receiver<Result<Vec<String>, String>>,
    config: DetectConfig,
    sync: bool,
}
struct SyncDialog {
    provider: String,
    endpoint: String,
    ids: Vec<String>,
    selected: HashSet<String>,
    search: String,
    types: HashMap<String, ModelType>,
    filter: Option<ModelType>,
    folded: HashSet<String>,
}

enum EditorDialog {
    Provider { id: Option<String>, name: String },
    Endpoint { provider: String, id: Option<String>, name: String, url: String, protocol: Protocol },
    Model { provider: String, endpoint: String, original: Option<String>, value: ModelEntry },
    Keys { provider: String, endpoint: String },
}

pub struct ModelSettings {
    store: CatalogStore,
    saved: SettingsFile,
    draft: SettingsFile,
    catalog: ModelCatalog,
    options: Vec<ModelOption>,
    image_options: Vec<ModelOption>,
    provider: Option<String>,
    endpoint: Option<String>,
    search: String,
    key_drafts: HashMap<String, KeyDraft>,
    notice: String,
    error: bool,
    dirty: bool,
    delete_provider: Option<String>,
    delete_endpoint: Option<String>,
    delete_key: Option<(String, String, String)>,
    sync_dialog: Option<SyncDialog>,
    job: Option<Job>,
    catalog_refresh: Option<catalog_refresh::CatalogRefreshJob>,
    catalog_refresh_notice: String,
    provider_filter: u8,
    model_query: String,
    capability_filter: u8,
    folded: HashSet<String>,
    editor: Option<EditorDialog>,
    view: Option<view::ModelView>,
}

impl ModelSettings {
    fn load(root: &Path) -> Result<Self, String> {
        let mut store = CatalogStore::open(root)?;
        let (providers, presets_initialized) = store.load()?;
        let mut saved = SettingsFile { providers, presets_initialized };
        project(&saved)?;
        if !saved.presets_initialized {
            saved.seed_presets();
            project(&saved)?;
            store.save(&saved.providers, saved.presets_initialized)?;
        }
        let mut state = Self {
            store, draft: saved.clone(), catalog: ModelCatalog::new(saved.providers.clone()), options: model_options(&saved), image_options: model_options_for(&saved, ModelType::Image), saved, provider: None, endpoint: None,
            search: String::new(), key_drafts: HashMap::new(),
            notice: String::new(), error: false, dirty: false,
            delete_provider: None, delete_endpoint: None, delete_key: None, sync_dialog: None, job: None,
            catalog_refresh: None, catalog_refresh_notice: String::new(),
            provider_filter: 0, model_query: String::new(), capability_filter: 0, folded: HashSet::new(), editor: None,
            view: None,
        };
        state.repair_selection();
        Ok(state)
    }

    fn indices(&self) -> Option<(usize, Option<usize>)> {
        let p = self.draft.providers.iter().position(|p| Some(&p.id) == self.provider.as_ref())?;
        let e = self.draft.providers[p].endpoints.iter().position(|e| Some(&e.id) == self.endpoint.as_ref());
        Some((p, e))
    }
    fn repair_selection(&mut self) {
        if !self.draft.providers.iter().any(|p| Some(&p.id) == self.provider.as_ref()) {
            self.provider = self.draft.providers.first().map(|p| p.id.clone());
        }
        let selected = self.draft.providers.iter().find(|p| Some(&p.id) == self.provider.as_ref());
        if !selected.is_some_and(|p| p.endpoints.iter().any(|e| Some(&e.id) == self.endpoint.as_ref())) {
            self.endpoint = selected.and_then(|p| p.endpoints.first()).map(|e| e.id.clone());
        }
    }
    fn report(&mut self, result: Result<(), String>, success: &str) -> bool {
        match result {
            Ok(()) => { self.notice = success.into(); self.error = false; true }
            Err(error) => { self.notice = error; self.error = true; false }
        }
    }
    fn save(&mut self) -> Result<(), String> {
        project(&self.draft)?;
        self.store.save(&self.draft.providers, self.draft.presets_initialized)?;
        self.saved = self.draft.clone();
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        self.dirty = false;
        Ok(())
    }
    fn persist_keys(&mut self, p: usize, e: usize) -> Result<(), String> {
        let endpoint = &self.draft.providers[p].endpoints[e];
        let saved = self.saved.providers.iter_mut().find(|provider| provider.id == self.draft.providers[p].id)
            .and_then(|provider| provider.endpoints.iter_mut().find(|saved| saved.id == endpoint.id)).ok_or("线路已不存在")?;
        self.store.save_keys(&endpoint.id, &endpoint.keys, endpoint.selected_key.as_deref())?;
        saved.keys.clone_from(&endpoint.keys);
        saved.selected_key.clone_from(&endpoint.selected_key);
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        Ok(())
    }
    fn commit(&mut self) { let result = self.save(); self.report(result, "已保存"); }


    fn set_enabled(&mut self, p: usize, enabled: bool) {
        let id = self.draft.providers[p].id.clone();
        let mut candidate = self.saved.clone();
        let Some(provider) = candidate.providers.iter_mut().find(|provider| provider.id == id) else { return; };
        provider.enabled = enabled;
        let result = self.store.set_enabled(&id, enabled);
        if self.report(result, "已保存") {
            self.saved = candidate;
            self.draft.providers[p].enabled = enabled;
            self.catalog = ModelCatalog::new(self.saved.providers.clone());
            self.options = model_options(&self.saved);
            self.image_options = model_options_for(&self.saved, ModelType::Image);
        } else {
            self.draft.providers[p].enabled = self.saved.providers.iter().find(|provider| provider.id == id).is_some_and(|provider| provider.enabled);
        }
    }

    fn open_endpoint(&mut self, p: usize, e: Option<usize>) {
        let endpoint = e.map(|e| &self.draft.providers[p].endpoints[e]);
        self.editor = Some(EditorDialog::Endpoint {
            provider: self.draft.providers[p].id.clone(), id: endpoint.map(|e| e.id.clone()),
            name: endpoint.map_or_else(|| "新线路".into(), |e| e.name.clone()),
            url: endpoint.map_or_else(String::new, |e| e.base_url.clone()),
            protocol: endpoint.map_or(Protocol::OpenAiChat, |e| e.protocol),
        });
    }

    fn reveal_key(&mut self, p: usize, e: usize, key_id: &str) {
        let endpoint = &self.draft.providers[p].endpoints[e];
        let result = endpoint.keys.iter().find(|key| key.id == key_id).ok_or_else(|| "密钥已不存在".to_owned()).and_then(SavedKey::open);
        let endpoint_id = endpoint.id.clone();
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default(); draft.revealed = None; draft.revealed_key = None;
        match result {
            Ok(secret) => { let draft = self.key_drafts.entry(endpoint_id).or_default(); draft.revealed = Some(secret); draft.revealed_key = Some(key_id.into()); }
            Err(error) => { self.report(Err(error), ""); }
        }
    }

    fn save_key(&mut self, p: usize, e: usize) {
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
        let value = Zeroizing::new(draft.value.trim().to_owned());
        let editing = draft.editing.clone();
        let original_keys = self.draft.providers[p].endpoints[e].keys.clone();
        let original_selection = self.draft.providers[p].endpoints[e].selected_key.clone();
        let endpoint = &mut self.draft.providers[p].endpoints[e];
        let result = if value.is_empty() { Err("密钥不能为空，原凭据未修改".into()) }
            else if let Some(id) = editing {
                endpoint.keys.iter_mut().find(|key| key.id == id).ok_or_else(|| "待修改密钥已不存在".to_owned()).and_then(|key| key.replace(&value))
            } else if endpoint.keys.len() >= MAX_KEYS { Err("每条线路最多保存 16 个密钥".into()) }
            else { SavedKey::new(&value).map(|key| { if endpoint.selected_key.is_none() { endpoint.selected_key = Some(key.id.clone()); } endpoint.keys.push(key); }) };
        match result {
            Ok(()) => {
                let result = self.persist_keys(p, e);
                if self.report(result, "密钥已加密保存") {
                    let draft = self.key_drafts.get_mut(&endpoint_id).unwrap(); draft.value.zeroize(); draft.editing = None; draft.revealed = None; draft.revealed_key = None;
                } else {
                    self.draft.providers[p].endpoints[e].keys = original_keys;
                    self.draft.providers[p].endpoints[e].selected_key = original_selection;
                }
            }
            Err(error) => { self.report(Err(error), ""); }
        }
    }

    fn apply_editor(&mut self, editor: &EditorDialog) -> Result<(), String> {
        match editor {
            EditorDialog::Provider { id, name } => {
                if !valid_name(name) { return Err("名称应为 1–128 字符且不含控制字符".into()); }
                if let Some(id) = id {
                    self.draft.providers.iter_mut().find(|p| p.id == *id).ok_or("服务商已不存在")?.name = name.trim().into();
                } else {
                    let id = new_id()?;
                    self.draft.providers.push(ProviderEntry { id: id.clone(), name: name.trim().into(), enabled: false, endpoints: Vec::new() });
                    self.provider = Some(id); self.endpoint = None;
                }
            }
            EditorDialog::Endpoint { provider, id, name, url, protocol } => {
                if !valid_name(name) { return Err("线路名称无效".into()); }
                let url = validate_url(url)?;
                let p = self.draft.providers.iter_mut().find(|p| p.id == *provider).ok_or("服务商已不存在")?;
                if let Some(id) = id {
                    let endpoint = p.endpoints.iter_mut().find(|e| e.id == *id).ok_or("线路已不存在")?;
                    endpoint.name = name.trim().into(); endpoint.base_url = url; endpoint.protocol = *protocol;
                } else {
                    let id = new_id()?;
                    p.endpoints.push(EndpointEntry { id: id.clone(), name: name.trim().into(), base_url: url, protocol: *protocol,
                        keys: Vec::new(), models: Vec::new(), selected_key: None, selected_model: None });
                    self.endpoint = Some(id);
                }
            }
            EditorDialog::Model { provider, endpoint, original, value, .. } => {
                let mut value = value.clone(); value.manual = true;
                if original.is_none() { value.id = value.id.trim().to_owned(); }
                validate_model_id(&value.id)?;
                let endpoint = self.draft.providers.iter_mut().find(|p| p.id == *provider)
                    .and_then(|p| p.endpoints.iter_mut().find(|e| e.id == *endpoint)).ok_or("线路已不存在")?;
                value.settings.validate(endpoint.protocol)?;
                if original.as_ref().is_some_and(|id| id != &value.id) { return Err("已保存模型的 ID 不可修改；请新增模型".into()); }
                if endpoint.models.iter().any(|model| model.id == value.id && Some(&model.id) != original.as_ref()) { return Err("该模型 ID 已存在".into()); }
                for field in [&mut value.settings.name, &mut value.settings.group_name] {
                    *field = field.take().map(|text| text.trim().to_owned()).filter(|text| !text.is_empty());
                }
                if let Some(original) = original {
                    let model = endpoint.models.iter_mut().find(|m| m.id == *original).ok_or("模型已不存在")?;
                    if endpoint.selected_model.as_ref() == Some(original) { endpoint.selected_model = Some(value.id.clone()); }
                    *model = value;
                } else { endpoint.models.push(value); }
                repair_model_selection(endpoint);
            }
            EditorDialog::Keys { .. } => {}
        }
        Ok(())
    }

    fn start_job(&mut self, sync: bool) {
        if self.job.is_some() { return; }
        if self.dirty { self.report(Err("请先成功保存当前草稿，再使用已保存配置请求模型列表".into()), ""); return; }
        let Some((p, Some(e))) = self.indices() else { return; };
        let provider_id = &self.draft.providers[p].id;
        let endpoint_id = &self.draft.providers[p].endpoints[e].id;
        let Some(provider) = self.saved.providers.iter().find(|p| &p.id == provider_id) else { return; };
        let Some(endpoint) = provider.endpoints.iter().find(|e| &e.id == endpoint_id) else { return; };
        let config = DetectConfig { provider: provider.id.clone(), endpoint: endpoint.clone() };
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let worker_config = config.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || { let result = detect_models(&worker_config.endpoint, worker_cancel); let _ = tx.send(result); });
        self.job = Some(Job { cancel, thread, rx, config, sync });
        self.notice = "正在获取模型列表…".into(); self.error = false;
    }
    pub fn poll(&mut self) -> bool {
        let catalog_changed = self.poll_catalog_refresh();
        let result = self.job.as_ref().and_then(|job| match job.rx.try_recv() { Ok(result) => Some(result), Err(mpsc::TryRecvError::Disconnected) => Some(Err("模型请求线程异常结束，请重试".into())), Err(mpsc::TryRecvError::Empty) => None });
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            let cancelled = job.cancel.is_cancelled();
            let _ = job.thread.join();
            if cancelled { self.report(Err("请求已取消；原模型列表未修改".into()), ""); return true; }
            match result {
                Ok(ids) => {
                    self.notice = format!("检测到 {} 个模型；原模型列表未修改", ids.len()); self.error = false;
                    if job.sync {
                        let selected = job.config.endpoint.models.iter().map(|m| m.id.clone()).collect();
                        // /models returns ids only: saved models keep their type, new ones get a name-based guess the user can change.
                        let mut types: HashMap<String, ModelType> = ids.iter().map(|id| (id.clone(), ModelType::infer(id))).collect();
                        types.extend(job.config.endpoint.models.iter().map(|model| (model.id.clone(), model.settings.model_type)));
                        self.sync_dialog = Some(SyncDialog { provider: job.config.provider, endpoint: job.config.endpoint.id, ids, selected, search: String::new(), types, filter: None, folded: HashSet::new() });
                    }
                }
                Err(error) => { self.report(Err(error), ""); }
            }
            return true;
        }
        catalog_changed
    }

    fn clear_drafts(&mut self) {
        self.key_drafts.clear();
        self.sync_dialog = None;
        self.editor = None;
        self.delete_key = None;
        if let Some(job) = &self.job { job.cancel.cancel(); }
    }
    pub fn available_models(&self) -> &[ModelOption] { &self.options }
    pub fn image_models(&self) -> &[ModelOption] { &self.image_options }
    pub fn model_settings(&self, model: &lumakit_ai::ModelRef) -> Option<&lumakit_ai::model_config::ModelSettings> {
        self.saved.providers.iter().find(|p| p.enabled && p.id == model.provider)?
            .endpoints.iter().find(|e| e.id == model.endpoint && e.protocol == model.protocol)?
            .models.iter().find(|m| m.id == model.model).map(|m| &m.settings)
    }
    pub fn image_connection(&self, model: &lumakit_ai::ModelRef) -> Result<AiConnection, String> { self.catalog.resolve_for(model, ModelType::Image) }
    pub fn connection(&self, model: &lumakit_ai::ModelRef) -> Result<AiConnection, String> { self.catalog.resolve(model) }
}
impl Drop for ModelSettings {
    fn drop(&mut self) { self.cancel_catalog_refresh(); self.clear_drafts(); if let Some(job) = self.job.take() { let _ = job.thread.join(); } }
}


fn protocol_name(protocol: Protocol) -> &'static str { match protocol { Protocol::OpenAiChat => "OpenAI Chat", Protocol::OpenAiResponses => "OpenAI Responses", Protocol::AnthropicMessages => "Anthropic Messages" } }
fn unknown_capabilities() -> ModelCapabilities { ModelCapabilities { streaming: false, tools: false, vision: false } }
fn validate_model_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() || id.len() > 512 || id != id.trim() || id.chars().any(char::is_control) { Err("模型 ID 不能为空、含控制字符或首尾空白，长度最多 512 字节".into()) } else { Ok(()) }
}
fn model_url(base: &str, protocol: Protocol) -> Result<String, String> {
    let base = validate_url(base)?;
    let mut url = reqwest::Url::parse(&base).map_err(|_| "地址无效".to_owned())?;
    let path = url.path().trim_end_matches('/');
    let path = if protocol == Protocol::AnthropicMessages && !path.split('/').any(|segment| segment == "v1") { format!("{path}/v1/models") } else { format!("{path}/models") };
    url.set_path(&path); Ok(url.to_string())
}
fn valid_name(name: &str) -> bool { !name.trim().is_empty() && name.chars().count() <= 128 && !name.chars().any(char::is_control) }
fn project(file: &SettingsFile) -> Result<(), String> {
    let mut ids = HashSet::new();
    if file.providers.len() > 128 { return Err("最多保存 128 个服务商".into()); }
    for provider in &file.providers {
        if provider.id.is_empty() || provider.id.len() > 128 || !ids.insert(provider.id.clone()) || !valid_name(&provider.name) || provider.endpoints.len() > 64 { return Err("服务商名称应为 1–128 字符且不含控制字符；每个服务商最多 64 条线路，配置标识必须唯一".into()); }
        for endpoint in &provider.endpoints {
            if endpoint.id.is_empty() || endpoint.id.len() > 128 || !ids.insert(endpoint.id.clone()) || !valid_name(&endpoint.name) || endpoint.models.len() > 10000 { return Err("线路名称应为 1–128 字符且不含控制字符；每条线路最多 10000 个模型，配置标识必须唯一".into()); }
            validate_url(&endpoint.base_url)?;
            if endpoint.keys.len() > MAX_KEYS { return Err("每条线路最多保存 16 个密钥".into()); }
            for key in &endpoint.keys { if key.id.is_empty() || !ids.insert(key.id.clone()) || key.ciphertext.is_empty() { return Err("密钥元数据无效；原配置未覆盖".into()); } }
            let mut model_ids = HashSet::new();
            for model in &endpoint.models { validate_model_id(&model.id)?; model.settings.validate(endpoint.protocol)?; if !model_ids.insert(&model.id) { return Err("同一线路不能重复添加模型 ID".into()); } }
        }
    }
    Ok(())
}

fn model_options(file: &SettingsFile) -> Vec<ModelOption> { model_options_for(file, ModelType::Chat) }
fn model_options_for(file: &SettingsFile, kind: ModelType) -> Vec<ModelOption> {
    file.providers.iter().filter(|p| p.enabled).flat_map(|p| p.endpoints.iter().flat_map(move |e| e.models.iter().filter(move |m| m.settings.model_type == kind).map(move |m| ModelOption {
        model: lumakit_ai::ModelRef { provider: p.id.clone(), endpoint: e.id.clone(), model: m.id.clone(), protocol: e.protocol },
        name: m.settings.display_name(&m.id).to_owned(), provider: p.name.clone(), endpoint: e.name.clone(),
        settings: m.effective_settings(e.protocol), has_key: e.keys.iter().any(|key| !key.ciphertext.is_empty()),
    }))).collect()
}
fn repair_model_selection(endpoint: &mut EndpointEntry) {
    if !endpoint.models.iter().any(|m| Some(&m.id) == endpoint.selected_model.as_ref()) { endpoint.selected_model = endpoint.models.first().map(|m| m.id.clone()); }
}
fn merge_models(models: &mut Vec<ModelEntry>, discovered: &[String], selected: &HashSet<String>) {
    models.retain(|m| !discovered.contains(&m.id) || selected.contains(&m.id));
    for id in discovered { if selected.contains(id) && !models.iter().any(|m| &m.id == id) { models.push(ModelEntry { id: id.clone(), capabilities: unknown_capabilities(), manual: false, settings: Default::default() }); } }
}
fn parse_models(body: &[u8]) -> Result<Vec<String>, String> {
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| "响应不是有效 JSON；原模型未修改".to_owned())?;
    let rows = json.get("data").and_then(serde_json::Value::as_array).ok_or_else(|| "响应缺少 data 模型列表；原模型未修改".to_owned())?;
    if rows.len() > 10000 { return Err("模型目录超过 10000 条限制，原模型未修改".into()); }
    let mut seen = HashSet::new(); let mut ids = Vec::new();
    for row in rows { let id = row.get("id").and_then(serde_json::Value::as_str).ok_or_else(|| "模型列表含无效 ID；原模型未修改".to_owned())?; validate_model_id(id)?; if seen.insert(id.to_owned()) { ids.push(id.to_owned()); } }
    Ok(ids)
}
fn model_page(body: &[u8], protocol: Protocol) -> Result<(Vec<String>, Option<String>), String> {
    let ids = parse_models(body)?;
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| "模型目录 JSON 无效".to_owned())?;
    let has_more = match json.get("has_more") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| "模型目录分页标记无效；原模型未修改".to_owned())?,
    };
    if !has_more { return Ok((ids, None)); }
    if protocol != Protocol::AnthropicMessages { return Err("此协议返回了不支持的分页格式；原模型未修改".into()); }
    if ids.is_empty() { return Err("模型目录分页为空但仍标记有下一页；原模型未修改".into()); }
    let cursor = json.get("last_id").and_then(serde_json::Value::as_str).ok_or_else(|| "模型目录分页缺少 last_id；原模型未修改".to_owned())?;
    validate_model_id(cursor)?;
    Ok((ids, Some(cursor.to_owned())))
}
fn detect_models(endpoint: &EndpointEntry, cancel: CancellationToken) -> Result<Vec<String>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "无法启动网络任务".to_owned())?;
    runtime.block_on(async {
        let url = model_url(&endpoint.base_url, endpoint.protocol)?;
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(20)).build().map_err(|_| "无法初始化 HTTPS 客户端".to_owned())?;
        let operation = async {
            // 每个密钥只尝试一轮目录；认证失败换下一密钥，从第一页重新开始。
            let attempts = endpoint.keys.len().max(1);
            let mut last_status = None;
            'keys: for offset in 0..attempts {
                if cancel.is_cancelled() { return Err("请求已取消".into()); }
                let first = endpoint.selected_key.as_ref().and_then(|id| endpoint.keys.iter().position(|k| &k.id == id)).unwrap_or(0);
                let secret = if endpoint.keys.is_empty() { None } else { Some(endpoint.keys[(first + offset) % endpoint.keys.len()].open()?) };
                let mut ids = Vec::new();
                let mut seen = HashSet::new();
                let mut cursors = HashSet::new();
                let mut after_id: Option<String> = None;
                for _ in 0..10000 {
                    // 只在本地生成的同源 URL 上添加游标，不跟随服务端链接或重定向。
                    let mut page_url = reqwest::Url::parse(&url).map_err(|_| "模型目录地址无效".to_owned())?;
                    if let Some(cursor) = &after_id { page_url.query_pairs_mut().append_pair("after_id", cursor); }
                    let mut request = client.get(page_url);
                    if endpoint.protocol == Protocol::AnthropicMessages { request = request.header("anthropic-version", "2023-06-01"); }
                    if let Some(secret) = &secret { request = if endpoint.protocol == Protocol::AnthropicMessages { request.header("x-api-key", secret.as_str()) } else { request.bearer_auth(secret.as_str()) }; }
                    let response = request.send().await.map_err(|error| if error.is_timeout() { "请求超时，请检查地址或网络".to_owned() } else { "连接失败，请检查地址、网络及证书".to_owned() })?;
                    let status = response.status().as_u16();
                    if !response.status().is_success() { last_status = Some(status); if matches!(status, 401 | 403 | 429) { continue 'keys; } return Err(format!("服务返回 HTTP {status}，请检查线路地址及服务状态")); }
                    if response.content_length().is_some_and(|length| length > MAX_BODY as u64) { return Err("模型目录超过 2 MiB 限制".into()); }
                    let mut stream = response.bytes_stream(); let mut body = Vec::new();
                    while let Some(chunk) = stream.next().await { let chunk = chunk.map_err(|_| "读取响应失败，原模型未修改".to_owned())?; if body.len() + chunk.len() > MAX_BODY { return Err("模型目录超过 2 MiB 限制".into()); } body.extend_from_slice(&chunk); }
                    let (page, next) = model_page(&body, endpoint.protocol)?;
                    for id in page { if seen.insert(id.clone()) { ids.push(id); } }
                    if ids.len() > 10000 { return Err("模型目录总数超过 10000 条限制；原模型未修改".into()); }
                    let Some(next) = next else { return Ok(ids); };
                    if !cursors.insert(next.clone()) { return Err("模型目录分页游标重复；原模型未修改".into()); }
                    after_id = Some(next);
                }
                return Err("模型目录分页次数超过限制；原模型未修改".into());
            }
            let status = last_status.unwrap_or(401);
            // Detection reads only saved settings; a key still in the input box is not sent.
            if endpoint.keys.is_empty() { return Err(format!("此线路还没有已保存的密钥，服务返回 HTTP {status}；请先点“加密保存密钥”再检测")); }
            Err(format!("全部密钥请求失败：HTTP {status}（401/403 检查密钥权限，429 检查额度或稍后重试）"))
        };
        let operation = futures_util::future::select(Box::pin(cancel.cancelled()), Box::pin(tokio::time::timeout(Duration::from_secs(45), operation))).await;
        match operation { futures_util::future::Either::Left(_) => Err("请求已取消，原模型未修改".into()), futures_util::future::Either::Right((result, _)) => result.map_err(|_| "模型请求总时限已到，原模型未修改".to_owned())? }
    })
}

#[cfg(test)]
mod tests;
