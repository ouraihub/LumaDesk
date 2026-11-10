//! Native input entities and change subscriptions.
use super::*;
impl SearchSettings {
    pub(super) fn with_inputs(mut self, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let mut input =
            |value: String, secret: bool, window: &mut Window, cx: &mut Context<Self>| {
                let state = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(value)
                        .placeholder(if secret {
                            "输入密钥；失败时保留草稿"
                        } else {
                            ""
                        })
                        .masked(secret)
                });
                subscriptions.push(cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
                state
            };
        let providers: Vec<ProviderInputs> = self
            .drafts
            .iter()
            .map(|draft| ProviderInputs {
                endpoint: input(draft.endpoint.clone(), false, window, cx),
                username: input(draft.username.clone(), false, window, cx),
                key: input(String::new(), true, window, cx),
                label: cx.new(|cx| InputState::new(window, cx).placeholder("标签")),
            })
            .collect();
        let query = input(String::new(), false, window, cx);
        let target = input(String::new(), false, window, cx);
        let max_results = input(self.preferences.max_results.to_string(), false, window, cx);
        let cutoff = input(self.preferences.cutoff.to_string(), false, window, cx);
        // Both manager fields submit on Enter; only the visible, unsaved card key saves on Blur.
        for (index, provider) in providers.iter().enumerate() {
            for (is_label, input) in [(false, &provider.key), (true, &provider.label)] {
                subscriptions.push(cx.subscribe_in(
                    input,
                    window,
                    move |this, input, event: &InputEvent, window, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = input.read(cx).value().to_string();
                            let draft = &mut this.drafts[index];
                            if is_label {
                                draft.label = value;
                            } else {
                                draft.key.zeroize();
                                draft.key = value;
                            }
                            draft.submitted = false;
                            draft.generation = draft.generation.wrapping_add(1);
                        } else if matches!(event, InputEvent::PressEnter { .. })
                            && !this.is_busy()
                            && (this.key_dialog == Some(index)
                                || (!is_label && this.owns_primary_key_input(index)))
                        {
                            this.save_key_input(index, window, cx);
                        } else if !is_label
                            && matches!(event, InputEvent::Blur)
                            && this.owns_primary_key_input(index)
                            && !this.drafts[index].submitted
                        {
                            let generation = this.drafts[index].generation;
                            cx.defer_in(window, move |this, window, cx| {
                                let draft = &this.drafts[index];
                                if !this.is_busy()
                                    && this.owns_primary_key_input(index)
                                    && draft.generation == generation
                                    && !draft.submitted
                                    && !draft.key.trim().is_empty()
                                {
                                    this.save_key_input(index, window, cx);
                                    cx.notify();
                                }
                            });
                        }
                        cx.notify();
                    },
                ));
            }
        }
        let blacklist =
            cx.new(|cx| TextareaState::new(window, cx).default_value(self.blacklist.clone()));
        subscriptions.push(cx.subscribe(&blacklist, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        self.inputs = Some(SearchInputs {
            providers,
            query,
            target,
            max_results,
            cutoff,
            blacklist,
            _subscriptions: subscriptions,
            focus: cx.focus_handle(),
        });
        self
    }

    fn owns_primary_key_input(&self, index: usize) -> bool {
        self.key_dialog.is_none()
            && (self.search_index == index || self.fetch_index == index)
            && self.stored.configs[index].keys.is_empty()
    }
}
