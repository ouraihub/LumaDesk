use std::{path::{Path, PathBuf}, sync::Arc};
 use eframe::egui::{self, Color32, FontId, Stroke, TextStyle, Vec2};
 use serde::{Deserialize, Serialize};
use crate::settings_store;

pub(crate) mod tokens {
    pub const CONTROL: f32 = 30.0;
    pub const CONTROL_SMALL: f32 = 28.0;
    pub const ROW: f32 = 36.0;
    pub const TEXT: f32 = 13.0;
    pub const TEXT_SMALL: f32 = 11.5;
    pub const TITLE: f32 = 15.0;
    pub const MONO: f32 = 12.0;
    pub const SPACE_XS: f32 = 4.0;
    pub const SPACE_SM: f32 = 8.0;
    pub const SPACE_MD: f32 = 12.0;
    pub const SPACE_LG: f32 = 16.0;
    pub const SPACE_XL: f32 = 24.0;
    pub const PAGE_MAX: f32 = 760.0;
    pub const PROVIDER_WIDTH: f32 = 224.0;
    pub const COLUMN_GAP: f32 = 24.0;
    pub const STACK_AT: f32 = 600.0;
    pub const DETAIL_MIN: f32 = 300.0;
    pub const DIALOG_WIDTH: f32 = 460.0;
    pub const DIALOG_MIN: f32 = 280.0;
    pub const RADIUS: u8 = 8;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Theme { #[default] Light, Dark, System, NativeLight }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Accent { #[default] Red, Blue, Green, Yellow, Cyan, Purple, Gray }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Background { #[default] Warm, White }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Density { #[default] Comfortable, Compact }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences { theme: Theme, accent: Accent, background: Background, density: Density }

pub(crate) struct Appearance {
    root: PathBuf,
    saved: Preferences,
    styles: [Arc<egui::Style>; 2],
    error: Option<String>,
}

impl Appearance {
    pub(crate) fn load(root: &Path) -> Result<Self, String> {
        let saved = match settings_store::load_appearance(root)? {
            Some(value) => serde_json::from_value(value).map_err(|_| "外观设置 JSON 无效；原文件未修改".to_owned())?,
            None => {
                let value = match lumakit_support::load_versioned(&root.join("appearance.json"), 1).map_err(|_| "无法读取 appearance.json；请检查版本、格式或权限，原文件未修改".to_owned())? {
                    None => Preferences::default(), Some((1, value)) => value, Some(_) => return Err("外观设置版本不受支持；原文件未修改".into()),
                };
                settings_store::save_appearance(root, &serde_json::to_value(value).map_err(|_| "外观设置序列化失败".to_owned())?)?;
                value
            }
        };
        Ok(Self { root: root.to_owned(), saved, styles: styles(saved), error: None })
    }
    fn save(&mut self, next: Preferences) -> Result<(), String> {
        settings_store::save_appearance(&self.root, &serde_json::to_value(next).map_err(|_| "外观设置序列化失败".to_owned())?)
            .map_err(|_| "保存外观失败；请检查目录权限或磁盘空间。原设置保持不变，可重新选择重试".to_owned())?;
        self.saved = next;
        self.styles = styles(next);
        Ok(())
    }


    pub(crate) fn apply(&self, ctx: &egui::Context) {
        let dark = match self.saved.theme {
            Theme::Light | Theme::NativeLight => false,
            Theme::Dark => true,
            Theme::System => ctx.input(|i| i.raw.system_theme == Some(egui::Theme::Dark)),
        };
        ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.set_global_style(self.styles[usize::from(dark)].clone());
    }

    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(tokens::PAGE_MAX);
            ui.heading("外观");
            ui.weak("选择后立即保存；主题与密度应用于整个工作区。");
            ui.add_space(tokens::SPACE_XL);
            let mut next = self.saved;
            ui.label("主题");
            ui.horizontal_wrapped(|ui| {
                for (value, label) in [(Theme::NativeLight, "简洁浅色"), (Theme::Light, "自定义浅色"), (Theme::Dark, "深色"), (Theme::System, "跟随系统")] {
                    if ui.selectable_value(&mut next.theme, value, label).changed() && value == Theme::NativeLight {
                        next.accent = Accent::Blue;
                        next.background = Background::White;
                        next.density = Density::Comfortable;
                    }
                }
            });
            if next.theme == Theme::NativeLight { ui.weak("原生浅色控件；强调色、背景和密度仍可调整，选择后立即保存。"); }
            ui.add_space(tokens::SPACE_LG);
            ui.horizontal(|ui| {
                ui.label("强调色");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (value, label) in [(Accent::Gray, "灰"), (Accent::Purple, "紫"), (Accent::Cyan, "青"), (Accent::Yellow, "黄"), (Accent::Green, "绿"), (Accent::Blue, "蓝"), (Accent::Red, "朱红")] {
                        let preview = visuals(Preferences { accent: value, ..next }, ui.visuals().dark_mode);
                        if color_option(ui, preview.selection.bg_fill, next.accent == value, label, true).clicked() { next.accent = value; }
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("背景色");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (value, label) in [(Background::White, "纯白"), (Background::Warm, "暖纸")] {
                        let preview = visuals(Preferences { background: value, ..next }, ui.visuals().dark_mode);
                        if color_option(ui, preview.panel_fill, next.background == value, label, false).clicked() { next.background = value; }
                    }
                });
            });
            ui.add_space(tokens::SPACE_LG);
            ui.label("密度");
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut next.density, Density::Comfortable, "舒适");
                ui.selectable_value(&mut next.density, Density::Compact, "紧凑");
            });
            if next != self.saved {
                self.error = self.save(next).err();
                if self.error.is_none() {
                    self.apply(ui.ctx());
                    ui.set_style(ui.ctx().global_style());
                    ui.ctx().request_repaint();
                }
            }
            if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
        });
    }
}

