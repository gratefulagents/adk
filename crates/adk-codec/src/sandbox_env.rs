//! Standalone legacy environment projections; never read ambient variables.
use std::collections::{BTreeMap, BTreeSet};

pub const SANDBOX_MODE_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX";
pub const SANDBOX_PATH_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX_PATH";
pub const SANDBOX_PATH_PREPEND_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX_PATH_PREPEND";
pub const SANDBOX_PATH_APPEND_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX_PATH_APPEND";
pub const SANDBOX_EXTRA_READ_ONLY_PATHS_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX_EXTRA_RO_PATHS";
pub const SANDBOX_EXTRA_WRITABLE_PATHS_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX_EXTRA_RW_PATHS";
pub const SANDBOX_EXTRA_ENV_ENV: &str = "GRATEFULAGENTS_COMMAND_SANDBOX_EXTRA_ENV";
pub const SANDBOX_EXPOSE_KUBERNETES_SERVICE_ACCOUNT_ENV: &str =
    "GRATEFULAGENTS_COMMAND_SANDBOX_EXPOSE_KUBERNETES_SERVICE_ACCOUNT";
pub const SANDBOX_ALLOW_UNSAFE_READ_ONLY_LOCAL_ENV: &str =
    "GRATEFULAGENTS_COMMAND_SANDBOX_ALLOW_UNSAFE_READONLY_LOCAL";

/// Legacy host configuration names in SDK order, not native execution grants.
/// The source list intentionally excludes the sandbox mode and GOROOT variables.
pub fn config_env_names() -> [String; 8] {
    [
        SANDBOX_PATH_ENV,
        SANDBOX_PATH_PREPEND_ENV,
        SANDBOX_PATH_APPEND_ENV,
        SANDBOX_EXTRA_READ_ONLY_PATHS_ENV,
        SANDBOX_EXTRA_WRITABLE_PATHS_ENV,
        SANDBOX_EXTRA_ENV_ENV,
        SANDBOX_EXPOSE_KUBERNETES_SERVICE_ACCOUNT_ENV,
        SANDBOX_ALLOW_UNSAFE_READ_ONLY_LOCAL_ENV,
    ]
    .map(str::to_owned)
}

