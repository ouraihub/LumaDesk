//! 设置草稿与已保存目录分开，聊天不直接读取设置控件。
use gpui_kit::{App, Context, Entity, Window};
use lumakit_support::secrets::{SavedKey, new_id, normalize_key_label};
mod catalog_refresh;
mod data;
mod detection;
mod editor;
mod persistence;
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
    label: String,
    editing: Option<String>,
    revealed: Option<Zeroizing<String>>,
    revealed_key: Option<String>,
    submitted: bool,
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
    fn report_task<T>(
        &mut self,
        trace: &lumakit_observability::TraceContext,
        result: &Result<T, crate::settings_task::Failure>,
        success: &str,
    ) -> bool {
        let accepted = self.report(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|error| error.notice(trace)),
            success,
        );
        self.diagnostic_trace = result.as_ref().err().map(|_| trace.trace_id().to_owned());
        accepted
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
        self.close_job();
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod logging_tests;
