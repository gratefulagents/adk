use crate::Error;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn discover(roots: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
    let mut pending = roots.to_vec();
    let mut git_dirs = BTreeSet::new();
    while let Some(dir) = pending.pop() {
        if dir.join("HEAD").is_file()
            && (dir.join("objects").is_dir() || dir.join("commondir").is_file())
        {
            git_dirs.insert(dir.clone());
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if entry.file_name() == ".git" {
                if metadata.is_dir() {
                    git_dirs.insert(path.clone());
                } else if metadata.is_file() {
                    let text = fs::read_to_string(&path)?;
                    let target = text.trim().strip_prefix("gitdir: ").ok_or_else(invalid)?;
                    git_dirs.insert(resolve(&dir, target, roots)?);
                } else {
                    return Err(invalid());
                }
            }
            if metadata.is_dir() {
                pending.push(path);
            }
        }
    }
    let mut pending: Vec<_> = git_dirs.into_iter().collect();
    let mut visited = BTreeSet::new();
    let mut masks = BTreeSet::new();
    while let Some(dir) = pending.pop() {
        if !visited.insert(dir.clone()) {
            continue;
        }
        for name in ["config", "config.worktree"] {
            let path = dir.join(name);
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.nlink() != 1 {
                        return Err(invalid());
                    }
                    // Git parses quoting, continuations and case folding; do not invent a
                    // second config parser that could miss a credential-bearing include.
                    let output = Command::new("/usr/bin/git")
                        .env_clear()
                        .env("PATH", "/usr/bin:/bin")
                        .args(["config", "--no-includes", "--null", "--name-only", "--file"])
                        .arg(&path)
                        .arg("--list")
                        .output()?;
                    if !output.status.success()
                        || output.stdout.split(|b| *b == 0).any(|key| {
                            key.eq_ignore_ascii_case(b"include.path")
                                || key.get(..10).is_some_and(|prefix| {
                                    prefix.eq_ignore_ascii_case(b"includeif.")
                                })
                        })
                    {
                        return Err(Error::Invalid(
                            "credential masking requires valid Git configs without includes".into(),
                        ));
                    }
                    masks.insert(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let common = dir.join("commondir");
        match fs::symlink_metadata(&common) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.nlink() != 1 {
                    return Err(invalid());
                }
                pending.push(resolve(&dir, fs::read_to_string(common)?.trim(), roots)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // Submodules keep their own configs under the parent's Git directory.
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || (metadata.is_file() && metadata.nlink() != 1) {
                return Err(invalid());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(masks.into_iter().collect())
}

fn resolve(base: &Path, target: &str, roots: &[PathBuf]) -> Result<PathBuf, Error> {
    let path = base.join(target).canonicalize()?;
    if !path.is_dir() || !roots.iter().any(|root| path.starts_with(root)) {
        return Err(invalid());
    }
    Ok(path)
}

fn invalid() -> Error {
    Error::Invalid(
        "credential masking requires unaliased Git metadata inside authorized roots".into(),
    )
}
