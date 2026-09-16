use crate::{Completion, Config, Error, OutputMode, Request, RunResult, backend, policy};
use adk_core::Context;
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, unix::AsyncFd},
    process::Child,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn start(config: Config, context: Context, request: Request) -> crate::RunningProcess {
    let abandoned = CancellationToken::new();
    // The supervisor is intentionally not aborted when its caller disappears.
    let task = tokio::spawn(supervise(config, context, request, abandoned.clone(), true));
    crate::RunningProcess {
        task,
        cancel: abandoned,
    }
}

#[derive(Default)]
struct Capture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
    limit: usize,
}
impl Capture {
    fn append(&mut self, bytes: &[u8], stderr: bool) {
        let remaining = self
            .limit
            .saturating_sub(self.stdout.len() + self.stderr.len());
        let kept = remaining.min(bytes.len());
        self.truncated |= kept != bytes.len();
        if stderr {
            self.stderr.extend_from_slice(&bytes[..kept]);
        } else {
            self.stdout.extend_from_slice(&bytes[..kept]);
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
        ))
        .await?;
        if !result.status.success()
            || result.stdout != b"enforced"
            || denied.exists()
            || std::fs::read(private.0.join("home/readable"))? != b"probe"
        {
            return Err(Error::Unavailable(
                "Seatbelt functional read/write enforcement probe failed".into(),
            ));
        }
    }
    let mut built = backend::build(&config, &request)?;
    let capture = Arc::new(Mutex::new(Capture {
        limit: config.output_limit,
        ..Capture::default()
    }));
    let master = match request.output {
        OutputMode::Pipes => {
            built
                .command
                .stdin(Stdio::null())
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
            Some(master)
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
    // Command retains Stdio handles; release the PTY slave copies before reading.
    drop(built.command);
    let mut readers: Vec<JoinHandle<io::Result<()>>> = Vec::new();
    if let Some(master) = master {
        readers.push(tokio::spawn(drain_pty(master, capture.clone())));
    } else {
        let stdout = group.child.stdout.take().expect("piped stdout");
        let stderr = group.child.stderr.take().expect("piped stderr");
        readers.push(tokio::spawn(drain(stdout, capture.clone(), false)));
        readers.push(tokio::spawn(drain(stderr, capture.clone(), true)));
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
    let status = group.cleanup(config.term_grace).await;
    // Escaped setsid descendants may keep descriptors open on local/Seatbelt.
    // Bound draining even there, and never leave detached reader tasks behind.
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    let mut drain_error = None;
    for mut reader in readers {
        match tokio::time::timeout_at(drain_deadline, &mut reader).await {
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
                capture.lock().unwrap().truncated = true;
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
    let mut capture = capture.lock().unwrap();
    Ok(RunResult {
        status,
        completion,
        stdout: std::mem::take(&mut capture.stdout),
        stderr: std::mem::take(&mut capture.stderr),
        truncated: capture.truncated,
    })
}

async fn drain(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    capture: Arc<Mutex<Capture>>,
    stderr: bool,
) -> io::Result<()> {
    let mut bytes = [0u8; 8192];
    loop {
        let n = reader.read(&mut bytes).await?;
        if n == 0 {
            return Ok(());
        }
        capture.lock().unwrap().append(&bytes[..n], stderr);
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

async fn drain_pty(master: AsyncFd<File>, capture: Arc<Mutex<Capture>>) -> io::Result<()> {
    let mut bytes = [0u8; 8192];
    loop {
        let mut ready = master.readable().await?;
        match ready.try_io(|inner| inner.get_ref().read(&mut bytes)) {
            Ok(Ok(0)) => return Ok(()),
            Ok(Ok(n)) => capture.lock().unwrap().append(&bytes[..n], false),
            Ok(Err(error)) if error.raw_os_error() == Some(libc::EIO) => return Ok(()),
            Ok(Err(error)) => return Err(error),
            Err(_) => {}
        }
    }
}
