//! Search and fetch provider configuration cards.
use super::*;
impl SearchSettings {
    pub(super) fn provider_ui(&self, fetch: bool, cx: &Context<Self>) -> AnyElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let busy = self.is_busy();
        let index = if fetch {
            self.fetch_index
        } else {
            self.search_index
        };
        let config = &self.stored.configs[index];
        let provider = config.provider;
        let Some(inputs) = &self.inputs else {
            return div().into_any_element();
        };
        let input = &inputs.providers[index];
        let page = cx.entity();
        let options: Vec<_> = (if fetch {
            FETCH_PROVIDERS.as_slice()
        } else {
            SEARCH_PROVIDERS.as_slice()
        })
        .iter()
        .filter_map(|p| {
            self.stored
                .configs
                .iter()
                .position(|config| config.provider == *p)
                .map(|i| (i, p.name().into()))
        })
        .collect();
        let select = ui::select(
            ui::select_button(
                ("search-provider", index),
                if fetch {
                    "读取网页服务商"
                } else {
                    "网页搜索服务商"
                },
                provider.name(),
            )
            .w(PROVIDER_WIDTH)
            .disabled(busy),
            Some(index),
            options,
            move |index, window, cx| {
                page.update(cx, |this, cx| {
                    this.choose_provider(fetch, index, window, cx)
                })
            },
        );
        let website = Button::new(("provider-site", index))
            .small()
            .link()
            .icon(IconName::ExternalLink)
            .accessibility_label(format!("{} 官方网站", provider.name()))
            .tooltip("官方网站")
            .on_click(move |_, _, cx| cx.open_url(provider.website()));
        let header = h_flex()
            .items_start()
            .flex_wrap()
            .gap(space::S12)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .flex_basis(px(240.))
                    .gap(space::S4)
                    .child(label(
                        text::TITLE_SECTION,
                        if fetch {
                            "读取网页"
                        } else {
                            "网页搜索"
                        },
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap(space::S4)
                            .items_start()
                            .child(
                                div().flex_1().min_w_0().child(
                                    label(text::UI, provider.description())
                                        .whitespace_normal()
                                        .text_color(r.text_muted),
                                ),
                            )
                            .child(website),
                    ),
            )
            .child(select);
        let mut fields = v_flex()
            .min_w_0()
            .gap(space::S12)
            .border_t_1()
            .border_color(r.border_muted)
            .pt(space::S12);
        if provider != Provider::Direct {
            let mut heading = h_flex().gap(space::S8).child(label(
                text::UI,
                if provider == Provider::Searxng {
                    "密码（可选）"
                } else {
                    "API 密钥"
                },
            ));
            if let Some(url) = provider.key_site() {
                heading = heading.child(
                    Button::new(("get-key", index))
                        .small()
                        .link()
                        .label("获取密钥")
                        .on_click(move |_, _, cx| cx.open_url(url)),
                );
            }
            let masked = if config.keys.is_empty() {
                "未配置".to_owned()
            } else {
                config
                    .keys
                    .iter()
                    .map(|key| key.masked.as_str())
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            let keys = icon_button(
                ("manage-search-keys", index),
                IconName::List,
                format!("管理密钥 · {}", provider.name()),
                busy,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.previous_focus = window.focused(cx);
                this.clear_provider(index, window, cx);
                this.key_dialog = Some(index);
                if let Some(inputs) = &this.inputs {
                    inputs.focus.focus(window, cx);
                }
                cx.notify();
            }));
            let check = icon_button(
                ("check-search", index),
                IconName::Activity,
                format!("检测 · {}", provider.name()),
                busy,
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.check_provider(index, window, cx)),
            );
            fields = fields.child(
                v_flex()
                    .gap(space::S6)
                    .child(heading)
                    .child(
                        h_flex()
                            .flex_nowrap()
                            .gap(space::S8)
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&input.key)
                                        .aria_label(format!("{} 新密钥", provider.name()))
                                        .disabled(busy),
                                ),
                            )
                            .child(keys)
                            .child(check),
                    )
                    .child(label(text::METADATA, masked).text_color(r.text_muted)),
            );
        }
        if provider == Provider::Searxng {
            fields = fields.child(field(
                "用户名（可选）",
                Input::new(&input.username)
                    .aria_label("SearXNG 用户名")
                    .disabled(busy),
                cx,
            ));
        }
        if provider != Provider::Direct {
            fields = fields.child(field(
                "基础地址",
                Input::new(&input.endpoint)
                    .aria_label(format!("{} 基础地址", provider.name()))
                    .suffix(Button::new(("copy-search-address", index)).xsmall().ghost().icon(IconName::Copy)
                        .accessibility_label(format!("复制基础地址 · {}", provider.name())).disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(inputs) = &this.inputs { cx.write_to_clipboard(ClipboardItem::new_string(inputs.providers[index].endpoint.read(cx).value().to_string())); }
                        })))
                    .disabled(busy),
                cx,
            ));
        } else {
            fields = fields.child(label(text::METADATA, "无需配置密钥或基础地址。下方输入网址读取，检测将请求 https://example.com。").text_color(r.text_muted))
                .child(icon_button(("check-direct", index), IconName::Activity, "检测 · 直接获取", busy).on_click(cx.listener(move |this, _, window, cx| this.check_provider(index, window, cx))));
        }
        let mut footer = h_flex().flex_wrap().gap(space::S8);
        if provider != Provider::Direct {
            footer = footer.child(
                Button::new(("save-search-provider", index))
                    .small()
                    .primary()
                    .label("保存")
                    .accessibility_label(format!("保存 · {}", provider.name()))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.submit(index, None, window, cx);
                        cx.notify();
                    })),
            );
        }
        footer = footer.child(
            label(text::METADATA, self.card_status[index].clone()).text_color(
                if self.card_error[index] {
                    r.feedback_error
                } else {
                    r.text_muted
                },
            ),
        );
        if self
            .job
            .as_ref()
            .is_some_and(|job| job.check == Some(index))
        {
            footer = footer.child(
                Button::new(("cancel-check", index))
                    .small()
                    .ghost()
                    .label("取消检测")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(job) = &this.job {
                            job.cancel.cancel();
                        }
                        cx.notify();
                    })),
            );
        }
        card()
            .border_1()
            .border_color(r.border_muted)
            .child(header)
            .child(fields)
            .child(footer)
            .into_any_element()
    }
}
