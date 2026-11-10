//! 设置草稿与已保存目录分开，聊天不直接读取设置控件。
use gpui_kit::{App, Context, Entity, Window};
use lumakit_support::secrets::{SavedKey, new_id};
mod catalog_refresh;
mod data;
mod detection;
mod editor;
mod worker;
use data::*;
use detection::detect_models;
mod view;
use futures_util::StreamExt;
use lumakit_ai::catalog::{EndpointEntry, ModelCatalog, ModelEntry, ProviderEntry, validate_url};
use lumakit_ai::catalog_store::CatalogStore;
use lumakit_ai::chat::ModelOption;
use lumakit_ai::model_config::ModelType;
use lumakit_ai::providers::ModelCapabilities;
use lumakit_ai::{AiConnection, Protocol};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::mpsc,
    thread::JoinHandle,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

#[cfg(test)]
const VERSION: u32 = 1;
const MAX_KEYS: usize = 16;
const MAX_BODY: usize = 2 * 1024 * 1024;

const PRESETS: [(&str, &str, &str, &str); 3] = [
    (
        "preset-qiniu",
        "七牛云",
        "https://api.qnaigc.com",
        "https://s.qiniu.com/BfyeU3",
    ),
    (
        "preset-siliconflow",
        "硅基流动",
        "https://api.siliconflow.cn/v1",
        "https://cloud.siliconflow.cn/i/ieP7r6Bx",
    ),
    (
        "preset-fenno",
        "Fenno",
        "https://api.fenno.ai/v1",
        "https://api.fenno.ai/s/vWbM",
    ),
];

#[derive(Clone, Default, Serialize, Deserialize)]
struct SettingsFile {
    providers: Vec<ProviderEntry>,
    #[serde(default)]
    presets_initialized: bool,
}

impl SettingsFile {
    fn seed_presets(&mut self) {
        if self.presets_initialized {
            return;
        }
        for (id, name, base_url, _) in PRESETS {
            if self
                .providers
                .iter()
                .any(|p| p.id == id || p.name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            self.providers.push(ProviderEntry {
                id: id.into(),
                name: name.into(),
                enabled: false,
                endpoints: vec![EndpointEntry {
                    id: format!("{id}-main"),
                    name: "默认线路".into(),
                    base_url: base_url.into(),
                    protocol: Protocol::OpenAiChat,
                    keys: Vec::new(),
                    models: Vec::new(),
                    selected_key: None,
                    selected_model: None,
                }],
            });
        }
        // 删除预设后重启不应重新出现；不能用“列表为空”判断是否首次初始化。
        self.presets_initialized = true;
    }
}

#[derive(Default)]
struct KeyDraft {
    value: Zeroizing<String>,
    editing: Option<String>,
    revealed: Option<Zeroizing<String>>,
    revealed_key: Option<String>,
}
#[derive(Clone)]
struct DetectConfig {
    provider: String,
    endpoint: EndpointEntry,
}
struct Job {
    cancel: CancellationToken,
    thread: JoinHandle<()>,
    rx: mpsc::Receiver<Result<Vec<String>, crate::settings_task::Failure>>,
    config: DetectConfig,
    sync: bool,
    trace: lumakit_observability::TraceContext,
    root: lumakit_observability::OperationGuard,
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
    Provider {
        id: Option<String>,
        name: String,
    },
    Endpoint {
        provider: String,
        id: Option<String>,
        name: String,
        url: String,
        protocol: Protocol,
    },
    Model {
        provider: String,
        endpoint: String,
        original: Option<String>,
        value: ModelEntry,
    },
    Keys {
        provider: String,
        endpoint: String,
    },
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
    diagnostic_trace: Option<String>,
    catalog_diagnostic_trace: Option<String>,
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
        let mut saved = SettingsFile {
            providers,
            presets_initialized,
        };
        project(&saved)?;
        if !saved.presets_initialized {
            saved.seed_presets();
            project(&saved)?;
            store.save(&saved.providers, saved.presets_initialized)?;
        }
        let mut state = Self {
            store,
            draft: saved.clone(),
            catalog: ModelCatalog::new(saved.providers.clone()),
            options: model_options(&saved),
            image_options: model_options_for(&saved, ModelType::Image),
            saved,
            provider: None,
            endpoint: None,
            search: String::new(),
            key_drafts: HashMap::new(),
            notice: String::new(),
            error: false,
            diagnostic_trace: None,
            catalog_diagnostic_trace: None,
            dirty: false,
            delete_provider: None,
            delete_endpoint: None,
            delete_key: None,
            sync_dialog: None,
            job: None,
            catalog_refresh: None,
            catalog_refresh_notice: String::new(),
            provider_filter: 0,
            model_query: String::new(),
            capability_filter: 0,
            folded: HashSet::new(),
            editor: None,
            view: None,
        };
        state.repair_selection();
        Ok(state)
    }

