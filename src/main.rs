#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
extern crate gpui_kit as gpui;

mod app_view;
mod chat;
mod chat_usage;
mod log_settings;
mod mcp_settings;
mod model_editor;
mod model_settings;
mod search_settings;
mod settings_store;
mod settings_task;
mod startup;
mod theme;
mod usage_settings;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Selectable, Sizable, TitleBar,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use lumakit_ai::chat::ChatAction;
use lumakit_design::components::{ActivityItem, TextStyled, label, settings as ui};
use lumakit_design::metrics::{size, space, text};
use lumakit_design::{LumaTheme, Roles};
use lumakit_gpui::ChatPanel;
use std::{path::PathBuf, time::Duration};

const APPS: [&str; 5] = ["对话", "工作", "笔记", "知识库", "翻译"];
/// Width of the app rail and size of a launchpad tile (LumaDesk's own layout).
const RAIL: Pixels = px(64.);
const TILE_W: Pixels = px(112.);
const TILE_H: Pixels = px(88.);
fn app_icon(app: usize) -> IconName {
    match app {
        0 => IconName::Bot,
        1 => IconName::SquareTerminal,
        2 => IconName::FileText,
        3 => IconName::BookOpen,
        _ => IconName::Globe,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabKind {
    Launchpad,
    App(usize),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    Models,
    Search,
    Mcp,
    Usage,
    Appearance,
    Logs,
}
type SettingsItem = (SettingsPage, IconName, &'static str, &'static str);
/// Settings navigation groups, ThinkRail-style: (section, [(page, icon, title, search terms)]).
const SETTINGS: [(&str, &[SettingsItem]); 3] = [
    (
        "AGENT",
        &[
            (
                SettingsPage::Models,
                IconName::KeyRound,
                "模型服务",
                "provider api 密钥 协议 模型 ai agent",
            ),
            (
                SettingsPage::Search,
                IconName::Globe,
                "网络搜索",
                "search fetch 网页 黑名单 密钥 ai agent",
            ),
            (
                SettingsPage::Mcp,
                IconName::Plug,
                "MCP 工具",
                "mcp 工具 tool server 信任 skills agent",
            ),
            (
                SettingsPage::Usage,
                IconName::ScrollText,
                "用量统计",
                "usage token cost 用量 费用 统计 ai agent",
            ),
        ],
    ),
    (
        "界面",
        &[(
            SettingsPage::Appearance,
            IconName::Palette,
            "外观",
            "appearance theme 主题 强调色 背景 密度 浅色 深色 系统",
        )],
    ),
    (
        "系统",
        &[(
            SettingsPage::Logs,
            IconName::ScrollText,
            "后台日志",
            "logs traceid 日志 诊断 级别 链路 系统",
        )],
    ),
];

struct DesktopApp {
    _data_lock: lumakit_support::FileLock,
    logs: Entity<log_settings::LogSettings>,
    models: Entity<model_settings::ModelSettings>,
    search: Entity<search_settings::SearchSettings>,
    mcp: Entity<mcp_settings::McpSettings>,
    appearance: Entity<theme::Appearance>,
    chat: chat::ChatState,
    usage: usage_settings::UsageSettings,
    panel: Entity<ChatPanel>,
    tabs: Vec<TabKind>,
    active: usize,
    settings: Option<SettingsPage>,
    settings_query: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    background: Option<Task<()>>,
}

impl DesktopApp {
    fn busy(&self, cx: &App) -> bool {
        self.chat.is_busy() || self.models.read(cx).is_busy() || self.search.read(cx).is_busy()
    }

    fn ensure_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.background.is_some() || !self.busy(cx) {
            return;
        }
        // ponytail: bounded 40ms polling only while a worker exists; replace with an async wake channel if measured latency requires it.
        self.background = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let keep = cx
                    .update(|_, cx| {
                        this.update(cx, |this, cx| {
                            let mut changed = this.chat.poll();
                            changed |= this.models.update(cx, |models, cx| {
                                let changed = models.poll();
                                if changed {
                                    cx.notify();
                                }
                                changed
                            });
                            changed |= this.search.update(cx, |search, cx| {
                                let changed = search.poll();
                                if changed {
                                    cx.notify();
                                }
                                changed
                            });
                            if changed {
                                this.chat.refresh_usage(this.models.read(cx));
                            }
                            let busy = this.busy(cx);
                            if changed {
                                cx.notify();
                            }
                            if !busy {
                                this.background = None;
                            }
                            busy
                        })
                        .unwrap_or_else(|_| {
                            tracing::debug!(reason = "view_closed", "app.background.discarded");
                            false
                        })
                    })
                    .unwrap_or_else(|_| {
                        tracing::debug!(reason = "window_closed", "app.background.discarded");
                        false
                    });
                if !keep {
                    break;
                }
            }
        }));
    }

    fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.models
            .update(cx, |models, cx| models.clear_secrets(window, cx));
        self.search
            .update(cx, |search, cx| search.clear_secrets(window, cx));
        self.logs.update(cx, |logs, cx| logs.set_active(false, cx));
        self.settings = None;
    }
    fn open_settings(&mut self, page: SettingsPage, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_settings(window, cx);
        self.settings = Some(page);
        if page == SettingsPage::Usage {
            self.refresh_usage();
        }
        if page == SettingsPage::Logs {
            self.logs.update(cx, |logs, cx| logs.set_active(true, cx));
        }
        cx.notify();
    }
    fn launchpad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_settings(window, cx);
        if let Some(index) = self.tabs.iter().position(|tab| *tab == TabKind::Launchpad) {
            self.active = index;
        } else {
            self.tabs.push(TabKind::Launchpad);
            self.active = self.tabs.len() - 1;
        }
        cx.notify();
    }
    fn launch(&mut self, app: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_settings(window, cx);
        if let Some(existing) = self.tabs.iter().position(|tab| *tab == TabKind::App(app)) {
            if self.tabs[self.active] == TabKind::Launchpad {
                self.tabs.remove(self.active);
                self.active = if existing > self.active {
                    existing - 1
                } else {
                    existing
                };
            } else {
                self.active = existing;
            }
        } else {
            self.tabs[self.active] = TabKind::App(app);
        }
        cx.notify();
    }
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs[index] == TabKind::App(0) {
            self.chat.cancel();
            self.ensure_background(window, cx);
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(TabKind::Launchpad);
            self.active = 0;
        } else if self.active > index {
            self.active -= 1;
        } else {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        cx.notify();
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
                    .child(
                        div()
                            .p(space::S24)
                            .flex()
                            .flex_col()
                            .gap(space::S12)
                            .child(label(text::HEADING_SM, "配置加载失败"))
                            .child(label(text::UI, error.clone()))
                            .child(
                                label(
                                    text::METADATA,
                                    "原配置没有被覆盖。请检查文件权限或修复配置后重新启动。",
                                )
                                .text_color(r.text_subtle),
                            ),
                    )
                    .into_any_element()
            }
        }
    }
}

