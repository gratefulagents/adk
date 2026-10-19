use super::*;

#[tokio::test]
async fn named_copilot_configuration_matches_pinned_protocol_routes_and_scoped_bearer() {
    let cases = fixture()["copilot"].as_array().unwrap().clone();
    let (endpoint, server) = super::http_fixture::serve(cases.clone());
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