/// Expand `$name` and `${name}` using only `environment`, without recursion.
/// Bytes preserve Go strings, including invalid UTF-8 in names and values.
/// This helper does not grant variables to native sandbox executions.
pub fn expand_safe(value: &[u8], environment: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<u8> {
    let mut output = Vec::new();
    let mut rest = value;
    while let Some(dollar) = rest.iter().position(|byte| *byte == b'$') {
        output.extend_from_slice(&rest[..dollar]);
        rest = &rest[dollar + 1..];
        if rest.is_empty() {
            output.push(b'$');
            break;
        }
        let (name, consumed) = if let Some(braced) = rest.strip_prefix(b"{") {
            match braced.iter().position(|byte| *byte == b'}') {
                Some(end) => (&braced[..end], end + 2),
                None => (&b""[..], 1),
            }
        } else if b"*#$@!?-0123456789".contains(&rest[0]) {
            (&rest[..1], 1)
        } else {
            let end = rest
                .iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
                .count();
            (&rest[..end], end)
        };
        if consumed == 0 {
            output.push(b'$');
        } else if let Some(replacement) = environment.get(name).filter(|_| !name.is_empty()) {
            output.extend_from_slice(replacement);
        }
        rest = &rest[consumed..];
    }
    output.extend_from_slice(rest);
    output
}

/// Environment-only projection of legacy sandbox configuration, not execution authority.
#[derive(Clone, Default)]
pub struct EnvironmentConfig {
    pub path: Vec<u8>,
    pub path_prepend: Vec<Vec<u8>>,
    pub path_append: Vec<Vec<u8>>,
    pub extra_env: BTreeMap<Vec<u8>, Vec<u8>>,
    pub go_root: Vec<u8>,
}

/// Legacy configuration data, never automatically applied to native execution.
#[derive(Clone, Default)]
pub struct ConfigProjection {
    pub mode: Vec<u8>,
    pub running_in_kubernetes: bool,
    pub workspace_root: Vec<u8>,
    pub environment: EnvironmentConfig,
    pub extra_read_only_paths: Vec<Vec<u8>>,
    pub extra_writable_paths: Vec<Vec<u8>>,
    pub expose_kubernetes_service_account: bool,
    pub allow_unsafe_read_only_local: bool,
}

/// Parse legacy POSIX configuration from an explicit host environment snapshot.
/// Parsed flags and paths are data, not sandbox grants or native defaults.
pub fn config_from_environment_posix(environment: &BTreeMap<Vec<u8>, Vec<u8>>) -> ConfigProjection {
    let value = |key: &str| {
        environment
            .get(key.as_bytes())
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    let raw_extra = value(SANDBOX_EXTRA_ENV_ENV);
    let separator = if raw_extra.contains(&b'\n') {
        b'\n'
    } else {
        b','
    };
    let mut extra_env = BTreeMap::new();
    for pair in raw_extra.split(|byte| *byte == separator) {
        if let Some(equal) = pair.iter().position(|byte| *byte == b'=') {
            let key = trim_space(&pair[..equal]);
            if valid_key(key) {
                extra_env.insert(key.to_vec(), trim_space(&pair[equal + 1..]).to_vec());
            }
        }
    }
    ConfigProjection {
        mode: value(SANDBOX_MODE_ENV).to_vec(),
        environment: EnvironmentConfig {
            path: value(SANDBOX_PATH_ENV).to_vec(),
            path_prepend: path_list(value(SANDBOX_PATH_PREPEND_ENV)),
            path_append: path_list(value(SANDBOX_PATH_APPEND_ENV)),
            extra_env,
            go_root: value("GOROOT").to_vec(),
        },
        extra_read_only_paths: path_list(value(SANDBOX_EXTRA_READ_ONLY_PATHS_ENV)),
        extra_writable_paths: path_list(value(SANDBOX_EXTRA_WRITABLE_PATHS_ENV)),
        expose_kubernetes_service_account: env_flag(value(
            SANDBOX_EXPOSE_KUBERNETES_SERVICE_ACCOUNT_ENV,
        )),
        allow_unsafe_read_only_local: env_flag(value(SANDBOX_ALLOW_UNSAFE_READ_ONLY_LOCAL_ENV)),
        ..Default::default()
    }
}

fn env_flag(value: &[u8]) -> bool {
    let value = trim_space(value);
    matches!(value, b"1" | b"t" | b"T" | b"true" | b"TRUE" | b"True")
        || matches!(value.to_ascii_lowercase().as_slice(), b"yes" | b"y" | b"on")
}

/// Project the SDK's POSIX base environment from an explicit parent snapshot.
/// This does not read the process environment or configure a native executor.
/// Its output is not a native sandbox grant; native policy owns protected keys.
pub fn safe_environment_map_posix(
    parent: &BTreeMap<Vec<u8>, Vec<u8>>,
    config: &EnvironmentConfig,
) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut environment = BTreeMap::new();
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "TERM",
        "COLORTERM",
        "LANG",
        "TZ",
        "TMPDIR",
    ] {
        if let Some(value) = parent
            .get(key.as_bytes())
            .filter(|value| !trim_space(value).is_empty())
        {
            environment.insert(key.as_bytes().to_vec(), value.clone());
        }
    }
    for (key, value) in parent {
        if key.starts_with(b"LC_") && valid_key(key) {
            environment.insert(key.clone(), value.clone());
        }
    }
    let mut paths = path_list(&config.path);
    if paths.is_empty() {
        paths = environment
            .get(b"PATH".as_slice())
            .map_or_else(Vec::new, |path| path_list(path));
        if paths.is_empty() {
            paths = [b"/usr/local/bin".as_slice(), b"/usr/bin", b"/bin"]
                .map(<[u8]>::to_vec)
                .into();
        }
        paths.splice(
            0..0,
            config
                .path_prepend
                .iter()
                .filter_map(|path| absolute_path(path)),
        );
        paths.extend(
            config
                .path_append
                .iter()
                .filter_map(|path| absolute_path(path)),
        );
    }
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    environment.insert(b"PATH".to_vec(), paths.join(&b':'));
    if let Some(root) = absolute_path(&config.go_root) {
        environment.insert(b"GOROOT".to_vec(), root);
    }
    for (key, value) in &config.extra_env {
        let key = trim_space(key);
        if valid_key(key) {
            let expanded = expand_safe(trim_space(value), &environment);
            environment.insert(key.to_vec(), expanded);
        }
    }
    environment
}

fn valid_key(key: &[u8]) -> bool {
    key.first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && key
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn path_list(value: &[u8]) -> Vec<Vec<u8>> {
    value
        .split(|byte| *byte == b':')
        .filter_map(absolute_path)
        .collect()
}

fn absolute_path(value: &[u8]) -> Option<Vec<u8>> {
    let value = trim_space(value);
    if !value.starts_with(b"/") {
        return None;
    }
    let mut components = Vec::new();
    for component in value.split(|byte| *byte == b'/') {
        match component {
            b"" | b"." => {}
            b".." => {
                components.pop();
            }
            _ => components.push(component),
        }
    }
    let mut result = b"/".to_vec();
    result.extend(components.join(&b'/'));
    Some(result)
}

fn trim_space(mut value: &[u8]) -> &[u8] {
    const SPACES: &[&[u8]] = &[
        b"\t",
        b"\n",
        b"\x0b",
        b"\x0c",
        b"\r",
        b" ",
        "\u{85}".as_bytes(),
        "\u{a0}".as_bytes(),
        "\u{1680}".as_bytes(),
        "\u{2000}".as_bytes(),
        "\u{2001}".as_bytes(),
        "\u{2002}".as_bytes(),
        "\u{2003}".as_bytes(),
        "\u{2004}".as_bytes(),
        "\u{2005}".as_bytes(),
        "\u{2006}".as_bytes(),
        "\u{2007}".as_bytes(),
        "\u{2008}".as_bytes(),
        "\u{2009}".as_bytes(),
        "\u{200a}".as_bytes(),
        "\u{2028}".as_bytes(),
        "\u{2029}".as_bytes(),
        "\u{202f}".as_bytes(),
        "\u{205f}".as_bytes(),
        "\u{3000}".as_bytes(),
    ];
    while let Some(rest) = SPACES.iter().find_map(|space| value.strip_prefix(*space)) {
        value = rest;
    }
    while let Some(rest) = SPACES.iter().find_map(|space| value.strip_suffix(*space)) {
        value = rest;
    }
    value
}
