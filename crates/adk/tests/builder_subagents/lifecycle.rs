use super::*;
use std::time::Duration;

struct WaitingModel {
    started: tokio::sync::Notify,
    dropped: Arc<AtomicUsize>,
}
struct ActiveCall(Arc<AtomicUsize>);
impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Model for WaitingModel {
    fn provider(&self) -> &str {
        "waiting"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        _: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            let _active = ActiveCall(self.dropped.clone());
            self.started.notify_one();
            std::future::pending().await
        })
    }
}
impl StreamingModel for WaitingModel {
    fn stream<'a>(
        &'a self,
        context: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let response = self.complete(context, request).await?;
            Ok(Box::new(Events(Some(response))) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn owned_automatic_children_stop_on_close_and_drop_with_escaped_handles() {
    for explicit_close in [false, true] {
        let child = Arc::new(WaitingModel {
            started: tokio::sync::Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        let parent = Script::new(vec![]);
        let mut c = config();
        c.roles[0].model_override = "worker/waiting".into();
        c.features.as_mut().unwrap().handoffs = false;
        c.features.as_mut().unwrap().subagents.task = true;
        let mut bundle = builder(c, &parent)
            .model("worker", Kind::Local, child.clone())
            .unwrap()
            .subagent_host(Arc::new(TestHost::default()))
            .build(&context())
            .await
            .unwrap();
        let handle = bundle.session();
        let scheduler = handle.subagents().unwrap().scheduler.clone();
        let tool = bundle
            .agent()
            .tools
            .iter()
            .find(|t| t.definition().name == "subagent")
            .unwrap()
            .clone();
        let context = ToolContext {
            operation: context(),
            work_dir: ".".into(),
            policy: ToolPolicy::default(),
            idempotency_key: None,
        };
        let call = ToolCall {
            raw_arguments: None,
            id: "background".into(),
            name: "subagent".into(),
            arguments: json!({"message":"work","mode":"background"}),
        };
        let result = tool.execute(&context, call.clone()).await.unwrap();
        assert!(!result.is_error);
        tokio::time::timeout(Duration::from_secs(5), child.started.notified())
            .await
            .unwrap();
        if explicit_close {
            bundle.close().await.unwrap();
        }
        drop(bundle);
        tokio::time::timeout(Duration::from_secs(5), async {
            while child.dropped.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(child.dropped.load(Ordering::SeqCst), 1);
        assert!(handle.is_closed());
        assert!(
            scheduler
                .submit(Submission::new("reviewer", "after close"))
                .await
                .is_err()
        );
        assert_eq!(
            tool.execute(&context, call)
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::Cancelled
        );
    }
}
