//! The provider card's API-key area and the key manager dialog, drawn with the shared
//! `lumakit_design` key controls exactly as the model page does; secrets stay in the drafts.
use super::*;
use gpui_kit::component::switch::Switch;
use gpui_kit::{App, ClickEvent};
use lumakit_gpui::providers as shared;

impl SearchSettings {
    /// A click handler that runs `action` on this page and repaints.
    pub(super) fn listen(
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _, window, cx| {
            action(this, window, cx);
            cx.notify();
        })
    }

    /// The shared model-service key section; SearXNG keeps its optional-password heading.
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
        let manage = div()
            .capture_any_mouse_down(cx.listener(move |this, _, _, _| {
                // Suppress the card's Blur submit before the button click opens the overlay.
                // Opening on mouse-down would let the following mouse-up hit the new overlay.
                this.drafts[index].submitted = true;
                this.drafts[index].generation = this.drafts[index].generation.wrapping_add(1);
            }))
            .child(
                ui::key_manage_button(("manage-search-keys", index))
                    .disabled(busy)
                    .on_click(Self::listen(cx, move |this, window, cx| {
                        if this.key_dialog != Some(index) {
                            this.open_keys(index, None, window, cx);
                        }
                    })),
            );
        let check = Button::new(("check-search", index))
            .outline()
            .label("检测")
            .accessibility_label(format!("检测 · {}", provider.name()))
            .disabled(busy)
            .on_click(Self::listen(cx, move |this, _, cx| {
                this.check_provider(index, cx)
            }));
        // Never mount the same input entity in both the card and the manager.
        let value = if let Some(key) = preferred(config) {
            self.key_row(index, key, false, busy, cx)
                .more(config.keys.len() - 1)
                .into_any_element()
        } else if self.key_dialog.is_none() {
            Input::new(&inputs.providers[index].key)
                .aria_label(format!("{} 新密钥", provider.name()))
                .disabled(busy)
                .flex_1()
                .into_any_element()
        } else {
            label(text::METADATA, "尚未保存密钥")
                .text_color(r.text_subtle)
                .into_any_element()
        };
        let result =
            (!config.keys.is_empty() && !config.keys.iter().any(|key| key.enabled)).then(|| {
                label(
                    text::METADATA,
                    "所有密钥均已停用；请在密钥管理中启用至少一个密钥。",
                )
                .whitespace_normal()
                .text_color(r.text_subtle)
                .into_any_element()
            });
        let section = if provider == Provider::Searxng {
            // Shared key_section fixes the API heading; only this semantic wrapper differs.
            v_flex()
                .min_w_0()
                .gap(space::S6)
                .child(ui::field_title("密码（可选）", links, &r))
                .child(
                    h_flex()
                        .min_w_0()
                        .gap(space::S8)
                        .flex_wrap()
                        .child(div().flex_1().min_w_0().child(value))
                        .child(manage)
                        .child(check),
                )
                .children(result)
        } else {
            shared::key_section(
                links,
                value,
                manage.into_any_element(),
                check.into_any_element(),
                result,
                &r,
            )
        };
        v_flex()
            .min_w_0()
            .gap(space::S8)
            .child(section)
            .child(shared::strategy(
                "失败时轮换",
                "仅在 HTTP 401/403/429 时换下一个已启用密钥，不跨服务商重试",
                &r,
            ))
            .into_any_element()
    }

    /// Opens the manager without submitting the card input; editing never decrypts the secret.
    fn open_keys(
        &mut self,
        index: usize,
        edit: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() {
            return;
        }
        if self.key_dialog != Some(index) {
            self.previous_focus = window.focused(cx);
            self.clear_provider(index, window, cx);
            self.key_dialog = Some(index);
            if let Some(inputs) = &self.inputs {
                inputs.focus.focus(window, cx);
            }
        }
        if let Some(id) = edit {
            let Some(label) = self.stored.configs[index]
                .keys
                .iter()
                .find(|key| key.id == id)
                .map(|key| key.label.clone().unwrap_or_default())
            else {
                return;
            };
            self.clear_provider(index, window, cx);
            self.drafts[index].label = label.clone();
            self.drafts[index].editing = Some(id);
            if let Some(inputs) = &self.inputs {
                inputs.providers[index].label.update(cx, |input, cx| {
                    input.set_value(label, window, cx);
                    input.focus(window, cx);
                });
            }
        }
    }

    /// Copy/edit are hover actions; reveal is card-only and delete/toggle are manager-only.
    pub(super) fn key_row(
        &self,
        index: usize,
        key: &SavedKey,
        in_dialog: bool,
        busy: bool,
        cx: &Context<Self>,
    ) -> ui::KeyRow {
        let revealed = self.drafts[index]
            .revealed
            .as_ref()
            .filter(|(id, _)| !in_dialog && id == &key.id)
            .map(|(_, value)| value.clone());
        let is_revealed = revealed.is_some();
        let (reveal_id, copy_id, edit_id, delete_id, toggle_id) = (
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
        .label(key.label.clone().unwrap_or_else(|| "API Key".into()))
        .roles(lumakit_design::LumaTheme::global(cx).roles)
        .hover_actions(!in_dialog)
        .all_actions(in_dialog)
        .disabled(busy)
        .on_copy(Self::listen(cx, move |this, _, cx| {
            this.reveal_key(index, &copy_id, true, cx)
        }))
        .on_edit(Self::listen(cx, move |this, window, cx| {
            this.open_keys(index, Some(edit_id.clone()), window, cx);
        }));
        if !in_dialog {
            row = row.on_reveal(Self::listen(cx, move |this, _, cx| {
                if is_revealed {
                    if let Some((_, mut value)) = this.drafts[index].revealed.take() {
                        value.zeroize();
                    }
                } else {
                    this.reveal_key(index, &reveal_id, false, cx);
                }
            }));
        } else {
            row = row
                .on_delete(Self::listen(cx, move |this, window, cx| {
                    this.clear_provider(index, window, cx);
                    this.delete_key = Some((index, delete_id.clone()));
                }))
                .trailing(
                    Switch::new(format!("search-key-enabled-{index}-{}", key.id))
                        .checked(key.enabled)
                        .color(lumakit_design::LumaTheme::global(cx).roles.feedback_success)
                        .disabled(busy)
                        .accessibility_label(format!(
                            "启用密钥 {}",
                            key.label.as_deref().unwrap_or(&key.masked)
                        ))
                        .on_click(cx.listener(move |this, enabled: &bool, window, cx| {
                            this.submit_key(
                                index,
                                KeyAction::SetEnabled(toggle_id.clone(), *enabled),
                                window,
                                cx,
                            );
                            cx.notify();
                        })),
                );
        }
        row
    }

    fn reveal_key(&mut self, index: usize, id: &str, copy: bool, cx: &mut Context<Self>) {
        let (trace, root) = lumakit_observability::TraceContext::root(
            if copy {
                "settings.search.credentials.copy"
            } else {
                "settings.search.credentials.reveal"
            },
            "ui",
        );
        let _entered = root.span().clone().entered();
        let result = self.stored.configs[index]
            .keys
            .iter()
            .find(|key| key.id == id)
            .ok_or("密钥不存在".to_owned())
            .and_then(SavedKey::open);
        let result = result.map_err(|_| {
            crate::settings_task::Failure::failed(
                "SETTINGS_SECRET_FAILED",
                "无法读取密钥，请在原 Windows 用户下运行或重新填写",
            )
        });
        crate::settings_task::end(root, &result);
        self.card_diagnostic_trace[index] =
            result.as_ref().err().map(|_| trace.trace_id().to_owned());
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
                self.card_status[index] = error.notice(&trace);
                self.card_error[index] = true;
            }
        }
    }
}

/// Match the model card: enabled preference, then any enabled key, then a stored disabled key.
fn preferred(config: &Config) -> Option<&SavedKey> {
    config
        .selected_key
        .as_ref()
        .and_then(|id| config.keys.iter().find(|key| key.enabled && &key.id == id))
        .or_else(|| config.keys.iter().find(|key| key.enabled))
        .or_else(|| config.keys.first())
}
