//! Catalog storage boundaries; drafts become saved state only after a successful write.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;

impl ModelSettings {
    pub(super) fn load(root: &Path) -> Result<Self, String> {
        let (trace, operation) = TraceContext::root("settings.models.load", "ui");
        let _entered = operation.span().clone().entered();
        let result = (|| {
            let mut store = task::sync(
                &trace,
                "settings.models.store.open",
                "SETTINGS_STORE_READ_FAILED",
                || CatalogStore::open(root).map_err(|_| "无法打开模型配置；原数据未修改".into()),
            )?;
            let (providers, presets_initialized) = task::sync(
                &trace,
                "settings.models.store.load",
                "SETTINGS_STORE_READ_FAILED",
                || {
                    store
                        .load()
                        .map_err(|_| "无法读取模型配置；原数据未修改".into())
                },
            )?;
            let mut saved = SettingsFile {
                providers,
                presets_initialized,
            };
            project(&saved).map_err(|_| {
                Failure::failed(
                    "SETTINGS_STORED_CONFIG_INVALID",
                    "已保存的模型配置无效；原数据未修改",
                )
            })?;
            if !saved.presets_initialized {
                saved.seed_presets();
                project(&saved).map_err(|_| {
                    Failure::failed(
                        "SETTINGS_STORED_CONFIG_INVALID",
                        "模型预设配置无效；原数据未修改",
                    )
                })?;
                task::sync(
                    &trace,
                    "settings.models.store.seed",
                    "SETTINGS_STORE_WRITE_FAILED",
                    || {
                        store
                            .save(&saved.providers, saved.presets_initialized)
                            .map_err(|_| "无法保存模型预设；原数据未修改".into())
                    },
                )?;
            }
            let mut state = Self {
                store,
                draft: saved.clone(),
                catalog: ModelCatalog::new(saved.providers.clone()),
                options: model_options(&saved),
                image_options: model_options_for(&saved, ModelType::Image),
                saved,
                provider: None,
                endpoint: None,
                search: String::new(),
                key_drafts: HashMap::new(),
                notice: String::new(),
                error: false,
                diagnostic_trace: None,
                catalog_diagnostic_trace: None,
                dirty: false,
                delete_provider: None,
                delete_endpoint: None,
                delete_key: None,
                sync_dialog: None,
                job: None,
                catalog_refresh: None,
                catalog_refresh_notice: String::new(),
                provider_filter: 0,
                model_query: String::new(),
                capability_filter: 0,
                folded: HashSet::new(),
                editor: None,
                view: None,
            };
            state.repair_selection();
            Ok(state)
        })();
        task::end(operation, &result);
        result.map_err(|error| error.notice(&trace))
    }

    pub(super) fn save_in(&mut self, trace: &TraceContext) -> Result<(), Failure> {
        project(&self.draft)
            .map_err(|message| Failure::failed("SETTINGS_INPUT_INVALID", message))?;
        task::sync(
            trace,
            "settings.models.store.save",
            "SETTINGS_STORE_WRITE_FAILED",
            || {
                self.store
                    .save(&self.draft.providers, self.draft.presets_initialized)
                    .map_err(|_| "无法保存模型配置；已保存配置未修改，请保留草稿后重试".into())
            },
        )?;
        self.saved = self.draft.clone();
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        self.dirty = false;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn save(&mut self) -> Result<(), Failure> {
        let (trace, root) = TraceContext::root("settings.models.save", "ui");
        let _entered = root.span().clone().entered();
        let result = self.save_in(&trace);
        task::end(root, &result);
        result
    }

    pub(super) fn persist_keys(
        &mut self,
        trace: &TraceContext,
        p: usize,
        e: usize,
    ) -> Result<(), Failure> {
        let endpoint = &self.draft.providers[p].endpoints[e];
        let saved = self
            .saved
            .providers
            .iter_mut()
            .find(|provider| provider.id == self.draft.providers[p].id)
            .and_then(|provider| {
                provider
                    .endpoints
                    .iter_mut()
                    .find(|saved| saved.id == endpoint.id)
            })
            .ok_or_else(|| Failure::failed("SETTINGS_TARGET_MISSING", "线路已不存在"))?;
        task::sync(
            trace,
            "settings.keys.store.save",
            "SETTINGS_STORE_WRITE_FAILED",
            || {
                self.store
                    .save_keys(
                        &endpoint.id,
                        &endpoint.keys,
                        endpoint.selected_key.as_deref(),
                    )
                    .map_err(|_| "无法保存密钥；原凭据未修改".into())
            },
        )?;
        saved.keys.clone_from(&endpoint.keys);
        saved.selected_key.clone_from(&endpoint.selected_key);
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        Ok(())
    }

    pub(super) fn commit(&mut self) {
        let (trace, root) = TraceContext::root("settings.models.save", "ui");
        let _entered = root.span().clone().entered();
        let result = self.save_in(&trace);
        self.report_task(&trace, &result, "已保存");
        task::end(root, &result);
    }

    pub(super) fn set_enabled(&mut self, p: usize, enabled: bool) {
        let (trace, root) = TraceContext::root("settings.provider.enabled", "ui");
        let _entered = root.span().clone().entered();
        let id = self.draft.providers[p].id.clone();
        let mut candidate = self.saved.clone();
        let result = if let Some(provider) = candidate
            .providers
            .iter_mut()
            .find(|provider| provider.id == id)
        {
            provider.enabled = enabled;
            task::sync(
                &trace,
                "settings.provider.store.enabled",
                "SETTINGS_STORE_WRITE_FAILED",
                || {
                    self.store
                        .set_enabled(&id, enabled)
                        .map_err(|_| "无法保存服务商启用状态；原配置未修改".into())
                },
            )
        } else {
            Err(Failure::failed("SETTINGS_TARGET_MISSING", "服务商已不存在"))
        };
        if self.report_task(&trace, &result, "已保存") {
            self.saved = candidate;
            self.draft.providers[p].enabled = enabled;
            self.catalog = ModelCatalog::new(self.saved.providers.clone());
            self.options = model_options(&self.saved);
            self.image_options = model_options_for(&self.saved, ModelType::Image);
        } else {
            self.draft.providers[p].enabled = self
                .saved
                .providers
                .iter()
                .find(|provider| provider.id == id)
                .is_some_and(|provider| provider.enabled);
        }
        task::end(root, &result);
    }
}
