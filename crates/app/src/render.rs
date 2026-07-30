use serde_json::{json, Value};

use crate::{
    AccessAuditEntry, AccessAuditPage, AdmitRunResponse, DriveResponse, EntryPointContract,
    PublicError, PublicRunView, ReplayResponse, TransitionTracePage,
};

/// Reviewed JSON rendering implemented only for public application responses.
///
/// Annex-owned values are rendered from their already validated canonical bytes. This trait does
/// not create semantic, store, object, or access authority.
pub trait PublicJsonResponse {
    /// Returns the reviewed JSON value for a transport success payload.
    fn public_json(&self) -> Result<Value, PublicError>;
}

fn canonical_json(bytes: &[u8]) -> Result<Value, PublicError> {
    serde_json::from_slice(bytes).map_err(|_| {
        PublicError::internal(
            "PublicResponseRenderingFailed",
            "A public response could not be rendered",
        )
    })
}

macro_rules! canonical_response_json {
    ($($type:ty),+ $(,)?) => {
        $(
            impl PublicJsonResponse for $type {
                fn public_json(&self) -> Result<Value, PublicError> {
                    canonical_json(self.as_bytes())
                }
            }
        )+
    };
}

canonical_response_json!(
    AdmitRunResponse,
    DriveResponse,
    PublicRunView,
    ReplayResponse,
);

impl PublicJsonResponse for [EntryPointContract] {
    fn public_json(&self) -> Result<Value, PublicError> {
        self.iter()
            .map(|entry| {
                entry
                    .canonical_json()
                    .map_err(|_| {
                        PublicError::internal(
                            "EntryPointContractInvalid",
                            "The compiled entry-point contract is invalid",
                        )
                    })
                    .and_then(|bytes| canonical_json(bytes.as_bytes()))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    }
}

impl PublicJsonResponse for Vec<EntryPointContract> {
    fn public_json(&self) -> Result<Value, PublicError> {
        self.as_slice().public_json()
    }
}

impl PublicJsonResponse for TransitionTracePage {
    fn public_json(&self) -> Result<Value, PublicError> {
        let transitions = self
            .transitions()
            .iter()
            .map(|transition| canonical_json(transition.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "run_id": self.run_id(),
            "at_journal_head": canonical_json(self.at_journal_head().as_bytes())?,
            "transitions": transitions,
            "next_cursor": self.next_cursor(),
        }))
    }
}

impl PublicJsonResponse for AccessAuditPage {
    fn public_json(&self) -> Result<Value, PublicError> {
        let entries = self
            .entries()
            .iter()
            .map(access_audit_entry_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "run_id": self.run_id(),
            "complete_as_of_journal_head":
                canonical_json(self.complete_as_of_journal_head().as_bytes())?,
            "entries": entries,
            "next_cursor": self.next_cursor(),
        }))
    }
}

fn access_audit_entry_json(entry: &AccessAuditEntry) -> Result<Value, PublicError> {
    let observation_ref = entry
        .observation_ref()
        .map(|value| canonical_json(value.as_bytes()))
        .transpose()?;
    let observation_journal_head = entry
        .observation_journal_head()
        .map(|value| canonical_json(value.as_bytes()))
        .transpose()?;
    let result_ref = entry
        .result_ref()
        .map(|value| canonical_json(value.as_bytes()))
        .transpose()?;
    let failure = entry
        .failure()
        .map(|value| canonical_json(value.as_bytes()))
        .transpose()?;
    let non_domain_failure = entry
        .non_domain_failure()
        .map(|value| {
            let persisted = mfm_journal::PersistedNonDomainFailure::new(value).map_err(|_| {
                PublicError::internal(
                    "PublicResponseRenderingFailed",
                    "A public response could not be rendered",
                )
            })?;
            canonical_json(persisted.as_bytes())
        })
        .transpose()?;
    let delivery_audit_ref = entry
        .delivery_audit_ref()
        .map(|value| canonical_json(value.as_bytes()))
        .transpose()?;

    Ok(json!({
        "authorization_ref": canonical_json(entry.authorization_ref().as_bytes())?,
        "observation_ref": observation_ref,
        "authorization_journal_head":
            canonical_json(entry.authorization_journal_head().as_bytes())?,
        "observation_journal_head": observation_journal_head,
        "capability_binding_ref":
            canonical_json(entry.capability_binding_ref().as_bytes())?,
        "capability_operation_id": entry.capability_operation_id().as_str(),
        "request_ref": canonical_json(entry.request_ref().as_bytes())?,
        "status": entry.status().as_str(),
        "result_ref": result_ref,
        "failure": failure,
        "non_domain_failure": non_domain_failure,
        "effect_key": entry.effect_key().map(|value| value.as_str()),
        "delivery_audit_ref": delivery_audit_ref,
        "delivery_audit_terminal": entry.delivery_audit_terminal(),
    }))
}
