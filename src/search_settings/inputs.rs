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
                        .placeholder(if secret { "新密钥" } else { "" })
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
                edit: input(String::new(), true, window, cx),
            })
            .collect();
        let query = input(String::new(), false, window, cx);
        let target = input(String::new(), false, window, cx);
        let max_results = input(self.preferences.max_results.to_string(), false, window, cx);
        let cutoff = input(self.preferences.cutoff.to_string(), false, window, cx);
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
}
