//! Producer-free semantic commands authored only by Runtime.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::FactSet;
use mfm_ids::{AppendRequestId, ContentRef, InvocationIdentity, StableId, TenantScopeId};
use mfm_journal::structured::{
    HistoryObject, LexicalValueRef, PriorRunFactSourceManifest, RecordRef,
    ADMISSION_CONFIGURATION_OBJECT_TYPE, ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
    ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_spec::structured::CertifiedProgramDocument;
use serde::Serialize;

use super::HistoryError;

/// Producer-free canonical value proposed by a qualified callback or admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedCanonicalValue {
    canonical: PlainCanonicalJsonBytes,
}

impl ProposedCanonicalValue {
    /// Canonicalizes one float-free serializable value.
    pub fn from_value<T: Serialize>(value: &T) -> Result<Self, HistoryError> {
        let json = serde_json::to_string(value).map_err(|_| HistoryError::InvalidHistory)?;
        Self::from_json(&json)
    }

    /// Parses one exact float-free JSON value into canonical bytes.
    pub fn from_json(json: &str) -> Result<Self, HistoryError> {
        let canonical = PlainCanonicalJsonBytes::from_json_str(json)
            .map_err(|_| HistoryError::InvalidHistory)?;
        Ok(Self { canonical })
    }

    /// Returns the exact canonical proposal bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Store-only construction after fold-side validation of already-canonical bytes.
    #[doc(hidden)]
    pub fn from_canonical_bytes(canonical: PlainCanonicalJsonBytes) -> Self {
        Self { canonical }
    }
}

/// Exact producer-free public objects selected before run admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredAdmissionMaterial {
    /// Validated configuration object.
    #[doc(hidden)]
    pub configuration: HistoryObject,
    /// Validated context manifest object.
    #[doc(hidden)]
    pub context_manifest: HistoryObject,
    /// Validated prior-run source manifest object.
    #[doc(hidden)]
    pub prior_run_source_manifest: HistoryObject,
    /// Validated routing policy object.
    #[doc(hidden)]
    pub routing_policy: HistoryObject,
    /// Validated stable resource lineage contract references.
    #[doc(hidden)]
    pub stable_resource_lineage_contract_refs: Vec<ContentRef>,
}

impl StructuredAdmissionMaterial {
    /// Validates and canonically orders one complete non-secret admission bundle.
    pub fn new(
        configuration: HistoryObject,
        context_manifest: HistoryObject,
        prior_run_source_manifest: HistoryObject,
        routing_policy: HistoryObject,
        mut stable_resource_lineage_contract_refs: Vec<ContentRef>,
    ) -> Result<Self, HistoryError> {
        validate_admission_object(&configuration, ADMISSION_CONFIGURATION_OBJECT_TYPE)?;
        validate_admission_object(&context_manifest, ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE)?;
        validate_admission_object(
            &prior_run_source_manifest,
            ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE,
        )?;
        prior_run_source_manifest
            .decode_persisted::<PriorRunFactSourceManifest>()
            .map_err(|_| HistoryError::InvalidHistory)?;
        validate_admission_object(&routing_policy, ADMISSION_ROUTING_POLICY_OBJECT_TYPE)?;
        stable_resource_lineage_contract_refs.sort();
        if stable_resource_lineage_contract_refs
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(HistoryError::InvalidHistory);
        }
        let mut all_refs = vec![
            &configuration.content_ref,
            &context_manifest.content_ref,
            &prior_run_source_manifest.content_ref,
            &routing_policy.content_ref,
        ];
        all_refs.extend(stable_resource_lineage_contract_refs.iter());
        all_refs.sort();
        if all_refs.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(HistoryError::InvalidHistory);
        }
        Ok(Self {
            configuration,
            context_manifest,
            prior_run_source_manifest,
            routing_policy,
            stable_resource_lineage_contract_refs,
        })
    }

    /// Returns the configuration object.
    pub const fn configuration(&self) -> &HistoryObject {
        &self.configuration
    }

    /// Returns the context manifest object.
    pub const fn context_manifest(&self) -> &HistoryObject {
        &self.context_manifest
    }

    /// Returns the prior-run source manifest object.
    pub const fn prior_run_source_manifest(&self) -> &HistoryObject {
        &self.prior_run_source_manifest
    }

    /// Returns the routing policy object.
    pub const fn routing_policy(&self) -> &HistoryObject {
        &self.routing_policy
    }

    /// Returns stable resource lineage contract references.
    pub fn stable_resource_lineage_contract_refs(&self) -> &[ContentRef] {
        &self.stable_resource_lineage_contract_refs
    }
}

fn validate_admission_object(
    object: &HistoryObject,
    expected_type: &str,
) -> Result<(), HistoryError> {
    object
        .validate()
        .map_err(|_| HistoryError::InvalidHistory)?;
    if object.object_type.as_str() != expected_type {
        return Err(HistoryError::InvalidHistory);
    }
    Ok(())
}

/// Complete producer-free admission command. Run identity is derived by the store.
pub struct StructuredAdmissionCommand {
    tenant_scope_id: TenantScopeId,
    invocation_identity: InvocationIdentity,
    entry_point_operation_id: StableId,
    certified_program: CertifiedProgramDocument,
    material: StructuredAdmissionMaterial,
    initial_values: Vec<ProposedCanonicalValue>,
    append_request_id: AppendRequestId,
}

