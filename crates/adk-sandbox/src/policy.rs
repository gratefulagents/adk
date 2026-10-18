use crate::{Backend, Config, Error, Network, OutputMode, Request};
use adk_core::{AccessMode, Context};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    time::Instant,
};

#[cfg(unix)]
pub(crate) const PROTECTED: &[&str] = &[
    ".git/config",
    ".git/hooks",
    ".mcp.json",
    ".codex",
    ".claude",
    ".gemini",
    ".agents",
];
#[cfg(unix)]
pub(crate) const SECRET: &[&str] = &[
    ".ssh",
    ".aws",
    ".azure",
    ".kube",
    ".gnupg",
    ".config",
    ".docker",
    ".codex",
    ".claude",
    ".gemini",
    ".agents",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".env",
];

pub(crate) fn active(context: &Context) -> Result<(), Error> {
    if context.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if context.deadline.is_some_and(|d| d <= Instant::now()) {
        return Err(Error::TimedOut);
    }
    Ok(())
}

pub(crate) fn workspace(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute() || path.components().any(|p| p == Component::ParentDir) {
        return Err(Error::Invalid(
            "workspace must be absolute without '..'".into(),
        ));
    }
    let canonical = path.canonicalize()?;
    if !canonical.is_dir() || canonical.parent().is_none() {
        return Err(Error::Invalid(
            "workspace must be an existing non-root directory".into(),
        ));
    }
    if ["/tmp", "/private", "/private/tmp", "/var", "/private/var"]
        .iter()
        .any(|p| canonical == Path::new(p))
    {
        return Err(Error::Invalid(
            "workspace must not be a shared temporary/system root".into(),
        ));
    }
    #[cfg(unix)]
    for forbidden in [
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/dev", "/proc", "/sys", "/run",
        "/System", "/Library",
    ] {
        let forbidden = Path::new(forbidden);
        if canonical.starts_with(forbidden) || forbidden.starts_with(&canonical) {
            return Err(Error::Invalid("workspace overlaps a system mount".into()));
        }
    }
    Ok(canonical)
}

pub(crate) fn validate(config: &Config, mut request: Request) -> Result<Request, Error> {
    if !request.scratch.is_empty() && request.access != AccessMode::WorkspaceWrite {
        return Err(Error::Invalid(
            "scratch grants require WorkspaceWrite".into(),
        ));
    }
    for scratch in &request.scratch {
        if scratch.path().starts_with(&config.workspace)
            || config.workspace.starts_with(scratch.path())
        {
            return Err(Error::Invalid(
                "scratch directory overlaps workspace".into(),
            ));
        }
    }
    if request
        .timeout
        .is_some_and(|timeout| Instant::now().checked_add(timeout).is_none())
    {
        return Err(Error::Invalid("timeout is not representable".into()));
    }
    if request.cwd.components().any(|p| p == Component::ParentDir) {
        return Err(Error::Invalid("cwd may not contain '..'".into()));
    }
    request.cwd = config.workspace.join(&request.cwd).canonicalize()?;
    if !request.cwd.is_dir() || !request.cwd.starts_with(&config.workspace) {
        return Err(Error::Invalid("cwd escapes workspace".into()));
    }
    if !request.program.is_absolute()
        || request
            .program
            .components()
            .any(|p| p == Component::ParentDir)
    {
        return Err(Error::Invalid(
            "program must be absolute without '..'".into(),
        ));
    }
    request.program = request.program.canonicalize()?;
    if !request.program.is_file() {
        return Err(Error::Invalid("program must be a file".into()));
    }
    if let OutputMode::Pty { rows, cols } = request.output {
        if rows == 0 || cols == 0 {
            return Err(Error::Invalid("PTY dimensions must be nonzero".into()));
        }
    }
    if config.backend == Backend::Local {
        if request.access != AccessMode::FullAccess || request.network != Network::Allow {
            return Err(Error::Invalid(
                "local requires explicit FullAccess and network Allow".into(),
            ));
        }
    } else if request.access == AccessMode::FullAccess {
        return Err(Error::Invalid(
            "FullAccess requires explicit Local backend".into(),
        ));
    }
    environment(&request.env, &config.application_env, Path::new("/tmp"))?;
    Ok(request)
}

