//! 独立网络搜索/读取设置；不与模型线路共享明文、草稿或凭据。
use lumakit_support::secrets::SavedKey;
use gpui_kit::{div, prelude::*, AppContext, AnyElement, ClipboardItem, Context, Entity, Render, SharedString, Subscription, Window};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{button::{Button, ButtonVariants}, input::{Input, InputEvent, InputState, Textarea, TextareaState}, Disableable, Selectable, Sizable};
use lumakit_design::components::{label, IconButton};
use lumakit_design::components::settings as ui;
use lumakit_design::metrics::{radius, size, space, text};
use lumakit_design::Roles;
use futures_util::StreamExt;
use lumakit_ai::search::{self, SearchHit, SearchPreferences, SearchResult};
use lumakit_ai::search_adapters::SearchAdapterKind;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::mpsc, thread, time::Duration};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;
use crate::settings_store::{self, SearchConfigRow, SearchData, SearchKeyRow};

const FILE: &str = "search-settings.json";
const BLACKLIST_HEIGHT: gpui_kit::Pixels = gpui_kit::px(112.);
const VALUE_BOX: gpui_kit::Pixels = gpui_kit::px(32.);

/// A form field with an optional subtle help line under its control.
fn field(title: &'static str, note: Option<SharedString>, control: impl IntoElement, r: &Roles) -> AnyElement {
    ui::field(title, v_flex().gap(space::S4).child(control)
        .when_some(note, |this, note| this.child(label(text::METADATA, note).text_color(r.text_subtle))), r).into_any_element()
}

