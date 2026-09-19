use crate::{Capability, json_text, workspace::Workspace};
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use rustix::fs::{AtFlags, FileType, RenameFlags, renameat_with, statat, unlinkat};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::File, io, path::Path, sync::Arc};

pub(crate) fn builtin(capability: &Capability) -> Option<Arc<dyn Tool>> {
    matches!(capability.name.as_str(), "Move" | "Delete").then(|| {
        Arc::new(Lifecycle {
            definition: capability.definition.clone().expect("lifecycle definition"),
        }) as Arc<dyn Tool>
    })
}

struct Lifecycle {
    definition: ToolDefinition,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    path: String,
    source_path: String,
    destination_path: String,
}

fn clean(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("path is required".into());
    }
    if value.len() > 512 {
        return Err(format!(
            "path is too long ({} bytes, limit 512)",
            value.len()
        ));
    }
    if value.starts_with('/') || value.contains(['\\', '\0', '\t', '\n', '\r']) {
        return Err("path must be a relative slash-separated path".into());
    }
    let parts: Vec<_> = value.split('/').filter(|part| !part.is_empty()).collect();
    if parts.iter().any(|part| matches!(*part, "." | "..")) || parts.is_empty() {
        return Err("path traversal is not allowed".into());
    }
    Ok(parts.join("/"))
}

fn parent(workspace: &Workspace, path: &str) -> io::Result<(File, String)> {
    let path = Path::new(path);
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok((
        workspace.open(directory)?,
        path.file_name()
            .expect("clean path")
            .to_string_lossy()
            .into_owned(),
    ))
}

fn quarantine(parent: &File, name: &str) -> io::Result<String> {
    for _ in 0..100 {
        let temporary = format!(".agentsdk-lifecycle-{}", uuid::Uuid::new_v4().simple());
        match renameat_with(parent, name, parent, &temporary, RenameFlags::NOREPLACE) {
            Ok(()) => return Ok(temporary),
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::Error::other("could not allocate quarantine entry"))
}

fn kind(parent: &File, name: &str, path: &str) -> io::Result<FileType> {
    let metadata = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    let kind = FileType::from_raw_mode(metadata.st_mode);
    match kind {
        FileType::RegularFile if metadata.st_nlink != 1 => {
            Err(io::Error::other("file must have exactly one hard link"))
        }
        FileType::RegularFile | FileType::Directory => Ok(kind),
        FileType::Symlink => Err(io::Error::other(format!("target {path} is a symlink"))),
        _ => Err(io::Error::other(format!(
            "target {path} is not a regular file or directory"
        ))),
    }
}

fn mutate(workspace: &Workspace, source: &str, destination: Option<&str>) -> io::Result<()> {
    let (source_parent, source_name) = parent(workspace, source)?;
    let target = destination
        .map(|path| parent(workspace, path))
        .transpose()?;
    let temporary = quarantine(&source_parent, &source_name)?;
    // Validate only after quarantine so an inode swap cannot replace the checked target.
    let result = (|| {
        let kind = kind(&source_parent, &temporary, source)?;
        if let Some((parent, name)) = target {
            renameat_with(
                &source_parent,
                &temporary,
                &parent,
                name,
                RenameFlags::NOREPLACE,
            )?;
        } else {
            let flags = if kind == FileType::Directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            };
            unlinkat(&source_parent, &temporary, flags)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(restore) = renameat_with(
            &source_parent,
            &temporary,
            &source_parent,
            &source_name,
            RenameFlags::NOREPLACE,
        ) {
            return Err(io::Error::other(format!(
                "{error}; target remains quarantined as {temporary:?}: {restore}"
            )));
        }
        return Err(error);
    }
    Ok(())
}

impl Lifecycle {
    fn execute_inner(&self, context: &ToolContext, mut arguments: Value) -> Result<String, String> {
        if arguments.is_null() {
            arguments = json!({});
        }
        if let Some(fields) = arguments.as_object_mut() {
            fields.retain(|_, value| !value.is_null());
        }
        let input: Input =
            serde_json::from_value(arguments).map_err(|e| format!("Invalid input: {e}"))?;
        if self.definition.name == "Move" {
            let source =
                clean(&input.source_path).map_err(|e| format!("Invalid source_path: {e}"))?;
            let destination = clean(&input.destination_path)
                .map_err(|e| format!("Invalid destination_path: {e}"))?;
            if source == destination {
                return Err("source_path and destination_path must differ".into());
            }
            let workspace =
                Workspace::new(&context.work_dir).map_err(|e| format!("Error moving path: {e}"))?;
            mutate(&workspace, &source, Some(&destination))
                .map_err(|e| format!("Error moving path: {e}"))?;
            Ok(json_text(
                &json!({"operation":"move", "source_path":source, "destination_path":destination}),
            )
            .expect("string fields"))
        } else {
            let path = clean(&input.path).map_err(|e| format!("Invalid path: {e}"))?;
            let workspace = Workspace::new(&context.work_dir)
                .map_err(|e| format!("Error deleting path: {e}"))?;
            mutate(&workspace, &path, None).map_err(|e| format!("Error deleting path: {e}"))?;
            Ok(json_text(&json!({"operation":"delete", "path":path})).expect("string fields"))
        }
    }
}

impl Tool for Lifecycle {
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
            let (text, is_error) = match self.execute_inner(context, call.arguments) {
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
