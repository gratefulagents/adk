pub use crate::builder::{FileConfigSource, load_role_catalog};
use crate::builder::{ModeSpec, RoleSpec};
use crate::host::{ConfigSource, PermissionMode};
use adk_core::{BoxFuture, Context, Error};

impl ConfigSource for FileConfigSource {
    fn permission_mode<'a>(
        &'a self,
        context: &'a Context,
    ) -> BoxFuture<'a, Result<PermissionMode, Error>> {
        Box::pin(async move {
            let mode = self.mode_snapshot(context).await?;
            Ok(
                if mode.is_some_and(|mode| mode.tool_access == "read-only") {
                    PermissionMode::ReadOnly
                } else {
                    PermissionMode::WorkspaceWrite
                },
            )
        })
    }

    fn mode_snapshot<'a>(
        &'a self,
        context: &'a Context,
    ) -> BoxFuture<'a, Result<Option<ModeSpec>, Error>> {
        Box::pin(async move {
            self.active_mode()
                .map(|name| self.get_mode(context, name))
                .transpose()
        })
    }

    fn mode_directive<'a>(&'a self, context: &'a Context) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            Ok(build_mode_directive(
                self.mode_snapshot(context).await?.as_ref(),
            ))
        })
    }

    fn role_catalog<'a>(
        &'a self,
        context: &'a Context,
    ) -> BoxFuture<'a, Result<Vec<RoleSpec>, Error>> {
        Box::pin(async move { self.load_roles(context) })
    }
}

pub fn build_mode_directive(mode: Option<&ModeSpec>) -> String {
    let Some(mode) = mode else {
        return String::new();
    };
    let mut parts = Vec::new();
    let label = if mode.display_name.trim().is_empty() {
        mode.name.trim()
    } else {
        mode.display_name.trim()
    };
    if !label.is_empty() {
        parts.push(format!("Mode: {label}"));
    }
    if !mode.description.trim().is_empty() {
        parts.push(format!("Mode description: {}", mode.description.trim()));
    }
    if matches!(
        mode.tool_access.trim().to_ascii_lowercase().as_str(),
        "read_only" | "readonly" | "read-only" | "analysis"
    ) {
        parts.push("Tool access: read-only. Do not modify files or run mutating commands. Bash is restricted to statically-authorized commands: no command substitution $(...) or backticks, no heredocs, no VAR=value prefixes, no eval/source, no function definitions, no git-mutating subcommands (commit/add/push/merge/rebase/branch/tag/stash/remote), and no gh CLI — use plain literal read commands and the dedicated read tools (read_file, grep, glob, list_files) instead.".into());
    }
    if !mode.instructions.trim().is_empty() {
        parts.push(mode.instructions.trim().into());
    }
    parts.join("\n\n")
}