/// A settings group whose fields are stacked under its title.
fn fields(title: &'static str, rows: impl IntoIterator<Item = AnyElement>, r: &Roles) -> AnyElement {
    ui::group(title, None, r).child(v_flex().gap(space::S16).pt(space::S12).children(rows)).into_any_element()
}
const MAX_BODY: usize = 2 * 1024 * 1024;
const PROVIDERS: [Provider; 5] = [Provider::Tavily, Provider::Brave, Provider::Exa, Provider::Direct, Provider::Jina];

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum Provider { Tavily, Brave, Exa, Direct, Jina }
impl Provider {
    fn name(self) -> &'static str { match self { Self::Tavily => "Tavily", Self::Brave => "Brave Search", Self::Exa => "Exa", Self::Direct => "直接获取", Self::Jina => "Jina Reader" } }
    fn endpoint(self) -> &'static str { match self { Self::Tavily => "https://api.tavily.com", Self::Brave => "https://api.search.brave.com", Self::Exa => "https://api.exa.ai", Self::Direct => "", Self::Jina => "https://r.jina.ai" } }
    fn link(self) -> &'static str { match self { Self::Tavily => "https://app.tavily.com/home", Self::Brave => "https://brave.com/search/api/", Self::Exa => "https://dashboard.exa.ai/api-keys", Self::Direct => "https://developer.mozilla.org/zh-CN/docs/Web/HTTP", Self::Jina => "https://jina.ai/reader/" } }
    fn description(self) -> &'static str { match self { Self::Tavily => "面向 AI 的网页搜索，返回标题、链接及内容摘要。", Self::Brave => "Brave 独立索引搜索；上游单次最多返回 20 条。", Self::Exa => "Exa 搜索及网页文本摘要。", Self::Direct => "直接 GET 目标网页，不经第三方；仅提取文本，不运行脚本。地址可留空，读取时填写 URL。", Self::Jina => "将目标 URL 发送到 Jina Reader，返回实际网页文本。密钥可选，匿名限额由供应商决定。" } }
    fn kind(self) -> Option<SearchAdapterKind> { match self { Self::Tavily => Some(SearchAdapterKind::Tavily), Self::Brave => Some(SearchAdapterKind::Brave), Self::Exa => Some(SearchAdapterKind::Exa), _ => None } }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config { provider: Provider, endpoint: String, keys: Vec<SavedKey> }
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Compression { None, Cutoff }
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored { default_search: Provider, default_fetch: Provider, configs: Vec<Config>, preferences: SearchPreferences, compression: Compression }
impl Default for Stored {
    fn default() -> Self { Self { default_search: Provider::Tavily, default_fetch: Provider::Jina, configs: PROVIDERS.iter().map(|&p| Config { provider: p, endpoint: p.endpoint().into(), keys: Vec::new() }).collect(), preferences: SearchPreferences::default(), compression: Compression::None } }
}
impl Stored {
    fn from_database(data: SearchData) -> Result<Self, String> {
        let provider = |name: String| serde_json::from_value(serde_json::Value::String(name)).map_err(|_| "搜索数据库包含未知服务商".to_owned());
        let value = Self {
            default_search: provider(data.default_search)?, default_fetch: provider(data.default_fetch)?,
            configs: data.configs.into_iter().map(|config| Ok(Config { provider: provider(config.provider)?, endpoint: config.endpoint,
                keys: config.keys.into_iter().map(|key| SavedKey { id: key.id, masked: key.masked, ciphertext: key.ciphertext }).collect() })).collect::<Result<_, String>>()?,
            preferences: serde_json::from_value(data.preferences).map_err(|_| "搜索数据库偏好无效".to_owned())?,
            compression: serde_json::from_value(serde_json::Value::String(data.compression)).map_err(|_| "搜索数据库压缩方式无效".to_owned())?,
        };
        validate_stored(&value)?;
        Ok(value)
    }
    fn save(&self, root: &Path) -> Result<(), String> {
        validate_stored(self)?;
        let name = |provider: Provider| match provider { Provider::Tavily => "tavily", Provider::Brave => "brave", Provider::Exa => "exa", Provider::Direct => "direct", Provider::Jina => "jina" }.to_owned();
        settings_store::save_search(root, &SearchData {
            default_search: name(self.default_search), default_fetch: name(self.default_fetch),
            configs: self.configs.iter().map(|config| SearchConfigRow { provider: name(config.provider), endpoint: config.endpoint.clone(),
                keys: config.keys.iter().map(|key| SearchKeyRow { id: key.id.clone(), masked: key.masked.clone(), ciphertext: key.ciphertext.clone() }).collect() }).collect(),
            preferences: serde_json::to_value(&self.preferences).map_err(|_| "搜索偏好序列化失败".to_owned())?,
            compression: match self.compression { Compression::None => "none", Compression::Cutoff => "cutoff" }.into(),
        })
    }
}
struct Draft { endpoint: String, key: String, editing: Option<String>, edit_value: String, revealed: Option<(String, String)> }
impl Draft {
    fn new(endpoint: &str) -> Self { Self { endpoint: endpoint.into(), key: String::new(), editing: None, edit_value: String::new(), revealed: None } }
    fn clear(&mut self) { self.key.zeroize(); self.edit_value.zeroize(); self.editing = None; if let Some((_, mut value)) = self.revealed.take() { value.zeroize(); } }
}
impl Drop for Draft { fn drop(&mut self) { self.clear(); } }
#[derive(Debug)]
struct Page { title: String, url: String, body: String }
enum Output { Search(SearchResult), Fetch(Page) }
struct Job { cancel: CancellationToken, thread: thread::JoinHandle<()>, receiver: mpsc::Receiver<Result<Output, String>>, discard: bool }
enum KeyAction { Add, Replace(String, String), Delete(String) }

struct ProviderInputs { endpoint: Entity<InputState>, key: Entity<InputState>, edit: Entity<InputState> }
struct SearchInputs {
    providers: Vec<ProviderInputs>, query: Entity<InputState>, target: Entity<InputState>,
    max_results: Entity<InputState>, cutoff: Entity<InputState>, blacklist: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
}

pub struct SearchSettings {
    root: PathBuf, stored: Stored, drafts: Vec<Draft>, search_index: usize, fetch_index: usize,
    preferences: SearchPreferences, compression: Compression, blacklist: String,
    query: String, target: String, result: Option<SearchResult>, page: Option<Page>, status: String, job: Option<Job>,
    inputs: Option<SearchInputs>, advanced: bool, delete_key: Option<(usize, String)>,
}
impl SearchSettings {
    pub fn create(root: &Path, window: &mut Window, cx: &mut gpui_kit::App) -> Result<Entity<Self>, String> {
        let this = Self::load(root)?;
        Ok(cx.new(|cx| this.with_inputs(window, cx)))
    }
    fn with_inputs(mut self, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = &mut self;
        let mut subscriptions = Vec::new();
        let mut input = |value: String, secret: bool, window: &mut Window, cx: &mut Context<Self>| {
            let state = cx.new(|cx| InputState::new(window, cx).default_value(value).masked(secret));
            subscriptions.push(cx.subscribe(&state, |_, _, event: &InputEvent, cx| { if matches!(event, InputEvent::Change) { cx.notify(); } }));
            state
        };
        let providers = this.drafts.iter().map(|draft| ProviderInputs {
            endpoint: input(draft.endpoint.clone(), false, window, cx), key: input(String::new(), true, window, cx), edit: input(String::new(), true, window, cx),
        }).collect();
        let query = input(String::new(), false, window, cx);
        let target = input(String::new(), false, window, cx);
        let max_results = input(this.preferences.max_results.to_string(), false, window, cx);
        let cutoff = input(this.preferences.cutoff.to_string(), false, window, cx);
        let blacklist = cx.new(|cx| TextareaState::new(window, cx).default_value(this.blacklist.clone()));
        subscriptions.push(cx.subscribe(&blacklist, |_, _, event: &InputEvent, cx| { if matches!(event, InputEvent::Change) { cx.notify(); } }));
        this.inputs = Some(SearchInputs { providers, query, target, max_results, cutoff, blacklist, _subscriptions: subscriptions });
        self
    }
    fn load(root: &Path) -> Result<Self, String> {
        let stored = match settings_store::load_search(root)? {
            Some(data) => Stored::from_database(data)?,
            None => {
                let value = match lumakit_support::load_versioned::<Stored>(&root.join(FILE), 1).map_err(|_| "无法读取网络搜索设置；原文件未修改，请检查文件版本或权限".to_owned())? {
                    Some((1, value)) => value,
                    Some(_) => return Err("网络搜索设置版本不受支持；未覆盖原文件".into()),
                    None => Stored::default(),
                };
                value.save(root)?;
                value
            }
        };
        let drafts = stored.configs.iter().map(|c| Draft::new(&c.endpoint)).collect();
        let search_index = stored.configs.iter().position(|c| c.provider == stored.default_search).ok_or("缺少默认搜索配置")?;
        let fetch_index = stored.configs.iter().position(|c| c.provider == stored.default_fetch).ok_or("缺少默认读取配置")?;
        Ok(Self { root: root.to_owned(), preferences: stored.preferences.clone(), compression: stored.compression, blacklist: stored.preferences.blacklist.join("\n"), stored, drafts, search_index, fetch_index, query: String::new(), target: String::new(), result: None, page: None, status: String::new(), job: None, inputs: None, advanced: false, delete_key: None })
    }
    fn cancel_and_clear(&mut self) { for d in &mut self.drafts { d.clear(); } self.delete_key = None; if let Some(job) = &mut self.job { job.cancel.cancel(); job.discard = true; } }
    pub fn clear_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_and_clear();
        if let Some(inputs) = &self.inputs { for provider in &inputs.providers {
            provider.key.update(cx, |input, cx| input.set_value("", window, cx));
            provider.edit.update(cx, |input, cx| input.set_value("", window, cx));
        } }
        cx.notify();
    }
    pub fn is_busy(&self) -> bool { self.job.is_some() }
    pub fn poll(&mut self) -> bool {
        if !self.job.as_ref().is_some_and(|job| job.thread.is_finished()) { return false; }
        self.drain(); true
    }
    fn read_inputs(&mut self, cx: &Context<Self>) -> bool {
        let inputs = self.inputs.as_ref().expect("search entity inputs");
        for (draft, input) in self.drafts.iter_mut().zip(&inputs.providers) {
            draft.endpoint = input.endpoint.read(cx).value().to_string();
            draft.key.zeroize(); draft.key = input.key.read(cx).value().to_string();
            draft.edit_value.zeroize(); draft.edit_value = input.edit.read(cx).value().to_string();
        }
        self.query = inputs.query.read(cx).value().to_string();
        self.target = inputs.target.read(cx).value().to_string();
        self.blacklist = inputs.blacklist.read(cx).value().to_string();
        let max_results = inputs.max_results.read(cx).value().trim().parse();
        let cutoff = inputs.cutoff.read(cx).value().trim().parse();
        match (max_results, cutoff) {
            (Ok(max_results), Ok(cutoff)) => { self.preferences.max_results = max_results; self.preferences.cutoff = cutoff; true }
            _ => { self.status = "结果数须为 1–50，截断长度须为 100–20000 的整数".into(); false }
        }
    }
    fn submit(&mut self, index: usize, mut action: Option<KeyAction>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.read_inputs(cx) {
            if let Some(KeyAction::Replace(_, value)) = &mut action { value.zeroize(); }
            return false;
        }
        if !self.save_clicked(index, action) { return false; }
        let inputs = self.inputs.as_ref().expect("search entity inputs");
        if self.drafts[index].key.is_empty() { inputs.providers[index].key.update(cx, |input, cx| input.set_value("", window, cx)); }
        inputs.blacklist.update(cx, |input, cx| input.set_value(self.blacklist.clone(), window, cx));
        true
    }
    fn clear_provider(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.drafts[index].clear(); self.delete_key = None;
        let input = &self.inputs.as_ref().expect("search entity inputs").providers[index];
        input.key.update(cx, |input, cx| input.set_value("", window, cx));
        input.edit.update(cx, |input, cx| input.set_value("", window, cx));
    }
    fn save(&mut self, index: usize, action: Option<KeyAction>) -> Result<(), String> {
        let mut next = self.stored.clone();
        next.default_search = next.configs[self.search_index].provider;
        next.default_fetch = next.configs[self.fetch_index].provider;
        for (config, draft) in next.configs.iter_mut().zip(&self.drafts) { config.endpoint = draft.endpoint.trim().to_owned(); }
        next.preferences = preferences_from_draft(&self.preferences, &self.blacklist)?;
        next.preferences.model_tools_preferred = false;
        next.compression = self.compression;
        let mut clear_added = false;
        match action {
            Some(KeyAction::Add) => {
                if self.drafts[index].key.trim().is_empty() { return Err("请输入需要添加的密钥".into()); }
                if next.configs[index].keys.len() >= 16 { return Err("每个服务商最多 16 个密钥".into()); }
                next.configs[index].keys.push(SavedKey::new(&self.drafts[index].key)?); clear_added = true;
            }
            Some(KeyAction::Replace(id, value)) => {
                let value = zeroize::Zeroizing::new(value);
                next.configs[index].keys.iter_mut().find(|k| k.id == id).ok_or("密钥不存在")?.replace(&value)?;
            }
            Some(KeyAction::Delete(id)) => next.configs[index].keys.retain(|k| k.id != id),
            None => {
                if !self.drafts[index].key.trim().is_empty() {
                    if next.configs[index].keys.len() >= 16 { return Err("每个服务商最多 16 个密钥".into()); }
                    next.configs[index].keys.push(SavedKey::new(&self.drafts[index].key)?); clear_added = true;
                }
            }
        }
        validate_stored(&next)?;
        next.save(&self.root).map_err(|error| format!("保存失败：{error}；草稿和原凭据均已保留"))?;
        self.preferences = next.preferences.clone(); self.blacklist = self.preferences.blacklist.join("\n"); self.stored = next;
        if clear_added { self.drafts[index].key.zeroize(); }
        Ok(())
    }
    fn save_clicked(&mut self, index: usize, action: Option<KeyAction>) -> bool {
        match self.save(index, action) { Ok(()) => { self.status = "已保存".into(); true }, Err(error) => { self.status = error; false } }
    }
    fn begin(&mut self, fetch: bool) {
        let index = if fetch { self.fetch_index } else { self.search_index };
        if self.job.is_some() || !self.save_clicked(index, None) { return; }
        let input = if fetch { if self.target.trim().is_empty() { self.stored.configs[index].endpoint.clone() } else { self.target.trim().to_owned() } } else { self.query.trim().to_owned() };
        if input.is_empty() || input.chars().count() > 4096 { self.status = "请输入 1–4096 字符的搜索词或 URL".into(); return; }
        let config = self.stored.configs[index].clone(); let mut prefs = self.preferences.clone();
        if self.compression == Compression::None { prefs.cutoff = usize::MAX; }
        let cancel = CancellationToken::new(); let worker_cancel = cancel.clone(); let (sender, receiver) = mpsc::channel();
        self.result = None; self.page = None; self.status = if fetch { "正在读取…".into() } else { "正在搜索…".into() };
        let worker = thread::spawn(move || {
            let outcome = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "无法创建网络运行时".to_owned()).and_then(|rt| rt.block_on(async move {
                let operation = async move {
                    if fetch { fetch_page(config, input, prefs, worker_cancel).await.map(Output::Fetch) }
                    else { search_query(config, input, prefs, worker_cancel).await.map(Output::Search) }
                };
                tokio::time::timeout(Duration::from_secs(if fetch { 20 } else { 15 }), operation).await.map_err(|_| "请求达到总超时，已停止；未继续重试".to_owned())?
            }));
            let _ = sender.send(outcome);
        });
        self.job = Some(Job { cancel, thread: worker, receiver, discard: false });
    }
    fn drain(&mut self) {
        let ready = self.job.as_ref().is_some_and(|job| job.thread.is_finished());
        if !ready { return; }
        if let Some(job) = self.job.take() {
            let result = job.receiver.try_recv(); let _ = job.thread.join();
            if job.discard { self.status = "已取消".into(); return; }
            match result {
                Ok(Ok(Output::Search(result))) => { self.status = if result.hits.is_empty() { "搜索完成，没有结果（或结果被黑名单过滤）".into() } else { format!("搜索完成：{} 条结果", result.hits.len()) }; self.result = Some(result); }
                Ok(Ok(Output::Fetch(page))) => { self.status = if page.body.trim().is_empty() { "读取完成，正文为空".into() } else { "读取完成".into() }; self.page = Some(page); }
                Ok(Err(error)) => self.status = error,
                Err(_) => self.status = "网络工作线程异常结束".into(),
            }
        }
    }
    fn provider_ui(&self, fetch: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let busy = self.is_busy();
        let index = if fetch { self.fetch_index } else { self.search_index };
        let provider = self.stored.configs[index].provider;
        let inputs = &self.inputs.as_ref().expect("search entity inputs").providers[index];
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let mut choices = h_flex().flex_wrap().gap(space::S8);
        for (i, config) in self.stored.configs.iter().enumerate().filter(|(_, config)| config.provider.kind().is_none() == fetch) {
            // Outline for the current provider, ghost for the others: Kit's `selected` alone barely changes a dark theme.
            let current = i == index;
            choices = choices.child(Button::new(("provider", i)).small().label(config.provider.name()).when(current, |b| b.outline().icon(IconName::Check)).when(!current, |b| b.ghost()).selected(current).disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let old = if fetch { this.fetch_index } else { this.search_index };
                    if old != i { this.clear_provider(old, window, cx); if fetch { this.fetch_index = i; } else { this.search_index = i; } }
                    cx.notify();
                })));
        }
        let site = Button::new(("provider-site", index)).small().link().icon(IconName::ExternalLink)
            .label(if provider == Provider::Direct { "HTTP 说明" } else { "官方网站 / 获取密钥" }).on_click(move |_, _, cx| cx.open_url(provider.link()));
        let mut rows = vec![
            field("默认服务商", Some(provider.description().into()), choices.child(site), &r),
            field(if provider == Provider::Direct { "默认读取 URL（可留空）" } else { "服务地址" }, None, Input::new(&inputs.endpoint).disabled(busy), &r),
        ];
        if provider != Provider::Direct {
            let mut keys = v_flex().gap(space::S8);
            for (key_index, key) in self.stored.configs[index].keys.iter().enumerate() {
                keys = keys.child(self.key_row(index, key_index, key, busy, cx));
            }
            keys = keys.child(h_flex().gap(space::S8).child(div().flex_1().min_w_0().child(Input::new(&inputs.key).disabled(busy)))
                .child(Button::new(("add-key", index)).small().outline().icon(IconName::Plus).label("添加密钥").disabled(busy).on_click(cx.listener(move |this, _, window, cx| { this.submit(index, Some(KeyAction::Add), window, cx); cx.notify(); }))));
            rows.push(field("API 密钥", Some("留空保留已有凭据；仅在 HTTP 401/403/429 时轮换其余密钥，最多 16 次，不跨服务商重试。".into()), keys, &r));
        }
        rows.push(h_flex().justify_end().child(Button::new(("save-provider", index)).small().primary().label("保存设置").disabled(busy)
            .on_click(cx.listener(move |this, _, window, cx| { this.submit(index, None, window, cx); cx.notify(); }))).into_any_element());
        fields(if fetch { "网页读取" } else { "网页搜索" }, rows, &r)
    }

    /// One saved key: masked value with icon actions; reveal, edit and delete expand below it.
    fn key_row(&self, index: usize, key_index: usize, key: &SavedKey, busy: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let inputs = &self.inputs.as_ref().expect("search entity inputs").providers[index];
        let id = index * 16 + key_index;
        let revealed = self.drafts[index].revealed.as_ref().filter(|(revealed, _)| revealed == &key.id).map(|(_, value)| value.clone());
        let actions = h_flex().gap(space::S2)
            .child(IconButton::new(("reveal-key", id), if revealed.is_some() { IconName::EyeOff } else { IconName::Eye }).tooltip(if revealed.is_some() { "隐藏密钥" } else { "显示密钥" }).disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let key = &this.stored.configs[index].keys[key_index];
                    if this.drafts[index].revealed.as_ref().is_some_and(|(id, _)| id == &key.id) {
                        if let Some((_, mut value)) = this.drafts[index].revealed.take() { value.zeroize(); }
                    } else { match key.open() {
                        Ok(value) => { if let Some((_, mut old)) = this.drafts[index].revealed.take() { old.zeroize(); } this.drafts[index].revealed = Some((key.id.clone(), value.to_string())); }
                        Err(error) => this.status = error,
                    } }
                    cx.notify();
                })))
            .child(IconButton::new(("copy-key", id), IconName::Copy).tooltip("复制密钥").disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
                match this.stored.configs[index].keys[key_index].open() { Ok(value) => cx.write_to_clipboard(ClipboardItem::new_string(value.to_string())), Err(error) => this.status = error } cx.notify();
            })))
            .child(IconButton::new(("edit-key", id), IconName::SquarePen).tooltip("修改密钥").disabled(busy).on_click(cx.listener(move |this, _, window, cx| {
                this.clear_provider(index, window, cx); this.drafts[index].editing = Some(this.stored.configs[index].keys[key_index].id.clone()); cx.notify();
            })))
            .child(IconButton::new(("delete-key", id), IconName::Trash).tooltip("删除密钥").disabled(busy).on_click(cx.listener(move |this, _, _, cx| { this.delete_key = Some((index, this.stored.configs[index].keys[key_index].id.clone())); cx.notify(); })));
        let mut row = v_flex().gap(space::S8)
            .child(h_flex().h(VALUE_BOX).px(space::S8).gap(space::S8).rounded(radius::MD).border_1().border_color(r.border_default)
                .child(label(text::CODE_TEXT, SharedString::from(revealed.unwrap_or_else(|| key.masked.clone()))).flex_1().min_w_0().truncate()).child(actions));
        if self.delete_key.as_ref().is_some_and(|(i, key_id)| *i == index && key_id == &key.id) {
            row = row.child(h_flex().flex_wrap().gap(space::S8).child(label(text::METADATA, "确认删除此密钥？").flex_1().text_color(r.feedback_error))
                .child(Button::new(("cancel-delete", id)).small().ghost().label("取消").on_click(cx.listener(|this, _, _, cx| { this.delete_key = None; cx.notify(); })))
                .child(Button::new(("confirm-delete", id)).small().danger().label("确认删除").disabled(busy).on_click(cx.listener(move |this, _, window, cx| {
                    let key_id = this.stored.configs[index].keys[key_index].id.clone();
                    if this.submit(index, Some(KeyAction::Delete(key_id)), window, cx) { this.clear_provider(index, window, cx); } cx.notify();
                }))));
        }
        if self.drafts[index].editing.as_deref() == Some(key.id.as_str()) {
            row = row.child(Input::new(&inputs.edit).disabled(busy)).child(h_flex().justify_end().gap(space::S8)
                .child(Button::new(("cancel-key-edit", id)).small().ghost().label("取消修改").on_click(cx.listener(move |this, _, window, cx| { this.clear_provider(index, window, cx); cx.notify(); })))
                .child(Button::new(("save-key-edit", id)).small().primary().label("保存修改").disabled(busy).on_click(cx.listener(move |this, _, window, cx| {
                    let key_id = this.stored.configs[index].keys[key_index].id.clone();
                    let value = this.inputs.as_ref().expect("search entity inputs").providers[index].edit.read(cx).value().to_string();
                    if this.submit(index, Some(KeyAction::Replace(key_id, value)), window, cx) { this.clear_provider(index, window, cx); } cx.notify();
                }))));
        }
        row
    }
}

