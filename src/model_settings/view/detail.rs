//! Provider header and saved route controls; shared components own their layout.
use super::*;

impl ModelSettings {
    pub(super) fn detail_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let Some((p, e)) = self.indices() else {
            return div().child(shared::empty("选择或新建一个服务商，开始配置模型。", &r));
        };
        let header = self.detail_header(p, cx).into_any_element();
        let Some(e) = e else {
            let add = self
                .button(
                    "new-endpoint",
                    "添加线路",
                    cx,
                    move |this, window, cx| {
                        this.edit_endpoint(p, None, window, cx);
                    },
                )
                .link()
                .disabled(self.job.is_some())
                .into_any_element();
            return shared::detail(
                header,
                None,
                div().into_any_element(),
                div().into_any_element(),
                shared::route_section(
                    Some(add),
                    None,
                    shared::empty("尚未配置线路", &r),
                    div().into_any_element(),
                    None,
                    None,
                    &r,
                )
                .into_any_element(),
                div().into_any_element(),
            );
        };
        shared::detail(
            header,
            None,
            self.key_section(p, e, cx).into_any_element(),
            shared::strategy(
                "失败时轮换",
                "仅在 HTTP 401/403/429 时换下一个已保存密钥，不跨服务商重试",
                &r,
            )
            .into_any_element(),
            self.address_section(p, e, cx).into_any_element(),
            self.models_view(p, e, cx).into_any_element(),
        )
    }

    fn detail_header(&self, p: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let provider = &self.draft.providers[p];
        let settings = self
            .icon(
                "edit-provider",
                IconName::Settings2,
                "服务商设置",
                cx,
                move |this, window, cx| {
                    let p = &this.draft.providers[p];
                    this.open_editor(
                        EditorDialog::Provider {
                            id: Some(p.id.clone()),
                            name: p.name.clone(),
                        },
                        window,
                        cx,
                    );
                },
            )
            .disabled(self.job.is_some());
        let enabled = Switch::new("enabled")
            .checked(provider.enabled)
            .disabled(self.job.is_some())
            .accessibility_label(format!("启用 {}", provider.name))
            .on_click(cx.listener(move |this, value: &bool, _, cx| {
                if this.job.is_none() {
                    this.set_enabled(p, *value);
                }
                cx.notify();
            }));
        shared::detail_header(
            provider.name.clone(),
            settings.into_any_element(),
            enabled.into_any_element(),
        )
    }

    fn address_section(&self, p: usize, e: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let provider = &self.draft.providers[p];
        let endpoint = &provider.endpoints[e];
        let busy = self.job.is_some();
        let add = self
            .button(
                "new-endpoint",
                "添加线路",
                cx,
                move |this, window, cx| {
                    this.edit_endpoint(p, None, window, cx);
                },
            )
            .link()
            .disabled(busy)
            .into_any_element();
        let picker = (provider.endpoints.len() > 1).then(|| {
            let page = cx.entity();
            let lines: Vec<_> = provider
                .endpoints
                .iter()
                .map(|l| (l.id.clone(), l.name.clone()))
                .collect();
            let current = endpoint.id.clone();
            Button::new("provider-line")
                .outline()
                .w_full()
                .disabled(busy)
                .label(endpoint.name.clone())
                .icon(IconName::ChevronDown)
                .dropdown_menu(move |menu, _, _| {
                    lines.iter().fold(menu, |menu, (id, name)| {
                        let (page, id) = (page.clone(), id.clone());
                        menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(id == current)
                                .on_click(move |_, window, cx| {
                                    page.update(cx, |this, cx| {
                                        if this.job.is_some() {
                                            return;
                                        }
                                        if this.dirty {
                                            this.report(
                                                Err("当前地址尚未保存，请先修正或重试保存".into()),
                                                "",
                                            );
                                        } else {
                                            this.endpoint = Some(id.clone());
                                            this.refresh_endpoint(window, cx);
                                        }
                                        cx.notify();
                                    })
                                }),
                        )
                    })
                })
                .into_any_element()
        });
        let copy = self.icon(
            "copy-url",
            IconName::Copy,
            "复制 API 地址",
            cx,
            move |this, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    this.draft.providers[p].endpoints[e].base_url.clone(),
                ));
                this.notice = "已复制 API 地址".into();
            },
        );
        let settings = self.on(
            Button::new("edit-endpoint")
                .outline()
                .icon(IconName::Settings2)
                .disabled(busy)
                .accessibility_label("线路设置")
                .tooltip("线路设置"),
            cx,
            move |this, window, cx| this.edit_endpoint(p, Some(e), window, cx),
        );
        let retry = self.dirty.then(|| {
            self.button("save-url", "保存地址", cx, |this, _, _| this.commit())
                .primary()
                .disabled(busy)
                .into_any_element()
        });
        let request = ui::request_line(
            match model_url(&endpoint.base_url, endpoint.protocol) {
                Ok(url) => format!("{} · GET {url}", protocol_name(endpoint.protocol)),
                Err(error) => error,
            },
            &r,
        )
        .into_any_element();
        shared::route_section(
            Some(add),
            picker,
            self.input("url")
                .disabled(busy)
                .suffix(copy)
                .flex_1()
                .into_any_element(),
            settings.into_any_element(),
            retry,
            Some(request),
            &r,
        )
    }
}
