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
    pub(super) fn save(&mut self, index: usize, action: Option<KeyAction>) -> Result<(), String> {
        let mut next = self.stored.clone();
        let config = &mut next.configs[index];
        // Credential manager changes credentials only, never unfinished address or policy drafts.
        if action.is_none() {
            let endpoint = self.drafts[index].endpoint.trim();
            config.endpoint = if endpoint.is_empty() {
                config.provider.endpoint().into()
            } else {
                endpoint.into()
            };
            config.username = self.drafts[index].username.trim().into();
        }
        let mut clear_added = false;
        match action {
            Some(KeyAction::Delete(id)) => config.keys.retain(|key| key.id != id),
            Some(KeyAction::Replace(id, value)) => {
                let value = zeroize::Zeroizing::new(value);
                validate_key(&value)?;
                config
                    .keys
                    .iter_mut()
                    .find(|key| key.id == id)
                    .ok_or("密钥不存在")?
                    .replace(&value)?;
            }
            Some(KeyAction::Add) => {
                add_key(config, &self.drafts[index].key)?;
                clear_added = true;
            }
            None => {
                if !self.drafts[index].key.trim().is_empty() {
                    add_key(config, &self.drafts[index].key)?;
                    clear_added = true;
                }
            }
        }
        next.save(&self.root)
            .map_err(|error| format!("保存失败：{error}；草稿和原凭据均已保留"))?;
        self.stored = next;
        if clear_added {
            self.drafts[index].key.zeroize();
        }
        Ok(())
    }
    pub(super) fn submit(
        &mut self,
        index: usize,
        action: Option<KeyAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if action.is_none() && let Some(inputs) = &self.inputs {
            let input = &inputs.providers[index];
            self.drafts[index].endpoint = input.endpoint.read(cx).value().to_string();
            self.drafts[index].username = input.username.read(cx).value().to_string();
            self.drafts[index].key.zeroize();
            self.drafts[index].key = input.key.read(cx).value().to_string();
        }
        match self.save(index, action) {
            Ok(()) => {
                self.card_error[index] = false;
                self.card_status[index] = "已保存".into();
                if self.drafts[index].key.is_empty() && let Some(inputs) = &self.inputs {
                    inputs.providers[index].key.update(cx, |input, cx| input.set_value("", window, cx));
                }
                true
            }
            Err(error) => {
                self.card_error[index] = true;
                self.card_status[index] = error;
                false
            }
        }
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
