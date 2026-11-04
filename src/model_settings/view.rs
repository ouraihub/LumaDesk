//! GPUI controls over the existing catalog, key and discovery operations, laid out as ThinkRail
//! Native's providers page: a provider rail, the selected provider's detail, and in-page dialogs.
use super::*;
use gpui_kit::{div, prelude::*, px, AnyElement, ClipboardItem, ElementId, FocusHandle, IntoElement, Pixels, Render, Role, SharedString, Subscription};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{button::{Button, ButtonVariants}, input::{Input, InputEvent, InputState}, menu::{DropdownMenu, PopupMenuItem}, switch::Switch, tag::Tag, Disableable, FocusTrapElement, Icon, Selectable, Sizable};
use crate::model_editor::{ModelEditor, MODEL_TYPES};
use lumakit_design::{LumaTheme, Roles};
use lumakit_design::components::{label, IconButton};
use lumakit_design::components::settings as ui;
use lumakit_design::metrics::{radius, size, space, text};

mod dialogs;

/// Dialog column width (ThinkRail's wide form dialogs).
const DIALOG_WIDTH: Pixels = px(640.);
/// The dialog layer is at least this tall, so a short page still frames the dialog.
const DIALOG_MIN_HEIGHT: Pixels = px(560.);
/// Window width above which the provider rail sits beside the detail (settings nav + rail + detail).
const WIDE_PAGE: Pixels = px(880.);
/// The selected key's value box (and each key in the keys dialog).
const KEY_BOX: Pixels = px(32.);
/// A model group header and a model row in the models list.
const GROUP_ROW: Pixels = px(40.);
const MODEL_ROW: Pixels = px(44.);
const PROVIDER_FILTERS: [(u8, &str); 3] = [(0, "全部平台"), (1, "已启用"), (2, "已停用")];
const CAPABILITY_FILTERS: [(u8, &str); 5] = [(0, "全部"), (1, "视觉"), (2, "推理"), (3, "工具"), (4, "联网")];

fn roles(cx: &App) -> Roles { LumaTheme::global(cx).roles }
/// A muted one-line hint (ThinkRail's subtle metadata line).
fn hint(content: impl Into<SharedString>, r: &Roles) -> gpui_kit::Div { label(text::METADATA, content).text_color(r.text_subtle) }
/// A dialog title over its body.
fn dialog_title(title: impl Into<SharedString>) -> gpui_kit::Div { label(text::TITLE_DIALOG, title) }
/// A collapsible group header: chevron, name, subtle count; the caller sizes, tints and wires it.
fn group_header(id: impl Into<ElementId>, folded: bool, name: &str, count: usize, r: &Roles) -> gpui_kit::Stateful<gpui_kit::Div> {
    h_flex().id(id).gap(space::S8).cursor_pointer().role(Role::Button).aria_label(format!("{name}，{count} 个")).aria_expanded(!folded)
        .child(Icon::new(if folded { IconName::ChevronRight } else { IconName::ChevronDown }).small().text_color(r.text_muted))
        .child(label(text::UI, name.to_owned()).min_w(px(0.)).truncate())
        .child(label(text::METADATA, count.to_string()).text_color(r.text_subtle))
}

