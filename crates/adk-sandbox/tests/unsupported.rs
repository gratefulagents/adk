#![cfg(not(any(target_os = "linux", target_os = "macos")))]
use adk_core::{BoxFuture, Cancellation, Context};
use adk_sandbox::{Config, Error, Executor, Request};
use std::sync::Arc;
struct Never;
impl Cancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
#[test]
fn unsupported_platform_fails_closed() {
    let workspace = std::env::temp_dir().join(format!("adk-unsupported-{}", std::process::id()));
    std::fs::create_dir(&workspace).unwrap();
    let executor = Executor::new(Config::new(&workspace)).unwrap();
    let context = Context {
        run_id: "test".into(),
        cancellation: Arc::new(Never),
        deadline: None,
    };
    let result = executor.start(&context, Request::new(std::env::current_exe().unwrap()));
    assert!(matches!(result, Err(Error::Unavailable(_))));
    std::fs::remove_dir(workspace).unwrap();
}
