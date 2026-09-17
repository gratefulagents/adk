use crate::{
    Backend, Config, Error, Network, Request,
    policy::{self, PROTECTED, SECRET},
};
use adk_core::AccessMode;
use std::{
    ffi::{CString, OsString},
    fs,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};
use tokio::process::Command;

#[derive(Debug)]
pub(crate) struct PrivateDir(pub PathBuf);
impl PrivateDir {
    pub fn new() -> Result<Self, Error> {
        let mut template = b"/tmp/adk-sandbox-XXXXXX\0".to_vec();
        // mkdtemp atomically creates a 0700 directory, independently of umask.
        let result = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
        if result.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(&template[..template.len() - 1]));
        let private = Self(path.canonicalize()?);
        fs::create_dir(private.0.join("home"))?;
        Ok(private)
    }
}
impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) struct Built {
    pub command: Command,
    pub _private: PrivateDir,
}

pub(crate) fn build(config: &Config, request: &Request) -> Result<Built, Error> {
    let private = PrivateDir::new()?;
    let backend = match config.backend {
        Backend::Auto if cfg!(target_os = "linux") => Backend::Bubblewrap,
        Backend::Auto if cfg!(target_os = "macos") => Backend::Seatbelt,
        backend => backend,
    };
    let mut command = match backend {
        Backend::Local => {
            if request.hide_git_credentials {
                return Err(Error::Invalid(
                    "credential masking requires an enforcing backend".into(),
                ));
            }
            let mut cmd = Command::new(&request.program);
            cmd.args(&request.args).current_dir(&request.cwd);
            cmd.env_clear()
                .envs(policy::environment(&request.env, &private.0)?);
            cmd
        }
        Backend::Bubblewrap if cfg!(target_os = "linux") => {
            validate_tree(&config.workspace)?;
            let mut cmd = Command::new("/usr/bin/bwrap");
            cmd.args(bwrap_args(config, request, &private)?);
            cmd.current_dir("/")
                .env_clear()
                .env("PATH", "/usr/bin:/bin");
            cmd
        }
        Backend::Seatbelt if cfg!(target_os = "macos") => {
            validate_tree(&config.workspace)?;
            let mut cmd = Command::new("/usr/bin/sandbox-exec");
            let masks = credential_masks(config, request)?;
            cmd.arg("-p")
                .arg(seatbelt_profile_with_masks(config, request, &masks)?);
            for (i, path) in masks.iter().enumerate() {
                let path = path
                    .to_str()
                    .ok_or_else(|| Error::Invalid("non-UTF8 Git config path".into()))?;
                cmd.arg(format!("-DGITCONFIG{i}={path}"));
            }
            for (i, path) in credential_ancestors(config, request, &masks)
                .iter()
                .enumerate()
            {
                let path = path
                    .to_str()
                    .ok_or_else(|| Error::Invalid("non-UTF8 Git ancestor path".into()))?;
                cmd.arg(format!("-DGITPARENT{i}={path}"));
            }
            cmd.arg(format!("-DWORKSPACE={}", config.workspace.display()));
            cmd.arg(format!("-DGIT={}", config.workspace.join(".git").display()));
            cmd.arg(format!("-DPRIVATE={}", private.0.display()));
            for (i, root) in config.runtime_roots.iter().enumerate() {
                cmd.arg(format!("-DRUNTIME{i}={}", root.display()));
            }
            if request.access == AccessMode::WorkspaceWrite {
                for (i, scratch) in request.scratch.iter().enumerate() {
                    cmd.arg(format!("-DSCRATCH{i}={}", scratch.path().display()));
                }
            }
            for (i, name) in PROTECTED.iter().chain(SECRET.iter()).enumerate() {
                cmd.arg(format!(
                    "-DMASK{i}={}",
                    config.workspace.join(name).display()
                ));
            }
            cmd.args(["--", "/usr/bin/env", "-i"]);
            for (key, value) in policy::environment(&request.env, &private.0)? {
                cmd.arg(format!("{key}={value}"));
            }
            cmd.arg(&request.program).args(&request.args);
            cmd.current_dir(&request.cwd)
                .env_clear()
                .env("PATH", "/usr/bin:/bin");
            cmd
        }
        _ => {
            return Err(Error::Unavailable(
                "requested OS backend is not supported on this platform".into(),
            ));
        }
    };
    command.kill_on_drop(true);
    Ok(Built {
        command,
        _private: private,
    })
}

