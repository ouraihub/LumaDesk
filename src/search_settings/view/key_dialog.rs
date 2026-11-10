//! Credential management overlay; shared components own rows, entry and compact form layout.
use super::*;
use lumakit_gpui::providers as shared;

impl SearchSettings {
    /// The key manager, or the delete confirmation while one is pending, over the page.
    pub(super) fn key_dialog_view(
        &self,
        index: usize,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let busy = self.is_busy();
        let Some(inputs) = &self.inputs else {
            return div().into_any_element();
        };
        let config = &self.stored.configs[index];
        let status = (!self.card_status[index].is_empty()).then(|| {
            v_flex()
                .min_w_0()
                .gap(space::S4)
                .child(
                    label(text::METADATA, self.card_status[index].clone())
                        .whitespace_normal()
                        .text_color(if self.card_error[index] {
                            r.feedback_error
                        } else {
                            r.text_muted
                        }),
                )
                .children(
                    self.card_diagnostic_trace[index]
                        .clone()
                        .filter(|_| self.card_error[index])
                        .map(|trace| {
                            Button::new("copy-search-key-diagnostic")
                                .small()
                                .link()
                                .label("复制诊断 ID")
                                .accessibility_label(format!("复制诊断 ID {trace}"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(trace.clone()));
                                })
                        }),
                )
                .into_any_element()
        });
        let body = if let Some((_, id)) = self.delete_key.as_ref().filter(|(i, _)| *i == index) {
            let id = id.clone();
            v_flex()
                .gap(space::S12)
                .child(
                    label(
                        text::METADATA,
                        "将删除此密钥；其它密钥和服务商设置保持不变。",
                    )
                    .whitespace_normal()
                    .text_color(r.text_muted),
                )
                .child(
                    h_flex()
                        .gap(space::S8)
                        .flex_wrap()
                        .child(
                            Button::new("cancel-search-key-delete")
                                .outline()
                                .label("取消")
                                .disabled(busy)
                                .on_click(Self::listen(cx, |this, _, _| this.delete_key = None)),
                        )
                        .child(
                            Button::new("confirm-search-key-delete")
                                .danger()
                                .label("删除")
                                .disabled(busy)
                                .on_click(Self::listen(cx, move |this, window, cx| {
                                    this.submit_key(
                                        index,
                                        KeyAction::Delete(id.clone()),
                                        window,
                                        cx,
                                    );
                                })),
                        ),
                )
                .into_any_element()
        } else {
            let editing = self.drafts[index].editing.is_some();
            let save = Button::new(("save-search-key", index))
                .primary()
                .label(if editing {
                    "保存"
                } else {
                    "添加 API 密钥"
                })
                .disabled(busy || (!editing && config.keys.len() >= 16))
                .on_click(Self::listen(cx, move |this, window, cx| {
                    this.save_key_input(index, window, cx)
                }));
            let cancel = editing.then(|| {
                Button::new(("cancel-search-key-edit", index))
                    .outline()
                    .label("取消修改")
                    .disabled(busy)
                    .on_click(Self::listen(cx, move |this, window, cx| {
                        this.clear_provider(index, window, cx)
                    }))
                    .into_any_element()
            });
            shared::key_manager(
                config
                    .keys
                    .iter()
                    .map(|key| self.key_row(index, key, true, busy, cx).into_any_element()),
                std::iter::empty(),
                v_flex()
                    .min_w_0()
                    .gap(space::S8)
                    .child(shared::key_entry(
                        Input::new(&inputs.providers[index].label)
                            .aria_label("密钥标签（可选）")
                            .disabled(busy)
                            .into_any_element(),
                        Input::new(&inputs.providers[index].key)
                            .aria_label(if config.provider == Provider::Searxng {
                                "密码（修改时留空保留原密码）"
                            } else {
                                "API 密钥（修改时留空保留原密钥）"
                            })
                            .mask_toggle()
                            .disabled(busy)
                            .into_any_element(),
                        h_flex()
                            .gap(space::S8)
                            .flex_wrap()
                            .child(save)
                            .children(cancel)
                            .into_any_element(),
                    ))
                    .child(
                        label(
                            text::METADATA,
                            format!(
                                "{} / {} 已启用",
                                config.keys.iter().filter(|key| key.enabled).count(),
                                config.keys.len(),
                            ),
                        )
                        .text_color(r.text_subtle),
                    )
                    .into_any_element(),
                label(
                    text::METADATA,
                    "每个服务商最多 16 个密钥；修改时留空保留原密钥。",
                )
                .whitespace_normal()
                .text_color(r.text_subtle)
                .into_any_element(),
            )
            .into_any_element()
        };
        let close = Button::new("close-search-keys")
            .outline()
            .label("关闭")
            .on_click(Self::listen(cx, |this, window, cx| {
                this.close_keys(window, cx)
            }))
            .into_any_element();
        let content = shared::compact_form_body(
            "API 密钥管理",
            body,
            shared::form_footer(None, close, None).into_any_element(),
            status,
            &r,
        );
        let viewport = window.viewport_size();
        let panel = v_flex()
            .min_w_0()
            .p(space::S12)
            .rounded(radius::LG)
            .w(px((f32::from(viewport.width) - 32.).max(100.)).min(size::SETTINGS_DIALOG))
            .max_h(px((f32::from(viewport.height) - 32.).max(100.)))
            .border_1()
            .border_color(r.border_default)
            .bg(r.container_elevated_bg)
            .shadow_lg()
            .id("search-key-dialog")
            .overflow_y_scroll()
            .track_focus(&inputs.focus)
            .child(content)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        if this.delete_key.take().is_none() {
                            this.close_keys(window, cx);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .focus_trap("search-key-trap", &inputs.focus);
        gpui_kit::deferred(
            gpui_kit::anchored()
                .position(gpui_kit::point(px(0.), px(0.)))
                .anchor(gpui_kit::Anchor::TopLeft)
                .child(
                    div()
                        .w(viewport.width)
                        .h(viewport.height)
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p(space::S16)
                        .bg(r.overlay)
                        .child(panel),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
    fn close_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.key_dialog.take() {
            self.clear_provider(index, window, cx);
        }
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
}
