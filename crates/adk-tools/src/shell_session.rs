use super::buffer::{Buffer, terminal_limit};
use adk_core::{BoxFuture, Cancellation, Context, ToolContext};
use adk_sandbox::{Completion, Executor, ProcessSession, Request, SessionInput};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

pub(super) struct Signal(watch::Sender<bool>);
impl Signal {
    fn new() -> Self {
        Self(watch::channel(false).0)
    }
    fn cancel(&self) {
        self.0.send_replace(true);
    }
}
impl Cancellation for Signal {
    fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut rx = self.0.subscribe();
            let _ = rx.wait_for(|cancelled| *cancelled).await;
        })
    }
}
struct Combined {
    owner: Arc<Signal>,
    job: Arc<Signal>,
    caller: Arc<dyn Cancellation>,
}
impl Cancellation for Combined {
    fn is_cancelled(&self) -> bool {
        self.owner.is_cancelled() || self.job.is_cancelled() || self.caller.is_cancelled()
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::select! { _ = self.owner.cancelled() => {}, _ = self.job.cancelled() => {}, _ = self.caller.cancelled() => {} }
        })
    }
}
struct Jobs {
    bash_id: u64,
    terminal_id: u64,
    sync_id: u64,
    jobs: BTreeMap<String, Arc<Job>>,
}
pub(super) struct Manager {
    jobs: Arc<Mutex<Jobs>>,
    cancel: Arc<Signal>,
}
impl Manager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(Jobs {
                bash_id: 0,
                terminal_id: 0,
                sync_id: 0,
                jobs: BTreeMap::new(),
            })),
            cancel: Arc::new(Signal::new()),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        executor: &Executor,
        context: &ToolContext,
        request: Request,
        description: &str,
        cap: usize,
        terminal: bool,
        asynchronous: bool,
    ) -> Result<Arc<Job>, String> {
        let cancel = Arc::new(Signal::new());
        let operation = Context {
            run_id: context.operation.run_id.clone(),
            cancellation: Arc::new(Combined {
                owner: self.cancel.clone(),
                job: cancel.clone(),
                caller: context.operation.cancellation.clone(),
            }),
            deadline: context.operation.deadline,
        };
        let timeout = request.timeout.unwrap_or_default();
        let (job, ready) =
            {
                // Serialize registration with close: no untracked session may escape teardown.
                let mut jobs = self.jobs.lock().unwrap();
                if self.cancel.is_cancelled() {
                    return Err("shell bundle is closed".into());
                }
                let mut session = executor
                    .start_session(&operation, request)
                    .map_err(|e| format!("Error: {e}"))?;
                let input = session.take_input();
                let input = if terminal {
                    input
                } else {
                    drop(input);
                    None
                };
                let counter = if terminal {
                    &mut jobs.terminal_id
                } else if asynchronous {
                    &mut jobs.bash_id
                } else {
                    &mut jobs.sync_id
                };
                *counter += 1;
                let id = format!(
                    "{}-{}",
                    if terminal {
                        "term"
                    } else if asynchronous {
                        "bash"
                    } else {
                        "sync"
                    },
                    counter
                );
                jobs.jobs.retain(|_, job| {
                    !job.data.lock().unwrap().ended.is_some_and(|end| {
                        !job.terminal && end.elapsed() > Duration::from_secs(1800)
                    })
                });
                let job = Arc::new(Job {
                    id: id.clone(),
                    terminal,
                    asynchronous,
                    cancel,
                    finished: Signal::new(),
                    input: AsyncMutex::new(input),
                    started: Instant::now(),
                    started_at: timestamp(),
                    timeout,
                    description: description.into(),
                    data: Mutex::new(Data {
                        buffer: Buffer::new(if terminal { 256 * 1024 } else { cap }, terminal),
                        exit_code: -1,
                        timed_out: false,
                        error: None,
                        blocked: None,
                        screen_tail: Vec::new(),
                        ended: None,
                        ended_at: None,
                    }),
                });
                jobs.jobs.insert(id, job.clone());
                let monitored = job.clone();
                let map = self.jobs.clone();
                let (ready, readiness) = oneshot::channel();
                tokio::spawn(async move {
                    let result = session.ready().await;
                    let failed = result.is_err();
                    let _ = ready.send(result);
                    monitored.collect(session).await;
                    if !asynchronous || failed {
                        map.lock().unwrap().jobs.remove(&monitored.id);
                    }
                });
                (job, readiness)
            };
        let mut guard = CancelOnDrop(Some(job.clone()));
        ready
            .await
            .map_err(|e| format!("Error: startup monitor failed: {e}"))?
            .map_err(|e| format!("Error: {e}"))?;
        guard.0.take();
        Ok(job)
    }
    pub fn get(&self, id: &str, terminal: bool) -> Option<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap()
            .jobs
            .get(id)
            .filter(|s| s.terminal == terminal && s.asynchronous)
            .cloned()
    }
    pub fn list(&self, terminal: bool) -> Vec<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap()
            .jobs
            .values()
            .filter(|s| s.terminal == terminal && s.asynchronous)
            .cloned()
            .collect()
    }
    pub fn cancel(&self) {
        let _guard = self.jobs.lock().unwrap();
        self.cancel.cancel();
    }
    pub async fn close(&self) {
        self.cancel();
        let jobs: Vec<_> = self.jobs.lock().unwrap().jobs.values().cloned().collect();
        for job in jobs {
            job.done().await;
        }
    }
}
pub(super) struct Job {
    pub id: String,
    terminal: bool,
    asynchronous: bool,
    cancel: Arc<Signal>,
    finished: Signal,
    input: AsyncMutex<Option<SessionInput>>,
    started: Instant,
    started_at: String,
    timeout: Duration,
    description: String,
    data: Mutex<Data>,
}
struct Data {
    buffer: Buffer,
    exit_code: i32,
    timed_out: bool,
    error: Option<String>,
    blocked: Option<String>,
    screen_tail: Vec<u8>,
    ended: Option<Instant>,
    ended_at: Option<String>,
}
pub(super) struct CancelOnDrop(pub Option<Arc<Job>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(job) = &self.0 {
            job.cancel.cancel();
        }
    }
}
impl Job {
    async fn collect(&self, mut session: ProcessSession) {
        loop {
            let out = session.next_output().await;
            {
                let mut data = self.data.lock().unwrap();
                self.append(&mut data, &out.stdout, out.truncated);
                self.append(&mut data, &out.stderr, false);
            }
            if out.finished {
                break;
            }
        }
        let result = session.wait().await;
        // Drop input only after supervision has closed all descriptors; a blocked
        // concurrent PTY write is then guaranteed to wake and release this lock.
        self.input.lock().await.take();
        let mut data = self.data.lock().unwrap();
        match result {
            Ok(result) => {
                self.append(&mut data, &result.stdout, result.truncated);
                self.append(&mut data, &result.stderr, false);
                data.timed_out = result.completion == Completion::TimedOut;
                data.exit_code = if data.timed_out {
                    -1
                } else {
                    result.status.code().unwrap_or(-1)
                };
            }
            Err(adk_sandbox::Error::Cancelled) => {}
            Err(adk_sandbox::Error::TimedOut) => data.timed_out = true,
            Err(error) => data.error = Some(error.to_string()),
        }
        self.screen(&mut data);
        data.ended = Some(Instant::now());
        data.ended_at = Some(timestamp());
        self.finished.cancel();
    }
    pub async fn done(&self) {
        self.finished.cancelled().await;
    }
    pub async fn kill(&self) {
        self.cancel.cancel();
        self.done().await;
    }
    pub async fn wait_for(&self, context: &ToolContext, millis: u64) {
        if millis == 0 {
            return;
        }
        let wait = Duration::from_millis(millis).min(
            context
                .operation
                .deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::MAX),
        );
        tokio::select! { _ = self.done() => {}, _ = context.operation.cancellation.cancelled() => {}, _ = tokio::time::sleep(wait) => {} }
    }
    pub async fn send(&self, context: &ToolContext, bytes: &[u8]) -> Result<(), String> {
        if self.finished.is_cancelled() {
            return Err("session has exited".into());
        }
        let write = async {
            self.input
                .lock()
                .await
                .as_mut()
                .ok_or("session has exited")?
                .write_all(bytes)
                .await
                .map_err(|e| e.to_string())
        };
        let deadline = async {
            match context.operation.deadline {
                Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            result = write => return result,
            _ = context.operation.cancellation.cancelled() => {},
            _ = deadline => {},
        }
        self.kill().await;
        Err("terminal input interrupted".into())
    }
    pub fn startup_error(&self) -> Option<String> {
        self.data.lock().unwrap().error.clone()
    }
    fn elapsed(&self, data: &Data) -> u64 {
        data.ended
            .unwrap_or_else(Instant::now)
            .duration_since(self.started)
            .as_millis() as u64
    }
    fn append(&self, data: &mut Data, bytes: &[u8], lost: bool) {
        if data.blocked.is_some() {
            return;
        }
        let mut accumulated = std::mem::take(&mut data.screen_tail);
        accumulated.extend_from_slice(bytes);
        if let Err(error) = adk_security::check_secrets(&String::from_utf8_lossy(&accumulated)) {
            data.blocked = Some(error.to_string());
            self.cancel.cancel();
            return;
        }
        let discard = accumulated.len().saturating_sub(data.buffer.cap);
        accumulated.drain(..discard);
        data.screen_tail = accumulated;
        data.buffer.append(bytes, lost);
        self.screen(data);
    }
    fn screen(&self, data: &mut Data) {
        if data.blocked.is_none()
            && let Err(error) = adk_security::check_secrets(&data.buffer.full())
                .and_then(|()| adk_security::check_secrets(&self.description))
        {
            data.blocked = Some(error.to_string());
            self.cancel.cancel();
        }
    }
    pub fn bash_result(&self) -> Result<String, String> {
        let mut data = self.data.lock().unwrap();
        self.screen(&mut data);
        if let Some(error) = &data.blocked {
            return Err(error.clone());
        }
        let mut output = data.buffer.full();
        if data.buffer.total == 0 {
            output.clear();
        }
        if data.timed_out {
            output += "\n[command timed out]";
        } else if let Some(error) = &data.error {
            return Err(format!("{output}\nError: {error}"));
        } else if data.exit_code != 0 {
            output += &format!("\nExit code: {}", data.exit_code);
        } else if output.is_empty() {
            output = "(no output)".into();
        }
        Ok(output)
    }
    pub fn bash_snapshot(&self, incremental: bool) -> Result<Value, String> {
        let mut data = self.data.lock().unwrap();
        self.screen(&mut data);
        if let Some(error) = &data.blocked {
            return Err(error.clone());
        }
        let running = data.ended.is_none();
        let status = if running {
            "running"
        } else if data.timed_out {
            "timed_out"
        } else if data.error.is_some() {
            "error"
        } else {
            "exited"
        };
        let (mut output, gap) = if incremental {
            data.buffer.consume()
        } else {
            (data.buffer.full(), false)
        };
        if incremental && output.is_empty() {
            output = "(no new output since last poll)".into();
        }
        let mut snap = json!({"id":self.id,"status":status,"running":running,"exit_code":data.exit_code,"timed_out":data.timed_out,"started_at":self.started_at,"elapsed_ms":self.elapsed(&data),"timeout_ms":self.timeout.as_millis() as u64,"output":output});
        if let Some(error) = &data.error {
            snap["error"] = json!(error);
        }
        if let Some(ended) = &data.ended_at {
            snap["ended_at"] = json!(ended);
        }
        if !self.description.is_empty() {
            snap["description"] = json!(self.description);
        }
        if gap {
            snap["note"] =
                json!("some output between polls exceeded the retention cap and was discarded");
        }
        Ok(snap)
    }
    pub fn terminal_snapshot(&self, consume: bool) -> Result<Value, String> {
        let mut data = self.data.lock().unwrap();
        self.screen(&mut data);
        if let Some(error) = &data.blocked {
            return Err(error.clone());
        }
        let (output, gap) = if consume {
            data.buffer.consume()
        } else {
            (String::new(), false)
        };
        let mut snap = json!({"session_id":self.id,"status":if data.ended.is_some() {"exited"} else {"running"},"elapsed_ms":self.elapsed(&data)});
        let mut note = if gap {
            "older scrollback before this output was discarded".to_owned()
        } else {
            String::new()
        };
        if output.is_empty() {
            note +=
                " no new output since last read; wait and read again if a command is still running";
        } else {
            snap["output"] = json!(terminal_limit(output));
        }
        if !note.is_empty() {
            snap["note"] = json!(note.trim());
        }
        Ok(snap)
    }
}
fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Gregorian civil date from Unix days, avoiding locale/ambient timezone state.
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use adk_core::{AccessMode, ToolPolicy};
    use adk_sandbox::{Backend, Config, Network};
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn start_rejects_existing_nonexecutable_and_missing_interpreter_fixtures() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::new(dir.path());
        config.backend = Backend::Local;
        config.term_grace = Duration::ZERO;
        let executor = Executor::new(config).unwrap();
        let context = ToolContext {
            operation: Context {
                run_id: "spawn-test".into(),
                cancellation: Arc::new(Signal::new()),
                deadline: None,
            },
            work_dir: dir.path().into(),
            policy: ToolPolicy {
                access: AccessMode::FullAccess,
                ..Default::default()
            },
            idempotency_key: None,
        };
        let manager = Manager::new();
        for executable in [false, true] {
            let program = dir.path().join("program");
            std::fs::write(&program, b"#!/adk-missing-interpreter\n").unwrap();
            std::fs::set_permissions(
                &program,
                std::fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
            )
            .unwrap();
            assert!(program.is_file());
            let mut request = Request::new(&program);
            request.access = AccessMode::FullAccess;
            request.network = Network::Allow;
            request.cwd = dir.path().into();
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                manager.start(
                    &executor,
                    &context,
                    request,
                    "spawn fixture",
                    4096,
                    false,
                    true,
                ),
            )
            .await
            .unwrap();
            let Err(error) = result else {
                panic!("spawn failure must not return a job ID")
            };
            assert!(error.contains("subprocess I/O"), "{error}");
        }
        manager.close().await;
        assert!(manager.list(false).is_empty());
    }

    #[tokio::test]
    async fn abandoned_startup_is_cancelled_and_reaped_by_close() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::new(dir.path());
        config.backend = Backend::Local;
        config.term_grace = Duration::from_millis(20);
        let executor = Executor::new(config).unwrap();
        let context = ToolContext {
            operation: Context {
                run_id: "abandoned-start-test".into(),
                cancellation: Arc::new(Signal::new()),
                deadline: None,
            },
            work_dir: dir.path().into(),
            policy: ToolPolicy {
                access: AccessMode::FullAccess,
                ..Default::default()
            },
            idempotency_key: None,
        };
        let manager = Manager::new();
        let mut request = Request::new("/bin/sh");
        request.args = vec![
            "-c".into(),
            "echo $$ > leader; trap '' TERM; while :; do sleep 1; done".into(),
        ];
        request.access = AccessMode::FullAccess;
        request.network = Network::Allow;
        request.cwd = dir.path().into();
        let mut start = Box::pin(manager.start(
            &executor,
            &context,
            request,
            "abandoned startup",
            4096,
            false,
            true,
        ));
        poll_fn(|cx| {
            assert!(start.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(dir.path().join("leader"))
                    && let Ok(pid) = pid.trim().parse::<u32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        drop(start);
        tokio::time::timeout(Duration::from_secs(2), manager.close())
            .await
            .unwrap();
        let mut check = Request::new("/bin/sh");
        check.args = vec!["-c".into(), format!("kill -0 {pid} 2>/dev/null")];
        check.access = AccessMode::FullAccess;
        check.network = Network::Allow;
        assert!(
            !executor
                .run(&context.operation, check)
                .await
                .unwrap()
                .status
                .success()
        );
    }
}