fn validate_tree(root: &Path) -> Result<(), Error> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                if metadata.nlink() != 1 {
                    return Err(Error::Invalid("workspace hardlinks are unsupported".into()));
                }
            } else if metadata.file_type().is_symlink() {
                let path = entry.path();
                let relative = path.strip_prefix(root).expect("workspace descendant");
                let first = relative.components().next().unwrap().as_os_str();
                if first == ".git"
                    || PROTECTED
                        .iter()
                        .chain(SECRET.iter())
                        .any(|name| first == *name)
                {
                    return Err(Error::Invalid(
                        "symlinks in protected metadata are unsupported".into(),
                    ));
                }
            } else {
                return Err(Error::Invalid(
                    "workspace device/FIFO/socket entries are unsupported".into(),
                ));
            }
        }
    }
    Ok(())
}

fn bwrap_args(
    config: &Config,
    request: &Request,
    private: &PrivateDir,
) -> Result<Vec<OsString>, Error> {
    let mut args: Vec<OsString> = [
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--cap-drop",
        "ALL",
        "--clearenv",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if request.network == Network::Deny {
        args.push("--unshare-net".into());
    }
    for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64"] {
        if Path::new(path).exists() {
            args.extend(["--ro-bind".into(), path.into(), path.into()]);
        }
    }
    for path in ["/etc/ld.so.cache", "/etc/ld.so.conf"] {
        if Path::new(path).is_file() {
            args.extend(["--ro-bind".into(), path.into(), path.into()]);
        }
    }
    if request.network == Network::Allow {
        for path in ["/etc/resolv.conf", "/etc/hosts", "/etc/nsswitch.conf"] {
            if Path::new(path).is_file() {
                args.extend(["--ro-bind".into(), path.into(), path.into()]);
            }
        }
    }
    args.extend(
        [
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/tmp/home",
        ]
        .into_iter()
        .map(Into::into),
    );
    for root in &config.runtime_roots {
        args.extend([
            "--ro-bind".into(),
            root.as_os_str().to_owned(),
            root.as_os_str().to_owned(),
        ]);
    }
    let mode = if request.access == AccessMode::ReadOnly {
        "--ro-bind"
    } else {
        "--bind"
    };
    args.extend([
        mode.into(),
        config.workspace.as_os_str().to_owned(),
        config.workspace.as_os_str().to_owned(),
    ]);
    if request.access == AccessMode::WorkspaceWrite {
        for scratch in &request.scratch {
            args.extend([
                "--bind".into(),
                scratch.path().as_os_str().to_owned(),
                scratch.path().as_os_str().to_owned(),
            ]);
        }
        for name in PROTECTED {
            let path = config.workspace.join(name);
            let metadata = fs::symlink_metadata(&path).map_err(|_| {
                Error::Invalid(format!(
                    "workspace-write requires host-precreated metadata: {name}"
                ))
            })?;
            let directory = !matches!(*name, ".git/config" | ".mcp.json");
            if metadata.file_type().is_symlink() || metadata.is_dir() != directory {
                return Err(Error::Invalid(format!(
                    "invalid protected metadata mountpoint: {name}"
                )));
            }
        }
        // Pin the writable .git parent too: otherwise it could be renamed and
        // replaced, bypassing the config/hooks child mounts.
        let git = config.workspace.join(".git").into_os_string();
        args.extend(["--bind".into(), git.clone(), git]);
        for name in PROTECTED {
            let path = config.workspace.join(name).into_os_string();
            args.extend(["--ro-bind".into(), path.clone(), path]);
        }
    }
    let empty_file = private.0.join("empty");
    fs::write(&empty_file, [])?;
    for name in SECRET {
        let path = config.workspace.join(name);
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            let source = if metadata.is_dir() {
                private.0.join("home")
            } else {
                empty_file.clone()
            };
            args.extend([
                "--ro-bind".into(),
                source.into_os_string(),
                path.into_os_string(),
            ]);
        }
    }
    for path in credential_masks(config, request)? {
        args.extend([
            "--ro-bind".into(),
            empty_file.as_os_str().to_owned(),
            path.into_os_string(),
        ]);
    }
    for (key, value) in policy::environment(&request.env, Path::new("/tmp"))? {
        args.extend(["--setenv".into(), key.into(), value.into()]);
    }
    args.extend([
        "--chdir".into(),
        request.cwd.as_os_str().to_owned(),
        "--".into(),
        request.program.as_os_str().to_owned(),
    ]);
    args.extend(request.args.iter().map(Into::into));
    Ok(args)
}

fn credential_masks(config: &Config, request: &Request) -> Result<Vec<PathBuf>, Error> {
    if !request.hide_git_credentials {
        return Ok(Vec::new());
    }
    let mut roots = vec![config.workspace.clone()];
    for scratch in &request.scratch {
        validate_tree(scratch.path())?;
        roots.push(scratch.path().to_owned());
    }
    crate::git_credentials::discover(&roots)
}

