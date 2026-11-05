//! Picker refresh reads saved state; discovery only adds models and never opens the sync editor.
use super::*;

type Discovery = Vec<(DetectConfig, Result<Vec<String>, String>)>;
pub(super) struct CatalogRefreshJob {
    cancel: CancellationToken,
    thread: JoinHandle<()>,
    rx: mpsc::Receiver<Discovery>,
}

enum RefreshFailure {
    Store(String),
    Worker,
    Endpoint { provider: String, endpoint: String, detail: String },
}
impl RefreshFailure {
    fn notice(self) -> String {
        match self {
            Self::Store(detail) => format!("[catalog_store] {detail}；原模型目录未修改"),
            Self::Worker => "[catalog_worker] 模型刷新线程异常结束；原模型目录未修改，请重试".into(),
            Self::Endpoint { provider, endpoint, detail } => format!("[catalog_endpoint] {provider} / {endpoint}：{detail}"),
        }
    }
}

impl ModelSettings {
    /// Reload saved configuration; force additionally discovers every enabled saved endpoint.
    pub fn refresh_catalog(&mut self, force: bool) {
        if let Err(error) = self.reload_catalog() {
            self.catalog_refresh_notice = RefreshFailure::Store(error).notice();
            return;
        }
        if self.refreshing_catalog() { return; }
        if !force {
            self.catalog_refresh_notice = format!("已重新载入本地目录：{} 个聊天模型", self.options.len());
            return;
        }
        let configs: Vec<_> = self.saved.providers.iter().filter(|provider| provider.enabled)
            .flat_map(|provider| provider.endpoints.iter().map(|endpoint| DetectConfig { provider: provider.id.clone(), endpoint: endpoint.clone() })).collect();
        if configs.is_empty() {
            self.catalog_refresh_notice = "没有已启用的已保存线路；未请求网络".into();
            return;
        }
        let count = configs.len();
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new().name("model-catalog-refresh".into()).spawn(move || {
            let mut results = Vec::with_capacity(configs.len());
            for config in configs {
                if worker_cancel.is_cancelled() { return; }
                let result = detect_models(&config.endpoint, worker_cancel.clone());
                results.push((config, result));
            }
            let _ = tx.send(results);
        });
        match thread {
            Ok(thread) => {
                self.catalog_refresh = Some(CatalogRefreshJob { cancel, thread, rx });
                self.catalog_refresh_notice = format!("正在刷新 {count} 条已保存线路的模型目录…");
            }
            Err(_) => self.catalog_refresh_notice = RefreshFailure::Worker.notice(),
        }
    }

    /// Whether the picker catalog worker still owns an outstanding request.
    pub fn refreshing_catalog(&self) -> bool { self.catalog_refresh.is_some() }
    /// Local reload, network discovery, or typed failure notice for the picker.
    pub fn catalog_refresh_notice(&self) -> &str { &self.catalog_refresh_notice }

    fn reload_catalog(&mut self) -> Result<(), String> {
        let (providers, presets_initialized) = self.store.load()?;
        let file = SettingsFile { providers, presets_initialized };
        project(&file)?;
        self.install_catalog(file);
        Ok(())
    }

    fn install_catalog(&mut self, file: SettingsFile) {
        if !self.dirty {
            self.draft = file.clone();
        } else {
            // Keep unfinished editor fields; incorporate only models absent from the previous saved state.
            for provider in &file.providers {
                for endpoint in &provider.endpoints {
                    let previous = self.saved.providers.iter().find(|p| p.id == provider.id)
                        .and_then(|p| p.endpoints.iter().find(|e| e.id == endpoint.id));
                    let draft = self.draft.providers.iter_mut().find(|p| p.id == provider.id)
                        .and_then(|p| p.endpoints.iter_mut().find(|e| e.id == endpoint.id && e.protocol == endpoint.protocol && e.base_url == endpoint.base_url));
                    if let (Some(previous), Some(draft)) = (previous, draft) {
                        for model in &endpoint.models {
                            if !previous.models.iter().any(|m| m.id == model.id) && !draft.models.iter().any(|m| m.id == model.id) {
                                draft.models.push(model.clone());
                            }
                        }
                    }
                }
            }
        }
        self.saved = file;
        self.catalog = ModelCatalog::new(self.saved.providers.clone());
        self.options = model_options(&self.saved);
        self.image_options = model_options_for(&self.saved, ModelType::Image);
        self.repair_selection();
    }

