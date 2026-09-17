use crate::*;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

pub trait Redactor: Send + Sync {
    fn redact(&self, classification: DataClassification, value: Value) -> Result<Value>;
}
impl<F> Redactor for F
where
    F: Fn(DataClassification, Value) -> Result<Value> + Send + Sync,
{
    fn redact(&self, classification: DataClassification, value: Value) -> Result<Value> {
        self(classification, value)
    }
}
pub trait Encryptor: Send + Sync {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;
}
#[derive(Clone, Default)]
pub struct StoreOptions {
    pub redactor: Option<Arc<dyn Redactor>>,
    pub encryptor: Option<Arc<dyn Encryptor>>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RetentionPolicy {
    pub now: Option<DateTime<Utc>>,
}

pub trait RunStore: Send + Sync {
    fn create(&self, snapshot: RunSnapshot) -> Result<()>;
    fn load(&self, tenant: &TenantId, run: &RunId) -> Result<(RunSnapshot, Vec<Event>)>;
    fn append(
        &self,
        lease: &Lease,
        expected_revision: u64,
        events: Vec<Event>,
        snapshot: RunSnapshot,
    ) -> Result<RunSnapshot>;
    fn acquire_lease(
        &self,
        tenant: &TenantId,
        run: &RunId,
        owner: &str,
        ttl: Duration,
    ) -> Result<Lease>;
    fn renew_lease(&self, lease: &Lease, ttl: Duration) -> Result<Lease>;
    fn release_lease(&self, lease: &Lease) -> Result<()>;
    fn delete_run(&self, tenant: &TenantId, run: &RunId) -> Result<()>;
    fn delete_tenant(&self, tenant: &TenantId) -> Result<()>;
    fn apply_retention(&self, policy: RetentionPolicy) -> Result<u64>;
}

pub(crate) fn validate_snapshot(snapshot: &RunSnapshot) -> Result<()> {
    if snapshot.tenant_id.is_empty() || snapshot.run_id.is_empty() {
        return Err(Error::Invalid(
            "snapshot requires tenant and run IDs".into(),
        ));
    }
    crate::codec::check_version(snapshot.schema_version, true)
}
pub(crate) fn validate_key(snapshot: &RunSnapshot, tenant: &TenantId, run: &RunId) -> Result<()> {
    if &snapshot.tenant_id != tenant || &snapshot.run_id != run {
        return Err(Error::Invalid(
            "snapshot key does not match store key".into(),
        ));
    }
    Ok(())
}
pub(crate) fn prepare_create(
    mut snapshot: RunSnapshot,
    options: &StoreOptions,
) -> Result<RunSnapshot> {
    validate_snapshot(&snapshot)?;
    snapshot.schema_version = SCHEMA_VERSION;
    if snapshot.created_at == zero_time() {
        snapshot.created_at = Utc::now();
    }
    if snapshot.updated_at == zero_time() {
        snapshot.updated_at = snapshot.created_at;
    }
    redact_snapshot(&mut snapshot, options)?;
    Ok(snapshot)
}
pub(crate) fn prepare_append(
    lease: &Lease,
    revision: u64,
    sequence: u64,
    mut events: Vec<Event>,
    mut snapshot: RunSnapshot,
    options: &StoreOptions,
) -> Result<(RunSnapshot, Vec<Event>)> {
    validate_snapshot(&snapshot)?;
    validate_key(&snapshot, &lease.tenant_id, &lease.run_id)?;
    let next_revision = revision
        .checked_add(1)
        .ok_or_else(|| Error::Invalid("revision overflow".into()))?;
    if snapshot.revision != next_revision {
        return Err(Error::Invalid(format!(
            "snapshot revision must be {next_revision}"
        )));
    }
    let end_sequence = sequence
        .checked_add(events.len() as u64)
        .ok_or_else(|| Error::Invalid("event sequence overflow".into()))?;
    for (i, event) in events.iter_mut().enumerate() {
        if (!event.tenant_id.is_empty() && event.tenant_id != lease.tenant_id)
            || (!event.run_id.is_empty() && event.run_id != lease.run_id)
        {
            return Err(Error::Invalid("event key does not match append key".into()));
        }
        if event.id.is_empty() {
            event.id = EventId::new();
        }
        if event.at == zero_time() {
            event.at = snapshot.updated_at;
        }
        event.tenant_id = lease.tenant_id.clone();
        event.run_id = lease.run_id.clone();
        event.sequence = sequence + i as u64 + 1;
        redact_value(&mut event.payload, event.classification, options)?;
    }
    snapshot.schema_version = SCHEMA_VERSION;
    snapshot.event_sequence = end_sequence;
    if snapshot.updated_at == zero_time() {
        snapshot.updated_at = Utc::now();
    }
    redact_snapshot(&mut snapshot, options)?;
    Ok((snapshot, events))
}
fn redact_value(
    value: &mut Option<Value>,
    classification: DataClassification,
    options: &StoreOptions,
) -> Result<()> {
    if let Some(redactor) = &options.redactor {
        if let Some(current) = value.take() {
            *value = Some(redactor.redact(classification, current)?);
        }
    }
    Ok(())
}
fn redact_snapshot(snapshot: &mut RunSnapshot, options: &StoreOptions) -> Result<()> {
    if options.redactor.is_none() {
        return Ok(());
    }
    redact_value(&mut snapshot.state, snapshot.classification, options)?;
    for step in &mut snapshot.steps {
        redact_value(&mut step.data, snapshot.classification, options)?;
    }
    for tool in &mut snapshot.tool_calls {
        let classification = if tool.classification.is_unspecified() {
            snapshot.classification
        } else {
            tool.classification
        };
        redact_value(&mut tool.input, classification, options)?;
        redact_value(&mut tool.output, classification, options)?;
    }
    for effect in &mut snapshot.effects {
        let classification = if effect.data_classification.is_unspecified() {
            snapshot.classification
        } else {
            effect.data_classification
        };
        redact_value(&mut effect.outcome, classification, options)?;
    }
    Ok(())
}
pub(crate) fn expiry(now: DateTime<Utc>, ttl: Duration) -> Result<DateTime<Utc>> {
    if ttl.is_zero() {
        return Err(Error::Invalid("positive lease TTL is required".into()));
    }
    let delta =
        chrono::Duration::from_std(ttl).map_err(|_| Error::Invalid("lease TTL overflow".into()))?;
    now.checked_add_signed(delta)
        .ok_or_else(|| Error::Invalid("lease TTL overflow".into()))
}
