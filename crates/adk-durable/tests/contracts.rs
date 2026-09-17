use adk_durable::*;
use chrono::Utc;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/durable")
            .join(name),
    )
    .unwrap()
}
fn snapshot(tenant: &str, run: &str) -> RunSnapshot {
    RunSnapshot::new(tenant.into(), run.into(), Utc::now())
}
#[derive(Clone)]
struct Xor;
impl Encryptor for Xor {
    fn encrypt(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        Ok(bytes.iter().map(|b| b ^ 0x5a).collect())
    }
    fn decrypt(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        self.encrypt(bytes)
    }
}
fn encrypted() -> StoreOptions {
    StoreOptions {
        encryptor: Some(Arc::new(Xor)),
        ..Default::default()
    }
}

#[test]
fn go_fixtures_migration_full_types_and_recovery() {
    let migrated = decode_document(&fixture("v1.json")).unwrap();
    let expected = decode_document(&fixture("v1-migrated.json")).unwrap();
    assert_eq!(migrated, expected);
    assert_eq!(migrated.snapshot.cumulative_budget.input_tokens, 123);
    assert_eq!(migrated.events[0].sequence, 1);
    assert_eq!(migrated.snapshot.status, RunStatus::Pending);
    let doc = decode_document(&fixture("v2.json")).unwrap();
    assert_eq!(
        decode_document(&encode_document(&doc).unwrap()).unwrap(),
        doc
    );
    assert_eq!(
        doc.snapshot.state.as_ref().unwrap()["large"].as_u64(),
        Some(u64::MAX)
    );
    assert_eq!(
        doc.snapshot.cumulative_budget.input_tokens,
        9007199254740993
    );
    assert_eq!(doc.snapshot.attempts[0].ended_at, zero_time());
    assert_eq!(doc.snapshot.created_at.timestamp_subsec_nanos(), 123456789);
    let matrix: Vec<Value> = serde_json::from_slice(&fixture("recovery.json")).unwrap();
    for case in matrix {
        let effect: Effect = serde_json::from_value(case["effect"].clone()).unwrap();
        let decision: RecoveryDecision = serde_json::from_value(case["decision"].clone()).unwrap();
        assert_eq!(recover_effect(&effect), decision);
        assert_eq!(
            effect.idempotency_key,
            idempotency_key(&"run_go".into(), &effect.id)
        );
    }
}
#[test]
fn schema_and_effects_fail_closed() {
    let null: Event = serde_json::from_value(json!({"payload":null})).unwrap();
    assert_eq!(null.payload, Some(Value::Null));
    assert!(
        serde_json::to_value(null)
            .unwrap()
            .get("payload")
            .unwrap()
            .is_null()
    );
    for bytes in [
        b"{}".as_slice(),
        b"{\"schema_version\":0}",
        b"{\"schema_version\":3}",
        b"{\"schema_version\":-1}",
        b"{\"schema_version\":2",
    ] {
        assert!(decode_document(bytes).is_err());
    }
    let mut value: Value = serde_json::from_slice(&fixture("v2.json")).unwrap();
    value["snapshot"]["schema_version"] = json!(99);
    assert!(matches!(
        decode_document(&serde_json::to_vec(&value).unwrap()),
        Err(Error::UnsupportedSchema(99))
    ));
    value["snapshot"]["schema_version"] = json!(2);
    value["snapshot"]["effects"][0]["classification"] = json!("future-danger");
    assert!(decode_document(&serde_json::to_vec(&value).unwrap()).is_err());
    use EffectState::*;
    for initial in [Prepared, Dispatched, Succeeded, Failed, OutcomeUnknown] {
        for next in [Prepared, Dispatched, Succeeded, Failed, OutcomeUnknown] {
            let mut effect = Effect::new(
                &RunId::new(),
                EffectClassification::NonReplayable,
                Utc::now(),
            );
            effect.state = initial;
            let before = effect.clone();
            let valid = matches!(
                (initial, next),
                (Prepared, Dispatched | Failed)
                    | (Dispatched, Succeeded | Failed | OutcomeUnknown)
                    | (OutcomeUnknown, Succeeded | Failed)
            );
            assert_eq!(
                transition_effect(&mut effect, next, Utc::now()).is_ok(),
                valid
            );
            if !valid {
                assert_eq!(effect, before);
            }
        }
    }
    let mut effect = Effect::new(
        &RunId::new(),
        EffectClassification::NonReplayable,
        Utc::now(),
    );
    transition_effect(&mut effect, Dispatched, Utc::now()).unwrap();
    assert_eq!(recover_effect(&effect).action, RecoveryAction::Reconcile);
    mark_interrupted_effect(&mut effect, Utc::now()).unwrap();
    assert_eq!(
        recover_effect(&effect).action,
        RecoveryAction::OperatorResolution
    );
    assert!(!recover_effect(&effect).automatic);
}

