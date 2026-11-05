//! Encrypted credential manager view and explicit reveal actions.
use super::*;
impl SearchSettings {
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
        let input = &inputs.providers[index];
        let mut list = v_flex()
            .id("search-key-list")
            .max_h(px(f32::from(window.viewport_size().height) * 0.35))
            .overflow_y_scroll()
            .gap(space::S12);
        for (i, key) in config.keys.iter().enumerate() {
            let id = key.id.clone();
            let edit_id = id.clone();
            let reveal_id = id.clone();
            let copy_id = id.clone();
            let shown = self.drafts[index]
                .revealed
                .as_ref()
                .filter(|(key_id, _)| key_id == &id)
                .map(|(_, value)| value.as_str())
                .unwrap_or(&key.masked);
            let action = |name: &'static str| {
                Button::new((name, i))
                    .small()
                    .ghost()
                    .label(name)
                    .disabled(busy)
            };
            let row = v_flex()
                .gap(space::S4)
                .pb(space::S8)
                .border_b_1()
                .border_color(r.border_muted)
                .child(label(text::CODE_TEXT, shown.to_owned()))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(space::S4)
                        .child(
                            action("修改").on_click(cx.listener(move |this, _, window, cx| {
                                this.clear_provider(index, window, cx);
                                this.drafts[index].editing = Some(edit_id.clone());
                                cx.notify();
                            })),
                        )
                        .child(action("显示").on_click(cx.listener(move |this, _, _, cx| {
                            this.reveal_key(index, &reveal_id, false, cx);
                            cx.notify();
                        })))
                        .child(action("复制").on_click(cx.listener(move |this, _, _, cx| {
                            this.reveal_key(index, &copy_id, true, cx);
                            cx.notify();
                        })))
                        .child(action("删除").on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_key = Some((index, id.clone()));
                            cx.notify();
                        }))),
                );
            list = list.child(row);
        }
        let editing = self.drafts[index].editing.clone();
        let mut body = v_flex()
            .gap(space::S12)
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(
                        label(
                            text::TITLE_DIALOG,
                            format!("管理密钥 · {}", config.provider.name()),
                        )
                        .flex_1(),
                    )
                    .child(
                        Button::new("close-search-keys")
                            .small()
                            .ghost()
                            .icon(IconName::X)
                            .accessibility_label("关闭密钥管理")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_keys(window, cx)),
                            ),
                    ),
            )
            .child(list)
            .child(field(
                if editing.is_some() {
                    "修改密钥"
                } else {
                    "新密钥"
                },
                Input::new(&input.edit)
                    .aria_label("密钥管理输入")
                    .disabled(busy),
                cx,
            ));
        if let Some((_, id)) = self
            .delete_key
            .as_ref()
            .filter(|(provider, _)| *provider == index)
        {
            let id = id.clone();
            body = body.child(
                h_flex()
                    .flex_wrap()
                    .gap(space::S8)
                    .child(label(text::UI, "确认删除此密钥？").text_color(r.feedback_error))
                    .child(
                        Button::new("cancel-search-key-delete")
                            .small()
                            .ghost()
                            .label("取消删除")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.delete_key = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("confirm-search-key-delete")
                            .small()
                            .danger()
                            .label("确认删除")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.submit(
                                    index,
                                    Some(KeyAction::Delete(id.clone())),
                                    window,
                                    cx,
                                ) {
                                    this.clear_provider(index, window, cx);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        if !self.card_status[index].is_empty() {
            body = body.child(
                label(text::METADATA, self.card_status[index].clone()).text_color(
                    if self.card_error[index] {
                        r.feedback_error
                    } else {
                        r.text_muted
                    },
                ),
            );
        }
        body = body.child(
            h_flex()
                .justify_end()
                .gap(space::S8)
                .child(
                    Button::new("save-managed-search-key")
                        .small()
                        .primary()
                        .label(if editing.is_some() {
                            "保存"
                        } else {
                            "添加密钥"
                        })
                        .disabled(
                            busy || input.edit.read(cx).value().trim().is_empty()
                                || (editing.is_none() && config.keys.len() >= 16),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let Some(inputs) = &this.inputs else {
                                return;
                            };
                            let value = inputs.providers[index].edit.read(cx).value().to_string();
                            let action = match &editing {
                                Some(id) => KeyAction::Replace(id.clone(), value),
                                None => {
                                    this.drafts[index].key.zeroize();
                                    this.drafts[index].key = value;
                                    KeyAction::Add
                                }
                            };
                            if this.submit(index, Some(action), window, cx) {
                                this.clear_provider(index, window, cx);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("cancel-search-keys")
                        .small()
                        .outline()
                        .label("取消")
                        .on_click(cx.listener(|this, _, window, cx| this.close_keys(window, cx))),
                ),
        );
        let viewport = window.viewport_size();
        let panel = card()
            .w(px((f32::from(viewport.width) - 32.).max(100.)).min(DIALOG_WIDTH))
            .max_h(px((f32::from(viewport.height) - 32.).max(100.)))
            .border_1()
            .border_color(r.border_default)
            .bg(r.container_elevated_bg)
            .shadow_lg()
            .id("search-key-dialog")
            .overflow_y_scroll()
            .track_focus(&inputs.focus)
            .child(body)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_keys(window, cx);
                        cx.stop_propagation();
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
    fn reveal_key(&mut self, index: usize, id: &str, copy: bool, cx: &mut Context<Self>) {
        let result = self.stored.configs[index]
            .keys
            .iter()
            .find(|key| key.id == id)
            .ok_or("密钥不存在".to_owned())
            .and_then(SavedKey::open);
        match result {
            Ok(value) => {
                if copy {
                    cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
                } else {
                    if let Some((_, mut old)) = self.drafts[index].revealed.take() {
                        old.zeroize();
                    }
                    self.drafts[index].revealed = Some((id.into(), value.to_string()));
                }
            }
            Err(error) => {
                self.card_status[index] = error;
                self.card_error[index] = true;
            }
        }
    }
}
