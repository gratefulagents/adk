use std::{
    fs::File,
    io,
    path::{Component, Path, PathBuf},
};

pub(crate) struct Workspace {
    pub root: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    directory: File,
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub name: String,
    pub directory: bool,
    pub symlink: bool,
}

impl Workspace {
    pub fn new(root: &Path) -> io::Result<Self> {
        if root.as_os_str().is_empty() {
            return Err(io::Error::other("workspace root is required"));
        }
        let root = root.canonicalize()?;
        #[cfg(target_os = "linux")]
        {
            use rustix::fs::{Mode, OFlags, open};
            let directory = File::from(open(
                &root,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )?);
            Ok(Self { root, directory })
        }
        #[cfg(target_os = "macos")]
        {
            use rustix::fs::{Mode, OFlags, open};
            let slash = File::from(open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )?);
            let directory = walk(
                &slash,
                root.strip_prefix("/").expect("canonical root"),
                true,
            )?;
            Ok(Self { root, directory })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "secure workspace filesystem is unsupported on this platform",
            ))
        }
    }

    pub fn relative(&self, input: &str) -> io::Result<PathBuf> {
        let input = input.trim();
        let mut path = PathBuf::new();
        let input = Path::new(if input.is_empty() { "." } else { input });
        let input = if input.is_absolute() {
            input.strip_prefix(&self.root).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "path is outside the workspace root",
                )
            })?
        } else {
            input
        };
        for component in input.components() {
            match component {
                Component::Normal(part) => path.push(part),
                Component::CurDir => {}
                Component::ParentDir if path.pop() => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "path is outside the workspace root",
                    ));
                }
            }
        }
        if path.as_os_str().is_empty() {
            path.push(".");
        }
        Ok(path)
    }

    pub fn open(&self, path: &Path) -> io::Result<File> {
        #[cfg(target_os = "linux")]
        {
            use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
            Ok(File::from(openat2(
                &self.directory,
                path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
            )?))
        }
        #[cfg(target_os = "macos")]
        {
            if path.as_os_str().is_empty() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "empty path"));
            }
            walk(&self.directory, path, false)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "secure workspace filesystem is unsupported on this platform",
            ))
        }
    }

    pub fn entries(&self, path: &Path) -> io::Result<Vec<Entry>> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use rustix::fs::{AtFlags, Dir, FileType, statat};
            let directory = self.open(path)?;
            let mut entries = Vec::new();
            for entry in Dir::read_from(&directory)? {
                let entry = entry?;
                let name = entry.file_name();
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    continue;
                }
                let kind = FileType::from_raw_mode(
                    statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW)?.st_mode,
                );
                entries.push(Entry {
                    name: name.to_string_lossy().into_owned(),
                    directory: kind == FileType::Directory,
                    symlink: kind == FileType::Symlink,
                });
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "secure workspace filesystem is unsupported on this platform",
            ))
        }
    }

    pub fn read_file(&self, path: &Path) -> io::Result<File> {
        let file = self.open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::other("not a regular file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "hard-linked file refused",
                ));
            }
        }
        Ok(file)
    }
}

#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
fn walk(directory: &File, path: &Path, directory_only: bool) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags, openat};
    use std::os::unix::ffi::OsStrExt;

    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "path must stay beneath the workspace descriptor",
        ));
    }
    let mut directory = directory.try_clone()?;
    let mut components = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .peekable();
    let bytes = path.as_os_str().as_bytes();
    let directory_only = directory_only || bytes.ends_with(b"/") || bytes.ends_with(b"/.");
    while let Some(name) = components.next() {
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
        if directory_only || components.peek().is_some() {
            flags |= OFlags::DIRECTORY;
        }
        // Each lookup is one component beneath an owned descriptor: ancestor swaps
        // cannot redirect a later lookup through a symlink.
        directory = File::from(openat(&directory, name, flags, Mode::empty())?);
    }
    Ok(directory)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::{fs, io::Read, os::unix::fs::symlink};

    #[test]
    fn descriptor_walk_refuses_links_and_nonrelative_components() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("nested/file"), "inside").unwrap();
        fs::write(outside.path().join("file"), "outside").unwrap();
        symlink(outside.path(), root.path().join("alias")).unwrap();
        symlink(outside.path().join("file"), root.path().join("link")).unwrap();
        let directory = File::open(root.path()).unwrap();
        for path in [
            "alias/file",
            "link",
            "../file",
            "nested/../file",
            "/etc/passwd",
            "nested/file/",
            "nested/file/.",
        ] {
            assert!(walk(&directory, Path::new(path), false).is_err(), "{path}");
        }
        let mut content = String::new();
        walk(&directory, Path::new("./nested/file"), false)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "inside");
        assert!(
            walk(&directory, Path::new("."), true)
                .unwrap()
                .metadata()
                .unwrap()
                .is_dir()
        );
    }

    #[test]
    fn workspace_pins_root_lists_links_and_refuses_hardlinked_reads() {
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file"), "inside").unwrap();
        fs::write(outside.path().join("file"), "outside").unwrap();
        fs::hard_link(outside.path().join("file"), root.join("hard")).unwrap();
        symlink(outside.path(), root.join("link")).unwrap();
        let workspace = Workspace::new(&root).unwrap();
        fs::rename(&root, base.path().join("parked")).unwrap();
        symlink(outside.path(), &root).unwrap();
        let mut content = String::new();
        workspace
            .read_file(Path::new("file"))
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "inside");
        assert!(workspace.read_file(Path::new("hard")).is_err());
        assert!(workspace.read_file(Path::new("link/file")).is_err());
        let entries = workspace.entries(Path::new(".")).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["file", "hard", "link"]
        );
        assert!(entries[2].symlink);
        assert!(!entries[2].directory);
    }

    #[test]
    fn descriptor_walk_resists_ancestor_symlink_swaps() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("parent")).unwrap();
        fs::write(root.path().join("parent/file"), "inside").unwrap();
        fs::write(outside.path().join("file"), "outside").unwrap();
        let directory = File::open(root.path()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let parent = root.path().join("parent");
        let parked = root.path().join("parked");
        let target = outside.path().to_path_buf();
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                fs::rename(&parent, &parked).unwrap();
                symlink(&target, &parent).unwrap();
                std::thread::yield_now();
                fs::remove_file(&parent).unwrap();
                fs::rename(&parked, &parent).unwrap();
            }
        });
        let mut contents = Vec::new();
        for _ in 0..1000 {
            if let Ok(mut file) = walk(&directory, Path::new("parent/file"), false) {
                let mut content = String::new();
                file.read_to_string(&mut content).unwrap();
                contents.push(content);
            }
        }
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        assert!(contents.iter().all(|content| content == "inside"));
    }
}
