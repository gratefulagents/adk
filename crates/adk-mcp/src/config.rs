use crate::Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const CONFIG_FILE_NAME: &str = ".mcp.json";
pub const MAX_CONFIG_BYTES: usize = 1 << 20;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, ServerConfig>,
}
impl Config {
    pub fn servers(&self) -> &BTreeMap<String, ServerConfig> {
        &self.mcp_servers
    }
    pub fn server(&self, name: &str) -> Option<&ServerConfig> {
        self.mcp_servers.get(name)
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    #[serde(default, rename = "type")]
    transport_type: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    url: String,
    enabled: Option<bool>,
    #[serde(default)]
    allow_env: Vec<String>,
    #[serde(default)]
    trust_read_only_hint: bool,
    #[serde(default)]
    allowed_tools: Vec<String>,
}
impl ServerConfig {
    pub fn transport_type(&self) -> &str {
        if self.transport_type.is_empty() {
            "stdio"
        } else {
            &self.transport_type
        }
    }
    pub fn command(&self) -> &str {
        &self.command
    }
    pub fn args(&self) -> &[String] {
        &self.args
    }
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    pub fn allow_env(&self) -> &[String] {
        &self.allow_env
    }
    pub fn trust_read_only_hint(&self) -> bool {
        self.trust_read_only_hint
    }
    pub fn allowed_tools(&self) -> &[String] {
        &self.allowed_tools
    }
    pub fn is_remote(&self) -> bool {
        self.transport_type() != "stdio"
    }
    pub fn tool_allowed(&self, name: &str) -> bool {
        self.allowed_tools.is_empty() || self.allowed_tools.iter().any(|n| n == name)
    }
    pub fn validate(&self) -> Result<(), Error> {
        match self.transport_type() {
            "stdio" if !self.command.trim().is_empty() => {}
            "stdio" => return Err(Error::Config("stdio command is required".into())),
            "streamable-http" | "sse" => {
                self.origin()?;
            }
            _ => return Err(Error::Config("unsupported transport".into())),
        }
        if self
            .env
            .iter()
            .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
        {
            return Err(Error::Config("invalid environment entry".into()));
        }
        Ok(())
    }
    pub fn origin(&self) -> Result<String, Error> {
        let u =
            url::Url::parse(&self.url).map_err(|_| Error::Config("invalid remote URL".into()))?;
        if !matches!(u.scheme(), "http" | "https")
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err(Error::Config("unsafe remote URL".into()));
        }
        Ok(u.origin().ascii_serialization())
    }
    pub fn filtered_env(
        &self,
        inherited: &BTreeMap<String, String>,
        host_allow: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut out = filter_credential_env(inherited, host_allow);
        // Repository opt-ins can narrow, but never widen, the host's credential grant.
        let allowed = self
            .allow_env
            .iter()
            .filter(|n| host_allow.contains(*n))
            .cloned()
            .collect();
        out.extend(filter_credential_env(&self.env, &allowed));
        out
    }
}

pub fn is_credential_env_name(name: &str) -> bool {
    let n = name.trim().to_ascii_uppercase();
    n.starts_with("AWS_")
        || matches!(
            n.as_str(),
            "GH_TOKEN" | "GH_PAT" | "GITHUB_PAT" | "PASSWORD" | "SECRET"
        )
        || ["_API_KEY", "_SECRET", "_TOKEN", "_PASSWORD", "_PASSWD"]
            .iter()
            .any(|s| n.ends_with(s))
        || (n.starts_with("AZURE_") && n.ends_with("KEY"))
        || (n.starts_with("SLACK_") && ["TOKEN", "SECRET", "KEY"].iter().any(|s| n.ends_with(s)))
        || (n.starts_with("NPM_") && n.contains("AUTH"))
}
pub fn filter_credential_env(
    env: &BTreeMap<String, String>,
    host_allow: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    env.iter()
        .filter(|(k, _)| !is_credential_env_name(k) || host_allow.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[derive(Clone)]
pub struct ConfigSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    sha256: Option<[u8; 32]>,
    config: Config,
}
impl ConfigSnapshot {
    pub fn load(work_dir: impl AsRef<Path>) -> Result<Self, Error> {
        Self::from_path(work_dir.as_ref().join(CONFIG_FILE_NAME))
    }
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = std::path::absolute(path)
            .map_err(|_| Error::Config("cannot resolve config path".into()))?;
        let bytes = read_bounded(&path)?;
        let config: Config = match &bytes {
            Some(b) => serde_json::from_slice(b)
                .map_err(|_| Error::Config("invalid config JSON".into()))?,
            None => Config::default(),
        };
        for (name, server) in config.servers() {
            if name.trim().is_empty() {
                return Err(Error::Config("empty server name".into()));
            }
            server.validate()?;
        }
        let sha256 = bytes.as_ref().map(|b| Sha256::digest(b).into());
        Ok(Self {
            path,
            bytes,
            sha256,
            config,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
    pub fn sha256(&self) -> Option<[u8; 32]> {
        self.sha256
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn verify_unchanged(&self) -> Result<(), Error> {
        let bytes = read_bounded(&self.path).map_err(|_| Error::ConfigChanged)?;
        if bytes != self.bytes {
            return Err(Error::ConfigChanged);
        }
        Ok(())
    }
}

#[cfg(unix)]
fn open_config(path: &Path) -> Result<Option<File>, Error> {
    use rustix::fs::{Mode, OFlags, open, openat};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let mut parent = open("/", flags | OFlags::DIRECTORY, Mode::empty())
        .map_err(|_| Error::Config("cannot open config root".into()))?;
    let mut components = path.components().peekable();
    while let Some(part) = components.next() {
        let name = match part {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            _ => return Err(Error::Config("unsafe config path".into())),
        };
        let final_component = components.peek().is_none();
        let options = if final_component {
            flags
        } else {
            flags | OFlags::DIRECTORY
        };
        match openat(&parent, name, options, Mode::empty()) {
            Ok(fd) if final_component => return Ok(Some(File::from(fd))),
            Ok(fd) => parent = fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => {
                return Err(Error::Config(
                    "cannot open config without following symlinks".into(),
                ));
            }
        }
    }
    Err(Error::Config("config must be a regular file".into()))
}
#[cfg(not(unix))]
fn open_config(_: &Path) -> Result<Option<File>, Error> {
    Err(Error::Config(
        "safe config snapshots unavailable on this platform".into(),
    ))
}
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    let Some(file) = open_config(path)? else {
        return Ok(None);
    };
    let meta = file
        .metadata()
        .map_err(|_| Error::Config("cannot inspect config".into()))?;
    if !meta.is_file() {
        return Err(Error::Config("config must be a regular file".into()));
    }
    if meta.len() > MAX_CONFIG_BYTES as u64 {
        return Err(Error::Limit);
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Config("cannot read config".into()))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(Error::Limit);
    }
    Ok(Some(bytes))
}
