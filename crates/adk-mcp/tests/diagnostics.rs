use adk_mcp::{
    Error, Limits, Transport,
    client::{HostPolicy, ServerPolicy},
    config::ConfigSnapshot,
    connection,
    transport::StdioTransport,
};
use serde_json::json;
use std::{collections::BTreeMap, path::Path, time::Duration};

async fn python(script: &str, env: BTreeMap<String, String>, cap: usize) -> StdioTransport {
    StdioTransport::connect(
        "crashy",
        "/usr/bin/python3",
        &["-u".into(), "-c".into(), script.into()],
        &env,
        Path::new("."),
        Limits {
            max_stderr_bytes: cap,
            timeout: Duration::from_secs(2),
            ..Limits::default()
        },
    )
    .await
    .unwrap()
}

fn unknown() -> Error {
    Error::ReconciliationRequired {
        server: "crashy".into(),
        operation: "initialize".into(),
    }
}

#[tokio::test]
async fn go_equivalent_bounded_tail_survives_failure_and_close() {
    assert_eq!(Limits::default().max_stderr_bytes, 4096);
    let mut transport = python(
        "import os,sys; sys.stdin.readline(); os.write(2,b'abcdefgh'); os.write(2,b'XYZ')",
        BTreeMap::new(),
        8,
    )
    .await;
    assert_eq!(
        transport.request("initialize", json!({})).await,
        Err(unknown())
    );
    assert_eq!(transport.diagnostics().as_deref(), Some("defghXYZ"));
    transport.close().await.unwrap();
    transport.close().await.unwrap();
    assert_eq!(transport.diagnostics().as_deref(), Some("defghXYZ"));
}

#[tokio::test]
async fn stderr_after_stdout_eof_is_drained_with_bounded_grace() {
    let mut transport = python(
        "import os,sys,time; sys.stdin.readline(); os.close(1); time.sleep(.05); os.write(2,b'boom-traceback'); time.sleep(30)",
        BTreeMap::new(), 4096,
    ).await;
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        transport.request("initialize", json!({})),
    )
    .await
    .unwrap();
    assert_eq!(result, Err(unknown()));
    assert_eq!(transport.diagnostics().as_deref(), Some("boom-traceback"));
}

#[tokio::test]
async fn credentials_are_redacted_across_chunks_and_tail_truncation() {
    let secret = "credential-secret-value";
    for cap in [0, 8, 64] {
        let mut transport = python(
            "import os,sys,time; sys.stdin.readline(); s=os.environ['API_TOKEN'].encode(); os.write(2,b'x'*10000+s[:10]); time.sleep(.02); os.write(2,s[10:]+b' end')",
            BTreeMap::from([("API_TOKEN".into(), secret.into())]), cap,
        ).await;
        assert_eq!(
            transport.request("initialize", json!({})).await,
            Err(unknown())
        );
        let diagnostic = transport.diagnostics().unwrap_or_default();
        assert!(diagnostic.len() <= cap);
        assert!(!diagnostic.contains("value"));
        assert!(!diagnostic.contains(secret));
        if cap > 0 {
            assert!(diagnostic.ends_with("**** end"), "{diagnostic:?}");
        }
    }
}

#[tokio::test]
async fn controls_and_invalid_utf8_remain_sanitized_and_byte_bounded() {
    for cap in [1, 2, 3, 8, 64] {
        let mut transport = python(
            r"import os,sys; sys.stdin.readline(); os.write(2,b'\x1b[31mboom\x00\r\t\x7f'+ '\u202e'.encode()+b'\xff'+ 'ééé'.encode())",
            BTreeMap::new(), cap,
        ).await;
        assert_eq!(
            transport.request("initialize", json!({})).await,
            Err(unknown())
        );
        let diagnostic = transport.diagnostics().unwrap_or_default();
        assert!(diagnostic.len() <= cap);
        assert!(!diagnostic.chars().any(char::is_control));
        assert!(!diagnostic.contains('\u{202e}'));
    }
}

#[tokio::test]
async fn normal_close_retains_tail_and_empty_stderr_is_none() {
    for output in ["", "close-trace"] {
        let script = format!(
            "import sys,os,json; r=json.loads(sys.stdin.readline()); os.write(2,b'{output}'); print(json.dumps({{'jsonrpc':'2.0','id':r['id'],'result':{{}}}})); sys.stdin.readline()"
        );
        let mut transport = python(&script, BTreeMap::new(), 4096).await;
        transport.request("initialize", json!({})).await.unwrap();
        transport.close().await.unwrap();
        assert_eq!(
            transport.diagnostics().as_deref(),
            if output.is_empty() {
                None
            } else {
                Some(output)
            }
        );
    }
}

#[tokio::test]
async fn startup_failure_keeps_boom_traceback_host_only_and_error_typed() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".mcp.json"),
        json!({"mcpServers":{"crashy":{
            "command":"/bin/sh", "args":["-c", "echo boom-traceback >&2; exit 3"]
        }}})
        .to_string(),
    )
    .unwrap();
    let snapshot = ConfigSnapshot::load(temp.path()).unwrap();
    let policy = HostPolicy {
        tenant_id: "tenant".into(),
        servers: BTreeMap::from([(
            "crashy".into(),
            ServerPolicy {
                enabled: true,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let failure = connection::connect_with_diagnostics(
        &snapshot,
        "crashy",
        policy.clone(),
        None,
        &BTreeMap::new(),
        temp.path(),
        Limits::default(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(failure.error, unknown());
    assert_eq!(failure.diagnostics(), Some("boom-traceback"));
    for formatted in [
        format!("{failure}"),
        format!("{failure:?}"),
        format!("{failure:#?}"),
    ] {
        assert!(!formatted.contains("boom-traceback"));
    }
    assert_eq!(
        std::error::Error::source(&failure).unwrap().to_string(),
        unknown().to_string()
    );
    let original: Error = connection::connect(
        &snapshot,
        "crashy",
        policy,
        None,
        &BTreeMap::new(),
        temp.path(),
        Limits::default(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(original, unknown());
    assert!(!original.to_string().contains("boom-traceback"));
}

#[tokio::test]
async fn cancellation_during_stderr_grace_kills_and_reaps_child() {
    let mut transport = python(
        "import sys,os,json,time; r=json.loads(sys.stdin.readline()); print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':os.getpid()})); sys.stdin.readline(); os.close(1); time.sleep(30)",
        BTreeMap::new(), 4096,
    ).await;
    let pid = transport.request("pid", json!({})).await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            transport.request("initialize", json!({}))
        )
        .await
        .is_err()
    );
    assert_eq!(
        transport.request("initialize", json!({})).await,
        Err(Error::Closed)
    );
    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let pid = rustix::process::Pid::from_raw(pid.as_i64().unwrap() as i32).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }
    transport.close().await.unwrap();
}