    fn indices(&self) -> Option<(usize, Option<usize>)> {
        let p = self
            .draft
            .providers
            .iter()
            .position(|p| Some(&p.id) == self.provider.as_ref())?;
        let e = self.draft.providers[p]
            .endpoints
            .iter()
            .position(|e| Some(&e.id) == self.endpoint.as_ref());
        Some((p, e))
    }
    fn repair_selection(&mut self) {
        if !self
            .draft
            .providers
            .iter()
            .any(|p| Some(&p.id) == self.provider.as_ref())
        {
            self.provider = self.draft.providers.first().map(|p| p.id.clone());
        }
        let selected = self
            .draft
            .providers
            .iter()
            .find(|p| Some(&p.id) == self.provider.as_ref());
        if !selected.is_some_and(|p| {
            p.endpoints
                .iter()
                .any(|e| Some(&e.id) == self.endpoint.as_ref())
        }) {
            self.endpoint = selected
                .and_then(|p| p.endpoints.first())
                .map(|e| e.id.clone());
        }
    }
    fn report(&mut self, result: Result<(), String>, success: &str) -> bool {
        self.diagnostic_trace = None;
        match result {
            Ok(()) => {
                self.notice = success.into();
                self.error = false;
                true
            }
            Err(error) => {
                self.notice = error;
                self.error = true;
                false
            }
        }
    }
    fn save(&mut self) -> Result<(), String> {
        project(&self.draft)?;
        self.store
            .save(&self.draft.providers, self.draft.presets_initialized)?;
        self.saved = self.draft.clone();
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        self.dirty = false;
        Ok(())
    }
    fn persist_keys(&mut self, p: usize, e: usize) -> Result<(), String> {
        let endpoint = &self.draft.providers[p].endpoints[e];
        let saved = self
            .saved
            .providers
            .iter_mut()
            .find(|provider| provider.id == self.draft.providers[p].id)
            .and_then(|provider| {
                provider
                    .endpoints
                    .iter_mut()
                    .find(|saved| saved.id == endpoint.id)
            })
            .ok_or("线路已不存在")?;
        self.store.save_keys(
            &endpoint.id,
            &endpoint.keys,
            endpoint.selected_key.as_deref(),
        )?;
        saved.keys.clone_from(&endpoint.keys);
        saved.selected_key.clone_from(&endpoint.selected_key);
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        Ok(())
    }
    fn commit(&mut self) {
        let result = self.save();
        self.report(result, "已保存");
    }

    fn set_enabled(&mut self, p: usize, enabled: bool) {
        let id = self.draft.providers[p].id.clone();
        let mut candidate = self.saved.clone();
        let Some(provider) = candidate
            .providers
            .iter_mut()
            .find(|provider| provider.id == id)
        else {
            return;
        };
        provider.enabled = enabled;
        let result = self.store.set_enabled(&id, enabled);
        if self.report(result, "已保存") {
            self.saved = candidate;
            self.draft.providers[p].enabled = enabled;
            self.catalog = ModelCatalog::new(self.saved.providers.clone());
            self.options = model_options(&self.saved);
            self.image_options = model_options_for(&self.saved, ModelType::Image);
        } else {
            self.draft.providers[p].enabled = self
                .saved
                .providers
                .iter()
                .find(|provider| provider.id == id)
                .is_some_and(|provider| provider.enabled);
        }
    }

    fn clear_drafts(&mut self) {
        self.key_drafts.clear();
        self.sync_dialog = None;
        self.editor = None;
        self.delete_key = None;
        if let Some(job) = &self.job {
            job.cancel.cancel();
        }
    }
    pub fn available_models(&self) -> &[ModelOption] {
        &self.options
    }
    pub fn image_models(&self) -> &[ModelOption] {
        &self.image_options
    }
    pub fn model_settings(
        &self,
        model: &lumakit_ai::ModelRef,
    ) -> Option<&lumakit_ai::model_config::ModelSettings> {
        self.saved
            .providers
            .iter()
            .find(|p| p.enabled && p.id == model.provider)?
            .endpoints
            .iter()
            .find(|e| e.id == model.endpoint && e.protocol == model.protocol)?
            .models
            .iter()
            .find(|m| m.id == model.model)
            .map(|m| &m.settings)
    }
    pub fn image_connection(&self, model: &lumakit_ai::ModelRef) -> Result<AiConnection, String> {
        self.catalog.resolve_for(model, ModelType::Image)
    }
    pub fn connection(&self, model: &lumakit_ai::ModelRef) -> Result<AiConnection, String> {
        self.catalog.resolve(model)
    }
}
impl Drop for ModelSettings {
    fn drop(&mut self) {
        self.cancel_catalog_refresh();
        self.clear_drafts();
        if let Some(job) = self.job.take() {
            let _ = job.thread.join();
            job.root.cancel("settings_closed");
        }
    }
}

#[cfg(test)]
mod tests;
