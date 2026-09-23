use adk_core::{BoxFuture, Context, Error, ErrorCategory};
use adk_runtime::{CompositeHooks, HookErrors, Observation, RunHooks};
use std::sync::{Arc, Mutex};

struct Sink {
    id: u32,
    seen: Arc<Mutex<Vec<u32>>>,
    fail: bool,
    replay_safe: bool,
}
impl RunHooks for Sink {
    fn durable_observer(&self) -> bool {
        self.replay_safe
    }
    fn observe<'a>(&'a self, _: &'a Context, _: Observation) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(self.id);
            if self.fail {
                Err(Error::new(ErrorCategory::Host, "sink failed"))
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn all_sinks_receive_ordered_events_and_failures_are_aggregated() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hooks = CompositeHooks::new((0..3).map(|id| {
        Arc::new(Sink {
            id,
            seen: seen.clone(),
            fail: id != 1,
            replay_safe: true,
        }) as Arc<dyn RunHooks>
    }));
    assert_eq!(hooks.hooks().len(), 3);
    assert!(hooks.durable_observer());
    let context = Context {
        run_id: "hooks".into(),
        cancellation: Arc::new(adk_runtime::CancellationToken::default()),
        deadline: None,
    };
    let error = hooks
        .observe(
            &context,
            Observation::AgentStarted {
                agent: "agent".into(),
                instructions: String::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(*seen.lock().unwrap(), vec![0, 1, 2]);
    assert_eq!(
        error
            .source
            .unwrap()
            .downcast::<HookErrors>()
            .unwrap()
            .errors
            .len(),
        2
    );
}

#[test]
fn durable_replay_requires_every_sink_to_opt_in() {
    assert!(CompositeHooks::default().durable_observer());
    let hooks = CompositeHooks::new([Arc::new(Sink {
        id: 0,
        seen: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        replay_safe: false,
    }) as Arc<dyn RunHooks>]);
    assert!(!hooks.durable_observer());
}