// 色卡保留原生 Button 的点击、键盘和禁用语义；只替换视觉，避免只画颜色没有交互。
fn color_option(ui: &mut egui::Ui, color: Color32, selected: bool, label: &str, round: bool) -> egui::Response {
    let size = if round { Vec2::splat(40.0) } else { Vec2::new(112.0, 42.0) };
    let response = ui.add_sized(size, egui::Button::new("").frame(false))
        .on_hover_text(label).accessible_name(label);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label));
    let rect = response.rect.shrink(3.0);
    let foreground = ui.visuals().text_color();
    let ring = if selected { if round { color } else { ui.visuals().selection.bg_fill } } else if response.has_focus() { foreground } else { ui.visuals().widgets.inactive.bg_stroke.color };
    let stroke = Stroke::new(if selected || response.has_focus() { 2.0 } else { 1.0 }, ring);
    if round {
        let radius = rect.width() / 2.0;
        ui.painter().circle_filled(rect.center(), radius - 4.0, color);
        if selected || response.hovered() || response.has_focus() { ui.painter().circle_stroke(rect.center(), radius, stroke); }
        if selected { ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "✓", FontId::proportional(18.0), if color.intensity() > 0.6 { Color32::BLACK } else { Color32::WHITE }); }
    } else {
        ui.painter().rect_filled(rect, tokens::RADIUS, color);
        ui.painter().rect_stroke(rect, tokens::RADIUS, stroke, egui::StrokeKind::Inside);
        let sample = egui::pos2(rect.left() + 18.0, rect.center().y);
        ui.painter().circle(sample, 9.0, color, Stroke::new(1.0, foreground.gamma_multiply(0.45)));
        ui.painter().text(egui::pos2(rect.left() + 34.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(tokens::TEXT), foreground);
        if selected { ui.painter().text(egui::pos2(rect.right() - 12.0, rect.center().y), egui::Align2::CENTER_CENTER, "✓", FontId::proportional(14.0), foreground); }
    }
    response
}

fn styles(preferences: Preferences) -> [Arc<egui::Style>; 2] {
    [false, true].map(|dark| {
        let mut style = egui::Style::default();
        style.visuals = visuals(preferences, dark);
        let compact = preferences.density == Density::Compact;
        if preferences.theme == Theme::NativeLight {
            if compact {
                style.spacing.item_spacing.y *= 0.5;
                style.spacing.button_padding.y *= 0.75;
                style.spacing.interact_size.y *= 0.9;
            }
            return Arc::new(style);
        }
        style.spacing.item_spacing = Vec2::new(tokens::SPACE_SM, if compact { tokens::SPACE_XS } else { tokens::SPACE_SM });
        style.spacing.button_padding = Vec2::new(tokens::SPACE_SM, if compact { 4.0 } else { 6.0 });
        style.spacing.interact_size.y = if compact { tokens::CONTROL_SMALL } else { tokens::CONTROL };
        for (kind, size) in [(TextStyle::Body, tokens::TEXT), (TextStyle::Button, tokens::TEXT), (TextStyle::Small, tokens::TEXT_SMALL), (TextStyle::Heading, tokens::TITLE)] {
            style.text_styles.insert(kind, FontId::proportional(size));
        }
        style.text_styles.insert(TextStyle::Monospace, FontId::monospace(tokens::MONO));
        Arc::new(style)
    })
}

