#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chat;
mod model_settings;
mod model_editor;
mod search_settings;
mod settings_store;
mod mcp_settings;
mod theme;

use eframe::egui::{self, Align, FontData, FontDefinitions, FontFamily, Layout, RichText, Vec2};
const BOOKS: &str = "\u{E758}";
const CHAT: &str = "\u{E15C}";
const GEAR: &str = "\u{E270}";
const HOUSE: &str = "\u{E2C2}";
const NOTE: &str = "\u{E348}";
const PENCIL: &str = "\u{E3AE}";
const TRANSLATE: &str = "\u{E4A2}";
use model_settings::ModelSettings;
use search_settings::SearchSettings;


const APPS: [(&str, &str); 5] = [(CHAT, "对话"), (PENCIL, "工作"), (NOTE, "笔记"), (BOOKS, "知识库"), (TRANSLATE, "翻译")];
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabKind { Launchpad, App(usize) }
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage { Models, Search, Appearance, Mcp }

struct DesktopApp {
    _data_lock: lumakit_support::FileLock,
    _logs: lumakit_observability::LogHandle,
    models: ModelSettings,
    search: SearchSettings,
    mcp: mcp_settings::McpSettings,
    chat: chat::ChatState,
    tabs: Vec<TabKind>,
    active: usize,
    settings: Option<SettingsPage>,
    settings_query: String,
    appearance: theme::Appearance,
}


