//! Search/read worker ownership and result presentation.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;
impl SearchSettings {
    pub(super) fn launch(
        &mut self,
        fetch: bool,
        check: Option<usize>,
        context: Option<(TraceContext, lumakit_observability::OperationGuard)>,
    ) {
        if self.is_busy() {
            return;
        }
        let index = check.unwrap_or(if fetch {
            self.fetch_index
        } else {
            self.search_index
        });
        let input = if check.is_some() {
            if fetch {
                "https://example.com".to_owned()
            } else {
                "web search".to_owned()
            }
        } else if fetch {
            self.target.trim().to_owned()
        } else {
            self.query.trim().to_owned()
        };
        if input.is_empty() || input.chars().count() > 4096 {
            self.status = "请输入 1–4096 字符的搜索词或 URL".into();
            return;
        }
        let config = self.stored.configs[index].clone();
        let mut prefs = self.stored.preferences.clone();
        if self.stored.compression == Compression::None {
            prefs.cutoff = usize::MAX;
        }
        if check.is_some() {
            self.card_diagnostic_trace[index] = None;
            self.card_status[index] = "正在检测…".into();
            self.card_error[index] = false;
        } else {
            self.diagnostic_trace = None;
            self.status = if fetch {
                "正在读取…"
            } else {
                "正在搜索…"
            }
            .into();
            if fetch {
                self.page = None;
            } else {
                self.result = None;
            }
        }
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let (trace, root) = context.unwrap_or_else(|| {
            TraceContext::root(
                if check.is_some() {
                    "settings.search.check"
                } else if fetch {
                    "settings.search.read"
                } else {
                    "settings.search.query"
                },
                "ui",
            )
        });
        let span = root.span().clone();
        let worker_trace = trace.clone();
        let worker = thread::Builder::new()
            .name("search-request".into())
            .spawn(move || {
                let _entered = span.entered();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| {
                            Failure::failed("SETTINGS_RUNTIME_FAILED", "无法创建网络运行时")
                        })?;
                    rt.block_on(async {
                        let result = if fetch {
                            fetch_page(config, input, prefs, worker_cancel.clone(), &worker_trace)
                                .await
                                .map(Output::Fetch)
                        } else {
                            search_query(config, input, prefs, worker_cancel.clone(), &worker_trace)
                                .await
                                .map(Output::Search)
                        };
                        if worker_cancel.is_cancelled() {
                            return Err(Failure::Cancelled);
                        }
                        if check.is_some()
                            && matches!(&result, Ok(Output::Search(value)) if value.hits.is_empty())
                        {
                            return Err(Failure::failed("SETTINGS_EMPTY_RESULT", "检测未返回结果"));
                        }
                        result
                    })
                }))
                .unwrap_or_else(|_| {
                    Err(Failure::failed(
                        "SETTINGS_WORKER_FAILED",
                        "网络工作线程异常结束",
                    ))
                });
                let _ = sender.send(outcome);
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(_) => {
                let error = Failure::failed("SETTINGS_WORKER_START_FAILED", "无法启动网络请求线程");
                if let Some(index) = check {
                    self.card_status[index] = error.notice(&trace);
                    self.card_error[index] = true;
                    self.card_diagnostic_trace[index] = Some(trace.trace_id().to_owned());
                } else {
                    self.status = error.notice(&trace);
                    self.diagnostic_trace = Some(trace.trace_id().to_owned());
                }
                task::end(root, &Err::<(), _>(error));
                return;
            }
        };
        self.job = Some(Job {
            cancel,
            thread: worker,
            receiver,
            discard: false,
            check,
            trace,
            root,
        });
    }
    pub(super) fn drain(&mut self) {
        if !self
            .job
            .as_ref()
            .is_some_and(|job| job.thread.is_finished())
        {
            return;
        }
        if let Some(job) = self.job.take() {
            let result = job.receiver.try_recv().unwrap_or_else(|_| {
                Err(Failure::failed(
                    "SETTINGS_WORKER_FAILED",
                    "网络工作线程异常结束",
                ))
            });
            let _ = job.thread.join();
            let result = if job.discard || job.cancel.is_cancelled() {
                Err(Failure::Cancelled)
            } else {
                result
            };
            task::end(job.root, &result);
            if job.discard {
                return;
            }
            if let Some(index) = job.check {
                self.card_diagnostic_trace[index] = result
                    .as_ref()
                    .err()
                    .map(|_| job.trace.trace_id().to_owned());
                let checked = match result {
                    Ok(Output::Search(result)) => Ok(result.hits.len()),
                    Ok(Output::Fetch(_)) => Ok(1),
                    Err(error) => Err(error.notice(&job.trace)),
                };
                self.card_error[index] = checked.is_err();
                self.card_status[index] =
                    checked.map_or_else(|error| error, |count| format!("检测成功：{count} 条结果"));
            } else {
                self.diagnostic_trace = result
                    .as_ref()
                    .err()
                    .map(|_| job.trace.trace_id().to_owned());
                match result {
                    Ok(Output::Search(result)) => {
                        self.status = format!("搜索完成：{} 条结果", result.hits.len());
                        self.result = Some(result);
                    }
                    Ok(Output::Fetch(page)) => {
                        self.status = "读取完成".into();
                        self.page = Some(page);
                    }
                    Err(error) => self.status = error.notice(&job.trace),
                }
            }
        }
    }
}
