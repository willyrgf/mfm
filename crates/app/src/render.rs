use serde_json::{json, Value};

use crate::{
    AccessAuditPage, AdmitRunResponse, DriveResponse, EntryPointContract, PublicError,
    PublicRunView, ReplayResponse, TransitionTracePage,
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
            "at_journal_head": self.at_journal_head(),
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
            .map(|entry| canonical_json(entry.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "run_id": self.run_id(),
            "complete_as_of_journal_head": self.complete_as_of_journal_head(),
            "entries": entries,
            "next_cursor": self.next_cursor(),
        }))
    }
}
