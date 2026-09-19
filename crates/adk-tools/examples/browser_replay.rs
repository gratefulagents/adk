#[cfg(target_os = "linux")]
mod replay {
    use adk_core::*;
    use adk_tools::browser;
    use serde_json::{Value, json};
    use std::{
        io::{self, BufRead, Write},
        sync::Arc,
    };
    struct Runner {
        dom: Vec<u8>,
        exit: i32,
    }
    impl browser::Runner for Runner {
        fn run<'a>(
            &'a self,
            _: &'a ToolContext,
            request: browser::Request,
        ) -> BoxFuture<'a, Result<browser::Execution, String>> {
            Box::pin(async move {
                for path in request.writable_paths {
                    std::fs::write(path.join("screenshot.png"), b"png-data").unwrap();
                }
                Ok(browser::Execution {
                    output: self.dom.clone(),
                    exit_code: self.exit,
                    timed_out: false,
                })
            })
        }
    }
    pub async fn run() {
        for line in io::stdin().lock().lines() {
            let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let root = tempfile::tempdir().unwrap();
            let dom = input["dom"]
                .as_str()
                .unwrap_or("")
                .repeat(input["repeat"].as_u64().unwrap_or(1) as usize);
            let tool = browser::tool(browser::Config {
                runner: Arc::new(Runner {
                    dom: dom.into_bytes(),
                    exit: input["exit"].as_i64().unwrap_or(0) as i32,
                }),
                executable: Some("/usr/bin/chromium".into()),
                screenshot_dir: root.path().into(),
                access: if input["read_only"].as_bool().unwrap_or(false) {
                    AccessMode::ReadOnly
                } else {
                    AccessMode::WorkspaceWrite
                },
                allow_private_network_urls: input["private"].as_bool().unwrap_or(true),
            });
            let ctx = ToolContext {
                operation: Context {
                    run_id: "fixture".into(),
                    cancellation: Arc::new(adk_runtime::CancellationToken::new()),
                    deadline: None,
                },
                work_dir: root.path().into(),
                policy: Default::default(),
                idempotency_key: None,
            };
            let output = tool
                .execute(
                    &ctx,
                    ToolCall {
                        id: "b".into(),
                        name: "Browser".into(),
                        arguments: input["arguments"].clone(),
                    },
                )
                .await
                .unwrap();
            let image = std::fs::read_to_string(root.path().join("sub/test.png")).ok();
            println!("{}", json!({"result":output,"image":image}));
            io::stdout().flush().unwrap();
        }
    }
}
#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    replay::run().await
}
#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("confined replay requires Linux");
}
