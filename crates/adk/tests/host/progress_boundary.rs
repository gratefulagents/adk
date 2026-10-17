use super::*;
use adk::progress::ProgressSnapshot;

#[derive(Default)]
struct ProgressStatus(Mutex<Vec<ProgressSnapshot>>);

impl RunStatusSink for ProgressStatus {
    fn publish_progress<'a>(
        &'a self,
        context: &'a Context,
        snapshot: &'a ProgressSnapshot,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            assert_eq!(context.run_id, "host-test");
            self.0.lock().unwrap().push(snapshot.clone());
            Ok(())
        })
    }

    fn publish_final<'a>(
        &'a self,
        _: &'a Context,
        _: &'a RunResult,
    ) -> BoxFuture<'a, Result<(), Error>> {
        panic!("not a final-result publication")
    }
}

#[tokio::test]
async fn sdk_progress_snapshots_reach_host_sink_without_native_projection() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/progress/observations.json"
    ))
    .unwrap();
    let probe = ProgressStatus::default();
    let sink: &dyn RunStatusSink = &probe;
    let context = context();
    let mut count = 0;
    for observations in fixture["observations"].as_object().unwrap().values() {
        for observation in observations.as_array().unwrap() {
            for state in observation["states"].as_object().unwrap().values() {
                let mut expected = state["snapshot"].clone();
                for event in expected["Events"].as_array_mut().unwrap() {
                    assert_eq!(event["timestamp"], "<time>");
                    event["timestamp"] = json!("2024-01-02T03:04:05.123456789Z");
                }
                let snapshot: ProgressSnapshot = serde_json::from_value(expected.clone()).unwrap();
                sink.publish_progress(&context, &snapshot).await.unwrap();
                let captured = probe.0.lock().unwrap().pop().unwrap();
                assert_eq!(captured, snapshot);
                let wire: serde_json::Value =
                    serde_json::from_str(&serde_json::to_string(&captured).unwrap()).unwrap();
                assert_eq!(wire, expected);
                count += 1;
            }
        }
    }
    assert_eq!(count, 107);
}
