use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use adk_core::{Cancellation, Context, ErrorCategory};
use adk_runtime::{CancellationToken, TaskGroup};
use tokio::sync::oneshot;

struct DropCount(Arc<AtomicUsize>);

impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_is_latched_hierarchical_and_trait_object_safe() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let sibling = parent.child_token();
    child.cancel();
    assert!(!parent.is_cancelled());
    assert!(!sibling.is_cancelled());
    let signal: Arc<dyn Cancellation> = Arc::new(sibling);
    let waiter = signal.cancelled();
    tokio::pin!(waiter);
    assert!(matches!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(waiter.as_mut().poll(cx))).await,
        std::task::Poll::Pending
    ));
    parent.cancel();
    parent.cancel();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), signal.cancelled())
        .await
        .unwrap();
    assert!(parent.child_token().is_cancelled());
    let context = Context {
        run_id: "r".into(),
        cancellation: signal.clone(),
        deadline: Some(Instant::now()),
    };
    assert_eq!(
        context.check_active().unwrap_err().info.category,
        ErrorCategory::Cancelled
    );
}

#[tokio::test(flavor = "current_thread")]
async fn drop_cancels_and_aborts_even_uncooperative_tasks() {
    let parent = CancellationToken::new();
    let mut group = TaskGroup::new(&parent);
    let token = group.cancellation();
    let count = Arc::new(AtomicUsize::new(0));
    let guard = DropCount(count.clone());
    let (started, ready) = oneshot::channel();
    group.spawn(|_| async move {
        let _guard = guard;
        started.send(()).unwrap();
        pending::<()>().await;
    });
    ready.await.unwrap();
    drop(group);
    assert!(token.is_cancelled());
    assert!(!parent.is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), async {
        while count.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_joins_every_task_before_returning() {
    let parent = CancellationToken::new();
    let mut group = TaskGroup::new(&parent);
    let token = group.cancellation();
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..8 {
        let guard = DropCount(count.clone());
        group.spawn(|_| async move {
            let _guard = guard;
            pending::<()>().await;
        });
    }
    let report = tokio::time::timeout(Duration::from_secs(1), group.shutdown())
        .await
        .unwrap();
    assert_eq!(report.cancelled, 8);
    assert_eq!(report.completed, 0);
    assert!(report.panics.is_empty());
    assert_eq!(count.load(Ordering::SeqCst), 8);
    assert!(token.is_cancelled());
    assert!(!parent.is_cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_retains_panics_and_finished_tasks() {
    let mut group = TaskGroup::new(&CancellationToken::new());
    let (done, ready) = oneshot::channel();
    group.spawn(|_| async move {
        done.send(()).unwrap();
    });
    ready.await.unwrap();
    let (done, ready) = oneshot::channel();
    group.spawn(|_| async move {
        done.send(()).unwrap();
        panic!("task panic evidence");
    });
    ready.await.unwrap();
    let report = group.shutdown().await;
    assert_eq!(report.completed, 1);
    assert_eq!(report.cancelled, 0);
    assert_eq!(report.panics.len(), 1);
    assert!(report.panics[0].is_panic());
}

#[tokio::test(flavor = "current_thread")]
async fn cooperative_cancellation_can_be_joined_before_shutdown() {
    let parent = CancellationToken::new();
    let mut group = TaskGroup::new(&parent);
    group.spawn(|token| async move {
        token.cancelled().await;
    });
    parent.cancel();
    tokio::time::timeout(Duration::from_secs(1), group.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(group.join_next().await.is_none());
    let report = group.shutdown().await;
    assert_eq!(report.completed + report.cancelled + report.panics.len(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_shutdown_future_does_not_detach_tasks() {
    let mut group = TaskGroup::new(&CancellationToken::new());
    let token = group.cancellation();
    let count = Arc::new(AtomicUsize::new(0));
    let guard = DropCount(count.clone());
    group.spawn(|_| async move {
        let _guard = guard;
        pending::<()>().await;
    });
    let mut shutdown = Box::pin(group.shutdown());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(shutdown.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(shutdown);
    assert!(token.is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), async {
        while count.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
