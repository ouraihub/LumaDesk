//! 设置草稿与已保存目录分开，聊天不直接读取设置控件。
use lumakit_support::secrets::{SavedKey, new_id};
use eframe::egui;
use egui::AtomExt;
use crate::theme::tokens as t;
use futures_util::StreamExt;
use lumakit_ai::{AiConnection, Protocol};
use lumakit_ai::providers::ModelCapabilities;
use lumakit_ai::catalog::{EndpointEntry, ModelEntry, ProviderEntry, ModelCatalog, validate_url};
use lumakit_ai::catalog_store::CatalogStore;
use lumakit_ai::model_config::ModelType;
use lumakit_ui::chat::ModelOption;
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
    Model { provider: String, endpoint: String, original: Option<String>, value: ModelEntry, state: crate::model_editor::EditorState },
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
    model_search_open: bool,
    capability_filter: u8,
    folded: HashSet<String>,
    editor: Option<EditorDialog>,
}

impl ModelSettings {
    pub fn new(root: &Path) -> Result<Self, String> {
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
            model_search_open: false,
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

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.drain(ui.ctx());
        ui.scope(|ui| {
            egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(t::SPACE_LG as i8).show(ui, |ui| {
                ui.set_width(ui.available_width().min(t::PAGE_MAX));
                ui.heading("模型服务");
                ui.add_space(t::SPACE_LG);
                egui::ScrollArea::vertical().id_salt("models-content").show(ui, |ui| {
                    ui.add_enabled_ui(self.job.is_none() && self.editor.is_none() && self.sync_dialog.is_none() && self.delete_provider.is_none() && self.delete_endpoint.is_none() && self.delete_key.is_none(), |ui| {
                        if ui.available_width() >= t::STACK_AT {
                            let width = ui.available_width().min(t::PAGE_MAX);
                            ui.horizontal_top(|ui| {
                                ui.allocate_ui_with_layout(egui::vec2(t::PROVIDER_WIDTH, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_width(t::PROVIDER_WIDTH); self.providers_ui(ui);
                                });
                                ui.add_space(t::COLUMN_GAP - t::SPACE_SM * 2.0);
                                ui.allocate_ui_with_layout(egui::vec2((width - t::PROVIDER_WIDTH - t::COLUMN_GAP).max(t::DETAIL_MIN), 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_width((width - t::PROVIDER_WIDTH - t::COLUMN_GAP).max(t::DETAIL_MIN)); self.detail_ui(ui);
                                });
                            });
                        } else {
                            self.providers_ui(ui); ui.add_space(t::SPACE_XL); self.detail_ui(ui);
                        }
                    });
                    if self.job.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner(); ui.label("正在检测已保存的线路…");
                            if ui.button("取消请求").clicked() { if let Some(job) = &self.job { job.cancel.cancel(); } }
                        });
                    }
                    if !self.notice.is_empty() {
                        ui.add_space(t::SPACE_MD);
                        ui.colored_label(if self.error { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() }, &self.notice);
                    }
                    if self.dirty && ui.button("重试保存修改").clicked() { self.commit(); }
                });
            });
        });
        self.editor_ui(ui.ctx());
        self.confirmations(ui.ctx());
        self.sync_ui(ui.ctx());
    }

    fn providers_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - ui.spacing().interact_size.x - t::SPACE_SM).max(t::CONTROL * 3.0);
            ui.add_sized([width, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut self.search).hint_text("搜索模型平台…").id_salt("provider-search"));
            ui.menu_button("≡", |ui| {
                for (value, label) in [(0, "全部平台"), (1, "已启用"), (2, "已停用")] {
                    if ui.selectable_value(&mut self.provider_filter, value, label).clicked() { ui.close(); }
                }
            }).response.on_hover_text("筛选服务商").accessible_name("筛选服务商");
        });
        ui.add_space(t::SPACE_XS);
        let query = self.search.trim().to_lowercase();
        let rows: Vec<_> = self.draft.providers.iter().filter(|p| p.name.to_lowercase().contains(&query)
            && (self.provider_filter == 0 || p.enabled == (self.provider_filter == 1)))
            .map(|p| (p.id.clone(), p.name.clone(), p.enabled)).collect();
        if rows.is_empty() { ui.weak("没有匹配的平台"); }
        for (id, name, enabled) in rows {
            ui.push_id(&id, |ui| {
                let selected = self.provider.as_ref() == Some(&id);
                let initials = name.chars().next().unwrap_or('?');
                let text = egui::RichText::new(format!("{initials}   {name}"))
                    .color(if enabled || selected { ui.visuals().text_color() } else { ui.visuals().weak_text_color() });
                let response = ui.add_sized([ui.available_width(), t::ROW], egui::Button::new(text.atom_grow(true).atom_align(egui::Align2::LEFT_CENTER))
                    .right_text(egui::RichText::new(if enabled { "●" } else { "" }).color(crate::theme::success_color(ui.visuals())))
                    .fill(if selected { ui.visuals().faint_bg_color } else { egui::Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE).truncate());
                if response.clicked() && self.provider.as_ref() != Some(&id) {
                    if self.dirty { self.report(Err("当前地址尚未保存，请修正或重试保存后再切换平台".into()), ""); }
                    else {
                        self.key_drafts.clear(); self.provider = Some(id.clone()); self.repair_selection();
                        self.model_query.clear(); self.notice.clear();
                    }
                }
            });
        }
        ui.add_space(t::SPACE_MD);
        if ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], egui::Button::new("＋ 新建服务商")).clicked() {
            self.editor = Some(EditorDialog::Provider { id: None, name: String::new() });
        }
    }

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

    fn detail_ui(&mut self, ui: &mut egui::Ui) {
        let Some((p, e)) = self.indices() else { ui.weak("选择或新建一个服务商，开始配置模型。"); return; };
        ui.horizontal(|ui| {
            let title_width = (ui.available_width() - t::CONTROL * 4.0 - t::SPACE_XL).max(t::CONTROL * 2.0);
            ui.allocate_ui_with_layout(egui::vec2(title_width, ui.spacing().interact_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&self.draft.providers[p].name).size(t::TITLE).strong()).truncate());
            });
            if icon_button(ui, "\u{E434}", "服务商设置").clicked() {
                self.editor = Some(EditorDialog::Provider { id: Some(self.draft.providers[p].id.clone()), name: self.draft.providers[p].name.clone() });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut enabled = self.draft.providers[p].enabled;
                if lumakit_egui::switch(ui, &mut enabled, "启用").changed() { self.set_enabled(p, enabled); }
            });
        });
        ui.add_space(t::SPACE_LG);
        let Some(e) = e else {
            ui.weak("尚未配置线路，添加 API 地址后即可管理密钥和模型。");
            if ui.button("＋ 添加线路").clicked() { self.open_endpoint(p, None); }
            return;
        };
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        ui.push_id(&endpoint_id, |ui| {
            ui.horizontal(|ui| {
                ui.strong("API 密钥");
                if let Some((_, _, _, link)) = PRESETS.iter().find(|(id, _, _, _)| *id == self.draft.providers[p].id) { ui.hyperlink_to("获取密钥", *link); }
            });
            ui.add_space(t::SPACE_XS);
            let mut save_key = false;
            let mut detect = false;
            ui.horizontal(|ui| {
                let width = (ui.available_width() - t::CONTROL * 3.0 - t::SPACE_LG).max(t::CONTROL * 2.0);
                let endpoint = &self.draft.providers[p].endpoints[e];
                if endpoint.keys.is_empty() {
                    let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
                    let response = ui.add_sized([width, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut *draft.value).password(true).hint_text("输入 API 密钥").id_salt("primary-key"));
                    save_key = finished(&response, ui) && !draft.value.trim().is_empty();
                } else {
                    let key = endpoint.selected_key.as_ref().and_then(|id| endpoint.keys.iter().find(|key| &key.id == id))
                        .unwrap_or(&endpoint.keys[0]);
                    let key_id = key.id.clone();
                    self.primary_key_ui(ui, p, e, &key_id, width);
                }
                if ui.button("\u{E2D6}").on_hover_text("密钥管理").accessible_name("密钥管理").clicked() {
                    self.editor = Some(EditorDialog::Keys { provider: self.draft.providers[p].id.clone(), endpoint: endpoint_id.clone() });
                }
                detect = ui.add_enabled(!self.dirty, egui::Button::new("检测")).clicked();
            });
            if save_key { self.save_key(p, e); }
            if detect && (!save_key || !self.error) { self.start_job(false); }
            ui.add_space(t::SPACE_LG);
            ui.horizontal_wrapped(|ui| {
                ui.strong("API 地址");
                if ui.link("添加线路").clicked() { self.open_endpoint(p, None); }
                if self.draft.providers[p].endpoints.len() > 1 {
                    egui::ComboBox::from_id_salt("endpoints").selected_text(&self.draft.providers[p].endpoints[e].name).show_ui(ui, |ui| {
                        for endpoint in &self.draft.providers[p].endpoints {
                            if ui.selectable_value(&mut self.endpoint, Some(endpoint.id.clone()), &endpoint.name).changed() { self.key_drafts.clear(); }
                        }
                    });
                }
            });
            ui.add_space(t::SPACE_XS);
            let mut commit = false;
            ui.horizontal(|ui| {
                let width = (ui.available_width() - ui.spacing().interact_size.x - t::SPACE_LG).max(t::CONTROL * 2.0);
                egui::Frame::new().stroke(ui.visuals().widgets.inactive.bg_stroke).fill(ui.visuals().extreme_bg_color)
                    .corner_radius(t::RADIUS).inner_margin(egui::Margin::symmetric(t::SPACE_XS as i8, 0)).show(ui, |ui| {
                        ui.set_width((width - t::SPACE_SM).max(0.0));
                        ui.horizontal(|ui| {
                            let input_width = (ui.available_width() - t::CONTROL_SMALL - t::SPACE_SM).max(0.0);
                            let response = ui.add_sized([input_width, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut self.draft.providers[p].endpoints[e].base_url)
                                .frame(egui::Frame::NONE).font(egui::TextStyle::Monospace).hint_text("https://api.example.com/v1").id_salt("base-url"));
                            self.dirty |= response.changed(); commit = finished(&response, ui) && self.dirty;
                            if ui.add_sized([t::CONTROL_SMALL, ui.spacing().interact_size.y], egui::Button::new(egui::RichText::new("\u{E1CA}").size(t::TITLE)).frame(false))
                                .on_hover_text("复制 API 地址").accessible_name("复制 API 地址").clicked() {
                                ui.ctx().copy_text(self.draft.providers[p].endpoints[e].base_url.clone());
                                self.notice = "已复制 API 地址".into();
                            }
                        });
                    });
                if icon_button(ui, "\u{E434}", "线路设置").clicked() { self.open_endpoint(p, Some(e)); }
            });
            if commit { self.commit(); }
            let endpoint = &self.draft.providers[p].endpoints[e];
            match model_url(&endpoint.base_url, endpoint.protocol) {
                Ok(url) => { ui.add(egui::Label::new(egui::RichText::new(format!("GET {url}")).monospace().size(t::TEXT_SMALL).weak()).truncate()).on_hover_text(url); }
                Err(error) => { ui.colored_label(ui.visuals().error_fg_color, error); }
            }
            ui.add_space(t::SPACE_XL);
            self.models_ui(ui, p, e);
        });
    }

    fn primary_key_ui(&mut self, ui: &mut egui::Ui, p: usize, e: usize, key_id: &str, width: f32) {
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let mut reveal = false;
        let mut copy = false;
        let mut edit = false;
        let mut delete = false;
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
        if draft.revealed_key.as_deref() != Some(key_id) { draft.revealed = None; draft.revealed_key = None; }
        let expanded = draft.revealed.is_some();
        let height = ui.spacing().interact_size.y;
        egui::Frame::new().stroke(ui.visuals().widgets.inactive.bg_stroke)
            .corner_radius(t::RADIUS).inner_margin(egui::Margin::symmetric(t::SPACE_XS as i8, 0)).show(ui, |ui| {
                ui.set_width((width - t::SPACE_SM).max(0.0));
                ui.horizontal(|ui| {
                    let button_width = t::CONTROL_SMALL;
                    let count = if expanded { 4.0 } else { 1.0 };
                    let text_width = (ui.available_width() - count * (button_width + t::SPACE_XS)).max(0.0);
                    ui.spacing_mut().item_spacing.x = t::SPACE_XS;
                    ui.allocate_ui_with_layout(egui::vec2(text_width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(text_width);
                        let text = draft.revealed.as_deref().map_or("••••••••", |value| value.as_str());
                        ui.add(egui::Label::new(egui::RichText::new(text).monospace()).truncate());
                    });
                    let button = |ui: &mut egui::Ui, icon, name| ui.add_sized([button_width, height],
                        egui::Button::new(egui::RichText::new(icon).size(t::TITLE)).frame(false))
                        .on_hover_text(name).accessible_name(name).clicked();
                    if expanded {
                        copy = button(ui, "\u{E1CA}", "复制密钥");
                        edit = button(ui, "\u{E3B4}", "修改密钥");
                        delete = button(ui, "\u{E4A6}", "删除密钥");
                    }
                    reveal = button(ui, if expanded { "\u{E224}" } else { "\u{E220}" }, if expanded { "隐藏密钥" } else { "显示密钥" });
                });
            });
        if reveal {
            if expanded { let draft = self.key_drafts.get_mut(&endpoint_id).unwrap(); draft.revealed = None; draft.revealed_key = None; }
            else { self.reveal_key(p, e, key_id); }
        }
        if copy {
            if let Some(secret) = self.key_drafts.get(&endpoint_id).and_then(|draft| draft.revealed.as_ref()) {
                ui.ctx().copy_text(secret.to_string()); self.notice = "已复制密钥到系统剪贴板；请留意其他程序访问剪贴板".into();
            }
        }
        if edit {
            let draft = self.key_drafts.get_mut(&endpoint_id).unwrap();
            draft.value.zeroize(); draft.editing = Some(key_id.into()); draft.revealed = None; draft.revealed_key = None;
            self.editor = Some(EditorDialog::Keys { provider: self.draft.providers[p].id.clone(), endpoint: endpoint_id.clone() });
        }
        if delete {
            self.key_drafts.remove(&endpoint_id);
            self.delete_key = Some((self.draft.providers[p].id.clone(), endpoint_id, key_id.into()));
        }
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
            match result { Ok(secret) => { if copying { ui.ctx().copy_text(secret.to_string()); self.notice = "已复制密钥到系统剪贴板；请留意其他程序访问剪贴板".into(); } else { let draft = self.key_drafts.entry(endpoint_id.clone()).or_default(); draft.revealed = Some(secret); draft.revealed_key = Some(id); } }, Err(error) => { self.report(Err(error), ""); } }
        }
        if let Some(id) = remove {
            self.draft.providers[p].endpoints[e].keys.retain(|key| key.id != id);
            if self.draft.providers[p].endpoints[e].selected_key.as_ref() == Some(&id) { self.draft.providers[p].endpoints[e].selected_key = self.draft.providers[p].endpoints[e].keys.first().map(|key| key.id.clone()); }
            changed = true;
        }
        let draft = self.key_drafts.entry(endpoint_id.clone()).or_default();
        if let Some(secret) = &draft.revealed { ui.label(secret.as_str()); if ui.button("隐藏密钥").clicked() { draft.revealed = None; draft.revealed_key = None; } }
        ui.label(if draft.editing.is_some() { "修改密钥：空草稿不会清空原密钥" } else { "添加密钥" });
        ui.add(egui::TextEdit::singleline(&mut *draft.value).password(true).hint_text("输入密钥；失败时保留草稿").desired_width(f32::INFINITY));
        let apply = ui.button(if draft.editing.is_some() { "保存修改后的密钥" } else { "保存新密钥" }).clicked();
        if draft.editing.is_some() && ui.button("取消修改").clicked() { draft.editing = None; draft.value.zeroize(); }
        if apply { self.save_key(p, e); }
        if changed {
            let result = self.persist_keys(p, e);
            if !self.report(result, "密钥已保存") {
                if let Some(saved) = self.saved.providers[p].endpoints.iter().find(|saved| saved.id == endpoint_id) {
                    self.draft.providers[p].endpoints[e].keys.clone_from(&saved.keys);
                    self.draft.providers[p].endpoints[e].selected_key.clone_from(&saved.selected_key);
                }
            }
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

    fn models_ui(&mut self, ui: &mut egui::Ui, p: usize, e: usize) {
        let provider = self.draft.providers[p].id.clone();
        let endpoint_id = self.draft.providers[p].endpoints[e].id.clone();
        let query = self.model_query.trim().to_lowercase();
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (index, model) in self.draft.providers[p].endpoints[e].models.iter().enumerate() {
            if !(model.id.to_lowercase().contains(&query) || model.settings.display_name(&model.id).to_lowercase().contains(&query)) { continue; }
            if !match self.capability_filter {
                1 => model.capabilities.vision, 2 => model.settings.reasoning.is_some(),
                3 => model.settings.supports_tools.unwrap_or(model.capabilities.tools), 4 => model.settings.native_web_search, _ => true,
            } { continue; }
            let group = model.settings.group_name(&model.id);
            if let Some((_, items)) = groups.iter_mut().find(|(key, _)| key == group) { items.push(index); }
            else { groups.push((group.to_owned(), vec![index])); }
        }
        let all_folded = !groups.is_empty() && groups.iter().all(|(group, _)| self.folded.contains(group));
        ui.horizontal_wrapped(|ui| {
            ui.strong("模型");
            if ui.add_enabled(!groups.is_empty(), egui::Button::new(egui::RichText::new("\u{E536}").size(t::TITLE)).frame(false)).on_hover_text(if all_folded { "展开全部模型分组" } else { "收起全部模型分组" }).accessible_name(if all_folded { "展开全部模型分组" } else { "收起全部模型分组" }).clicked() {
                if all_folded { self.folded.clear(); } else { self.folded.extend(groups.iter().map(|(group, _)| group.clone())); }
            }
            if self.model_search_open || !self.model_query.is_empty() {
                let response = ui.add_sized([t::CONTROL * 4.0, t::CONTROL_SMALL], egui::TextEdit::singleline(&mut self.model_query).hint_text("搜索名称或 ID…").id_salt("model-search"));
                if self.model_search_open && !response.has_focus() && !response.lost_focus() { response.request_focus(); }
                if response.lost_focus() { self.model_search_open = false; }
                if response.has_focus() && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    self.model_query.clear(); self.model_search_open = false; response.surrender_focus();
                }
            } else if icon_button(ui, "\u{E30C}", "搜索模型").clicked() { self.model_search_open = true; }
            egui::ComboBox::from_id_salt("cap-filter").width(t::CONTROL * 2.0)
                .selected_text(["全部", "视觉", "推理", "工具", "联网"][self.capability_filter as usize]).show_ui(ui, |ui| {
                    for (value, label) in [(0, "全部"), (1, "视觉"), (2, "推理"), (3, "工具"), (4, "联网")] { ui.selectable_value(&mut self.capability_filter, value, label); }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, "\u{E3D4}", "手动添加模型").clicked() {
                    self.editor = Some(EditorDialog::Model { provider: provider.clone(), endpoint: endpoint_id.clone(), original: None,
                        value: ModelEntry { id: String::new(), capabilities: unknown_capabilities(), manual: true, settings: Default::default() }, state: Default::default() });
                }
                if ui.add_enabled(!self.dirty && self.job.is_none(), egui::Button::new("同步模型")).clicked() { self.start_job(true); }
            });
        });
        ui.add_space(t::SPACE_SM);
        if groups.is_empty() {
            egui::Frame::group(ui.style()).inner_margin(t::SPACE_XL as i8).show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 1.0).max(0.0));
                ui.vertical_centered(|ui| { ui.weak(if self.draft.providers[p].endpoints[e].models.is_empty() { "未选择模型" } else { "没有匹配的模型" }); });
            });
            return;
        }
        let mut removed = HashSet::new();
        let mut selected = None;
        for (group, models) in groups {
            ui.push_id((&endpoint_id, &group), |ui| {
                egui::Frame::group(ui.style()).inner_margin(t::SPACE_SM as i8).show(ui, |ui| {
                    let folded = self.folded.contains(&group);
                    ui.horizontal(|ui| {
                        let text = format!("{}  {}   {}", if folded { "▸" } else { "▾" }, if group.is_empty() { "未分组" } else { &group }, models.len());
                        if ui.add_sized([ui.available_width() - t::CONTROL_SMALL - t::SPACE_SM, t::CONTROL_SMALL],
                            egui::Button::new(text.atom_grow(true).atom_align(egui::Align2::LEFT_CENTER)).frame(false)).clicked() {
                            if folded { self.folded.remove(&group); } else { self.folded.insert(group.clone()); }
                        }
                        if icon_button(ui, "\u{E32A}", "移除分组模型").clicked() {
                            removed.extend(models.iter().copied());
                        }
                    });
                    if !folded {
                        for index in models {
                            let model = &self.draft.providers[p].endpoints[e].models[index];
                            ui.separator();
                            ui.push_id(&model.id, |ui| {
                                ui.horizontal(|ui| {
                                    let name = model.settings.display_name(&model.id);
                                    let (avatar, _) = ui.allocate_exact_size(egui::vec2(t::SPACE_XL, t::SPACE_XL), egui::Sense::hover());
                                    if ui.is_rect_visible(avatar) {
                                        ui.painter().circle_filled(avatar.center(), t::SPACE_MD, ui.visuals().faint_bg_color);
                                        ui.painter().text(avatar.center(), egui::Align2::CENTER_CENTER, name.chars().next().unwrap_or('?').to_uppercase().to_string(),
                                            egui::FontId::proportional(t::TEXT_SMALL), ui.visuals().weak_text_color());
                                    }
                                    let badges = [model.capabilities.vision, model.settings.reasoning.is_some(), model.settings.supports_tools.unwrap_or(model.capabilities.tools), model.settings.native_web_search];
                                    let reserve = t::CONTROL_SMALL * 2.0 + badges.iter().filter(|b| **b).count() as f32 * (t::SPACE_XL + t::SPACE_XS) + t::SPACE_XL;
                                    let width = (ui.available_width() - reserve).max(t::SPACE_XL);
                                    if ui.add_sized([width, t::ROW], egui::Button::new(egui::RichText::new(name).strong().atom_grow(true).atom_align(egui::Align2::LEFT_CENTER)).frame(false).truncate())
                                        .on_hover_text(&model.id).clicked() {
                                        selected = Some(model.id.clone());
                                    }
                                    ui.scope(|ui| {
                                        ui.spacing_mut().item_spacing.x = t::SPACE_XS / 2.0;
                                        for (shown, icon, label, kind) in [(badges[0], "\u{E220}", "视觉输入", 0), (badges[1], "\u{E74E}", "推理", 1), (badges[2], "\u{E5D4}", "工具调用", 2), (badges[3], "\u{E288}", "原生联网", 3)] {
                                            if shown { capability_badge(ui, icon, label, kind); }
                                        }
                                    });
                                    if icon_button(ui, "\u{E434}", &format!("模型设置 {}", model.id)).clicked() {
                                        self.editor = Some(EditorDialog::Model { provider: provider.clone(), endpoint: endpoint_id.clone(), original: Some(model.id.clone()), value: model.clone(), state: Default::default() });
                                    }
                                    if icon_button(ui, "\u{E32A}", &format!("移除模型 {}", model.id)).clicked() {
                                        removed.insert(index);
                                    }
                                });
                            });
                        }
                    }
                });
            });
            ui.add_space(t::SPACE_SM);
        }
        if !removed.is_empty() || selected.is_some() {
            let before = self.draft.providers[p].clone();
            let endpoint = &mut self.draft.providers[p].endpoints[e];
            if let Some(id) = selected { endpoint.selected_model = Some(id); }
            let mut index = 0;
            endpoint.models.retain(|_| { let keep = !removed.contains(&index); index += 1; keep });
            repair_model_selection(endpoint);
            let result = self.save(); if !self.report(result, "已保存") { self.draft.providers[p] = before; }
        }
    }

    fn editor_ui(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else { return; };
        let mut save = false;
        let mut close = false;
        let mut delete = false;
        let mut validation = Ok(());
        let modal = egui::Modal::new(egui::Id::unique("model-settings-editor")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - t::SPACE_XL * 2.0).clamp(t::DIALOG_MIN, t::DIALOG_WIDTH));
            ui.horizontal(|ui| {
                let title = match &editor {
                    EditorDialog::Provider { id, .. } => if id.is_some() { "服务商设置" } else { "新建服务商" },
                    EditorDialog::Endpoint { id, .. } => if id.is_some() { "线路设置" } else { "添加线路" },
                    EditorDialog::Model { original, .. } => if original.is_some() { "编辑模型" } else { "添加模型" },
                    EditorDialog::Keys { .. } => "密钥管理",
                };
                ui.strong(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { close |= icon_button(ui, "\u{E4F6}", "关闭编辑").clicked(); });
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("editor-body").max_height((ctx.content_rect().height() * 0.85 - t::SPACE_XL * 5.0).max(t::CONTROL * 3.0)).show(ui, |ui| {
                match &mut editor {
                    EditorDialog::Provider { id, name } => {
                        ui.add_space(t::SPACE_LG); ui.label("名称");
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                        if !valid_name(name) { validation = Err("名称应为 1–128 字符且不含控制字符".into()); }
                        if id.is_some() && ui.button("删除服务商…").clicked() { delete = true; }
                    }
                    EditorDialog::Endpoint { id, name, url, protocol, .. } => {
                        ui.add_space(t::SPACE_LG); ui.label("线路名称");
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                        ui.add_space(t::SPACE_MD); ui.label("API 地址");
                        ui.add(egui::TextEdit::singleline(url).desired_width(f32::INFINITY));
                        ui.add_space(t::SPACE_MD); ui.label("协议");
                        egui::ComboBox::from_id_salt("editor-protocol").selected_text(protocol_name(*protocol)).show_ui(ui, |ui| {
                            for value in [Protocol::OpenAiChat, Protocol::OpenAiResponses, Protocol::AnthropicMessages] { ui.selectable_value(protocol, value, protocol_name(value)); }
                        });
                        validation = if !valid_name(name) { Err("线路名称应为 1–128 字符且不含控制字符".into()) } else { validate_url(url).map(|_| ()) };
                        if let Ok(url) = model_url(url, *protocol) { ui.small(format!("GET {url}")); }
                        if id.is_some() && ui.button("删除线路…").clicked() { delete = true; }
                    }
                    EditorDialog::Model { provider, endpoint, original, value, state } => {
                        let protocol = self.draft.providers.iter().find(|p| p.id == *provider)
                            .and_then(|p| p.endpoints.iter().find(|e| e.id == *endpoint)).map(|e| e.protocol).unwrap_or(Protocol::OpenAiChat);
                        validation = ui.push_id((provider.as_str(), endpoint.as_str(), original.as_deref()), |ui| {
                            crate::model_editor::fields(ui, value, protocol, original.is_some(), state)
                        }).inner;
                    }
                    EditorDialog::Keys { provider, endpoint } => {
                        if let Some(p) = self.draft.providers.iter().position(|p| p.id == *provider) {
                            if let Some(e) = self.draft.providers[p].endpoints.iter().position(|e| e.id == *endpoint) { self.keys_ui(ui, p, e); }
                        }
                    }
                }
            });
            ui.separator();
            if let Err(error) = &validation { ui.colored_label(ui.visuals().error_fg_color, error); }
            if self.error { ui.colored_label(ui.visuals().error_fg_color, &self.notice); }
            if matches!(editor, EditorDialog::Model { .. }) { ui.small("保存后从下一次调用生效；正在生成的回复不受影响。"); }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !matches!(editor, EditorDialog::Keys { .. }) {
                    save = ui.add_enabled(validation.is_ok(), egui::Button::new(egui::RichText::new("保存").color(ui.visuals().selection.stroke.color)).fill(ui.visuals().selection.bg_fill)).clicked();
                }
                close |= ui.button(if matches!(editor, EditorDialog::Keys { .. }) { "完成" } else { "取消" }).clicked();
            });
        });
        if delete {
            match &editor {
                EditorDialog::Provider { id: Some(id), .. } => self.delete_provider = Some(id.clone()),
                EditorDialog::Endpoint { id: Some(id), .. } => self.delete_endpoint = Some(id.clone()),
                _ => {}
            }
            close = true;
        }
        if save {
            let before = self.draft.clone();
            let selected_provider = self.provider.clone();
            let selected_endpoint = self.endpoint.clone();
            let result = self.apply_editor(&editor).and_then(|_| self.save());
            if self.report(result, "已保存") { close = true; }
            else { self.draft = before; self.provider = selected_provider; self.endpoint = selected_endpoint; }
        }
        if close || modal.should_close() { self.key_drafts.clear(); }
        else { self.editor = Some(editor); }
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

    fn confirmations(&mut self, ctx: &egui::Context) {
        if self.delete_provider.is_none() && self.delete_endpoint.is_none() && self.delete_key.is_none() { return; }
        let mut confirm = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("delete-model-config")).show(ctx, |ui| {
            ui.set_width(t::DIALOG_WIDTH.min(ctx.content_rect().width() - t::SPACE_XL * 2.0));
            ui.heading("确认删除"); ui.add_space(t::SPACE_MD);
            ui.label(if self.delete_key.is_some() { "将删除此密钥；其它密钥、模型和线路保持不变。" } else if self.delete_provider.is_some() { "将永久删除此服务商及全部线路、模型与密钥。" } else { "将永久删除此线路及模型与密钥。" });
            if self.error { ui.colored_label(ui.visuals().error_fg_color, &self.notice); }
            ui.horizontal(|ui| { close = ui.button("取消").clicked(); confirm = ui.button("确认删除").clicked(); });
        });
        if confirm {
            let before = self.draft.clone();
            if let Some(id) = &self.delete_provider { self.draft.providers.retain(|p| p.id != *id); }
            if let Some(id) = &self.delete_endpoint { for p in &mut self.draft.providers { p.endpoints.retain(|e| e.id != *id); } }
            if let Some((provider, endpoint, key)) = &self.delete_key {
                if let Some(endpoint) = self.draft.providers.iter_mut().find(|p| &p.id == provider).and_then(|p| p.endpoints.iter_mut().find(|e| &e.id == endpoint)) {
                    endpoint.keys.retain(|k| &k.id != key);
                    if endpoint.selected_key.as_ref() == Some(key) { endpoint.selected_key = endpoint.keys.first().map(|k| k.id.clone()); }
                }
            }
            let result = if let Some((provider, endpoint, _)) = &self.delete_key {
                let indices = self.draft.providers.iter().position(|p| &p.id == provider)
                    .and_then(|p| self.draft.providers[p].endpoints.iter().position(|e| &e.id == endpoint).map(|e| (p, e)));
                match indices { Some((p, e)) => self.persist_keys(p, e), None => Err("线路已不存在".into()) }
            } else { self.save() };
            if self.report(result, "已删除") { self.key_drafts.clear(); self.repair_selection(); close = true; }
            else { self.draft = before; }
        }
        if close || modal.should_close() { self.delete_provider = None; self.delete_endpoint = None; self.delete_key = None; }
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
                        let types = job.config.endpoint.models.iter().map(|model| (model.id.clone(), model.settings.model_type)).collect();
                        self.sync_dialog = Some(SyncDialog { provider: job.config.provider, endpoint: job.config.endpoint.id, ids, selected, search: String::new(), types, filter: None, folded: HashSet::new() });
                    }
                }
                Err(error) => { self.report(Err(error), ""); }
            }
        }
        if self.job.is_some() { ctx.request_repaint_after(Duration::from_millis(80)); }
    }
    fn sync_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.sync_dialog.take() else { return; };
        let previous_selected = dialog.selected.clone();
        let previous_types = dialog.types.clone();
        let mut changed = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("sync-models")).show(ctx, |ui| {
            ui.set_width(t::PAGE_MAX.min(ctx.content_rect().width() - t::SPACE_XL * 2.0));
            ui.horizontal(|ui| { ui.strong(format!("选择模型  {}", dialog.ids.len())); close = icon_button(ui, "\u{E4F6}", "关闭模型选择").clicked(); });
            ui.small("点击加入或移除即保存；不会改动本次目录之外的模型。");
            ui.horizontal(|ui| {
                ui.add_sized([ui.available_width() - t::CONTROL * 3.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut dialog.search).hint_text("搜索模型…"));
                let query = dialog.search.trim().to_lowercase();
                let shown: Vec<_> = dialog.ids.iter().filter(|id| id.to_lowercase().contains(&query) && dialog.filter.is_none_or(|kind| dialog.types.get(*id).copied().unwrap_or(ModelType::Chat) == kind)).cloned().collect();
                let remove = !shown.is_empty() && shown.iter().all(|id| dialog.selected.contains(id));
                if ui.add_enabled(!shown.is_empty(), egui::Button::new(if remove { "全部移除" } else { "全部加入" })).clicked() {
                    for id in shown { if remove { dialog.selected.remove(&id); } else { dialog.selected.insert(id); } } changed = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                let query = dialog.search.trim().to_lowercase();
                let count = |kind: Option<ModelType>| dialog.ids.iter().filter(|id| id.to_lowercase().contains(&query) && kind.is_none_or(|kind| dialog.types.get(*id).copied().unwrap_or(ModelType::Chat) == kind)).count();
                let total = count(None);
                ui.selectable_value(&mut dialog.filter, None, format!("全部 {total}"));
                for kind in [ModelType::Chat, ModelType::Image, ModelType::Embedding, ModelType::Rerank, ModelType::Video, ModelType::Tts, ModelType::Stt, ModelType::Unknown] {
                    let hits = count(Some(kind));
                    ui.add_enabled_ui(hits != 0, |ui| { ui.selectable_value(&mut dialog.filter, Some(kind), format!("{} {hits}", kind.label())); });
                }
            });
            ui.separator();
            let query = dialog.search.trim().to_lowercase();
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.5).show(ui, |ui| {
                let mut groups: Vec<(&str, Vec<&String>)> = Vec::new();
                for id in &dialog.ids {
                    let kind = dialog.types.get(id).copied().unwrap_or(ModelType::Chat);
                    if !id.to_lowercase().contains(&query) || dialog.filter.is_some_and(|filter| filter != kind) { continue; }
                    let group = id.split(['-', '/', ':']).next().unwrap_or(id);
                    if let Some((_, ids)) = groups.iter_mut().find(|(name, _)| *name == group) { ids.push(id); }
                    else { groups.push((group, vec![id])); }
                }
                if groups.is_empty() { ui.weak("没有匹配的模型"); }
                for (group, ids) in groups {
                    let folded = query.is_empty() && dialog.folded.contains(group);
                    if ui.button(format!("{} {group}  {}", if folded { "▸" } else { "▾" }, ids.len())).clicked() {
                        if folded { dialog.folded.remove(group); } else { dialog.folded.insert(group.to_owned()); }
                    }
                    if folded { continue; }
                    for id in ids {
                        ui.push_id(id, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_sized([ui.available_width() - t::CONTROL * 5.0, t::ROW], egui::Label::new(id).truncate());
                                let value = dialog.types.entry(id.clone()).or_insert(ModelType::Chat);
                                let previous = *value;
                                egui::ComboBox::from_id_salt("type").selected_text(value.label()).show_ui(ui, |ui| {
                                    for kind in [ModelType::Chat, ModelType::Image, ModelType::Embedding, ModelType::Rerank, ModelType::Video, ModelType::Tts, ModelType::Stt, ModelType::Unknown] { ui.selectable_value(value, kind, kind.label()); }
                                });
                                if previous != *value && dialog.selected.contains(id) { changed = true; }
                                let selected = dialog.selected.contains(id);
                                if icon_button(ui, if selected { "\u{E32A}" } else { "\u{E3D4}" }, &format!("{}模型 {id}", if selected { "移除" } else { "加入" })).clicked() {
                                    if selected { dialog.selected.remove(id); } else { dialog.selected.insert(id.clone()); } changed = true;
                                }
                            });
                        });
                    }
                }
            });
            ui.separator(); ui.small(format!("已选择 {} 个模型", dialog.ids.iter().filter(|id| dialog.selected.contains(*id)).count()));
            if self.error { ui.colored_label(ui.visuals().error_fg_color, &self.notice); }
        });
        if changed {
            let before = self.draft.clone();
            let result = (|| {
                let endpoint = self.draft.providers.iter_mut().find(|p| p.id == dialog.provider)
                    .and_then(|p| p.endpoints.iter_mut().find(|e| e.id == dialog.endpoint)).ok_or("目标线路已不存在")?;
                merge_models(&mut endpoint.models, &dialog.ids, &dialog.selected);
                for model in &mut endpoint.models {
                    if let Some(kind) = dialog.types.get(&model.id) {
                        model.settings.model_type = *kind;
                        if *kind != ModelType::Chat { model.settings.reasoning = None; model.settings.native_web_search = false; model.settings.pricing = None; model.settings.supports_tools = None; }
                    }
                }
                repair_model_selection(endpoint);
                self.save()
            })();
            if !self.report(result, "模型选择已保存") { self.draft = before; dialog.selected = previous_selected; dialog.types = previous_types; }
        }
        if !close && !modal.should_close() { self.sync_dialog = Some(dialog); }
    }

    pub fn clear_secrets(&mut self) {
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
    fn drop(&mut self) { self.clear_secrets(); if let Some(job) = self.job.take() { let _ = job.thread.join(); } }
}


