use crate::{Error, Event, Result, SCHEMA_VERSION, engine::State, recall::EmbeddingRecord};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Default)]
pub struct StoreOptions {
    pub project_id: String,
    pub work_dir: PathBuf,
    pub actor: String,
    pub run_id: String,
}
#[derive(Debug, Clone)]
pub struct FilesystemOptions {
    pub state_dir: PathBuf,
    pub store: StoreOptions,
    pub lock_timeout: Duration,
}
impl Default for FilesystemOptions {
    fn default() -> Self {
        Self {
            state_dir: PathBuf::new(),
            store: StoreOptions::default(),
            lock_timeout: Duration::from_secs(30),
        }
    }
}
#[derive(Debug, Clone)]
pub struct SQLiteOptions {
    pub path: PathBuf,
    pub table_prefix: String,
    pub store: StoreOptions,
}
impl Default for SQLiteOptions {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            table_prefix: "projectstate_".into(),
            store: StoreOptions::default(),
        }
    }
}
pub fn sanitize_project_id(value: &str) -> String {
    let mut out = String::new();
    for c in value.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').into()
}
pub fn derive_project_id(work_dir: &str) -> String {
    let path = if work_dir.trim().is_empty() {
        "project"
    } else {
        work_dir.trim()
    };
    let base = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != ".")
        .unwrap_or("project");
    format!(
        "{}-{}",
        sanitize_project_id(base),
        &format!("{:x}", Sha1::digest(path.as_bytes()))[..8]
    )
}
pub fn default_state_dir(project_id: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Invalid("HOME is required".into()))?;
    let id = sanitize_project_id(project_id);
    if id.is_empty() {
        return Err(Error::Invalid("project id is required".into()));
    }
    Ok(PathBuf::from(home)
        .join(".gratefulagents/projects")
        .join(id)
        .join("state"))
}
pub(crate) fn normalize_options(mut options: StoreOptions) -> Result<StoreOptions> {
    if !options.work_dir.as_os_str().is_empty() {
        options.work_dir = std::path::absolute(&options.work_dir)?;
    }
    options.project_id = sanitize_project_id(&options.project_id);
    if options.project_id.is_empty() {
        options.project_id = derive_project_id(&options.work_dir.to_string_lossy());
    }
    options.actor = options.actor.trim().into();
    options.run_id = options.run_id.trim().into();
    Ok(options)
}
fn reject_symlinks(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::Invalid(format!(
                    "symlink not allowed: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    // Windows cannot open directories with File::open; only Unix supports this directory fsync.
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    reject_symlinks(path)?;
    let mut missing = Vec::new();
    let mut parent = path;
    while !parent.exists() {
        missing.push(parent.to_path_buf());
        parent = parent
            .parent()
            .ok_or_else(|| Error::Invalid("invalid state directory".into()))?;
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(Error::Invalid("state path is not a directory".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    for dir in missing.iter().rev() {
        sync_directory(dir)?;
        sync_directory(dir.parent().unwrap())?;
    }
    Ok(())
}
fn check_regular(file: &File) -> Result<()> {
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid("state file is not regular".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
fn private_open(path: &Path, append: bool, exclusive: bool) -> Result<File> {
    reject_symlinks(path)?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Err(Error::Invalid("state file is not regular".into()));
        }
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(true)
        .append(append)
        .create_new(exclusive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    check_regular(&file)?;
    Ok(file)
}
fn read_private(path: &Path) -> Result<Vec<u8>> {
    reject_symlinks(path)?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Err(Error::Invalid("state file is not regular".into()));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    check_regular(&file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    reject_symlinks(path)?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Err(Error::Invalid("state file is not regular".into()));
        }
    }
    let tmp = path.with_extension(crate::id("tmp"));
    let result = (|| {
        let mut f = private_open(&tmp, false, true)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        sync_directory(path.parent().unwrap())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
struct Lock {
    path: PathBuf,
    token: String,
    _gate: File,
}
impl Lock {
    fn acquire(dir: &Path, timeout: Duration) -> Result<Self> {
        let path = dir.join("locks/state.lock");
        let token = crate::id("lock");
        let start = Instant::now();
        let gate = private_open(&dir.join("locks/rust-state.lock"), false, false)?;
        loop {
            match fs4::FileExt::try_lock(&gate) {
                Ok(()) => break,
                Err(fs4::TryLockError::WouldBlock) => {
                    if start.elapsed() >= timeout {
                        return Err(Error::LockTimeout);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(fs4::TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        loop {
            match private_open(&path, false, true) {
                Ok(mut f) => {
                    let guard = Self {
                        path: path.clone(),
                        token: token.clone(),
                        _gate: gate,
                    };
                    writeln!(
                        f,
                        "pid={}\ntoken={}\ntime={}",
                        std::process::id(),
                        token,
                        Utc::now().to_rfc3339()
                    )?;
                    f.sync_all()?;
                    return Ok(guard);
                }
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // A stable flock serializes Rust recovery and ownership of the Go-compatible token file.
                    // Never remove a live or unverifiable owner's lock, regardless of age.
                    if dead_lock_owner(&path) {
                        let stale = path.with_extension(crate::id("stale"));
                        if fs::rename(&path, &stale).is_ok() {
                            let _ = fs::remove_file(stale);
                        }
                        continue;
                    }
                    if start.elapsed() >= timeout {
                        return Err(Error::LockTimeout);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path)
            .is_ok_and(|s| s.lines().any(|l| l == format!("token={}", self.token)))
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
pub(crate) enum Backend {
    Filesystem {
        path: PathBuf,
        timeout: Duration,
    },
    Sqlite {
        conn: Mutex<Connection>,
        prefix: String,
        project_id: String,
    },
}
pub(crate) trait Transaction {
    fn events(&mut self) -> Result<Vec<Event>>;
    fn append(&mut self, event: &Event) -> Result<()>;
    fn replace(&mut self, events: &[Event]) -> Result<()>;
    fn snapshot(&mut self, state: &State) -> Result<()>;
    fn embeddings(&mut self) -> Result<BTreeMap<String, EmbeddingRecord>>;
    fn put_embeddings(&mut self, records: &BTreeMap<String, EmbeddingRecord>) -> Result<()>;
    fn delete_embeddings(&mut self, ids: &[String]) -> Result<()>;
}
impl Backend {
    pub fn filesystem(path: PathBuf, timeout: Duration) -> Result<Self> {
        for sub in ["", "indexes", "snapshots", "locks"] {
            private_dir(&path.join(sub))?;
        }
        Ok(Self::Filesystem { path, timeout })
    }
    pub fn sqlite(path: &Path, prefix: &str, project_id: &str) -> Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(Error::Invalid("SQLite path is required".into()));
        }
        if let Some(parent) = path.parent() {
            private_dir(parent)?;
        }
        drop(private_open(path, false, false)?);
        sync_directory(path.parent().unwrap())?;
        Self::connection(Connection::open(path)?, prefix, project_id)
    }
    pub fn connection(conn: Connection, prefix: &str, project_id: &str) -> Result<Self> {
        let prefix = if prefix.trim().is_empty() {
            "projectstate_"
        } else {
            prefix.trim()
        };
        if !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || prefix.starts_with(|c: char| c.is_ascii_digit())
        {
            return Err(Error::Invalid("invalid SQL table prefix".into()));
        }
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(&format!("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS {prefix}events (project_id TEXT NOT NULL, seq INTEGER NOT NULL, event_id TEXT NOT NULL, run_id TEXT, actor TEXT, ts INTEGER NOT NULL, type TEXT NOT NULL, payload BLOB, PRIMARY KEY(project_id,seq));
            CREATE TABLE IF NOT EXISTS {prefix}embeddings (project_id TEXT NOT NULL, memory_id TEXT NOT NULL, hash TEXT NOT NULL, model TEXT NOT NULL, dims INTEGER NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(project_id,memory_id));"))?;
        Ok(Self::Sqlite {
            conn: Mutex::new(conn),
            prefix: prefix.into(),
            project_id: project_id.into(),
        })
    }
    pub fn transaction<T>(&self, f: impl FnOnce(&mut dyn Transaction) -> Result<T>) -> Result<T> {
        match self {
            Self::Filesystem { path, timeout } => {
                let _lock = Lock::acquire(path, *timeout)?;
                f(&mut FsTransaction { path })
            }
            Self::Sqlite {
                conn,
                prefix,
                project_id,
            } => {
                let mut conn = conn.lock().map_err(|_| Error::Poisoned)?;
                // Lock before replay, not just INSERT, so updates from separate handles cannot be lost.
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let out = f(&mut SqlTransaction {
                    conn: &tx,
                    prefix,
                    project_id,
                })?;
                tx.commit()?;
                Ok(out)
            }
        }
    }
}
struct FsTransaction<'a> {
    path: &'a Path,
}
impl Transaction for FsTransaction<'_> {
    fn events(&mut self) -> Result<Vec<Event>> {
        let path = self.path.join("events.jsonl");
        let bytes = match read_private(&path) {
            Ok(v) => v,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut events = Vec::new();
        let mut valid = 0;
        let mut offset = 0;
        let mut torn = None;
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            offset += line.len();
            if line.iter().all(u8::is_ascii_whitespace) {
                if torn.is_none() {
                    valid = offset;
                }
                continue;
            }
            if let Some(e) = torn {
                return Err(Error::Json(e));
            }
            match serde_json::from_slice(line) {
                Ok(ev) => {
                    events.push(ev);
                    valid = offset;
                }
                Err(e) => torn = Some(e),
            }
        }
        if torn.is_some() {
            let f = private_open(&path, false, false)?;
            f.set_len(valid as u64)?;
            f.sync_all()?;
        }
        // A valid unterminated final JSON record must not merge with the next append.
        if valid > 0 && bytes[valid - 1] != b'\n' {
            let mut f = private_open(&path, true, false)?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        Ok(events)
    }
    fn append(&mut self, event: &Event) -> Result<()> {
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        let mut f = private_open(&self.path.join("events.jsonl"), true, false)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        sync_directory(self.path)?;
        Ok(())
    }
    fn replace(&mut self, events: &[Event]) -> Result<()> {
        let mut bytes = vec![];
        for ev in events {
            serde_json::to_writer(&mut bytes, ev)?;
            bytes.push(b'\n');
        }
        atomic_write(&self.path.join("events.jsonl"), &bytes)
    }
    fn snapshot(&mut self, state: &State) -> Result<()> {
        let now = Utc::now();
        let base = self.path.join("indexes");
        atomic_json(&base.join("project.json"), &state.project)?;
        atomic_json(
            &base.join("tasks.json"),
            &serde_json::json!({"schema_version":SCHEMA_VERSION,"updated_at":now,"tasks":state.sorted_tasks()}),
        )?;
        atomic_json(
            &base.join("memories.json"),
            &serde_json::json!({"schema_version":SCHEMA_VERSION,"updated_at":now,"memories":state.sorted_memories()}),
        )?;
        atomic_json(
            &base.join("sessions.json"),
            &serde_json::json!({"schema_version":SCHEMA_VERSION,"updated_at":now,"sessions":state.sorted_sessions()}),
        )
    }
    fn embeddings(&mut self) -> Result<BTreeMap<String, EmbeddingRecord>> {
        match read_private(&self.path.join("indexes/embeddings.json")) {
            Ok(v) => Ok(serde_json::from_value(
                serde_json::from_slice::<serde_json::Value>(&v)?["vectors"].clone(),
            )?),
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e),
        }
    }
    fn put_embeddings(&mut self, records: &BTreeMap<String, EmbeddingRecord>) -> Result<()> {
        let mut all = self.embeddings()?;
        all.extend(records.clone());
        atomic_json(
            &self.path.join("indexes/embeddings.json"),
            &serde_json::json!({"schema_version":SCHEMA_VERSION,"updated_at":Utc::now(),"model":records.values().next().map(|r| r.model.as_str()).unwrap_or(""),"vectors":all}),
        )
    }
    fn delete_embeddings(&mut self, ids: &[String]) -> Result<()> {
        let mut all = self.embeddings()?;
        for id in ids {
            all.remove(id);
        }
        atomic_json(
            &self.path.join("indexes/embeddings.json"),
            &serde_json::json!({"schema_version":SCHEMA_VERSION,"updated_at":Utc::now(),"model":"","vectors":all}),
        )
    }
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut v = serde_json::to_vec_pretty(value)?;
    v.push(b'\n');
    atomic_write(path, &v)
}
struct SqlTransaction<'a> {
    conn: &'a Connection,
    prefix: &'a str,
    project_id: &'a str,
}
impl Transaction for SqlTransaction<'_> {
    fn events(&mut self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(&format!("SELECT seq,event_id,run_id,actor,ts,type,payload FROM {}events WHERE project_id=? ORDER BY seq", self.prefix))?;
        let mut rows = stmt.query([self.project_id])?;
        let mut out = vec![];
        while let Some(row) = rows.next()? {
            let ns: i64 = row.get(4)?;
            let data: Vec<u8> = row.get(6)?;
            out.push(Event {
                seq: row.get(0)?,
                event_id: row.get(1)?,
                project_id: self.project_id.into(),
                run_id: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                actor: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                time: DateTime::from_timestamp_nanos(ns),
                event_type: row.get(5)?,
                payload: serde_json::from_slice(&data)?,
            });
        }
        Ok(out)
    }
    fn append(&mut self, ev: &Event) -> Result<()> {
        let ts = ev.time.timestamp_nanos_opt().ok_or_else(|| {
            Error::Invalid("event timestamp outside SQLite nanosecond range".into())
        })?;
        self.conn.execute(&format!("INSERT INTO {}events (project_id,seq,event_id,run_id,actor,ts,type,payload) VALUES (?,?,?,?,?,?,?,?)", self.prefix), params![self.project_id,ev.seq,ev.event_id,ev.run_id,ev.actor,ts,ev.event_type,serde_json::to_vec(&ev.payload)?])?;
        Ok(())
    }
    fn replace(&mut self, events: &[Event]) -> Result<()> {
        self.conn.execute(
            &format!("DELETE FROM {}events WHERE project_id=?", self.prefix),
            [self.project_id],
        )?;
        for ev in events {
            self.append(ev)?;
        }
        Ok(())
    }
    fn snapshot(&mut self, _: &State) -> Result<()> {
        Ok(())
    }
    fn embeddings(&mut self) -> Result<BTreeMap<String, EmbeddingRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT memory_id,hash,model,dims,vector FROM {}embeddings WHERE project_id=?",
            self.prefix
        ))?;
        let mut rows = stmt.query([self.project_id])?;
        let mut out = BTreeMap::new();
        while let Some(r) = rows.next()? {
            let blob: Vec<u8> = r.get(4)?;
            let dims: usize = r.get(3)?;
            if blob.len() != dims * 4 {
                return Err(Error::Invalid("invalid embedding blob dimensions".into()));
            }
            let vector = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            out.insert(
                r.get(0)?,
                EmbeddingRecord {
                    hash: r.get(1)?,
                    model: r.get(2)?,
                    dims,
                    vector,
                },
            );
        }
        Ok(out)
    }
    fn put_embeddings(&mut self, records: &BTreeMap<String, EmbeddingRecord>) -> Result<()> {
        for (id, r) in records {
            let bytes: Vec<u8> = r.vector.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.conn.execute(&format!("INSERT INTO {}embeddings (project_id,memory_id,hash,model,dims,vector) VALUES (?,?,?,?,?,?) ON CONFLICT(project_id,memory_id) DO UPDATE SET hash=excluded.hash,model=excluded.model,dims=excluded.dims,vector=excluded.vector",self.prefix), params![self.project_id,id,r.hash,r.model,r.dims,bytes])?;
        }
        Ok(())
    }
    fn delete_embeddings(&mut self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.conn.execute(
                &format!(
                    "DELETE FROM {}embeddings WHERE project_id=? AND memory_id=?",
                    self.prefix
                ),
                params![self.project_id, id],
            )?;
        }
        Ok(())
    }
}

fn dead_lock_owner(path: &Path) -> bool {
    #[cfg(any(unix, windows))]
    {
        let Some(pid) = fs::read_to_string(path).ok().and_then(|s| {
            s.lines().find_map(|line| {
                line.strip_prefix("pid=")
                    .and_then(|v| v.parse::<u32>().ok())
            })
        }) else {
            return false;
        };
        if pid == 0 {
            return false;
        }
        #[cfg(unix)]
        {
            let Ok(pid) = i32::try_from(pid) else {
                return false;
            };
            // Signal zero probes existence without sending a signal; EPERM means the owner is alive.
            unsafe {
                libc::kill(pid, 0) == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError},
                System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            };

            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if handle.is_null() {
                    // Access denied and other errors cannot prove the owner is dead.
                    GetLastError() == ERROR_INVALID_PARAMETER
                } else {
                    CloseHandle(handle);
                    false
                }
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        false
    }
}
