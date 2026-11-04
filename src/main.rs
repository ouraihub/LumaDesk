#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
extern crate gpui_kit as gpui;

mod chat;
mod model_settings;
mod model_editor;
mod search_settings;
mod settings_store;
mod mcp_settings;
mod theme;

use gpui_kit::{*, prelude::FluentBuilder as _};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable, Selectable, TitleBar, button::{Button, ButtonVariants}, input::{Input, InputState, InputEvent}};
use lumakit_design::components::{label, settings as ui, ActivityItem, TextStyled};
use lumakit_design::metrics::{size, space, text};
use lumakit_design::{LumaTheme, Roles};
use lumakit_gpui::ChatPanel;
use lumakit_ai::chat::ChatAction;
use std::{path::PathBuf, time::Duration};

const APPS: [&str; 5] = ["对话", "工作", "笔记", "知识库", "翻译"];
/// Width of the app rail and size of a launchpad tile (LumaDesk's own layout).
const RAIL: Pixels = px(64.);
const TILE_W: Pixels = px(112.);
const TILE_H: Pixels = px(88.);
fn app_icon(app: usize) -> IconName {
    match app { 0 => IconName::Bot, 1 => IconName::SquareTerminal, 2 => IconName::FileText, 3 => IconName::BookOpen, _ => IconName::Globe }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabKind { Launchpad, App(usize) }
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage { Models, Search, Mcp, Appearance }
/// Settings navigation groups, ThinkRail-style: (section, [(page, icon, title, search terms)]).
const SETTINGS: [(&str, &[(SettingsPage, IconName, &str, &str)]); 2] = [
    ("AI", &[
        (SettingsPage::Models, IconName::KeyRound, "模型服务", "provider api 密钥 协议 模型 ai"),
        (SettingsPage::Search, IconName::Globe, "网络搜索", "search fetch 网页 黑名单 密钥 ai"),
        (SettingsPage::Mcp, IconName::Plug, "MCP 工具", "mcp 工具 tool server 信任 skills"),
    ]),
    ("界面", &[
        (SettingsPage::Appearance, IconName::Palette, "外观", "appearance theme 主题 强调色 背景 密度 浅色 深色 系统"),
    ]),
];

struct DesktopApp {
    _data_lock: lumakit_support::FileLock,
    _logs: lumakit_observability::LogHandle,
    models: Entity<model_settings::ModelSettings>,
    search: Entity<search_settings::SearchSettings>,
    mcp: Entity<mcp_settings::McpSettings>,
    appearance: Entity<theme::Appearance>,
    chat: chat::ChatState,
    panel: Entity<ChatPanel>,
    tabs: Vec<TabKind>,
    active: usize,
    settings: Option<SettingsPage>,
    settings_query: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    background: Option<Task<()>>,
}

impl DesktopApp {
    /// `font_error`: why the bundled fonts could not be registered (logged once logging runs).
    fn create(font_error: Option<String>, window: &mut Window, cx: &mut App) -> Result<Entity<Self>, String> {
        let root = match std::env::var_os("LUMADESK_DATA_DIR") {
            Some(path) => PathBuf::from(path),
            None => std::env::current_exe().map_err(|error| error.to_string())?
                .parent().ok_or("无法定位程序目录")?.join("data-gpui"),
        };
        std::fs::create_dir_all(&root).map_err(|_| "无法创建配置目录，请将程序移动到可写目录".to_owned())?;
        let data_lock = lumakit_support::FileLock::acquire(&root.join("lumadesk.lock"))
            .map_err(|_| "此数据目录已在使用，或无法加锁；请关闭另一个实例后重试".to_owned())?;
        let logs = lumakit_observability::LogHandle::init(&root.join("logs"), lumakit_observability::LogLevel::Info, 5 * 1024 * 1024)
            .map_err(|error| format!("无法初始化诊断日志：{error}"))?;
        tracing::info!(version = env!("CARGO_PKG_VERSION"), ui = "gpui-kit", "LumaDesk started");
        if let Some(error) = font_error { tracing::error!(error = %error, "bundled fonts not registered; system fonts are used"); }
        let appearance = theme::Appearance::load(&root)?;
        appearance.apply(window, cx);
        let appearance = cx.new(|_| appearance);
        let models = model_settings::ModelSettings::create(&root, window, cx)?;
        let search = search_settings::SearchSettings::create(&root, window, cx)?;
        let mcp = mcp_settings::McpSettings::create(&root, window, cx)?;
        let chat = chat::ChatState::new(&root)?;
        let panel = cx.new(|cx| ChatPanel::new(window, cx));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索设置"));
        let app = cx.new(|cx: &mut Context<Self>| {
            let subscriptions = vec![
                cx.subscribe_in(&panel, window, |this, _, action: &ChatAction, window, cx| {
                    let open_settings = this.chat.dispatch(action.clone(), this.models.read(cx));
                    if open_settings { this.open_settings(SettingsPage::Models, window, cx); }
                    this.ensure_background(window, cx);
                    cx.notify();
                }),
                cx.observe(&panel, |_, _, cx| cx.notify()),
                cx.observe_in(&models, window, |this, _, window, cx| { this.ensure_background(window, cx); cx.notify(); }),
                cx.observe_in(&search, window, |this, _, window, cx| { this.ensure_background(window, cx); cx.notify(); }),
                cx.observe(&appearance, |_, _, cx| cx.notify()),
                cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify()),
                cx.observe_window_appearance(window, |this, window, cx| {
                    this.appearance.update(cx, |appearance, cx| appearance.apply(window, cx));
                    cx.notify();
                }),
            ];
            Self { _data_lock: data_lock, _logs: logs, models, search, mcp, appearance, chat, panel,
                tabs: vec![TabKind::Launchpad], active: 0, settings: None, settings_query: query,
                _subscriptions: subscriptions, background: None }
        });
        let weak = app.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if this.chat.can_close() {
                    this.leave_settings(window, cx);
                    true
                } else {
                    this.launch(0, window, cx);
                    this.ensure_background(window, cx);
                    cx.notify();
                    false
                }
            }).unwrap_or(true)
        });
        Ok(app)
    }

    fn busy(&self, cx: &App) -> bool {
        self.chat.is_busy() || self.models.read(cx).is_busy() || self.search.read(cx).is_busy()
    }

    fn ensure_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.background.is_some() || !self.busy(cx) { return; }
        // ponytail: bounded 40ms polling only while a worker exists; replace with an async wake channel if measured latency requires it.
        self.background = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(40)).await;
                let keep = cx.update(|_, cx| this.update(cx, |this, cx| {
                    let mut changed = this.chat.poll();
                    changed |= this.models.update(cx, |models, cx| { let changed = models.poll(); if changed { cx.notify(); } changed });
                    changed |= this.search.update(cx, |search, cx| { let changed = search.poll(); if changed { cx.notify(); } changed });
                    let busy = this.busy(cx);
                    if changed { cx.notify(); }
                    if !busy { this.background = None; }
                    busy
                }).unwrap_or(false)).unwrap_or(false);
                if !keep { break; }
            }
        }));
    }

    fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.models.update(cx, |models, cx| models.clear_secrets(window, cx));
        self.search.update(cx, |search, cx| search.clear_secrets(window, cx));
        self.settings = None;
    }
    fn open_settings(&mut self, page: SettingsPage, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_settings(window, cx);
        self.settings = Some(page);
        cx.notify();
    }
    fn launchpad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_settings(window, cx);
        if let Some(index) = self.tabs.iter().position(|tab| *tab == TabKind::Launchpad) { self.active = index; }
        else { self.tabs.push(TabKind::Launchpad); self.active = self.tabs.len() - 1; }
        cx.notify();
    }
    fn launch(&mut self, app: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_settings(window, cx);
        if let Some(existing) = self.tabs.iter().position(|tab| *tab == TabKind::App(app)) {
            if self.tabs[self.active] == TabKind::Launchpad {
                self.tabs.remove(self.active);
                self.active = if existing > self.active { existing - 1 } else { existing };
            } else { self.active = existing; }
        } else { self.tabs[self.active] = TabKind::App(app); }
        cx.notify();
    }
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs[index] == TabKind::App(0) { self.chat.cancel(); self.ensure_background(window, cx); }
        self.tabs.remove(index);
        if self.tabs.is_empty() { self.tabs.push(TabKind::Launchpad); self.active = 0; }
        else if self.active > index { self.active -= 1; }
        else { self.active = self.active.min(self.tabs.len() - 1); }
        cx.notify();
    }
}