fn icon_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    ui.add_sized([t::CONTROL_SMALL, t::CONTROL_SMALL], egui::Button::new(egui::RichText::new(icon).size(t::TITLE)).frame(false))
        .on_hover_text(label).accessible_name(label)
}

fn capability_badge(ui: &mut egui::Ui, icon: &str, label: &str, kind: u8) {
    let color = crate::theme::capability_color(ui.visuals(), kind);
    // 内置 Label 会受全局输入行高撑大，导致 24×20 的能力胶囊变成方块。
    let (rect, response) = ui.allocate_exact_size(egui::vec2(t::SPACE_XL, t::SPACE_LG + t::SPACE_XS), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, rect.height() / 2.0, color.gamma_multiply(0.12));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(t::TEXT_SMALL), color);
    }
    response.on_hover_text(label);
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
        let mut settings = ModelSettings::new(&root).unwrap();
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
        let restored = ModelSettings::new(&root).unwrap();
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
        settings.clear_secrets(); assert!(settings.key_drafts.is_empty());
        drop(db); drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_cancel_and_save_keep_other_endpoint() {
        let root = std::env::temp_dir().join(format!("lumadesk-editor-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::new(&root).unwrap();
        let provider = settings.saved.providers[0].id.clone();
        let endpoint = settings.saved.providers[0].endpoints[0].id.clone();
        let original = serde_json::to_value(settings.store.load().unwrap().0).unwrap();
        settings.editor = Some(EditorDialog::Endpoint { provider: provider.clone(), id: Some(endpoint.clone()), name: "edited".into(), url: "https://example.com/v1".into(), protocol: Protocol::OpenAiResponses });
        settings.clear_secrets();
        assert_eq!(serde_json::to_value(settings.store.load().unwrap().0).unwrap(), original);
        let before_other = serde_json::to_value(&settings.saved.providers[1]).unwrap();
        let model = EditorDialog::Model { provider, endpoint, original: None, value: ModelEntry { id: "manual-model".into(), capabilities: ModelCapabilities { streaming: true, tools: false, vision: true }, manual: true, settings: Default::default() }, state: Default::default() };
        settings.apply_editor(&model).unwrap(); settings.save().unwrap();
        let restored = ModelSettings::new(&root).unwrap();
        assert_eq!(restored.saved.providers[0].endpoints[0].models[0].id, "manual-model");
        assert!(restored.saved.providers[0].endpoints[0].models[0].capabilities.vision);
        assert_eq!(serde_json::to_value(&restored.saved.providers[1]).unwrap(), before_other);
        assert!(settings.apply_editor(&model).is_err());
        drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enable_is_independent_of_invalid_address_and_restores_on_write_failure() {
        let root = std::env::temp_dir().join(format!("lumadesk-enable-test-{}", new_id().unwrap()));
        let mut settings = ModelSettings::new(&root).unwrap();
        let original_url = settings.saved.providers[0].endpoints[0].base_url.clone();
        settings.draft.providers[0].endpoints[0].base_url = "unfinished".into(); settings.dirty = true;
        settings.set_enabled(0, true);
        assert!(settings.saved.providers[0].enabled);
        assert_eq!(settings.saved.providers[0].endpoints[0].base_url, original_url);
        assert_eq!(settings.draft.providers[0].endpoints[0].base_url, "unfinished");
        let restored = ModelSettings::new(&root).unwrap(); assert!(restored.saved.providers[0].enabled);
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
        let mut settings = ModelSettings::new(&root).unwrap();
        assert_eq!(settings.saved.providers.len(), 3);
        assert_eq!(serde_json::to_value(&settings.saved.providers[0]).unwrap(), original["providers"][0]);
        settings.draft.providers.retain(|p| p.id != "preset-qiniu");
        settings.save().unwrap();
        let restored = ModelSettings::new(&root).unwrap();
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
        let mut settings = ModelSettings::new(&root).unwrap();
        settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: "回环测试".into(), enabled: true, endpoints: vec![endpoint] });
        settings.save().unwrap();
        let restored = ModelSettings::new(&root).unwrap();
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
}