impl Render for SearchSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.is_busy();
        let inputs = self.inputs.as_ref().expect("search entity inputs");
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let run = |input: &Entity<InputState>, id: &'static str, label: &'static str, icon: IconName, fetch: bool, cx: &mut Context<Self>| {
            h_flex().gap(space::S8).child(div().flex_1().min_w_0().child(Input::new(input).disabled(busy)))
                .child(Button::new(id).small().outline().icon(icon).label(label).disabled(busy).on_click(cx.listener(move |this, _, window, cx| { this.start_request(fetch, window, cx); })))
        };
        let try_rows = vec![
            field("搜索词", None, run(&inputs.query, "run-search", "搜索 / 检测", IconName::Search, false, cx), &r),
            field("读取 URL", Some("HTTPS 或回环 HTTP；留空使用默认 URL。".into()), run(&inputs.target, "run-fetch", "读取 / 检测", IconName::Globe, true, cx), &r),
        ];
        let mut body = v_flex().id("search-settings").w_full().max_w(size::CONTENT_MAX).gap(space::S24)
            .child(ui::page_header("网络搜索", Some("仅点击搜索或读取时联网；搜索词和读取 URL 会发送给所选服务。当前桌面宿主未接入模型原生搜索路由。".into()), &r))
            .child(self.provider_ui(false, cx)).child(self.provider_ui(true, cx))
            .child(fields("检测", try_rows, &r));
        let toggle = Button::new("advanced-search").small().ghost().icon(if self.advanced { IconName::ChevronDown } else { IconName::ChevronRight })
            .label("高级设置").on_click(cx.listener(|this, _, _, cx| { this.advanced = !this.advanced; cx.notify(); }));
        body = body.child(h_flex().child(toggle));
        if self.advanced {
            let compression = h_flex().flex_wrap().gap(space::S8)
                .child(Button::new("no-cutoff").small().label("不截断").selected(self.compression == Compression::None).disabled(busy).on_click(cx.listener(|this, _, _, cx| { this.compression = Compression::None; cx.notify(); })))
                .child(Button::new("use-cutoff").small().label("按 Unicode 字符截断").selected(self.compression == Compression::Cutoff).disabled(busy).on_click(cx.listener(|this, _, _, cx| { this.compression = Compression::Cutoff; cx.notify(); })));
            body = body.child(v_flex().gap(space::S16)
                .child(field("结果数", Some("1–50。".into()), Input::new(&inputs.max_results).disabled(busy), &r))
                .child(field("正文压缩", None, compression, &r))
                .child(field("截断长度", Some("100–20000 个字符。".into()), Input::new(&inputs.cutoff).disabled(busy || self.compression != Compression::Cutoff), &r))
                .child(field("黑名单", Some("每行一条，最多 32 条、每条 256 字符；域名含子域，URL 路径区分大小写。只控制本页外部搜索/读取。".into()), Textarea::new(&inputs.blacklist).h(BLACKLIST_HEIGHT).disabled(busy), &r))
                .child(h_flex().justify_end().child(Button::new("save-search-advanced").small().primary().label("保存高级设置及默认服务商").disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| { this.submit(this.search_index, None, window, cx); cx.notify(); })))));
        }
        if busy || !self.status.is_empty() {
            body = body.child(h_flex().gap(space::S8).child(label(text::METADATA, self.status.clone()).flex_1().text_color(r.text_subtle))
                .when(busy, |this| this.child(Button::new("cancel-search").small().ghost().icon(IconName::Close).label("取消请求").on_click(cx.listener(|this, _, _, cx| { if let Some(job) = &mut this.job { job.cancel.cancel(); } cx.notify(); })))));
        }
        if let Some(result) = &self.result {
            let hits = result.hits.iter().enumerate().map(|(i, hit)| {
                let url = hit.url.clone();
                v_flex().gap(space::S4).py(space::S12).border_b_1().border_color(r.border_muted)
                    .child(label(text::UI, if hit.title.is_empty() { "（无标题）".to_owned() } else { hit.title.clone() }))
                    .child(h_flex().child(Button::new(("search-hit", i)).small().link().label(hit.url.clone()).on_click(move |_, _, cx| cx.open_url(&url))))
                    .child(label(text::METADATA, hit.snippet.clone()).text_color(r.text_subtle))
            }).collect::<Vec<_>>();
            if !hits.is_empty() { body = body.child(ui::group("搜索结果", None, &r).children(hits)); }
        }
        if let Some(result) = &self.page {
            let url = result.url.clone();
            body = body.child(ui::group("读取结果", None, &r).child(v_flex().gap(space::S8).pt(space::S12).child(label(text::UI, result.title.clone()))
                .child(h_flex().child(Button::new("fetched-url").small().link().label(result.url.clone()).on_click(move |_, _, cx| cx.open_url(&url))))
                .child(label(text::READING, result.body.clone()))));
        }
        body.on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| { if event.keystroke.key == "escape" { this.clear_secrets(window, cx); } }))
    }
}