impl Render for DesktopApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < size::NARROW;
        let r = LumaTheme::global(cx).roles;
        // The tab strip is the window title bar. Clickable parts occlude the drag area so a click
        // reaches them; the remaining empty space moves the window.
        let mut tabs = div().id("app-tabs").occlude().flex().items_center().gap(space::S4).min_w_0().overflow_x_scroll();
        for (index, tab) in self.tabs.iter().copied().enumerate() {
            let title = match tab { TabKind::Launchpad => "启动台", TabKind::App(app) => APPS[app] };
            tabs = tabs.child(div().flex().items_center().flex_shrink_0()
                .child(Button::new(("tab", index)).label(title).small().ghost().selected(self.settings.is_none() && index == self.active)
                    .on_click(cx.listener(move |this, _, window, cx| { this.leave_settings(window, cx); this.active = index; cx.notify(); })))
                .child(Button::new(("close-tab", index)).icon(IconName::X).accessibility_label(format!("关闭{title}")).tooltip(format!("关闭{title}")).small().ghost()
                    .on_click(cx.listener(move |this, _, window, cx| this.close_tab(index, window, cx)))));
        }
        let top = title_bar(r)
            .child(div().flex().items_center().flex_1().min_w_0().h_full().gap(space::S4)
                .child(tabs)
                .child(div().id("new-tab-area").occlude().child(Button::new("new-tab").icon(IconName::Plus).accessibility_label("打开启动台").tooltip("打开启动台").small().ghost().on_click(cx.listener(|this, _, window, cx| this.launchpad(window, cx)))))
                .child(div().flex_1())
                .child(div().id("title-actions").occlude().flex().items_center().gap(space::S4)
                    .child(Button::new("appearance").icon(IconName::Palette).label("外观").small().ghost().on_click(cx.listener(|this, _, window, cx| this.open_settings(SettingsPage::Appearance, window, cx))))
                    .child(Button::new("settings").icon(IconName::Settings).label("设置").small().ghost().on_click(cx.listener(|this, _, window, cx| this.open_settings(SettingsPage::Models, window, cx))))));
        let content = if let Some(page) = self.settings {
            let query = self.settings_query.read(cx).value().trim().to_lowercase();
            // Narrow windows stack: the same items become a wrapping row above the page.
            let mut nav = if narrow { ui::nav_row(&r) } else { ui::nav(&r) }
                .child(ui::nav_item("settings-return", IconName::ArrowLeft, "返回", false, &r, cx.listener(|this, _, window, cx| { this.leave_settings(window, cx); cx.notify(); })))
                .child(div().when(narrow, |d| d.w(ui::INLINE_SEARCH)).py(space::S4).child(Input::new(&self.settings_query).small()));
            let mut found = false;
            for (section, (group, items)) in SETTINGS.iter().enumerate() {
                let matches: Vec<_> = items.iter().copied().filter(|(_, _, title, terms)| query.is_empty() || title.contains(&query) || terms.contains(&query)).collect();
                if matches.is_empty() { continue; }
                if !narrow { nav = nav.child(ui::nav_section(*group, !found, &r)); }
                found = true;
                for (item, (target, icon, title, _)) in matches.into_iter().enumerate() {
                    nav = nav.child(ui::nav_item(("settings-page", section * 10 + item), icon, title, page == target, &r,
                        cx.listener(move |this, _, window, cx| this.open_settings(target, window, cx))));
                }
            }
            if !found { nav = nav.child(label(text::METADATA, "没有匹配的设置项").text_color(r.text_subtle).py(space::S4)); }
            let page_view = match page {
                SettingsPage::Models => self.models.clone().into_any_element(),
                SettingsPage::Search => self.search.clone().into_any_element(),
                SettingsPage::Mcp => self.mcp.clone().into_any_element(),
                SettingsPage::Appearance => self.appearance.clone().into_any_element(),
            };
            (if narrow { ui::stacked(nav, page_view) } else { ui::shell(nav, page_view) }).into_any_element()
        } else {
            let page = match self.tabs[self.active] {
                TabKind::App(0) => {
                    let panel = self.panel.clone();
                    self.models.update(cx, |models, cx| panel.update(cx, |panel, cx| {
                        panel.set_external_busy(self.chat.picking());
                        let metadata = self.chat.session.history().model.as_ref().and_then(|model| models.model_settings(model));
                        panel.render_session(&self.chat.session, models.available_models(), models.image_models(), metadata, window, cx)
                    }))
                }
                TabKind::Launchpad => {
                    let mut tiles = div().flex().flex_wrap().gap(space::S12);
                    for (app, title) in APPS.iter().copied().enumerate() {
                        tiles = tiles.child(Button::new(("launch", app)).label(title).icon(app_icon(app)).w(TILE_W).h(TILE_H)
                            .bg(r.container_elevated_bg).border_1().border_color(r.border_muted)
                            .on_click(cx.listener(move |this, _, window, cx| this.launch(app, window, cx))));
                    }
                    div().id("launchpad").size_full().overflow_y_scroll().p(space::S24).flex().flex_col().gap(space::S24)
                        .child(div().flex().flex_col().gap(space::S4).child(label(text::HEADING_SM, "启动台"))
                            .child(label(text::METADATA, "选择应用；已打开的应用会出现在左侧和上方。").text_color(r.text_subtle)))
                        .child(tiles)
                        .into_any_element()
                }
                TabKind::App(app) => div().p(space::S24).flex().flex_col().gap(space::S4).child(label(text::HEADING_SM, APPS[app]))
                    .child(label(text::METADATA, "应用业务尚未接入。").text_color(r.text_subtle)).into_any_element(),
            };
            let mut row = div().flex().size_full().min_h_0();
            if !narrow {
                let mut rail = div().flex().flex_col().items_center().gap(space::S8).w(RAIL).flex_shrink_0().py(space::S8)
                    .bg(r.container_sidebar_bg).border_r_1().border_color(r.border_muted)
                    .child(ActivityItem::new("home", IconName::LayoutDashboard, "启动台").active(self.tabs[self.active] == TabKind::Launchpad)
                        .on_click(cx.listener(|this, _, window, cx| this.launchpad(window, cx))));
                for (index, tab) in self.tabs.iter().copied().enumerate() {
                    if let TabKind::App(app) = tab {
                        rail = rail.child(ActivityItem::new(("rail", index), app_icon(app), APPS[app]).active(index == self.active)
                            .on_click(cx.listener(move |this, _, window, cx| { this.leave_settings(window, cx); this.active = index; cx.notify(); })));
                    }
                }
                rail = rail.child(div().flex_1()).child(ActivityItem::new("rail-settings", IconName::Settings, "设置")
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(SettingsPage::Models, window, cx))));
                row = row.child(rail);
            }
            row.child(div().flex_1().min_w_0().h_full().min_h_0().child(page)).into_any_element()
        };
        root(r).child(top).child(div().flex_1().min_h_0().child(content))
    }
}