pub(super) struct ModelView {
    inputs: HashMap<&'static str, Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
    model: Option<Entity<ModelEditor>>,
    focus: FocusHandle,
    modal_open: bool,
    previous_focus: Option<FocusHandle>,
}
impl ModelSettings {
    pub fn create(root: &Path, window: &mut Window, cx: &mut App) -> Result<Entity<Self>, String> {
        let mut state = Self::load(root)?;
        Ok(cx.new(|cx| { state.init_view(window, cx); state }))
    }
    fn init_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut inputs = HashMap::new(); let mut subscriptions = Vec::new();
        for (key, placeholder) in [("provider-search", "搜索模型平台…"), ("url", "https://api.example.com/v1"), ("model-search", "搜索名称或 ID…"), ("sync-search", "搜索模型…"), ("name", "名称"), ("editor-url", "API 地址"), ("key", "输入 API 密钥；失败时保留草稿")] {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(key == "key"));
            subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    match key {
                        "provider-search" => this.search = value,
                        "model-search" => this.model_query = value,
                        "sync-search" => if let Some(dialog) = &mut this.sync_dialog { dialog.search = value; },
                        "url" => if let Some((p, Some(e))) = this.indices() { this.draft.providers[p].endpoints[e].base_url = value; this.dirty = true; },
                        "name" => match &mut this.editor { Some(EditorDialog::Provider { name, .. } | EditorDialog::Endpoint { name, .. }) => *name = value, _ => {} },
                        "editor-url" => if let Some(EditorDialog::Endpoint { url, .. }) = &mut this.editor { *url = value; },
                        "key" => if let Some((p, Some(e))) = this.indices() { *this.key_drafts.entry(this.draft.providers[p].endpoints[e].id.clone()).or_default().value = value; },
                        _ => {}
                    }
                } else if key == "url" && matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) && this.dirty { this.commit(); }
                else if key == "key" && matches!(event, InputEvent::PressEnter { .. }) { this.save_key_input(window, cx); }
                cx.notify();
            }));
            inputs.insert(key, input);
        }
        self.view = Some(ModelView { inputs, _subscriptions: subscriptions, model: None, focus: cx.focus_handle(), modal_open: false, previous_focus: None });
        self.refresh_endpoint(window, cx);
    }
    pub fn is_busy(&self) -> bool { self.job.is_some() }
    pub fn clear_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_drafts(); self.delete_provider = None; self.delete_endpoint = None;
        if let Some(view) = &mut self.view { view.model = None; view.modal_open = false; view.previous_focus = None; }
        self.set_input("key", "", window, cx); self.set_input("sync-search", "", window, cx);
        cx.notify();
    }
    fn set_input(&self, key: &'static str, value: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let value: SharedString = value.into();
        if let Some(input) = self.view.as_ref().and_then(|view| view.inputs.get(key)) { input.update(cx, |input, cx| input.set_value(value, window, cx)); }
    }
    fn input(&self, key: &'static str) -> Input { Input::new(&self.view.as_ref().unwrap().inputs[key]) }
    fn refresh_endpoint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_drafts.clear(); self.set_input("key", "", window, cx);
        let url = self.indices().and_then(|(p, e)| e.map(|e| self.draft.providers[p].endpoints[e].base_url.clone())).unwrap_or_default();
        self.set_input("url", url, window, cx);
    }
    fn button(&self, id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &Context<Self>, action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> Button {
        self.on(Button::new(id).label(label), cx, action)
    }
    /// Attaches `action` to an already built button (icon buttons, rows) and repaints afterwards.
    fn on(&self, button: Button, cx: &Context<Self>, action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> Button {
        button.on_click(cx.listener(move |this, _, window, cx| { action(this, window, cx); cx.notify(); }))
    }
    fn open_editor(&mut self, editor: EditorDialog, window: &mut Window, cx: &mut Context<Self>) {
        self.key_drafts.clear(); self.set_input("key", "", window, cx);
        if let Some(view) = &mut self.view { view.model = None; }
        match &editor {
            EditorDialog::Provider { name, .. } => self.set_input("name", name.clone(), window, cx),
            EditorDialog::Endpoint { name, url, .. } => { self.set_input("name", name.clone(), window, cx); self.set_input("editor-url", url.clone(), window, cx); },
            EditorDialog::Model { provider, endpoint, original, value, .. } => {
                let protocol = self.draft.providers.iter().find(|p| p.id == *provider).and_then(|p| p.endpoints.iter().find(|e| e.id == *endpoint)).map_or(Protocol::OpenAiChat, |e| e.protocol);
                let model = cx.new(|cx| ModelEditor::new(value.clone(), protocol, original.is_some(), window, cx));
                self.view.as_mut().unwrap().model = Some(model);
            }
            EditorDialog::Keys { .. } => {}
        }
        self.editor = Some(editor); self.notice.clear(); self.error = false;
    }
    fn edit_endpoint(&mut self, p: usize, e: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.open_endpoint(p, e); let editor = self.editor.take().unwrap(); self.open_editor(editor, window, cx);
    }
    fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None; self.view.as_mut().unwrap().model = None;
        self.key_drafts.clear(); self.set_input("key", "", window, cx);
    }
    fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut editor) = self.editor.take() else { return; };
        if let EditorDialog::Model { value, .. } = &mut editor {
            if let Some(model) = &self.view.as_ref().unwrap().model {
                if let Err(error) = model.read(cx).validate() { self.report(Err(error), ""); self.editor = Some(editor); return; }
                *value = model.read(cx).value.clone();
            }
        }
        let before = self.draft.clone(); let provider = self.provider.clone(); let endpoint = self.endpoint.clone();
        let result = self.apply_editor(&editor).and_then(|_| self.save());
        if self.report(result, "已保存") { self.close_editor(window, cx); self.refresh_endpoint(window, cx); }
        else { self.draft = before; self.provider = provider; self.endpoint = endpoint; self.editor = Some(editor); }
    }
    fn save_key_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((p, Some(e))) = self.indices() {
            self.save_key(p, e); if !self.error { self.set_input("key", "", window, cx); }
        }
    }
    fn change_key(&mut self, p: usize, e: usize, id: &str) {
        let before = self.draft.providers[p].endpoints[e].selected_key.clone();
        self.draft.providers[p].endpoints[e].selected_key = Some(id.into());
        let result = self.persist_keys(p, e);
        if !self.report(result, "密钥选择已保存") { self.draft.providers[p].endpoints[e].selected_key = before; }
    }
    fn delete_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let indices = self.draft.providers.iter().position(|p| &p.id == provider).and_then(|p| self.draft.providers[p].endpoints.iter().position(|e| &e.id == endpoint).map(|e| (p, e)));
            match indices { Some((p, e)) => self.persist_keys(p, e), None => Err("线路已不存在".into()) }
        } else { self.save() };
        if self.report(result, "已删除") {
            self.delete_provider = None; self.delete_endpoint = None; self.delete_key = None;
            self.repair_selection(); self.refresh_endpoint(window, cx);
        } else { self.draft = before; }
    }
    fn save_sync(&mut self, selected: HashSet<String>, types: HashMap<String, ModelType>) {
        let Some(dialog) = &self.sync_dialog else { return; };
        let before = self.draft.clone();
        let result = (|| {
            let endpoint = self.draft.providers.iter_mut().find(|p| p.id == dialog.provider).and_then(|p| p.endpoints.iter_mut().find(|e| e.id == dialog.endpoint)).ok_or("目标线路已不存在")?;
            merge_models(&mut endpoint.models, &dialog.ids, &selected);
            for model in &mut endpoint.models {
                if let Some(kind) = types.get(&model.id) {
                    // A discovery round must not erase manually configured settings on unchanged types.
                    if model.settings.model_type != *kind {
                        model.settings.model_type = *kind;
                        if *kind != ModelType::Chat { model.settings.reasoning = None; model.settings.native_web_search = false; model.settings.pricing = None; model.settings.supports_tools = None; }
                    }
                }
            }
            repair_model_selection(endpoint); Ok::<_, String>(())
        })().and_then(|_| self.save());
        if self.report(result, "模型选择已保存") { if let Some(dialog) = &mut self.sync_dialog { dialog.selected = selected; dialog.types = types; } }
        else { self.draft = before; }
    }
    fn edit_models(&mut self, p: usize, e: usize, selected: Option<String>, removed: &[String]) {
        let before = self.draft.providers[p].clone(); let endpoint = &mut self.draft.providers[p].endpoints[e];
        if let Some(id) = selected { endpoint.selected_model = Some(id); }
        endpoint.models.retain(|m| !removed.contains(&m.id)); repair_model_selection(endpoint);
        let result = self.save(); if !self.report(result, "已保存") { self.draft.providers[p] = before; }
    }
    /// The provider rail: search with a status filter menu, one row per provider, then 新建服务商.
    fn providers_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let page = cx.entity(); let current = self.provider_filter;
        let filter = Button::new("provider-filter").outline().icon(IconName::SlidersHorizontal).accessibility_label("筛选平台").tooltip("筛选平台")
            .dropdown_menu(move |menu, _, _| PROVIDER_FILTERS.into_iter().fold(menu, |menu, (value, title)| {
                let page = page.clone();
                menu.item(PopupMenuItem::new(title).checked(current == value).on_click(move |_, _, cx| page.update(cx, |this, cx| { this.provider_filter = value; cx.notify(); })))
            }));
        let search = h_flex().gap(space::S6).child(self.input("provider-search").prefix(Icon::new(IconName::Search).small()).flex_1()).child(filter);
        let query = self.search.trim().to_lowercase();
        let rows: Vec<AnyElement> = self.draft.providers.iter()
            .filter(|p| p.name.to_lowercase().contains(&query) && (self.provider_filter == 0 || p.enabled == (self.provider_filter == 1)))
            .map(|provider| self.provider_row(provider, &r, cx)).collect();
        let empty = rows.is_empty().then(|| hint("没有匹配的平台", &r).p(space::S8));
        v_flex().w_full().gap(space::S4).child(search).children(rows).children(empty)
            .child(self.button("new-provider", "新建服务商", cx, |this, window, cx| this.open_editor(EditorDialog::Provider { id: None, name: String::new() }, window, cx)).w_full().icon(IconName::Plus))
    }
    /// Avatar, name (muted when disabled), status dot (green with a saved key, amber without, none when disabled), ··· menu.
    fn provider_row(&self, provider: &ProviderEntry, r: &Roles, cx: &Context<Self>) -> AnyElement {
        let id = provider.id.clone(); let selected = self.provider.as_ref() == Some(&provider.id);
        let has_key = provider.endpoints.iter().any(|e| !e.keys.is_empty());
        let state = if !provider.enabled { "已停用" } else if has_key { "已启用" } else { "已启用，未保存密钥" };
        let dot = provider.enabled.then(|| ui::dot(if has_key { r.feedback_success } else { r.feedback_warning }));
        let page = cx.entity(); let (edit, remove) = (provider.id.clone(), provider.id.clone());
        let menu = ui::row_menu_button(format!("provider-menu-{id}"), format!("{} 的操作", provider.name)).dropdown_menu(move |menu, _, _| {
            let (page_edit, page_remove, edit, remove) = (page.clone(), page.clone(), edit.clone(), remove.clone());
            menu.item(PopupMenuItem::new("服务商设置").icon(IconName::Pencil).on_click(move |_, window, cx| page_edit.update(cx, |this, cx| {
                if let Some(p) = this.draft.providers.iter().find(|p| p.id == edit) { let name = p.name.clone(); this.open_editor(EditorDialog::Provider { id: Some(edit.clone()), name }, window, cx); }
                cx.notify();
            })))
            .item(PopupMenuItem::new("删除服务商…").icon(IconName::Trash).on_click(move |_, _, cx| page_remove.update(cx, |this, cx| { this.delete_provider = Some(remove.clone()); cx.notify(); })))
        });
        ui::list_row(format!("provider-{id}"), format!("{}，{state}", provider.name), selected, r)
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.provider.as_ref() == Some(&id) { return; }
                if this.dirty { this.report(Err("当前地址尚未保存，请修正或重试保存后再切换平台".into()), ""); cx.notify(); return; }
                this.provider = Some(id.clone()); this.repair_selection(); this.model_query.clear(); this.set_input("model-search", "", window, cx); this.notice.clear(); this.refresh_endpoint(window, cx);
                cx.notify();
            }))
            .child(ui::avatar(&provider.name, r))
            .child(label(text::UI, provider.name.clone()).flex_1().min_w(px(0.)).truncate().when(!provider.enabled, |d| d.text_color(r.text_muted)))
            .children(dot).child(menu).into_any_element()
    }
    fn detail_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let body = v_flex().flex_1().w_full().min_w(px(0.)).gap(space::S16);
        let Some((p, e)) = self.indices() else { return body.child(ui::placeholder("未选择服务商", "选择或新建一个服务商，开始配置模型。", &r)); };
        let body = body.child(self.detail_header(p, cx));
        let Some(e) = e else {
            return body.child(v_flex().gap(space::S8)
                .child(ui::field_title("API 地址", [ui::link("new-endpoint", "添加线路").on_click(cx.listener(move |this, _, window, cx| { this.edit_endpoint(p, None, window, cx); cx.notify(); })).into_any_element()], &r))
                .child(hint("尚未配置线路，添加 API 地址后即可管理密钥和模型。", &r)));
        };
        body.child(self.key_section(p, e, cx))
            .child(ui::value("密钥使用方式", "失败时轮换", Some("仅在 HTTP 401/403/429 时换下一个已保存密钥，不跨服务商重试".into()), &r))
            .child(self.address_section(p, e, cx))
            .child(self.models_view(p, e, cx))
    }
    /// An icon-only action that repaints after `action`.
    fn icon(&self, id: impl Into<ElementId>, icon: IconName, tooltip: &'static str, cx: &Context<Self>, action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> IconButton {
        IconButton::new(id, icon).tooltip(tooltip).on_click(cx.listener(move |this, _, window, cx| { action(this, window, cx); cx.notify(); }))
    }
    /// Provider name, its settings entry, and the independently saved enable switch at the far right.
    fn detail_header(&self, p: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let provider = &self.draft.providers[p];
        h_flex().gap(space::S8)
            .child(label(text::HEADING_SM, provider.name.clone()).min_w(px(0.)).truncate())
            .child(self.icon("edit-provider", IconName::Settings2, "服务商设置", cx, move |this, window, cx| {
                let p = &this.draft.providers[p]; this.open_editor(EditorDialog::Provider { id: Some(p.id.clone()), name: p.name.clone() }, window, cx);
            }))
            .child(div().flex_1())
            .child(Switch::new("enabled").checked(provider.enabled).accessibility_label(format!("启用 {}", provider.name))
                .on_click(cx.listener(move |this, value: &bool, _, cx| { this.set_enabled(p, *value); cx.notify(); })))
    }
    /// API 密钥 · 获取密钥; the selected key's value box (or the first-key input), key management and 检测.
    fn key_section(&self, p: usize, e: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let provider = &self.draft.providers[p]; let endpoint = &provider.endpoints[e];
        let links: Vec<AnyElement> = PRESETS.iter().find(|(id, _, _, _)| *id == provider.id)
            .map(|(_, _, _, link)| { let link = *link; ui::link("get-key", "获取密钥").on_click(move |_, _, cx| cx.open_url(link)).into_any_element() }).into_iter().collect();
        let manage = self.on(Button::new("manage-keys").outline().icon(IconName::KeyRound).accessibility_label("密钥管理").tooltip("密钥管理"), cx, move |this, window, cx| {
            this.open_editor(EditorDialog::Keys { provider: this.draft.providers[p].id.clone(), endpoint: this.draft.providers[p].endpoints[e].id.clone() }, window, cx);
        });
        let detect = self.button("detect", "检测", cx, |this, _, _| this.start_job(false)).outline().disabled(self.dirty);
        // The key input is one entity: while a dialog shows it, the main row must not.
        let value = if !endpoint.keys.is_empty() {
            let key = endpoint.selected_key.as_ref().and_then(|id| endpoint.keys.iter().find(|key| &key.id == id)).unwrap_or(&endpoint.keys[0]);
            self.key_row(p, e, key, cx)
        } else if self.editor.is_none() {
            h_flex().flex_1().min_w(px(0.)).gap(space::S8).child(self.input("key").flex_1())
                .child(self.button("save-primary-key", "加密保存", cx, |this, window, cx| this.save_key_input(window, cx)).primary())
        } else { h_flex().flex_1().child(hint("尚未保存密钥", &r)) };
        v_flex().gap(space::S8).child(ui::field_title("API 密钥", links, &r)).child(h_flex().gap(space::S8).child(value).child(manage).child(detect))
    }
    /// API 地址 · 添加线路; a line picker when there are several lines, the URL with copy and line settings, the request line.
    fn address_section(&self, p: usize, e: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let provider = &self.draft.providers[p]; let endpoint = &provider.endpoints[e];
        let add = ui::link("new-endpoint", "添加线路").on_click(cx.listener(move |this, _, window, cx| { this.edit_endpoint(p, None, window, cx); cx.notify(); })).into_any_element();
        let mut section = v_flex().gap(space::S8).child(ui::field_title("API 地址", [add], &r));
        if provider.endpoints.len() > 1 {
            let page = cx.entity(); let lines: Vec<(String, String)> = provider.endpoints.iter().map(|l| (l.id.clone(), l.name.clone())).collect(); let current = endpoint.id.clone();
            section = section.child(Button::new("provider-line").outline().w_full().label(endpoint.name.clone()).icon(IconName::ChevronDown).dropdown_menu(move |menu, _, _| {
                lines.iter().fold(menu, |menu, (id, name)| {
                    let (page, id) = (page.clone(), id.clone());
                    menu.item(PopupMenuItem::new(name.clone()).checked(id == current).on_click(move |_, window, cx| page.update(cx, |this, cx| {
                        if this.dirty { this.report(Err("当前地址尚未保存，请先修正或重试保存".into()), ""); } else { this.endpoint = Some(id.clone()); this.refresh_endpoint(window, cx); }
                        cx.notify();
                    })))
                })
            }));
        }
        let copy = self.icon("copy-url", IconName::Copy, "复制 API 地址", cx, move |this, _, cx| { cx.write_to_clipboard(ClipboardItem::new_string(this.draft.providers[p].endpoints[e].base_url.clone())); this.notice = "已复制 API 地址".into(); });
        let settings = self.on(Button::new("edit-endpoint").outline().icon(IconName::Settings2).accessibility_label("线路设置").tooltip("线路设置"), cx, move |this, window, cx| this.edit_endpoint(p, Some(e), window, cx));
        section = section.child(h_flex().gap(space::S8).child(self.input("url").suffix(copy).flex_1()).child(settings)
            .when(self.dirty, |d| d.child(self.button("save-url", "保存地址", cx, |this, _, _| this.commit()).primary())));
        section.child(ui::request_line(match model_url(&endpoint.base_url, endpoint.protocol) { Ok(url) => format!("{} · GET {url}", protocol_name(endpoint.protocol)), Err(error) => error }, &r))
    }
    /// One saved key: masked (or explicitly revealed) value with icon actions. The main row shows only
    /// the eye until revealed; the keys dialog always shows every action plus 使用此密钥.
    fn key_row(&self, p: usize, e: usize, key: &SavedKey, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let endpoint = &self.draft.providers[p].endpoints[e];
        let draft = self.key_drafts.get(&endpoint.id);
        let revealed = draft.is_some_and(|d| d.revealed_key.as_ref() == Some(&key.id) && d.revealed.is_some());
        let shown = if revealed { draft.unwrap().revealed.as_ref().unwrap().to_string() } else { key.masked.clone() };
        let in_dialog = matches!(self.editor, Some(EditorDialog::Keys { .. }));
        let mut actions = h_flex().flex_none().gap(space::S2);
        let id = key.id.clone(); actions = actions.child(self.icon(format!("reveal-key-{id}"), if revealed { IconName::EyeOff } else { IconName::Eye }, if revealed { "隐藏密钥" } else { "显示密钥" }, cx, move |this, _, _| {
            if revealed { if let Some(draft) = this.key_drafts.get_mut(&this.draft.providers[p].endpoints[e].id) { draft.revealed = None; draft.revealed_key = None; } } else { this.reveal_key(p, e, &id); }
        }));
        if revealed || in_dialog {
            let id = key.id.clone(); actions = actions.child(self.icon(format!("copy-key-{id}"), IconName::Copy, "复制密钥", cx, move |this, _, cx| {
                let result = this.draft.providers[p].endpoints[e].keys.iter().find(|key| key.id == id).ok_or_else(|| "密钥已不存在".to_owned()).and_then(SavedKey::open);
                match result { Ok(secret) => { cx.write_to_clipboard(ClipboardItem::new_string(secret.to_string())); this.notice = "已复制密钥到系统剪贴板；请留意其他程序访问剪贴板".into(); }, Err(error) => { this.report(Err(error), ""); } }
            }));
            let id = key.id.clone(); actions = actions.child(self.icon(format!("edit-key-{id}"), IconName::Pencil, "修改密钥", cx, move |this, window, cx| {
                if !matches!(this.editor, Some(EditorDialog::Keys { .. })) { this.open_editor(EditorDialog::Keys { provider: this.draft.providers[p].id.clone(), endpoint: this.draft.providers[p].endpoints[e].id.clone() }, window, cx); }
                let draft = this.key_drafts.entry(this.draft.providers[p].endpoints[e].id.clone()).or_default();
                draft.value.zeroize(); draft.editing = Some(id.clone()); draft.revealed = None; draft.revealed_key = None; this.set_input("key", "", window, cx);
            }));
            let id = key.id.clone(); actions = actions.child(self.icon(format!("delete-key-{id}"), IconName::Trash, "删除密钥…", cx, move |this, window, cx| {
                this.key_drafts.clear(); this.set_input("key", "", window, cx);
                this.delete_key = Some((this.draft.providers[p].id.clone(), this.draft.providers[p].endpoints[e].id.clone(), id.clone()));
            }));
        }
        let extra = (!in_dialog && endpoint.keys.len() > 1).then(|| hint(format!("+{}", endpoint.keys.len() - 1), &r));
        let mut row = h_flex().flex_1().min_w(px(0.)).h(KEY_BOX).px(space::S8).gap(space::S8).rounded(radius::MD).border_1().border_color(r.border_default)
            .child(label(text::CODE_TEXT, shown).flex_1().min_w(px(0.)).truncate()).children(extra).child(actions);
        if in_dialog && endpoint.keys.len() > 1 {
            let selected = endpoint.selected_key.as_ref() == Some(&key.id); let id = key.id.clone();
            row = row.child(self.button(format!("select-key-{}", key.id), "使用此密钥", cx, move |this, _, _| this.change_key(p, e, &id)).xsmall().ghost().icon(IconName::Check).selected(selected));
        }
        row
    }
    fn models_view(&self, p: usize, e: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let endpoint = &self.draft.providers[p].endpoints[e];
        let query = self.model_query.trim().to_lowercase(); let mut groups: Vec<(String, Vec<&ModelEntry>)> = Vec::new();
        for model in &endpoint.models {
            if !(model.id.to_lowercase().contains(&query) || model.settings.display_name(&model.id).to_lowercase().contains(&query)) { continue; }
            if !match self.capability_filter { 1 => model.capabilities.vision, 2 => model.settings.reasoning.is_some(), 3 => model.settings.supports_tools.unwrap_or(model.capabilities.tools), 4 => model.settings.native_web_search, _ => true } { continue; }
            let group = model.settings.group_name(&model.id);
            if let Some((_, models)) = groups.iter_mut().find(|(key, _)| key == group) { models.push(model); } else { groups.push((group.into(), vec![model])); }
        }
        let names: Vec<_> = groups.iter().map(|(g, _)| g.clone()).collect();
        let body = if groups.is_empty() { ui::empty(if endpoint.models.is_empty() { "未选择模型" } else { "没有匹配的模型" }, &r) } else {
            let mut list = v_flex().rounded(radius::LG).border_1().border_color(r.border_muted).overflow_hidden();
            for (index, (group, models)) in groups.into_iter().enumerate() { list = list.children(self.model_group(p, e, index, group, models, cx)); }
            list.into_any_element()
        };
        v_flex().gap(space::S8).child(self.models_toolbar(p, e, &names, cx)).child(body)
    }
    /// 模型, fold-all, inline search, capability menu; 同步模型 and manual add on the right.
    fn models_toolbar(&self, p: usize, e: usize, names: &[String], cx: &Context<Self>) -> gpui_kit::Div {
        let all_folded = !names.is_empty() && names.iter().all(|group| self.folded.contains(group)); let names = names.to_vec();
        let fold = self.icon("fold-all", if all_folded { IconName::ChevronsUpDown } else { IconName::ChevronsDownUp }, if all_folded { "展开全部分组" } else { "收起全部分组" }, cx,
            move |this, _, _| { if all_folded { this.folded.clear(); } else { this.folded.extend(names.iter().cloned()); } }).disabled(self.draft.providers[p].endpoints[e].models.is_empty());
        let page = cx.entity(); let current = self.capability_filter;
        let title = CAPABILITY_FILTERS.iter().find(|(v, _)| *v == current).map_or("全部", |(_, t)| *t);
        let capability = Button::new("cap-filter").ghost().small().label(title).icon(IconName::ChevronDown).accessibility_label(format!("按能力筛选：{title}"))
            .dropdown_menu(move |menu, _, _| CAPABILITY_FILTERS.into_iter().fold(menu, |menu, (value, title)| {
                let page = page.clone();
                menu.item(PopupMenuItem::new(title).checked(current == value).on_click(move |_, _, cx| page.update(cx, |this, cx| { this.capability_filter = value; cx.notify(); })))
            }));
        let add = self.on(Button::new("new-model").outline().icon(IconName::Plus).accessibility_label("手动添加模型").tooltip("手动添加模型"), cx, move |this, window, cx| this.open_editor(EditorDialog::Model {
            provider: this.draft.providers[p].id.clone(), endpoint: this.draft.providers[p].endpoints[e].id.clone(), original: None,
            value: ModelEntry { id: String::new(), capabilities: unknown_capabilities(), manual: true, settings: Default::default() } }, window, cx));
        h_flex().gap(space::S4).flex_wrap()
            .child(label(text::TITLE_SECTION, "模型")).child(fold)
            .child(self.input("model-search").prefix(Icon::new(IconName::Search).small()).small().w(ui::INLINE_SEARCH))
            .child(capability).child(div().flex_1())
            .child(self.button("sync-models", "同步模型", cx, |this, window, cx| { this.set_input("sync-search", "", window, cx); this.start_job(true); }).outline().icon(IconName::RefreshCw).disabled(self.dirty))
            .child(add)
    }
    /// One model group: a fold header with its count and remove-all, then a row per model.
    fn model_group(&self, p: usize, e: usize, index: usize, group: String, models: Vec<&ModelEntry>, cx: &Context<Self>) -> Vec<AnyElement> {
        let r = roles(cx);
        let folded = self.folded.contains(&group); let name = group.clone();
        let removed: Vec<_> = models.iter().map(|m| m.id.clone()).collect();
        let header = h_flex().h(GROUP_ROW).px(space::S12).gap(space::S8).bg(r.container_elevated_bg).when(index > 0, |d| d.border_t_1().border_color(r.border_muted))
            .child(group_header(("model-group", index), folded, if group.is_empty() { "未分组" } else { &group }, models.len(), &r).flex_1().min_w(px(0.))
                .on_click(cx.listener(move |this, _, _, cx| { if !this.folded.remove(&name) { this.folded.insert(name.clone()); } cx.notify(); })))
            .child(self.icon(("remove-group", index), IconName::Trash, "移除分组模型", cx, move |this, _, _| this.edit_models(p, e, None, &removed)));
        let mut rows = vec![header.into_any_element()];
        if !folded { rows.extend(models.into_iter().map(|model| self.model_row(p, e, model, cx).into_any_element())); }
        rows
    }
    /// A model: name, capability tags and id · type (click selects it); settings and remove on the right.
    fn model_row(&self, p: usize, e: usize, model: &ModelEntry, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let endpoint = &self.draft.providers[p].endpoints[e];
        let selected = endpoint.selected_model.as_ref() == Some(&model.id);
        let name = model.settings.display_name(&model.id).to_owned(); let id = model.id.clone();
        let tags: Vec<_> = [(model.capabilities.vision, "视觉"), (model.settings.reasoning.is_some(), "推理"), (model.settings.supports_tools.unwrap_or(model.capabilities.tools), "工具"), (model.settings.native_web_search, "联网")]
            .into_iter().filter_map(|(on, title)| on.then(|| Tag::secondary().small().child(title))).collect();
        let select = h_flex().id(format!("select-model-{id}")).flex_1().min_w(px(0.)).gap(space::S8).cursor_pointer().role(Role::Button).aria_label(name.clone()).aria_selected(selected)
            .child(label(text::UI, name).min_w(px(0.)).truncate()).children(tags)
            .child(hint(format!("{} · {}", model.id, model.settings.model_type.label()), &r).min_w(px(0.)).truncate())
            .on_click(cx.listener(move |this, _, _, cx| { this.edit_models(p, e, Some(id.clone()), &[]); cx.notify(); }));
        let value = model.clone(); let id = model.id.clone();
        h_flex().h(MODEL_ROW).px(space::S12).gap(space::S8).border_t_1().border_color(r.border_muted)
            .when(selected, |d| d.bg(r.control_bg_selected)).when(!selected, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
            .child(select)
            .child(self.icon(format!("edit-model-{id}"), IconName::Settings2, "模型设置", cx, move |this, window, cx| this.open_editor(EditorDialog::Model { provider: this.draft.providers[p].id.clone(), endpoint: this.draft.providers[p].endpoints[e].id.clone(), original: Some(value.id.clone()), value: value.clone() }, window, cx)))
            .child(self.icon(format!("remove-model-{id}"), IconName::Trash, "移除模型", cx, move |this, _, _| this.edit_models(p, e, None, std::slice::from_ref(&id))))
    }
    /// The page notice and, while an address is unsaved, the retry entry.
    fn status(&self, cx: &Context<Self>) -> Option<gpui_kit::Div> {
        let r = roles(cx);
        let retry = self.dirty && self.editor.is_none() && self.sync_dialog.is_none() && self.job.is_none();
        if self.notice.is_empty() && !retry { return None; }
        let notice = (!self.notice.is_empty()).then(|| if self.error { ui::notice(self.notice.clone(), ui::Tone::Error, &r) } else { hint(self.notice.clone(), &r).into_any_element() });
        Some(h_flex().flex_wrap().gap(space::S8).children(notice.map(|n| div().flex_1().min_w(px(0.)).child(n)))
            .when(retry, |d| d.child(self.button("retry-save", "重试保存修改", cx, |this, _, _| this.commit()).link())))
    }
    /// Provider rail and detail, side by side when wide. The page and the background behind a
    /// dialog both draw this, so they cannot drift apart.
    fn columns(&self, wide: bool, cx: &Context<Self>) -> gpui_kit::Div {
        let rail = div().flex_none().child(self.providers_view(cx));
        let rail = if wide { rail.w(size::LIST_COLUMN) } else { rail.w_full() };
        let columns = div().flex().w_full().gap(space::S16).child(rail).child(self.detail_view(cx));
        if wide { columns.flex_row().items_start() } else { columns.flex_col() }
    }
}
impl Render for ModelSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modal = self.delete_provider.is_some() || self.delete_endpoint.is_some() || self.delete_key.is_some() || self.editor.is_some() || self.sync_dialog.is_some();
        let focus = self.view.as_ref().unwrap().focus.clone();
        let view = self.view.as_mut().unwrap();
        if modal && !view.modal_open {
            view.previous_focus = window.focused(cx);
            focus.focus(window, cx);
        } else if !modal && view.modal_open {
            if let Some(previous) = view.previous_focus.take() { previous.focus(window, cx); }
            else { view.inputs["provider-search"].update(cx, |input, cx| input.focus(window, cx)); }
        }
        view.modal_open = modal;
        let r = roles(cx);
        let wide = window.viewport_size().width > WIDE_PAGE;
        let escape = cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                if this.delete_provider.is_some() || this.delete_endpoint.is_some() || this.delete_key.is_some() { this.delete_provider = None; this.delete_endpoint = None; this.delete_key = None; }
                else if this.editor.is_some() { this.close_editor(window, cx); }
                else if this.sync_dialog.is_some() { this.sync_dialog = None; }
                else if let Some(job) = &this.job { job.cancel.cancel(); }
                else { this.model_query.clear(); this.set_input("model-search", "", window, cx); }
                cx.notify();
            }
        });
        let header = ui::page_header("模型服务", None, &r);
        if !modal {
            let main = if self.job.is_some() {
                h_flex().gap(space::S8).child(label(text::UI, "正在检测已保存的线路…").flex_1())
                    .child(self.button("cancel-job", "取消请求", cx, |this, _, _| { if let Some(job) = &this.job { job.cancel.cancel(); } }).outline().icon(IconName::X))
            } else { self.columns(wide, cx) };
            return v_flex().id("model-settings").w_full().gap(space::S16).track_focus(&focus).on_key_down(escape)
                .child(header).child(main).children(self.status(cx)).into_any_element();
        }
        let content = if self.delete_provider.is_some() || self.delete_endpoint.is_some() || self.delete_key.is_some() { self.delete_view(cx) }
            else if self.editor.is_some() { self.editor_view(cx) } else { self.sync_view(cx) };
        let dialog = v_flex().id("model-settings").size_full().min_h(px(0.)).overflow_y_scroll().p(space::S24).gap(space::S16)
            .track_focus(&focus).on_key_down(escape).child(content).children(self.status(cx));
        // Keep the dialog bounded when adding models grows the background page.
        div().relative().w_full().h_full().min_h(DIALOG_MIN_HEIGHT).overflow_hidden()
            .child(v_flex().id("models-background").w_full().gap(space::S16).child(header).child(self.columns(wide, cx)))
            .child(div().absolute().inset_0().occlude().flex().justify_center().items_start().p(space::S12).bg(r.overlay)
                .child(v_flex().w_full().max_w(DIALOG_WIDTH).h_full().min_h(px(0.)).rounded(radius::LG).border_1().border_color(r.border_default).bg(r.container_elevated_bg).shadow_lg()
                    .child(dialog.focus_trap("model-settings-dialog", &focus)))).into_any_element()
    }
}
