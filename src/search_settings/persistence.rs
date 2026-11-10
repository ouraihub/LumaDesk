//! Independent provider, credential and preference commits.
use super::*;
impl SearchSettings {
    pub(super) fn choose_provider(
        &mut self,
        fetch: bool,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() {
            return;
        }
        let old = if fetch {
            self.fetch_index
        } else {
            self.search_index
        };
        if old == index {
            return;
        }
        let mut next = self.stored.clone();
        if fetch {
            next.default_fetch = next.configs[index].provider;
        } else {
            next.default_search = next.configs[index].provider;
        }
        match next.save(&self.root) {
            Ok(()) => {
                self.clear_provider(old, window, cx);
                self.stored = next;
                if fetch {
                    self.fetch_index = index;
                } else {
                    self.search_index = index;
                }
            }
            Err(error) => {
                self.card_status[old] = error;
                self.card_error[old] = true;
            }
        }
        cx.notify();
    }
    /// Commits one provider: `None` saves the card (address, Basic username) and never touches
    /// credentials; `Some` changes credentials only, never unfinished address drafts.
    pub(super) fn save(&mut self, index: usize, action: Option<KeyAction>) -> Result<(), String> {
        let mut next = self.stored.clone();
        let config = &mut next.configs[index];
        let mut clear_credential = false;
        match action {
            None => {
                let endpoint = self.drafts[index].endpoint.trim();
                config.endpoint = if endpoint.is_empty() {
                    config.provider.endpoint().into()
                } else {
                    endpoint.into()
                };
                config.username = self.drafts[index].username.trim().into();
            }
            Some(KeyAction::Delete(id)) => {
                config.keys.retain(|key| key.id != id);
                if config.selected_key.as_ref() == Some(&id) {
                    config.selected_key = config.keys.first().map(|key| key.id.clone());
                }
            }
            Some(KeyAction::Replace(id, value)) => {
                let value = zeroize::Zeroizing::new(value);
                let label = normalize_key_label(&self.drafts[index].label)?.map(str::to_owned);
                let key = config
                    .keys
                    .iter_mut()
                    .find(|key| key.id == id)
                    .ok_or("密钥不存在")?;
                if !value.trim().is_empty() {
                    validate_key(&value)?;
                    key.replace(value.trim())?;
                }
                key.label = label;
                clear_credential = true;
            }
            Some(KeyAction::Add) => {
                let label = normalize_key_label(&self.drafts[index].label)?.map(str::to_owned);
                add_key(config, &self.drafts[index].key)?;
                if let Some(key) = config.keys.last_mut() {
                    key.label = label;
                }
                clear_credential = true;
            }
            Some(KeyAction::SetEnabled(id, enabled)) => {
                config
                    .keys
                    .iter_mut()
                    .find(|key| key.id == id)
                    .ok_or("密钥不存在")?
                    .enabled = enabled;
            }
        }
        next.save(&self.root)
            .map_err(|error| format!("保存失败：{error}；草稿和原凭据均已保留"))?;
        self.stored = next;
        if clear_credential {
            self.drafts[index].key.zeroize();
            self.drafts[index].label.clear();
        }
        Ok(())
    }
    /// Saves the card's address and username from its inputs.
    pub(super) fn submit_card(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if let Some(inputs) = &self.inputs {
            let input = &inputs.providers[index];
            self.drafts[index].endpoint = input.endpoint.read(cx).value().to_string();
            self.drafts[index].username = input.username.read(cx).value().to_string();
        }
        let result = self.save(index, None);
        self.report(index, result, "已保存")
    }
    /// Commits a credential change; the shared key input is cleared only on success.
    pub(super) fn submit_key(
        &mut self,
        index: usize,
        action: KeyAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let preserve_draft = matches!(&action, KeyAction::SetEnabled(..));
        let success = match action {
            KeyAction::Add | KeyAction::Replace(..) => "密钥已加密保存",
            KeyAction::Delete(_) => "已删除",
            KeyAction::SetEnabled(..) => "密钥启用状态已保存",
        };
        let result = self.save(index, Some(action));
        let saved = self.report(index, result, success);
        if saved && !preserve_draft {
            self.clear_provider(index, window, cx);
        }
        saved
    }
    /// Saves the key input as a new key, or as the replacement of the key being edited.
    pub(super) fn save_key_input(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() {
            return;
        }
        let Some(inputs) = &self.inputs else {
            return;
        };
        let input = &inputs.providers[index];
        let value = zeroize::Zeroizing::new(input.key.read(cx).value().to_string());
        self.drafts[index].label = input.label.read(cx).value().to_string();
        self.drafts[index].key.zeroize();
        self.drafts[index].key = value.as_str().to_owned();
        if self.drafts[index].editing.is_none() && value.trim().is_empty() {
            if self.key_dialog == Some(index) {
                self.report(index, Err("密钥不能为空，原凭据未修改".into()), "");
            }
            return;
        }
        self.drafts[index].submitted = true;
        let action = match self.drafts[index].editing.clone() {
            Some(id) => KeyAction::Replace(id, value.as_str().to_owned()),
            None => KeyAction::Add,
        };
        self.submit_key(index, action, window, cx);
        self.drafts[index].submitted = true;
    }
    fn report(&mut self, index: usize, result: Result<(), String>, success: &str) -> bool {
        self.card_diagnostic_trace[index] = None;
        self.card_error[index] = result.is_err();
        self.card_status[index] = result
            .as_ref()
            .map_or_else(Clone::clone, |()| success.into());
        result.is_ok()
    }
    pub(super) fn save_preferences(&mut self, cx: &Context<Self>) {
        let Some(inputs) = &self.inputs else {
            return;
        };
        let result = (|| {
            let mut preferences = self.preferences.clone();
            preferences.max_results = inputs
                .max_results
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| "结果数须为 1–50 的整数")?;
            preferences.cutoff = inputs
                .cutoff
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| "截断长度须为 100–20000 的整数")?;
            let text = inputs.blacklist.read(cx).value().to_string();
            let mut next = self.stored.clone();
            next.preferences = preferences_from_draft(&preferences, &text)?;
            next.compression = self.compression;
            next.save(&self.root)?;
            self.preferences = next.preferences.clone();
            self.blacklist = self.preferences.blacklist.join("\n");
            self.stored = next;
            Ok::<_, String>(())
        })();
        self.status = result.map_or_else(|error| error, |()| "高级设置已保存".into());
    }
}
