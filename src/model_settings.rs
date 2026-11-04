//! 设置草稿与已保存目录分开，聊天不直接读取设置控件。
use lumakit_support::secrets::{SavedKey, new_id};
use gpui_kit::{App, Context, Entity, Window};
mod view;
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
                        let types = job.config.endpoint.models.iter().map(|model| (model.id.clone(), model.settings.model_type)).collect();
                        self.sync_dialog = Some(SyncDialog { provider: job.config.provider, endpoint: job.config.endpoint.id, ids, selected, search: String::new(), types, filter: None, folded: HashSet::new() });
                    }
                }
                Err(error) => { self.report(Err(error), ""); }
            }
            return true;
        }
        false
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
    fn drop(&mut self) { self.clear_drafts(); if let Some(job) = self.job.take() { let _ = job.thread.join(); } }
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
mod tests {
    use super::*;
    #[test]
    fn display_name_and_model_type_change_picker_without_changing_wire_id() {
        let mut file = SettingsFile::default(); file.seed_presets();
        file.providers[0].enabled = true;
        let endpoint = &mut file.providers[0].endpoints[0];
        endpoint.models.push(ModelEntry { id: "wire-chat".into(), manual: true, capabilities: unknown_capabilities(),
            settings: lumakit_ai::model_config::ModelSettings { name: Some("显示名称".into()), group_name: Some("自定义分组".into()), ..Default::default() } });
        endpoint.models.push(ModelEntry { id: "image-model".into(), manual: true, capabilities: unknown_capabilities(),
            settings: lumakit_ai::model_config::ModelSettings { model_type: ModelType::Image, ..Default::default() } });
        let options = model_options(&file);
        assert_eq!(options.len(), 1); assert_eq!(options[0].name, "显示名称"); assert_eq!(options[0].model.model, "wire-chat");
        assert_eq!(file.providers[0].endpoints[0].models[0].settings.group_name("wire-chat"), "自定义分组");
        let images = model_options_for(&file, ModelType::Image);
        assert_eq!(images.iter().map(|option| option.model.model.as_str()).collect::<Vec<_>>(), ["image-model"]);
    }

