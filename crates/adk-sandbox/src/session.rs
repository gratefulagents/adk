use crate::{Error, RunResult, RunningProcess};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, mpsc, oneshot, watch};

/// Bytes since the previous poll. PTYs merge stderr into stdout.
#[derive(Debug, Default)]
pub struct SessionOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Bytes were discarded since the previous poll (including incomplete drain).
    pub truncated: bool,
    /// The supervisor has completed cleanup. Obtain status/errors with `wait`.
    pub finished: bool,
}

/// Owned interactive process. Drop requests cancellation; `cancel_and_wait`
/// waits for group cleanup and direct-child reaping. Keep the runtime alive.
/// Output is continuously drained with a combined `Config::output_limit` budget
/// between polls; slow consumers lose excess bytes, indicated by `truncated`.
/// Protocol consumers must treat truncation as a fatal framing error.
pub struct ProcessSession {
    pub(crate) running: RunningProcess,
    pub(crate) output: Arc<Output>,
    pub(crate) input: Option<SessionInput>,
    pub(crate) readiness: watch::Receiver<Option<Result<(), String>>>,
}

impl ProcessSession {
    /// Wait for backend/PTY setup and successful child spawn, not output or exit.
    /// Cancellation-safe and repeatable: dropping this future leaves supervision
    /// running; dropping the session requests cleanup and direct-child reaping.
    /// Startup failures are returned as `Error::Supervisor` diagnostics; `wait`
    /// retains the original error. A successful spawn may still exit unsuccessfully.
    pub async fn ready(&mut self) -> Result<(), Error> {
        self.readiness
            .wait_for(|result| result.is_some())
            .await
            .map_err(|_| Error::Supervisor("startup supervisor stopped without readiness".into()))?
            .as_ref()
            .expect("readiness is set")
            .clone()
            .map_err(Error::Supervisor)
    }

    /// Separate the writer so input and output can be driven concurrently.
    pub fn take_input(&mut self) -> Option<SessionInput> {
        self.input.take()
    }

    /// Drain currently buffered output without waiting. Safe to call after exit.
    pub fn poll(&mut self) -> SessionOutput {
        self.output.take()
    }

    /// Wait for bytes, truncation, or completed cleanup. Cancellation-safe.
    pub async fn next_output(&mut self) -> SessionOutput {
        loop {
            let notified = self.output.changed.notified();
            let output = self.output.take();
            if output.finished
                || output.truncated
                || !output.stdout.is_empty()
                || !output.stderr.is_empty()
            {
                return output;
            }
            notified.await;
        }
    }

    /// Request TERM/grace/KILL cleanup without waiting.
    pub fn cancel(&self) {
        self.running.cancel.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.output.finished.load(Ordering::Acquire)
    }

    /// Close any writer still owned by this handle, await cleanup, and return
    /// the unread output and exit status. Previously polled bytes are not repeated.
    pub async fn wait(self) -> Result<RunResult, Error> {
        drop(self.input);
        let mut result = self.running.wait().await?;
        let output = self.output.take();
        result.stdout = output.stdout;
        result.stderr = output.stderr;
        result.truncated = output.truncated;
        Ok(result)
    }

    pub async fn cancel_and_wait(self) -> Result<RunResult, Error> {
        self.cancel();
        self.wait().await
    }
}

/// Bounded, acknowledged input to a supervised process. Dropping closes pipe
/// stdin after queued writes; PTYs cannot half-close (send the terminal's EOF
/// character instead). The supervisor closes all descriptors on termination,
/// even if this handle outlives the session. Writes are not cancellation-safe:
/// cancelling a write future can leave a prefix written to the child.
pub struct SessionInput {
    pub(crate) sender: mpsc::Sender<InputCommand>,
}

impl SessionInput {
    pub async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        for chunk in bytes.chunks(8192) {
            let (done, ack) = oneshot::channel();
            self.sender
                .send(InputCommand {
                    bytes: chunk.to_vec(),
                    done,
                })
                .await
                .map_err(|_| closed())?;
            ack.await.map_err(|_| closed())??;
        }
        Ok(())
    }

    /// Close pipe stdin after all acknowledged writes. For PTYs this only
    /// releases the writer; the master remains open for output until cleanup.
    pub fn close(self) {}
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "process input is closed")
}

pub(crate) struct InputCommand {
    pub bytes: Vec<u8>,
    pub done: oneshot::Sender<io::Result<()>>,
}

pub(crate) struct Output {
    pub buffer: Mutex<SessionOutput>,
    limit: usize,
    changed: Notify,
    finished: AtomicBool,
}

impl Output {
    pub fn new(limit: usize) -> Self {
        Self {
            buffer: Mutex::new(SessionOutput::default()),
            limit,
            changed: Notify::new(),
            finished: AtomicBool::new(false),
        }
    }

    pub fn append(&self, bytes: &[u8], stderr: bool) {
        let mut buffer = self.buffer.lock().unwrap();
        let remaining = self
            .limit
            .saturating_sub(buffer.stdout.len() + buffer.stderr.len());
        let kept = remaining.min(bytes.len());
        buffer.truncated |= kept != bytes.len();
        if stderr {
            buffer.stderr.extend_from_slice(&bytes[..kept]);
        } else {
            buffer.stdout.extend_from_slice(&bytes[..kept]);
        }
        self.changed.notify_one();
    }

    pub fn truncate(&self) {
        self.buffer.lock().unwrap().truncated = true;
        self.changed.notify_one();
    }

    pub fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        self.changed.notify_one();
    }

    fn take(&self) -> SessionOutput {
        let mut buffer = self.buffer.lock().unwrap();
        let mut output = std::mem::take(&mut *buffer);
        output.finished = self.finished.load(Ordering::Acquire);
        output
    }
}

pub(crate) struct FinishOutput(pub Arc<Output>);
impl Drop for FinishOutput {
    fn drop(&mut self) {
        self.0.finish();
    }
}
