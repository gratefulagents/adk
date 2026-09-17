use crate::{
    Capability, edit_diff,
    workspace::Workspace,
    write::{atomic_write, clean, resolve_existing},
};
use adk_core::{
    BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use rustix::fs::{Mode, OFlags, open};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    sync::Arc,
};
const MAX_FILE: usize = 5 * 1024 * 1024;

pub(crate) fn builtin(capability: &Capability) -> Option<Arc<dyn Tool>> {
    (capability.name == "Edit").then(|| {
        Arc::new(Edit {
            definition: capability.definition.clone().expect("edit definition"),
            confined: capability.mode == "workspace_write",
        }) as Arc<dyn Tool>
    })
}
struct Edit {
    definition: ToolDefinition,
    confined: bool,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    file_path: String,
    old_string: String,
    new_string: String,
    replace_all: bool,
}
impl Edit {
    fn edit(&self, context: &ToolContext, mut arguments: Value) -> Result<String, String> {
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
        if input.old_string.is_empty() {
            return Err("old_string is required".into());
        }
        if input.old_string == input.new_string {
            return Err("old_string and new_string are identical — if the file already shows the desired text, the edit is already applied; re-read the file before retrying".into());
        }
        let mut path = clean(&context.work_dir.join(&input.file_path));
        let workspace = if self.confined {
            let workspace = Workspace::new(&context.work_dir)
                .map_err(|e| format!("Error resolving file path: {e}"))?;
            let absolute = std::path::absolute(&path)
                .map_err(|e| format!("Error resolving file path: {e}"))?;
            path = resolve_existing(&clean(&absolute))
                .map_err(|e| format!("Error resolving file path: {e}"))?;
            if !path.starts_with(&workspace.root) {
                return Err("Error resolving file path: path is outside the workspace root".into());
            }
            Some(workspace)
        } else {
            None
        };
        let file = if let Some(workspace) = &workspace {
            workspace
                .read_file(path.strip_prefix(&workspace.root).expect("checked root"))
                .map_err(|e| format!("Error reading file: {e}"))?
        } else {
            File::from(
                open(
                    &path,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|e| format!("Error reading file: {e}"))?,
            )
        };
        let metadata = file
            .metadata()
            .map_err(|e| format!("Error reading file: {e}"))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a regular file", path.display()));
        }
        if metadata.len() > MAX_FILE as u64 {
            return Err(format!(
                "file is too large to edit ({} bytes, max {MAX_FILE})",
                metadata.len()
            ));
        }
        let mut content = Vec::new();
        file.take(MAX_FILE as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|e| format!("Error reading file: {e}"))?;
        if content.len() > MAX_FILE {
            return Err(format!("file is too large to edit (> {MAX_FILE} bytes)"));
        }
        let old = input.old_string.as_bytes();
        let new = input.new_string.as_bytes();
        let finder = memchr::memmem::Finder::new(old);
        let count = finder.find_iter(&content).count();
        if count == 0 {
            return Err("old_string not found in file. The match must be byte-exact including whitespace, indentation, and line endings; re-read the relevant lines with read_file and copy them verbatim".into());
        }
        if count > 1 && !input.replace_all {
            return Err(format!(
                "old_string is not unique in file (found {count} times). Use replace_all or provide more context to make it unique."
            ));
        }
        let mut replacement = Vec::new();
        let mut offsets = Vec::new();
        let mut position = 0;
        for offset in finder.find_iter(&content) {
            replacement.extend_from_slice(&content[position..offset]);
            replacement.extend_from_slice(new);
            position = offset + old.len();
            if offsets.len() < 256 {
                offsets.push(offset);
            }
            if !input.replace_all {
                break;
            }
        }
        replacement.extend_from_slice(&content[position..]);
        let diff = edit_diff::render(
            &content,
            old,
            new,
            &offsets,
            count > 256 && input.replace_all,
        );
        let mode = metadata.permissions().mode() & 0o777;
        if let Some(workspace) = &workspace {
            atomic_write(
                workspace,
                path.strip_prefix(&workspace.root).expect("checked root"),
                &replacement,
                Some(mode),
            )
            .map_err(|e| format!("Error writing file: {e}"))?;
        } else {
            let mut file = File::from(
                open(
                    &path,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC
                        | OFlags::NONBLOCK,
                    Mode::from_raw_mode(mode as _),
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
                .and_then(|_| file.write_all(&replacement))
                .map_err(|e| format!("Error writing file: {e}"))?;
        }
        let summary = if input.replace_all {
            format!(
                "Successfully replaced {count} occurrences in {}",
                path.display()
            )
        } else {
            format!("Successfully edited {}", path.display())
        };
        Ok(if diff.is_empty() {
            summary
        } else {
            format!("{summary}\n{diff}")
        })
    }
}
impl Tool for Edit {
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
            let (text, is_error) = match self.edit(context, call.arguments) {
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
