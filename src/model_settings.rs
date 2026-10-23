//! 设置草稿与已保存目录分开，聊天不直接读取设置控件。
use lumakit_support::secrets::{SavedKey, new_id};
use eframe::egui;
use futures_util::StreamExt;
use lumakit_ai::{AiConnection, Protocol};
use lumakit_ai::providers::ModelCapabilities;
use lumakit_ai::catalog::{EndpointEntry, ModelEntry, ProviderEntry, ModelCatalog, validate_url};
use lumakit_ui::chat::ModelOption;
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, sync::mpsc, thread::JoinHandle, time::Duration};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

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
struct KeyDraft { value: Zeroizing<String>, editing: Option<String>, revealed: Option<Zeroizing<String>> }
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
}

pub struct ModelSettings {
    path: PathBuf,
    saved: SettingsFile,
    draft: SettingsFile,
    catalog: ModelCatalog,
    options: Vec<ModelOption>,
    provider: Option<String>,
    endpoint: Option<String>,
    search: String,
    key_drafts: HashMap<String, KeyDraft>,
    model_input: String,
    notice: String,
    error: bool,
    dirty: bool,
    delete_provider: Option<String>,
    delete_endpoint: Option<String>,
    sync_dialog: Option<SyncDialog>,
    job: Option<Job>,
}

