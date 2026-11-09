//! Independent search/read configuration, encrypted credentials and cancellable checks.
use crate::settings_store::{self, SearchConfigRow, SearchData, SearchKeyRow};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::{AppContext, Context, Entity, Subscription, Window};
use lumakit_ai::search::{SearchPreferences, SearchResult};
use lumakit_support::secrets::SavedKey;
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
mod network;
mod persistence;
mod stored;
#[cfg(test)]
mod tests;
mod view;
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
    /// Requests start from this key, then rotate in list order; `None` means the first key.
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
    editing: Option<String>,
    revealed: Option<(String, String)>,
}
impl Draft {
    fn new(config: &Config) -> Self {
        Self {
            endpoint: config.endpoint.clone(),
            username: config.username.clone(),
            key: String::new(),
            editing: None,
            revealed: None,
        }
    }
    fn clear(&mut self) {
        self.key.zeroize();
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
    receiver: mpsc::Receiver<Result<Output, String>>,
    discard: bool,
    check: Option<usize>,
}
enum KeyAction {
    Add,
    Replace(String, String),
    Delete(String),
    Select(String),
}
struct ProviderInputs {
    endpoint: Entity<InputState>,
    username: Entity<InputState>,
    key: Entity<InputState>,
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
        let mut stored = match settings_store::load_search(root)? {
            Some(data) => Stored::from_database(data)?,
            None => {
                let value = match lumakit_support::load_versioned::<Stored>(&root.join(FILE), 1)
                    .map_err(|_| "无法读取网络搜索设置；原文件未修改".to_owned())?
                {
                    Some((1, value)) => value,
                    Some(_) => return Err("网络搜索设置版本不受支持；未覆盖原文件".into()),
                    None => Stored::default(),
                };
                value.save(root)?;
                value
            }
        };
        // Add only newly supported providers; missing legacy rows remain a corruption error.
        let mut added = false;
        for provider in PROVIDERS {
            if !stored
                .configs
                .iter()
                .any(|config| config.provider == provider)
            {
                stored.configs.push(Config {
                    provider,
                    endpoint: provider.endpoint().into(),
                    keys: Vec::new(),
                    username: String::new(),
                    selected_key: None,
                });
                added = true;
            }
        }
        if added {
            stored.save(root)?;
        }
        let search_index = stored
            .configs
            .iter()
            .position(|c| c.provider == stored.default_search)
            .ok_or("缺少默认搜索配置")?;
        let fetch_index = stored
            .configs
            .iter()
            .position(|c| c.provider == stored.default_fetch)
            .ok_or("缺少默认读取配置")?;
        let count = stored.configs.len();
        Ok(Self {
            root: root.into(),
            drafts: stored.configs.iter().map(Draft::new).collect(),
            preferences: stored.preferences.clone(),
            compression: stored.compression,
            blacklist: stored.preferences.blacklist.join("\n"),
            stored,
            search_index,
            fetch_index,
            query: String::new(),
            target: String::new(),
            result: None,
            page: None,
            status: String::new(),
            job: None,
            inputs: None,
            advanced: false,
            delete_key: None,
            key_dialog: None,
            card_status: vec![String::new(); count],
            card_error: vec![false; count],
            previous_focus: None,
        })
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
            if let Some(index) = job.check {
                self.card_status[index] = "已取消".into();
                self.card_error[index] = false;
            }
        }
    }
    pub fn clear_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_and_clear();
        if let Some(inputs) = &self.inputs {
            for provider in &inputs.providers {
                provider
                    .key
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
        }
    }
    fn start_request(&mut self, fetch: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(inputs) = &self.inputs {
            self.query = inputs.query.read(cx).value().to_string();
            self.target = inputs.target.read(cx).value().to_string();
        }
        self.launch(fetch, None);
        let _ = window;
        cx.notify();
    }
    fn check_provider(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.is_busy() || !self.submit_card(index, cx) {
            return;
        }
        self.launch(self.stored.configs[index].provider.is_fetch(), Some(index));
        cx.notify();
    }
    fn launch(&mut self, fetch: bool, check: Option<usize>) {
        if self.is_busy() {
            return;
        }
        let index = check.unwrap_or(if fetch {
            self.fetch_index
        } else {
            self.search_index
        });
        let input = if check.is_some() {
            if fetch {
                "https://example.com".to_owned()
            } else {
                "web search".to_owned()
            }
        } else if fetch {
            self.target.trim().to_owned()
        } else {
            self.query.trim().to_owned()
        };
        if input.is_empty() || input.chars().count() > 4096 {
            self.status = "请输入 1–4096 字符的搜索词或 URL".into();
            return;
        }
        let config = self.stored.configs[index].clone();
        let mut prefs = self.stored.preferences.clone();
        if self.stored.compression == Compression::None {
            prefs.cutoff = usize::MAX;
        }
        if check.is_some() {
            self.card_status[index] = "正在检测…".into();
            self.card_error[index] = false;
        } else {
            self.status = if fetch {
                "正在读取…"
            } else {
                "正在搜索…"
            }
            .into();
            if fetch {
                self.page = None;
            } else {
                self.result = None;
            }
        }
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let outcome = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "无法创建网络运行时".to_owned())
                .and_then(|rt| {
                    rt.block_on(async move {
                        let operation = async {
                            if fetch {
                                fetch_page(config, input, prefs, worker_cancel)
                                    .await
                                    .map(Output::Fetch)
                            } else {
                                search_query(config, input, prefs, worker_cancel)
                                    .await
                                    .map(Output::Search)
                            }
                        };
                        tokio::time::timeout(
                            Duration::from_secs(if fetch { 20 } else { 15 }),
                            operation,
                        )
                        .await
                        .map_err(|_| "请求达到总超时，已停止".to_owned())?
                    })
                });
            let _ = sender.send(outcome);
        });
        self.job = Some(Job {
            cancel,
            thread: worker,
            receiver,
            discard: false,
            check,
        });
    }
    fn drain(&mut self) {
        if !self
            .job
            .as_ref()
            .is_some_and(|job| job.thread.is_finished())
        {
            return;
        }
        if let Some(job) = self.job.take() {
            let result = job.receiver.try_recv();
            let _ = job.thread.join();
            if job.discard {
                return;
            }
            if let Some(index) = job.check {
                let checked = match result {
                    Ok(Ok(Output::Search(result))) if !result.hits.is_empty() => {
                        Ok(result.hits.len())
                    }
                    Ok(Ok(Output::Fetch(_))) => Ok(1),
                    Ok(Ok(_)) => Err("检测未返回结果".into()),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err("网络工作线程异常结束".into()),
                };
                self.card_error[index] = checked.is_err();
                self.card_status[index] =
                    checked.map_or_else(|error| error, |count| format!("检测成功：{count} 条结果"));
            } else {
                match result {
                    Ok(Ok(Output::Search(result))) => {
                        self.status = format!("搜索完成：{} 条结果", result.hits.len());
                        self.result = Some(result);
                    }
                    Ok(Ok(Output::Fetch(page))) => {
                        self.status = "读取完成".into();
                        self.page = Some(page);
                    }
                    Ok(Err(error)) => self.status = error,
                    Err(_) => self.status = "网络工作线程异常结束".into(),
                }
            }
        }
    }
}
impl Drop for SearchSettings {
    fn drop(&mut self) {
        self.cancel_and_clear();
        if let Some(job) = self.job.take() {
            let _ = job.thread.join();
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
