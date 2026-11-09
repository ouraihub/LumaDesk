//! The provider card's API-key area and the key manager dialog, drawn with the shared
//! `lumakit_design` key controls exactly as the model page does; secrets stay in the drafts.
use super::*;
use gpui_kit::{App, ClickEvent};

impl SearchSettings {
    /// A click handler that runs `action` on this page and repaints.
    fn listen(
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _, window, cx| {
            action(this, window, cx);
            cx.notify();
        })
    }

    /// API 密钥 · 获取密钥; the preferred key (or the first-key input), key management and 检测.
    pub(super) fn key_section(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let busy = self.is_busy();
        let config = &self.stored.configs[index];
        let provider = config.provider;
        let Some(inputs) = &self.inputs else {
            return div().into_any_element();
        };
        let links: Vec<AnyElement> = provider
            .key_site()
            .map(|url| {
                ui::link(("get-key", index), "获取密钥")
                    .on_click(move |_, _, cx| cx.open_url(url))
                    .into_any_element()
            })
            .into_iter()
            .collect();
        let manage = ui::key_manage_button(("manage-search-keys", index))
            .disabled(busy)
            .on_click(Self::listen(cx, move |this, window, cx| {
                this.open_keys(index, None, window, cx)
            }));
        let check = Button::new(("check-search", index))
            .outline()
            .label("检测")
            .accessibility_label(format!("检测 · {}", provider.name()))
            .disabled(busy)
            .on_click(Self::listen(cx, move |this, _, cx| {
                this.check_provider(index, cx)
            }));
        // The key input is one entity per provider: while the dialog shows it, the card must not.
        let value = if let Some(key) = preferred(config) {
            self.key_row(index, key, false, busy, cx)
                .more(config.keys.len() - 1)
                .into_any_element()
        } else if self.key_dialog != Some(index) {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(space::S8)
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&inputs.providers[index].key)
                            .aria_label(format!("{} 新密钥", provider.name()))
                            .disabled(busy),
                    ),
                )
                .child(
                    Button::new(("save-first-search-key", index))
                        .primary()
                        .label("加密保存")
                        .disabled(busy)
                        .on_click(Self::listen(cx, move |this, window, cx| {
                            this.save_key_input(index, window, cx)
                        })),
                )
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .child(label(text::METADATA, "尚未保存密钥").text_color(r.text_subtle))
                .into_any_element()
        };
        v_flex()
            .gap(space::S8)
            .child(ui::field_title(
                if provider == Provider::Searxng {
                    "密码（可选）"
                } else {
                    "API 密钥"
                },
                links,
                &r,
            ))
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(value)
                    .child(manage)
                    .child(check),
            )
            .child(ui::value(
                "密钥使用方式",
                "失败时轮换",
                Some("仅在 HTTP 401/403/429 时换下一个已保存密钥，不跨服务商重试".into()),
                &r,
            ))
            .into_any_element()
    }

    /// Opens the key manager; `edit` starts it replacing that key.
    fn open_keys(
        &mut self,
        index: usize,
        edit: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.key_dialog != Some(index) {
            self.previous_focus = window.focused(cx);
            self.clear_provider(index, window, cx);
            self.key_dialog = Some(index);
            if let Some(inputs) = &self.inputs {
                inputs.focus.focus(window, cx);
            }
        }
        self.drafts[index].editing = edit;
    }

    /// One saved key. On the card only the eye shows until revealed; in the manager every action
    /// shows, plus 使用此密钥 when there is a choice.
    fn key_row(
        &self,
        index: usize,
        key: &SavedKey,
        in_dialog: bool,
        busy: bool,
        cx: &Context<Self>,
    ) -> ui::KeyRow {
        let config = &self.stored.configs[index];
        let revealed = self.drafts[index]
            .revealed
            .as_ref()
            .filter(|(id, _)| id == &key.id)
            .map(|(_, value)| value.clone());
        let is_revealed = revealed.is_some();
        let (reveal_id, copy_id, edit_id, delete_id, select_id) = (
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
        );
        let mut row = ui::KeyRow::new(
            format!("search-key-{index}-{}", key.id),
            revealed.unwrap_or_else(|| key.masked.clone()),
            is_revealed,
        )
        .all_actions(in_dialog)
        .disabled(busy)
        .on_reveal(Self::listen(cx, move |this, _, cx| {
            if is_revealed {
                if let Some((_, mut value)) = this.drafts[index].revealed.take() {
                    value.zeroize();
                }
            } else {
                this.reveal_key(index, &reveal_id, false, cx);
            }
        }))
        .on_copy(Self::listen(cx, move |this, _, cx| {
            this.reveal_key(index, &copy_id, true, cx)
        }))
        .on_edit(Self::listen(cx, move |this, window, cx| {
            this.open_keys(index, Some(edit_id.clone()), window, cx);
            if let Some((_, mut value)) = this.drafts[index].revealed.take() {
                value.zeroize();
            }
        }))
        .on_delete(Self::listen(cx, move |this, window, cx| {
            this.clear_provider(index, window, cx);
            this.delete_key = Some((index, delete_id.clone()));
        }));
        if in_dialog && config.keys.len() > 1 {
            row = row.on_select(
                preferred(config).is_some_and(|preferred| preferred.id == key.id),
                Self::listen(cx, move |this, window, cx| {
                    this.submit_key(index, KeyAction::Select(select_id.clone()), window, cx);
                }),
            );
        }
        row
    }

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
            label(text::METADATA, self.card_status[index].clone()).text_color(
                if self.card_error[index] {
                    r.feedback_error
                } else {
                    r.text_muted
                },
            )
        });
        let body = if let Some((_, id)) = self.delete_key.as_ref().filter(|(i, _)| *i == index) {
            let id = id.clone();
            ui::delete_confirm(
                "将删除此密钥；其它密钥和服务商设置保持不变。",
                &r,
                Self::listen(cx, |this, _, _| this.delete_key = None),
                Self::listen(cx, move |this, window, cx| {
                    this.submit_key(index, KeyAction::Delete(id.clone()), window, cx);
                }),
            )
        } else {
            let editing = self.drafts[index].editing.is_some();
            let draft = inputs.providers[index].key.read(cx).value();
            v_flex()
                .gap(space::S16)
                .child(label(
                    text::TITLE_DIALOG,
                    format!("密钥管理 · {}", config.provider.name()),
                ))
                .child(
                    ui::KeyManager::new(
                        Input::new(&inputs.providers[index].key)
                            .aria_label("密钥管理输入")
                            .disabled(busy),
                    )
                    .note("每个服务商最多 16 个，加密保存。")
                    .rows(
                        config
                            .keys
                            .iter()
                            .map(|key| self.key_row(index, key, true, busy, cx)),
                    )
                    .editing(editing)
                    .can_save(
                        !busy && !draft.trim().is_empty() && (editing || config.keys.len() < 16),
                    )
                    .on_save(Self::listen(cx, move |this, window, cx| {
                        this.save_key_input(index, window, cx)
                    }))
                    .on_cancel_edit(Self::listen(cx, move |this, window, cx| {
                        this.clear_provider(index, window, cx)
                    })),
                )
                .child(
                    h_flex().justify_end().child(
                        Button::new("close-search-keys")
                            .outline()
                            .label("完成")
                            .on_click(Self::listen(cx, |this, window, cx| {
                                this.close_keys(window, cx)
                            })),
                    ),
                )
        };
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
            .children(status)
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
                    self.card_status[index] =
                        "已复制密钥到系统剪贴板；请留意其他程序访问剪贴板".into();
                    self.card_error[index] = false;
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

/// The key requests start from: the preferred key, else the first.
fn preferred(config: &Config) -> Option<&SavedKey> {
    config
        .selected_key
        .as_ref()
        .and_then(|id| config.keys.iter().find(|key| &key.id == id))
        .or_else(|| config.keys.first())
}