fn store_contract(store: &dyn RunStore, tenant: &str) {
    let t = TenantId::from(tenant);
    let other = TenantId::from(format!("{tenant}_other"));
    let r = RunId::from("shared");
    let original = snapshot(t.as_str(), r.as_str());
    store.create(original.clone()).unwrap();
    store.create(snapshot(other.as_str(), r.as_str())).unwrap();
    assert!(matches!(
        store.create(original.clone()),
        Err(Error::AlreadyExists)
    ));
    assert!(matches!(
        store.load(&t, &"missing".into()),
        Err(Error::NotFound)
    ));
    assert!(
        store
            .acquire_lease(&t, &r, "", Duration::from_secs(1))
            .is_err()
    );
    assert!(store.acquire_lease(&t, &r, "a", Duration::ZERO).is_err());
    let lease = store
        .acquire_lease(&t, &r, "a", Duration::from_secs(60))
        .unwrap();
    assert!(matches!(
        store.acquire_lease(&t, &r, "a", Duration::from_secs(60)),
        Err(Error::LeaseHeld)
    ));
    let lease = store.renew_lease(&lease, Duration::from_secs(60)).unwrap();
    let mut next = original.clone();
    next.revision = 1;
    next.status = RunStatus::Running;
    next.state = Some(json!({"checkpoint":"tool_prepared"}));
    next.cumulative_budget.input_tokens = 42;
    let events = vec![
        Event {
            event_type: "tool.prepared".into(),
            payload: Some(json!({"x":1})),
            ..Default::default()
        },
        Event {
            event_type: "tool.dispatched".into(),
            ..Default::default()
        },
    ];
    let updated = store.append(&lease, 0, events, next.clone()).unwrap();
    assert_eq!(updated.event_sequence, 2);
    assert!(matches!(
        store.append(&lease, 0, vec![], next.clone()),
        Err(Error::Conflict)
    ));
    next.revision = 3;
    assert!(store.append(&lease, 1, vec![], next.clone()).is_err());
    next.revision = 2;
    assert!(
        store
            .append(
                &lease,
                1,
                vec![Event {
                    tenant_id: other.clone(),
                    ..Default::default()
                }],
                next.clone()
            )
            .is_err()
    );
    next.tenant_id = other.clone();
    assert!(store.append(&lease, 1, vec![], next).is_err());
    let (loaded, events) = store.load(&t, &r).unwrap();
    assert_eq!(loaded, updated);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[1].sequence, 2);
    assert_eq!(events[0].payload, Some(json!({"x":1})));
    assert!(!events[0].id.is_empty());
    assert_eq!(store.load(&other, &r).unwrap().0.revision, 0);
    store.release_lease(&lease).unwrap();
    let short = store
        .acquire_lease(&t, &r, "short", Duration::from_millis(25))
        .unwrap();
    std::thread::sleep(Duration::from_millis(55));
    assert!(matches!(
        store.renew_lease(&short, Duration::from_secs(1)),
        Err(Error::LeaseLost)
    ));
    let new = store
        .acquire_lease(&t, &r, "new", Duration::from_secs(60))
        .unwrap();
    assert_ne!(short.token, new.token);
    let mut next = updated.clone();
    next.revision = 2;
    assert!(matches!(
        store.append(&short, 1, vec![], next),
        Err(Error::LeaseLost)
    ));
    assert!(matches!(store.release_lease(&short), Err(Error::LeaseLost)));
    store.release_lease(&new).unwrap();
    let now = Utc::now();
    for (name, deadline) in [
        ("expired", Some(now)),
        ("future", Some(now + chrono::Duration::hours(1))),
        ("forever", None),
    ] {
        let mut snap = snapshot(t.as_str(), name);
        snap.retain_until = deadline;
        store.create(snap).unwrap();
    }
    assert_eq!(
        store
            .apply_retention(RetentionPolicy { now: Some(now) })
            .unwrap(),
        1
    );
    assert!(matches!(
        store.load(&t, &"expired".into()),
        Err(Error::NotFound)
    ));
    store.delete_run(&t, &r).unwrap();
    assert!(matches!(store.delete_run(&t, &r), Err(Error::NotFound)));
    store.delete_tenant(&t).unwrap();
    store.delete_tenant(&t).unwrap();
    assert!(matches!(
        store.load(&t, &"future".into()),
        Err(Error::NotFound)
    ));
    assert!(store.load(&other, &r).is_ok());
    store.delete_tenant(&other).unwrap();
}
#[test]
fn filesystem_contract() {
    let temp = tempfile::tempdir().unwrap();
    store_contract(
        &FilesystemStore::new(temp.path(), Default::default()).unwrap(),
        "contract",
    );
}
#[test]
fn filesystem_reads_go_records_and_migrates_embedded_v1() {
    for (file, options) in [
        ("filesystem.json", StoreOptions::default()),
        ("filesystem-encrypted.json", encrypted()),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store = FilesystemStore::new(temp.path(), options).unwrap();
        fs::create_dir(temp.path().join("tenants/tenant_go")).unwrap();
        fs::write(
            temp.path().join("tenants/tenant_go/run_go.json"),
            fixture(file),
        )
        .unwrap();
        let (snap, events) = store.load(&"tenant_go".into(), &"run_go".into()).unwrap();
        assert_eq!(snap, decode_document(&fixture("v2.json")).unwrap().snapshot);
        assert_eq!(events.len(), 1);
        let lease = store
            .acquire_lease(
                &snap.tenant_id,
                &snap.run_id,
                "rust",
                Duration::from_secs(60),
            )
            .unwrap();
        store.release_lease(&lease).unwrap();
    }
    let temp = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(temp.path(), Default::default()).unwrap();
    fs::create_dir(temp.path().join("tenants/tenant_go")).unwrap();
    let old: Value = serde_json::from_slice(&fixture("v1.json")).unwrap();
    fs::write(
        temp.path().join("tenants/tenant_go/run_go.json"),
        serde_json::to_vec(&json!({"document":old})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store
            .load(&"tenant_go".into(), &"run_go".into())
            .unwrap()
            .0
            .revision,
        4
    );
}

struct Failable {
    fail: Arc<AtomicBool>,
}
impl Encryptor for Failable {
    fn encrypt(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        if self.fail.load(Ordering::SeqCst) {
            Err(Error::Protection("injected failure".into()))
        } else {
            Xor.encrypt(bytes)
        }
    }
    fn decrypt(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        Xor.decrypt(bytes)
    }
}
fn fault_contract(store: &dyn RunStore, fail: &AtomicBool, tenant: &str) {
    let mut snap = snapshot(tenant, "fault");
    snap.effects.push(Effect::new(
        &snap.run_id,
        EffectClassification::NonReplayable,
        Utc::now(),
    ));
    store.create(snap.clone()).unwrap();
    let lease = store
        .acquire_lease(
            &snap.tenant_id,
            &snap.run_id,
            "worker",
            Duration::from_secs(60),
        )
        .unwrap();
    for state in [EffectState::Dispatched, EffectState::Succeeded] {
        let mut next = snap.clone();
        next.revision += 1;
        transition_effect(&mut next.effects[0], state, Utc::now()).unwrap();
        fail.store(true, Ordering::SeqCst);
        assert!(
            store
                .append(
                    &lease,
                    snap.revision,
                    vec![Event {
                        event_type: format!("{state:?}"),
                        ..Default::default()
                    }],
                    next.clone()
                )
                .is_err()
        );
        fail.store(false, Ordering::SeqCst);
        assert_eq!(store.load(&snap.tenant_id, &snap.run_id).unwrap().0, snap);
        if state == EffectState::Succeeded {
            let mut recovered = store
                .load(&snap.tenant_id, &snap.run_id)
                .unwrap()
                .0
                .effects
                .remove(0);
            mark_interrupted_effect(&mut recovered, Utc::now()).unwrap();
            assert_eq!(
                recover_effect(&recovered).action,
                RecoveryAction::OperatorResolution
            );
        }
        snap = store
            .append(
                &lease,
                snap.revision,
                vec![Event {
                    event_type: format!("{state:?}"),
                    ..Default::default()
                }],
                next,
            )
            .unwrap();
    }
    assert_eq!(
        store.load(&snap.tenant_id, &snap.run_id).unwrap().1.len(),
        2
    );
    store.delete_tenant(&snap.tenant_id).unwrap();
}
#[test]
fn filesystem_persistence_faults_do_not_cross_effect_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let store = FilesystemStore::new(
        temp.path(),
        StoreOptions {
            encryptor: Some(Arc::new(Failable { fail: fail.clone() })),
            ..Default::default()
        },
    )
    .unwrap();
    fault_contract(&store, &fail, "faults");
}
fn privacy_contract(store: &dyn RunStore, tenant: &str) {
    let mut snap = decode_document(&fixture("v2.json")).unwrap().snapshot;
    snap.tenant_id = tenant.into();
    snap.run_id = "private".into();
    snap.revision = 0;
    snap.event_sequence = 0;
    snap.retain_until = None;
    store.create(snap.clone()).unwrap();
    let lease = store
        .acquire_lease(
            &snap.tenant_id,
            &snap.run_id,
            "private-owner",
            Duration::from_secs(60),
        )
        .unwrap();
    snap.revision = 1;
    store
        .append(
            &lease,
            0,
            vec![Event {
                classification: DataClassification::Secret,
                payload: Some(json!("raw-event")),
                ..Default::default()
            }],
            snap.clone(),
        )
        .unwrap();
    store.renew_lease(&lease, Duration::from_secs(60)).unwrap();
    store.release_lease(&lease).unwrap();
    let (loaded, events) = store.load(&snap.tenant_id, &snap.run_id).unwrap();
    assert_eq!(loaded.state, Some(json!("sensitive")));
    assert_eq!(loaded.steps[0].data, Some(json!("sensitive")));
    assert_eq!(loaded.tool_calls[0].input, Some(json!("secret")));
    assert_eq!(loaded.tool_calls[0].output, Some(json!("secret")));
    assert_eq!(loaded.effects[0].outcome, Some(json!("secret")));
    assert_eq!(events[0].payload, Some(json!("secret")));
}
fn private_options() -> StoreOptions {
    StoreOptions {
        encryptor: Some(Arc::new(Xor)),
        redactor: Some(Arc::new(|class: DataClassification, _: Value| {
            Ok(serde_json::to_value(class).unwrap())
        })),
    }
}
#[test]
fn filesystem_privacy_encryption_downgrade_permissions_and_paths() {
    let temp = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(temp.path(), private_options()).unwrap();
    privacy_contract(&store, "private");
    let path = temp.path().join("tenants/private/private.json");
    let bytes = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-owner"));
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["encrypted"],
        true
    );
    assert!(
        FilesystemStore::new(temp.path(), Default::default())
            .unwrap()
            .load(&"private".into(), &"private".into())
            .is_err()
    );
    for (t, r) in [
        ("../outside", "run"),
        ("tenant", "../outside"),
        ("a/b", "x"),
    ] {
        assert!(store.create(snapshot(t, r)).is_err());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for dir in [
            temp.path().to_path_buf(),
            temp.path().join("tenants"),
            temp.path().join("locks"),
            temp.path().join("tenants/private"),
        ] {
            assert_eq!(
                fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), temp.path().join("tenants/evil")).unwrap();
        assert!(store.create(snapshot("evil", "run")).is_err());
        symlink(&path, temp.path().join("tenants/private/link.json")).unwrap();
        assert!(store.load(&"private".into(), &"link".into()).is_err());
        symlink(&path, temp.path().join("locks/link.lock")).unwrap();
        assert!(store.load(&"link".into(), &"run".into()).is_err());
    }
    let mut record: Value = serde_json::from_slice(&fixture("filesystem.json")).unwrap();
    record["document"]["snapshot"]["tenant_id"] = json!("private");
    record["document"]["snapshot"]["run_id"] = json!("private");
    record.as_object_mut().unwrap().remove("lease");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(matches!(
        store.load(&"private".into(), &"private".into()),
        Err(Error::Protection(_))
    ));
    assert!(
        store
            .acquire_lease(
                &"private".into(),
                &"private".into(),
                "attacker",
                Duration::from_secs(60)
            )
            .is_err()
    );
}