    #[cfg(windows)]
    #[test]
    fn key_reveal_replace_and_failure_preserve_secret_contract() {
        let root = std::env::temp_dir().join(format!("lumadesk-inline-key-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::load(&root).unwrap();
        let key = SavedKey::new("inline-original").unwrap(); let id = key.id.clone();
        let endpoint = settings.draft.providers[0].endpoints[0].id.clone();
        settings.draft.providers[0].endpoints[0].keys.push(key); settings.save().unwrap();
        settings.reveal_key(0, 0, &id);
        assert_eq!(settings.key_drafts[&endpoint].revealed.as_ref().unwrap().as_str(), "inline-original");
        settings.reveal_key(0, 0, "missing");
        assert!(settings.key_drafts[&endpoint].revealed.is_none());
        let draft = settings.key_drafts.get_mut(&endpoint).unwrap(); draft.editing = Some(id.clone()); *draft.value = "replacement".into();
        let original_url = settings.saved.providers[0].endpoints[0].base_url.clone();
        settings.draft.providers[0].endpoints[0].base_url = "unfinished address".into(); settings.dirty = true;
        settings.save_key(0, 0);
        assert!(settings.key_drafts[&endpoint].value.is_empty() && settings.key_drafts[&endpoint].revealed.is_none());
        let restored = ModelSettings::load(&root).unwrap();
        assert_eq!(restored.saved.providers[0].endpoints[0].keys[0].id, id);
        assert_eq!(restored.saved.providers[0].endpoints[0].keys[0].open().unwrap().as_str(), "replacement");
        assert_eq!(restored.saved.providers[0].endpoints[0].base_url, original_url);
        assert_eq!(settings.draft.providers[0].endpoints[0].base_url, "unfinished address"); assert!(settings.dirty);
        let draft = settings.key_drafts.get_mut(&endpoint).unwrap(); draft.editing = Some(id.clone()); *draft.value = "uncommitted".into();
        let db = rusqlite::Connection::open(root.join("providers.db")).unwrap();
        db.execute_batch("CREATE TRIGGER deny_key BEFORE UPDATE ON endpoint_keys BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
        settings.save_key(0, 0);
        assert_eq!(settings.draft.providers[0].endpoints[0].keys[0].open().unwrap().as_str(), "replacement");
        assert_eq!(settings.key_drafts[&endpoint].value.as_str(), "uncommitted");
        settings.clear_drafts(); assert!(settings.key_drafts.is_empty());
        drop(db); drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_cancel_and_save_keep_other_endpoint() {
        let root = std::env::temp_dir().join(format!("lumadesk-editor-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::load(&root).unwrap();
        let provider = settings.saved.providers[0].id.clone();
        let endpoint = settings.saved.providers[0].endpoints[0].id.clone();
        let original = serde_json::to_value(settings.store.load().unwrap().0).unwrap();
        settings.editor = Some(EditorDialog::Endpoint { provider: provider.clone(), id: Some(endpoint.clone()), name: "edited".into(), url: "https://example.com/v1".into(), protocol: Protocol::OpenAiResponses });
        settings.clear_drafts();
        assert_eq!(serde_json::to_value(settings.store.load().unwrap().0).unwrap(), original);
        let before_other = serde_json::to_value(&settings.saved.providers[1]).unwrap();
        let model = EditorDialog::Model { provider, endpoint, original: None, value: ModelEntry { id: "manual-model".into(), capabilities: ModelCapabilities { streaming: true, tools: false, vision: true }, manual: true, settings: Default::default() } };
        settings.apply_editor(&model).unwrap(); settings.save().unwrap();
        let restored = ModelSettings::load(&root).unwrap();
        assert_eq!(restored.saved.providers[0].endpoints[0].models[0].id, "manual-model");
        assert!(restored.saved.providers[0].endpoints[0].models[0].capabilities.vision);
        assert_eq!(serde_json::to_value(&restored.saved.providers[1]).unwrap(), before_other);
        assert!(settings.apply_editor(&model).is_err());
        drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enable_is_independent_of_invalid_address_and_restores_on_write_failure() {
        let root = std::env::temp_dir().join(format!("lumadesk-enable-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::load(&root).unwrap();
        let original_url = settings.saved.providers[0].endpoints[0].base_url.clone();
        settings.draft.providers[0].endpoints[0].base_url = "unfinished".into(); settings.dirty = true;
        settings.set_enabled(0, true);
        assert!(settings.saved.providers[0].enabled);
        assert_eq!(settings.saved.providers[0].endpoints[0].base_url, original_url);
        assert_eq!(settings.draft.providers[0].endpoints[0].base_url, "unfinished");
        let restored = ModelSettings::load(&root).unwrap(); assert!(restored.saved.providers[0].enabled);
        let db = rusqlite::Connection::open(root.join("providers.db")).unwrap();
        db.execute_batch("CREATE TRIGGER deny_enable BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
        settings.set_enabled(0, false);
        assert!(settings.error && settings.saved.providers[0].enabled && settings.draft.providers[0].enabled);
        drop(db); drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sync_preserves_manual_identity_and_capabilities() {
        let mut models = vec![ModelEntry { id: "manual".into(), capabilities: ModelCapabilities { streaming: true, tools: true, vision: false }, manual: true, settings: Default::default() }];
        merge_models(&mut models, &["manual".into(), "remote".into()], &HashSet::from(["manual".into(), "remote".into()]));
        assert_eq!(models.len(), 2); assert_eq!(models[0].id, "manual"); assert!(models[0].capabilities.tools); assert!(!models[1].capabilities.streaming);
        assert!(parse_models(br#"{"data":[{"id":"a"},{"id":"a"},{"id":"b"}]}"#).is_ok_and(|ids| ids.len() == 2));
        assert!(parse_models(br#"{"data":[{"id":""}]}"#).is_err());
        assert!(validate_model_id(" bad ").is_err()); assert!(validate_model_id("line\nbreak").is_err());
    }
    #[test]
    fn empty_save_reload_and_url_conventions() {
        let root = std::env::temp_dir().join(format!("lumadesk-model-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::load(&root).unwrap();
        assert_eq!(settings.saved.providers.len(), 3);
        assert!(settings.saved.providers.iter().all(|p| !p.enabled && p.endpoints.len() == 1 && p.endpoints[0].keys.is_empty() && p.endpoints[0].models.is_empty()));
        settings.draft.providers.clear(); settings.save().unwrap();
        let restored = ModelSettings::load(&root).unwrap(); assert!(restored.saved.providers.is_empty()); assert!(restored.available_models().is_empty());
        assert_eq!(model_url("http://127.0.0.1:1234/v1", Protocol::OpenAiChat).unwrap(), "http://127.0.0.1:1234/v1/models");
        assert_eq!(model_url("http://[::1]:1234", Protocol::AnthropicMessages).unwrap(), "http://[::1]:1234/v1/models");
        assert!(validate_url("https://user:password@example.com").is_err()); assert!(validate_url("http://example.com").is_err());
        assert!(validate_url("https://:password@example.com").is_err());
        assert!(validate_url("https://example.com?key=hidden").is_err());
        let original = serde_json::to_value(settings.store.load().unwrap().0).unwrap();
        settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: String::new(), enabled: true, endpoints: Vec::new() });
        assert!(settings.save().is_err()); assert_eq!(serde_json::to_value(settings.store.load().unwrap().0).unwrap(), original); assert!(settings.saved.providers.is_empty());
        drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn existing_provider_survives_preset_initialization_and_restart() {
        let root = std::env::temp_dir().join(format!("lumadesk-preset-test-{}", new_id().unwrap()));
        let original = serde_json::json!({"providers": [{"id": "user-fenno", "name": "fenno", "enabled": true, "endpoints": []}]});
        lumakit_support::save_versioned(&root.join("model-settings.json"), VERSION, &original).unwrap();
        let mut settings = ModelSettings::load(&root).unwrap();
        assert_eq!(settings.saved.providers.len(), 3);
        assert_eq!(serde_json::to_value(&settings.saved.providers[0]).unwrap(), original["providers"][0]);
        settings.draft.providers.retain(|p| p.id != "preset-qiniu");
        settings.save().unwrap();
        let restored = ModelSettings::load(&root).unwrap();
        assert!(!restored.saved.providers.iter().any(|p| p.id == "preset-qiniu"));
        assert_eq!(restored.saved.providers.iter().filter(|p| p.name.eq_ignore_ascii_case("fenno")).count(), 1);
        drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn actual_loopback_detection_auth_failover_and_encrypted_reload() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in [("401 Unauthorized", "{}"), ("200 OK", "{\"data\":[{\"id\":\"loopback-model\"}]}"), ("200 OK", "{\"data\":[{\"id\":\"loopback-model\"}]}"), ("200 OK", "{\"data\":[{\"id\":\"loopback-model\"}]}")] {
                let (mut socket, _) = listener.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap(); let mut bytes = Vec::new();
                loop { let mut part = [0u8; 2048]; let size = socket.read(&mut part).unwrap(); bytes.extend_from_slice(&part[..size]); if size == 0 || bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; } }
                requests.push(String::from_utf8(bytes).unwrap());
                write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        let first = SavedKey::new("first-key").unwrap(); let second = SavedKey::new("second-key").unwrap();
        let endpoint = EndpointEntry { id: new_id().unwrap(), name: "回环".into(), base_url: format!("http://{address}/v1"), protocol: Protocol::OpenAiChat, selected_key: Some(first.id.clone()), keys: vec![first, second], models: Vec::new(), selected_model: None };
        assert_eq!(detect_models(&endpoint, CancellationToken::new()).unwrap(), vec!["loopback-model"]);
        let mut messages = endpoint.clone(); messages.protocol = Protocol::AnthropicMessages; messages.base_url = format!("http://{address}"); messages.keys.remove(0); messages.selected_key = None;
        assert_eq!(detect_models(&messages, CancellationToken::new()).unwrap(), vec!["loopback-model"]);
        let mut responses = messages.clone(); responses.protocol = Protocol::OpenAiResponses; responses.base_url = format!("http://{address}/custom/v1");
        assert_eq!(detect_models(&responses, CancellationToken::new()).unwrap(), vec!["loopback-model"]);
        let cancelled = CancellationToken::new(); cancelled.cancel(); assert!(detect_models(&responses, cancelled).is_err());
        let requests = server.join().unwrap(); assert!(requests[0].starts_with("GET /v1/models ")); assert!(requests[0].to_ascii_lowercase().contains("authorization: bearer first-key")); assert!(requests[1].to_ascii_lowercase().contains("authorization: bearer second-key"));
        assert!(requests[2].starts_with("GET /v1/models ")); assert!(requests[2].to_ascii_lowercase().contains("x-api-key: second-key")); assert!(requests[2].to_ascii_lowercase().contains("anthropic-version: 2023-06-01"));
        assert!(requests[3].starts_with("GET /custom/v1/models ")); assert!(requests[3].to_ascii_lowercase().contains("authorization: bearer second-key"));
        let encoded = serde_json::to_string(&endpoint).unwrap(); assert!(!encoded.contains("first-key")); let restored: EndpointEntry = serde_json::from_str(&encoded).unwrap(); assert_eq!(restored.keys[0].id, endpoint.keys[0].id); assert_eq!(restored.keys[0].open().unwrap().as_str(), "first-key");
        let root = std::env::temp_dir().join(format!("lumadesk-model-keys-{}", new_id().unwrap()));
        let mut settings = ModelSettings::load(&root).unwrap();
        settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: "回环测试".into(), enabled: true, endpoints: vec![endpoint] });
        settings.save().unwrap();
        let restored = ModelSettings::load(&root).unwrap();
        let restored_endpoint = &restored.saved.providers.last().unwrap().endpoints[0];
        assert_eq!(restored_endpoint.keys[0].id, settings.saved.providers.last().unwrap().endpoints[0].keys[0].id);
        let selected = lumakit_ai::ModelRef { provider: settings.saved.providers.last().unwrap().id.clone(), endpoint: restored_endpoint.id.clone(), model: "loopback-model".into(), protocol: restored_endpoint.protocol };
        assert!(settings.connection(&selected).is_err());
        settings.draft.providers.last_mut().unwrap().endpoints[0].models.push(ModelEntry { id: selected.model.clone(), capabilities: unknown_capabilities(), manual: true, settings: Default::default() });
        assert!(settings.available_models().is_empty());
        settings.save().unwrap();
        assert_eq!(settings.available_models().len(), 1);
        assert_eq!(settings.connection(&selected).unwrap().api_key.unwrap().as_str(), "first-key");
        settings.draft.providers.last_mut().unwrap().enabled = false;
        settings.save().unwrap();
        assert!(settings.connection(&selected).is_err());
        let bytes = std::fs::read(root.join("providers.db")).unwrap(); assert!(!bytes.windows(b"first-key".len()).any(|part| part == b"first-key"));
        drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bounded_response_and_pending_request_cancellation() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0u8; 2048]; let _ = socket.read(&mut request).unwrap();
                if attempt == 0 {
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BODY + 1).unwrap();
                } else {
                    accepted_tx.send(()).unwrap();
                    let _ = release_rx.recv_timeout(Duration::from_secs(5));
                }
            }
        });
        let endpoint = EndpointEntry { id: "test".into(), name: "回环".into(), base_url: format!("http://{address}"), protocol: Protocol::OpenAiChat, keys: Vec::new(), models: Vec::new(), selected_key: None, selected_model: None };
        assert!(detect_models(&endpoint, CancellationToken::new()).unwrap_err().contains("2 MiB"));
        let cancel = CancellationToken::new(); let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || detect_models(&endpoint, worker_cancel));
        accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        cancel.cancel();
        assert!(worker.join().unwrap().unwrap_err().contains("取消"));
        release_tx.send(()).unwrap(); server.join().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn anthropic_two_page_catalog_uses_same_origin_and_auth() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in [r#"{"data":[{"id":"page-one"}],"has_more":true,"last_id":"page-one"}"#, r#"{"data":[{"id":"page-two"}],"has_more":false,"last_id":"page-two"}"#] {
                let (mut socket, _) = listener.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new();
                loop { let mut part = [0u8; 2048]; let size = socket.read(&mut part).unwrap(); bytes.extend_from_slice(&part[..size]); if size == 0 || bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; } }
                requests.push(String::from_utf8(bytes).unwrap());
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        let endpoint = EndpointEntry { id: "pagination-test".into(), name: "分页".into(), base_url: format!("http://{address}"), protocol: Protocol::AnthropicMessages, keys: vec![SavedKey::new("page-key").unwrap()], models: Vec::new(), selected_key: None, selected_model: None };
        assert_eq!(detect_models(&endpoint, CancellationToken::new()).unwrap(), vec!["page-one", "page-two"]);
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /v1/models HTTP/1.1"));
        assert!(requests[1].starts_with("GET /v1/models?after_id=page-one HTTP/1.1"));
        for request in requests { let request = request.to_ascii_lowercase(); assert!(request.contains("x-api-key: page-key")); assert!(request.contains("anthropic-version: 2023-06-01")); assert!(request.contains(&format!("host: {address}"))); }
        assert!(model_page(br#"{"data":[],"has_more":true,"last_id":"a"}"#, Protocol::AnthropicMessages).is_err());
        assert!(model_page(br#"{"data":[{"id":"a"}],"has_more":true}"#, Protocol::AnthropicMessages).is_err());
    }
    #[test]
    fn unauthorized_without_saved_key_says_save_the_key_first() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            loop { let mut part = [0u8; 2048]; let size = socket.read(&mut part).unwrap(); bytes.extend_from_slice(&part[..size]); if size == 0 || bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; } }
            write!(socket, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let endpoint = EndpointEntry { id: "no-key".into(), name: "无密钥".into(), base_url: format!("http://{address}/v1"), protocol: Protocol::OpenAiChat, keys: Vec::new(), models: Vec::new(), selected_key: None, selected_model: None };
        let error = detect_models(&endpoint, CancellationToken::new()).unwrap_err();
        assert!(error.contains("没有已保存的密钥") && error.contains("401"), "{error}");
        assert!(!server.join().unwrap().to_ascii_lowercase().contains("authorization:"));
    }
}