impl DesktopApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let mut fonts = FontDefinitions::default();
        lumakit_egui::install_icon_font(&mut fonts);
        let windows = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:/Windows".into());
        let data = std::fs::read(windows.join("Fonts/msyh.ttc")).map_err(|_| "无法加载中文字体：请确认 Windows 微软雅黑已安装".to_owned())?;
        fonts.font_data.insert("chinese".into(), FontData::from_owned(data).into());
        fonts.families.entry(FontFamily::Proportional).or_default().push("chinese".into());
        fonts.families.entry(FontFamily::Monospace).or_default().push("chinese".into());
        cc.egui_ctx.set_fonts(fonts);
        let root = match std::env::var_os("LUMADESK_DATA_DIR") { Some(path) => std::path::PathBuf::from(path), None => lumakit_support::data_root("luma-desk").map_err(|error| error.to_string())? };
        std::fs::create_dir_all(&root).map_err(|_| "无法创建配置目录，请将程序移动到可写目录".to_owned())?;
        let data_lock = lumakit_support::FileLock::acquire(&root.join("lumadesk.lock")).map_err(|_| "此数据目录已在使用，或无法加锁；请关闭另一个实例后重试".to_owned())?;
        let logs = lumakit_observability::LogHandle::init(&root.join("logs"), lumakit_observability::LogLevel::Info, 5 * 1024 * 1024).map_err(|error| format!("无法初始化诊断日志：{error}"))?;
        tracing::info!(version = env!("CARGO_PKG_VERSION"), "LumaDesk started");
        let appearance = theme::Appearance::load(&root)?;
        appearance.apply(&cc.egui_ctx);
        Ok(Self { _data_lock: data_lock, _logs: logs, models: ModelSettings::new(&root)?, search: SearchSettings::new(&root)?, mcp: mcp_settings::McpSettings::new(&root)?, chat: chat::ChatState::new(&root, &cc.egui_ctx)?, tabs: vec![TabKind::Launchpad], active: 0, settings: None, settings_query: String::new(), appearance })
    }
    fn leave_settings(&mut self) {
        self.models.clear_secrets(); self.search.clear_secrets(); self.mcp.clear_secrets(); self.settings = None;
    }
    fn launchpad(&mut self) {
        self.leave_settings();
        if let Some(index) = self.tabs.iter().position(|kind| *kind == TabKind::Launchpad) { self.active = index; }
        else { self.tabs.push(TabKind::Launchpad); self.active = self.tabs.len() - 1; }
    }
    fn launch(&mut self, app: usize) {
        if let Some(existing) = self.tabs.iter().position(|kind| *kind == TabKind::App(app)) {
            if self.tabs[self.active] == TabKind::Launchpad { self.tabs.remove(self.active); self.active = if existing > self.active { existing - 1 } else { existing }; }
            else { self.active = existing; }
        } else { self.tabs[self.active] = TabKind::App(app); }
    }
    fn close_tab(&mut self, index: usize) {
        if self.tabs[index] == TabKind::App(0) { self.chat.cancel(); }
        self.tabs.remove(index);
        if self.tabs.is_empty() { self.tabs.push(TabKind::Launchpad); self.active = 0; }
        else if self.active > index { self.active -= 1; }
        else { self.active = self.active.min(self.tabs.len() - 1); }
    }
    fn topbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("topbar").exact_size(48.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // 先预留右侧按钮，再让标签滚动；主题按钮占满余宽时设置会被推出窗口。
                let tabs_width = (ui.available_width() - 174.0).max(120.0);
                ui.allocate_ui_with_layout(Vec2::new(tabs_width, 32.0), Layout::left_to_right(Align::Center), |ui| {
                    let mut close = None;
                    egui::ScrollArea::horizontal().id_salt("app-tabs").show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for index in 0..self.tabs.len() {
                                ui.push_id(index, |ui| {
                                    let title = match self.tabs[index] { TabKind::Launchpad => "启动台", TabKind::App(app) => APPS[app].1 };
                                    if ui.selectable_label(self.settings.is_none() && self.active == index, title).clicked() { self.leave_settings(); self.active = index; }
                                    if ui.small_button("×").on_hover_text(format!("关闭 {title}")).clicked() { close = Some(index); }
                                });
                            }
                            if ui.button("+").on_hover_text("打开启动台").clicked() { self.launchpad(); }
                        });
                    });
                    if let Some(index) = close { self.close_tab(index); }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(format!("{GEAR} 设置")).clicked() { self.leave_settings(); self.settings = Some(SettingsPage::Models); }
                    if ui.button("外观").clicked() { self.leave_settings(); self.settings = Some(SettingsPage::Appearance); }
                });
            });
        });
    }
}
impl eframe::App for DesktopApp {
    fn logic(&mut self, _: &egui::Context, _: &mut eframe::Frame) { self.chat.poll(); }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.appearance.apply(&ctx);
        ui.set_style(ctx.global_style());
        if ctx.input(|input| input.viewport().close_requested()) && !self.chat.can_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.leave_settings();
            if let Some(index) = self.tabs.iter().position(|tab| *tab == TabKind::App(0)) { self.active = index; }
            else { self.launch(0); }
        }
        self.topbar(ui);
        egui::Panel::left("app-rail").exact_size(64.0).resizable(false).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                if ui.add_sized([44.0,40.0], egui::Button::new(HOUSE)).on_hover_text("启动台").clicked() { self.launchpad(); }
                for index in 0..self.tabs.len() {
                    if let TabKind::App(app) = self.tabs[index] {
                        ui.push_id(app, |ui| { if ui.add_sized([44.0,40.0], egui::Button::new(APPS[app].0).selected(self.settings.is_none() && index == self.active)).on_hover_text(APPS[app].1).clicked() { self.leave_settings(); self.active = index; } });
                    }
                }
            });
            ui.with_layout(Layout::bottom_up(Align::Center), |ui| { if ui.button(format!("{GEAR}\n设置")).clicked() { self.leave_settings(); self.settings = Some(SettingsPage::Models); } });
        });
        if let Some(page) = self.settings {
            egui::Panel::top("settings-heading").exact_size(54.0).show(ui, |ui| {
                ui.horizontal_centered(|ui| { if ui.button("← 返回").clicked() { self.leave_settings(); } ui.heading("设置"); });
            });
            egui::Panel::left("settings-nav").exact_size(190.0).resizable(false).show(ui, |ui| {
                ui.add_space(12.0);
                ui.add(egui::TextEdit::singleline(&mut self.settings_query).hint_text("搜索设置").desired_width(f32::INFINITY));
                ui.add_space(theme::tokens::SPACE_LG); ui.weak("偏好设置");
                let query = self.settings_query.trim().to_lowercase();
                let mut shown = false;
                for (target, label, terms) in [(SettingsPage::Models, "模型服务", "provider api 密钥 协议 模型 ai"), (SettingsPage::Search, "网络搜索", "search fetch 网页 黑名单 密钥 ai"), (SettingsPage::Mcp, "MCP 工具", "mcp 工具 tool server 信任 skills"), (SettingsPage::Appearance, "外观", "appearance theme 主题 强调色 背景 密度 浅色 深色 系统")] {
                    if query.is_empty() || label.contains(&query) || terms.contains(&query) {
                        shown = true;
                        if ui.selectable_label(page == target, label).clicked() { self.models.clear_secrets(); self.search.clear_secrets(); self.mcp.clear_secrets(); self.settings = Some(target); }
                    }
                }
                if !shown { ui.weak("没有匹配的设置项"); }
            });
            egui::CentralPanel::default().show(ui, |ui| {
                match self.settings { Some(SettingsPage::Models) => self.models.ui(ui), Some(SettingsPage::Search) => self.search.ui(ui), Some(SettingsPage::Mcp) => self.mcp.ui(ui), Some(SettingsPage::Appearance) => self.appearance.ui(ui), None => {} }
            });
        } else {
            egui::CentralPanel::default().show(ui, |ui| {
                if self.tabs[self.active] == TabKind::App(0) {
                    if self.chat.ui(ui, &self.models) { self.settings = Some(SettingsPage::Models); }
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(28.0);
                        match self.tabs[self.active] {
                            TabKind::Launchpad => {
                                ui.heading("启动台"); ui.weak("选择应用；已打开的应用会出现在左侧和上方。"); ui.add_space(24.0);
                                ui.horizontal_wrapped(|ui| { for (app, (icon, title)) in APPS.iter().enumerate() { if ui.add_sized([112.0,88.0], egui::Button::new(RichText::new(format!("{icon}\n{title}")).size(20.0))).clicked() { self.launch(app); } } });
                            }
                            TabKind::App(app) => { ui.heading(APPS[app].1); ui.weak("应用业务尚未接入。"); }
                        }
                    });
                }
            });
        }
    }
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_title("LumaDesk").with_inner_size([1200.0,800.0]).with_min_inner_size([860.0,560.0]), ..Default::default() };
    eframe::run_native("luma-desk", options, Box::new(|cc| {
        match DesktopApp::new(cc) {
            Ok(app) => Ok(Box::new(app)),
            Err(message) => Ok(Box::new(StartupError(message))),
        }
    }))
}

struct StartupError(String);
impl eframe::App for StartupError {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("配置加载失败");
            ui.label(&self.0);
            ui.label("原配置没有被覆盖。请检查文件权限或修复配置后重新启动。");
        });
    }
}

