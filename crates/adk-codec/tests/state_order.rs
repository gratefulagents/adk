use adk_codec::replay;
use serde_json::json;

#[test]
fn ready_uses_priority_then_latest_update_then_id_like_go() {
    let tasks = [
        ("old", 1, "2000-01-01T00:00:00.000000001Z"),
        ("new-b", 1, "2000-01-01T00:00:00.1Z"),
        ("new-a", 1, "2000-01-01T00:00:00.100000000Z"),
        ("urgent", 0, "2000-01-01T00:00:00Z"),
    ];
    let events: Vec<_> = tasks.into_iter().enumerate().map(|(index, (id, priority, updated))| {
        json!({"seq": index + 1, "event_id": format!("e{index}"), "type": "task.created", "payload": {
            "id": id, "priority": priority, "status": "open", "depends_on": [],
            "created_at": "2000-01-01T00:00:00Z", "updated_at": updated
        }})
    }).collect();
    assert_eq!(
        replay("state_ready", &json!(events)).unwrap(),
        json!(["urgent", "new-a", "new-b", "old"])
    );
}
