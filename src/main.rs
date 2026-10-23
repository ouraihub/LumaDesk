#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chat;
mod model_settings;
mod search_settings;

use eframe::egui::{self, Align, Color32, FontData, FontDefinitions, FontFamily, Layout, RichText, Vec2};
use egui_phosphor::regular::{BOOKS, CHAT, GEAR, HOUSE, NOTE, PENCIL, TRANSLATE};
use model_settings::ModelSettings;
use search_settings::SearchSettings;


const APPS: [(&str, &str); 5] = [(CHAT, "对话"), (PENCIL, "工作"), (NOTE, "笔记"), (BOOKS, "知识库"), (TRANSLATE, "翻译")];
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabKind { Launchpad, App(usize) }
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage { Models, Search }

struct DesktopApp {
    _data_lock: lumakit_support::FileLock,
    models: ModelSettings,
    search: SearchSettings,
    chat: chat::ChatState,
    tabs: Vec<TabKind>,
    active: usize,
    settings: Option<SettingsPage>,
    settings_query: String,
    dark: bool,
}

// 配色只在这一处定义；浅色文字硬编码到深色页的现象是切换主题后标题消失。
fn visuals(dark: bool) -> egui::Visuals {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let (ink, muted, paper, surface, card, border, hover, pressed, accent) = if dark {
        (Color32::from_rgb(230,240,245), Color32::from_rgb(171,193,205), Color32::from_rgb(25,42,54), Color32::from_rgb(36,56,70), Color32::from_rgb(30,48,61), Color32::from_rgb(72,99,115), Color32::from_rgb(49,75,92), Color32::from_rgb(60,88,105), Color32::from_rgb(45,105,138))
    } else {
        (Color32::from_rgb(35,59,73), Color32::from_rgb(77,103,117), Color32::from_rgb(241,247,249), Color32::from_rgb(229,240,245), Color32::from_rgb(250,253,254), Color32::from_rgb(193,211,221), Color32::from_rgb(214,232,240), Color32::from_rgb(194,218,229), Color32::from_rgb(43,105,139))
    };
    v.override_text_color = None;
    v.panel_fill = paper;
    v.window_fill = card;
    v.faint_bg_color = surface;
    v.extreme_bg_color = card;
    v.hyperlink_color = if dark { Color32::from_rgb(132,202,241) } else { accent };
    v.selection.bg_fill = accent;
    v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    for widget in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        widget.bg_stroke = egui::Stroke::new(1.0_f32, border);
        widget.fg_stroke.color = ink;
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    v.widgets.noninteractive.bg_fill = card;
    v.widgets.noninteractive.fg_stroke.color = muted;
    v.widgets.inactive.bg_fill = surface;
    v.widgets.inactive.weak_bg_fill = surface;
    v.widgets.hovered.bg_fill = hover;
    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.active.bg_fill = pressed;
    v.widgets.active.weak_bg_fill = pressed;
    v.widgets.open.bg_fill = hover;
    v
}

