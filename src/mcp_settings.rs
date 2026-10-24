//! MCP 配置的最小宿主设置页：保存安全边界，不在未接入 AiHost 前暴露工具入口。
use eframe::egui::{self, Ui};
use lumakit_ai::mcp::McpServerConfig;
use std::path::{Path, PathBuf};

pub struct McpSettings {
    root: PathBuf,
    servers: Vec<McpServerConfig>,
    url: String,
    enabled: bool,
    trusted: bool,
    status: String,
}

impl McpSettings {
    pub fn new(root: &Path) -> Result<Self, String> {
        let servers = crate::settings_store::load_mcp(root)?.map(|data| data.servers).unwrap_or_default();
        Ok(Self { root: root.to_owned(), servers, url: String::new(), enabled: false, trusted: false, status: String::new() })
    }

    pub fn clear_secrets(&mut self) {}

    pub fn ui(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("MCP 工具服务器");
            ui.label("仅保存 HTTPS 或回环 HTTP 地址；启用且信任后才允许后续宿主接入工具。");
            ui.add(egui::TextEdit::singleline(&mut self.url).hint_text("例如 https://example.com/mcp").desired_width(ui.available_width()));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.enabled, "启用");
                ui.checkbox(&mut self.trusted, "信任");
                if ui.button("添加服务器").clicked() { self.add(); }
            });
            if !self.status.is_empty() { ui.label(&self.status); }
            ui.separator();
            let mut remove = false;
            for index in 0..self.servers.len() {
                let server = &self.servers[index];
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&server.url);
                        ui.label(if server.enabled && server.trusted { "已启用 · 已信任" } else if server.enabled { "已启用 · 未信任" } else { "已停用" });
                        if ui.button("删除").clicked() { remove = true; }
                    });
                });
                if remove { self.servers.remove(index); let _ = self.save(); break; }
            }
            ui.weak("工具暴露、Skills 和审批入口将在 AiHost 接入后显示；当前页面只保存服务器信任配置。");
        });
    }

    fn add(&mut self) {
        let config = McpServerConfig { url: self.url.trim().to_owned(), enabled: self.enabled, trusted: self.trusted };
        if let Err(error) = config.validate_url() { self.status = error.to_string(); return; }
        if self.servers.iter().any(|server| server.url == config.url) { self.status = "服务器已存在".into(); return; }
        self.servers.push(config);
        if self.save().is_ok() { self.url.clear(); self.status = "已保存".into(); }
    }

    fn save(&mut self) -> Result<(), String> {
        crate::settings_store::save_mcp(&self.root, &crate::settings_store::McpData { servers: self.servers.clone() })
    }
}
