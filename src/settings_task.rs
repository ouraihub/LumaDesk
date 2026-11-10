//! Private diagnostics for cancellable settings work; never logs error messages or inputs.
use lumakit_observability::{OperationGuard, TraceContext, operation};
use std::{fmt, future::Future, time::Duration};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

#[derive(Clone, Debug)]
pub(crate) enum Failure {
    Failed { code: &'static str, message: String },
    Http(u16),
    Cancelled,
    Timeout(u64),
}
impl Failure {
    pub(crate) fn failed(code: &'static str, message: impl Into<String>) -> Self {
        Self::Failed {
            code,
            message: message.into(),
        }
    }
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Failed { code, .. } => code,
            Self::Http(401 | 403) => "SETTINGS_HTTP_UNAUTHORIZED",
            Self::Http(429) => "SETTINGS_HTTP_RATE_LIMITED",
            Self::Http(400..=499) => "SETTINGS_HTTP_REJECTED",
            Self::Http(_) => "SETTINGS_HTTP_UPSTREAM_ERROR",
            Self::Cancelled => "SETTINGS_CANCELLED",
            Self::Timeout(_) => "SETTINGS_TIMEOUT",
        }
    }
    pub(crate) fn request(error: reqwest::Error, connect_ms: u64, limit_ms: u64) -> Self {
        if error.is_timeout() {
            Self::Timeout(if error.is_connect() {
                connect_ms
            } else {
                limit_ms
            })
        } else {
            Self::failed(
                "SETTINGS_CONNECTION_FAILED",
                "连接或响应读取失败，请检查网络、地址与证书",
            )
        }
    }
    pub(crate) fn notice(&self, trace: &TraceContext) -> String {
        format!("{self}\n诊断 TraceId：{}", trace.trace_id())
    }
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::failed("SETTINGS_INVALID_RESPONSE", message)
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { message, .. } => f.write_str(message),
            Self::Http(status) => write!(
                f,
                "服务返回 HTTP {status}；未跟随重定向，请检查线路及服务状态"
            ),
            Self::Cancelled => f.write_str("请求已取消；原数据未修改"),
            Self::Timeout(_) => f.write_str("请求超时，已停止；请检查网络或稍后重试"),
        }
    }
}

pub(crate) fn end<T>(guard: OperationGuard, result: &Result<T, Failure>) {
    match result {
        Ok(_) => guard.finish("SUCCESS"),
        Err(Failure::Cancelled) => guard.cancel("user_cancelled"),
        Err(Failure::Timeout(limit)) => guard.timeout(*limit),
        Err(error) => guard.fail(error.code()),
    }
}

pub(crate) fn sync<T>(
    trace: &TraceContext,
    name: &'static str,
    code: &'static str,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, Failure> {
    let guard = operation(trace, name, "repository");
    let entered = guard.span().clone().entered();
    let result = work().map_err(|message| Failure::failed(code, message));
    drop(entered);
    end(guard, &result);
    result
}

/// Own the guard outside the selected future, so cancellation/timeout never drops it unfinished.
pub(crate) async fn http<T>(
    trace: &TraceContext,
    name: &'static str,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
    total_limit_ms: u64,
    work: impl Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    let guard = operation(trace, name, "external");
    let span = guard.span().clone();
    let result = async {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(Failure::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout(total_limit_ms)),
            result = work => result,
        }
    }
    .instrument(span)
    .await;
    end(guard, &result);
    result
}

pub(crate) fn deadline(limit_ms: u64) -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_millis(limit_ms)
}

pub(crate) fn status(status: u16) -> Failure {
    Failure::Http(status)
}

#[cfg(test)]
mod tests;