struct StartupView(Result<Entity<DesktopApp>, String>);
impl Render for StartupView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.0 {
            Ok(app) => app.clone().into_any_element(),
            Err(error) => {
                // Without the system caption this bar is the only way to move or close the window.
                // `main` installs the default theme before any window opens.
                let r = LumaTheme::global(cx).roles;
                root(r)
                    .child(title_bar(r).child(div().flex_1()))
                    .child(div().p(space::S24).flex().flex_col().gap(space::S12)
                        .child(label(text::HEADING_SM, "配置加载失败"))
                        .child(label(text::UI, error.clone()))
                        .child(label(text::METADATA, "原配置没有被覆盖。请检查文件权限或修复配置后重新启动。").text_color(r.text_subtle)))
                    .into_any_element()
            }
        }
    }
}

/// The window's root: workspace background and the UI type style every descendant inherits.
fn root(r: Roles) -> Div {
    div().size_full().flex().flex_col().text_style(text::UI).bg(r.container_workspace_bg).text_color(r.text_default)
}

/// The window's own title bar; Kit draws the minimize / maximize / close controls on its right.
fn title_bar(r: Roles) -> TitleBar {
    TitleBar::new().h(size::TOP_BAR).pl(space::S8).bg(r.container_header_bg).border_color(r.border_muted)
}

