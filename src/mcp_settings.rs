//! MCP 配置只保存服务器与信任边界，不暴露尚未接入的工具执行入口。
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    switch::Switch,
};
use gpui_kit::{
    AppContext, Context, Entity, Render, SharedString, Subscription, Window, div, prelude::*,
};
use lumakit_ai::mcp::McpServerConfig;
use lumakit_design::components::settings as ui;
use lumakit_design::components::{IconButton, label};
use lumakit_design::metrics::{size, space, text};
use std::path::{Path, PathBuf};

pub struct McpSettings {
    root: PathBuf,
    servers: Vec<McpServerConfig>,
    url: Entity<InputState>,
    enabled: bool,
    trusted: bool,
    status: String,
    _subscriptions: Vec<Subscription>,
}

impl McpSettings {
    pub fn create(
        root: &Path,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> Result<Entity<Self>, String> {
        let servers = crate::settings_store::load_mcp(root)?
            .map(|data| data.servers)
            .unwrap_or_default();
        Ok(cx.new(|cx| Self::with_servers(root, servers, window, cx)))
    }
    fn with_servers(
        root: &Path,
        servers: Vec<McpServerConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://example.com/mcp"));
        let subscription = cx.subscribe(&url, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            root: root.to_owned(),
            servers,
            url,
            enabled: false,
            trusted: false,
            status: String::new(),
            _subscriptions: vec![subscription],
        }
    }

    fn save(&mut self, next: Vec<McpServerConfig>) -> bool {
        match crate::settings_store::save_mcp(
            &self.root,
            &crate::settings_store::McpData {
                servers: next.clone(),
            },
        ) {
            Ok(()) => {
                self.servers = next;
                self.status = "已保存".into();
                true
            }
            Err(error) => {
                self.status = format!("保存失败：{error}；原配置保持不变，可重试");
                false
            }
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = McpServerConfig {
            url: self.url.read(cx).value().trim().to_owned(),
            enabled: self.enabled,
            trusted: self.trusted,
        };
        if let Err(error) = config.validate_url() {
            self.status = error.to_string();
            return;
        }
        if self.servers.iter().any(|server| server.url == config.url) {
            self.status = "服务器已存在".into();
            return;
        }
        let mut next = self.servers.clone();
        next.push(config);
        if self.save(next) {
            self.url
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
}

impl Render for McpSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let empty = self.url.read(cx).value().trim().is_empty();
        let add = ui::group("添加服务器", None, &r)
            .child(
                ui::field(
                    "地址",
                    v_flex().gap(space::S4).child(Input::new(&self.url)).child(
                        label(text::METADATA, "仅 HTTPS 或回环 HTTP。").text_color(r.text_subtle),
                    ),
                    &r,
                )
                .pt(space::S12),
            )
            .child(ui::row(
                "启用",
                None,
                Switch::new("mcp-enabled")
                    .small()
                    .checked(self.enabled)
                    .accessibility_label("新服务器启用")
                    .on_click(cx.listener(|this, value: &bool, _, cx| {
                        this.enabled = *value;
                        cx.notify();
                    })),
                &r,
            ))
            .child(ui::row(
                "信任",
                Some("信任后才允许后续宿主接入其工具。".into()),
                Switch::new("mcp-trusted")
                    .small()
                    .checked(self.trusted)
                    .accessibility_label("新服务器信任")
                    .on_click(cx.listener(|this, value: &bool, _, cx| {
                        this.trusted = *value;
                        cx.notify();
                    })),
                &r,
            ))
            .child(
                h_flex().justify_end().pt(space::S12).child(
                    Button::new("mcp-add")
                        .small()
                        .primary()
                        .icon(IconName::Plus)
                        .label("添加服务器")
                        .disabled(empty)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add(window, cx);
                            cx.notify();
                        })),
                ),
            );
        let mut saved = ui::group("已保存", None, &r);
        if self.servers.is_empty() {
            saved = saved.child(div().pt(space::S12).child(ui::empty("还没有服务器。", &r)));
        }
        for (index, server) in self.servers.iter().enumerate() {
            let state = match (server.enabled, server.trusted) {
                (true, true) => "已启用 · 已信任",
                (true, false) => "已启用 · 未信任",
                (false, true) => "已停用 · 已信任",
                (false, false) => "已停用 · 未信任",
            };
            let controls = h_flex()
                .gap(space::S12)
                .child(
                    Switch::new(("enabled", index))
                        .small()
                        .label("启用")
                        .checked(server.enabled)
                        .on_click(cx.listener(move |this, value: &bool, _, cx| {
                            let mut next = this.servers.clone();
                            next[index].enabled = *value;
                            this.save(next);
                            cx.notify();
                        })),
                )
                .child(
                    Switch::new(("trusted", index))
                        .small()
                        .label("信任")
                        .checked(server.trusted)
                        .on_click(cx.listener(move |this, value: &bool, _, cx| {
                            let mut next = this.servers.clone();
                            next[index].trusted = *value;
                            this.save(next);
                            cx.notify();
                        })),
                )
                .child(
                    IconButton::new(("remove", index), IconName::Trash)
                        .tooltip(format!("删除 {}", server.url))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut next = this.servers.clone();
                            next.remove(index);
                            this.save(next);
                            cx.notify();
                        })),
                );
            saved = saved.child(
                h_flex()
                    .gap(space::S16)
                    .py(space::S12)
                    .border_b_1()
                    .border_color(r.border_muted)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(space::S2)
                            .child(
                                label(text::CODE_TEXT, SharedString::from(server.url.clone()))
                                    .truncate(),
                            )
                            .child(label(text::METADATA, state).text_color(r.text_subtle)),
                    )
                    .child(div().flex_none().child(controls)),
            );
        }
        let mut body = v_flex().id("mcp-settings").w_full().max_w(size::CONTENT_MAX).gap(space::S24)
            .child(ui::page_header("MCP 工具服务器", Some("只保存服务器地址与信任边界；工具暴露、Skills 和审批入口将在 AiHost 接入后显示。".into()), &r))
            .child(add).child(saved);
        if !self.status.is_empty() {
            body = body.child(if self.status == "已保存" {
                label(text::METADATA, self.status.clone())
                    .text_color(r.text_subtle)
                    .into_any_element()
            } else {
                ui::notice(self.status.clone(), ui::Tone::Error, &r)
            });
        }
        body
    }
}
