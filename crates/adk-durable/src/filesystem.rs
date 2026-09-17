use crate::{
    codec::{protect, unprotect},
    store::*,
    *,
};
use chrono::Utc;
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct FilesystemStore {
    root: PathBuf,
    options: StoreOptions,
}
#[derive(Serialize, Deserialize)]
struct Record {
    document: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease: Option<Lease>,
}
impl FilesystemStore {
    pub fn new(root: impl AsRef<Path>, options: StoreOptions) -> Result<Self> {
        if root.as_ref().as_os_str().is_empty() {
            return Err(Error::Invalid("filesystem root is required".into()));
        }
        #[cfg(not(unix))]
        return Err(Error::Invalid(
            "private filesystem store requires Unix; use PostgreSQL".into(),
        ));
        private_dir(root.as_ref())?;
        let root = fs::canonicalize(root)?;
        private_dir(&root.join("tenants"))?;
        private_dir(&root.join("locks"))?;
        Ok(Self { root, options })
    }
    fn lock(&self, tenant: &TenantId) -> Result<File> {
        safe_id(tenant.as_str())?;
        inspect_dir(&self.root.join("locks"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        secure_options(&mut options);
        let file = options.open(self.root.join("locks").join(format!("{tenant}.lock")))?;
        regular(&file)?;
        private_file(&file)?;
        FileExt::lock(&file)?;
        Ok(file)
    }
    fn tenant_dir(&self, tenant: &TenantId) -> Result<PathBuf> {
        safe_id(tenant.as_str())?;
        inspect_dir(&self.root.join("tenants"))?;
        let path = self.root.join("tenants").join(tenant.as_str());
        match fs::symlink_metadata(&path) {
            Ok(_) => inspect_dir(&path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        Ok(path)
    }
    fn path(&self, tenant: &TenantId, run: &RunId) -> Result<PathBuf> {
        safe_id(run.as_str())?;
        Ok(self.tenant_dir(tenant)?.join(format!("{run}.json")))
    }
    fn read(&self, tenant: &TenantId, run: &RunId) -> Result<(Document, Option<Lease>)> {
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let mut file = options.open(self.path(tenant, run)?).map_err(not_found)?;
        regular(&file)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let record: Record = unprotect(&bytes, &self.options)?;
        let document = decode_document(&serde_json::to_vec(&record.document)?)?;
        validate_key(&document.snapshot, tenant, run)?;
        let mut sequence = 0;
        for event in &document.events {
            if &event.tenant_id != tenant
                || &event.run_id != run
                || event.sequence <= sequence
                || event.sequence > document.snapshot.event_sequence
            {
                return Err(Error::Invalid(
                    "event metadata does not match record".into(),
                ));
            }
            sequence = event.sequence;
        }
        if let Some(lease) = &record.lease {
            if &lease.tenant_id != tenant || &lease.run_id != run {
                return Err(Error::Invalid("lease key does not match record".into()));
            }
        }
        Ok((document, record.lease))
    }
    fn write(&self, document: &Document, lease: Option<Lease>) -> Result<()> {
        let record = Record {
            document: serde_json::from_slice(&encode_document(document)?)?,
            lease,
        };
        let bytes = protect(&record, &self.options)?;
        let path = self.path(&document.snapshot.tenant_id, &document.snapshot.run_id)?;
        let dir = path.parent().unwrap();
        let mut file = tempfile::Builder::new()
            .prefix(".durable-")
            .tempfile_in(dir)?;
        private_file(file.as_file())?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|e| Error::Io(e.error))?;
        File::open(dir)?.sync_all()?;
        Ok(())
    }
}
impl RunStore for FilesystemStore {
    fn create(&self, snapshot: RunSnapshot) -> Result<()> {
        validate_snapshot(&snapshot)?;
        let _lock = self.lock(&snapshot.tenant_id)?;
        let dir = self.tenant_dir(&snapshot.tenant_id)?;
        private_dir(&dir)?;
        File::open(self.root.join("tenants"))?.sync_all()?;
        match fs::symlink_metadata(self.path(&snapshot.tenant_id, &snapshot.run_id)?) {
            Ok(_) => return Err(Error::AlreadyExists),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        let snapshot = prepare_create(snapshot, &self.options)?;
        self.write(
            &Document {
                schema_version: SCHEMA_VERSION,
                snapshot,
                events: vec![],
            },
            None,
        )
    }
    fn load(&self, tenant: &TenantId, run: &RunId) -> Result<(RunSnapshot, Vec<Event>)> {
        let _lock = self.lock(tenant)?;
        let (document, _) = self.read(tenant, run)?;
        Ok((document.snapshot, document.events))
    }
    fn append(
        &self,
        lease: &Lease,
        expected_revision: u64,
        events: Vec<Event>,
        snapshot: RunSnapshot,
    ) -> Result<RunSnapshot> {
        let _lock = self.lock(&lease.tenant_id)?;
        let (mut document, stored) = self.read(&lease.tenant_id, &lease.run_id)?;
        check_lease(stored.as_ref(), lease, true)?;
        if document.snapshot.revision != expected_revision {
            return Err(Error::Conflict);
        }
        let (updated, events) = prepare_append(
            lease,
            expected_revision,
            document.snapshot.event_sequence,
            events,
            snapshot,
            &self.options,
        )?;
        document.snapshot = updated.clone();
        document.events.extend(events);
        self.write(&document, stored)?;
        Ok(updated)
    }
    fn acquire_lease(
        &self,
        tenant: &TenantId,
        run: &RunId,
        owner: &str,
        ttl: Duration,
    ) -> Result<Lease> {
        if owner.is_empty() {
            return Err(Error::Invalid("lease owner is required".into()));
        }
        expiry(Utc::now(), ttl)?;
        let _lock = self.lock(tenant)?;
        let (document, stored) = self.read(tenant, run)?;
        let now = Utc::now();
        if stored.is_some_and(|l| l.expires_at > now) {
            return Err(Error::LeaseHeld);
        }
        let lease = Lease {
            tenant_id: tenant.clone(),
            run_id: run.clone(),
            owner: owner.into(),
            token: LeaseToken::new(),
            expires_at: expiry(now, ttl)?,
        };
        self.write(&document, Some(lease.clone()))?;
        Ok(lease)
    }
    fn renew_lease(&self, lease: &Lease, ttl: Duration) -> Result<Lease> {
        expiry(Utc::now(), ttl)?;
        let _lock = self.lock(&lease.tenant_id)?;
        let (document, stored) = self.read(&lease.tenant_id, &lease.run_id)?;
        check_lease(stored.as_ref(), lease, true)?;
        let mut renewed = stored.unwrap();
        renewed.expires_at = expiry(Utc::now(), ttl)?;
        self.write(&document, Some(renewed.clone()))?;
        Ok(renewed)
    }
    fn release_lease(&self, lease: &Lease) -> Result<()> {
        let _lock = self.lock(&lease.tenant_id)?;
        let (document, stored) = self.read(&lease.tenant_id, &lease.run_id)?;
        check_lease(stored.as_ref(), lease, false)?;
        self.write(&document, None)
    }
    fn delete_run(&self, tenant: &TenantId, run: &RunId) -> Result<()> {
        let _lock = self.lock(tenant)?;
        let path = self.path(tenant, run)?;
        fs::remove_file(&path).map_err(not_found)?;
        File::open(path.parent().unwrap())?.sync_all()?;
        Ok(())
    }
    fn delete_tenant(&self, tenant: &TenantId) -> Result<()> {
        let _lock = self.lock(tenant)?;
        match fs::remove_dir_all(self.tenant_dir(tenant)?) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        File::open(self.root.join("tenants"))?.sync_all()?;
        Ok(())
    }
    fn apply_retention(&self, policy: RetentionPolicy) -> Result<u64> {
        let now = policy.now.unwrap_or_else(Utc::now);
        let mut deleted = 0;
        inspect_dir(&self.root.join("tenants"))?;
        for tenant in fs::read_dir(self.root.join("tenants"))? {
            let tenant = tenant?;
            if !tenant.file_type()?.is_dir() {
                continue;
            }
            let tenant = TenantId::from(tenant.file_name().to_string_lossy().into_owned());
            if safe_id(tenant.as_str()).is_err() {
                continue;
            }
            let _lock = self.lock(&tenant)?;
            let dir = self.tenant_dir(&tenant)?;
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Some(run) = name.strip_suffix(".json") else {
                    continue;
                };
                let (document, _) = self.read(&tenant, &RunId::from(run))?;
                if document
                    .snapshot
                    .retain_until
                    .is_some_and(|until| until <= now)
                {
                    fs::remove_file(entry.path())?;
                    File::open(&dir)?.sync_all()?;
                    deleted += 1;
                }
            }
        }
        Ok(deleted)
    }
}
fn check_lease(stored: Option<&Lease>, lease: &Lease, check_expiry: bool) -> Result<()> {
    match stored {
        Some(stored)
            if stored.token == lease.token && (!check_expiry || stored.expires_at > Utc::now()) =>
        {
            Ok(())
        }
        _ => Err(Error::LeaseLost),
    }
}
fn safe_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::Invalid("unsafe filesystem ID".into()));
    }
    Ok(())
}
fn inspect_dir(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(Error::Invalid("store path is not a real directory".into()));
    }
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    if !path.try_exists()? {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path)?;
    }
    inspect_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn secure_options(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
}
fn private_file(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
fn regular(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::Invalid("store record is not a regular file".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(Error::Invalid("store file has multiple hard links".into()));
        }
    }
    Ok(())
}
fn not_found(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::NotFound
    } else {
        error.into()
    }
}