fn main() {
    gpui_kit::application().with_assets(lumakit_design::UiAssets).with_quit_mode(QuitMode::LastWindowClosed).run(|cx| {
        gpui_kit::init(cx);
        // Logging starts with the data directory; the window reports a failure once it exists.
        let fonts = lumakit_design::fonts::register_bundled(cx).err().map(|error| format!("{error:#}"));
        gpui_kit::component::set_locale("zh-CN");
        // A theme exists before the first frame even if loading the user's preferences fails.
        match lumakit_design::ThemeCatalog::bundled() {
            Ok(catalog) => catalog.resolve(lumakit_design::theme::DEFAULT_THEME, None).theme.install(None, cx),
            Err(error) => { eprintln!("内置主题无法加载：{error}"); cx.quit(); return; }
        }
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        // No system caption: the app's own title bar draws the window controls and handles
        // dragging. The title stays for the taskbar and Alt+Tab.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(320.), px(480.))),
            titlebar: Some(TitlebarOptions { title: Some("LumaDesk GPUI".into()), ..TitleBar::title_bar_options() }),
            ..TitleBar::window_options()
        };
        if let Err(error) = gpui_kit::open_window(options, cx, move |window, cx| {
            let loaded = DesktopApp::create(fonts, window, cx);
            cx.new(|_| StartupView(loaded))
        }) { eprintln!("无法创建 LumaDesk 窗口：{error}"); cx.quit(); }
    });
}