    pub(super) fn poll_catalog_refresh(&mut self) -> bool {
        let result = self.catalog_refresh.as_ref().and_then(|job| match job.rx.try_recv() {
            Ok(results) => Some(Ok(results)),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(RefreshFailure::Worker)),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        let Some(result) = result else { return false; };
        let Some(job) = self.catalog_refresh.take() else { return false; };
        let joined = job.thread.join();
        if job.cancel.is_cancelled() {
            self.catalog_refresh_notice = "[catalog_cancelled] 模型刷新已取消；原模型目录未修改".into();
            return true;
        }
        let result = if joined.is_err() { Err(RefreshFailure::Worker) } else { result };
        self.catalog_refresh_notice = match result.and_then(|results| self.apply_discovery(results)) {
            Ok(notice) => notice,
            Err(error) => error.notice(),
        };
        true
    }

    fn apply_discovery(&mut self, results: Discovery) -> Result<String, RefreshFailure> {
        // Re-read on the UI thread so edits committed while HTTP was running are not overwritten.
        let (providers, presets_initialized) = self.store.load().map_err(RefreshFailure::Store)?;
        let mut candidate = SettingsFile { providers, presets_initialized };
        project(&candidate).map_err(RefreshFailure::Store)?;
        let mut failures = Vec::new();
        let mut succeeded = 0;
        let mut added = 0;
        for (config, result) in results {
            let Some(provider) = candidate.providers.iter_mut().find(|p| p.id == config.provider && p.enabled) else { continue; };
            let Some(endpoint) = provider.endpoints.iter_mut().find(|e| e.id == config.endpoint.id) else { continue; };
            if endpoint.base_url != config.endpoint.base_url || endpoint.protocol != config.endpoint.protocol {
                failures.push(RefreshFailure::Endpoint { provider: provider.name.clone(), endpoint: endpoint.name.clone(), detail: "线路配置已改变；未应用旧请求结果，请重新刷新".into() }.notice());
                continue;
            }
            match result {
                Ok(ids) => {
                    let new_ids: Vec<_> = ids.iter().filter(|id| !endpoint.models.iter().any(|model| &model.id == *id)).collect();
                    if endpoint.models.len() + new_ids.len() > 10000 {
                        failures.push(RefreshFailure::Endpoint { provider: provider.name.clone(), endpoint: endpoint.name.clone(), detail: "合并后超过 10000 个模型；该线路未修改".into() }.notice());
                        continue;
                    }
                    let selected = ids.iter().cloned().collect();
                    merge_models(&mut endpoint.models, &ids, &selected);
                    for model in &mut endpoint.models {
                        if new_ids.contains(&&model.id) { model.settings.model_type = ModelType::infer(&model.id); }
                    }
                    added += new_ids.len();
                    repair_model_selection(endpoint);
                    succeeded += 1;
                }
                Err(detail) => failures.push(RefreshFailure::Endpoint { provider: provider.name.clone(), endpoint: endpoint.name.clone(), detail }.notice()),
            }
        }
        project(&candidate).map_err(RefreshFailure::Store)?;
        if succeeded > 0 { self.store.save(&candidate.providers, candidate.presets_initialized).map_err(RefreshFailure::Store)?; }
        self.install_catalog(candidate);
        let mut notice = format!("已刷新 {succeeded} 条线路，新增 {added} 个模型；保留已有模型及自定义设置");
        if !failures.is_empty() { notice.push_str(&format!("；{} 条线路失败，原目录保留\n{}", failures.len(), failures.join("\n"))); }
        Ok(notice)
    }

    pub(super) fn cancel_catalog_refresh(&mut self) {
        if let Some(job) = self.catalog_refresh.take() {
            job.cancel.cancel();
            let _ = job.thread.join();
        }
    }
}
