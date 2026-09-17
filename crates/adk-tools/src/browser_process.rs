use super::{Execution, Request, Runner};
use adk_core::{BoxFuture, Cancellation, ToolContext};
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

/// Owns runner completions independently of tool-call futures. Trusted runners
/// must observe context cancellation and finish cleanup before returning.
/// Call `close().await` before shutting down Tokio; Drop only signals cancellation.
pub struct ManagedRunner {
    runner: Arc<dyn Runner>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    calls: Vec<Arc<Call>>,
}

struct Call {
    cancel: watch::Sender<bool>,
    completion: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

struct CancelOnDrop(watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct CallCancellation {
    parent: Arc<dyn Cancellation>,
    cancelled: watch::Receiver<bool>,
}
impl Cancellation for CallCancellation {
    fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow() || self.parent.is_cancelled()
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut cancelled = self.cancelled.clone();
            tokio::select! {
                _ = self.parent.cancelled() => {},
                _ = cancelled.wait_for(|cancelled| *cancelled) => {},
            }
        })
    }
}

impl ManagedRunner {
    pub fn new(runner: Arc<dyn Runner>) -> Self {
        Self {
            runner,
            state: Mutex::new(State::default()),
        }
    }

    /// Reject new calls and signal all retained calls, including abandoned ones.
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        for call in &state.calls {
            call.cancel.send_replace(true);
        }
    }

    /// Cancel and await every retained completion. Safe to call concurrently or
    /// retry after dropping a pending close future.
    pub async fn close(&self) -> Result<(), String> {
        self.cancel();
        let calls = self.state.lock().unwrap().calls.clone();
        let mut error = None;
        for call in calls {
            let mut completion = call.completion.lock().await;
            if let Some(task) = completion.as_mut() {
                if let Err(failure) = task.await {
                    error = Some(failure.to_string());
                }
                completion.take();
            }
        }
        self.state.lock().unwrap().calls.clear();
        error.map_or(Ok(()), Err)
    }
}

impl Drop for ManagedRunner {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Runner for ManagedRunner {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        request: Request,
    ) -> BoxFuture<'a, Result<Execution, String>> {
        Box::pin(async move {
            let (cancel, cancelled) = watch::channel(false);
            let _cancel_on_drop = CancelOnDrop(cancel.clone());
            let mut operation = context.operation.clone();
            operation.cancellation = Arc::new(CallCancellation {
                parent: operation.cancellation,
                cancelled,
            });
            let context = ToolContext {
                operation,
                work_dir: context.work_dir.clone(),
                policy: context.policy.clone(),
                idempotency_key: context.idempotency_key.clone(),
            };
            let (sender, receiver) = oneshot::channel();
            {
                let mut state = self.state.lock().unwrap();
                if state.closed {
                    return Err("browser runner is closed".into());
                }
                let runner = self.runner.clone();
                // Never select-drop the inner future: Executor::run must finish
                // awaiting its supervisor even when the tool call disappears.
                let completion = tokio::spawn(async move {
                    let result = runner.run(&context, request).await;
                    let _ = sender.send(result);
                });
                state.calls.push(Arc::new(Call {
                    cancel,
                    completion: tokio::sync::Mutex::new(Some(completion)),
                }));
            }
            receiver.await.map_err(|e| e.to_string())?
        })
    }
}
