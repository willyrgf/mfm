use std::marker::PhantomData;

use mfm_capabilities::{CapabilityError, EffectCapabilityContract};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, MfmValue as Value, Object};
use serde::{Deserialize, Serialize};

use crate::{ObservationPoint, TransactionIdentity};

/// Associates a semantic transaction request with its successful domain result.
pub trait TransactionRequest: Value {
    /// Facts established by authenticated successful settlement.
    type Applied: Value;
}

/// Complete semantic request and public native preparation retained before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields, bound(deserialize = "R: TransactionRequest"))]
#[mfm(
    namespace = "mfm.chain",
    name = "prepared-transaction",
    version = "1",
    schema = "mfm.chain-prepared-transaction"
)]
pub struct PreparedTransaction<R: TransactionRequest> {
    request: R,
    implementation_ref: ContentRef,
    binding_ref: ContentRef,
    native: Object,
}

impl<R: TransactionRequest> PreparedTransaction<R> {
    /// Retains checked identities and native bytes; native validation belongs to the implementation.
    pub fn new(
        request: R,
        implementation_ref: ContentRef,
        binding_ref: ContentRef,
        native: Object,
    ) -> Self {
        Self {
            request,
            implementation_ref,
            binding_ref,
            native,
        }
    }
    /// Complete semantic request.
    pub fn request(&self) -> &R {
        &self.request
    }
    /// Consumes preparation and returns its authoritative request.
    pub fn into_request(self) -> R {
        self.request
    }
    /// Exact selected native implementation ABI.
    pub fn implementation_ref(&self) -> &ContentRef {
        &self.implementation_ref
    }
    /// Exact public native binding.
    pub fn binding_ref(&self) -> &ContentRef {
        &self.binding_ref
    }
    /// Public native command, with its original canonical representation.
    pub fn native(&self) -> &Object {
        &self.native
    }
}

/// Authenticated settlement result, excluding transport uncertainty and pending execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
// External tagging avoids Serde's content buffer, which cannot retain checked RawValue payloads.
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "transaction-result",
    version = "1",
    schema = "mfm.chain-transaction-result"
)]
pub enum TransactionResult<T> {
    /// Successful settlement with its domain facts.
    Applied {
        /// Result established by the selected native implementation.
        output: T,
    },
    /// Authenticated unsuccessful settlement.
    Rejected {
        /// Reviewed explanation, absent when the protocol supplies none.
        reason: Option<String>,
    },
}

/// A semantic settlement whose transaction and observation refer to different ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("transaction evidence ledger mismatch")]
pub struct TransactionEvidenceError;

/// Semantic settlement retaining the exact native original and execution provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "transaction-evidence",
    version = "1",
    schema = "mfm.chain-transaction-evidence"
)]
pub struct TransactionEvidence<T: Value> {
    effect_id: EffectId,
    command_ref: ContentRef,
    implementation_ref: ContentRef,
    transaction: TransactionIdentity,
    observed_at: ObservationPoint,
    original: Object,
    result: TransactionResult<T>,
}

impl<T: Value> TransactionEvidence<T> {
    /// Checks shared ledger agreement; the native owner must establish protocol-specific facts.
    pub fn new(
        effect_id: EffectId,
        command_ref: ContentRef,
        implementation_ref: ContentRef,
        transaction: TransactionIdentity,
        observed_at: ObservationPoint,
        original: Object,
        result: TransactionResult<T>,
    ) -> Result<Self, TransactionEvidenceError> {
        if transaction.ledger() != observed_at.ledger() {
            return Err(TransactionEvidenceError);
        }
        Ok(Self {
            effect_id,
            command_ref,
            implementation_ref,
            transaction,
            observed_at,
            original,
            result,
        })
    }
    /// Runtime's exact Effect identity.
    pub fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }
    /// Exact semantic command identity.
    pub fn command_ref(&self) -> &ContentRef {
        &self.command_ref
    }
    /// Selected native implementation ABI.
    pub fn implementation_ref(&self) -> &ContentRef {
        &self.implementation_ref
    }
    /// Native-qualified transaction identity.
    pub fn transaction(&self) -> &TransactionIdentity {
        &self.transaction
    }
    /// Exact settlement observation point.
    pub fn observed_at(&self) -> &ObservationPoint {
        &self.observed_at
    }
    /// Native evidence retained without re-encoding.
    pub fn original(&self) -> &Object {
        &self.original
    }
    /// Authenticated semantic result.
    pub fn result(&self) -> &TransactionResult<T> {
        &self.result
    }
}

impl<'de, T: Value> Deserialize<'de> for TransactionEvidence<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, bound(deserialize = "T: Value"))]
        struct Wire<T: Value> {
            effect_id: EffectId,
            command_ref: ContentRef,
            implementation_ref: ContentRef,
            transaction: TransactionIdentity,
            observed_at: ObservationPoint,
            original: Object,
            result: TransactionResult<T>,
        }
        let wire = Wire::<T>::deserialize(deserializer)?;
        Self::new(
            wire.effect_id,
            wire.command_ref,
            wire.implementation_ref,
            wire.transaction,
            wire.observed_at,
            wire.original,
            wire.result,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Request-specialized semantic transaction capability, independent of the network.
pub struct TransactionEffect<R: TransactionRequest>(PhantomData<R>);

impl<R: TransactionRequest> EffectCapabilityContract for TransactionEffect<R> {
    type Command = PreparedTransaction<R>;
    type Evidence = TransactionEvidence<R::Applied>;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.chain.transaction@1")?)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &Self::Command,
        native_evidence_ref: &ContentRef,
        evidence: &Self::Evidence,
    ) -> Result<(), InvocationDiagnostic> {
        if evidence.effect_id() != effect_id
            || evidence.command_ref() != command_ref
            || evidence.implementation_ref() != command.implementation_ref()
            || evidence.original().value_ref() != native_evidence_ref
        {
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "bind_transaction_evidence",
                &CapabilityError::EvidenceBinding,
                None,
            ));
        }
        Ok(())
    }
}
