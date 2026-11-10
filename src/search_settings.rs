//! Independent search/read configuration, encrypted credentials and cancellable checks.
use crate::settings_store::{self, SearchConfigRow, SearchData, SearchKeyRow};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::{AppContext, Context, Entity, Subscription, Window};
use lumakit_ai::search::{SearchPreferences, SearchResult};
use lumakit_support::secrets::{SavedKey, normalize_key_label};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

mod catalog;
mod inputs;
#[cfg(test)]
mod lifecycle_tests;
mod loading;
mod network;
mod persistence;
mod stored;
#[cfg(test)]
mod tests;
mod view;
mod worker;
use catalog::{PROVIDERS, Provider};
use network::*;

const FILE: &str = "search-settings.json";
const MAX_BODY: usize = 2 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    provider: Provider,
    endpoint: String,
    keys: Vec<SavedKey>,
    #[serde(default)]
    username: String,
    /// Legacy request preference, used only while the selected credential remains enabled.
    #[serde(default)]
    selected_key: Option<String>,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Compression {
    None,
    Cutoff,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    default_search: Provider,
    default_fetch: Provider,
    configs: Vec<Config>,
    preferences: SearchPreferences,
    compression: Compression,
}
struct Draft {
    endpoint: String,
    username: String,
    key: String,
    label: String,
    submitted: bool,
    generation: u64,
    editing: Option<String>,
    revealed: Option<(String, String)>,
}
impl Draft {
    fn new(config: &Config) -> Self {
        Self {
            endpoint: config.endpoint.clone(),
            username: config.username.clone(),
            key: String::new(),
            label: String::new(),
            submitted: false,
            generation: 0,
            editing: None,
            revealed: None,
        }
    }
    fn clear(&mut self) {
        self.key.zeroize();
        self.label.clear();
        self.submitted = false;
        self.generation = self.generation.wrapping_add(1);
        self.editing = None;
        if let Some((_, mut value)) = self.revealed.take() {
            value.zeroize();
        }
    }
}
impl Drop for Draft {
    fn drop(&mut self) {
        self.clear();
    }
}
#[derive(Debug)]
struct Page {
    title: String,
    url: String,
    body: String,
}
enum Output {
    Search(SearchResult),
    Fetch(Page),
}
struct Job {
    cancel: CancellationToken,
    thread: thread::JoinHandle<()>,
    receiver: mpsc::Receiver<Result<Output, crate::settings_task::Failure>>,
    discard: bool,
    check: Option<usize>,
    trace: lumakit_observability::TraceContext,
    root: lumakit_observability::OperationGuard,
}
enum KeyAction {
    Add,
    Replace(String, String),
    Delete(String),
    SetEnabled(String, bool),
}
struct ProviderInputs {
    endpoint: Entity<InputState>,
    username: Entity<InputState>,
    key: Entity<InputState>,
    label: Entity<InputState>,
}
struct SearchInputs {
    providers: Vec<ProviderInputs>,
    query: Entity<InputState>,
    target: Entity<InputState>,
    max_results: Entity<InputState>,
    cutoff: Entity<InputState>,
    blacklist: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
    focus: gpui_kit::FocusHandle,
}
pub struct SearchSettings {
    root: PathBuf,
    stored: Stored,
    drafts: Vec<Draft>,
    search_index: usize,
    fetch_index: usize,
    preferences: SearchPreferences,
    compression: Compression,
    blacklist: String,
    query: String,
    target: String,
    result: Option<SearchResult>,
    page: Option<Page>,
    status: String,
    diagnostic_trace: Option<String>,
    card_diagnostic_trace: Vec<Option<String>>,
    job: Option<Job>,
    inputs: Option<SearchInputs>,
    advanced: bool,
    delete_key: Option<(usize, String)>,
    key_dialog: Option<usize>,
    card_status: Vec<String>,
    card_error: Vec<bool>,
    previous_focus: Option<gpui_kit::FocusHandle>,
}
impl SearchSettings {
    pub fn create(
        root: &Path,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> Result<Entity<Self>, String> {
        let this = Self::load(root)?;
        Ok(cx.new(|cx| this.with_inputs(window, cx)))
    }
    fn load(root: &Path) -> Result<Self, String> {
        let (trace, guard) =
            lumakit_observability::TraceContext::root("settings.search.load", "ui");
        let _entered = guard.span().clone().entered();
        let result = Self::load_traced(root, &trace);
        crate::settings_task::end(guard, &result);
        result.map_err(|error| error.notice(&trace))
    }
    fn cancel_and_clear(&mut self) {
        for draft in &mut self.drafts {
            draft.clear();
        }
        self.delete_key = None;
        self.key_dialog = None;
        if let Some(job) = &mut self.job {
            job.cancel.cancel();
            job.discard = true;
        }
    }
    pub fn clear_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_and_clear();
        if let Some(inputs) = &self.inputs {
            for provider in &inputs.providers {
                provider
                    .key
                    .update(cx, |input, cx| input.set_value("", window, cx));
                provider
                    .label
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
        cx.notify();
    }
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn poll(&mut self) -> bool {
        if !self
            .job
            .as_ref()
            .is_some_and(|job| job.thread.is_finished())
        {
            return false;
        }
        self.drain();
        true
    }
    fn clear_provider(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.drafts[index].clear();
        self.delete_key = None;
        if let Some(inputs) = &self.inputs {
            inputs.providers[index]
                .key
                .update(cx, |input, cx| input.set_value("", window, cx));
            inputs.providers[index]
                .label
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
    fn start_request(&mut self, fetch: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(inputs) = &self.inputs {
            self.query = inputs.query.read(cx).value().to_string();
            self.target = inputs.target.read(cx).value().to_string();
        }
        self.launch(fetch, None, None);
        let _ = window;
        cx.notify();
    }
    fn check_provider(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let (trace, root) =
            lumakit_observability::TraceContext::root("settings.search.check", "ui");
        let _entered = root.span().clone().entered();
        if let Some(inputs) = &self.inputs {
            let input = &inputs.providers[index];
            self.drafts[index].endpoint = input.endpoint.read(cx).value().to_string();
            self.drafts[index].username = input.username.read(cx).value().to_string();
        }
        let saved = self.save_with_trace(index, None, &trace);
        if let Err(error) = saved {
            self.card_status[index] = error.notice(&trace);
            self.card_error[index] = true;
            self.card_diagnostic_trace[index] = Some(trace.trace_id().to_owned());
            crate::settings_task::end(root, &Err::<(), _>(error));
            cx.notify();
            return;
        }
        self.launch(
            self.stored.configs[index].provider.is_fetch(),
            Some(index),
            Some((trace, root)),
        );
        cx.notify();
    }
}
impl Drop for SearchSettings {
    fn drop(&mut self) {
        self.cancel_and_clear();
        if let Some(job) = self.job.take() {
            let result = worker::complete_job(job, Some("settings_closed"));
            drop(result);
        }
    }
}
fn validate_key(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("密钥须为 1–4096 字节且不含控制字符".into());
    }
    Ok(())
}
fn add_key(config: &mut Config, value: &str) -> Result<(), String> {
    validate_key(value)?;
    if config.keys.len() >= 16 {
        return Err("每个服务商最多 16 个密钥".into());
    }
    let key = SavedKey::new(value.trim())?;
    if config.selected_key.is_none() {
        config.selected_key = Some(key.id.clone());
    }
    config.keys.push(key);
    Ok(())
}
fn validate_stored(value: &Stored) -> Result<(), String> {
    let legacy = [
        Provider::Tavily,
        Provider::Brave,
        Provider::Exa,
        Provider::Direct,
        Provider::Jina,
    ];
    if legacy
        .iter()
        .any(|p| !value.configs.iter().any(|c| c.provider == *p))
        || PROVIDERS
            .iter()
            .any(|p| value.configs.iter().filter(|c| c.provider == *p).count() > 1)
    {
        return Err("网络搜索配置缺失或重复；原文件未修改".into());
    }
    if value.default_search.is_fetch() || !value.default_fetch.is_fetch() {
        return Err("默认服务商与能力不匹配".into());
    }
    preferences_from_draft(&value.preferences, &value.preferences.blacklist.join("\n"))?;
    for config in &value.configs {
        if !(config.provider == Provider::Direct && config.endpoint.is_empty()) {
            validate_url(&config.endpoint)?;
        }
        if config.username.len() > 256
            || config.username.chars().any(char::is_control)
            || config.username.contains(':')
        {
            return Err("Basic 用户名无效".into());
        }
        if config.keys.len() > 16 {
            return Err("每个服务商最多 16 个密钥".into());
        }
        let mut seen = std::collections::HashSet::new();
        for key in &config.keys {
            if key.id.is_empty() || key.ciphertext.is_empty() || !seen.insert(&key.id) {
                return Err("密钥配置无效".into());
            }
            normalize_key_label(key.label.as_deref().unwrap_or_default())?;
        }
        if config
            .selected_key
            .as_ref()
            .is_some_and(|id| !seen.contains(id))
        {
            return Err("密钥配置无效".into());
        }
    }
    Ok(())
}
