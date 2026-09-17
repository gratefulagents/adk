use crate::session::{FinishOutput, InputCommand, Output};
use crate::{Completion, Config, Error, OutputMode, Request, RunResult, backend, policy};
use adk_core::Context;
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, unix::AsyncFd},
    process::Child,
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn start(config: Config, context: Context, request: Request) -> crate::RunningProcess {
    let abandoned = CancellationToken::new();
    // The supervisor is intentionally not aborted when its caller disappears.
    let task = tokio::spawn(supervise(
        config,
        context,
        request,
        abandoned.clone(),
        true,
        None,
    ));
    crate::RunningProcess {
        task,
        cancel: abandoned,
    }
}

pub(crate) fn start_session(
    config: Config,
    context: Context,
    request: Request,
) -> crate::ProcessSession {
    let cancel = CancellationToken::new();
    let output = Arc::new(Output::new(config.output_limit));
    let (sender, input) = mpsc::channel(1);
    let (ready, readiness) = watch::channel(None);
    let io = SessionIo {
        output: output.clone(),
        input,
        ready: ready.clone(),
    };
    let abandoned = cancel.clone();
    let finish = FinishOutput(output.clone());
    let task = tokio::spawn(async move {
        let _finish = finish;
        let result = supervise(config, context, request, abandoned, true, Some(io)).await;
        if let Err(error) = &result {
            ready.send_if_modified(|state| {
                if state.is_some() {
                    return false;
                }
                *state = Some(Err(error.to_string()));
                true
            });
        }
        result
    });
    crate::ProcessSession {
        running: crate::RunningProcess { task, cancel },
        output,
        input: Some(crate::SessionInput { sender }),
        readiness,
    }
}

struct SessionIo {
    output: Arc<Output>,
    input: mpsc::Receiver<InputCommand>,
    ready: watch::Sender<Option<Result<(), String>>>,
}

#[derive(Default)]
struct IoTasks(Vec<JoinHandle<io::Result<()>>>);
impl Drop for IoTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

struct Group {
    child: Child,
    pid: i32,
    armed: bool,
}
impl Group {
    fn signal(&self, signal: i32) {
        // The leader is held unreaped until all group signals have been sent,
        // preventing the numeric process-group ID from being recycled.
        unsafe {
            libc::kill(-self.pid, signal);
        }
    }
    async fn cleanup(&mut self, grace: Duration) -> io::Result<std::process::ExitStatus> {
        self.signal(libc::SIGTERM);
        tokio::time::sleep(grace).await;
        self.signal(libc::SIGKILL);
        let status = self.child.wait().await?;
        self.armed = false;
        Ok(status)
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        if self.armed {
            self.signal(libc::SIGKILL);
        }
    }
}

