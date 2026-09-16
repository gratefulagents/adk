use adk_core::{Error, ErrorCategory};
use adk_providers::error::{
    RequestFailure, RequestRepair, provider_error, request_repair, response_error, retry_advice,
    retry_after,
};
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

fn headers(values: &[(&'static str, &str)]) -> HeaderMap {
    values
        .iter()
        .map(|(key, value)| {
            (
                key.parse::<reqwest::header::HeaderName>().unwrap(),
                value.parse().unwrap(),
            )
        })
        .collect()
}

fn assert_advice(error: Error, retry: bool, reason: &str) {
    assert_eq!(error.info.category, ErrorCategory::Provider);
    let advice = retry_advice(&error).unwrap();
    assert_eq!(advice.should_retry, retry);
    assert_eq!(advice.reason, reason);
    assert_eq!(advice.retry_after, Duration::ZERO);
    let failure = error
        .source
        .as_ref()
        .unwrap()
        .downcast_ref::<RequestFailure>()
        .unwrap();
    assert_eq!(failure.retryable(), retry);
}

#[test]
fn http_status_and_explicit_retry_hints_match_baseline() {
    for status in [400, 401, 403, 404, 408, 409, 422, 429, 500, 502, 503, 529] {
        for (hint, expected) in [
            ("false", false),
            ("true", true),
            ("invalid", status == 429 || status >= 500),
        ] {
            let error = RequestFailure::http(
                status,
                &headers(&[("x-should-retry", hint)]),
                SystemTime::UNIX_EPOCH,
            )
            .into_error();
            assert_advice(error, expected, &status.to_string());
        }
    }
    for hint in ["0", "f", "F", "false", "FALSE", "False", " false "] {
        assert!(
            !RequestFailure::http(
                429,
                &headers(&[("x-should-retry", hint)]),
                SystemTime::UNIX_EPOCH
            )
            .retryable()
        );
    }
    for hint in ["1", "t", "T", "true", "TRUE", "True"] {
        assert!(
            RequestFailure::http(
                408,
                &headers(&[("x-should-retry", hint)]),
                SystemTime::UNIX_EPOCH
            )
            .retryable()
        );
    }
}

#[test]
fn retry_delay_precedence_floor_dates_and_caps() {
    let now = SystemTime::UNIX_EPOCH;
    let mut h = headers(&[
        ("retry-after", "7"),
        ("anthropic-ratelimit-unified-reset", "120"),
    ]);
    assert_eq!(retry_after(&h, now), Duration::from_secs(7));
    for (raw, seconds) in [
        ("1", 1),
        ("999", 1),
        ("1999", 1),
        ("2999", 2),
        ("18446744073709551615", 300),
    ] {
        h.insert("retry-after-ms", raw.parse().unwrap());
        assert_eq!(retry_after(&h, now), Duration::from_secs(seconds));
    }
    for raw in ["0", "-1", "invalid"] {
        h.insert("retry-after-ms", raw.parse().unwrap());
        assert_eq!(retry_after(&h, now), Duration::from_secs(7));
    }
    h.remove("retry-after-ms");
    for (raw, seconds) in [
        ("999999", 300),
        ("Thu, 01 Jan 1970 00:01:00 GMT", 60),
        ("Thu, 01 Jan 1970 01:00:00 GMT", 300),
        ("0", 120),
        ("-1", 120),
        ("invalid", 120),
        ("Thu, 01 Jan 1970 00:00:00 GMT", 120),
    ] {
        h.insert("retry-after", raw.parse().unwrap());
        assert_eq!(retry_after(&h, now), Duration::from_secs(seconds));
    }
}

#[test]
fn earliest_future_reset_uses_all_baseline_headers_and_millisecond_precision() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(60);
    for key in [
        "anthropic-ratelimit-unified-reset",
        "anthropic-ratelimit-requests-reset",
        "anthropic-ratelimit-input-tokens-reset",
        "anthropic-ratelimit-output-tokens-reset",
        "anthropic-ratelimit-tokens-reset",
    ] {
        let h = headers(&[(key, "1970-01-01T00:01:07.250Z")]);
        assert_eq!(retry_after(&h, now), Duration::from_millis(7250));
        assert_eq!(
            retry_after(&headers(&[(key, "1970-01-01T01:00:00Z")]), now),
            Duration::from_secs(300)
        );
    }
    let h = headers(&[
        ("anthropic-ratelimit-unified-reset", "180"),
        ("anthropic-ratelimit-requests-reset", "1970-01-01T00:00:30Z"),
        (
            "anthropic-ratelimit-input-tokens-reset",
            "1970-01-01T00:01:00Z",
        ),
        ("anthropic-ratelimit-output-tokens-reset", "invalid"),
        (
            "anthropic-ratelimit-tokens-reset",
            "1970-01-01T01:01:07+01:00",
        ),
    ]);
    assert_eq!(retry_after(&h, now), Duration::from_secs(7));
    for raw in ["-1", "0", "60", "invalid", "9223372036854775807"] {
        assert_eq!(
            retry_after(&headers(&[("anthropic-ratelimit-unified-reset", raw)]), now),
            Duration::ZERO
        );
    }
    assert_eq!(
        retry_after(
            &headers(&[("anthropic-ratelimit-requests-reset", "180")]),
            now
        ),
        Duration::ZERO
    );
}

#[test]
fn chat_error_envelopes_preserve_numeric_and_string_status() {
    for status in [400, 401, 429, 503] {
        for code in [json!(status), json!(status.to_string())] {
            for body in [
                json!({"error":{"code":code,"message":"provider rate limited"}}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"error","error":{"code":code,"message":"all providers failed","metadata":{"provider_name":"upstream"}}}]}),
            ] {
                assert_advice(
                    provider_error(&body).unwrap(),
                    status == 429 || status >= 500,
                    &status.to_string(),
                );
            }
        }
    }
    for code in [
        Value::Null,
        json!("unknown"),
        json!(399),
        json!(600),
        json!(429.5),
        json!({"secret":"payload"}),
    ] {
        assert_advice(
            provider_error(&json!({"error":{"code":code}})).unwrap(),
            false,
            "400",
        );
        assert_advice(
            provider_error(&json!({"choices":[{"error":{"code":code}}]})).unwrap(),
            true,
            "502",
        );
    }
    assert_advice(
        provider_error(&json!({"choices":[{"finish_reason":" ERROR "}]})).unwrap(),
        true,
        "502",
    );
}

