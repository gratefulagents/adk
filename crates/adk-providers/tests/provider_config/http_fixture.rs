use super::*;

pub(super) fn serve(cases: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for case in cases {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let (headers, body) = loop {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length: usize = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .unwrap()
                        .1
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break (
                            headers,
                            serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                                .unwrap(),
                        );
                    }
                }
            };
            let header = |name: &str| {
                headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.trim())
                    .unwrap_or_default()
            };
            captured.push(json!({
                "path":headers.lines().next().unwrap().split_whitespace().nth(1).unwrap(),
                "model":body["model"], "authorization":header("authorization"),
                "integration":header("copilot-integration-id"), "api_version":header("x-github-api-version"),
                "account":header("chatgpt-account-id"),
            }));
            let reply = case["reply"].as_str().unwrap();
            let content_type = case["content_type"].as_str().unwrap_or("text/event-stream");
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        }
        captured
    });
    (endpoint, server)
}