impl DesktopApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let mut fonts = FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        let windows = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:/Windows".into());
        let data = std::fs::read(windows.join("Fonts/msyh.ttc")).map_err(|_| "无法加载中文字体：请确认 Windows 微软雅黑已安装".to_owned())?;
        fonts.font_data.insert("chinese".into(), FontData::from_owned(data).into());
        fonts.families.entry(FontFamily::Proportional).or_default().push("chinese".into());
        fonts.families.entry(FontFamily::Monospace).or_default().push("chinese".into());
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.style_mut(|style| { style.spacing.item_spacing = Vec2::new(8.0, 10.0); style.spacing.button_padding = Vec2::new(10.0, 6.0); });
        let root = match std::env::var_os("LUMADESK_DATA_DIR") { Some(path) => std::path::PathBuf::from(path), None => lumakit_support::data_root("luma-desk").map_err(|error| error.to_string())? };
        std::fs::create_dir_all(&root).map_err(|_| "无法创建配置目录，请将程序移动到可写目录".to_owned())?;
        let data_lock = lumakit_support::FileLock::acquire(&root.join("lumadesk.lock")).map_err(|_| "此数据目录已在使用，或无法加锁；请关闭另一个实例后重试".to_owned())?;
        Ok(Self { _data_lock: data_lock, models: ModelSettings::new(&root)?, search: SearchSettings::new(&root)?, chat: chat::ChatState::new(&root, &cc.egui_ctx)?, tabs: vec![TabKind::Launchpad], active: 0, settings: None, settings_query: String::new(), dark: false })
    }
    fn leave_settings(&mut self) {
        self.models.clear_secrets(); self.search.clear_secrets(); self.settings = None;
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
    fn topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar").exact_height(48.0).show(ctx, |ui| {
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
                    if ui.button(if self.dark { "浅色" } else { "深色" }).clicked() { self.dark = !self.dark; }
                });
            });
        });
    }
}
impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.set_visuals(visuals(self.dark));
        self.chat.poll();
        if ctx.input(|input| input.viewport().close_requested()) && !self.chat.can_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.leave_settings();
            if let Some(index) = self.tabs.iter().position(|tab| *tab == TabKind::App(0)) { self.active = index; }
            else { self.launch(0); }
        }
        self.topbar(ctx);
        egui::SidePanel::left("app-rail").exact_width(64.0).resizable(false).show(ctx, |ui| {
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
            egui::TopBottomPanel::top("settings-heading").exact_height(54.0).show(ctx, |ui| {
                ui.horizontal_centered(|ui| { if ui.button("← 返回").clicked() { self.leave_settings(); } ui.heading("设置"); });
            });
            egui::SidePanel::left("settings-nav").exact_width(190.0).resizable(false).show(ctx, |ui| {
                ui.add_space(12.0);
                ui.add(egui::TextEdit::singleline(&mut self.settings_query).hint_text("搜索设置").desired_width(f32::INFINITY));
                ui.add_space(16.0); ui.weak("AI");
                let query = self.settings_query.trim().to_lowercase();
                let mut shown = false;
                for (target, label, terms) in [(SettingsPage::Models, "模型服务", "provider api 密钥 协议 模型"), (SettingsPage::Search, "网络搜索", "search fetch 网页 黑名单 密钥")] {
                    if query.is_empty() || label.contains(&query) || terms.contains(&query) || query == "ai" {
                        shown = true;
                        if ui.selectable_label(page == target, label).clicked() { self.models.clear_secrets(); self.search.clear_secrets(); self.settings = Some(target); }
                    }
                }
                if !shown { ui.weak("没有匹配的设置项"); }
            });
            egui::CentralPanel::default().show(ctx, |ui| {
                match self.settings { Some(SettingsPage::Models) => self.models.ui(ui), Some(SettingsPage::Search) => self.search.ui(ui), None => {} }
            });
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
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
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("配置加载失败");
            ui.label(&self.0);
            ui.label("原配置没有被覆盖。请检查文件权限或修复配置后重新启动。");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color32) -> f32 {
        let [r, g, b, _] = color.to_array();
        let channel = |v: u8| { let v = v as f32 / 255.0; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    #[test]
    fn selected_and_interactive_text_remain_readable_in_both_themes() {
        for dark in [false, true] {
            let v = visuals(dark);
            let pairs = [(v.selection.stroke.color, v.selection.bg_fill), (v.widgets.inactive.fg_stroke.color, v.widgets.inactive.bg_fill), (v.widgets.hovered.fg_stroke.color, v.widgets.hovered.bg_fill), (v.widgets.active.fg_stroke.color, v.widgets.active.bg_fill), (v.widgets.noninteractive.fg_stroke.color, v.panel_fill)];
            for (fg, bg) in pairs {
                let (a, b) = (luminance(fg), luminance(bg));
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5, "主题 dark={dark} 的文字与底色对比不足");
            }
        }
    }
}
