use super::*;

#[derive(Default)]
struct Authorizer(Mutex<Vec<String>>);
impl ActionAuthorizer for Authorizer {
    fn authorize<'a>(
        &'a self,
        _: &'a Context,
        request: ActionRequest,
    ) -> BoxFuture<'a, Result<ActionAuthorization, ActionAuthorizationFailure>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request.tool_name.clone());
            Ok(ActionAuthorization {
                decision: if request.tool_name == "inspect" {
                    ACTION_DECISION_DENY
                } else {
                    ACTION_DECISION_ALLOW
                }
                .into(),
                ..Default::default()
            })
        })
    }
}

#[tokio::test]
async fn automatic_child_inherits_parent_action_authorizer() {
    let parent = Script::new(vec![
        response(vec![RunItem::ToolCall {
            call: ToolCall {
                raw_arguments: None,
                id: "delegate".into(),
                name: "subagent".into(),
                arguments: json!({"message":"review","mode":"sync"}),
            },
        }]),
        response(vec![message("parent done")]),
    ]);
    let child = Script::new(vec![
        response(vec![RunItem::ToolCall {
            call: ToolCall {
                raw_arguments: None,
                id: "inspect-call".into(),
                name: "inspect".into(),
                arguments: json!({}),
            },
        }]),
        response(vec![message("child done")]),
    ]);
    let inspect = Probe::new("inspect", true);
    let authorizer = Arc::new(Authorizer::default());
    let mut c = config();
    c.roles[0].model_override = "worker/review".into();
    c.roles[0].tool_access = "read-only".into();
    c.features.as_mut().unwrap().handoffs = false;
    c.features.as_mut().unwrap().subagents.task = true;
    c.features.as_mut().unwrap().tools = ["ExtraTools".into()].into();
    let host = Arc::new(TestHost::default());
    let mut bundle = builder(c, &parent)
        .model("worker", Kind::Local, child)
        .unwrap()
        .extra_tools([inspect.clone() as Arc<dyn Tool>])
        .subagent_host(host.clone())
        .runner_config(RunnerConfig {
            action_authorizer: Some(authorizer.clone()),
            ..Default::default()
        })
        .build(&context())
        .await
        .unwrap();
    let result = bundle.run(context(), vec![], host).await.unwrap();
    assert_eq!(result.result.final_output, Some(json!("parent done")));
    assert_eq!(*authorizer.0.lock().unwrap(), ["subagent", "inspect"]);
    assert_eq!(inspect.calls.load(Ordering::SeqCst), 0);
    assert_eq!(result.result.action_audit_records.len(), 1);
    bundle.close().await.unwrap();
}
