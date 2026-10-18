//! Explicit-map legacy environment expansion; never reads ambient variables.
use std::collections::BTreeMap;

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
