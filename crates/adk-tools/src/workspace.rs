use std::{
    fs::File,
    io,
    path::{Component, Path, PathBuf},
};

pub(crate) struct Workspace {
    pub root: PathBuf,
    #[cfg(target_os = "linux")]
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "secure workspace opens require openat2",
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "secure workspace opens require openat2",
            ))
        }
    }

    pub fn entries(&self, path: &Path) -> io::Result<Vec<Entry>> {
        #[cfg(target_os = "linux")]
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "secure workspace opens require openat2",
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