async fn exited_unreaped(pid: i32) -> io::Result<()> {
    loop {
        {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            } else if unsafe { info.si_pid() } != 0 {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn supervise(
    config: Config,
    context: Context,
    request: Request,
    abandoned: CancellationToken,
    probe: bool,
    session: Option<SessionIo>,
) -> Result<RunResult, Error> {
    let start = Instant::now();
    let deadline = match (
        context.deadline,
        request.timeout.and_then(|t| start.checked_add(t)),
    ) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if probe
        && cfg!(target_os = "macos")
        && matches!(
            config.backend,
            crate::Backend::Auto | crate::Backend::Seatbelt
        )
    {
        let private = backend::PrivateDir::new()?;
        let workspace = private.0.join("home");
        std::fs::write(workspace.join("readable"), b"probe")?;
        let denied = private.0.join("must-not-exist");
        let mut probe_config = Config::new(&workspace);
        probe_config.backend = crate::Backend::Seatbelt;
        probe_config.term_grace = Duration::from_millis(20);
        let mut probe_request = Request::new("/bin/sh");
        probe_request.cwd = workspace;
        probe_request.args = vec!["-c".into(), "test -r readable || exit 72; if (printf x > \"$1\") 2>/dev/null; then exit 71; fi; if ln readable \"$TMPDIR/alias\" 2>/dev/null; then if (printf bad > \"$TMPDIR/alias\") 2>/dev/null; then exit 73; fi; fi; printf enforced".into(), "probe".into(), denied.to_string_lossy().into_owned()];
        probe_request.timeout = Some(Duration::from_secs(2));
        let mut probe_context = context.clone();
        probe_context.deadline = deadline;
        let result = Box::pin(supervise(
            probe_config,
            probe_context,
            probe_request,
            abandoned.clone(),
            false,
            None,
        ))
        .await?;
        if !result.status.success()
            || result.stdout != b"enforced"
            || denied.exists()
            || std::fs::read(private.0.join("home/readable"))? != b"probe"
        {
            // This is output of a fixed host-owned probe, never the requested
            // workload. Keep diagnostics bounded so native CI can distinguish
            // profile/compiler failures from a failed enforcement assertion.
            return Err(Error::Unavailable(format!(
                "Seatbelt functional read/write enforcement probe failed (status {}, completion {:?}, stderr: {})",
                result.status,
                result.completion,
                String::from_utf8_lossy(&result.stderr[..result.stderr.len().min(2048)])
            )));
        }
    }
    let mut built = backend::build(&config, &request)?;
    let interactive = session.is_some();
    let capture = session
        .as_ref()
        .map(|s| s.output.clone())
        .unwrap_or_else(|| Arc::new(Output::new(config.output_limit)));
    let master = match request.output {
        OutputMode::Pipes => {
            built
                .command
                .stdin(if interactive {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            None
        }
        OutputMode::Pty { rows, cols } => {
            let (master, slave) = pty(rows, cols)?;
            built
                .command
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(slave));
            Some(Arc::new(master))
        }
    };
    let has_pty = master.is_some();
    // Only async-signal-safe syscalls run between fork and exec.
    unsafe {
        built.command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if has_pty && libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) < 0 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(not(target_os = "linux"))]
            {
                let mut limit: libc::rlimit = std::mem::zeroed();
                if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) < 0 {
                    return Err(io::Error::last_os_error());
                }
                for fd in 3..limit.rlim_cur.min(i32::MAX as _) as i32 {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }
    policy::active(&context)?;
    if abandoned.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if deadline.is_some_and(|d| d <= Instant::now()) {
        return Err(Error::TimedOut);
    }
    let child = built.command.spawn()?;
    let mut group = Group {
        pid: child.id().expect("fresh child PID") as i32,
        child,
        armed: true,
    };
    if let Some(session) = &session {
        session.ready.send_replace(Some(Ok(())));
    }
    // Command retains Stdio handles; release the PTY slave copies before reading.
    drop(built.command);
    let mut writer = IoTasks::default();
    if let Some(session) = session {
        let input = match &master {
            Some(master) => Input::Pty(master.clone()),
            None => Input::Pipe(group.child.stdin.take().expect("piped stdin")),
        };
        writer
            .0
            .push(tokio::spawn(write_input(input, session.input)));
    }
    let mut readers = IoTasks::default();
    if let Some(master) = master {
        readers
            .0
            .push(tokio::spawn(drain_pty(master, capture.clone())));
    } else {
        let stdout = group.child.stdout.take().expect("piped stdout");
        let stderr = group.child.stderr.take().expect("piped stderr");
        readers
            .0
            .push(tokio::spawn(drain(stdout, capture.clone(), false)));
        readers
            .0
            .push(tokio::spawn(drain(stderr, capture.clone(), true)));
    }
    let timer = async {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
            None => std::future::pending::<()>().await,
        }
    };
    let mut wait_error = None;
    let completion = tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => Completion::Cancelled,
        _ = abandoned.cancelled() => Completion::Cancelled,
        _ = timer => Completion::TimedOut,
        result = exited_unreaped(group.pid) => { wait_error = result.err(); Completion::Exited }
    };
    for task in &mut writer.0 {
        task.abort();
        let _ = task.await;
    }
    let status = group.cleanup(config.term_grace).await;
    // Escaped setsid descendants may keep descriptors open on local/Seatbelt.
    // Bound draining even there, and never leave detached reader tasks behind.
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    let mut drain_error = None;
    for reader in &mut readers.0 {
        match tokio::time::timeout_at(drain_deadline, &mut *reader).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                drain_error = Some(Error::Io(error));
            }
            Ok(Err(error)) => {
                drain_error = Some(Error::Supervisor(error.to_string()));
            }
            Err(_) => {
                reader.abort();
                let _ = reader.await;
                capture.truncate();
            }
        }
    }
    let status = status?;
    if let Some(error) = wait_error {
        return Err(error.into());
    }
    if let Some(error) = drain_error {
        return Err(error);
    }
    let mut capture = capture.buffer.lock().unwrap();
    Ok(RunResult {
        status,
        completion,
        stdout: if interactive {
            Vec::new()
        } else {
            std::mem::take(&mut capture.stdout)
        },
        stderr: if interactive {
            Vec::new()
        } else {
            std::mem::take(&mut capture.stderr)
        },
        truncated: capture.truncated,
    })
}

async fn drain(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    capture: Arc<Output>,
    stderr: bool,
) -> io::Result<()> {
    let mut bytes = [0u8; 8192];
    loop {
        let n = reader.read(&mut bytes).await?;
        if n == 0 {
            return Ok(());
        }
        capture.append(&bytes[..n], stderr);
    }
}

fn pty(rows: u16, cols: u16) -> io::Result<(AsyncFd<File>, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((AsyncFd::new(master)?, slave))
}

async fn drain_pty(master: Arc<AsyncFd<File>>, capture: Arc<Output>) -> io::Result<()> {
    let mut bytes = [0u8; 8192];
    loop {
        let mut ready = master.readable().await?;
        match ready.try_io(|inner| inner.get_ref().read(&mut bytes)) {
            Ok(Ok(0)) => return Ok(()),
            Ok(Ok(n)) => capture.append(&bytes[..n], false),
            Ok(Err(error)) if error.raw_os_error() == Some(libc::EIO) => return Ok(()),
            Ok(Err(error)) => return Err(error),
            Err(_) => {}
        }
    }
}

enum Input {
    Pipe(tokio::process::ChildStdin),
    Pty(Arc<AsyncFd<File>>),
}

async fn write_input(
    mut input: Input,
    mut commands: mpsc::Receiver<InputCommand>,
) -> io::Result<()> {
    while let Some(command) = commands.recv().await {
        let result = match &mut input {
            Input::Pipe(stdin) => stdin.write_all(&command.bytes).await,
            Input::Pty(master) => write_pty(master, &command.bytes).await,
        };
        let failed = result.is_err();
        let _ = command.done.send(result);
        if failed {
            break;
        }
    }
    Ok(())
}

async fn write_pty(master: &AsyncFd<File>, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let mut ready = master.writable().await?;
        match ready.try_io(|inner| inner.get_ref().write(bytes)) {
            Ok(Ok(0)) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(Ok(n)) => bytes = &bytes[n..],
            Ok(Err(error)) => return Err(error),
            Err(_) => {}
        }
    }
    Ok(())
}
