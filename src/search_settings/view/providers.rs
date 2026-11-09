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
            fields = fields.child(self.key_section(index, cx));
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
                    .suffix(
                        Button::new(("copy-search-address", index))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Copy)
                            .accessibility_label(format!("复制基础地址 · {}", provider.name()))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(inputs) = &this.inputs {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        inputs.providers[index]
                                            .endpoint
                                            .read(cx)
                                            .value()
                                            .to_string(),
                                    ));
                                }
                            })),
                    )
                    .disabled(busy),
                cx,
            ));
        } else {
            let check = Button::new(("check-direct", index))
                .outline()
                .label("检测")
                .accessibility_label("检测 · 直接获取")
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.check_provider(index, cx);
                    cx.notify();
                }));
            fields = fields
  .child(
      label(
   text::METADATA,
   "无需配置密钥或基础地址。下方输入网址读取，检测将请求 https://example.com。",
      )
      .text_color(r.text_muted),
         )
         .child(h_flex().child(check));
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit_card(index, cx);
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
