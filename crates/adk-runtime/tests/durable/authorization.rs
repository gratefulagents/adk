use super::*;

struct Authorizer {
    key: Option<&'static str>,
    calls: AtomicUsize,
}
impl ActionAuthorizer for Authorizer {
    fn authorize<'a>(
        &'a self,
        _: &'a Context,
        _: ActionRequest,
    ) -> BoxFuture<'a, Result<ActionAuthorization, ActionAuthorizationFailure>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ActionAuthorization {
                decision: ACTION_DECISION_ALLOW.into(),
                ..Default::default()
            })
        })
    }
    fn durable_key(&self) -> Option<&str> {
        self.key
    }
}

#[tokio::test]
async fn action_authorizer_requires_replay_key_and_preserves_audits_across_recovery() {
    let (_, model, tool) = setup(vec![call("c")], false, false);
    let mut agent = AgentConfig::new("agent", ModelBinding::complete("model", model.clone()));
    agent.tools.push(tool.clone());
    let callback = Arc::new(Authorizer {
        key: None,
        calls: AtomicUsize::new(0),
    });
    let runner = Runner::new(
        agent.clone(),
        RunnerConfig {
            action_authorizer: Some(callback.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let error = run(&runner, Arc::new(Store::default()), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.error.info.category, ErrorCategory::Unsupported);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    assert_eq!(callback.calls.load(Ordering::SeqCst), 0);
    let callback = Arc::new(Authorizer {
        key: Some("policy/v1"),
        calls: AtomicUsize::new(0),
    });
    let runner = Runner::new(
        agent.clone(),
        RunnerConfig {
            action_authorizer: Some(callback.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let store = Arc::new(Store::default());
    *store.fail.lock().unwrap() = Some(("tool_prepared".into(), true));
    run(&runner, store.clone(), None).await.err().unwrap();
    let checkpoint = store.latest();
    let encoded = serde_json::to_value(&checkpoint).unwrap();
    assert_eq!(
        encoded["runtime"]["result"]["action_audit_records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(tool.keys.lock().unwrap().is_empty());
    let mut uncertain = checkpoint.clone();
    uncertain.effect.as_mut().unwrap().state = adk_durable::EffectState::OutcomeUnknown;
    let error = run(&runner, Arc::new(Store::default()), Some(uncertain))
        .await
        .err()
        .unwrap();
    assert!(error.error.info.message.contains("operator_resolution"));
    assert_eq!(callback.calls.load(Ordering::SeqCst), 1);
    for key in [None, Some("policy/v2")] {
        let changed = Runner::new(
            agent.clone(),
            RunnerConfig {
                action_authorizer: key.map(|key| {
                    Arc::new(Authorizer {
                        key: Some(key),
                        calls: AtomicUsize::new(0),
                    }) as Arc<dyn ActionAuthorizer>
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let error = run(
            &changed,
            Arc::new(Store::default()),
            Some(checkpoint.clone()),
        )
        .await
        .err()
        .unwrap();
        assert!(
            error
                .error
                .info
                .message
                .contains("configuration or security policy changed")
        );
        assert!(tool.keys.lock().unwrap().is_empty());
    }
    let result = run(&runner, Arc::new(Store::default()), Some(checkpoint))
        .await
        .unwrap()
        .result;
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.action_audit_records.len(), 2);
    assert_eq!(callback.calls.load(Ordering::SeqCst), 2);
    assert_eq!(tool.keys.lock().unwrap().len(), 1);
}
