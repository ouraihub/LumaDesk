//! The endpoint's API-key area and the keys dialog body, drawn with the shared
//! `lumakit_design` key controls; secrets stay in `ModelSettings`.
use super::*;
use gpui_kit::ClickEvent;

impl ModelSettings {
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

    /// API 密钥 · 获取密钥; the first enabled key (or the first-key input), management and 检测.
    pub(super) fn key_section(&self, p: usize, e: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let provider = &self.draft.providers[p];
        let endpoint = &provider.endpoints[e];
        let links: Vec<AnyElement> = PRESETS
            .iter()
            .find(|(id, _, _, _)| *id == provider.id)
            .map(|(_, _, _, link)| {
                let link = *link;
                ui::link("get-key", "获取密钥")
                    .on_click(move |_, _, cx| cx.open_url(link))
                    .into_any_element()
            })
            .into_iter()
            .collect();
        let manage = div()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if this.job.is_none() {
                        this.open_keys(p, e, window, cx);
                        cx.notify();
                    }
                }),
            )
            .child(
                ui::key_manage_button("manage-keys")
                    .disabled(self.job.is_some())
                    .on_click(Self::listen(cx, move |this, window, cx| {
                        if this.editor.is_none() {
                            this.open_keys(p, e, window, cx);
                        }
                    })),
            );
        let detect = self
            .button("detect", "检测", cx, |this, _, _| this.start_job(false))
            .outline()
            .disabled(self.dirty || self.job.is_some());
        // The key input is one entity: while a dialog shows it, the main row must not.
        let value = if let Some(key) = primary_key(endpoint) {
            self.key_row(p, e, key, false, cx)
                .more(endpoint.keys.len() - 1)
                .into_any_element()
        } else if self.editor.is_none() {
            self.input("key")
                .disabled(self.job.is_some())
                .flex_1()
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .child(hint("尚未保存密钥", &r))
                .into_any_element()
        };
        let progress = self.job.as_ref().map(|job| {
            h_flex()
                .gap(space::S8)
                .flex_wrap()
                .child(hint(
                    if job.cancel.is_cancelled() {
                        "正在取消请求…"
                    } else {
                        "正在检测已保存的线路…"
                    },
                    &r,
                ))
                .child(
                    self.button("cancel-job", "取消请求", cx, |this, _, _| {
                        if let Some(job) = &this.job {
                            job.cancel.cancel();
                        }
                    })
                    .outline()
                    .icon(IconName::X)
                    .disabled(job.cancel.is_cancelled()),
                )
                .into_any_element()
        });
        let result = progress.or_else(|| self.status(cx).map(IntoElement::into_any_element));
        let result = if !endpoint.keys.is_empty() && !endpoint.keys.iter().any(|key| key.enabled) {
            Some(
                v_flex()
                    .gap(space::S8)
                    .children(result)
                    .child(hint(
                        "所有密钥均已停用；请在密钥管理中启用至少一个密钥。",
                        &r,
                    ))
                    .into_any_element(),
            )
        } else {
            result
        };
        shared::key_section(
            links,
            value,
            manage.into_any_element(),
            detect.into_any_element(),
            result,
            &r,
        )
    }

    fn open_keys(&mut self, p: usize, e: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.open_editor(
            EditorDialog::Keys {
                provider: self.draft.providers[p].id.clone(),
                endpoint: self.draft.providers[p].endpoints[e].id.clone(),
            },
            window,
            cx,
        );
    }

    /// Saved key label and mask; reveal is page-only, delete is manager-only.
    fn key_row(
        &self,
        p: usize,
        e: usize,
        key: &SavedKey,
        in_dialog: bool,
        cx: &Context<Self>,
    ) -> ui::KeyRow {
        let endpoint = &self.draft.providers[p].endpoints[e];
        let revealed = self
            .key_drafts
            .get(&endpoint.id)
            .filter(|d| d.revealed_key.as_ref() == Some(&key.id))
            .filter(|_| !in_dialog)
            .and_then(|d| d.revealed.as_ref());
        let shown = revealed.map_or_else(|| key.masked.clone(), |value| value.to_string());
        let is_revealed = revealed.is_some();
        let (reveal_id, copy_id, edit_id, delete_id, toggle_id) = (
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
        );
        let mut row = ui::KeyRow::new(format!("key-{}", key.id), shown, is_revealed)
            .disabled(self.job.is_some())
            .roles(roles(cx))
            .hover_actions(!in_dialog)
            .all_actions(in_dialog)
            .on_copy(Self::listen(cx, move |this, _, cx| {
                let result = this.draft.providers[p].endpoints[e]
                    .keys
                    .iter()
                    .find(|key| key.id == copy_id)
                    .ok_or_else(|| "密钥已不存在".to_owned())
                    .and_then(SavedKey::open);
                match result {
                    Ok(secret) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(secret.to_string()));
                        this.notice = "已复制密钥到系统剪贴板；请留意其他程序访问剪贴板".into();
                    }
                    Err(error) => {
                        this.report(Err(error), "");
                    }
                }
            }))
            .on_edit(Self::listen(cx, move |this, window, cx| {
                if !matches!(this.editor, Some(EditorDialog::Keys { .. })) {
                    this.open_keys(p, e, window, cx);
                }
                let Some(label) = this.draft.providers[p].endpoints[e]
                    .keys
                    .iter()
                    .find(|key| key.id == edit_id)
                    .map(|key| key.label.clone().unwrap_or_default())
                else {
                    this.report(Err("密钥已不存在".into()), "");
                    return;
                };
                let draft = this
                    .key_drafts
                    .entry(this.draft.providers[p].endpoints[e].id.clone())
                    .or_default();
                draft.value.zeroize();
                draft.label = label.clone();
                draft.submitted = false;
                draft.editing = Some(edit_id.clone());
                draft.revealed = None;
                draft.revealed_key = None;
                this.set_input("key", "", window, cx);
                this.set_input("key-label", label, window, cx);
            }));
        if let Some(label) = &key.label {
            row = row.label(label.clone());
        } else if in_dialog {
            row = row.label("API Key");
        }
        if !in_dialog {
            row = row.on_reveal(Self::listen(cx, move |this, _, _| {
                if is_revealed {
                    if let Some(draft) = this
                        .key_drafts
                        .get_mut(&this.draft.providers[p].endpoints[e].id)
                    {
                        draft.revealed = None;
                        draft.revealed_key = None;
                    }
                } else {
                    this.reveal_key(p, e, &reveal_id);
                }
            }));
        }
        if in_dialog {
            row = row.on_delete(Self::listen(cx, move |this, window, cx| {
                this.key_drafts.clear();
                this.set_input("key", "", window, cx);
                this.set_input("key-label", "", window, cx);
                this.delete_key = Some((
                    this.draft.providers[p].id.clone(),
                    this.draft.providers[p].endpoints[e].id.clone(),
                    delete_id.clone(),
                ));
            }));
        }
        if in_dialog {
            row = row.trailing(
                Switch::new(format!("key-enabled-{}", key.id))
                    .checked(key.enabled)
                    .color(roles(cx).feedback_success)
                    .disabled(self.job.is_some())
                    .accessibility_label(format!(
                        "启用密钥 {}",
                        key.label.as_deref().unwrap_or(&key.masked)
                    ))
                    .on_click(cx.listener(move |this, enabled: &bool, _, cx| {
                        this.set_key_enabled(p, e, &toggle_id, *enabled);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        row
    }

    /// The keys dialog body for `endpoint` of `provider`; nothing when either is gone.
    pub(super) fn keys_manager(
        &self,
        provider: &str,
        endpoint: &str,
        cx: &Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let p = self.draft.providers.iter().position(|p| p.id == provider)?;
        let e = self.draft.providers[p]
            .endpoints
            .iter()
            .position(|e| e.id == endpoint)?;
        let keys = &self.draft.providers[p].endpoints[e].keys;
        let editing = self
            .key_drafts
            .get(endpoint)
            .is_some_and(|draft| draft.editing.is_some());
        let r = roles(cx);
        let save = self
            .button(
                "save-key",
                if editing {
                    "保存"
                } else {
                    "添加 API 密钥"
                },
                cx,
                |this, window, cx| this.save_key_input(window, cx),
            )
            .primary()
            .disabled(self.job.is_some());
        let cancel = editing.then(|| {
            self.button("cancel-key-edit", "取消修改", cx, |this, window, cx| {
                this.key_drafts.clear();
                this.set_input("key", "", window, cx);
                this.set_input("key-label", "", window, cx);
            })
            .outline()
            .disabled(self.job.is_some())
            .into_any_element()
        });
        Some(shared::key_manager(
            keys.iter()
                .map(|key| self.key_row(p, e, key, true, cx).into_any_element()),
            std::iter::empty(),
            v_flex()
                .gap(space::S8)
                .child(shared::key_entry(
                    self.input("key-label")
                        .aria_label("密钥标签（可选）")
                        .disabled(self.job.is_some())
                        .into_any_element(),
                    self.input("key")
                        .aria_label("API 密钥（修改时留空保留原密钥）")
                        .mask_toggle()
                        .disabled(self.job.is_some())
                        .into_any_element(),
                    h_flex()
                        .gap(space::S8)
                        .flex_wrap()
                        .child(save)
                        .children(cancel)
                        .into_any_element(),
                ))
                .child(hint(
                    format!(
                        "{} / {} 已启用",
                        keys.iter().filter(|key| key.enabled).count(),
                        keys.len()
                    ),
                    &r,
                ))
                .into_any_element(),
            hint(
                format!("每条线路最多 {MAX_KEYS} 个密钥；修改时留空保留原密钥。"),
                &r,
            )
            .into_any_element(),
        ))
    }
}