fn visuals(p: Preferences, dark: bool) -> egui::Visuals {
    let dark = dark && p.theme != Theme::NativeLight;
    let rgb = |r, g, b| Color32::from_rgb(r, g, b);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let warm = p.background == Background::Warm;
    let (ink, muted, paper, surface, card, border, hover, pressed) = if dark {
        (rgb(238,236,231), rgb(184,181,173), if warm { rgb(30,29,27) } else { rgb(27,28,30) }, if warm { rgb(43,42,39) } else { rgb(39,40,43) }, if warm { rgb(36,35,33) } else { rgb(32,33,35) }, if warm { rgb(83,80,74) } else { rgb(66,68,72) }, if warm { rgb(57,55,51) } else { rgb(48,50,54) }, if warm { rgb(69,66,60) } else { rgb(58,60,64) })
    } else {
        (rgb(40,37,33), rgb(100,94,84), if warm { rgb(242,230,205) } else { rgb(255,255,255) }, if warm { rgb(235,220,191) } else { rgb(245,245,246) }, if warm { rgb(255,250,239) } else { rgb(255,255,255) }, if warm { rgb(190,171,139) } else { rgb(224,225,228) }, if warm { rgb(229,213,183) } else { rgb(237,238,240) }, if warm { rgb(216,197,162) } else { rgb(226,228,231) })
    };
    let (accent, link_dark) = match p.accent {
        Accent::Red => (rgb(174,50,36), rgb(255,156,137)),
        Accent::Blue => (rgb(33,91,165), rgb(139,190,255)),
        Accent::Green => (rgb(37,108,67), rgb(135,214,162)),
        Accent::Yellow => (rgb(127,91,0), rgb(239,203,101)),
        Accent::Cyan => (rgb(0,105,119), rgb(115,213,226)),
        Accent::Purple => (rgb(113,64,159), rgb(203,166,244)),
        Accent::Gray => (rgb(87,91,97), rgb(195,198,204)),
    };
    if p.theme == Theme::NativeLight {
        v = egui::Visuals::light();
        if p.background == Background::Warm {
            v.panel_fill = paper;
            v.window_fill = card;
            v.extreme_bg_color = card;
            v.faint_bg_color = surface;
        }
        if p.accent != Accent::Blue {
            v.selection.bg_fill = accent;
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            v.hyperlink_color = accent;
        }
        return v;
    }
    v.override_text_color = None;
    v.weak_text_color = Some(muted);
    v.panel_fill = paper;
    v.window_fill = card;
    v.faint_bg_color = surface;
    v.extreme_bg_color = card;
    v.hyperlink_color = if dark { link_dark } else { accent };
    v.selection.bg_fill = accent;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.error_fg_color = if dark { rgb(255,156,137) } else { rgb(174,50,36) };
    v.warn_fg_color = if dark { rgb(239,203,101) } else { rgb(127,91,0) };
    for widget in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        widget.bg_stroke = Stroke::new(1.0, border);
        widget.fg_stroke.color = ink;
        widget.corner_radius = egui::CornerRadius::same(tokens::RADIUS);
    }
    v.widgets.noninteractive.bg_fill = card;
    v.widgets.noninteractive.fg_stroke.color = ink;
    for (widget, fill) in [(&mut v.widgets.inactive, surface), (&mut v.widgets.hovered, hover), (&mut v.widgets.active, pressed), (&mut v.widgets.open, hover)] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
    }
    v
}

pub(crate) fn success_color(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode { Color32::from_rgb(135, 214, 162) } else { Color32::from_rgb(37, 108, 67) }
}