/// The window's root: workspace background and the UI type style every descendant inherits.
fn root(r: Roles) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .text_style(text::UI)
        .bg(r.container_workspace_bg)
        .text_color(r.text_default)
}

/// The window's own title bar; Kit draws the minimize / maximize / close controls on its right.
fn title_bar(r: Roles) -> TitleBar {
    TitleBar::new()
        .h(size::TOP_BAR)
        .pl(space::S8)
        .bg(r.container_header_bg)
        .border_color(r.border_muted)
}

fn main() {
    gpui_kit::application()
        .with_assets(lumakit_usage_status::UsageStatusAssets(
            lumakit_gpui::PickerAssets(lumakit_design::UiAssets),
        ))
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            gpui_kit::init(cx);
            // Logging starts with the data directory; the window reports a failure once it exists.
            let fonts = lumakit_design::fonts::register_bundled(cx)
                .err()
                .map(|_| "内置字体注册失败".to_owned());
            gpui_kit::component::set_locale("zh-CN");
            // A theme exists before the first frame even if loading the user's preferences fails.
            match lumakit_design::ThemeCatalog::bundled() {
                Ok(catalog) => catalog
                    .resolve(lumakit_design::theme::DEFAULT_THEME, None)
                    .theme
                    .install(None, cx),
                Err(_) => {
                    eprintln!("APP_THEME_LOAD_FAILED: 内置主题无法加载");
                    cx.quit();
                    return;
                }
            }
            let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
            // No system caption: the app's own title bar draws the window controls and handles
            // dragging. The title stays for the taskbar and Alt+Tab.
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(320.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("LumaDesk GPUI".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            };
            if gpui_kit::open_window(options, cx, move |window, cx| {
                let loaded = DesktopApp::create(fonts, window, cx);
                cx.new(|_| StartupView(loaded))
            })
            .is_err()
            {
                eprintln!("APP_WINDOW_OPEN_FAILED: 无法创建 LumaDesk 窗口");
                cx.quit();
            }
        });
}
