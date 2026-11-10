use super::*;

impl ModelSettings {
    pub(super) fn init_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut inputs = HashMap::new();
        let mut subscriptions = Vec::new();
        for (key, placeholder) in [
            ("provider-search", "搜索模型平台…"),
            ("url", "https://api.example.com/v1"),
            ("model-search", "搜索名称或 ID…"),
            ("sync-search", "搜索模型…"),
            ("name", "名称"),
            ("editor-url", "API 地址"),
            ("key-label", "标签"),
            ("key", "输入 API 密钥；失败时保留草稿"),
        ] {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(key == "key")
            });
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        match key {
                            "provider-search" => this.search = value,
                            "model-search" => this.model_query = value,
                            "sync-search" => {
                                if let Some(dialog) = &mut this.sync_dialog {
                                    dialog.search = value;
                                }
                            }
                            "url" => {
                                if this.job.is_some() {
                                    return;
                                }
                                if let Some((p, Some(e))) = this.indices() {
                                    this.draft.providers[p].endpoints[e].base_url = value;
                                    this.dirty = true;
                                }
                            }
                            "name" => {
                                if let Some(
                                    EditorDialog::Provider { name, .. }
                                    | EditorDialog::Endpoint { name, .. },
                                ) = &mut this.editor
                                {
                                    *name = value;
                                }
                            }
                            "editor-url" => {
                                if let Some(EditorDialog::Endpoint { url, .. }) = &mut this.editor {
                                    *url = value;
                                }
                            }
                            "key" | "key-label" => {
                                if let Some((p, e)) = this.key_target() {
                                    let draft = this
                                        .key_drafts
                                        .entry(this.draft.providers[p].endpoints[e].id.clone())
                                        .or_default();
                                    if key == "key-label" {
                                        draft.label = value;
                                    } else {
                                        *draft.value = value;
                                    }
                                    draft.submitted = false;
                                }
                            }
                            _ => {}
                        }
                    } else if key == "url"
                        && matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                        && this.dirty
                    {
                        this.commit();
                    } else if (key == "key"
                        || (key == "key-label"
                            && matches!(this.editor, Some(EditorDialog::Keys { .. }))))
                        && matches!(event, InputEvent::PressEnter { .. })
                    {
                        this.save_key_input(window, cx);
                    } else if key == "key"
                        && matches!(event, InputEvent::Blur)
                        && this.editor.is_none()
                    {
                        let target = this.key_target().map(|(p, e)| {
                            (
                                this.draft.providers[p].id.clone(),
                                this.draft.providers[p].endpoints[e].id.clone(),
                            )
                        });
                        cx.defer_in(window, move |this, window, cx| {
                            if this.editor.is_none()
                                && this.job.is_none()
                                && target
                                    == this.key_target().map(|(p, e)| {
                                        (
                                            this.draft.providers[p].id.clone(),
                                            this.draft.providers[p].endpoints[e].id.clone(),
                                        )
                                    })
                                && this.key_target().is_some_and(|(p, e)| {
                                    this.key_drafts
                                        .get(&this.draft.providers[p].endpoints[e].id)
                                        .is_some_and(|d| !d.submitted && !d.value.trim().is_empty())
                                })
                            {
                                this.save_key_input(window, cx);
                                cx.notify();
                            }
                        });
                    }
                    cx.notify();
                },
            ));
            inputs.insert(key, input);
        }
        self.view = Some(ModelView {
            inputs,
            _subscriptions: subscriptions,
            model: None,
            focus: cx.focus_handle(),
            modal_open: false,
            previous_focus: None,
        });
        self.refresh_endpoint(window, cx);
    }
    pub fn is_busy(&self) -> bool {
        self.job.is_some() || self.refreshing_catalog()
    }
    pub fn clear_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_drafts();
        self.delete_provider = None;
        self.delete_endpoint = None;
        if let Some(view) = &mut self.view {
            view.model = None;
            view.modal_open = false;
            view.previous_focus = None;
        }
        self.set_input("key", "", window, cx);
        self.set_input("key-label", "", window, cx);
        self.set_input("sync-search", "", window, cx);
        cx.notify();
    }
    pub(super) fn set_input(
        &self,
        key: &'static str,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value: SharedString = value.into();
        if let Some(input) = self.view.as_ref().and_then(|view| view.inputs.get(key)) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }
    pub(super) fn input(&self, key: &'static str) -> Input {
        Input::new(&self.view.as_ref().unwrap().inputs[key])
    }
    pub(super) fn refresh_endpoint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_drafts.clear();
        self.set_input("key", "", window, cx);
        self.set_input("key-label", "", window, cx);
        let url = self
            .indices()
            .and_then(|(p, e)| e.map(|e| self.draft.providers[p].endpoints[e].base_url.clone()))
            .unwrap_or_default();
        self.set_input("url", url, window, cx);
    }
}
