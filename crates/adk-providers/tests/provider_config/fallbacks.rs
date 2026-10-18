use super::*;
use adk_core::StreamingModel;

#[tokio::test]
async fn configured_fallbacks_match_pinned_factory_wire_without_cross_route_inheritance() {
    let cases = fixture()["fallbacks"].as_array().unwrap().clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let count = cases.len() * 4;
    let server = std::thread::spawn(move || {
        let mut bodies = Vec::new();
        for _ in 0..count {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes[..end].to_vec())
                        .unwrap()
                        .to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        bodies.push(
                            serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                                .unwrap(),
                        );
                        break;
                    }
                }
            }
            let response = r#"{"id":"fixture","model":"primary","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        }
        bodies
    });
    for case in &cases {
        let named = case["kind"] == "named";
        let mut value = json!({"provider":if named {"openrouter"} else {case["kind"].as_str().unwrap()}, "api_key":"fixture", "base_url":endpoint, "model_fallbacks":case["configured"]});
        if named {
            value["routes"] = json!([{"provider":"openrouter", "prefix":"named", "api_key":"fixture", "base_url":endpoint}]);
        }
        let config: Config = serde_json::from_value(value).unwrap();
        let resolved = config.resolve().unwrap();
        let routes = resolved
            .build(
                |scope, token| {
                    Ok(Arc::new(Store {
                        scope: scope.clone(),
                        token: token.cloned(),
                        reads: Arc::new(Mutex::new(Vec::new())),
                    }))
                },
                Arc::new(NoRefresh),
            )
            .unwrap();
        let name = if named { "named/primary" } else { "primary" };
        routes.complete(&context(), request(name)).await.unwrap();
        let stream_context = context();
        drop(routes.stream(&stream_context, request(name)).await.unwrap());
        let mut overridden = request(name);
        overridden
            .settings
            .insert("model_fallbacks".into(), json!([]));
        routes.complete(&context(), overridden).await.unwrap();
        let mut responses_config = config.clone();
        responses_config.api_mode = "responses".into();
        for route in &mut responses_config.routes {
            route.api_mode = "responses".into();
        }
        let responses_routes = responses_config
            .resolve()
            .unwrap()
            .build(
                |scope, token| {
                    Ok(Arc::new(Store {
                        scope: scope.clone(),
                        token: token.cloned(),
                        reads: Arc::new(Mutex::new(Vec::new())),
                    }))
                },
                Arc::new(NoRefresh),
            )
            .unwrap();
        drop(
            responses_routes
                .stream(&stream_context, request(name))
                .await
                .unwrap(),
        );
    }
    for (bodies, case) in server.join().unwrap().chunks_exact(4).zip(cases) {
        for body in &bodies[..2] {
            assert_eq!(
                body.get("models").is_some(),
                case["present"].as_bool().unwrap()
            );
            assert_eq!(
                body.get("models").cloned().unwrap_or(Value::Null),
                case["models"]
            );
        }
        assert_eq!(bodies[1]["stream"], true);
        assert!(bodies[2].get("models").is_none());
        assert!(bodies[3].get("models").is_none());
    }
}
