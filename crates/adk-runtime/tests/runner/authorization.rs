use super::*;

#[derive(Default)]
struct Authorizer {
    decision: Mutex<String>,
    requests: Mutex<Vec<ActionRequest>>,
    fail: bool,
    panic: bool,
    pending: bool,
    active: AtomicUsize,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ActionAuthorizer for Authorizer {
    fn authorize<'a>(
        &'a self,
        _: &'a Context,
        request: ActionRequest,
    ) -> BoxFuture<'a, Result<ActionAuthorization, ActionAuthorizationFailure>> {
        Box::pin(async move {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.requests.lock().unwrap().push(request);
            assert!(!self.panic, "private panic payload");
            if self.pending {
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(ActionAuthorizationFailure {
                    risk: "retained-risk".into(),
                    error: Error::new(ErrorCategory::Host, "private classifier failure"),
                });
            }
            Ok(ActionAuthorization {
                decision: self.decision.lock().unwrap().clone(),
                reason: " boundary ".into(),
                rule: "custom-rule".into(),
                risk: "retained-risk".into(),
            })
        })
    }
}
fn authorizer(decision: &str) -> Arc<Authorizer> {
    Arc::new(Authorizer {
        decision: Mutex::new(decision.into()),
        ..Default::default()
    })
}
fn action_call(id: &str) -> RunItem {
    RunItem::ToolCall {
        call: ToolCall {
            id: id.into(),
            name: "write_file".into(),
            arguments: json!({"target":"fixture"}),
            raw_arguments: Some("{\"target\":\"fixture\"}".into()),
        },
    }
}
fn action_tool(approval: bool) -> Arc<TestTool> {
    let mut tool = TestTool::new("write_file", approval, false);
    Arc::get_mut(&mut tool).unwrap().definition.read_only = false;
    tool
}
fn action_request() -> RunRequest {
    let mut req = request(3);
    req.policy.tools.access = AccessMode::WorkspaceWrite;
    req.policy.tool_use = ToolUseBehavior::StopAfterTool;
    req
}

