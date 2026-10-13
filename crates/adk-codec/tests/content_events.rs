use adk_codec::{dto::ContentEvent, events::compact_content_event};
use serde_json::Value;

#[test]
fn typed_content_event_format_matches_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/content-event/observations.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let event: ContentEvent = serde_json::from_value(case["event"].clone()).unwrap();
        assert_eq!(
            compact_content_event(&event),
            case["compact"].as_str().unwrap()
        );
    }
}