fn credential_ancestors(config: &Config, request: &Request, masks: &[PathBuf]) -> Vec<PathBuf> {
    let mut ancestors = std::collections::BTreeSet::new();
    for path in masks {
        for parent in path.ancestors().skip(1) {
            if !parent.starts_with(&config.workspace)
                && !request
                    .scratch
                    .iter()
                    .any(|scratch| parent.starts_with(scratch.path()))
            {
                break;
            }
            ancestors.insert(parent.to_owned());
        }
    }
    ancestors.into_iter().collect()
}

#[cfg(test)]
fn seatbelt_profile(config: &Config, request: &Request) -> Result<String, Error> {
    seatbelt_profile_with_masks(config, request, &credential_masks(config, request)?)
}

fn seatbelt_profile_with_masks(
    config: &Config,
    request: &Request,
    masks: &[PathBuf],
) -> Result<String, Error> {
    // Paths are passed as sandbox parameters, never interpolated as policy code.
    CString::new(config.workspace.as_os_str().as_bytes())
        .map_err(|_| Error::Invalid("NUL in workspace".into()))?;
    if config.workspace.to_str().is_none() {
        return Err(Error::Invalid("non-UTF8 workspace".into()));
    }
    let mut profile = String::from(include_str!("seatbelt-base.sb"));
    profile.push_str("(allow file-read* (subpath \"/usr\") (subpath \"/bin\") (subpath \"/sbin\") (subpath \"/System/Library\") (subpath \"/Library/Apple\") (subpath \"/private/var/db/dyld\") (literal \"/dev/null\") (literal \"/dev/urandom\") (literal \"/dev/random\"))\n");
    profile.push_str("(allow file-read* file-write* (subpath (param \"PRIVATE\")))\n(allow file-write-data (literal \"/dev/null\"))\n");
    for i in 0..config.runtime_roots.len() {
        profile.push_str(&format!(
            "(allow file-read* (subpath (param \"RUNTIME{i}\")))\n"
        ));
    }
    profile.push_str("(allow file-read* (require-all (subpath (param \"WORKSPACE\"))");
    for i in PROTECTED.len()..PROTECTED.len() + SECRET.len() {
        profile.push_str(&format!(" (require-not (subpath (param \"MASK{i}\")))"));
    }
    profile.push_str("))\n");
    if request.access == AccessMode::WorkspaceWrite {
        for i in 0..request.scratch.len() {
            // Keep the grant root pinned: only its descendants may be mutated.
            profile.push_str(&format!("(allow file-read* (subpath (param \"SCRATCH{i}\")))\n(allow file-write* (require-all (subpath (param \"SCRATCH{i}\")) (require-not (literal (param \"SCRATCH{i}\")))))\n"));
        }
        profile.push_str("(allow file-write* (require-all (subpath (param \"WORKSPACE\")) (require-not (literal (param \"WORKSPACE\")))");
        profile.push_str(" (require-not (literal (param \"GIT\")))");
        for i in 0..PROTECTED.len() + SECRET.len() {
            profile.push_str(&format!(" (require-not (subpath (param \"MASK{i}\")))"));
        }
        profile.push_str("))\n");
    }
    for i in 0..masks.len() {
        // ENOENT keeps optional repository config reads compatible with status/diff.
        profile.push_str(&format!("(deny file-read* (with errno 2) (literal (param \"GITCONFIG{i}\")))\n(deny file-write* (literal (param \"GITCONFIG{i}\")))\n"));
    }
    for i in 0..credential_ancestors(config, request, masks).len() {
        // A rename must not move the credential out of its path-based read mask.
        profile.push_str(&format!(
            "(deny file-write* (literal (param \"GITPARENT{i}\")))\n"
        ));
    }
    if request.network == Network::Allow {
        profile.push_str("(allow network*)\n");
    }
    Ok(profile)
}

