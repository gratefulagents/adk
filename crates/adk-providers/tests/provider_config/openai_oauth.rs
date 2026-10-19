use super::*;

struct WrongAccount(Store);
impl CredentialStore for WrongAccount {
    fn load<'a>(
        &'a self,
        context: &'a Context,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            let mut material = self.0.load(context, scope).await?;
            material.account = Some("wrong-account".into());
            Ok(material)
        })
    }
    fn replace<'a>(
        &'a self,
        context: &'a Context,
        scope: &'a Scope,
        revision: u64,
        material: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        self.0.replace(context, scope, revision, material)
    }
}

#[tokio::test]
async fn host_owned_openai_oauth_configuration_preserves_named_account_scope() {
    let case = fixture()["openai_oauth"].clone();
    let (endpoint, server) = super::http_fixture::serve(vec![case.clone()]);
    let config: Config = serde_json::from_value(json!({
        "provider":"openai", "auth_mode":"oauth", "openai_oauth_account_id":" parent-account ",
        "routes":[
            {"provider":"openai", "prefix":"selected", "auth_mode":"oauth", "openai_oauth_account_id":" selected-account ", "base_url":endpoint},
            {"provider":"openai", "prefix":"missing", "auth_mode":"oauth", "base_url":endpoint},
            {"provider":"openai", "prefix":"key", "api_key":"fixture-key"}
        ]
    })).unwrap();
    let resolved = config.resolve().unwrap();
    let parent = resolved
        .routes
        .iter()
        .find(|route| route.spec.prefix() == "openai")
        .unwrap();
    assert_eq!(
        parent.spec.scope().unwrap().endpoint,
        "https://chatgpt.com/backend-api/codex"
    );
    assert_eq!(parent.spec.account.as_deref(), Some("parent-account"));
    let ordinary = resolved
        .routes
        .iter()
        .find(|route| route.spec.prefix() == "key")
        .unwrap();
    assert_eq!(ordinary.spec.mode, AuthMode::ApiKey);
    assert!(ordinary.spec.account.is_none());
    let reads = Arc::new(Mutex::new(Vec::new()));
    let routes = resolved
        .build(
            |scope, seed| {
                if scope.mode == AuthMode::OpenAiOAuth {
                    assert!(seed.is_none());
                }
                Ok(Arc::new(Store {
                    scope: scope.clone(),
                    token: Some(
                        seed.cloned()
                            .unwrap_or_else(|| Secret::new("fixture-openai-oauth")),
                    ),
                    reads: reads.clone(),
                }))
            },
            Arc::new(NoRefresh),
        )
        .unwrap();
    assert!(reads.lock().unwrap().is_empty());
    routes
        .complete(&context(), request("selected/gpt-5.2"))
        .await
        .unwrap();
    let observed = server.join().unwrap().pop().unwrap();
    for field in ["path", "model", "authorization", "account"] {
        assert_eq!(observed[field], case[field]);
    }
    {
        let scopes = reads.lock().unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].route, "selected");
        assert_eq!(scopes[0].account.as_deref(), Some("selected-account"));
        assert_eq!(scopes[0].mode, AuthMode::OpenAiOAuth);
    }
    let error = routes
        .complete(&context(), request("missing/gpt-5.2"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
    assert!(reads.lock().unwrap().last().unwrap().account.is_none());
    let wrong_account_routes = resolved
        .build(
            |scope, _| {
                Ok(Arc::new(WrongAccount(Store {
                    scope: scope.clone(),
                    token: Some(Secret::new("fixture-openai-oauth")),
                    reads: reads.clone(),
                })))
            },
            Arc::new(NoRefresh),
        )
        .unwrap();
    let error = wrong_account_routes
        .complete(&context(), request("selected/gpt-5.2"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
    assert_eq!(error.info.message, "provider credential account mismatch");
}
