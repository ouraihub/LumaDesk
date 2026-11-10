//! Model probe/sync worker ownership and UI result application.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;
impl ModelSettings {
    pub(super) fn start_job(&mut self, sync: bool) {
        if self.job.is_some() {
            return;
        }
        if self.dirty {
            self.report(
                Err("请先成功保存当前草稿，再使用已保存配置请求模型列表".into()),
                "",
            );
            return;
        }
        let Some((p, Some(e))) = self.indices() else {
            return;
        };
        let provider_id = &self.draft.providers[p].id;
        let endpoint_id = &self.draft.providers[p].endpoints[e].id;
        let Some(provider) = self.saved.providers.iter().find(|p| &p.id == provider_id) else {
            return;
        };
        let Some(endpoint) = provider.endpoints.iter().find(|e| &e.id == endpoint_id) else {
            return;
        };
        let config = DetectConfig {
            provider: provider.id.clone(),
            endpoint: endpoint.clone(),
        };
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let worker_config = config.clone();
        let (tx, rx) = mpsc::channel();
        let (trace, root) = TraceContext::root(
            if sync {
                "settings.models.sync"
            } else {
                "settings.models.check"
            },
            "ui",
        );
        let span = root.span().clone();
        let worker_trace = trace.clone();
        let thread = std::thread::Builder::new()
            .name("model-probe".into())
            .spawn(move || {
                let _entered = span.entered();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    detect_models(&worker_config.endpoint, worker_cancel, &worker_trace)
                }))
                .unwrap_or_else(|_| {
                    Err(Failure::failed(
                        "SETTINGS_WORKER_FAILED",
                        "模型请求线程异常结束，请重试",
                    ))
                });
                let _ = tx.send(result);
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(_) => {
                let error = Failure::failed("SETTINGS_WORKER_START_FAILED", "无法启动模型请求线程");
                self.report(Err(error.notice(&trace)), "");
                self.diagnostic_trace = Some(trace.trace_id().to_owned());
                task::end(root, &Err::<(), _>(error));
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
                "SETTINGS_WORKER_FAILED",
                "模型请求线程异常结束，请重试",
            ))),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            let _ = job.thread.join();
            let result = if job.cancel.is_cancelled() {
                Err(Failure::Cancelled)
            } else {
                result
            };
            task::end(job.root, &result);
            match result {
                Ok(ids) => {
                    self.notice = format!("检测到 {} 个模型；原模型列表未修改", ids.len());
                    self.error = false;
                    if job.sync {
                        let selected = job
                            .config
                            .endpoint
                            .models
                            .iter()
                            .map(|m| m.id.clone())
                            .collect();
                        // /models returns ids only: saved models keep their type, new ones get a name-based guess the user can change.
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
                    self.report(Err(error.notice(&job.trace)), "");
                    self.diagnostic_trace = Some(job.trace.trace_id().to_owned());
                }
            }
            return true;
        }
        catalog_changed
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
                .ok_or("目标线路已不存在")?;
            merge_models(&mut endpoint.models, &dialog.ids, &selected);
            for model in &mut endpoint.models {
                if let Some(kind) = types.get(&model.id) {
                    // A discovery round must not erase manually configured settings on unchanged types.
                    if model.settings.model_type != *kind {
                        model.settings.model_type = *kind;
                        if *kind != ModelType::Chat {
                            model.settings.reasoning = None;
                            model.settings.native_web_search = false;
                            model.settings.pricing = None;
                            model.settings.supports_tools = None;
                        }
                    }
                }
            }
            repair_model_selection(endpoint);
            Ok::<_, String>(())
        })()
        .and_then(|_| {
            task::sync(
                &trace,
                "settings.models.save",
                "SETTINGS_STORE_WRITE_FAILED",
                || self.save(),
            )
            .map_err(|error| error.to_string())
        });
        let outcome = result.map_err(|message| Failure::failed("SETTINGS_SYNC_FAILED", message));
        task::end(root, &outcome);
        let result = outcome.map_err(|error| error.notice(&trace));
        if self.report(result, "模型选择已保存") {
            if let Some(dialog) = &mut self.sync_dialog {
                dialog.selected = selected;
                dialog.types = types;
            }
        } else {
            self.draft = before;
            self.diagnostic_trace = Some(trace.trace_id().to_owned());
        }
    }
}
