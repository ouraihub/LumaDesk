//! Picker refresh reads saved state; discovery only adds models and never opens the sync editor.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::{OperationGuard, TraceContext};

type Discovery = Vec<(DetectConfig, Result<Vec<String>, Failure>)>;
pub(super) struct CatalogRefreshJob {
    cancel: CancellationToken,
    thread: JoinHandle<()>,
    rx: mpsc::Receiver<Discovery>,
    trace: TraceContext,
    root: OperationGuard,
}

struct RefreshApplied {
    notice: String,
    failure: Option<Failure>,
    applied: usize,
}

fn refresh_completed(
    joined: std::thread::Result<()>,
    result: Result<Discovery, Failure>,
    cancelled: bool,
) -> Result<Discovery, Failure> {
    if joined.is_err() {
        return Err(Failure::failed(
            "SETTINGS_WORKER_PANIC",
            "模型刷新线程发生程序错误；原模型目录未修改",
        ));
    }
    if cancelled && let Ok(results) = &result {
        return Err(results
            .iter()
            .find_map(|(_, result)| {
                result
                    .as_ref()
                    .err()
                    .filter(|error| !matches!(error, Failure::Cancelled))
                    .cloned()
            })
            .unwrap_or(Failure::Cancelled));
    }
    result
}

impl ModelSettings {
    /// Reload saved configuration; force additionally discovers every enabled saved endpoint.
    pub fn refresh_catalog(&mut self, force: bool) {
        let (trace, root) = TraceContext::root("settings.catalog.refresh", "ui");
        self.catalog_diagnostic_trace = None;
        let _entered = root.span().clone().entered();
        if let Err(error) = self.reload_catalog(&trace) {
            self.catalog_refresh_notice = error.notice(&trace);
            self.catalog_diagnostic_trace = Some(trace.trace_id().to_owned());
            task::end(root, &Err::<(), _>(error));
            return;
        }
        if self.refreshing_catalog() {
            root.finish("ALREADY_RUNNING");
            return;
        }
        if !force {
            self.catalog_refresh_notice =
                format!("已重新载入本地目录：{} 个聊天模型", self.options.len());
            root.finish("LOCAL_RELOADED");
            return;
        }
        let configs: Vec<_> = self
            .saved
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .flat_map(|provider| {
                provider.endpoints.iter().map(|endpoint| DetectConfig {
                    provider: provider.id.clone(),
                    endpoint: endpoint.clone(),
                })
            })
            .collect();
        if configs.is_empty() {
            self.catalog_refresh_notice = "没有已启用的已保存线路；未请求网络".into();
            root.finish("NO_ENDPOINTS");
            return;
        }
        let count = configs.len();
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let span = root.span().clone();
        let worker_trace = trace.clone();
        let thread = std::thread::Builder::new()
            .name("model-catalog-refresh".into())
            .spawn(move || {
                let mut results = Vec::with_capacity(configs.len());
                let _entered = span.entered();
                for config in configs {
                    if worker_cancel.is_cancelled() {
                        break;
                    }
                    let result =
                        detect_models(&config.endpoint, worker_cancel.clone(), &worker_trace);
                    results.push((config, result));
                }
                if tx.send(results).is_err() {
                    tracing::warn!(
                        event = "settings.catalog.delivery.closed",
                        reason = "receiver_closed"
                    );
                }
            });
        match thread {
            Ok(thread) => {
                self.catalog_refresh = Some(CatalogRefreshJob {
                    cancel,
                    thread,
                    rx,
                    trace,
                    root,
                });
                self.catalog_refresh_notice = format!("正在刷新 {count} 条已保存线路的模型目录…");
            }
            Err(_) => {
                let error = Failure::failed(
                    "SETTINGS_WORKER_START_FAILED",
                    "无法启动模型刷新线程；原模型目录未修改",
                );
                self.catalog_diagnostic_trace = Some(trace.trace_id().to_owned());
                self.catalog_refresh_notice = error.notice(&trace);
                task::end(root, &Err::<(), _>(error));
            }
        }
    }

    /// Whether the picker catalog worker still owns an outstanding request.
    pub fn refreshing_catalog(&self) -> bool {
        self.catalog_refresh.is_some()
    }
    /// Local reload, network discovery, or typed failure notice for the picker.
    pub fn catalog_refresh_notice(&self) -> &str {
        &self.catalog_refresh_notice
    }
    /// Diagnostic identity of the latest failed refresh, for the picker copy action.
    pub fn catalog_refresh_trace_id(&self) -> Option<&str> {
        self.catalog_diagnostic_trace.as_deref()
    }

