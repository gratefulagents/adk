use super::{MAX_FILE, Plan, State};
use crate::workspace::Workspace;
use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags, fchmod, openat, renameat_with, unlinkat};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read, Write},
    os::unix::fs::MetadataExt,
    path::Path,
};

fn read(mut file: File) -> io::Result<State> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("is not a regular file"));
    }
    if metadata.nlink() != 1 {
        return Err(io::Error::other("file must have exactly one hard link"));
    }
    if metadata.len() > MAX_FILE as u64 {
        return Err(io::Error::other(format!(
            "is too large ({} bytes, limit {MAX_FILE})",
            metadata.len()
        )));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE {
        return Err(io::Error::other(format!("is too large (limit {MAX_FILE})")));
    }
    if bytes.contains(&0) {
        return Err(io::Error::other("is a binary file"));
    }
    let data = String::from_utf8(bytes).map_err(|_| io::Error::other("is a binary file"))?;
    Ok(State {
        exists: true,
        data,
        mode: metadata.mode() & 0o777,
    })
}
pub(super) fn inspect(workspace: &Workspace, path: &str) -> io::Result<State> {
    match workspace.open(Path::new(path)) {
        Ok(file) => read(file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e),
    }
}
fn inspect_at(parent: &File, name: &str) -> io::Result<State> {
    match openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => read(File::from(fd)),
        Err(rustix::io::Errno::NOENT) => Ok(State::default()),
        Err(e) => Err(e.into()),
    }
}
fn move_exclusive(parent: &File, from: &str, to: &str) -> io::Result<()> {
    renameat_with(parent, from, parent, to, RenameFlags::NOREPLACE).map_err(Into::into)
}
fn temporary_name() -> String {
    format!(".agentsdk-patch-{}", uuid::Uuid::new_v4().simple())
}
struct Entry {
    parent: File,
    name: String,
    path: String,
    original: State,
    final_state: State,
    staged: Option<String>,
    quarantine: Option<String>,
    claimed: bool,
    published: bool,
}
impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(name) = &self.staged {
            let _ = unlinkat(&self.parent, name, AtFlags::empty());
        }
    }
}
impl Entry {
    fn validate_parent(&self, workspace: &Workspace) -> io::Result<()> {
        let path = Path::new(&self.path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let current = workspace.open(path)?.metadata()?;
        let pinned = self.parent.metadata()?;
        if (current.dev(), current.ino()) != (pinned.dev(), pinned.ino()) {
            return Err(io::Error::other(format!(
                "destination parent changed during patch: {}",
                self.path
            )));
        }
        Ok(())
    }
    fn stage(&mut self) -> io::Result<()> {
        if !self.final_state.exists {
            return Ok(());
        }
        let mode = Mode::from_raw_mode(self.final_state.mode as _);
        let name = temporary_name();
        let mut file = File::from(openat(
            &self.parent,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            mode,
        )?);
        self.staged = Some(name);
        fchmod(&file, mode)?;
        file.write_all(self.final_state.data.as_bytes())?;
        file.sync_all()
    }
    fn claim(&mut self) -> io::Result<()> {
        if !self.original.exists {
            return Ok(());
        }
        let name = temporary_name();
        move_exclusive(&self.parent, &self.name, &name)?;
        self.quarantine = Some(name.clone());
        self.claimed = true;
        // Inspect after the no-replace rename, not before, to detect inode swaps.
        match inspect_at(&self.parent, &name) {
            Ok(actual) if actual == self.original => Ok(()),
            result => {
                move_exclusive(&self.parent, &name, &self.name).map_err(|e| {
                    io::Error::other(format!(
                        "source changed during patch and restoring quarantine {} failed: {e}",
                        self.path
                    ))
                })?;
                self.quarantine = None;
                self.claimed = false;
                match result {
                    Err(e) => Err(e),
                    _ => Err(io::Error::other(format!(
                        "file changed during patch validation: {}",
                        self.path
                    ))),
                }
            }
        }
    }
    fn publish(&mut self) -> io::Result<()> {
        if let Some(staged) = &self.staged {
            if inspect_at(&self.parent, staged)? != self.final_state {
                return Err(io::Error::other("staged patch file changed"));
            }
            move_exclusive(&self.parent, staged, &self.name)?;
            self.staged = None;
            self.published = true;
        }
        Ok(())
    }
    fn rollback(&mut self) -> io::Result<()> {
        if self.published {
            let name = temporary_name();
            move_exclusive(&self.parent, &self.name, &name)?;
            match inspect_at(&self.parent, &name) {
                Ok(actual) if actual == self.final_state => {}
                result => {
                    move_exclusive(&self.parent, &name, &self.name)?;
                    return Err(io::Error::other(format!(
                        "refusing to overwrite concurrent changes to {}: {:?}",
                        self.path,
                        result.err()
                    )));
                }
            }
            // Keep the claimed output recoverable until the original is restored.
            if let Err(e) = self.restore() {
                let recovery = move_exclusive(&self.parent, &name, &self.name);
                return Err(io::Error::other(format!(
                    "restoring {}: {e}; output recovery: {recovery:?}",
                    self.path
                )));
            }
            unlinkat(&self.parent, &name, AtFlags::empty())?;
            self.published = false;
        } else {
            self.restore()?;
        }
        Ok(())
    }
    fn restore(&mut self) -> io::Result<()> {
        if !self.claimed {
            return Ok(());
        }
        if let Some(name) = &self.quarantine {
            move_exclusive(&self.parent, name, &self.name)?;
            self.quarantine = None;
        } else {
            // Earlier commit cleanup may already have released this original inode.
            let mode = Mode::from_raw_mode(self.original.mode as _);
            let name = temporary_name();
            let mut file = File::from(openat(
                &self.parent,
                &name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                mode,
            )?);
            let result = (|| {
                fchmod(&file, mode)?;
                file.write_all(self.original.data.as_bytes())?;
                file.sync_all()?;
                move_exclusive(&self.parent, &name, &self.name)
            })();
            if result.is_err() {
                let _ = unlinkat(&self.parent, &name, AtFlags::empty());
            }
            result?;
        }
        self.claimed = false;
        Ok(())
    }
}
pub(super) fn apply(
    workspace: &Workspace,
    plans: &[Plan],
    states: &BTreeMap<String, State>,
) -> io::Result<()> {
    apply_inner(workspace, plans, states, |_, _| Ok(()))
}
fn apply_inner(
    workspace: &Workspace,
    plans: &[Plan],
    states: &BTreeMap<String, State>,
    mut checkpoint: impl FnMut(&str, usize) -> io::Result<()>,
) -> io::Result<()> {
    for (path, expected) in states {
        if inspect(workspace, path)? != *expected {
            return Err(io::Error::other(format!(
                "file changed during patch validation: {path}"
            )));
        }
    }
    let mut finals = BTreeMap::new();
    for plan in plans {
        if !plan.new.is_empty() {
            finals.insert(plan.new.clone(), plan.state.clone());
        }
        if !plan.old.is_empty() && plan.old != plan.new {
            finals.insert(plan.old.clone(), State::default());
        }
    }
    let mut entries = Vec::new();
    for (path, original) in states {
        let relative = Path::new(path);
        let parent_path = relative
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = workspace
            .open(parent_path)
            .map_err(|e| io::Error::other(format!("destination parent for {path}: {e}")))?;
        if !parent.metadata()?.is_dir() {
            return Err(io::Error::other(format!(
                "destination parent for {path} is not a directory"
            )));
        }
        let name = relative
            .file_name()
            .expect("validated patch path")
            .to_str()
            .unwrap()
            .to_owned();
        if inspect_at(&parent, &name)? != *original {
            return Err(io::Error::other(format!(
                "file changed during patch validation: {path}"
            )));
        }
        entries.push(Entry {
            parent,
            name,
            path: path.clone(),
            original: original.clone(),
            final_state: finals.remove(path).expect("planned final"),
            staged: None,
            quarantine: None,
            claimed: false,
            published: false,
        });
    }
    for entry in &mut entries {
        entry.stage()?;
    }
    let result = (|| {
        for (i, entry) in entries.iter_mut().enumerate() {
            checkpoint("claim", i)?;
            entry.validate_parent(workspace)?;
            entry.claim()?;
        }
        for (i, entry) in entries.iter_mut().enumerate() {
            checkpoint("publish", i)?;
            entry.validate_parent(workspace)?;
            entry.publish()?;
        }
        for (i, entry) in entries.iter_mut().enumerate() {
            checkpoint("cleanup", i)?;
            entry.validate_parent(workspace)?;
            if let Some(name) = &entry.quarantine {
                unlinkat(&entry.parent, name, AtFlags::empty())?;
                entry.quarantine = None;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for entry in entries.iter_mut().rev() {
            if let Err(e) = entry.rollback() {
                failures.push(format!(
                    "{}: {e}; original quarantine: {:?}",
                    entry.path, entry.quarantine
                ));
            }
        }
        if !failures.is_empty() {
            return Err(io::Error::other(format!(
                "{error}; rollback failed safely without overwriting concurrent changes: {}",
                failures.join("; ")
            )));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn setup() -> (
        tempfile::TempDir,
        Workspace,
        Vec<Plan>,
        BTreeMap<String, State>,
    ) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a"), "old a\n").unwrap();
        fs::write(root.path().join("b"), "old b\n").unwrap();
        let workspace = Workspace::new(root.path()).unwrap();
        let files = super::super::parse::parse("*** Begin Patch\n*** Update File: a\n-old a\n+new a\n*** Delete File: b\n*** Add File: c\n+new c\n*** End Patch").unwrap();
        let (plans, states) = super::super::plan(&workspace, files).unwrap();
        (root, workspace, plans, states)
    }
    #[test]
    fn rollback_at_every_transaction_boundary() {
        for phase in ["claim", "publish", "cleanup"] {
            for fail_at in 0..3 {
                let (root, workspace, plans, states) = setup();
                let result = apply_inner(&workspace, &plans, &states, |point, index| {
                    if point == phase && index == fail_at {
                        Err(io::Error::other("injected failure"))
                    } else {
                        Ok(())
                    }
                });
                assert!(result.is_err());
                assert_eq!(
                    fs::read_to_string(root.path().join("a")).unwrap(),
                    "old a\n"
                );
                assert_eq!(
                    fs::read_to_string(root.path().join("b")).unwrap(),
                    "old b\n"
                );
                assert!(!root.path().join("c").exists());
                assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
            }
        }
    }
    #[test]
    fn rollback_refuses_concurrent_output_and_destination() {
        for concurrent in ["a", "c"] {
            let (root, workspace, plans, states) = setup();
            let result = apply_inner(&workspace, &plans, &states, |point, index| {
                if point == "publish" && index == 2 {
                    fs::write(root.path().join(concurrent), "concurrent\n")?;
                    return Err(io::Error::other("injected failure"));
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(
                fs::read_to_string(root.path().join(concurrent)).unwrap(),
                "concurrent\n"
            );
            assert_eq!(
                fs::read_to_string(root.path().join("b")).unwrap(),
                "old b\n"
            );
        }
    }
    #[test]
    fn changed_source_is_restored_without_consuming_it() {
        let (root, workspace, plans, states) = setup();
        let result = apply_inner(&workspace, &plans, &states, |point, index| {
            if point == "claim" && index == 1 {
                fs::write(root.path().join("b"), "concurrent\n")?;
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("a")).unwrap(),
            "old a\n"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("b")).unwrap(),
            "concurrent\n"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn swapped_sources_are_never_followed_or_consumed() {
        for kind in ["symlink", "hardlink", "fifo"] {
            let (root, workspace, plans, states) = setup();
            let outside = tempfile::tempdir().unwrap();
            let secret = outside.path().join("secret");
            fs::write(&secret, "outside\n").unwrap();
            let result = apply_inner(&workspace, &plans, &states, |point, index| {
                if point == "claim" && index == 1 {
                    let path = root.path().join("b");
                    fs::remove_file(&path)?;
                    match kind {
                        "symlink" => std::os::unix::fs::symlink(&secret, &path)?,
                        "hardlink" => fs::hard_link(&secret, &path)?,
                        _ => rustix::fs::mknodat(
                            rustix::fs::CWD,
                            &path,
                            rustix::fs::FileType::Fifo,
                            Mode::from_raw_mode(0o600),
                            0,
                        )?,
                    }
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(fs::read_to_string(&secret).unwrap(), "outside\n");
            assert_eq!(
                fs::read_to_string(root.path().join("a")).unwrap(),
                "old a\n"
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
            let metadata = fs::symlink_metadata(root.path().join("b")).unwrap();
            if kind == "symlink" {
                assert!(metadata.is_symlink());
            }
            if kind == "hardlink" {
                assert_eq!(metadata.nlink(), 2);
            }
        }
    }

    #[test]
    fn parent_swap_rolls_back_through_pinned_descriptors() {
        for phase in ["claim", "publish", "cleanup"] {
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            fs::create_dir(root.path().join("dir")).unwrap();
            fs::write(root.path().join("dir/a"), "old\n").unwrap();
            fs::write(outside.path().join("a"), "outside\n").unwrap();
            let workspace = Workspace::new(root.path()).unwrap();
            let files = super::super::parse::parse(
                "*** Begin Patch\n*** Update File: dir/a\n-old\n+new\n*** End Patch\n",
            )
            .unwrap();
            let (plans, states) = super::super::plan(&workspace, files).unwrap();
            let result = apply_inner(&workspace, &plans, &states, |point, _| {
                if point == phase {
                    fs::rename(root.path().join("dir"), root.path().join("parked"))?;
                    std::os::unix::fs::symlink(outside.path(), root.path().join("dir"))?;
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(
                fs::read_to_string(outside.path().join("a")).unwrap(),
                "outside\n"
            );
            assert_eq!(
                fs::read_to_string(root.path().join("parked/a")).unwrap(),
                "old\n"
            );
            assert_eq!(fs::read_dir(root.path().join("parked")).unwrap().count(), 1);
        }
    }
}