#[test]
fn anthropic_stream_errors_are_typed_and_sanitized() {
    for (kind, retry, reason) in [
        ("authentication_error", false, "400"),
        ("invalid_request_error", false, "400"),
        ("rate_limit_error", true, "429"),
        ("overloaded_error", true, "529"),
        ("api_error", true, "500"),
    ] {
        assert_advice(
            provider_error(
                &json!({"type":"error","error":{"type":kind,"message":"private payload"}}),
            )
            .unwrap(),
            retry,
            reason,
        );
    }
}

#[test]
fn responses_failed_and_incomplete_follow_reference_classification() {
    for (status, code, incomplete, retry, reason) in [
        ("failed", "server_error", "", true, "server_error"),
        (
            "failed",
            "server_is_overloaded",
            "",
            true,
            "server_is_overloaded",
        ),
        (
            "failed",
            "rate_limit_exceeded",
            "",
            true,
            "rate_limit_exceeded",
        ),
        (
            "failed",
            "vector_store_timeout",
            "",
            true,
            "vector_store_timeout",
        ),
        ("failed", " SERVER_ERROR ", "", true, "server_error"),
        ("failed", "", "", true, "failed"),
        ("failed", "invalid_image", "", false, "invalid_image"),
        ("failed", "unknown secret", "", false, "provider_error"),
        ("incomplete", "", "", true, "incomplete"),
        ("incomplete", "", "content_filter", false, "content_filter"),
        (
            "incomplete",
            "",
            "max_output_tokens",
            false,
            "max_output_tokens",
        ),
        ("cancelled", "", "", false, "cancelled"),
        ("canceled", "", "", false, "cancelled"),
    ] {
        let body = json!({"status":status,"error":{"code":code,"message":"upstream worker crashed"},"incomplete_details":{"reason":incomplete}});
        assert_advice(response_error(&body, false).unwrap(), retry, reason);
        let event = json!({"type":format!("response.{status}"),"response":body});
        assert_advice(response_error(&event, false).unwrap(), retry, reason);
        if status == "failed" {
            assert_advice(provider_error(&event).unwrap(), retry, reason);
        }
    }
    assert_advice(
        provider_error(&json!({"type":"response.failed","response":{}})).unwrap(),
        true,
        "failed",
    );
}