    fn reload_catalog(&mut self, trace: &TraceContext) -> Result<(), Failure> {
        let (providers, presets_initialized) = task::sync(
            trace,
            "settings.catalog.store.load",
            "SETTINGS_STORE_READ_FAILED",
            || {
                self.store
                    .load()
                    .map_err(|_| "无法读取模型目录；原模型目录未修改".into())
            },
        )?;
        let file = SettingsFile {
            providers,
            presets_initialized,
        };
        project(&file).map_err(|_| {
            Failure::failed(
                "SETTINGS_STORED_CONFIG_INVALID",
                "已保存的模型目录无效；原模型目录未修改",
            )
        })?;
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
                    let previous = self
                        .saved
                        .providers
                        .iter()
                        .find(|p| p.id == provider.id)
                        .and_then(|p| p.endpoints.iter().find(|e| e.id == endpoint.id));
                    let draft = self
                        .draft
                        .providers
                        .iter_mut()
                        .find(|p| p.id == provider.id)
                        .and_then(|p| {
                            p.endpoints.iter_mut().find(|e| {
                                e.id == endpoint.id
                                    && e.protocol == endpoint.protocol
                                    && e.base_url == endpoint.base_url
                            })
                        });
                    if let (Some(previous), Some(draft)) = (previous, draft) {
                        for model in &endpoint.models {
                            if !previous.models.iter().any(|m| m.id == model.id)
                                && !draft.models.iter().any(|m| m.id == model.id)
                            {
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
        let result = self
            .catalog_refresh
            .as_ref()
            .and_then(|job| match job.rx.try_recv() {
                Ok(results) => Some(Ok(results)),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(Failure::failed(
                    "SETTINGS_WORKER_CHANNEL_CLOSED",
                    "模型刷新结果通道异常关闭；原模型目录未修改",
                ))),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        let Some(result) = result else {
            return false;
        };
        let job = self.catalog_refresh.take().unwrap();
        let _entered = job.root.span().clone().entered();
        let result = refresh_completed(job.thread.join(), result, job.cancel.is_cancelled());
        let applied = result.and_then(|results| self.apply_discovery(results, &job.trace));
        let outcome = match &applied {
            Ok(applied) => applied.failure.clone().map_or(Ok(()), Err),
            Err(error) => Err(error.clone()),
        };
        self.catalog_diagnostic_trace = outcome
            .as_ref()
            .err()
            .map(|_| job.trace.trace_id().to_owned());
        self.catalog_refresh_notice = match &applied {
            Ok(applied) if outcome.is_ok() => applied.notice.clone(),
            Ok(applied) => format!(
                "{}\n错误码：{}\n诊断 TraceId：{}",
                applied.notice,
                applied
                    .failure
                    .as_ref()
                    .map_or("SETTINGS_CATALOG_APPLY_FAILED", Failure::code),
                job.trace.trace_id()
            ),
            Err(error) => error.notice(&job.trace),
        };
        if matches!(&applied, Ok(applied) if applied.applied == 0 && applied.failure.is_none()) {
            job.root.cancel("all_results_stale");
        } else {
            super::worker::end_worker(job.root, &outcome);
        }
        true
    }

    fn apply_discovery(
        &mut self,
        results: Discovery,
        trace: &TraceContext,
    ) -> Result<RefreshApplied, Failure> {
        // Re-read on the UI thread so committed edits are never overwritten by old HTTP results.
        let (providers, presets_initialized) = task::sync(
            trace,
            "settings.catalog.store.load",
            "SETTINGS_STORE_READ_FAILED",
            || {
                self.store
                    .load()
                    .map_err(|_| "无法读取模型目录；原模型目录未修改".into())
            },
        )?;
        let mut candidate = SettingsFile {
            providers,
            presets_initialized,
        };
        project(&candidate).map_err(|_| {
            Failure::failed(
                "SETTINGS_STORED_CONFIG_INVALID",
                "已保存的模型目录无效；原模型目录未修改",
            )
        })?;
        let mut failure = None;
        let mut failed = 0;
        let mut stale = 0;
        let mut succeeded = 0;
        let mut added = 0;
        for (config, result) in results {
            let ids = match result {
                Ok(ids) => ids,
                Err(error) => {
                    failed += 1;
                    if failure
                        .as_ref()
                        .is_none_or(|current| matches!(current, Failure::Cancelled))
                    {
                        failure = Some(error);
                    }
                    continue;
                }
            };
            let Some(provider) = candidate
                .providers
                .iter_mut()
                .find(|p| p.id == config.provider && p.enabled)
            else {
                tracing::debug!(
                    event = "settings.catalog.result.discarded",
                    reason = "provider_removed_or_disabled"
                );
                stale += 1;
                continue;
            };
            let Some(endpoint) = provider
                .endpoints
                .iter_mut()
                .find(|e| e.id == config.endpoint.id)
            else {
                tracing::debug!(
                    event = "settings.catalog.result.discarded",
                    reason = "endpoint_removed"
                );
                stale += 1;
                continue;
            };
            if endpoint.base_url != config.endpoint.base_url
                || endpoint.protocol != config.endpoint.protocol
            {
                tracing::debug!(
                    event = "settings.catalog.result.discarded",
                    reason = "endpoint_changed"
                );
                stale += 1;
                continue;
            }
            let new_ids: Vec<_> = ids
                .iter()
                .filter(|id| !endpoint.models.iter().any(|model| &model.id == *id))
                .collect();
            if endpoint.models.len() + new_ids.len() > 10000 {
                failed += 1;
                failure.get_or_insert_with(|| {
                    Failure::failed(
                        "SETTINGS_CATALOG_LIMIT",
                        "合并后超过 10000 个模型；该线路未修改",
                    )
                });
                continue;
            }
            let selected = ids.iter().cloned().collect();
            merge_models(&mut endpoint.models, &ids, &selected);
            for model in &mut endpoint.models {
                if new_ids.contains(&&model.id) {
                    model.settings.model_type = ModelType::infer(&model.id);
                }
            }
            added += new_ids.len();
            repair_model_selection(endpoint);
            succeeded += 1;
        }
        project(&candidate).map_err(|_| {
            Failure::failed(
                "SETTINGS_CATALOG_INVALID",
                "刷新后的模型目录无效；原模型目录未修改",
            )
        })?;
        if succeeded > 0 {
            task::sync(
                trace,
                "settings.catalog.store.save",
                "SETTINGS_STORE_WRITE_FAILED",
                || {
                    self.store
                        .save(&candidate.providers, candidate.presets_initialized)
                        .map_err(|_| "无法保存模型目录；原模型目录未修改".into())
                },
            )?;
        }
        self.install_catalog(candidate);
        let mut notice = if succeeded == 0 {
            format!("没有线路应用刷新结果；{stale} 条结果已失效，原模型目录未修改")
        } else {
            format!(
                "已刷新 {succeeded} 条线路，新增 {added} 个模型；保留已有模型及自定义设置；{stale} 条结果已失效"
            )
        };
        if let Some(error) = &failure {
            notice.push_str(&format!(
                "；[catalog_endpoint] {failed} 条线路失败，原目录保留：{error}"
            ));
        }
        Ok(RefreshApplied {
            notice,
            failure,
            applied: succeeded,
        })
    }

    pub(super) fn cancel_catalog_refresh(&mut self) {
        if let Some(job) = self.catalog_refresh.take() {
            let _entered = job.root.span().clone().entered();
            job.cancel.cancel();
            let joined = job.thread.join();
            let result = job.rx.recv().map_err(|_| {
                Failure::failed("SETTINGS_WORKER_CHANNEL_CLOSED", "模型刷新结果通道异常关闭")
            });
            let result = refresh_completed(joined, result, false);
            let outcome = result.and_then(|results| {
                results
                    .into_iter()
                    .find_map(|(_, result)| {
                        result
                            .err()
                            .filter(|error| !matches!(error, Failure::Cancelled))
                    })
                    .map_or(Ok(()), Err)
            });
            match outcome {
                Ok(()) | Err(Failure::Cancelled) => {
                    tracing::debug!(
                        event = "settings.catalog.result.discarded",
                        reason = "settings_closed"
                    );
                    job.root.cancel("settings_closed");
                }
                Err(error) => super::worker::end_worker(job.root, &Err::<(), _>(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests;
