//! Search policy editor and manual query/read tools.
use super::*;
impl SearchSettings {
    pub(super) fn advanced_view(&self, cx: &Context<Self>) -> AnyElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let busy = self.is_busy();
        let Some(inputs) = &self.inputs else {
            return div().into_any_element();
        };
        let mut section = card().border_1().border_color(r.border_muted).child(
            h_flex().child(
                Button::new("advanced-search")
                    .small()
                    .ghost()
                    .label("高级设置")
                    .icon(if self.advanced {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            ),
        );
        if self.advanced {
            let page = cx.entity();
            let compression = ui::select(
                ui::select_button(
                    "search-compression",
                    "正文压缩",
                    if self.compression == Compression::None {
                        "不截断"
                    } else {
                        "截断"
                    },
                )
                .disabled(busy),
                Some(self.compression),
                [
                    (Compression::None, "不截断".into()),
                    (Compression::Cutoff, "截断".into()),
                ],
                move |value, _, cx| {
                    page.update(cx, |this, cx| {
                        this.compression = value;
                        cx.notify();
                    })
                },
            );
            section = section
                .child(
                    h_flex()
                        .gap(space::S12)
                        .child(label(text::UI, "结果数").flex_1())
                        .child(
                            Input::new(&inputs.max_results)
                                .w(px(80.))
                                .aria_label("搜索结果数")
                                .disabled(busy),
                        ),
                )
                .child(
                    h_flex()
                        .gap(space::S12)
                        .child(label(text::UI, "正文压缩").flex_1())
                        .child(compression),
                );
            if self.compression == Compression::Cutoff {
                section = section.child(
                    h_flex()
                        .gap(space::S12)
                        .child(label(text::UI, "截断长度").flex_1())
                        .child(
                            Input::new(&inputs.cutoff)
                                .w(px(96.))
                                .aria_label("搜索截断长度")
                                .disabled(busy),
                        ),
                );
            }
            section = section
                .child(field(
                    "黑名单",
                    Textarea::new(&inputs.blacklist)
                        .aria_label("搜索黑名单")
                        .h(BLACKLIST_HEIGHT)
                        .disabled(busy),
                    cx,
                ))
                .child(
                    h_flex().child(
                        Button::new("save-search-preferences")
                            .small()
                            .primary()
                            .label("保存高级设置")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_preferences(cx);
                                cx.notify();
                            })),
                    ),
                );
        }
        section.into_any_element()
    }
    pub(super) fn tools_view(&self, cx: &Context<Self>) -> AnyElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let busy = self.is_busy();
        let Some(inputs) = &self.inputs else {
            return div().into_any_element();
        };
        let mut body = card().border_1().border_color(r.border_muted);
        for (fetch, name, input) in [
            (false, "搜索词", &inputs.query),
            (true, "网页地址", &inputs.target),
        ] {
            body = body.child(field(
                name,
                h_flex()
                    .flex_nowrap()
                    .gap(space::S8)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(input).aria_label(name).disabled(busy)),
                    )
                    .child(
                        Button::new(if fetch { "run-fetch" } else { "run-search" })
                            .small()
                            .primary()
                            .label(if fetch { "读取网页" } else { "搜索" })
                            .disabled(busy || input.read(cx).value().trim().is_empty())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_request(fetch, window, cx)
                            })),
                    ),
                cx,
            ));
        }
        if !self.status.is_empty() {
            body = body.child(label(text::METADATA, self.status.clone()).text_color(r.text_muted));
        }
        if busy {
            body = body.child(
                Button::new("cancel-search-request")
                    .small()
                    .ghost()
                    .label("取消请求")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(job) = &this.job {
                            job.cancel.cancel();
                        }
                        cx.notify();
                    })),
            );
        }
        if let Some(result) = &self.result {
            for (index, hit) in result.hits.iter().enumerate() {
                let url = hit.url.clone();
                body = body.child(
                    v_flex()
                        .gap(space::S4)
                        .child(
                            Button::new(("search-hit", index))
                                .small()
                                .link()
                                .label(hit.title.clone())
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                        .child(label(text::UI, hit.snippet.clone())),
                );
            }
        }
        if let Some(page) = &self.page {
            let url = page.url.clone();
            body = body
                .child(label(text::TITLE_SECTION, page.title.clone()))
                .child(
                    Button::new("search-page-url")
                        .small()
                        .link()
                        .label(page.url.clone())
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(
                    div()
                        .id("search-page-result")
                        .max_h(RESULT_HEIGHT)
                        .overflow_y_scroll()
                        .child(label(text::READING, page.body.clone())),
                );
        }
        body.into_any_element()
    }
}
