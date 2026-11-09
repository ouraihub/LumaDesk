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

    /// API 密钥 · 获取密钥; the selected key's value box (or the first-key input), key management and 检测.
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
        let manage = ui::key_manage_button("manage-keys")
            .on_click(Self::listen(cx, move |this, window, cx| {
                this.open_keys(p, e, window, cx)
            }));
        let detect = self
            .button("detect", "检测", cx, |this, _, _| this.start_job(false))
            .outline()
            .disabled(self.dirty);
        // The key input is one entity: while a dialog shows it, the main row must not.
        let value = if !endpoint.keys.is_empty() {
            let key = endpoint
                .selected_key
                .as_ref()
                .and_then(|id| endpoint.keys.iter().find(|key| &key.id == id))
                .unwrap_or(&endpoint.keys[0]);
            self.key_row(p, e, key, false, cx)
                .more(endpoint.keys.len() - 1)
                .into_any_element()
        } else if self.editor.is_none() {
            h_flex()
                .flex_1()
                .min_w(px(0.))
                .gap(space::S8)
                .child(self.input("key").flex_1())
                .child(
                    self.button(
                        "save-primary-key",
                        "加密保存",
                        cx,
                        |this, window, cx| this.save_key_input(window, cx),
                    )
                    .primary(),
                )
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .child(hint("尚未保存密钥", &r))
                .into_any_element()
        };
        v_flex()
            .gap(space::S8)
            .child(ui::field_title("API 密钥", links, &r))
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(value)
                    .child(manage)
                    .child(detect),
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

    /// One saved key. In the page row only the eye shows until revealed; in the keys dialog every
    /// action shows, plus 使用此密钥 when there is a choice.
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
            .and_then(|d| d.revealed.as_ref());
        let shown = revealed.map_or_else(|| key.masked.clone(), |value| value.to_string());
        let is_revealed = revealed.is_some();
        let (reveal_id, copy_id, edit_id, delete_id, select_id) = (
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
            key.id.clone(),
        );
        let mut row = ui::KeyRow::new(format!("key-{}", key.id), shown, is_revealed)
            .all_actions(in_dialog)
            .on_reveal(Self::listen(cx, move |this, _, _| {
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
            }))
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
                let draft = this
                    .key_drafts
                    .entry(this.draft.providers[p].endpoints[e].id.clone())
                    .or_default();
                draft.value.zeroize();
                draft.editing = Some(edit_id.clone());
                draft.revealed = None;
                draft.revealed_key = None;
                this.set_input("key", "", window, cx);
            }))
            .on_delete(Self::listen(cx, move |this, window, cx| {
                this.key_drafts.clear();
                this.set_input("key", "", window, cx);
                this.delete_key = Some((
                    this.draft.providers[p].id.clone(),
                    this.draft.providers[p].endpoints[e].id.clone(),
                    delete_id.clone(),
                ));
            }));
        if in_dialog && endpoint.keys.len() > 1 {
            row = row.on_select(
                endpoint.selected_key.as_ref() == Some(&key.id),
                Self::listen(cx, move |this, _, _| this.change_key(p, e, &select_id)),
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
    ) -> Option<ui::KeyManager> {
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
        Some(
            ui::KeyManager::new(self.input("key"))
                .note(format!("每条线路最多 {MAX_KEYS} 个，加密保存。"))
                .rows(keys.iter().map(|key| self.key_row(p, e, key, true, cx)))
                .editing(editing)
                .on_save(Self::listen(cx, |this, window, cx| {
                    this.save_key_input(window, cx)
                }))
                .on_cancel_edit(Self::listen(cx, |this, window, cx| {
                    this.key_drafts.clear();
                    this.set_input("key", "", window, cx);
                })),
        )
    }
}
