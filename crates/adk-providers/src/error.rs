//! Secret-safe transport errors and bounded provider retry advice.
use adk_core::{Error, ErrorCategory, ModelRetryAdvice};
use reqwest::header::HeaderMap;
use std::time::{Duration, SystemTime};

/// No URL, response body, headers, token or underlying transport error is retained.
#[derive(Debug, Clone, thiserror::Error)]
#[error("provider request failed ({kind:?}, status {status:?})")]
pub struct RequestFailure {
    pub kind: FailureKind,
    pub status: Option<u16>,
    pub retry_after: Duration,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Http,
    Connect,
    Timeout,
    Body,
    Protocol,
}
impl RequestFailure {
    pub fn http(status: u16, headers: &HeaderMap, now: SystemTime) -> Self {
        Self {
            kind: FailureKind::Http,
            status: Some(status),
            retry_after: retry_after(headers, now),
        }
    }
    pub fn transport(error: &reqwest::Error) -> Self {
        let kind = if error.is_timeout() {
            FailureKind::Timeout
        } else if error.is_connect() {
            FailureKind::Connect
        } else if error.is_body() {
            FailureKind::Body
        } else {
            FailureKind::Protocol
        };
        Self {
            kind,
            status: None,
            retry_after: Duration::ZERO,
        }
    }
    pub fn retryable(&self) -> bool {
        match self.status {
            Some(status) => matches!(status, 429 | 500..=599),
            None => matches!(
                self.kind,
                FailureKind::Connect | FailureKind::Timeout | FailureKind::Body
            ),
        }
    }
    pub fn into_error(self) -> Error {
        Error::new(ErrorCategory::Provider, self.to_string()).with_source(self)
    }
}
pub fn retry_advice(error: &Error) -> Option<ModelRetryAdvice> {
    let failure = error.source.as_ref()?.downcast_ref::<RequestFailure>()?;
    Some(ModelRetryAdvice {
        should_retry: failure.retryable(),
        retry_after: failure.retry_after.min(Duration::from_secs(300)),
        reason: "provider transport advice".to_owned(),
    })
}
/// Go reference semantics: milliseconds are floored to seconds, with a minimum
/// of one for a positive subsecond delay. A valid positive ms header wins.
pub fn retry_after(headers: &HeaderMap, now: SystemTime) -> Duration {
    let header = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    if let Some(ms) = header("retry-after-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
    {
        return Duration::from_secs((ms / 1000).clamp(1, 300));
    }
    let Some(raw) = header("retry-after") else {
        return Duration::ZERO;
    };
    if let Ok(seconds) = raw.parse::<i64>() {
        return Duration::from_secs(seconds.clamp(0, 300) as u64);
    }
    httpdate::parse_http_date(raw)
        .ok()
        .and_then(|date| date.duration_since(now).ok())
        .map(|duration| Duration::from_secs(duration.as_secs().min(300)))
        .unwrap_or_default()
}
