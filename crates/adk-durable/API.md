# Runner integration API

Synchronous, object-safe `RunStore: Send + Sync`. All methods return `adk_durable::Result<T>`; use a blocking adapter in async execution.

```rust
fn create(&self, snapshot: RunSnapshot) -> Result<()>;
fn load(&self, tenant: &TenantId, run: &RunId) -> Result<(RunSnapshot, Vec<Event>)>;
fn append(&self, lease: &Lease, expected_revision: u64, events: Vec<Event>, snapshot: RunSnapshot) -> Result<RunSnapshot>;
fn acquire_lease(&self, tenant: &TenantId, run: &RunId, owner: &str, ttl: std::time::Duration) -> Result<Lease>;
fn renew_lease(&self, lease: &Lease, ttl: std::time::Duration) -> Result<Lease>;
fn release_lease(&self, lease: &Lease) -> Result<()>;
fn delete_run(&self, tenant: &TenantId, run: &RunId) -> Result<()>;
fn delete_tenant(&self, tenant: &TenantId) -> Result<()>;
fn apply_retention(&self, policy: RetentionPolicy) -> Result<u64>;
```

- `FilesystemStore::new(path: impl AsRef<Path>, options: StoreOptions) -> Result<Self>`
- `PostgresStore::connect(url: &str, options: StoreOptions) -> Result<Self>`; `init(&self) -> Result<()>` creates the existing Go tables. PostgreSQL feature enabled by default.
- `StoreOptions { redactor: Option<Arc<dyn Redactor>>, encryptor: Option<Arc<dyn Encryptor>> }`.
- `Redactor::redact(&self, DataClassification, Value) -> Result<Value>`; closures with that signature implement it. `Encryptor::{encrypt,decrypt}(&self, &[u8]) -> Result<Vec<u8>>`. Configuring an encryptor rejects plaintext records (including lease mutations), preventing envelope downgrade.
- `PostgresStore::new(postgres::Client, StoreOptions) -> Self` supports caller-configured TLS connections; the store owns the client.
- `Error::{NotFound, AlreadyExists, Conflict, LeaseHeld, LeaseLost, UnsupportedSchema(i32), Invalid(String), ...}`.
- `TenantId`, `RunId`, `AttemptId`, `StepId`, `ToolCallId`, `ApprovalId`, `ChildRunId`, `EffectId`, `EventId`, `LeaseToken`: public tuple string newtypes, `.new()`, `From<String>`, `From<&str>`, `Display`, transparent serde.
- `RunSnapshot::new(tenant: TenantId, run: RunId, now: DateTime<Utc>)`; all fields public with Go snake_case names. `status: RunStatus`, `state: Option<Value>`, `revision/event_sequence: u64`, `effects: Vec<Effect>`, `cumulative_budget: BudgetCounters`. `Default` supports struct-update construction.
- `Event { event_type: String, payload: Option<Value>, ..Default::default() }` (`event_type` serializes as `type`). IDs, tenant/run keys, sequence and absent timestamp assigned at append.
- Time fields: `chrono::DateTime<chrono::Utc>`; absent Go times default to year-one zero time. Optional retention deadline is `Option<DateTime<Utc>>`.
- Classifications: `DataClassification::{Unspecified, Public, Internal, Sensitive, Secret}`. Effects: `EffectClassification::{Idempotent, Deduplicated, NonReplayable}` and `EffectState::{Prepared, Dispatched, Succeeded, Failed, OutcomeUnknown}`.
- `Effect::new(&RunId, EffectClassification, DateTime<Utc>)`; `transition_effect(&mut Effect, EffectState, DateTime<Utc>) -> Result<()>`; `mark_interrupted_effect(&mut Effect, DateTime<Utc>) -> Result<()>`; `recover_effect(&Effect) -> RecoveryDecision`; `idempotency_key(&RunId, &EffectId) -> String`.
- `RecoveryDecision { action: RecoveryAction, automatic: bool, idempotency_key: String }`; `RecoveryAction::{None, Retry, Reconcile, OperatorResolution}`. Non-replayable ambiguous effects never automatically retry.
- `encode_document(&Document) -> Result<Vec<u8>>`, `decode_document(&[u8]) -> Result<Document>`; `SCHEMA_VERSION = 2`; v1 migration and future-version rejection.
- `RetentionPolicy { now: Option<DateTime<Utc>> }` defaults to current time.

An encrypted filesystem record includes both document and lease. All mutations (including lease-only writes) retain encryption. The caller owns key management. Metadata columns in PostgreSQL remain cleartext, matching Go schema; snapshot/event bodies are encrypted. Leases are fencing tokens, not guarantees of exactly-once external effects.