#[test]
fn filesystem_future_and_cross_tenant_records_cannot_be_rewritten() {
    let temp = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(temp.path(), Default::default()).unwrap();
    store.create(snapshot("tenant_go", "run_go")).unwrap();
    let path = temp.path().join("tenants/tenant_go/run_go.json");
    let record: Value = serde_json::from_slice(&fixture("filesystem.json")).unwrap();
    for (field, value) in [
        ("schema", json!(99)),
        ("tenant", json!("other")),
        ("event", json!("other")),
    ] {
        let mut record = record.clone();
        match field {
            "schema" => record["document"]["schema_version"] = value,
            "tenant" => record["document"]["snapshot"]["tenant_id"] = value,
            _ => record["document"]["events"][0]["tenant_id"] = value,
        }
        let bytes = serde_json::to_vec(&record).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(store.load(&"tenant_go".into(), &"run_go".into()).is_err());
        assert!(
            store
                .acquire_lease(
                    &"tenant_go".into(),
                    &"run_go".into(),
                    "writer",
                    Duration::from_secs(60)
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn subprocess_worker() {
    let Ok(root) = std::env::var("ADK_DURABLE_WORKER_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let store = FilesystemStore::new(&root, Default::default()).unwrap();
    while !root.join("start").exists() {
        std::thread::sleep(Duration::from_millis(2));
    }
    match store.acquire_lease(
        &"process".into(),
        &"run".into(),
        "child",
        Duration::from_secs(1),
    ) {
        Ok(_) => println!("ACQUIRED"),
        Err(Error::LeaseHeld) => println!("HELD"),
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn filesystem_cross_process_fencing_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(root.path(), Default::default()).unwrap();
    store.create(snapshot("process", "run")).unwrap();
    let executable = std::env::args_os().next().unwrap();
    let mut children = vec![];
    for _ in 0..4 {
        children.push(
            Command::new(&executable)
                .args(["--exact", "subprocess_worker", "--nocapture"])
                .env("ADK_DURABLE_WORKER_ROOT", root.path())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    fs::write(root.path().join("start"), b"go").unwrap();
    let mut winners = 0;
    for child in children {
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        winners += usize::from(String::from_utf8_lossy(&result.stdout).contains("ACQUIRED"));
    }
    assert_eq!(winners, 1);
    std::thread::sleep(Duration::from_millis(1100));
    let restarted = FilesystemStore::new(root.path(), Default::default()).unwrap();
    let lease = restarted
        .acquire_lease(
            &"process".into(),
            &"run".into(),
            "restart",
            Duration::from_secs(60),
        )
        .unwrap();
    let mut snap = restarted.load(&lease.tenant_id, &lease.run_id).unwrap().0;
    snap.revision = 1;
    restarted
        .append(&lease, 0, vec![Event::default()], snap)
        .unwrap();
    assert_eq!(
        store
            .load(&lease.tenant_id, &lease.run_id)
            .unwrap()
            .0
            .revision,
        1
    );
}

#[test]
fn actual_go_reader_resumes_rust_records() {
    if std::env::var_os("ADK_TEST_GO").is_none() {
        eprintln!("set ADK_TEST_GO=1 for actual Go reader verification");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for (name, options) in [
        ("plain", StoreOptions::default()),
        ("encrypted", encrypted()),
    ] {
        let store = FilesystemStore::new(root.path().join(name), options).unwrap();
        let mut snap = snapshot("tenant_go", "run_go");
        snap.state = Some(json!({"from":"rust","large":u64::MAX}));
        snap.effects.push(Effect::new(
            &snap.run_id,
            EffectClassification::NonReplayable,
            Utc::now(),
        ));
        store.create(snap.clone()).unwrap();
        let lease = store
            .acquire_lease(
                &snap.tenant_id,
                &snap.run_id,
                "rust",
                Duration::from_secs(60),
            )
            .unwrap();
        snap.revision = 1;
        store
            .append(
                &lease,
                0,
                vec![Event {
                    event_type: "rust.prepared".into(),
                    ..Default::default()
                }],
                snap,
            )
            .unwrap();
        store.release_lease(&lease).unwrap();
        let (snapshot, events) = store.load(&"tenant_go".into(), &"run_go".into()).unwrap();
        let path = root.path().join(format!("{name}.json"));
        fs::write(
            &path,
            encode_document(&Document {
                schema_version: 2,
                snapshot,
                events,
            })
            .unwrap(),
        )
        .unwrap();
        go("verify-document", &path);
    }
    go("verify", root.path());
    for (name, options) in [
        ("plain", StoreOptions::default()),
        ("encrypted", encrypted()),
    ] {
        let store = FilesystemStore::new(root.path().join(name), options).unwrap();
        let (snap, events) = store.load(&"tenant_go".into(), &"run_go".into()).unwrap();
        assert_eq!(snap.revision, 2);
        assert_eq!(events[1].event_type, "go.resumed");
    }
}
fn go(mode: &str, path: &Path) {
    let output = Command::new("go")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../repos/sdk"))
        .args(["run", "../../fixtures/durable/generate.go", mode])
        .arg(path)
        .env("GOTOOLCHAIN", "local")
        .env("GOTELEMETRY", "off")
        .env("PGSSLMODE", "disable")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Go: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

#[cfg(feature = "postgres")]
mod pg {
    use super::*;
    use postgres::{Client, NoTls};
    fn setup() -> Option<(String, String)> {
        let Ok(url) = std::env::var("ADK_TEST_POSTGRES_URL") else {
            eprintln!("set ADK_TEST_POSTGRES_URL to run live PostgreSQL tests");
            return None;
        };
        let schema = format!("durable_test_{}", uuid::Uuid::new_v4().simple());
        let mut client = Client::connect(&url, NoTls).unwrap();
        client
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        Some((url, schema))
    }
    fn client(url: &str, schema: &str) -> Client {
        let mut c = Client::connect(url, NoTls).unwrap();
        c.batch_execute(&format!("SET search_path TO {schema}"))
            .unwrap();
        c
    }
    fn store(url: &str, schema: &str, options: StoreOptions) -> PostgresStore {
        PostgresStore::new(client(url, schema), options)
    }
    fn cleanup(url: &str, schema: &str) {
        Client::connect(url, NoTls)
            .unwrap()
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .unwrap();
    }
    #[test]
    fn actual_go_postgres_store_reads_and_resumes_rust_writes() {
        if std::env::var_os("ADK_TEST_GO").is_none() {
            return;
        }
        let Some((url, schema)) = setup() else {
            return;
        };
        let run_go = |mode: &str| {
            let output = Command::new("go")
                .current_dir(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/durable/postgres"),
                )
                .args(["run", ".", mode, &schema])
                .env("GOTOOLCHAIN", "local")
                .env("GOTELEMETRY", "off")
                .env("PGSSLMODE", "disable")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout));
        };
        run_go("seed");
        for (tenant, options) in [
            ("go_pg_plain", StoreOptions::default()),
            ("go_pg_encrypted", encrypted()),
        ] {
            let s = store(&url, &schema, options);
            let t = tenant.into();
            let r = "run_go".into();
            let (mut snap, events) = s.load(&t, &r).unwrap();
            assert_eq!(snap.revision, 1);
            assert_eq!(events.len(), 1);
            let lease = s
                .acquire_lease(&t, &r, "rust", Duration::from_secs(60))
                .unwrap();
            mark_interrupted_effect(&mut snap.effects[0], Utc::now()).unwrap();
            snap.revision = 2;
            snap.cumulative_budget.input_tokens += 7;
            s.append(
                &lease,
                1,
                vec![Event {
                    event_type: "rust.recovered".into(),
                    ..Default::default()
                }],
                snap,
            )
            .unwrap();
            s.release_lease(&lease).unwrap();
        }
        run_go("verify");
        for (tenant, options) in [
            ("go_pg_plain", StoreOptions::default()),
            ("go_pg_encrypted", encrypted()),
        ] {
            let (snap, events) = store(&url, &schema, options)
                .load(&tenant.into(), &"run_go".into())
                .unwrap();
            assert_eq!(snap.revision, 3);
            assert_eq!(events[2].event_type, "go.verified");
        }
        cleanup(&url, &schema);
    }
    #[test]
    fn postgres_process_worker() {
        let Ok(schema) = std::env::var("ADK_DURABLE_PG_SCHEMA") else {
            return;
        };
        let root = PathBuf::from(std::env::var("ADK_DURABLE_PG_GATE").unwrap());
        let s = store(
            &std::env::var("ADK_TEST_POSTGRES_URL").unwrap(),
            &schema,
            Default::default(),
        );
        while !root.join("start").exists() {
            std::thread::sleep(Duration::from_millis(2));
        }
        match s.acquire_lease(
            &"process_pg".into(),
            &"run".into(),
            "child",
            Duration::from_secs(2),
        ) {
            Ok(lease) => {
                fs::write(root.join("lease.json"), serde_json::to_vec(&lease).unwrap()).unwrap();
                println!("ACQUIRED");
            }
            Err(Error::LeaseHeld) => println!("HELD"),
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn postgres_cross_process_restart_expiry_fencing() {
        let Some((url, schema)) = setup() else {
            return;
        };
        let s = store(&url, &schema, Default::default());
        s.init().unwrap();
        s.create(snapshot("process_pg", "run")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut children = vec![];
        for _ in 0..4 {
            children.push(
                Command::new(std::env::args_os().next().unwrap())
                    .args(["--exact", "pg::postgres_process_worker", "--nocapture"])
                    .env("ADK_DURABLE_PG_SCHEMA", &schema)
                    .env("ADK_DURABLE_PG_GATE", root.path())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
        fs::write(root.path().join("start"), b"go").unwrap();
        let mut winners = 0;
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            winners += usize::from(String::from_utf8_lossy(&output.stdout).contains("ACQUIRED"));
        }
        assert_eq!(winners, 1);
        let stale: Lease =
            serde_json::from_slice(&fs::read(root.path().join("lease.json")).unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(2100));
        let fresh = s
            .acquire_lease(
                &stale.tenant_id,
                &stale.run_id,
                "new-process",
                Duration::from_secs(60),
            )
            .unwrap();
        assert_ne!(fresh.token, stale.token);
        let mut next = snapshot("process_pg", "run");
        next.revision = 1;
        assert!(matches!(
            s.append(&stale, 0, vec![], next),
            Err(Error::LeaseLost)
        ));
        assert!(matches!(
            s.renew_lease(&stale, Duration::from_secs(1)),
            Err(Error::LeaseLost)
        ));
        assert!(matches!(s.release_lease(&stale), Err(Error::LeaseLost)));
        cleanup(&url, &schema);
    }
    #[test]
    fn postgres_live_go_schema_contract_privacy_and_faults() {
        let Some((url, schema)) = setup() else {
            return;
        };
        let mut c = client(&url, &schema);
        c.batch_execute("CREATE TABLE conversation_messages (session_id TEXT,metadata JSONB)")
            .unwrap();
        c.batch_execute(include_str!("../../../fixtures/durable/platform-042.sql"))
            .unwrap();
        let s = store(&url, &schema, Default::default());
        s.init().unwrap();
        store_contract(&s, "pg");
        let snap: RunSnapshot = serde_json::from_slice(&fixture("snapshot.json")).unwrap();
        c.execute("INSERT INTO durable_runs (tenant_id,run_id,revision,event_sequence,snapshot,created_at,updated_at) VALUES ($1,$2,1,1,$3,$4,$5)",&[&snap.tenant_id.as_str(),&snap.run_id.as_str(),&fixture("snapshot.json"),&snap.created_at,&snap.updated_at]).unwrap();
        c.execute(
            "INSERT INTO durable_events VALUES ($1,$2,1,$3)",
            &[
                &snap.tenant_id.as_str(),
                &snap.run_id.as_str(),
                &fixture("event.json"),
            ],
        )
        .unwrap();
        assert_eq!(s.load(&snap.tenant_id, &snap.run_id).unwrap().0, snap);
        let lease = s
            .acquire_lease(
                &snap.tenant_id,
                &snap.run_id,
                "rust",
                Duration::from_secs(60),
            )
            .unwrap();
        let mut next = snap.clone();
        next.revision = 2;
        s.append(&lease, 1, vec![Event::default()], next).unwrap();
        s.delete_tenant(&snap.tenant_id).unwrap();
        assert_eq!(
            c.query_one("SELECT count(*) FROM durable_events", &[])
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        let private = store(&url, &schema, private_options());
        privacy_contract(&private, "private_pg");
        let body: Vec<u8> = c
            .query_one(
                "SELECT snapshot FROM durable_runs WHERE tenant_id='private_pg'",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["encrypted"],
            true
        );
        assert!(s.load(&"private_pg".into(), &"private".into()).is_err());
        let fail = Arc::new(AtomicBool::new(false));
        let faulty = store(
            &url,
            &schema,
            StoreOptions {
                encryptor: Some(Arc::new(Failable { fail: fail.clone() })),
                ..Default::default()
            },
        );
        fault_contract(&faulty, &fail, "fault_pg");
        let mut future = snapshot("future", "run");
        s.create(future.clone()).unwrap();
        future.schema_version = 99;
        c.execute(
            "UPDATE durable_runs SET snapshot=$1 WHERE tenant_id='future'",
            &[&serde_json::to_vec(&future).unwrap()],
        )
        .unwrap();
        assert!(matches!(
            s.load(&future.tenant_id, &future.run_id),
            Err(Error::UnsupportedSchema(99))
        ));
        assert!(
            s.acquire_lease(
                &future.tenant_id,
                &future.run_id,
                "worker",
                Duration::from_secs(1)
            )
            .is_err()
        );
        cleanup(&url, &schema);
    }
    #[test]
    fn postgres_snapshot_failure_rolls_back_already_inserted_events() {
        let Some((url, schema)) = setup() else {
            return;
        };
        let s = store(&url, &schema, Default::default());
        s.init().unwrap();
        let mut next = snapshot("rollback", "run");
        s.create(next.clone()).unwrap();
        let lease = s
            .acquire_lease(
                &next.tenant_id,
                &next.run_id,
                "worker",
                Duration::from_secs(60),
            )
            .unwrap();
        let mut c = client(&url, &schema);
        c.batch_execute("CREATE FUNCTION reject_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.revision=1 THEN RAISE EXCEPTION 'injected snapshot failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_snapshot BEFORE UPDATE ON durable_runs FOR EACH ROW EXECUTE FUNCTION reject_snapshot()").unwrap();
        next.revision = 1;
        assert!(
            s.append(&lease, 0, vec![Event::default()], next.clone())
                .is_err()
        );
        let (loaded, events) = s.load(&next.tenant_id, &next.run_id).unwrap();
        assert_eq!(loaded.revision, 0);
        assert!(events.is_empty());
        assert_eq!(
            c.query_one("SELECT count(*) FROM durable_events", &[])
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        c.batch_execute("DROP TRIGGER reject_snapshot ON durable_runs")
            .unwrap();
        s.append(&lease, 0, vec![Event::default()], next).unwrap();
        let mut overflow = snapshot("overflow", "run");
        overflow.revision = u64::MAX;
        assert!(s.create(overflow).is_err());
        cleanup(&url, &schema);
    }

    #[test]
    fn postgres_independent_connections_compete_on_leases_and_cas() {
        let Some((url, schema)) = setup() else {
            return;
        };
        let s = store(&url, &schema, Default::default());
        s.init().unwrap();
        s.create(snapshot("concurrent", "run")).unwrap();
        let gate = Arc::new(std::sync::Barrier::new(5));
        let mut handles = vec![];
        for _ in 0..4 {
            let s = store(&url, &schema, Default::default());
            let gate = gate.clone();
            handles.push(std::thread::spawn(move || {
                gate.wait();
                s.acquire_lease(
                    &"concurrent".into(),
                    &"run".into(),
                    "worker",
                    Duration::from_secs(60),
                )
            }));
        }
        gate.wait();
        let mut leases = vec![];
        for h in handles {
            match h.join().unwrap() {
                Ok(l) => leases.push(l),
                Err(Error::LeaseHeld) => (),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(leases.len(), 1);
        let lease = leases.pop().unwrap();
        let gate = Arc::new(std::sync::Barrier::new(5));
        let mut handles = vec![];
        for _ in 0..4 {
            let s = store(&url, &schema, Default::default());
            let gate = gate.clone();
            let lease = lease.clone();
            let mut next = snapshot("concurrent", "run");
            next.revision = 1;
            handles.push(std::thread::spawn(move || {
                gate.wait();
                s.append(&lease, 0, vec![Event::default()], next)
            }));
        }
        gate.wait();
        let mut winners = 0;
        for h in handles {
            match h.join().unwrap() {
                Ok(_) => winners += 1,
                Err(Error::Conflict) => (),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(winners, 1);
        assert_eq!(s.load(&lease.tenant_id, &lease.run_id).unwrap().1.len(), 1);
        cleanup(&url, &schema);
    }
}