#[cfg(test)]
#[path = "scratch_tests.rs"]
mod scratch_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_are_default_deny_without_host_root() {
        let private = PrivateDir::new().unwrap();
        fs::create_dir(private.0.join("work")).unwrap();
        let config = Config::new(private.0.join("work"));
        let req = Request::new("/bin/sh");
        let args = bwrap_args(&config, &req, &private).unwrap();
        assert!(args.contains(&"--unshare-net".into()));
        assert!(args.contains(&"--unshare-pid".into()));
        assert!(!args.windows(3).any(|w| w == ["--ro-bind", "/", "/"]));
        let profile = seatbelt_profile(&config, &req).unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow file-read* (literal \"/\"))"));
        assert!(!profile.contains("(subpath \"/\")"));
        assert!(profile.contains("(literal \"/private/var/select/sh\")"));
        assert!(profile.contains("(literal \"/private/var/select/developer_dir\")"));
        assert!(!profile.contains("(subpath \"/private/var\")"));
        assert!(profile.contains("(sysctl-name \"hw.pagesize_compat\")"));
        assert!(!profile.contains("(allow sysctl-read)"));
        assert!(!profile.contains("(sysctl-name \"kern.procargs2\")"));
        assert!(profile.contains("(deny sysctl-read (sysctl-name-prefix \"kern.procargs\"))"));
        assert!(!profile.contains("(allow network*)"));
        assert!(!profile.contains("(allow file-write*"));
    }

    #[test]
    fn runtime_roots_are_host_only_and_cannot_overlap_the_workspace() {
        let private = PrivateDir::new().unwrap();
        let workspace = private.0.join("work");
        let runtime = private.0.join("runtime");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&runtime).unwrap();
        let mut config = Config::new(&workspace);
        config.runtime_roots.push(runtime.clone());
        let executor = crate::Executor::new(config.clone()).unwrap();
        let profile = seatbelt_profile(&executor.config, &Request::new("/bin/sh")).unwrap();
        assert!(profile.contains("(allow file-read* (subpath (param \"RUNTIME0\")))"));
        assert!(!profile.contains(&runtime.display().to_string()));
        let args = bwrap_args(&executor.config, &Request::new("/bin/sh"), &private).unwrap();
        assert!(args.windows(3).any(|args| args[0] == "--ro-bind"
            && args[1] == executor.config.runtime_roots[0].as_os_str()
            && args[2] == executor.config.runtime_roots[0].as_os_str()));

        config.runtime_roots = vec![workspace];
        assert!(crate::Executor::new(config).is_err());
    }

    #[test]
    fn git_credential_plans_mask_nested_worktree_and_scratch_configs() {
        let private = PrivateDir::new().unwrap();
        let root = private.0.join("home");
        for dir in [".git", "nested/.git", "linked", "metadata"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for path in [
            ".git/config",
            "nested/.git/config",
            "metadata/config.worktree",
        ] {
            fs::write(
                root.join(path),
                "[remote \"origin\"]\nurl=https://secret@example.invalid\n",
            )
            .unwrap();
        }
        fs::write(root.join("linked/.git"), "gitdir: ../metadata\n").unwrap();
        fs::write(root.join("metadata/commondir"), "../.git\n").unwrap();
        let config = Config::new(&root);
        let mut request = Request::new("/bin/sh");
        request.hide_git_credentials = true;
        let masks = credential_masks(&config, &request).unwrap();
        assert_eq!(masks.len(), 3);
        let args = bwrap_args(&config, &request, &private).unwrap();
        for path in &masks {
            assert!(args.windows(3).any(|args| args[0] == "--ro-bind"
                && args[1] == private.0.join("empty").as_os_str()
                && args[2] == path.as_os_str()));
        }
        let profile = seatbelt_profile(&config, &request).unwrap();
        assert!(
            profile.contains("(deny file-read* (with errno 2) (literal (param \"GITCONFIG2\")))")
        );
        assert!(profile.contains("(deny file-write* (literal (param \"GITPARENT0\")))"));
        assert!(!profile.contains("secret@example.invalid"));
        let scratch = std::sync::Arc::new(crate::ScratchDirectory::new().unwrap());
        fs::create_dir(scratch.path().join(".git")).unwrap();
        fs::write(scratch.path().join(".git/config"), "").unwrap();
        request.scratch.push(scratch);
        assert_eq!(credential_masks(&config, &request).unwrap().len(), 4);
        fs::write(root.join("metadata/commondir"), "/usr\n").unwrap();
        assert!(credential_masks(&config, &request).is_err());
        fs::write(root.join("metadata/commondir"), "../.git\n").unwrap();
        fs::remove_file(root.join("nested/.git/config")).unwrap();
        std::os::unix::fs::symlink("../../.git/config", root.join("nested/.git/config")).unwrap();
        assert!(credential_masks(&config, &request).is_err());
    }

    #[test]
    fn missing_metadata_hardlinks_and_symlinked_metadata_fail_closed() {
        let private = PrivateDir::new().unwrap();
        let work = private.0.join("home");
        let config = Config::new(&work);
        let mut req = Request::new("/bin/sh");
        req.access = AccessMode::WorkspaceWrite;
        assert!(bwrap_args(&config, &req, &private).is_err());
        fs::write(work.join("original"), b"x").unwrap();
        fs::hard_link(work.join("original"), work.join("alias")).unwrap();
        assert!(validate_tree(&work).is_err());
        fs::remove_file(work.join("alias")).unwrap();
        std::os::unix::fs::symlink("original", work.join(".git")).unwrap();
        assert!(validate_tree(&work).is_err());
    }
}