impl SearchSettings {
    fn start_request(&mut self, fetch: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_inputs(cx) {
            self.begin(fetch);
            let index = if fetch { self.fetch_index } else { self.search_index };
            if self.drafts[index].key.is_empty() { self.inputs.as_ref().expect("search entity inputs").providers[index].key.update(cx, |input, cx| input.set_value("", window, cx)); }
        }
        cx.notify();
    }
}
impl Drop for SearchSettings { fn drop(&mut self) { self.cancel_and_clear(); if let Some(job) = self.job.take() { let _ = job.thread.join(); } } }

fn validate_url(raw: &str) -> Result<Url, String> {
    if raw.len() > 4096 { return Err("URL 不得超过 4096 字节".into()); }
    let url = search::validate_search_url(raw).map_err(|_| "URL 必须为 HTTPS 或回环 HTTP，且不能包含用户名/密码".to_owned())?;
    if url.query().is_some() || url.fragment().is_some() { return Err("URL 不得包含查询参数或片段".into()); }
    Ok(url)
}
fn validate_stored(value: &Stored) -> Result<(), String> {
    if value.configs.len() != PROVIDERS.len() || PROVIDERS.iter().any(|p| value.configs.iter().filter(|c| c.provider == *p).count() != 1) { return Err("网络搜索配置缺失或重复；原文件未修改".into()); }
    if value.default_search.kind().is_none() || value.default_fetch.kind().is_some() { return Err("默认服务商与能力不匹配".into()); }
    preferences_from_draft(&value.preferences, &value.preferences.blacklist.join("\n"))?;
    for c in &value.configs {
        if !(c.provider == Provider::Direct && c.endpoint.is_empty()) { validate_url(&c.endpoint)?; }
        if c.keys.len() > 16 { return Err("每个服务商最多 16 个密钥".into()); }
        let mut seen = std::collections::HashSet::new();
        for k in &c.keys { if k.id.is_empty() || k.ciphertext.is_empty() || !seen.insert(&k.id) { return Err("密钥配置无效；原文件未修改".into()); } }
    }
    Ok(())
}
fn preferences_from_draft(prefs: &SearchPreferences, text: &str) -> Result<SearchPreferences, String> {
    if !(1..=50).contains(&prefs.max_results) || !(100..=20_000).contains(&prefs.cutoff) { return Err("结果数须为 1–50，截断长度须为 100–20000".into()); }
    let rules = text.lines().map(str::trim).filter(|s| !s.is_empty()).map(normalize_rule).collect::<Result<Vec<_>, _>>()?;
    if rules.len() > 32 { return Err("黑名单最多 32 条".into()); }
    let mut next = prefs.clone(); next.blacklist = rules; Ok(next)
}
fn normalize_rule(raw: &str) -> Result<String, String> {
    let invalid = || "黑名单规则无效：仅支持域名、域名通配和完整 HTTP(S) URL 通配；不能含空白/凭据/片段".to_owned();
    if raw.chars().count() > 256 || raw.is_empty() || !raw.is_ascii() || raw.chars().any(|c| c.is_whitespace() || c.is_control()) { return Err(invalid()); }
    if let Some((scheme, rest)) = raw.split_once("://") {
        if !matches!(scheme, "http" | "https" | "*") || rest.contains('@') || rest.contains('#') || rest.contains('\\') { return Err(invalid()); }
        let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
        let host = authority.split_once(':').map_or(authority, |(host, _)| host);
        validate_host_pattern(host)?;
        if let Some((_, port)) = authority.split_once(':') { if port.parse::<u16>().ok().filter(|p| *p != 0).is_none() { return Err(invalid()); } }
        if path.contains(':') || path.contains('"') || path.contains('<') || path.contains('>') { return Err(invalid()); }
        Ok(format!("{scheme}://{}/{}", authority.to_ascii_lowercase(), path))
    } else { validate_host_pattern(raw)?; Ok(raw.to_ascii_lowercase()) }
}
fn validate_host_pattern(host: &str) -> Result<(), String> {
    if host.is_empty() || host.len() > 253 || !host.contains('.') && !host.eq_ignore_ascii_case("localhost") { return Err("无效黑名单域名".into()); }
    for label in host.split('.') { if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') || !label.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'*' | b'?')) { return Err("无效黑名单域名通配".into()); } }
    Ok(())
}
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes()); let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < t.len() { if i < p.len() && (p[i] == b'?' || p[i] == t[j]) { i += 1; j += 1; } else if i < p.len() && p[i] == b'*' { star = Some(i); i += 1; retry = j; } else if let Some(s) = star { retry += 1; j = retry; i = s + 1; } else { return false; } }
    while i < p.len() && p[i] == b'*' { i += 1; } i == p.len()
}
fn blocked(url: &Url, rules: &[String]) -> bool {
    let host = url.host_str().unwrap_or_default().trim_end_matches('.').to_ascii_lowercase();
    rules.iter().any(|rule| { if rule.contains("://") { glob(rule, url.as_str()) } else if rule.contains(['*', '?']) { glob(rule, &host) } else { host == *rule || host.strip_suffix(rule.as_str()).is_some_and(|s| s.ends_with('.')) } })
}
async fn search_query(config: Config, query: String, preferences: SearchPreferences, cancel: CancellationToken) -> Result<SearchResult, String> {
    let mut endpoint = validate_url(&config.endpoint)?;
    let kind = config.provider.kind().ok_or("此服务商不提供搜索")?;
    if endpoint.path() == "/" || endpoint.path().is_empty() { endpoint.set_path(if kind == SearchAdapterKind::Brave { "/res/v1/web/search" } else { "/search" }); }
    if config.keys.is_empty() { return Err("请先填写并保存搜索密钥".into()); }
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(15)).build().map_err(|_| "无法创建搜索客户端".to_owned())?;
    // 复用 LumaKit SearchHttpAdapter 的请求合同；状态在本层处理，避免从 AiError 文本猜测是否轮换密钥。
    for (i, saved) in config.keys.iter().enumerate().take(16) {
        let key = saved.open()?;
        let (header_name, prefix) = match kind { SearchAdapterKind::Tavily => ("authorization", "Bearer "), SearchAdapterKind::Brave => ("x-subscription-token", ""), SearchAdapterKind::Exa => ("x-api-key", "") };
        let mut header = reqwest::header::HeaderValue::from_str(&format!("{prefix}{}", key.as_str())).map_err(|_| "密钥不适合作为认证头".to_owned())?; header.set_sensitive(true);
        // 先取上游允许范围，再黑名单过滤，最后应用用户结果数；不让被过滤的首条挤掉后面的可用结果。
        let request = match kind {
            SearchAdapterKind::Tavily => client.post(endpoint.clone()).json(&serde_json::json!({"query": query, "max_results": 50})),
            SearchAdapterKind::Brave => client.get(endpoint.clone()).query(&[("q", query.as_str()), ("count", "20")]),
            SearchAdapterKind::Exa => client.post(endpoint.clone()).json(&serde_json::json!({"query": query, "numResults": 50, "contents": {"text": true}})),
        }.header(reqwest::header::ACCEPT, "application/json").header(header_name, header);
        let response = tokio::select! { _ = cancel.cancelled() => return Err("已取消".into()), r = request.send() => r.map_err(|_| "搜索请求失败或超时；未轮换密钥".to_owned())? };
        let status = response.status();
        if !status.is_success() { if matches!(status.as_u16(), 401 | 403 | 429) && i + 1 < config.keys.len().min(16) { continue; } return Err(format!("搜索失败（HTTP {}）；未跟随重定向", status.as_u16())); }
        if response.content_length().is_some_and(|n| n > MAX_BODY as u64) { return Err("搜索响应超过 2 MiB".into()); }
        let mut stream = response.bytes_stream(); let mut bytes = Vec::new();
        while let Some(chunk) = tokio::select! { _ = cancel.cancelled() => return Err("已取消".into()), c = stream.next() => c } { let chunk = chunk.map_err(|_| "搜索响应读取失败".to_owned())?; if bytes.len() + chunk.len() > MAX_BODY { return Err("搜索响应超过 2 MiB".into()); } bytes.extend_from_slice(&chunk); }
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "搜索响应不是有效 JSON".to_owned())?;
        if value.get("error").is_some_and(|e| !e.is_null()) || value.get("success").and_then(|v| v.as_bool()) == Some(false) { return Err("供应商返回业务错误；未轮换密钥".into()); }
        let rows = (if kind == SearchAdapterKind::Brave { value.pointer("/web/results") } else { value.get("results") }).and_then(|v| v.as_array()).ok_or("搜索响应缺少结果列表")?;
        let mut hits = Vec::new();
        for row in rows {
            let url = row.get("url").and_then(|v| v.as_str()).ok_or("搜索结果缺少 URL")?;
            let snippet_field = match kind { SearchAdapterKind::Tavily => "content", SearchAdapterKind::Brave => "description", SearchAdapterKind::Exa => "text" };
            if Url::parse(url).is_ok_and(|url| !blocked(&url, &preferences.blacklist)) { hits.push(SearchHit { title: row.get("title").and_then(|v| v.as_str()).unwrap_or("").to_owned(), url: url.to_owned(), snippet: row.get(snippet_field).and_then(|v| v.as_str()).unwrap_or("").to_owned() }); }
        }
        let mut post = preferences.clone(); post.blacklist.clear();
        return Ok(search::process_result(SearchResult { query, provider: kind.id().into(), hits }, &post));
    }
    Err("没有可用搜索密钥".into())
}
async fn fetch_page(config: Config, target: String, prefs: SearchPreferences, cancel: CancellationToken) -> Result<Page, String> {
    let url = validate_url(&target)?;
    if blocked(&url, &prefs.blacklist) { return Err("目标地址命中黑名单，未发起请求".into()); }
    if config.provider == Provider::Direct && !config.keys.is_empty() { return Err("直接获取不发送 API 密钥".into()); }
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20)).build().map_err(|_| "无法创建读取客户端".to_owned())?;
    let endpoint = if config.provider == Provider::Jina { validate_url(&config.endpoint)?; format!("{}/{}", config.endpoint.trim_end_matches('/'), target) } else { target.clone() };
    let attempts = config.keys.len().max(1).min(16);
    for i in 0..attempts {
        let mut request = client.get(&endpoint).header(reqwest::header::ACCEPT, "text/plain, text/html;q=0.8");
        if config.provider == Provider::Jina { request = request.header("X-Return-Format", "text"); }
        let secret = config.keys.get(i).map(SavedKey::open).transpose()?;
        if let Some(key) = &secret { let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key.as_str())).map_err(|_| "密钥不适合作为认证头".to_owned())?; header.set_sensitive(true); request = request.header(reqwest::header::AUTHORIZATION, header); }
        let response = tokio::select! { _ = cancel.cancelled() => return Err("已取消".into()), r = request.send() => r.map_err(|_| "读取请求失败或超时".to_owned())? };
        let status = response.status();
        if !status.is_success() { if matches!(status.as_u16(), 401 | 403 | 429) && i + 1 < attempts { continue; } return Err(format!("读取失败（HTTP {}）；未跟随重定向", status.as_u16())); }
        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        if !content_type.starts_with("text/") && !content_type.contains("xhtml") { return Err("读取仅支持文本或 HTML 响应".into()); }
        if response.content_length().is_some_and(|n| n > MAX_BODY as u64) { return Err("读取响应超过 2 MiB".into()); }
        let mut stream = response.bytes_stream(); let mut bytes = Vec::new();
        while let Some(chunk) = tokio::select! { _ = cancel.cancelled() => return Err("已取消".into()), chunk = stream.next() => chunk } { let chunk = chunk.map_err(|_| "读取正文失败".to_owned())?; if bytes.len() + chunk.len() > MAX_BODY { return Err("读取响应超过 2 MiB".into()); } bytes.extend_from_slice(&chunk); }
        let raw = String::from_utf8(bytes).map_err(|_| "仅支持 UTF-8 文本网页".to_owned())?;
        let (title, body) = if config.provider == Provider::Direct && (content_type.contains("html") || raw.trim_start().starts_with('<')) { html_text(&raw) } else { (raw.lines().find_map(|line| line.strip_prefix("Title: ")).unwrap_or("").to_owned(), raw) };
        return Ok(Page { title: if title.is_empty() { target.clone() } else { title }, url: target, body: body.chars().take(prefs.cutoff).collect() });
    }
    Err("没有可用读取密钥".into())
}
fn html_text(raw: &str) -> (String, String) {
    // ponytail: 保守文本扫描不执行 HTML；复杂 DOM 正文识别需要时改用专用解析器，Jina 已提供独立阅读模式。
    let lower = raw.to_ascii_lowercase(); let title = lower.find("<title>").and_then(|a| lower[a + 7..].find("</title>").map(|b| decode_entities(&raw[a + 7..a + 7 + b]))).unwrap_or_default();
    let mut out = String::new(); let mut pos = 0;
    while pos < raw.len() {
        if raw.as_bytes()[pos] == b'<' {
            let Some(end) = raw[pos..].find('>') else { break; }; let end = pos + end;
            let tag = lower[pos + 1..end].trim();
            let skip = ["script", "style", "noscript", "template"].into_iter().find(|name| tag == *name || tag.split_whitespace().next() == Some(*name));
            if let Some(name) = skip { if let Some(close) = lower[end + 1..].find(&format!("</{name}")) { let close = end + 1 + close; if let Some(close_end) = lower[close..].find('>') { pos = close + close_end + 1; continue; } } break; }
            out.push(' '); pos = end + 1;
        } else { let ch = raw[pos..].chars().next().unwrap(); out.push(ch); pos += ch.len_utf8(); }
    }
    (title, decode_entities(&out).split_whitespace().collect::<Vec<_>>().join(" "))
}
fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len()); let mut rest = raw;
    while let Some(pos) = rest.find('&') { out.push_str(&rest[..pos]); rest = &rest[pos..]; if let Some(end) = rest.find(';').filter(|n| *n <= 12) {
        let entity = &rest[1..end]; let c = match entity { "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'), "quot" => Some('"'), "apos" | "#39" => Some('\''), "nbsp" => Some(' '), _ => entity.strip_prefix("#x").and_then(|s| u32::from_str_radix(s, 16).ok()).or_else(|| entity.strip_prefix('#').and_then(|s| s.parse().ok())).and_then(char::from_u32) };
        if let Some(c) = c { out.push(c); rest = &rest[end + 1..]; continue; }
    } out.push('&'); rest = &rest[1..]; } out.push_str(rest); out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn sqlite_import_preserves_ciphertext_and_rolls_back_failed_save() {
        let root = std::env::temp_dir().join(format!("lumadesk-search-import-{}", lumakit_support::new_id().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(FILE);
        std::fs::write(&path, b"broken").unwrap();
        assert!(SearchSettings::load(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        let mut old = Stored::default();
        old.configs[0].keys.push(SavedKey::new("migration-secret").unwrap());
        lumakit_support::save_versioned(&path, 1, &old).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut settings = SearchSettings::load(&root).unwrap();
        assert_eq!(settings.stored.configs[0].keys[0].ciphertext, old.configs[0].keys[0].ciphertext);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::write(&path, b"not used after import").unwrap();
        let connection = rusqlite::Connection::open(root.join("settings.db")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_search_write BEFORE INSERT ON search_configs BEGIN SELECT RAISE(ABORT,'reject'); END;").unwrap();
        settings.drafts[0].endpoint = "https://changed.example".into();
        assert!(settings.save(0, None).is_err());
        let restored = SearchSettings::load(&root).unwrap();
        assert_eq!(restored.stored.configs[0].endpoint, old.configs[0].endpoint);
        assert_eq!(restored.stored.configs[0].keys[0].open().unwrap().as_str(), "migration-secret");
        connection.execute_batch("DROP TRIGGER reject_search_write; DELETE FROM search_configs;").unwrap();
        assert!(SearchSettings::load(&root).is_err());
        drop(connection); drop(restored); drop(settings);
        std::fs::remove_dir_all(root).unwrap();
    }

    use std::io::{Read, Write};
    #[test]
    fn invalid_blacklist_is_atomic_and_url_path_is_case_sensitive() {
        let mut original = SearchPreferences::default(); original.blacklist = vec!["kept.example".into()];
        assert!(preferences_from_draft(&original, "valid.example\nhttps://bad user/path").is_err()); assert_eq!(original.blacklist, ["kept.example"]);
        let rules = vec![normalize_rule("https://EXAMPLE.COM/Private/*").unwrap()];
        assert!(blocked(&Url::parse("https://example.com/Private/A").unwrap(), &rules));
        assert!(!blocked(&Url::parse("https://example.com/private/A").unwrap(), &rules));
        assert!(normalize_rule("https://bad_host.test/path").is_err());
        assert_eq!(html_text("<title>A &amp; B</title><script>secret()</script><p>你好</p>").0, "A & B");
        assert!(!html_text("<script>secret()</script><p>你好</p>").1.contains("secret"));
    }
    #[test]
    fn direct_fetch_uses_real_loopback_http_and_unicode_cutoff() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let endpoint = format!("http://{}/page", server.local_addr().unwrap());
        let worker = thread::spawn(move || { let (mut stream, _) = server.accept().unwrap(); stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap(); let mut request = [0; 4096]; let n = stream.read(&mut request).unwrap(); assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /page ")); let body = format!("<title>本地网页</title><script>hidden()</script><p>{}</p>", "文".repeat(200)); write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap(); });
        let config = Config { provider: Provider::Direct, endpoint: String::new(), keys: Vec::new() }; let mut prefs = SearchPreferences::default(); prefs.cutoff = 100;
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap(); let page = rt.block_on(fetch_page(config, endpoint.clone(), prefs, CancellationToken::new())).unwrap();
        assert_eq!(page.title, "本地网页"); assert_eq!(page.url, endpoint); assert_eq!(page.body.chars().count(), 100); assert!(!page.body.contains("hidden")); worker.join().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn search_http_and_encrypted_restart_readback() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let endpoint = format!("http://{}", server.local_addr().unwrap());
        let worker = thread::spawn(move || { let (mut stream, _) = server.accept().unwrap(); stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap(); let mut request = Vec::new(); let mut chunk = [0; 4096]; loop { let n = stream.read(&mut chunk).unwrap(); request.extend_from_slice(&chunk[..n]); if request.windows(4).any(|x| x == b"\r\n\r\n") { break; } } let text = String::from_utf8_lossy(&request).to_ascii_lowercase(); assert!(text.starts_with("post /search ")); assert!(text.contains("authorization: bearer local-test-key")); let body = r#"{"results":[{"title":"真实回环","url":"https://example.org/Private/A","content":"本地 HTTP 响应"}]}"#; write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap(); });
        let root = std::env::temp_dir().join(format!("lumadesk-search-{}", lumakit_support::new_id().unwrap()));
        let mut settings = SearchSettings::load(&root).unwrap(); settings.drafts[0].endpoint = endpoint; settings.drafts[0].key = "local-test-key".into(); settings.save(0, None).unwrap();
        let restored = SearchSettings::load(&root).unwrap(); assert_eq!(restored.stored.configs[0].keys[0].open().unwrap().as_str(), "local-test-key"); assert!(restored.stored.configs[4].keys.is_empty());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap(); let result = rt.block_on(search_query(restored.stored.configs[0].clone(), "本地查询".into(), restored.preferences.clone(), CancellationToken::new())).unwrap(); assert_eq!(result.hits[0].title, "真实回环"); worker.join().unwrap();
        assert!(!std::fs::read(root.join("settings.db")).unwrap().windows(b"local-test-key".len()).any(|bytes| bytes == b"local-test-key")); drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn search_fails_over_only_auth_status_and_filters_before_result_limit() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap(); stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new(); let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap(); assert!(n > 0); bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let len = headers.lines().find_map(|s| s.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                        if bytes.len() >= end + 4 + len { break; }
                    }
                }
                let request = String::from_utf8_lossy(&bytes);
                assert!(request.contains(if attempt == 0 { "failover-first" } else { "failover-second" }));
                if attempt == 0 { write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap(); }
                else {
                    let body = r#"{"results":[{"title":"blocked","url":"https://example.org/Private/A","content":"blocked"},{"title":"allowed","url":"https://example.org/private/A","content":"unicode 文本"}]}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                }
            }
        });
        let config = Config { provider: Provider::Tavily, endpoint, keys: vec![SavedKey::new("failover-first").unwrap(), SavedKey::new("failover-second").unwrap()] };
        let mut prefs = SearchPreferences::default(); prefs.max_results = 1; prefs.blacklist = vec!["https://example.org/Private/*".into()];
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = rt.block_on(search_query(config, "实际查询".into(), prefs, CancellationToken::new())).unwrap();
        assert_eq!(result.hits.len(), 1); assert_eq!(result.hits[0].title, "allowed"); server.join().unwrap();
    }
}
