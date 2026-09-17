use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::*;

pub fn encode_document(document: &Document) -> Result<Vec<u8>> {
    check_version(document.schema_version, true)?;
    check_version(document.snapshot.schema_version, true)?;
    let mut document = document.clone();
    document.schema_version = SCHEMA_VERSION;
    document.snapshot.schema_version = SCHEMA_VERSION;
    Ok(serde_json::to_vec(&document)?)
}

pub fn decode_document(data: &[u8]) -> Result<Document> {
    #[derive(Deserialize)]
    struct Header {
        schema_version: i32,
    }
    let header: Header = serde_json::from_slice(data)?;
    match header.schema_version {
        SCHEMA_VERSION => {
            let mut document: Document = serde_json::from_slice(data)?;
            check_version(document.snapshot.schema_version, true)?;
            document.snapshot.schema_version = SCHEMA_VERSION;
            Ok(document)
        }
        1 => migrate_v1(data),
        other => Err(Error::UnsupportedSchema(other)),
    }
}

fn migrate_v1(data: &[u8]) -> Result<Document> {
    #[derive(Deserialize)]
    struct V1 {
        tenant_id: TenantId,
        run_id: RunId,
        #[serde(default)]
        revision: u64,
        #[serde(default)]
        event_sequence: u64,
        #[serde(default)]
        status: RunStatus,
        #[serde(default)]
        cancelled: bool,
        #[serde(default)]
        budget_tokens: i64,
        #[serde(default = "zero_time")]
        created_at: chrono::DateTime<chrono::Utc>,
        #[serde(default = "zero_time")]
        updated_at: chrono::DateTime<chrono::Utc>,
        #[serde(default, deserialize_with = "crate::types::null_vec")]
        events: Vec<Event>,
    }
    let mut old: V1 = serde_json::from_slice(data)?;
    let mut snapshot = RunSnapshot::new(old.tenant_id, old.run_id, old.created_at);
    snapshot.updated_at = old.updated_at;
    snapshot.revision = old.revision;
    snapshot.event_sequence = if old.event_sequence == 0 {
        old.events.len() as u64
    } else {
        old.event_sequence
    };
    if old.status != RunStatus::Unspecified {
        snapshot.status = old.status;
    }
    snapshot.cumulative_budget.input_tokens = old.budget_tokens;
    if old.cancelled {
        snapshot.cancellation = Some(Cancellation {
            requested_at: old.updated_at,
            ..Cancellation::default()
        });
    }
    for (i, event) in old.events.iter_mut().enumerate() {
        if event.tenant_id.is_empty() {
            event.tenant_id = snapshot.tenant_id.clone();
        }
        if event.run_id.is_empty() {
            event.run_id = snapshot.run_id.clone();
        }
        if event.sequence == 0 {
            event.sequence = i as u64 + 1;
        }
    }
    Ok(Document {
        schema_version: SCHEMA_VERSION,
        snapshot,
        events: old.events,
    })
}

pub(crate) fn check_version(version: i32, allow_zero: bool) -> Result<()> {
    if version != SCHEMA_VERSION && !(allow_zero && version == 0) {
        return Err(Error::UnsupportedSchema(version));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    encrypted: bool,
    data: String,
}

pub(crate) fn protect<T: Serialize>(value: &T, options: &StoreOptions) -> Result<Vec<u8>> {
    let plain = serde_json::to_vec(value)?;
    match &options.encryptor {
        None => Ok(plain),
        Some(encryptor) => Ok(serde_json::to_vec(&Envelope {
            encrypted: true,
            data: STANDARD.encode(encryptor.encrypt(&plain)?),
        })?),
    }
}

pub(crate) fn unprotect<T: DeserializeOwned>(data: &[u8], options: &StoreOptions) -> Result<T> {
    let value: Value = serde_json::from_slice(data)?;
    if value.get("encrypted").and_then(Value::as_bool) == Some(true) {
        let envelope: Envelope = serde_json::from_value(value)?;
        let encryptor = options
            .encryptor
            .as_ref()
            .ok_or_else(|| Error::Protection("encrypted record requires an encryptor".into()))?;
        let ciphertext = STANDARD
            .decode(envelope.data)
            .map_err(|e| Error::Protection(e.to_string()))?;
        return Ok(serde_json::from_slice(&encryptor.decrypt(&ciphertext)?)?);
    }
    if options.encryptor.is_some() {
        return Err(Error::Protection(
            "plaintext record rejected by encrypted store".into(),
        ));
    }
    Ok(serde_json::from_value(value)?)
}
