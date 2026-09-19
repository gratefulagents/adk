use adk_mcp::{
    Error,
    config::{ConfigSnapshot, MAX_CONFIG_BYTES, ServerConfig, is_credential_env_name},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

#[test]
fn snapshot_pins_original_bytes_and_detects_mutation_deletion_creation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".mcp.json");
    let missing = ConfigSnapshot::load(dir.path()).unwrap();
    assert!(missing.bytes().is_none());
    missing.verify_unchanged().unwrap();
    let bytes =
        br#"{ "mcpServers": {"local":{"command":"echo","args":["hello"],"enabled":true}} }"#;
    fs::write(&path, bytes).unwrap();
    assert_eq!(missing.verify_unchanged(), Err(Error::ConfigChanged));
    let snapshot = ConfigSnapshot::load(dir.path()).unwrap();
    assert_eq!(snapshot.bytes(), Some(bytes.as_slice()));
    assert_eq!(
        snapshot.sha256(),
        Some(<[u8; 32]>::from(Sha256::digest(bytes)))
    );
    assert_eq!(snapshot.config().server("local").unwrap().command(), "echo");
    snapshot.verify_unchanged().unwrap();
    fs::write(&path, b"{}").unwrap();
    assert_eq!(snapshot.verify_unchanged(), Err(Error::ConfigChanged));
    assert_eq!(snapshot.config().server("local").unwrap().command(), "echo");
    fs::remove_file(&path).unwrap();
    assert_eq!(snapshot.verify_unchanged(), Err(Error::ConfigChanged));
}

#[test]
fn config_is_bounded_and_rejects_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".mcp.json");
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/mcp/config-malicious.json")).unwrap();
    for case in cases {
        fs::write(&path, serde_json::to_vec(&case["config"]).unwrap()).unwrap();
        assert!(
            ConfigSnapshot::load(dir.path()).is_err(),
            "{}",
            case["name"]
        );
    }
    fs::write(&path, vec![b' '; MAX_CONFIG_BYTES + 1]).unwrap();
    assert!(matches!(
        ConfigSnapshot::load(dir.path()),
        Err(Error::Limit)
    ));
    fs::write(&path, b"{invalid").unwrap();
    assert!(matches!(
        ConfigSnapshot::load(dir.path()),
        Err(Error::Config(_))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_file_and_parent_paths_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join(".mcp.json"), b"{}").unwrap();
    symlink(real.join(".mcp.json"), dir.path().join(".mcp.json")).unwrap();
    assert!(ConfigSnapshot::load(dir.path()).is_err());
    symlink(&real, dir.path().join("alias")).unwrap();
    assert!(ConfigSnapshot::load(dir.path().join("alias")).is_err());
    let snapshot = ConfigSnapshot::load(&real).unwrap();
    fs::remove_file(real.join(".mcp.json")).unwrap();
    symlink(dir.path().join("absent"), real.join(".mcp.json")).unwrap();
    assert_eq!(snapshot.verify_unchanged(), Err(Error::ConfigChanged));
}

#[test]
fn environment_repository_opt_in_cannot_widen_host_grants() {
    let config: ServerConfig = serde_json::from_value(json!({"command":"echo","env":{"GITHUB_TOKEN":"repo-secret","AWS_KEY":"repo-aws","NORMAL":"override"},"allowEnv":["GITHUB_TOKEN","AWS_KEY"]})).unwrap();
    let inherited: BTreeMap<String, String> = [
        ("GITHUB_TOKEN", "host-secret"),
        ("AWS_KEY", "host-aws"),
        ("NORMAL", "normal"),
        ("PATH", "/bin"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let none = config.filtered_env(&inherited, &BTreeSet::new());
    assert_eq!(none.len(), 2);
    assert_eq!(none["NORMAL"], "override");
    let host = BTreeSet::from(["GITHUB_TOKEN".into()]);
    let allowed = config.filtered_env(&inherited, &host);
    assert_eq!(allowed["GITHUB_TOKEN"], "repo-secret");
    assert!(!allowed.contains_key("AWS_KEY"));
    let config: ServerConfig = serde_json::from_value(
        json!({"command":"echo","env":{"GITHUB_TOKEN":"unapproved-override"}}),
    )
    .unwrap();
    assert_eq!(
        config.filtered_env(&inherited, &host)["GITHUB_TOKEN"],
        "host-secret"
    );
    for name in [
        "aws_REGION",
        "GH_PAT",
        "GITHUB_PAT",
        "NPM_CONFIG_AUTH",
        "SLACK_FOO_KEY",
        "AZURE_SIGNINGKEY",
        "X_TOKEN",
        "X_SECRET",
        "X_PASSWORD",
        "X_PASSWD",
        "OPENAI_API_KEY",
        "PASSWORD",
        "SECRET",
    ] {
        assert!(is_credential_env_name(name), "{name}");
    }
    assert!(!is_credential_env_name("PATH"));
}

#[test]
fn sdk_fields_defaults_and_private_snapshot_getters() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".mcp.json"), serde_json::to_vec(&json!({"mcpServers":{"local":{"command":"tool","args":["a"],"env":{"NORMAL":"value"},"allowEnv":[],"trustReadOnlyHint":true,"allowedTools":["read"]},"remote":{"type":"sse","url":"https://example.com:443/mcp","enabled":false}}})).unwrap()).unwrap();
    let snapshot = ConfigSnapshot::load(dir.path()).unwrap();
    let local = snapshot.config().server("local").unwrap();
    assert_eq!(local.transport_type(), "stdio");
    assert!(local.enabled());
    assert!(local.trust_read_only_hint());
    assert!(local.tool_allowed("read"));
    assert!(!local.tool_allowed("Read"));
    let remote = snapshot.config().server("remote").unwrap();
    assert!(!remote.enabled());
    assert_eq!(remote.origin().unwrap(), "https://example.com");
}
