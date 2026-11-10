//! Model probe/sync worker ownership and UI result application.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;

fn worker_failure() -> Failure {
    Failure::failed(
        "SETTINGS_WORKER_PANIC",
        "模型请求线程发生程序错误；原模型目录未修改",
    )
}

pub(super) fn end_worker<T>(
    root: lumakit_observability::OperationGuard,
    result: &Result<T, Failure>,
) {
    if matches!(result, Err(error) if error.code() == "SETTINGS_WORKER_PANIC") {
        tracing::warn!(
            event = "settings.models.worker.panicked",
            code = "SETTINGS_WORKER_PANIC"
        );
        // RUST_PANIC belongs to the process hook; unfinished records program interruption.
        drop(root);
    } else {
        task::end(root, result);
    }
}

fn completed<T>(
    joined: std::thread::Result<()>,
    result: Result<T, Failure>,
    cancelled: bool,
) -> Result<T, Failure> {
    if joined.is_err() {
        Err(worker_failure())
    } else if result.is_ok() && cancelled {
        Err(Failure::Cancelled)
    } else {
        result
    }
}

impl ModelSettings {
    pub(super) fn start_job(&mut self, sync: bool) {
        if self.job.is_some() {
            return;
        }
        let (trace, root) = TraceContext::root(
            if sync {
                "settings.models.sync"
            } else {
                "settings.models.check"
            },
            "ui",
        );
        let _entered = root.span().clone().entered();
        let config = (|| {
            if self.dirty {
                return Err(Failure::failed(
                    "SETTINGS_UNSAVED_DRAFT",
                    "请先成功保存当前草稿，再使用已保存配置请求模型列表",
                ));
            }
            let (p, e) = self
                .indices()
                .and_then(|(p, e)| e.map(|e| (p, e)))
                .ok_or_else(|| {
                    Failure::failed("SETTINGS_TARGET_MISSING", "请先选择已保存的线路")
                })?;
            self.saved
                .providers
                .iter()
                .find(|provider| provider.id == self.draft.providers[p].id)
                .and_then(|provider| {
                    provider
                        .endpoints
                        .iter()
                        .find(|endpoint| endpoint.id == self.draft.providers[p].endpoints[e].id)
                        .map(|endpoint| DetectConfig {
                            provider: provider.id.clone(),
                            endpoint: endpoint.clone(),
                        })
                })
                .ok_or_else(|| Failure::failed("SETTINGS_TARGET_MISSING", "线路已不存在"))
        })();
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                let result = Err::<(), _>(error);
                self.report_task(&trace, &result, "");
                task::end(root, &result);
                return;
            }
        };
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let worker_config = config.clone();
        let (tx, rx) = mpsc::channel();
        let span = root.span().clone();
        let worker_trace = trace.clone();
        let thread = std::thread::Builder::new()
            .name("model-probe".into())
            .spawn(move || {
                let _entered = span.entered();
                let result = detect_models(&worker_config.endpoint, worker_cancel, &worker_trace);
                if tx.send(result).is_err() {
                    tracing::warn!(
                        event = "settings.models.delivery.closed",
                        reason = "receiver_closed"
                    );
                }
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(_) => {
                let result = Err::<(), _>(Failure::failed(
                    "SETTINGS_WORKER_START_FAILED",
                    "无法启动模型请求线程",
                ));
                self.report_task(&trace, &result, "");
                task::end(root, &result);
                return;
            }
        };
        self.job = Some(Job {
            cancel,
            thread,
            rx,
            config,
            sync,
            trace,
            root,
        });
        self.notice = "正在获取模型列表…".into();
        self.diagnostic_trace = None;
        self.error = false;
    }

    pub fn poll(&mut self) -> bool {
        let catalog_changed = self.poll_catalog_refresh();
        let result = self.job.as_ref().and_then(|job| match job.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(Failure::failed(
                "SETTINGS_WORKER_CHANNEL_CLOSED",
                "模型请求结果通道异常关闭；原模型目录未修改",
            ))),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        let Some(result) = result else {
            return catalog_changed;
        };
        let job = self.job.take().unwrap();
        let _entered = job.root.span().clone().entered();
        let result = completed(job.thread.join(), result, job.cancel.is_cancelled());
        if result.is_ok() && !self.probe_target_current(&job.config) {
            tracing::debug!(
                event = "settings.models.result.discarded",
                reason = "target_changed"
            );
            self.notice = "线路已改变；未应用旧请求结果，请重新检测".into();
            self.error = false;
            self.diagnostic_trace = None;
            job.root.cancel("target_changed");
            return true;
        }
        let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
        match result {
            Ok(ids) => {
                self.notice = format!("检测到 {} 个模型；原模型列表未修改", ids.len());
                self.error = false;
                self.diagnostic_trace = None;
                if job.sync {
                    let selected = job
                        .config
                        .endpoint
                        .models
                        .iter()
                        .map(|m| m.id.clone())
                        .collect();
                    let mut types: HashMap<String, ModelType> = ids
                        .iter()
                        .map(|id| (id.clone(), ModelType::infer(id)))
                        .collect();
                    types.extend(
                        job.config
                            .endpoint
                            .models
                            .iter()
                            .map(|model| (model.id.clone(), model.settings.model_type)),
                    );
                    self.sync_dialog = Some(SyncDialog {
                        provider: job.config.provider,
                        endpoint: job.config.endpoint.id,
                        ids,
                        selected,
                        search: String::new(),
                        types,
                        filter: None,
                        folded: HashSet::new(),
                    });
                }
            }
            Err(error) => {
                self.report_task(&job.trace, &Err::<(), _>(error), "");
            }
        }
        end_worker(job.root, &outcome);
        true
    }

    fn probe_target_current(&self, config: &DetectConfig) -> bool {
        self.saved
            .providers
            .iter()
            .find(|provider| provider.id == config.provider)
            .and_then(|provider| {
                provider
                    .endpoints
                    .iter()
                    .find(|endpoint| endpoint.id == config.endpoint.id)
            })
            .is_some_and(|endpoint| {
                endpoint.base_url == config.endpoint.base_url
                    && endpoint.protocol == config.endpoint.protocol
            })
    }

    pub(super) fn close_job(&mut self) {
        if let Some(job) = self.job.take() {
            let _entered = job.root.span().clone().entered();
            job.cancel.cancel();
            let joined = job.thread.join();
            let result = job.rx.recv().unwrap_or_else(|_| {
                Err(Failure::failed(
                    "SETTINGS_WORKER_CHANNEL_CLOSED",
                    "模型请求结果通道异常关闭",
                ))
            });
            let result = completed(joined, result, false);
            match result {
                Ok(_) | Err(Failure::Cancelled) => {
                    tracing::debug!(
                        event = "settings.models.result.discarded",
                        reason = "settings_closed"
                    );
                    job.root.cancel("settings_closed");
                }
                Err(error) => end_worker(job.root, &Err::<(), _>(error)),
            }
        }
    }

    pub(super) fn save_sync(
        &mut self,
        selected: HashSet<String>,
        types: HashMap<String, ModelType>,
    ) {
        let Some(dialog) = &self.sync_dialog else {
            return;
        };
        let (trace, root) = TraceContext::root("settings.models.apply", "ui");
        let _entered = root.span().clone().entered();
        let before = self.draft.clone();
        let result = (|| {
            let endpoint = self
                .draft
                .providers
                .iter_mut()
                .find(|p| p.id == dialog.provider)
                .and_then(|p| p.endpoints.iter_mut().find(|e| e.id == dialog.endpoint))
                .ok_or_else(|| Failure::failed("SETTINGS_TARGET_MISSING", "目标线路已不存在"))?;
            merge_models(&mut endpoint.models, &dialog.ids, &selected);
            for model in &mut endpoint.models {
                if let Some(kind) = types.get(&model.id)
                    && model.settings.model_type != *kind
                {
                    model.settings.model_type = *kind;
                    if *kind != ModelType::Chat {
                        model.settings.reasoning = None;
                        model.settings.native_web_search = false;
                        model.settings.pricing = None;
                        model.settings.supports_tools = None;
                    }
                }
            }
            repair_model_selection(endpoint);
            Ok(())
        })()
        .and_then(|()| self.save_in(&trace));
        if self.report_task(&trace, &result, "模型选择已保存") {
            if let Some(dialog) = &mut self.sync_dialog {
                dialog.selected = selected;
                dialog.types = types;
            }
        } else {
            self.draft = before;
        }
        task::end(root, &result);
    }
}

#[cfg(test)]
mod tests;
