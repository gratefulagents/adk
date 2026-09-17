use crate::{
    codec::{check_version, protect, unprotect},
    store::*,
    *,
};
use chrono::{DateTime, Utc};
use postgres::{Client, NoTls, Row, Transaction};
use std::{
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub struct PostgresStore {
    client: Mutex<Client>,
    options: StoreOptions,
}
impl PostgresStore {
    pub fn connect(url: &str, options: StoreOptions) -> Result<Self> {
        Ok(Self::new(Client::connect(url, NoTls)?, options))
    }
    /// Supply a caller-configured connection here for TLS or custom connection settings.
    pub fn new(client: Client, options: StoreOptions) -> Self {
        Self {
            client: Mutex::new(client),
            options,
        }
    }
    fn client(&self) -> Result<MutexGuard<'_, Client>> {
        self.client
            .lock()
            .map_err(|_| Error::Invalid("PostgreSQL connection lock poisoned".into()))
    }
    pub fn init(&self) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.batch_execute(include_str!("schema.sql"))?;
        tx.commit()?;
        Ok(())
    }
    fn snapshot(&self, row: &Row, tenant: &TenantId, run: &RunId) -> Result<RunSnapshot> {
        let bytes: Vec<u8> = row.get("snapshot");
        let snapshot: RunSnapshot = unprotect(&bytes, &self.options)?;
        check_version(snapshot.schema_version, false)?;
        validate_key(&snapshot, tenant, run)?;
        if integer(snapshot.revision)? != row.get::<_, i64>("revision")
            || integer(snapshot.event_sequence)? != row.get::<_, i64>("event_sequence")
        {
            return Err(Error::Invalid(
                "snapshot metadata does not match row".into(),
            ));
        }
        Ok(snapshot)
    }
}
fn integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Invalid("value exceeds PostgreSQL BIGINT".into()))
}
fn lock_row(tx: &mut Transaction<'_>, tenant: &TenantId, run: &RunId) -> Result<Row> {
    tx.query_opt(
        "SELECT * FROM durable_runs WHERE tenant_id=$1 AND run_id=$2 FOR UPDATE",
        &[&tenant.as_str(), &run.as_str()],
    )?
    .ok_or(Error::NotFound)
}
fn now(tx: &mut Transaction<'_>) -> Result<DateTime<Utc>> {
    Ok(tx.query_one("SELECT clock_timestamp()", &[])?.get(0))
}
fn check_lease(row: &Row, lease: &Lease, now: Option<DateTime<Utc>>) -> Result<()> {
    let token: Option<String> = row.get("lease_token");
    let until: Option<DateTime<Utc>> = row.get("lease_until");
    if token.as_deref() != Some(lease.token.as_str())
        || now.is_some_and(|n| until.is_none_or(|u| u <= n))
    {
        return Err(Error::LeaseLost);
    }
    Ok(())
}
impl RunStore for PostgresStore {
    fn create(&self, snapshot: RunSnapshot) -> Result<()> {
        let snapshot = prepare_create(snapshot, &self.options)?;
        let body = protect(&snapshot, &self.options)?;
        let affected = self.client()?.execute("INSERT INTO durable_runs (tenant_id,run_id,revision,event_sequence,snapshot,retain_until,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (tenant_id,run_id) DO NOTHING", &[&snapshot.tenant_id.as_str(), &snapshot.run_id.as_str(), &integer(snapshot.revision)?, &integer(snapshot.event_sequence)?, &body, &snapshot.retain_until, &snapshot.created_at, &snapshot.updated_at])?;
        if affected == 0 {
            return Err(Error::AlreadyExists);
        }
        Ok(())
    }
    fn load(&self, tenant: &TenantId, run: &RunId) -> Result<(RunSnapshot, Vec<Event>)> {
        let mut client = self.client()?;
        let mut tx = client
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()?;
        let row = tx.query_opt("SELECT snapshot, revision, event_sequence FROM durable_runs WHERE tenant_id=$1 AND run_id=$2", &[&tenant.as_str(), &run.as_str()])?.ok_or(Error::NotFound)?;
        let snapshot = self.snapshot(&row, tenant, run)?;
        let mut events = vec![];
        for row in tx.query("SELECT sequence,body FROM durable_events WHERE tenant_id=$1 AND run_id=$2 AND sequence <= $3 ORDER BY sequence", &[&tenant.as_str(), &run.as_str(), &integer(snapshot.event_sequence)?])? {
            let bytes: Vec<u8> = row.get("body");
            let event: Event = unprotect(&bytes, &self.options)?;
            if &event.tenant_id != tenant || &event.run_id != run || integer(event.sequence)? != row.get::<_, i64>("sequence") { return Err(Error::Invalid("event key does not match row".into())); }
            events.push(event);
        }
        tx.commit()?;
        Ok((snapshot, events))
    }
    fn append(
        &self,
        lease: &Lease,
        expected_revision: u64,
        events: Vec<Event>,
        snapshot: RunSnapshot,
    ) -> Result<RunSnapshot> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let row = lock_row(&mut tx, &lease.tenant_id, &lease.run_id)?;
        let old = self.snapshot(&row, &lease.tenant_id, &lease.run_id)?;
        check_lease(&row, lease, Some(now(&mut tx)?))?;
        if old.revision != expected_revision {
            return Err(Error::Conflict);
        }
        let (updated, events) = prepare_append(
            lease,
            expected_revision,
            old.event_sequence,
            events,
            snapshot,
            &self.options,
        )?;
        for event in events {
            let body = protect(&event, &self.options)?;
            tx.execute(
                "INSERT INTO durable_events (tenant_id,run_id,sequence,body) VALUES ($1,$2,$3,$4)",
                &[
                    &lease.tenant_id.as_str(),
                    &lease.run_id.as_str(),
                    &integer(event.sequence)?,
                    &body,
                ],
            )?;
        }
        let body = protect(&updated, &self.options)?;
        tx.execute("UPDATE durable_runs SET revision=$1,event_sequence=$2,snapshot=$3,retain_until=$4,updated_at=$5 WHERE tenant_id=$6 AND run_id=$7", &[&integer(updated.revision)?, &integer(updated.event_sequence)?, &body, &updated.retain_until, &updated.updated_at, &lease.tenant_id.as_str(), &lease.run_id.as_str()])?;
        tx.commit()?;
        Ok(updated)
    }
    fn acquire_lease(
        &self,
        tenant: &TenantId,
        run: &RunId,
        owner: &str,
        ttl: Duration,
    ) -> Result<Lease> {
        if owner.is_empty() {
            return Err(Error::Invalid("lease owner is required".into()));
        }
        expiry(Utc::now(), ttl)?;
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let row = lock_row(&mut tx, tenant, run)?;
        self.snapshot(&row, tenant, run)?;
        let now = now(&mut tx)?;
        if row
            .get::<_, Option<DateTime<Utc>>>("lease_until")
            .is_some_and(|until| until > now)
        {
            return Err(Error::LeaseHeld);
        }
        let lease = Lease {
            tenant_id: tenant.clone(),
            run_id: run.clone(),
            owner: owner.into(),
            token: LeaseToken::new(),
            expires_at: expiry(now, ttl)?,
        };
        tx.execute("UPDATE durable_runs SET lease_owner=$1,lease_token=$2,lease_until=$3 WHERE tenant_id=$4 AND run_id=$5", &[&lease.owner, &lease.token.as_str(), &lease.expires_at, &tenant.as_str(), &run.as_str()])?;
        tx.commit()?;
        Ok(lease)
    }
    fn renew_lease(&self, lease: &Lease, ttl: Duration) -> Result<Lease> {
        expiry(Utc::now(), ttl)?;
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let row = lock_row(&mut tx, &lease.tenant_id, &lease.run_id).map_err(|e| {
            if matches!(e, Error::NotFound) {
                Error::LeaseLost
            } else {
                e
            }
        })?;
        self.snapshot(&row, &lease.tenant_id, &lease.run_id)?;
        let now = now(&mut tx)?;
        check_lease(&row, lease, Some(now))?;
        let mut renewed = lease.clone();
        renewed.owner = row
            .get::<_, Option<String>>("lease_owner")
            .ok_or_else(|| Error::Invalid("lease owner is absent".into()))?;
        renewed.expires_at = expiry(now, ttl)?;
        tx.execute(
            "UPDATE durable_runs SET lease_until=$1 WHERE tenant_id=$2 AND run_id=$3",
            &[
                &renewed.expires_at,
                &lease.tenant_id.as_str(),
                &lease.run_id.as_str(),
            ],
        )?;
        tx.commit()?;
        Ok(renewed)
    }
    fn release_lease(&self, lease: &Lease) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let row = lock_row(&mut tx, &lease.tenant_id, &lease.run_id).map_err(|e| {
            if matches!(e, Error::NotFound) {
                Error::LeaseLost
            } else {
                e
            }
        })?;
        self.snapshot(&row, &lease.tenant_id, &lease.run_id)?;
        check_lease(&row, lease, None)?;
        tx.execute("UPDATE durable_runs SET lease_owner=NULL,lease_token=NULL,lease_until=NULL WHERE tenant_id=$1 AND run_id=$2", &[&lease.tenant_id.as_str(), &lease.run_id.as_str()])?;
        tx.commit()?;
        Ok(())
    }
    fn delete_run(&self, tenant: &TenantId, run: &RunId) -> Result<()> {
        if self.client()?.execute(
            "DELETE FROM durable_runs WHERE tenant_id=$1 AND run_id=$2",
            &[&tenant.as_str(), &run.as_str()],
        )? == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
    fn delete_tenant(&self, tenant: &TenantId) -> Result<()> {
        self.client()?.execute(
            "DELETE FROM durable_runs WHERE tenant_id=$1",
            &[&tenant.as_str()],
        )?;
        Ok(())
    }
    fn apply_retention(&self, policy: RetentionPolicy) -> Result<u64> {
        Ok(self.client()?.execute(
            "DELETE FROM durable_runs WHERE retain_until IS NOT NULL AND retain_until <= $1",
            &[&policy.now.unwrap_or_else(Utc::now)],
        )?)
    }
}
