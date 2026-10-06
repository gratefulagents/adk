use crate::{Error, FilesystemOptions, Result, derive_project_id, storage::sanitize_project_id};
use std::path::{Component, Path, PathBuf};

/// Host-authorized paths; neither value is read from the process environment.
#[derive(Debug, Clone)]
pub struct FilesystemResolutionHost {
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
}

/// Pure resolution output for `FilesystemOptions::state_dir` and its `store` fields.
///
/// `state_dir` is always nonempty and absolute. `work_dir` is absolute unless the
/// configured work directory was blank. Opening a store still performs all native
/// filesystem safety and project-identity checks; resolution grants no access.
/// Do not pass the derived `project_id` back as an explicit store ID: that would
/// sanitize it a second time. Use [`FilesystemOptions::resolve`] to open a store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFilesystemOptions {
    pub project_id: String,
    pub state_dir: PathBuf,
    pub work_dir: PathBuf,
}

/// Resolve Go-compatible filesystem options without filesystem or environment IO.
///
/// Configured strings are trimmed. Relative paths use the explicit host cwd, not
/// the work directory. Host paths must be absolute, UTF-8, and NUL-free; a home is
/// required only for the default state directory. Cleaning is lexical and does
/// not follow symlinks or establish that any path exists or is authorized.
pub fn resolve_filesystem_options(
    host: &FilesystemResolutionHost,
    state_dir: &str,
    project_id: &str,
    work_dir: &str,
) -> Result<ResolvedFilesystemOptions> {
    validate_host_path(&host.cwd, "cwd")?;
    if let Some(home) = &host.home {
        validate_host_path(home, "home")?;
    }
    let work_dir = work_dir.trim();
    let work_dir = if work_dir.is_empty() {
        PathBuf::new()
    } else {
        resolve_path(&host.cwd, work_dir)?
    };
    let mut project_id = sanitize_project_id(project_id);
    if project_id.is_empty() {
        project_id = derive_project_id(work_dir.to_str().unwrap());
    }
    let state_dir = state_dir.trim();
    let state_dir = if state_dir.is_empty() {
        let home = host.home.as_ref().ok_or_else(|| {
            Error::Invalid("home is required for the default state directory".into())
        })?;
        clean_absolute(
            &home
                .join(".gratefulagents/projects")
                .join(sanitize_project_id(&project_id))
                .join("state"),
        )
    } else {
        resolve_path(&host.cwd, state_dir)?
    };
    Ok(ResolvedFilesystemOptions {
        project_id,
        state_dir,
        work_dir,
    })
}

impl FilesystemOptions {
    /// Resolve paths against explicit host authority without opening the store.
    pub fn resolve(mut self, host: &FilesystemResolutionHost) -> Result<Self> {
        let state_dir = self
            .state_dir
            .to_str()
            .ok_or_else(|| Error::Invalid("configured state directory must be UTF-8".into()))?;
        let work_dir = self
            .store
            .work_dir
            .to_str()
            .ok_or_else(|| Error::Invalid("configured work directory must be UTF-8".into()))?;
        let resolved =
            resolve_filesystem_options(host, state_dir, &self.store.project_id, work_dir)?;
        self.state_dir = resolved.state_dir;
        self.store.work_dir = resolved.work_dir;
        Ok(self)
    }
}

fn validate_host_path(path: &Path, name: &str) -> Result<()> {
    if !path.is_absolute() || path.to_str().is_none_or(|path| path.contains('\0')) {
        return Err(Error::Invalid(format!(
            "host {name} must be an absolute, UTF-8, NUL-free path"
        )));
    }
    Ok(())
}

fn resolve_path(cwd: &Path, value: &str) -> Result<PathBuf> {
    if value.contains('\0') {
        return Err(Error::Invalid("configured path must be NUL-free".into()));
    }
    let path = cwd.join(value);
    if !path.is_absolute() {
        return Err(Error::Invalid(
            "configured path must resolve absolutely".into(),
        ));
    }
    Ok(clean_absolute(&path))
}

fn clean_absolute(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}
