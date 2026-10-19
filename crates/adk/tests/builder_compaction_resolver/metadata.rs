use super::*;
use adk::providers::{
    auth::{AuthMode, CredentialStore, Material, Refresh, Scope, Session},
    runtime::MetadataCompactionResolver,
};

struct Credentials(Mutex<usize>);
impl CredentialStore for Credentials {
    fn load<'a>(&'a self, _: &'a Context, _: &'a Scope) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            *self.0.lock().unwrap() += 1;
            Err(Error::new(ErrorCategory::Host, "offline credentials"))
        })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: u64,
        _: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async { panic!("unexpected credential replacement") })
    }
}
impl Refresh for Credentials {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async { panic!("unexpected refresh") })
    }
}

#[tokio::test]
async fn metadata_injection_is_lazy_feature_gated_and_host_override_wins() {
    for feature in [false, true] {
        for host in [false, true] {
            let credentials = Arc::new(Credentials(Mutex::new(0)));
            let session = Arc::new(
                Session::new(
                    Scope::new(
                        "metadata",
                        "https://example.invalid/v1",
                        None,
                        AuthMode::ApiKey,
                    )
                    .unwrap(),
                    credentials.clone(),
                    credentials.clone(),
                )
                .unwrap(),
            );
            let metadata = Arc::new(MetadataCompactionResolver::new(session));
            let resolver = Arc::new(Resolver {
                value: Some((5000, 2500)),
                models: Mutex::new(vec![]),
            });
            let model = Arc::new(ProbeModel {
                requests: Mutex::new(vec![]),
                continuing: false,
            });
            let mut bundle = Builder::new(Config {
                model: "gpt-5-mini".into(),
                work_dir: "".into(),
                features: Some(Features {
                    compaction: feature,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .model("openai", Kind::OpenAi, model.clone())
            .unwrap()
            .compaction_metadata(metadata.clone())
            .runner_config(RunnerConfig {
                compaction_model_resolver: host
                    .then(|| resolver.clone() as Arc<dyn CompactionModelResolver>),
                ..Default::default()
            })
            .build(&context())
            .await
            .unwrap();
            assert_eq!(*credentials.0.lock().unwrap(), 0);
            bundle
                .run(context(), vec![], Arc::new(TestHost))
                .await
                .unwrap();
            assert_eq!(
                *credentials.0.lock().unwrap(),
                usize::from(feature && !host)
            );
            assert_eq!(
                resolver.models.lock().unwrap().len(),
                usize::from(feature && host)
            );
            assert_eq!(model.requests.lock().unwrap().len(), 1);
            bundle.close().await.unwrap();
        }
    }
}