impl StructuredAdmissionCommand {
    /// Binds one exact qualified program document and declaration-ordered roots.
    ///
    /// Callers never supply a run digest; the store derives `RunId` from the
    /// owner-derived preimage.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_scope_id: TenantScopeId,
        invocation_identity: InvocationIdentity,
        entry_point_operation_id: StableId,
        certified_program: CertifiedProgramDocument,
        material: StructuredAdmissionMaterial,
        initial_values: Vec<ProposedCanonicalValue>,
        append_request_id: AppendRequestId,
    ) -> Self {
        Self {
            tenant_scope_id,
            invocation_identity,
            entry_point_operation_id,
            certified_program,
            material,
            initial_values,
            append_request_id,
        }
    }

    /// Returns the tenant scope.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the invocation identity.
    pub const fn invocation_identity(&self) -> &InvocationIdentity {
        &self.invocation_identity
    }

    /// Returns the entry-point operation identity.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the certified program document.
    pub const fn certified_program(&self) -> &CertifiedProgramDocument {
        &self.certified_program
    }

    /// Returns admission material.
    pub const fn material(&self) -> &StructuredAdmissionMaterial {
        &self.material
    }

    /// Returns initial root values.
    pub fn initial_values(&self) -> &[ProposedCanonicalValue] {
        &self.initial_values
    }

    /// Returns the append request identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Consumes the command into owned parts for store fold preparation.
    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        TenantScopeId,
        InvocationIdentity,
        StableId,
        CertifiedProgramDocument,
        StructuredAdmissionMaterial,
        Vec<ProposedCanonicalValue>,
        AppendRequestId,
    ) {
        (
            self.tenant_scope_id,
            self.invocation_identity,
            self.entry_point_operation_id,
            self.certified_program,
            self.material,
            self.initial_values,
            self.append_request_id,
        )
    }
}

/// Producer-free state callback result selected for one current occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedTransitionValue {
    /// Successful state output plus exact fact proposals.
    Success {
        /// Canonical successful value.
        value: ProposedCanonicalValue,
        /// Exact callback-authored facts.
        facts: FactSet,
    },
    /// Typed state failure; failed transitions cannot emit facts.
    Failure(ProposedCanonicalValue),
}

/// One physical transition append request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransitionProposal {
    append_request_id: AppendRequestId,
    value: ProposedTransitionValue,
}

impl StateTransitionProposal {
    /// Constructs one successful callback proposal.
    #[doc(hidden)]
    pub fn success(
        append_request_id: AppendRequestId,
        value: ProposedCanonicalValue,
        facts: FactSet,
    ) -> Self {
        Self {
            append_request_id,
            value: ProposedTransitionValue::Success { value, facts },
        }
    }

    /// Constructs one typed failure callback proposal.
    #[doc(hidden)]
    pub fn failure(append_request_id: AppendRequestId, value: ProposedCanonicalValue) -> Self {
        Self {
            append_request_id,
            value: ProposedTransitionValue::Failure(value),
        }
    }

    /// Returns the append request identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the proposed transition value.
    pub const fn value(&self) -> &ProposedTransitionValue {
        &self.value
    }
}

/// Exact current input, request, and public certificate for one current access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAuthorizationProposal {
    append_request_id: AppendRequestId,
    state_input_ref: LexicalValueRef,
    request: ProposedCanonicalValue,
    physical_binding_certificate: HistoryObject,
}

impl AccessAuthorizationProposal {
    /// Constructs one access proposal from a sealed binding's public material.
    #[doc(hidden)]
    pub fn new(
        append_request_id: AppendRequestId,
        state_input_ref: LexicalValueRef,
        request: ProposedCanonicalValue,
        physical_binding_certificate: HistoryObject,
    ) -> Self {
        Self {
            append_request_id,
            state_input_ref,
            request,
            physical_binding_certificate,
        }
    }

    /// Returns the append request identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the state input reference.
    pub const fn state_input_ref(&self) -> &LexicalValueRef {
        &self.state_input_ref
    }

    /// Returns the authored request value.
    pub const fn request(&self) -> &ProposedCanonicalValue {
        &self.request
    }

    /// Returns the public physical binding certificate.
    pub const fn physical_binding_certificate(&self) -> &HistoryObject {
        &self.physical_binding_certificate
    }
}

/// Producer-free completion returned after consuming one affine invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedObservationOutcome {
    /// Exact returned response.
    Returned(ProposedCanonicalValue),
    /// Exact reviewed state-facing safe failure.
    SafeFailure(ProposedCanonicalValue),
    /// Qualified proof that a refreshable Effect did not enter.
    SupersededBeforeEntry {
        /// Current public lineage-head certificate.
        public_lineage_head: Box<HistoryObject>,
        /// Exact typed refresh evidence.
        evidence: ProposedCanonicalValue,
    },
    /// Effect entry may have happened.
    EntryUnknown {
        /// Stable reviewed redaction-safe fault code.
        fault_code: StableId,
    },
    /// Integrity evidence blocks semantic progress.
    IntegrityFault {
        /// Stable reviewed redaction-safe fault code.
        fault_code: StableId,
    },
}

/// Stable observation material independent of a predecessor-bound envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessObservationProposal {
    append_request_id: AppendRequestId,
    authorization_ref: RecordRef,
    outcome: ProposedObservationOutcome,
}

impl AccessObservationProposal {
    /// Binds one completion to the exact committed authorization it consumed.
    #[doc(hidden)]
    pub fn new(
        append_request_id: AppendRequestId,
        authorization_ref: RecordRef,
        outcome: ProposedObservationOutcome,
    ) -> Self {
        Self {
            append_request_id,
            authorization_ref,
            outcome,
        }
    }

    /// Returns the append request identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the authorization record reference.
    pub const fn authorization_ref(&self) -> &RecordRef {
        &self.authorization_ref
    }

    /// Returns the observation outcome.
    pub const fn outcome(&self) -> &ProposedObservationOutcome {
        &self.outcome
    }
}
