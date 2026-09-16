//! Secret-safe transport errors and bounded provider retry advice.
use adk_core::{Error, ErrorCategory, ModelRetryAdvice};
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::time::{Duration, SystemTime};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// No URL, response body, headers, token or underlying transport error is retained.
#[derive(Debug, Clone, thiserror::Error)]
#[error("provider request failed ({kind:?}, status {status:?}, {reason})")]
pub struct RequestFailure {
    pub kind: FailureKind,
    pub status: Option<u16>,
    pub retry_after: Duration,
    retry_override: Option<bool>,
    reason: &'static str,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Http,
    Connect,
    Timeout,
    Body,
    Protocol,
    Provider,
}
impl RequestFailure {
    pub fn http(status: u16, headers: &HeaderMap, now: SystemTime) -> Self {
        Self {
            kind: FailureKind::Http,
            status: Some(status),
            retry_after: retry_after(headers, now),
            retry_override: headers
                .get("x-should-retry")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| match value.trim() {
                    "1" | "t" | "T" | "true" | "TRUE" | "True" => Some(true),
                    "0" | "f" | "F" | "false" | "FALSE" | "False" => Some(false),
                    _ => None,
                }),
            reason: "http_error",
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
            retry_override: None,
            reason: match kind {
                FailureKind::Timeout => "timeout",
                FailureKind::Connect => "connect",
                FailureKind::Body => "body",
                _ => "protocol",
            },
        }
    }
    pub fn retryable(&self) -> bool {
        if let Some(should_retry) = self.retry_override {
            return should_retry;
        }
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
        reason: failure
            .status
            .map(|status| status.to_string())
            .unwrap_or_else(|| failure.reason.to_owned()),
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
    if let Some(raw) = header("retry-after") {
        let seconds = raw
            .parse::<i64>()
            .ok()
            .map(|seconds| seconds.max(0) as u64)
            .or_else(|| {
                httpdate::parse_http_date(raw)
                    .ok()
                    .and_then(|date| date.duration_since(now).ok())
                    .map(|duration| duration.as_secs())
            });
        if let Some(seconds) = seconds.filter(|seconds| *seconds > 0) {
            return Duration::from_secs(seconds.min(300));
        }
    }

    let now = OffsetDateTime::from(now);
    [
        "anthropic-ratelimit-unified-reset",
        "anthropic-ratelimit-requests-reset",
        "anthropic-ratelimit-input-tokens-reset",
        "anthropic-ratelimit-output-tokens-reset",
        "anthropic-ratelimit-tokens-reset",
    ]
    .into_iter()
    .filter_map(|name| {
        let raw = header(name)?;
        let reset = if name == "anthropic-ratelimit-unified-reset" {
            raw.parse::<i64>()
                .ok()
                .filter(|seconds| *seconds > 0)
                .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
                .or_else(|| OffsetDateTime::parse(raw, &Rfc3339).ok())
        } else {
            OffsetDateTime::parse(raw, &Rfc3339).ok()
        }?;
        (reset > now)
            .then(|| Duration::from_millis((reset - now).whole_milliseconds().min(300_000) as u64))
    })
    .min()
    .unwrap_or_default()
}

fn classified(status: Option<u16>, should_retry: bool, reason: &'static str) -> Error {
    RequestFailure {
        kind: FailureKind::Provider,
        status,
        retry_after: Duration::ZERO,
        retry_override: Some(should_retry),
        reason,
    }
    .into_error()
}

fn error_status(value: &Value) -> Option<u16> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|status| (400..=599).contains(status))
        .map(|status| status as u16)
}

