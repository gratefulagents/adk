use adk_mcp::{
    Error, Limits, Transport,
    transport::{HttpTransport, RemoteOptions},
};
use serde_json::json;
use std::{path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

#[tokio::test]
async fn explicit_ca_trust_preserves_certificate_and_hostname_verification() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp/tls");
    // Public test-only private key; no production credentials are involved.
    let script = r#"
import http.server, ssl, sys, json
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        body = json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'verified':True}}).encode()
        self.send_response(200)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(body)))
        self.end_headers()
        self.wfile.write(body)
server = http.server.HTTPServer(('127.0.0.1',0),Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(sys.argv[1],sys.argv[2])
server.socket = context.wrap_socket(server.socket,server_side=True)
print(server.server_port,flush=True)
server.serve_forever()
"#;
    let mut child = Command::new("python3")
        .args(["-u", "-c", script])
        .arg(fixture.join("server.pem"))
        .arg(fixture.join("server-key.pem"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut port = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut port),
    )
    .await
    .unwrap()
    .unwrap();
    let port = port.trim();
    let options = RemoteOptions {
        tenant_id: "tls-test".into(),
        allow_private_network: true,
        ..Default::default()
    };
    let mut untrusted = HttpTransport::connect(
        "tls",
        &format!("https://127.0.0.1:{port}/mcp"),
        false,
        options.clone(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert!(matches!(
        untrusted.request("ping", json!({})).await,
        Err(Error::ReconciliationRequired { .. })
    ));
    let trusted = RemoteOptions {
        root_certificates: vec![
            reqwest::Certificate::from_pem(&std::fs::read(fixture.join("ca.pem")).unwrap())
                .unwrap(),
        ],
        ..options
    };
    let mut wrong_host = HttpTransport::connect(
        "tls",
        &format!("https://localhost:{port}/mcp"),
        false,
        trusted.clone(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert!(matches!(
        wrong_host.request("ping", json!({})).await,
        Err(Error::ReconciliationRequired { .. })
    ));
    let mut valid = HttpTransport::connect(
        "tls",
        &format!("https://127.0.0.1:{port}/mcp"),
        false,
        trusted,
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        valid.request("ping", json!({})).await.unwrap(),
        json!({"verified":true})
    );
    valid.close().await.unwrap();
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