pub(crate) fn environment(
    overrides: &BTreeMap<String, String>,
    application: &BTreeMap<String, String>,
    private: &Path,
) -> Result<BTreeMap<String, String>, Error> {
    let mut env = BTreeMap::from([
        ("PATH".into(), "/usr/bin:/bin".into()),
        (
            "HOME".into(),
            private.join("home").to_string_lossy().into_owned(),
        ),
        ("TMPDIR".into(), private.to_string_lossy().into_owned()),
        ("LANG".into(), "C".into()),
        ("TERM".into(), "dumb".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_PAGER".into(), "cat".into()),
        ("PAGER".into(), "cat".into()),
        ("GIT_CONFIG_COUNT".into(), "2".into()),
        ("GIT_CONFIG_KEY_0".into(), "core.fsmonitor".into()),
        ("GIT_CONFIG_VALUE_0".into(), "false".into()),
        ("GIT_CONFIG_KEY_1".into(), "core.hooksPath".into()),
        ("GIT_CONFIG_VALUE_1".into(), "/dev/null".into()),
    ]);
    for (key, value) in application {
        if key.is_empty()
            || !key.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
            })
            || value.contains('\0')
            || env.contains_key(key)
            || key.starts_with("GIT_")
        {
            return Err(Error::Invalid(
                "invalid or policy-owned application environment key/value".into(),
            ));
        }
        env.insert(key.clone(), value.clone());
    }
    for (key, value) in overrides {
        // An allowlist also excludes loader injection, config/search paths,
        // proxy URLs, credential variables, and future unknown secret names.
        if !matches!(
            key.as_str(),
            "LANG" | "LC_ALL" | "LC_CTYPE" | "LC_MESSAGES" | "TERM" | "COLORTERM"
        ) || application.contains_key(key)
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.@".contains(&b))
        {
            return Err(Error::Invalid(format!(
                "unsafe environment override: {key}"
            )));
        }
        env.insert(key.clone(), value.clone());
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_grants_preserve_policy_and_request_boundaries() {
        let application = BTreeMap::from([("CUSTOM".into(), "value with spaces 猫".into())]);
        let environment =
            environment(&BTreeMap::new(), &application, Path::new("/tmp/private")).unwrap();
        assert_eq!(environment["CUSTOM"], "value with spaces 猫");
        assert_eq!(environment["PATH"], "/usr/bin:/bin");
        for key in ["CUSTOM", "OTHER", "PATH", "LD_PRELOAD", "OPENAI_API_KEY"] {
            assert!(
                super::environment(
                    &BTreeMap::from([(key.into(), "replacement".into())]),
                    &application,
                    Path::new("/tmp/private")
                )
                .is_err()
            );
        }
        for key in [
            "PATH",
            "HOME",
            "TMPDIR",
            "GIT_CONFIG_COUNT",
            "",
            "1BAD",
            "A=B",
            "BAD\nKEY",
        ] {
            assert!(
                super::environment(
                    &BTreeMap::new(),
                    &BTreeMap::from([(key.into(), "x".into())]),
                    Path::new("/tmp/private")
                )
                .is_err()
            );
        }
        assert!(
            super::environment(
                &BTreeMap::new(),
                &BTreeMap::from([("CUSTOM".into(), "x\0y".into())]),
                Path::new("/tmp/private")
            )
            .is_err()
        );
        let mut config = Config::new("/tmp/example");
        config.application_env = application;
        let debug = format!("{config:?}");
        assert!(debug.contains("CUSTOM"));
        assert!(!debug.contains("value with spaces"));
    }

    #[test]
    fn environment_rejects_paths_credentials_and_injection() {
        for key in [
            "PATH",
            "HOME",
            "TMPDIR",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "AWS_ACCESS_KEY_ID",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "BASH_ENV",
            "PYTHONPATH",
            "HTTP_PROXY",
            "NEW_UNKNOWN_SECRET",
        ] {
            assert!(
                environment(
                    &BTreeMap::from([(key.into(), "x".into())]),
                    &BTreeMap::new(),
                    Path::new("/tmp/private")
                )
                .is_err(),
                "{key}"
            );
        }
        for value in ["/tmp/locale", "en_US\nEVIL=1", "x\0y", "../x"] {
            assert!(
                environment(
                    &BTreeMap::from([("LANG".into(), value.into())]),
                    &BTreeMap::new(),
                    Path::new("/tmp/private")
                )
                .is_err()
            );
        }
        let env = environment(
            &BTreeMap::new(),
            &BTreeMap::new(),
            Path::new("/tmp/private"),
        )
        .unwrap();
        assert_eq!(env["PATH"], "/usr/bin:/bin");
        assert_eq!(
            env["HOME"],
            Path::new("/tmp/private").join("home").to_string_lossy()
        );
    }
}