/// Classify an explicit provider body or SSE error without retaining its payload.
/// Call `response_error` after parsing Responses output to check empty/incomplete output.
pub fn provider_error(body: &Value) -> Option<Error> {
    let event = body["type"].as_str().unwrap_or_default();
    let response = body.get("response").unwrap_or(body);
    let status = response["status"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(
        event,
        "response.failed" | "response.cancelled" | "response.canceled"
    ) || matches!(status.as_str(), "failed" | "cancelled" | "canceled")
    {
        return response_error(body, false);
    }
    if event == "response.incomplete" || status == "incomplete" {
        return None;
    }
    if let Some(error) = body.get("error").filter(|error| !error.is_null()) {
        return Some(envelope_error(error, 400));
    }
    if event == "error" {
        return Some(envelope_error(body, 400));
    }
    if let Some(choice) = body["choices"]
        .as_array()
        .and_then(|choices| choices.first())
    {
        if let Some(error) = choice.get("error").filter(|error| !error.is_null()) {
            return Some(envelope_error(error, 502));
        }
        if choice["finish_reason"]
            .as_str()
            .is_some_and(|reason| reason.trim().eq_ignore_ascii_case("error"))
        {
            return Some(classified(Some(502), true, "http_error"));
        }
    }
    None
}

fn envelope_error(error: &Value, fallback: u16) -> Error {
    let status = error_status(&error["code"]).or_else(|| error_status(&error["status"]));
    if let Some(status) = status {
        return classified(
            Some(status),
            matches!(status, 429 | 500..=599),
            "http_error",
        );
    }
    match error["type"].as_str().unwrap_or_default() {
        "rate_limit_error" => classified(Some(429), true, "rate_limit_error"),
        "overloaded_error" => classified(Some(529), true, "overloaded_error"),
        "api_error" => classified(Some(500), true, "api_error"),
        _ => classified(Some(fallback), fallback >= 500, "http_error"),
    }
}

/// Classify a Responses body or terminal SSE envelope after output parsing.
/// Usable incomplete output is successful; empty nonterminal output is retryable.
pub fn response_error(body: &Value, has_usable_output: bool) -> Option<Error> {
    let response = body.get("response").unwrap_or(body);
    let event_status = body["type"]
        .as_str()
        .unwrap_or_default()
        .strip_prefix("response.")
        .unwrap_or_default();
    let status = response["status"]
        .as_str()
        .unwrap_or(event_status)
        .trim()
        .to_ascii_lowercase();
    let failed = matches!(status.as_str(), "failed" | "cancelled" | "canceled");
    if !failed && has_usable_output {
        return None;
    }
    if !failed && status != "incomplete" {
        return Some(classified(None, true, "empty_output"));
    }
    if let Some(status) = error_status(&response["error"]["code"]) {
        return Some(classified(
            Some(status),
            matches!(status, 429 | 500..=599),
            "http_error",
        ));
    }
    let code = response["error"]["code"].as_str().unwrap_or_default();
    let incomplete = response["incomplete_details"]["reason"]
        .as_str()
        .unwrap_or_default();
    let known_code = match code.trim().to_ascii_lowercase().as_str() {
        "server_error" => Some("server_error"),
        "server_is_overloaded" => Some("server_is_overloaded"),
        "rate_limit_exceeded" => Some("rate_limit_exceeded"),
        "vector_store_timeout" => Some("vector_store_timeout"),
        _ => None,
    };
    let should_retry = known_code.is_some()
        || (status == "failed" && code.is_empty())
        || (status == "incomplete" && incomplete.is_empty());
    let reason = known_code.unwrap_or({
        if !code.is_empty() {
            match code {
                "invalid_image" => "invalid_image",
                "context_length_exceeded" => "context_length_exceeded",
                _ => "provider_error",
            }
        } else if !incomplete.is_empty() {
            match incomplete {
                "max_output_tokens" => "max_output_tokens",
                "content_filter" => "content_filter",
                _ => "incomplete",
            }
        } else {
            match status.as_str() {
                "failed" => "failed",
                "incomplete" => "incomplete",
                _ => "cancelled",
            }
        }
    });
    Some(classified(None, should_retry, reason))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRepair {
    ThinkingType,
    AdaptiveEffort,
    ReasoningEffort,
}

/// Classify only the baseline HTTP 400 healing triggers; retain no body text.
/// The caller must enforce request-shape eligibility and bounded repair attempts.
pub fn request_repair(status: u16, body: &str) -> Option<RequestRepair> {
    if status != 400 {
        return None;
    }
    let body = body.to_ascii_lowercase();
    if body.contains("thinking.type") {
        Some(RequestRepair::ThinkingType)
    } else if body.contains("reasoning") && body.contains("effort") {
        Some(RequestRepair::ReasoningEffort)
    } else if body.contains("effort") {
        Some(RequestRepair::AdaptiveEffort)
    } else {
        None
    }
}