impl ModelSettings {
    pub fn new(root: &Path) -> Result<Self, String> {
        let path = root.join("model-settings.json");
        let mut saved = match lumakit_support::load_versioned::<SettingsFile>(&path, VERSION)
            .map_err(|_| "无法读取 model-settings.json；请检查文件权限或恢复备份，原文件未修改".to_owned())? {
            None => SettingsFile::default(),
            Some((VERSION, value)) => value,
            Some(_) => return Err("模型设置版本不支持，原文件未修改".into()),
        };
        project(&saved)?;
        if !saved.presets_initialized {
            saved.seed_presets();
            project(&saved)?;
            lumakit_support::save_versioned(&path, VERSION, &saved)
                .map_err(|_| "无法保存初始服务商预设；请检查目录权限或磁盘空间".to_owned())?;
        }
        let mut state = Self {
            path, draft: saved.clone(), catalog: ModelCatalog::new(saved.providers.clone()), options: model_options(&saved), saved, provider: None, endpoint: None,
            search: String::new(), key_drafts: HashMap::new(), model_input: String::new(),
            notice: String::new(), error: false, dirty: false,
            delete_provider: None, delete_endpoint: None, sync_dialog: None, job: None,
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
        lumakit_support::save_versioned(&self.path, VERSION, &self.draft)
            .map_err(|_| "保存失败；请检查目录权限或磁盘空间。已保存配置未变，草稿仍保留，可重试".to_owned())?;
        self.saved = self.draft.clone();
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.dirty = false;
        Ok(())
    }
    fn commit(&mut self) { let result = self.save(); self.report(result, "已保存"); }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.drain(ui.ctx());
        ui.heading("模型服务");
        ui.label("地址按服务商要求填写基础路径（例如 /v1）；检测仅统计，获取模型后自行选择同步。");
        let busy = self.job.is_some();
        if busy {
            ui.horizontal(|ui| { ui.spinner(); ui.label("正在请求已保存的线路…"); if ui.button("取消请求").clicked() { if let Some(job) = &self.job { job.cancel.cancel(); } } });
        }
        egui::ScrollArea::vertical().id_salt("models-content").show(ui, |ui| {
            ui.add_enabled_ui(!busy && self.sync_dialog.is_none() && self.delete_provider.is_none() && self.delete_endpoint.is_none(), |ui| {
                if ui.available_width() >= 650.0 {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| { ui.set_width((ui.available_width() * 0.28).clamp(150.0, 250.0)); self.providers_ui(ui); });
                        ui.separator();
                        ui.vertical(|ui| { self.detail_ui(ui); });
                    });
                } else {
                    self.providers_ui(ui);
                    ui.separator();
                    self.detail_ui(ui);
                }
            });
            if self.dirty { ui.label("有未保存的草稿；保存成功前不会用于运行时请求。"); }
            if !self.notice.is_empty() { ui.label(egui::RichText::new(&self.notice).color(if self.error { ui.visuals().error_fg_color } else { ui.visuals().text_color() })); }
        });
        self.confirmations(ui.ctx());
        self.sync_ui(ui.ctx());
    }

    fn providers_ui(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("搜索服务商"));
        let query = self.search.trim().to_lowercase();
        let rows: Vec<_> = self.draft.providers.iter().filter(|p| p.name.to_lowercase().contains(&query)).map(|p| (p.id.clone(), p.name.clone(), p.enabled)).collect();
        if rows.is_empty() { ui.label(if self.draft.providers.is_empty() { "尚无服务商，请添加" } else { "没有匹配的服务商" }); }
        for (id, name, enabled) in rows {
            ui.push_id(egui::Id::new(&id), |ui| {
                if ui.selectable_label(self.provider.as_ref() == Some(&id), format!("{} {}", if enabled { "●" } else { "○" }, name)).clicked() {
                    self.provider = Some(id); self.repair_selection(); self.model_input.clear();
                }
            });
        }
        if ui.button("＋ 添加服务商").clicked() {
            match new_id() {
                Ok(id) => {
                    self.draft.providers.push(ProviderEntry { id: id.clone(), name: "新服务商".into(), enabled: true, endpoints: Vec::new() });
                    self.provider = Some(id); self.endpoint = None; self.dirty = true;
                }
                Err(e) => { self.report(Err(e), ""); }
            }
        }
    }

    fn detail_ui(&mut self, ui: &mut egui::Ui) {
        let Some((p, e)) = self.indices() else { return; };
        let mut commit = false;
        ui.horizontal_wrapped(|ui| {
            let response = ui.text_edit_singleline(&mut self.draft.providers[p].name);
            self.dirty |= response.changed(); commit |= finished(&response, ui);
            let response = ui.checkbox(&mut self.draft.providers[p].enabled, "启用");
            self.dirty |= response.changed(); commit |= response.changed();
            if ui.button("删除服务商").clicked() { self.delete_provider = self.provider.clone(); }
        });
        if let Some((_, _, _, link)) = PRESETS.iter().find(|(id, _, _, _)| *id == self.draft.providers[p].id) {
            ui.hyperlink_to("获取密钥", *link);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("线路");
            egui::ComboBox::from_id_salt("model-endpoints").selected_text(e.map(|e| self.draft.providers[p].endpoints[e].name.as_str()).unwrap_or("未配置线路")).show_ui(ui, |ui| {
                for endpoint in &self.draft.providers[p].endpoints {
                    if ui.selectable_value(&mut self.endpoint, Some(endpoint.id.clone()), &endpoint.name).changed() { self.model_input.clear(); }
                }
            });
            if ui.button("＋ 添加线路").clicked() {
                match new_id() {
                    Ok(id) => {
                        self.draft.providers[p].endpoints.push(EndpointEntry { id: id.clone(), name: "新线路".into(), base_url: String::new(), protocol: Protocol::OpenAiChat, keys: Vec::new(), models: Vec::new(), selected_key: None, selected_model: None });
                        self.endpoint = Some(id); self.dirty = true;
                    }
                    Err(error) => { self.report(Err(error), ""); }
                }
            }
        });
        let Some((p, Some(e))) = self.indices() else {
            if ui.button("保存服务商").clicked() || commit { self.commit(); }
            return;
        };
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        ui.push_id(&endpoint_id, |ui| {
            let endpoint = &mut self.draft.providers[p].endpoints[e];
            ui.label("线路名称");
            let response = ui.text_edit_singleline(&mut endpoint.name); self.dirty |= response.changed(); commit |= finished(&response, ui);
            ui.label("API 基础地址");
            let response = ui.add(egui::TextEdit::singleline(&mut endpoint.base_url).hint_text("https://服务商地址/v1").desired_width(f32::INFINITY));
            self.dirty |= response.changed(); commit |= finished(&response, ui);
            let before = endpoint.protocol;
            egui::ComboBox::from_id_salt("protocol").selected_text(protocol_name(endpoint.protocol)).show_ui(ui, |ui| {
                for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses, Protocol::AnthropicMessages] { ui.selectable_value(&mut endpoint.protocol, protocol, protocol_name(protocol)); }
            });
            if endpoint.protocol != before { self.dirty = true; commit = true; }
            match model_url(&endpoint.base_url, endpoint.protocol) { Ok(url) => { ui.label(format!("GET {url}")); } Err(_) => { ui.label("填写合法基础地址后显示模型列表请求地址"); } }
            ui.horizontal_wrapped(|ui| {
                if ui.button("保存线路").clicked() { commit = true; }
                if ui.button("删除线路").clicked() { self.delete_endpoint = Some(endpoint_id.clone()); }
            });
        });
        if commit && self.dirty { self.commit(); }
        ui.separator();
        self.keys_ui(ui, p, e);
        ui.separator();
        self.models_ui(ui, p, e);
        ui.horizontal_wrapped(|ui| {
            if ui.button("检测").clicked() { self.start_job(false); }
            if ui.button("同步模型…").clicked() { self.start_job(true); }
            if ui.button("保存全部").clicked() { self.commit(); }
        });
    }

    fn keys_ui(&mut self, ui: &mut egui::Ui, p: usize, e: usize) {
        ui.label("API 密钥（每条线路最多 16 个，加密保存）");
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let mut remove = None;
        let mut reveal = None;
        let mut copy = None;
        let mut edit = None;
        let mut changed = false;
        let rows: Vec<_> = self.draft.providers[p].endpoints[e].keys.iter().map(|key| (key.id.clone(), key.masked.clone())).collect();
        for (id, masked) in rows {
            ui.push_id(&id, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let endpoint = &mut self.draft.providers[p].endpoints[e];
                    changed |= ui.selectable_value(&mut endpoint.selected_key, Some(id.clone()), masked).changed();
                    if ui.button("修改").clicked() { edit = Some(id.clone()); }
                    if ui.button("显示").clicked() { reveal = Some(id.clone()); }
                    if ui.button("复制").clicked() { copy = Some(id.clone()); }
                    if ui.button("删除").clicked() { remove = Some(id.clone()); }
                });
            });
        }
        if let Some(id) = edit { self.key_drafts.entry(endpoint_id.clone()).or_default().editing = Some(id); }
        for (id, copying) in reveal.into_iter().map(|id| (id, false)).chain(copy.into_iter().map(|id| (id, true))) {
            let result = self.draft.providers[p].endpoints[e].keys.iter().find(|key| key.id == id).ok_or_else(|| "密钥已不存在".to_owned()).and_then(SavedKey::open);
            match result { Ok(secret) => { if copying { ui.ctx().copy_text(secret.to_string()); self.notice = "已复制密钥到系统剪贴板；请留意其他程序访问剪贴板".into(); } else { self.key_drafts.entry(endpoint_id.clone()).or_default().revealed = Some(secret); } }, Err(error) => { self.report(Err(error), ""); } }
        }
        if let Some(id) = remove {
            self.draft.providers[p].endpoints[e].keys.retain(|key| key.id != id);
            if self.draft.providers[p].endpoints[e].selected_key.as_ref() == Some(&id) { self.draft.providers[p].endpoints[e].selected_key = self.draft.providers[p].endpoints[e].keys.first().map(|key| key.id.clone()); }
            changed = true;
        }
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
        if let Some(secret) = &draft.revealed { ui.label(secret.as_str()); if ui.button("隐藏密钥").clicked() { draft.revealed = None; } }
        ui.label(if draft.editing.is_some() { "修改密钥：空草稿不会清空原密钥" } else { "添加密钥" });
        ui.add(egui::TextEdit::singleline(&mut *draft.value).password(true).hint_text("输入密钥；失败时保留草稿").desired_width(f32::INFINITY));
        let apply = ui.button(if draft.editing.is_some() { "保存修改后的密钥" } else { "保存新密钥" }).clicked();
        if draft.editing.is_some() && ui.button("取消修改").clicked() { draft.editing = None; draft.value.zeroize(); }
        if apply {
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
                    self.dirty = true;
                    let result = self.save();
                    if self.report(result, "密钥已加密保存") {
                        let draft = self.key_drafts.get_mut(&endpoint_id).unwrap(); draft.value.zeroize(); draft.editing = None;
                    } else {
                        self.draft.providers[p].endpoints[e].keys = original_keys;
                        self.draft.providers[p].endpoints[e].selected_key = original_selection;
                    }
                }
                Err(error) => { self.report(Err(error), ""); }
            }
        }
        if changed { self.dirty = true; self.commit(); }
    }

    fn models_ui(&mut self, ui: &mut egui::Ui, p: usize, e: usize) {
        ui.label("模型与明确能力（检测只获取 ID，不猜测流式、工具或视觉能力）");
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let mut remove = None;
        let mut commit = false;
        let count = self.draft.providers[p].endpoints[e].models.len();
        for i in 0..count {
            ui.push_id((&endpoint_id, i), |ui| {
                let endpoint = &mut self.draft.providers[p].endpoints[e];
                let model = &mut endpoint.models[i];
                let old_id = model.id.clone();
                ui.horizontal_wrapped(|ui| {
                    if ui.selectable_label(endpoint.selected_model.as_ref() == Some(&model.id), "默认").clicked() { endpoint.selected_model = Some(model.id.clone()); self.dirty = true; commit = true; }
                    let response = ui.text_edit_singleline(&mut model.id); self.dirty |= response.changed(); commit |= finished(&response, ui);
                    if response.changed() { model.manual = true; }
                    if endpoint.selected_model.as_ref() == Some(&old_id) && old_id != model.id { endpoint.selected_model = Some(model.id.clone()); }
                    for (value, label) in [(&mut model.capabilities.streaming, "流式"), (&mut model.capabilities.tools, "工具"), (&mut model.capabilities.vision, "视觉")] {
                        let response = ui.checkbox(value, label); self.dirty |= response.changed(); commit |= response.changed();
                    }
                    if ui.button("移除").clicked() { remove = Some(i); }
                });
            });
        }
        if let Some(i) = remove {
            let endpoint = &mut self.draft.providers[p].endpoints[e];
            endpoint.models.remove(i); repair_model_selection(endpoint); self.dirty = true; commit = true;
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.model_input).hint_text("手动输入真实模型 ID"));
            if ui.button("＋ 添加模型").clicked() {
                let id = self.model_input.trim().to_owned();
                let endpoint = &mut self.draft.providers[p].endpoints[e];
                let result = validate_model_id(&id).and_then(|_| if endpoint.models.iter().any(|m| m.id == id) { Err("该模型 ID 已存在".into()) } else { Ok(()) });
                match result {
                    Ok(()) => { endpoint.models.push(ModelEntry { id, capabilities: unknown_capabilities(), manual: true }); repair_model_selection(endpoint); self.model_input.clear(); self.dirty = true; commit = true; }
                    Err(error) => { self.report(Err(error), ""); }
                }
            }
        });
        if commit { self.commit(); }
    }

    fn confirmations(&mut self, ctx: &egui::Context) {
        if self.delete_provider.is_none() && self.delete_endpoint.is_none() { return; }
        egui::Window::new("确认删除").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(if self.delete_provider.is_some() { "将永久删除此服务商及全部线路、模型与密钥。" } else { "将永久删除此线路及模型与密钥。" });
            ui.horizontal(|ui| {
                if ui.button("确认删除").clicked() {
                    if let Some(id) = self.delete_provider.take() {
                        if let Some(provider) = self.draft.providers.iter().find(|p| p.id == id) { for endpoint in &provider.endpoints { self.key_drafts.remove(&endpoint.id); } }
                        self.draft.providers.retain(|p| p.id != id);
                    }
                    if let Some(id) = self.delete_endpoint.take() { for p in &mut self.draft.providers { p.endpoints.retain(|e| e.id != id); } self.key_drafts.remove(&id); }
                    self.dirty = true; self.commit(); self.repair_selection();
                }
                if ui.button("取消").clicked() { self.delete_provider = None; self.delete_endpoint = None; }
            });
        });
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
    fn drain(&mut self, ctx: &egui::Context) {
        let result = self.job.as_ref().and_then(|job| match job.rx.try_recv() { Ok(result) => Some(result), Err(mpsc::TryRecvError::Disconnected) => Some(Err("模型请求线程异常结束，请重试".into())), Err(mpsc::TryRecvError::Empty) => None });
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            let cancelled = job.cancel.is_cancelled();
            let _ = job.thread.join();
            if cancelled { self.report(Err("请求已取消；原模型列表未修改".into()), ""); return; }
            match result {
                Ok(ids) => {
                    self.notice = format!("检测到 {} 个模型；原模型列表未修改", ids.len()); self.error = false;
                    if job.sync {
                        let selected = job.config.endpoint.models.iter().map(|m| m.id.clone()).collect();
                        self.sync_dialog = Some(SyncDialog { provider: job.config.provider, endpoint: job.config.endpoint.id, ids, selected, search: String::new() });
                    }
                }
                Err(error) => { self.report(Err(error), ""); }
            }
        }
        if self.job.is_some() { ctx.request_repaint_after(Duration::from_millis(80)); }
    }
    fn sync_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.sync_dialog.as_mut() else { return; };
        let mut apply = false; let mut close = false;
        egui::Window::new("选择同步模型").collapsible(false).show(ctx, |ui| {
            ui.label("＋ 加入 / − 移除检测模型；手工添加的模型和已有能力始终保留。");
            ui.add(egui::TextEdit::singleline(&mut dialog.search).hint_text("搜索模型 ID"));
            let query = dialog.search.to_lowercase();
            egui::ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
                for id in dialog.ids.iter().filter(|id| id.to_lowercase().contains(&query)) {
                    ui.push_id(id, |ui| {
                        ui.horizontal(|ui| { let selected = dialog.selected.contains(id); if ui.button(if selected { "−" } else { "＋" }).clicked() { if selected { dialog.selected.remove(id); } else { dialog.selected.insert(id.clone()); } } ui.label(id); });
                    });
                }
            });
            ui.horizontal(|ui| { apply = ui.button("确认并保存").clicked(); close = ui.button("取消").clicked(); });
        });
        if apply {
            let dialog = self.sync_dialog.as_ref().unwrap();
            if let Some(endpoint) = self.draft.providers.iter_mut().find(|p| p.id == dialog.provider).and_then(|p| p.endpoints.iter_mut().find(|e| e.id == dialog.endpoint)) {
                merge_models(&mut endpoint.models, &dialog.ids, &dialog.selected);
                repair_model_selection(endpoint); self.dirty = true;
                let result = self.save(); if self.report(result, "模型同步已保存") { self.sync_dialog = None; }
            } else { self.report(Err("目标线路已不存在，未同步".into()), ""); }
        }
        if close { self.sync_dialog = None; }
    }

    pub fn clear_secrets(&mut self) {
        self.key_drafts.clear();
        self.sync_dialog = None;
        if let Some(job) = &self.job { job.cancel.cancel(); }
    }
    pub fn available_models(&self) -> &[ModelOption] { &self.options }
    pub fn connection(&self, model: &lumakit_ai::ModelRef) -> Result<AiConnection, String> { self.catalog.resolve(model) }
}
impl Drop for ModelSettings {
    fn drop(&mut self) { self.clear_secrets(); if let Some(job) = self.job.take() { let _ = job.thread.join(); } }
}

