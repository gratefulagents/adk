use crate::{Capability, workspace::Workspace};
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, fchmod, mkdirat, open, openat, renameat, statat, unlinkat,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub(crate) fn builtin(capability: &Capability) -> Option<Arc<dyn Tool>> {
    (capability.name == "Write").then(|| {
        Arc::new(FileWrite {
            definition: capability.definition.clone().expect("write definition"),
            confined: capability.mode == "workspace_write",
        }) as Arc<dyn Tool>
    })
}
struct FileWrite {
    definition: ToolDefinition,
    confined: bool,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    file_path: String,
    content: String,
}

struct Temporary<'a> {
    parent: &'a File,
    name: String,
    active: bool,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = unlinkat(self.parent, &self.name, AtFlags::empty());
        }
    }
}

pub(crate) fn make_parents(workspace: &Workspace, path: &Path) -> io::Result<()> {
    make_parents_mode(workspace, path, 0o755)
}

pub(crate) fn make_parents_mode(workspace: &Workspace, path: &Path, mode: u32) -> io::Result<()> {
    let mut parent = workspace.open(Path::new("."))?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        match mkdirat(&parent, name, Mode::from_raw_mode(mode as _)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        parent = File::from(openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
    }
    Ok(())
}

pub(crate) fn atomic_write(
    workspace: &Workspace,
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
) -> io::Result<()> {
    atomic_write_default(workspace, path, bytes, mode, 0o644)
}

pub(crate) fn atomic_write_default(
    workspace: &Workspace,
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    default_mode: u32,
) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = workspace.open(directory)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("target is not a regular file"))?;
    let existing_mode = match statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => {
            if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
                return Err(io::Error::other("target is not a regular file"));
            }
            Some(metadata.st_mode & 0o777)
        }
        Err(rustix::io::Errno::NOENT) => None,
        Err(error) => return Err(error.into()),
    };
    #[cfg(target_os = "macos")]
    let existing_mode = existing_mode.map(u32::from);
    let mode = Mode::from_raw_mode(mode.or(existing_mode).unwrap_or(default_mode) as _);
    let temporary_name = format!(".agentsdk-write-{}", uuid::Uuid::new_v4().simple());
    let mut file = File::from(openat(
        &parent,
        &temporary_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        mode,
    )?);
    let mut temporary = Temporary {
        parent: &parent,
        name: temporary_name,
        active: true,
    };
    fchmod(&file, mode)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    renameat(&parent, &temporary.name, &parent, name)?;
    temporary.active = false;
    Ok(())
}

pub(crate) fn clean(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if result.file_name().is_some_and(|name| name != "..") {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

pub(crate) fn resolve_existing(path: &Path) -> io::Result<PathBuf> {
    let mut parent = path;
    let mut tail = Vec::new();
    loop {
        match parent.canonicalize() {
            Ok(mut resolved) => {
                for part in tail.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if parent
                    .symlink_metadata()
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    return Err(error);
                }
                tail.push(parent.file_name().ok_or(error)?);
                parent = parent.parent().expect("path with file name");
            }
            Err(error) => return Err(error),
        }
    }
}

impl FileWrite {
    fn write(&self, context: &ToolContext, mut arguments: Value) -> Result<String, String> {
        if arguments.is_null() {
            arguments = json!({});
        }
        if let Some(fields) = arguments.as_object_mut() {
            fields.retain(|_, value| !value.is_null());
        }
        let input: Input =
            serde_json::from_value(arguments).map_err(|e| format!("Invalid input: {e}"))?;
        if input.file_path.is_empty() {
            return Err("file_path is required".into());
        }
        let path = clean(&context.work_dir.join(&input.file_path));
        let written = if self.confined {
            let workspace = Workspace::new(&context.work_dir)
                .map_err(|e| format!("Error resolving file path: {e}"))?;
            let absolute = std::path::absolute(&path)
                .map_err(|e| format!("Error resolving file path: {e}"))?;
            let path = resolve_existing(&clean(&absolute))
                .map_err(|e| format!("Error resolving file path: {e}"))?;
            let relative = path.strip_prefix(&workspace.root).map_err(|_| {
                "Error resolving file path: path is outside the workspace root".to_string()
            })?;
            make_parents(&workspace, relative.parent().unwrap_or(Path::new(".")))
                .map_err(|e| format!("Error creating parent directory: {e}"))?;
            atomic_write(&workspace, relative, input.content.as_bytes(), None)
                .map_err(|e| format!("Error writing file: {e}"))?;
            path
        } else {
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
                .map_err(|e| format!("Error creating directory: {e}"))?;
            let mut file = File::from(
                open(
                    &path,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC
                        | OFlags::NONBLOCK,
                    Mode::from_raw_mode(0o644),
                )
                .map_err(|e| format!("Error writing file: {e}"))?,
            );
            if !file
                .metadata()
                .map_err(|e| format!("Error writing file: {e}"))?
                .is_file()
            {
                return Err("Error writing file: target is not a regular file".into());
            }
            file.set_len(0)
                .and_then(|_| file.write_all(input.content.as_bytes()))
                .map_err(|e| format!("Error writing file: {e}"))?;
            path
        };
        Ok(format!(
            "Successfully wrote {} bytes to {}",
            input.content.len(),
            written.display()
        ))
    }
}
impl Tool for FileWrite {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            let (text, is_error) = match self.write(context, call.arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            Ok(ToolOutput {
                content: vec![Content::Text { text }],
                is_error,
                should_pause: false,
            })
        })
    }
}
