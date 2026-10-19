//! SDK-compatible trace-store documents and category files.

use serde::{Deserialize, Serialize};

pub const TRACE_SCHEMA_VERSION: u32 = 2;
pub const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunMetadata {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub candidate_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub permission_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_turns: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<String>,
    #[serde(default = "zero_time")]
    pub started_at: chrono::DateTime<chrono::FixedOffset>,
    #[serde(default = "zero_time")]
    pub finished_at: chrono::DateTime<chrono::FixedOffset>,
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}
fn zero_time() -> chrono::DateTime<chrono::FixedOffset> {
    ZERO_TIME.parse().expect("valid zero timestamp")
}

#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    pub candidate_id: String,
    pub since: Option<chrono::DateTime<chrono::FixedOffset>>,
}
impl Default for RunMetadata {
    fn default() -> Self {
        Self {
            run_id: String::new(),
            candidate_id: String::new(),
            model: String::new(),
            mode: String::new(),
            permission_mode: String::new(),
            cwd: String::new(),
            max_turns: 0,
            tools: Vec::new(),
            mcp_servers: Vec::new(),
            started_at: zero_time(),
            finished_at: zero_time(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoreMetrics {
    pub accuracy: f64,
    pub tokens_used: i64,
    pub cost_usd: f64,
    pub duration_sec: f64,
    pub tool_calls: i64,
    pub turns_used: i64,
    pub compaction_hits: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Score {
    pub task_id: String,
    pub candidate_id: String,
    pub success: bool,
    pub metrics: ScoreMetrics,
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub event_bytes: usize,
    pub append_file_bytes: u64,
    pub write_file_bytes: usize,
    pub rotations: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            event_bytes: 1 << 20,
            append_file_bytes: 64 << 20,
            write_file_bytes: 16 << 20,
            rotations: 4,
        }
    }
}

#[derive(Debug)]
pub enum StoreError {
    EventTooLarge,
    CategoryFull,
    FileTooLarge,
    Closed,
    InvalidPath,
    InvalidScore,
    Io(std::io::Error),
    Json(serde_json::Error),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventTooLarge => f.write_str("trace event exceeds per-event byte limit"),
            Self::CategoryFull => f.write_str("trace category exceeds storage quota"),
            Self::FileTooLarge => f.write_str("trace file exceeds per-file byte limit"),
            Self::Closed => f.write_str("filesystem trace store is closed"),
            Self::InvalidPath => f.write_str("unsafe trace path"),
            Self::InvalidScore => f.write_str("score metrics must be finite"),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
#[cfg(unix)]
impl From<rustix::io::Errno> for StoreError {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}

pub trait TraceStore: Send + Sync {
    fn create_run_dir(
        &self,
        run_id: &str,
        metadata: &RunMetadata,
    ) -> Result<std::path::PathBuf, StoreError>;
    fn append_trace(&self, run_id: &str, category: &str, data: &[u8]) -> Result<(), StoreError>;
    fn write_file(&self, run_id: &str, path: &str, data: &[u8]) -> Result<(), StoreError>;
    fn write_score(&self, run_id: &str, score: &Score) -> Result<(), StoreError>;
    fn list_runs(&self, filter: &RunFilter) -> Result<Vec<RunMetadata>, StoreError>;
    fn run_dir(&self, run_id: &str) -> Result<std::path::PathBuf, StoreError>;
    fn update_metadata_finished_at(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<(), StoreError>;
    fn update_metadata_mode(&self, run_id: &str, mode: &str) -> Result<(), StoreError>;
}

#[cfg(target_os = "linux")]
impl TraceStore for FilesystemTraceStore {
    fn create_run_dir(
        &self,
        run_id: &str,
        metadata: &RunMetadata,
    ) -> Result<std::path::PathBuf, StoreError> {
        self.create_run_dir(run_id, metadata)
    }
    fn append_trace(&self, run_id: &str, category: &str, data: &[u8]) -> Result<(), StoreError> {
        self.append_trace(run_id, category, data)
    }
    fn write_file(&self, run_id: &str, path: &str, data: &[u8]) -> Result<(), StoreError> {
        self.write_file(run_id, path, data)
    }
    fn write_score(&self, run_id: &str, score: &Score) -> Result<(), StoreError> {
        self.write_score(run_id, score)
    }
    fn list_runs(&self, filter: &RunFilter) -> Result<Vec<RunMetadata>, StoreError> {
        self.list_runs(filter)
    }
    fn run_dir(&self, run_id: &str) -> Result<std::path::PathBuf, StoreError> {
        self.run_dir(run_id)
    }
    fn update_metadata_finished_at(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<(), StoreError> {
        self.update_metadata_finished_at(run_id, finished_at)
    }
    fn update_metadata_mode(&self, run_id: &str, mode: &str) -> Result<(), StoreError> {
        self.update_metadata_mode(run_id, mode)
    }
}

#[cfg(target_os = "linux")]
mod filesystem {
    use super::*;
    use rustix::fs::{self, Mode, OFlags, ResolveFlags};
    use std::{
        fs::File,
        io::{Read, Write},
        os::unix::fs::MetadataExt,
        path::{Component, Path, PathBuf},
        sync::Mutex,
    };

    pub struct FilesystemTraceStore {
        root: PathBuf,
        fd: Mutex<Option<File>>,
        limits: Limits,
    }
    fn name(value: &str) -> Result<&str, StoreError> {
        let value = value.trim();
        if value.is_empty() || matches!(value, "." | "..") || value.contains(['/', '\\', '\0']) {
            return Err(StoreError::InvalidPath);
        }
        Ok(value)
    }
    fn relative(value: &str) -> Result<PathBuf, StoreError> {
        let value = value.trim();
        if value.is_empty() || value.contains(['\\', '\0']) {
            return Err(StoreError::InvalidPath);
        }
        let mut result = PathBuf::new();
        for part in Path::new(value).components() {
            match part {
                Component::Normal(part) => result.push(part),
                Component::CurDir => {}
                Component::ParentDir if result.pop() => {}
                _ => return Err(StoreError::InvalidPath),
            }
        }
        if result.as_os_str().is_empty() {
            return Err(StoreError::InvalidPath);
        }
        Ok(result)
    }
    fn open(parent: &File, path: &Path, flags: OFlags, mode: Mode) -> Result<File, StoreError> {
        Ok(File::from(fs::openat2(
            parent,
            path,
            flags | OFlags::CLOEXEC,
            mode,
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?))
    }
    fn directory(parent: &File, path: &Path, create: bool) -> Result<File, StoreError> {
        if create {
            match fs::mkdirat(parent, path, Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let fd = open(
            parent,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )?;
        if create {
            fs::fchmod(&fd, Mode::from_raw_mode(0o700))?;
        }
        Ok(fd)
    }
    fn atomic_write(parent: &File, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let (temp, mut file) = loop {
            let temp = PathBuf::from(format!(
                ".trace.tmp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match open(
                parent,
                &temp,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                Mode::from_raw_mode(0o600),
            ) {
                Ok(file) => break (temp, file),
                Err(StoreError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(error) => return Err(error),
            }
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::renameat(parent, &temp, parent, path)?;
            parent.sync_all()?;
            Ok(())
        })();
        let _ = fs::unlinkat(parent, &temp, fs::AtFlags::empty());
        result
    }
    fn read_metadata(run: &File) -> Result<RunMetadata, StoreError> {
        let mut bytes = Vec::new();
        let mut file = open(
            run,
            Path::new("metadata.json"),
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        if !file.metadata()?.is_file() {
            return Err(StoreError::InvalidPath);
        }
        file.read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    impl FilesystemTraceStore {
        pub fn new(root: impl AsRef<Path>) -> Result<Self, StoreError> {
            Self::with_limits(root, Limits::default())
        }
        pub fn with_limits(root: impl AsRef<Path>, limits: Limits) -> Result<Self, StoreError> {
            let absolute = std::path::absolute(root)?;
            let mut ancestor = absolute.as_path();
            let mut missing = Vec::new();
            let canonical = loop {
                match ancestor.canonicalize() {
                    Ok(path) => break path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing.push(
                            ancestor
                                .file_name()
                                .ok_or(StoreError::InvalidPath)?
                                .to_owned(),
                        );
                        ancestor = ancestor.parent().ok_or(StoreError::InvalidPath)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            let anchor = File::open("/")?;
            let relative = canonical.strip_prefix("/").unwrap_or(&canonical);
            let relative = if relative.as_os_str().is_empty() {
                Path::new(".")
            } else {
                relative
            };
            let mut fd = open(
                &anchor,
                relative,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?;
            let mut root = canonical;
            for part in missing.into_iter().rev() {
                fd = directory(&fd, Path::new(&part), true)?;
                root.push(part);
            }
            directory(&fd, Path::new("traces"), true)?;
            Ok(Self {
                root,
                fd: Mutex::new(Some(fd)),
                limits,
            })
        }
        pub fn close(&self) {
            self.fd.lock().expect("trace store lock poisoned").take();
        }
        pub fn create_run_dir(
            &self,
            run_id: &str,
            metadata: &RunMetadata,
        ) -> Result<PathBuf, StoreError> {
            name(run_id)?;
            let state = self.fd.lock().expect("trace store lock poisoned");
            let root = state.as_ref().ok_or(StoreError::Closed)?;
            let traces = directory(root, Path::new("traces"), false)?;
            let run = directory(&traces, Path::new(run_id), true)?;
            atomic_write(
                &run,
                Path::new("metadata.json"),
                &serde_json::to_vec_pretty(metadata)?,
            )?;
            drop(state);
            self.run_dir(run_id)
        }
        pub fn run_dir(&self, run_id: &str) -> Result<PathBuf, StoreError> {
            name(run_id)?;
            let state = self.fd.lock().expect("trace store lock poisoned");
            let root = state.as_ref().ok_or(StoreError::Closed)?;
            directory(root, &Path::new("traces").join(run_id), false)?;
            let path = self.root.join("traces").join(name(run_id)?);
            let visible = File::from(fs::open(
                &path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?)
            .metadata()?;
            let pinned = directory(root, &Path::new("traces").join(run_id), false)?.metadata()?;
            if visible.dev() != pinned.dev() || visible.ino() != pinned.ino() {
                return Err(StoreError::InvalidPath);
            }
            Ok(path)
        }
        pub fn append_trace(
            &self,
            run_id: &str,
            category: &str,
            data: &[u8],
        ) -> Result<(), StoreError> {
            name(run_id)?;
            let category = name(category)?;
            let mut data = data.to_vec();
            if !data.is_empty() && !data.ends_with(b"\n") {
                data.push(b'\n');
            }
            if data.len() > self.limits.event_bytes
                || data.len() as u64 > self.limits.append_file_bytes
            {
                return Err(StoreError::EventTooLarge);
            }
            let state = self.fd.lock().expect("trace store lock poisoned");
            let root = state.as_ref().ok_or(StoreError::Closed)?;
            let run = directory(root, &Path::new("traces").join(run_id), false)?;
            let path = PathBuf::from(format!("{category}.jsonl"));
            loop {
                let mut file = open(
                    &run,
                    &path,
                    OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE | OFlags::NONBLOCK,
                    Mode::from_raw_mode(0o600),
                )?;
                let metadata = file.metadata()?;
                if !metadata.is_file() {
                    return Err(StoreError::InvalidPath);
                }
                if metadata.nlink() != 1
                    || metadata.len() + data.len() as u64 > self.limits.append_file_bytes
                {
                    let mut rotated = false;
                    for index in 1..=self.limits.rotations {
                        let next = PathBuf::from(format!("{category}.jsonl.{index:03}"));
                        match open(
                            &run,
                            &next,
                            OFlags::RDONLY | OFlags::NONBLOCK,
                            Mode::empty(),
                        ) {
                            Ok(_) => continue,
                            Err(StoreError::Io(error))
                                if error.kind() == std::io::ErrorKind::NotFound =>
                            {
                                fs::renameat(&run, &path, &run, &next)?;
                                run.sync_all()?;
                                rotated = true;
                                break;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    if !rotated {
                        return Err(StoreError::CategoryFull);
                    }
                    continue;
                }
                file.write_all(&data)?;
                file.sync_data()?;
                if metadata.len() == 0 {
                    run.sync_all()?;
                }
                return Ok(());
            }
        }
        pub fn write_file(
            &self,
            run_id: &str,
            rel_path: &str,
            data: &[u8],
        ) -> Result<(), StoreError> {
            name(run_id)?;
            let path = relative(rel_path)?;
            if data.len() > self.limits.write_file_bytes {
                return Err(StoreError::FileTooLarge);
            }
            let state = self.fd.lock().expect("trace store lock poisoned");
            let root = state.as_ref().ok_or(StoreError::Closed)?;
            let mut parent = directory(root, &Path::new("traces").join(run_id), false)?;
            for part in path
                .parent()
                .expect("relative file has parent")
                .components()
            {
                parent = directory(&parent, Path::new(part.as_os_str()), true)?;
            }
            atomic_write(
                &parent,
                Path::new(path.file_name().expect("validated file")),
                data,
            )
        }
        pub fn write_score(&self, run_id: &str, score: &Score) -> Result<(), StoreError> {
            if ![
                score.metrics.accuracy,
                score.metrics.cost_usd,
                score.metrics.duration_sec,
            ]
            .iter()
            .all(|value| value.is_finite())
            {
                return Err(StoreError::InvalidScore);
            }
            self.write_file(run_id, "score.json", &serde_json::to_vec_pretty(score)?)
        }
        pub fn update_metadata_mode(&self, run_id: &str, mode: &str) -> Result<(), StoreError> {
            self.update_metadata(run_id, |metadata| metadata.mode = mode.into())
        }
        pub fn update_metadata_finished_at(
            &self,
            run_id: &str,
            finished_at: chrono::DateTime<chrono::FixedOffset>,
        ) -> Result<(), StoreError> {
            self.update_metadata(run_id, |metadata| metadata.finished_at = finished_at)
        }
        pub fn list_runs(&self, filter: &RunFilter) -> Result<Vec<RunMetadata>, StoreError> {
            let state = self.fd.lock().expect("trace store lock poisoned");
            let root = state.as_ref().ok_or(StoreError::Closed)?;
            let traces = directory(root, Path::new("traces"), false)?;
            let entries = fs::Dir::read_from(&traces)?;
            let mut runs = Vec::new();
            for entry in entries {
                let entry = entry?;
                let Ok(run_id) = entry.file_name().to_str() else {
                    continue;
                };
                if name(run_id).is_err() {
                    continue;
                }
                let Ok(run) = directory(&traces, Path::new(run_id), false) else {
                    continue;
                };
                let Ok(metadata) = read_metadata(&run) else {
                    continue;
                };
                if !filter.candidate_id.is_empty() && metadata.candidate_id != filter.candidate_id {
                    continue;
                }
                if filter
                    .since
                    .is_some_and(|since| metadata.started_at < since)
                {
                    continue;
                }
                runs.push(metadata);
            }
            Ok(runs)
        }
        fn update_metadata(
            &self,
            run_id: &str,
            update: impl FnOnce(&mut RunMetadata),
        ) -> Result<(), StoreError> {
            name(run_id)?;
            let state = self.fd.lock().expect("trace store lock poisoned");
            let root = state.as_ref().ok_or(StoreError::Closed)?;
            let run = directory(root, &Path::new("traces").join(run_id), false)?;
            let mut metadata = read_metadata(&run)?;
            update(&mut metadata);
            atomic_write(
                &run,
                Path::new("metadata.json"),
                &serde_json::to_vec_pretty(&metadata)?,
            )
        }
    }
}
#[cfg(target_os = "linux")]
pub use filesystem::FilesystemTraceStore;