fn finished(response: &egui::Response, ui: &egui::Ui) -> bool { response.lost_focus() || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) }
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
            for model in &endpoint.models { validate_model_id(&model.id)?; if !model_ids.insert(&model.id) { return Err("同一线路不能重复添加模型 ID".into()); } }
        }
    }
    Ok(())
}

fn model_options(file: &SettingsFile) -> Vec<ModelOption> {
    file.providers.iter().filter(|p| p.enabled).flat_map(|p| p.endpoints.iter().flat_map(move |e| e.models.iter().map(move |m| ModelOption {
        model: lumakit_ai::ModelRef { provider: p.id.clone(), endpoint: e.id.clone(), model: m.id.clone(), protocol: e.protocol },
        name: m.id.clone(), provider: p.name.clone(), endpoint: e.name.clone(),
    }))).collect()
}
fn repair_model_selection(endpoint: &mut EndpointEntry) {
    if !endpoint.models.iter().any(|m| Some(&m.id) == endpoint.selected_model.as_ref()) { endpoint.selected_model = endpoint.models.first().map(|m| m.id.clone()); }
}
fn merge_models(models: &mut Vec<ModelEntry>, discovered: &[String], selected: &HashSet<String>) {
    models.retain(|m| m.manual || !discovered.contains(&m.id) || selected.contains(&m.id));
    for id in discovered { if selected.contains(id) && !models.iter().any(|m| &m.id == id) { models.push(ModelEntry { id: id.clone(), capabilities: unknown_capabilities(), manual: false }); } }
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
            Err(format!("全部密钥请求失败：HTTP {}（401/403 检查密钥权限，429 检查额度或稍后重试）", last_status.unwrap_or(401)))
        };
        let operation = futures_util::future::select(Box::pin(cancel.cancelled()), Box::pin(tokio::time::timeout(Duration::from_secs(45), operation))).await;
        match operation { futures_util::future::Either::Left(_) => Err("请求已取消，原模型未修改".into()), futures_util::future::Either::Right((result, _)) => result.map_err(|_| "模型请求总时限已到，原模型未修改".to_owned())? }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_preserves_manual_identity_and_capabilities() {
        let mut models = vec![ModelEntry { id: "manual".into(), capabilities: ModelCapabilities { streaming: true, tools: true, vision: false }, manual: true }];
        merge_models(&mut models, &["manual".into(), "remote".into()], &HashSet::from(["remote".into()]));
        assert_eq!(models.len(), 2); assert_eq!(models[0].id, "manual"); assert!(models[0].capabilities.tools); assert!(!models[1].capabilities.streaming);
        assert!(parse_models(br#"{"data":[{"id":"a"},{"id":"a"},{"id":"b"}]}"#).is_ok_and(|ids| ids.len() == 2));
        assert!(parse_models(br#"{"data":[{"id":""}]}"#).is_err());
        assert!(validate_model_id(" bad ").is_err()); assert!(validate_model_id("line\nbreak").is_err());
    }
    #[test]
    fn empty_save_reload_and_url_conventions() {
        let root = std::env::temp_dir().join(format!("lumadesk-model-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::new(&root).unwrap();
        assert_eq!(settings.saved.providers.len(), 3);
        assert!(settings.saved.providers.iter().all(|p| !p.enabled && p.endpoints.len() == 1 && p.endpoints[0].keys.is_empty() && p.endpoints[0].models.is_empty()));
        settings.draft.providers.clear(); settings.save().unwrap();
        let restored = ModelSettings::new(&root).unwrap(); assert!(restored.saved.providers.is_empty()); assert!(restored.available_models().is_empty());
        assert_eq!(model_url("http://127.0.0.1:1234/v1", Protocol::OpenAiChat).unwrap(), "http://127.0.0.1:1234/v1/models");
        assert_eq!(model_url("http://[::1]:1234", Protocol::AnthropicMessages).unwrap(), "http://[::1]:1234/v1/models");
        assert!(validate_url("https://user:password@example.com").is_err()); assert!(validate_url("http://example.com").is_err());
        assert!(validate_url("https://:password@example.com").is_err());
        assert!(validate_url("https://example.com?key=hidden").is_err());
        let original = std::fs::read(root.join("model-settings.json")).unwrap();
        settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: String::new(), enabled: true, endpoints: Vec::new() });
        assert!(settings.save().is_err()); assert_eq!(std::fs::read(root.join("model-settings.json")).unwrap(), original); assert!(settings.saved.providers.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn existing_provider_survives_preset_initialization_and_restart() {
        let root = std::env::temp_dir().join(format!("lumadesk-preset-test-{}", new_id().unwrap()));
        let original = serde_json::json!({"providers": [{"id": "user-fenno", "name": "fenno", "enabled": true, "endpoints": []}]});
        lumakit_support::save_versioned(&root.join("model-settings.json"), VERSION, &original).unwrap();
        let mut settings = ModelSettings::new(&root).unwrap();
        assert_eq!(settings.saved.providers.len(), 3);
        assert_eq!(serde_json::to_value(&settings.saved.providers[0]).unwrap(), original["providers"][0]);
        settings.draft.providers.retain(|p| p.id != "preset-qiniu");
        settings.save().unwrap();
        let restored = ModelSettings::new(&root).unwrap();
        assert!(!restored.saved.providers.iter().any(|p| p.id == "preset-qiniu"));
        assert_eq!(restored.saved.providers.iter().filter(|p| p.name.eq_ignore_ascii_case("fenno")).count(), 1);
        std::fs::remove_dir_all(root).unwrap();
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
        let mut settings = ModelSettings::new(&root).unwrap();
        settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: "回环测试".into(), enabled: true, endpoints: vec![endpoint] });
        settings.save().unwrap();
        let restored = ModelSettings::new(&root).unwrap();
        let restored_endpoint = &restored.saved.providers.last().unwrap().endpoints[0];
        assert_eq!(restored_endpoint.keys[0].id, settings.saved.providers.last().unwrap().endpoints[0].keys[0].id);
        let selected = lumakit_ai::ModelRef { provider: settings.saved.providers.last().unwrap().id.clone(), endpoint: restored_endpoint.id.clone(), model: "loopback-model".into(), protocol: restored_endpoint.protocol };
        assert!(settings.connection(&selected).is_err());
        settings.draft.providers.last_mut().unwrap().endpoints[0].models.push(ModelEntry { id: selected.model.clone(), capabilities: unknown_capabilities(), manual: true });
        assert!(settings.available_models().is_empty());
        settings.save().unwrap();
        assert_eq!(settings.available_models().len(), 1);
        assert_eq!(settings.connection(&selected).unwrap().api_key.unwrap().as_str(), "first-key");
        settings.draft.providers.last_mut().unwrap().enabled = false;
        settings.save().unwrap();
        assert!(settings.connection(&selected).is_err());
        let bytes = std::fs::read_to_string(root.join("model-settings.json")).unwrap(); assert!(!bytes.contains("first-key"));
        std::fs::remove_dir_all(root).unwrap();
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
}