pub(crate) fn capability_color(visuals: &egui::Visuals, kind: u8) -> Color32 {
    match kind {
        0 => success_color(visuals),
        1 => if visuals.dark_mode { Color32::from_rgb(139, 190, 255) } else { Color32::from_rgb(33, 91, 165) },
        2 => visuals.warn_fg_color,
        _ => visuals.hyperlink_color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let channel = |v: u8| { let v = f32::from(v) / 255.0; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn all_appearance_combinations_preserve_text_contrast() {
        for theme in [Theme::Light, Theme::Dark, Theme::System] {
            for accent in [Accent::Red, Accent::Blue, Accent::Green, Accent::Yellow, Accent::Cyan, Accent::Purple, Accent::Gray] {
                for background in [Background::Warm, Background::White] {
                    for density in [Density::Comfortable, Density::Compact] {
                        let p = Preferences { theme, accent, background, density };
                        for dark in [false, true] {
                            let v = visuals(p, dark);
                            let mut pairs = vec![(v.selection.stroke.color, v.selection.bg_fill), (v.hyperlink_color, v.panel_fill), (v.weak_text_color.unwrap(), v.panel_fill), (v.error_fg_color, v.panel_fill), (v.warn_fg_color, v.panel_fill), (success_color(&v), v.panel_fill)];
                            pairs.extend((0..4).map(|kind| (capability_color(&v, kind), v.panel_fill)));
                            for widget in [v.widgets.noninteractive, v.widgets.inactive, v.widgets.hovered, v.widgets.active, v.widgets.open] {
                                pairs.push((widget.fg_stroke.color, widget.bg_fill));
                                pairs.push((widget.fg_stroke.color, v.panel_fill));
                            }
                            for (fg, bg) in pairs { assert!(contrast(fg, bg) >= 4.5, "{p:?}, dark={dark}, {fg:?} on {bg:?}"); }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn native_theme_keeps_preferences_effective() {
        let p = Preferences { theme: Theme::NativeLight, accent: Accent::Blue, background: Background::White, density: Density::Comfortable };
        let base = styles(p)[0].clone();
        let changed = styles(Preferences { accent: Accent::Purple, background: Background::Warm, density: Density::Compact, ..p })[0].clone();
        assert_ne!(base.visuals.selection.bg_fill, changed.visuals.selection.bg_fill);
        assert_ne!(base.visuals.panel_fill, changed.visuals.panel_fill);
        assert!(changed.spacing.item_spacing.y < base.spacing.item_spacing.y);
        assert!(!changed.visuals.dark_mode);
    }
    #[test]
    fn persistence_failure_restart_and_corruption_preserve_preferences() {
        let root = std::env::temp_dir().join(format!("lumadesk-appearance-{}", lumakit_support::new_id().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let mut appearance = Appearance::load(&root).unwrap();
        let next = Preferences { theme: Theme::NativeLight, accent: Accent::Purple, background: Background::White, density: Density::Compact };
        appearance.save(next).unwrap();
        assert_eq!(Appearance::load(&root).unwrap().saved, next);
        let path = root.join("settings.db");
        let bytes = std::fs::read(&path).unwrap();
        let real_root = appearance.root.clone();
        appearance.root = path.clone();
        assert!(appearance.save(Preferences::default()).is_err());
        assert_eq!(appearance.saved, next);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        appearance.root = real_root;
        std::fs::write(&path, b"broken").unwrap();
        assert!(Appearance::load(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_import_is_once_and_invalid_input_is_preserved() {
        let root = std::env::temp_dir().join(format!("lumadesk-appearance-import-{}", lumakit_support::new_id().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("appearance.json");
        std::fs::write(&path, b"invalid").unwrap();
        assert!(Appearance::load(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        let connection = rusqlite::Connection::open(root.join("settings.db")).unwrap();
        assert_eq!(connection.query_row("SELECT count(*) FROM settings_import_markers", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        let old = Preferences { theme: Theme::Dark, accent: Accent::Cyan, ..Default::default() };
        lumakit_support::save_versioned(&path, 1, &old).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut appearance = Appearance::load(&root).unwrap();
        assert_eq!(appearance.saved, old);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        appearance.save(Preferences::default()).unwrap();
        std::fs::write(&path, b"invalid but no longer a source").unwrap();
        assert_eq!(Appearance::load(&root).unwrap().saved, Preferences::default());
        connection.execute("DELETE FROM prefs WHERE key='appearance'", []).unwrap();
        assert!(Appearance::load(&root).is_err());
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn system_theme_tracks_input_and_defaults_to_light() {
        let root = std::env::temp_dir().join(format!("lumadesk-system-{}", lumakit_support::new_id().unwrap()));
        let mut appearance = Appearance::load(&root).unwrap();
        appearance.saved.theme = Theme::System;
        let ctx = egui::Context::default();
        for system_theme in [Some(egui::Theme::Dark), Some(egui::Theme::Light), None] {
            let mut output = ctx.run_ui(egui::RawInput { system_theme, ..Default::default() }, |ui| {
                appearance.apply(ui.ctx());
                assert_eq!(ui.ctx().global_style().visuals.dark_mode, system_theme == Some(egui::Theme::Dark));
            });
            output.textures_delta.clear();
        }
    }
}