#[test]
fn usable_incomplete_output_succeeds_and_empty_completed_output_retries() {
    let partial = json!({"type":"response.incomplete","response":{"status":"incomplete","output":[{"type":"message","content":[{"type":"output_text","text":"partial answer"}]}],"incomplete_details":{"reason":"max_output_tokens"}}});
    assert!(provider_error(&partial).is_none());
    assert!(response_error(&partial, true).is_none());
    let incomplete_with_error = json!({"status":"incomplete","error":{"code":"server_error"}});
    assert!(provider_error(&incomplete_with_error).is_none());
    assert!(response_error(&incomplete_with_error, true).is_none());
    assert_advice(
        response_error(&partial, false).unwrap(),
        false,
        "max_output_tokens",
    );
    let empty = json!({"status":"completed","output":[]});
    assert_advice(response_error(&empty, false).unwrap(), true, "empty_output");
    assert!(response_error(&empty, true).is_none());
    assert_advice(
        response_error(
            &json!({"status":"failed","error":{"code":"invalid_image"}}),
            true,
        )
        .unwrap(),
        false,
        "invalid_image",
    );
    for body in [
        json!({"choices":[{"finish_reason":"stop","message":{"content":"ok"}}]}),
        json!({"type":"content_block_delta"}),
        json!({"error":null}),
        json!({"type":"response.completed","response":empty}),
    ] {
        assert!(provider_error(&body).is_none());
    }
}

#[test]
fn response_numeric_codes_and_context_reasons_remain_safe_and_actionable() {
    for status in [429, 503] {
        for code in [json!(status), json!(status.to_string())] {
            assert_advice(
                provider_error(&json!({"status":"failed","error":{"code":code}})).unwrap(),
                true,
                &status.to_string(),
            );
        }
    }
    let error = provider_error(&json!({"status":" FAILED ","error":{"code":"context_length_exceeded","message":"private request content"}})).unwrap();
    assert!(error.info.message.contains("context_length_exceeded"));
    assert_advice(error, false, "context_length_exceeded");
    let error = RequestFailure::http(
        429,
        &headers(&[("x-should-retry", "false"), ("retry-after", "999999")]),
        SystemTime::UNIX_EPOCH,
    )
    .into_error();
    let advice = retry_advice(&error).unwrap();
    assert!(!advice.should_retry);
    assert_eq!(advice.retry_after, Duration::from_secs(300));
    assert_eq!(advice.reason, "429");
}

#[test]
fn arbitrary_payloads_are_not_retained_in_diagnostics_or_advice() {
    let secret = "private-token-and-user-content";
    for body in [
        json!({"error":{"code":secret,"type":secret,"message":secret,"metadata":{"provider_name":secret}}}),
        json!({"type":"response.failed","response":{"id":secret,"status":"failed","error":{"code":secret,"message":secret}}}),
    ] {
        let error = provider_error(&body).unwrap();
        let rendered = format!(
            "{error} {error:?} {:?} {:?}",
            error.source,
            retry_advice(&error)
        );
        assert!(!rendered.contains(secret), "{rendered}");
    }
    let error = RequestFailure::http(
        429,
        &headers(&[
            ("authorization", secret),
            ("retry-after", secret),
            ("x-should-retry", secret),
            ("anthropic-ratelimit-unified-status", secret),
        ]),
        SystemTime::UNIX_EPOCH,
    )
    .into_error();
    assert!(!format!("{error:?} {:?}", retry_advice(&error)).contains(secret));
    assert!(retry_advice(&Error::new(ErrorCategory::Provider, secret)).is_none());
}

#[test]
fn repair_classifier_uses_only_baseline_400_triggers() {
    for (body, expected) in [
        (
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"\"thinking.type.enabled\" is not supported for this model. Use \"thinking.type.adaptive\" and \"output_config.effort\" to control thinking behavior."}}"#,
            Some(RequestRepair::ThinkingType),
        ),
        (
            "output_config: effort: Input should be 'low', 'medium' or 'high'",
            Some(RequestRepair::AdaptiveEffort),
        ),
        (
            "Invalid reasoning effort: not supported for this model",
            Some(RequestRepair::ReasoningEffort),
        ),
        ("max_tokens is too large", None),
    ] {
        assert_eq!(request_repair(400, body), expected);
        assert_eq!(request_repair(400, &body.to_uppercase()), expected);
        for status in [401, 429, 500] {
            assert_eq!(request_repair(status, body), None);
        }
    }
}
