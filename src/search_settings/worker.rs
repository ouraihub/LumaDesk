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
        if input.is_empty() || input.chars().count() > 4096 {
            let error =
                Failure::failed("SETTINGS_INVALID_INPUT", "请输入 1–4096 字符的搜索词或 URL");
            self.status = error.notice(&trace);
            self.diagnostic_trace = Some(trace.trace_id().to_owned());
            task::end(root, &Err::<(), _>(error));
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
        let span = root.span().clone();
        let worker_trace = trace.clone();
        let worker = thread::Builder::new()
            .name("search-request".into())
            .spawn(move || {
                let _entered = span.entered();
                let outcome = (|| {
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
                        if check.is_some()
                            && matches!(&result, Ok(Output::Search(value)) if value.hits.is_empty())
                        {
                            return Err(Failure::failed("SETTINGS_EMPTY_RESULT", "检测未返回结果"));
                        }
                        result
                    })
                })();
                if sender.send(outcome).is_err() {
                    tracing::debug!(
                        event = "settings.search.result.discarded",
                        reason = "receiver_closed"
                    );
                }
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
            let _completion = job.root.span().clone().entered();
            let check = job.check;
            let discard = job.discard;
            let (trace, result) = complete_job(job, None);
            if discard && matches!(&result, Ok(_) | Err(Failure::Cancelled)) {
                return;
            }
            if let Some(index) = check {
                self.card_diagnostic_trace[index] =
                    result.as_ref().err().map(|_| trace.trace_id().to_owned());
                let checked = match result {
                    Ok(Output::Search(result)) => Ok(result.hits.len()),
                    Ok(Output::Fetch(_)) => Ok(1),
                    Err(error) => Err(error.notice(&trace)),
                };
                self.card_error[index] = checked.is_err();
                self.card_status[index] =
                    checked.map_or_else(|error| error, |count| format!("检测成功：{count} 条结果"));
            } else {
                self.diagnostic_trace = result.as_ref().err().map(|_| trace.trace_id().to_owned());
                match result {
                    Ok(Output::Search(result)) => {
                        self.status = format!("搜索完成：{} 条结果", result.hits.len());
                        self.result = Some(result);
                    }
                    Ok(Output::Fetch(page)) => {
                        self.status = "读取完成".into();
                        self.page = Some(page);
                    }
                    Err(error) => self.status = error.notice(&trace),
                }
            }
        }
    }
}

/// Joining owns the terminal outcome: a panic or completed failure always beats cancellation.
pub(super) fn complete_job(
    job: Job,
    closed: Option<&'static str>,
) -> (TraceContext, Result<Output, Failure>) {
    let _entered = job.root.span().clone().entered();
    let panicked = job.thread.join().is_err();
    let result = if panicked {
        Err(Failure::failed(
            "SETTINGS_WORKER_PANICKED",
            "网络工作线程发生程序异常",
        ))
    } else {
        job.receiver.try_recv().unwrap_or_else(|_| {
            Err(Failure::failed(
                "SETTINGS_WORKER_DISCONNECTED",
                "网络工作线程未返回结果",
            ))
        })
    };
    let discard_reason = closed.or(if job.discard {
        Some("settings_hidden")
    } else {
        None
    });
    if let Some(reason) = discard_reason {
        tracing::debug!(event = "settings.search.result.discarded", reason);
    }
    let result = if result.is_ok() && (discard_reason.is_some() || job.cancel.is_cancelled()) {
        Err(Failure::Cancelled)
    } else {
        result
    };
    if panicked {
        tracing::warn!(
            event = "settings.search.worker.panicked",
            code = "SETTINGS_WORKER_PANICKED"
        );
        drop(job.root);
    } else if matches!(&result, Err(Failure::Cancelled)) {
        job.root.cancel(discard_reason.unwrap_or("user_cancelled"));
    } else if result.is_ok() {
        job.root.finish("REQUEST_COMPLETED_PENDING_PRESENTATION");
    } else {
        task::end(job.root, &result);
    }
    (job.trace, result)
}