#[tokio::test]
async fn action_authorizer_decisions_preserve_approval_in_run_and_stream() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/action-authorization/observations.json"
    ))
    .unwrap();
    assert_eq!(fixture["runs"].as_array().unwrap().len(), 72);
    for streaming in [false, true] {
        for case in fixture["runs"].as_array().unwrap() {
            let callback = Arc::new(Authorizer {
                decision: Mutex::new(case["decision"].as_str().unwrap().into()),
                fail: case["failed"].as_bool().unwrap(),
                ..Default::default()
            });
            let input = case["input"].as_str().unwrap();
            let response = response(
                vec![RunItem::ToolCall {
                    call: ToolCall {
                        id: "call-1".into(),
                        name: "write_file".into(),
                        raw_arguments: Some(input.into()),
                        arguments: serde_json::from_str(input).unwrap_or(Value::Null),
                    },
                }],
                None,
            );
            let model = if streaming {
                TestModel::streaming(vec![StreamStep::Event(ModelEvent::Complete { response })])
            } else {
                TestModel::with(vec![Ok(response)])
            };
            let mut tool = action_tool(case["approval"].as_bool().unwrap());
            Arc::get_mut(&mut tool).unwrap().output.content = vec![Content::Text {
                text: "executed".into(),
            }];
            let mut a = agent(model);
            a.tools.push(tool.clone());
            let enabled = !case["disabled"].as_bool().unwrap();
            let runner = Runner::new(
                a,
                RunnerConfig {
                    action_authorizer: enabled
                        .then(|| callback.clone() as Arc<dyn ActionAuthorizer>),
                    work_dir: "/workspace/repo".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            let host = Arc::new(TestHost {
                approvals: Mutex::new([ApprovalDecision::Defer].into()),
                ..Default::default()
            });
            let outcome = if streaming {
                runner
                    .stream(context(), action_request(), host.clone())
                    .finish()
                    .await
            } else {
                runner.run(context(), action_request(), host.clone()).await
            };
            if input == "not-json" && case["interrupted"] == true {
                let error = outcome
                    .err()
                    .expect("malformed approval input must fail closed");
                assert_eq!(error.error.info.category, ErrorCategory::InvalidInput);
                assert!(
                    error
                        .error
                        .info
                        .message
                        .contains("invalid raw tool arguments")
                );
                let partial = error.partial.unwrap();
                assert_eq!(partial.status, RunStatus::Incomplete);
                assert_eq!(partial.action_audit_records[0].request.input, input);
                assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
                continue;
            }
            let outcome = outcome.unwrap();
            assert_eq!(
                tool.calls.load(Ordering::SeqCst) > 0,
                case["executed"],
                "{case}"
            );
            assert_eq!(
                outcome.result.is_interrupted(),
                case["interrupted"],
                "{case}"
            );
            let audits:Vec<_>=outcome.result.action_audit_records.iter().map(|audit|json!({"request":audit.request,"authorization":audit.authorization,"error":audit.error})).collect();
            assert_eq!(json!(audits), case["audits"], "{case}");
            assert_eq!(callback.requests.lock().unwrap().len(), audits.len());
            let output=outcome.result.new_items.iter().find_map(|item|if let RunItem::ToolResult {output,..}=item {Some(json!({"text":output.content.iter().filter_map(|content|if let Content::Text {text}=content {Some(text.as_str())} else {None}).collect::<String>(),"error":output.is_error}))} else {None});
            assert_eq!(json!(output), case["output"], "{case}");
            if outcome
                .result
                .action_audit_records
                .first()
                .is_some_and(|audit| audit.authorization.decision == ACTION_DECISION_DENY)
            {
                assert!(!host.events.lock().unwrap().iter().any(|event| matches!(
                    event,
                    RunEvent::ToolStarted { .. } | RunEvent::ApprovalRequired { .. }
                )));
            }
        }
    }
}

#[tokio::test]
async fn action_authorizer_rechecks_resumed_approval_and_clears_interruption() {
    let callback = authorizer("ask");
    let tool = action_tool(false);
    let mut a = agent(TestModel::with(vec![Ok(response(
        vec![action_call("c")],
        None,
    ))]));
    a.tools.push(tool.clone());
    let runner = Runner::new(
        a,
        RunnerConfig {
            action_authorizer: Some(callback.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let host = Arc::new(TestHost {
        approvals: Mutex::new([ApprovalDecision::Defer].into()),
        ..Default::default()
    });
    let outcome = runner.run(context(), action_request(), host).await.unwrap();
    assert!(outcome.result.is_interrupted());
    *callback.decision.lock().unwrap() = "deny".into();
    let resumed = outcome
        .continuation
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert!(!resumed.result.is_interrupted());
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        resumed
            .result
            .action_audit_records
            .iter()
            .map(|audit| audit.authorization.decision.as_str())
            .collect::<Vec<_>>(),
        ["ask", "deny"]
    );
}

#[tokio::test]
async fn action_authorizer_is_optional_and_cannot_expand_native_tool_access() {
    for enabled in [false, true] {
        for allowed in [false, true] {
            let callback = authorizer("allow");
            let tool = action_tool(false);
            let mut a = agent(TestModel::with(vec![Ok(response(
                vec![action_call("c")],
                None,
            ))]));
            a.tools.push(tool.clone());
            let runner = Runner::new(
                a,
                RunnerConfig {
                    action_authorizer: enabled
                        .then(|| callback.clone() as Arc<dyn ActionAuthorizer>),
                    ..Default::default()
                },
            )
            .unwrap();
            let mut req = action_request();
            if !allowed {
                req.policy.tools.access = AccessMode::ReadOnly;
            }
            let outcome = runner
                .run(context(), req, Arc::new(TestHost::default()))
                .await
                .unwrap();
            assert_eq!(tool.calls.load(Ordering::SeqCst), usize::from(allowed));
            assert_eq!(
                outcome.result.action_audit_records.len(),
                usize::from(enabled && allowed)
            );
            assert_eq!(
                callback.requests.lock().unwrap().len(),
                usize::from(enabled && allowed)
            );
        }
    }
}

#[tokio::test]
async fn action_authorizer_panic_and_deadline_never_execute() {
    for pending in [false, true] {
        let callback = Arc::new(Authorizer {
            panic: !pending,
            pending,
            ..Default::default()
        });
        let tool = action_tool(false);
        let mut a = agent(TestModel::with(vec![Ok(response(
            vec![action_call("c")],
            None,
        ))]));
        a.tools.push(tool.clone());
        let runner = Runner::new(
            a,
            RunnerConfig {
                action_authorizer: Some(callback.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut ctx = context();
        if pending {
            ctx.deadline = Some(Instant::now() + Duration::from_millis(30));
        }
        let result = runner
            .run(ctx, action_request(), Arc::new(TestHost::default()))
            .await;
        if pending {
            assert_eq!(
                result.err().unwrap().error.info.category,
                ErrorCategory::DeadlineExceeded
            );
        } else {
            let result = result.unwrap().result;
            assert_eq!(
                result.action_audit_records[0].authorization.rule,
                "authorizer-error"
            );
            assert!(
                !serde_json::to_string(&result)
                    .unwrap()
                    .contains("private panic payload")
            );
        }
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn action_authorizer_audits_batch_order_once_before_execution() {
    let callback = authorizer("allow");
    let tool = action_tool(false);
    let mut a = agent(TestModel::with(vec![Ok(response(
        vec![action_call("one"), action_call("two")],
        None,
    ))]));
    a.tools.push(tool.clone());
    let runner = Runner::new(
        a,
        RunnerConfig {
            action_authorizer: Some(callback.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let result = runner
        .run(context(), action_request(), Arc::new(TestHost::default()))
        .await
        .unwrap()
        .result;
    assert_eq!(result.action_audit_records.len(), 2);
    assert_eq!(callback.requests.lock().unwrap().len(), 2);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        result
            .new_items
            .iter()
            .filter_map(|item| if let RunItem::ToolResult { call_id, .. } = item {
                Some(call_id.as_str())
            } else {
                None
            })
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
}

#[tokio::test]
async fn action_authorizer_cancellation_and_owner_drop_join_pending_callback() {
    for cancel in [false, true] {
        let callback = Arc::new(Authorizer {
            pending: true,
            ..Default::default()
        });
        let tool = action_tool(false);
        let mut a = agent(TestModel::with(vec![Ok(response(
            vec![action_call("c")],
            None,
        ))]));
        a.tools.push(tool.clone());
        let runner = Runner::new(
            a,
            RunnerConfig {
                action_authorizer: Some(callback.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let token = Arc::new(CancellationToken::new());
        let ctx = Context {
            cancellation: token.clone(),
            ..context()
        };
        let mut future = Box::pin(runner.run(ctx, action_request(), Arc::new(TestHost::default())));
        tokio::select! {
            _=&mut future=>panic!("pending authorizer returned"),
            _=async {while callback.active.load(Ordering::SeqCst)==0 {tokio::task::yield_now().await;}}=>{}
        }
        if cancel {
            token.cancel();
            let error = tokio::time::timeout(Duration::from_secs(1), future)
                .await
                .unwrap()
                .err()
                .unwrap();
            assert_eq!(error.error.info.category, ErrorCategory::Cancelled);
            assert!(error.partial.unwrap().action_audit_records.is_empty());
        } else {
            drop(future);
        }
        assert_eq!(callback.active.load(Ordering::SeqCst), 0);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn action_authorizer_mixed_batch_retains_audit_and_output_order() {
    #[derive(Default)]
    struct Selective(Mutex<Vec<String>>);
    impl ActionAuthorizer for Selective {
        fn authorize<'a>(
            &'a self,
            _: &'a Context,
            request: ActionRequest,
        ) -> BoxFuture<'a, Result<ActionAuthorization, ActionAuthorizationFailure>> {
            Box::pin(async move {
                self.0.lock().unwrap().push(request.tool_name.clone());
                Ok(ActionAuthorization {
                    decision: match request.tool_name.as_str() {
                        "deny" => "deny",
                        "ask" => "ask",
                        _ => "allow",
                    }
                    .into(),
                    ..Default::default()
                })
            })
        }
    }
    let names = ["allow", "deny", "ask", "allow2"];
    let callback = Arc::new(Selective::default());
    let tools: Vec<_> = names
        .iter()
        .map(|name| TestTool::new(name, false, false))
        .collect();
    let mut a = agent(TestModel::with(vec![Ok(response(
        names.iter().map(|name| call(name, name)).collect(),
        None,
    ))]));
    a.tools = tools
        .iter()
        .cloned()
        .map(|tool| tool as Arc<dyn Tool>)
        .collect();
    let runner = Runner::new(
        a,
        RunnerConfig {
            action_authorizer: Some(callback.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let host = Arc::new(TestHost {
        approvals: Mutex::new([ApprovalDecision::Defer].into()),
        ..Default::default()
    });
    let outcome = runner.run(context(), action_request(), host).await.unwrap();
    assert_eq!(*callback.0.lock().unwrap(), names);
    assert_eq!(
        outcome
            .result
            .action_audit_records
            .iter()
            .map(|audit| audit.request.tool_name.as_str())
            .collect::<Vec<_>>(),
        names
    );
    assert_eq!(
        outcome
            .result
            .new_items
            .iter()
            .filter_map(|item| if let RunItem::ToolResult { call_id, .. } = item {
                Some(call_id.as_str())
            } else {
                None
            })
            .collect::<Vec<_>>(),
        ["allow", "deny", "allow2"]
    );
    assert_eq!(outcome.result.pending_approvals.len(), 1);
    assert_eq!(outcome.result.pending_approvals[0].call.name, "ask");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.calls.load(Ordering::SeqCst))
            .collect::<Vec<_>>(),
        [1, 0, 0, 1]
    );
    let resumed = outcome
        .continuation
        .unwrap()
        .resume(Some(ApprovalDecision::Approve))
        .await
        .unwrap();
    assert!(!resumed.result.is_interrupted());
    assert_eq!(resumed.result.action_audit_records.len(), 5);
    assert_eq!(tools[2].calls.load(Ordering::SeqCst), 1);
}
