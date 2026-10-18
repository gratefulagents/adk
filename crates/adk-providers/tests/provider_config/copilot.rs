use super::*;

#[tokio::test]
async fn named_copilot_configuration_matches_pinned_protocol_routes_and_scoped_bearer() {
    let cases = fixture()["copilot"].as_array().unwrap().clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server_cases = cases.clone();
    let server = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for case in server_cases {
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
            }));
            let reply = case["reply"].as_str().unwrap();
            let content_type = case["content_type"].as_str().unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        }
        captured
    });
    let config: Config = serde_json::from_value(json!({
        "provider":"anthropic", "api_key":"parent-must-not-inherit",
        "routes":[
            {"provider":"copilot", "prefix":"team", "api_key":"fixture-copilot", "base_url":endpoint},
            {"provider":"copilot", "prefix":"missing", "base_url":endpoint}
        ]
    })).unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let routes = config
        .resolve()
        .unwrap()
        .build(
            |scope, token| {
                Ok(Arc::new(Store {
                    scope: scope.clone(),
                    token: token.cloned(),
                    reads: reads.clone(),
                }))
            },
            Arc::new(NoRefresh),
        )
        .unwrap();
    assert!(reads.lock().unwrap().is_empty());
    for case in &cases {
        routes
            .complete(
                &context(),
                request(&format!("team/{}", case["input_model"].as_str().unwrap())),
            )
            .await
            .unwrap();
    }
    for (observed, case) in server.join().unwrap().into_iter().zip(cases) {
        for field in ["path", "model", "integration", "api_version"] {
            assert_eq!(observed[field], case[field]);
        }
        assert_eq!(observed["authorization"], case["authorization"]);
    }
    {
        let scopes = reads.lock().unwrap();
        assert_eq!(scopes.len(), 3);
        assert!(
            scopes
                .iter()
                .all(|scope| scope.route == "team" && scope.mode == AuthMode::CopilotOAuth)
        );
    }
    let error = routes
        .complete(&context(), request("missing/gpt-4.1"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
    assert_eq!(reads.lock().unwrap().last().unwrap().route, "missing");
}
