//! Owner qualification of immutable structured history.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mfm_certify::structured::{AdmissionVerificationRegistry, CertifiedProgram};
use mfm_ids::{
    ContentDigest, ContentRef, RunId, RunSemanticStateDigest, SchemaId, StableId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    derive_candidate_digest, derive_commit_digest, derive_record_hash, derive_run_id, AccessKind,
    AdmissionMaterialRefs, AssignedRecord, CommitCandidate, CommitDigestPreimage, CommittedBatch,
    ExternalAccessAuthorized, ExternalAccessObserved, HistoryObject, JournalHead, LexicalValueRef,
    RecordHashPreimage, RunAdmitted, RunClosed, RunRecord, StateTransitionCommitted, TypedValueRef,
};
use mfm_runtime::history::{
    ProposedObservationOutcome, ProposedTransitionValue, QualifiedRuntimeIntent,
    StructuredAdmissionCommand,
};
use mfm_spec::structured::{
    CertifiedProgramDocument, CertifiedProgramRoot, SecretFreeImplementationManifest,
    StateCapabilityAdapterSignerResourceManifest, StructuredCapabilityProtocolContract,
    StructuredComponentKind, StructuredEffectRefreshContract, StructuredLiveComponentContract,
};
use mfm_spec::CanonicalJsonValue;
use mfm_values::CanonicalJsonPersistedSchema;

use super::backend::{RawRunHistory, StructuredStoreIdentity};
use super::reducer::{ObservationValueIntent, QualifiedIntentEvent, TransitionValueIntent};
use super::validated_append::{MAX_BATCH_OBJECTS, MAX_BATCH_RECORDS, MAX_STORED_FRAME_BYTES};

/// Stable redaction-safe structured store failure.
pub type StructuredStoreError = mfm_runtime::history::HistoryError;

pub(super) const fn invalid() -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}

/// The one concrete certification registry accepted by history qualification.
pub struct ProgramVerificationRegistry {
    registry: AdmissionVerificationRegistry,
    certified: Mutex<BTreeMap<(StableId, ContentRef), Arc<CertifiedProgram>>>,
}

impl std::fmt::Debug for ProgramVerificationRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProgramVerificationRegistry")
            .finish_non_exhaustive()
    }
}

impl ProgramVerificationRegistry {
    /// Wraps the certification-owned admission registry.
    pub fn new(registry: AdmissionVerificationRegistry) -> Self {
        Self {
            registry,
            certified: Mutex::new(BTreeMap::new()),
        }
    }

    fn qualify(
        &self,
        entry_point_operation_id: &StableId,
        root_ref: &ContentRef,
        root: &CertifiedProgramRoot,
        authored: &CanonicalJsonValue,
    ) -> Result<Arc<CertifiedProgram>, StructuredStoreError> {
        let key = (entry_point_operation_id.clone(), root_ref.clone());
        if let Some(cached) = self
            .certified
            .lock()
            .map_err(|_| StructuredStoreError::Certification)?
            .get(&key)
            .cloned()
        {
            return program_matches(&cached, entry_point_operation_id, root, authored)
                .then_some(cached)
                .ok_or(StructuredStoreError::Certification);
        }
        let certified = Arc::new(
            self.registry
                .verify_root(entry_point_operation_id, root, authored)
                .map_err(|_| StructuredStoreError::Certification)?,
        );
        if !program_matches(&certified, entry_point_operation_id, root, authored) {
            return Err(StructuredStoreError::Certification);
        }
        let mut memo = self
            .certified
            .lock()
            .map_err(|_| StructuredStoreError::Certification)?;
        match memo.get(&key) {
            Some(existing) if existing.document().root == certified.document().root => {
                Ok(Arc::clone(existing))
            }
            Some(_) => Err(StructuredStoreError::Certification),
            None => {
                memo.insert(key, Arc::clone(&certified));
                Ok(certified)
            }
        }
    }
}

fn program_matches(
    certified: &CertifiedProgram,
    entry_point_operation_id: &StableId,
    root: &CertifiedProgramRoot,
    authored: &CanonicalJsonValue,
) -> bool {
    certified.document().root == *root
        && certified.expanded().operation_id == *entry_point_operation_id
        && certified
            .document()
            .component_closure
            .iter()
            .find(|object| {
                object.content_ref == certified.document().root.components.authored_program_ref
            })
            .is_some_and(|object| &object.value == authored)
}

/// Exact public inputs to physical authorization verification.
pub struct PhysicalBindingAuthorization<'a> {
    /// Read or Effect access kind.
    pub access_kind: AccessKind,
    /// Semantic capability contract.
    pub capability_contract_ref: &'a ContentRef,
    /// Qualified capability implementation.
    pub capability_implementation_ref: &'a ContentRef,
    /// Semantic adapter contract.
    pub adapter_contract_ref: &'a ContentRef,
    /// Qualified adapter implementation.
    pub adapter_implementation_ref: &'a ContentRef,
    /// Admission routing policy.
    pub admitted_routing_policy_ref: &'a ContentRef,
    /// Optional admitted resource lineage.
    pub stable_resource_lineage_contract_ref: Option<&'a ContentRef>,
    /// Retained non-rollback floor.
    pub minimum_lineage_head_ref: Option<&'a ContentRef>,
    /// Exact preceding physical binding.
    pub previous_physical_binding_ref: Option<&'a ContentRef>,
}

/// Exact public inputs to physical supersession verification.
pub struct PhysicalBindingSupersession<'a> {
    /// Semantic capability contract.
    pub capability_contract_ref: &'a ContentRef,
    /// Semantic adapter contract.
    pub adapter_contract_ref: &'a ContentRef,
    /// Qualified adapter implementation.
    pub adapter_implementation_ref: &'a ContentRef,
    /// Authorized physical binding.
    pub authorized_binding_ref: &'a ContentRef,
    /// Stable admitted lineage.
    pub stable_resource_lineage_contract_ref: &'a ContentRef,
    /// Claimed monotonic public lineage head.
    pub public_lineage_head_ref: &'a ContentRef,
}

/// Purpose-limited verifier of secret-free physical evidence.
pub trait PhysicalObligationChecker:
    mfm_authority_seal::PhysicalBindingVerifierSeal + Send + Sync
{
    /// Checks retained authorization evidence.
    fn verify_retained_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> Result<(), StructuredStoreError>;

    /// Checks authorization evidence against the current release.
    fn verify_current_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> Result<(), StructuredStoreError>;

    /// Checks retained supersession evidence.
    fn verify_retained_supersession(
        &self,
        context: &PhysicalBindingSupersession<'_>,
        public_lineage_head: &HistoryObject,
        evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError>;

    /// Checks supersession evidence against the current release.
    fn verify_current_supersession(
        &self,
        context: &PhysicalBindingSupersession<'_>,
        public_lineage_head: &HistoryObject,
        evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QualifiedAdmission {
    pub(super) store_scope_id: StoreScopeId,
    pub(super) store_epoch: StoreEpoch,
    pub(super) run_id: RunId,
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) invocation_identity: mfm_ids::InvocationIdentity,
    pub(super) entry_point_operation_id: StableId,
    pub(super) certified_program_ref: ContentRef,
    pub(super) admission_material_refs: mfm_journal::structured::AdmissionMaterialRefs,
    pub(super) initial_bindings: Vec<mfm_journal::structured::LexicalValueRef>,
    pub(super) recorded_genesis: Option<RunSemanticStateDigest>,
}

impl QualifiedAdmission {
    pub(super) fn record(
        &self,
        genesis_semantic_state_digest: RunSemanticStateDigest,
    ) -> RunAdmitted {
        RunAdmitted {
            store_scope_id: self.store_scope_id.clone(),
            store_epoch: self.store_epoch,
            run_id: self.run_id.clone(),
            tenant_scope_id: self.tenant_scope_id.clone(),
            invocation_identity: self.invocation_identity.clone(),
            entry_point_operation_id: self.entry_point_operation_id.clone(),
            certified_program_ref: self.certified_program_ref.clone(),
            admission_material_refs: self.admission_material_refs.clone(),
            initial_bindings: self.initial_bindings.clone(),
            genesis_semantic_state_digest,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct QualifiedRunContext {
    pub(super) program: Arc<CertifiedProgram>,
    pub(super) admission: QualifiedAdmission,
    pub(super) values: Arc<QualifiedValueIndex>,
    pub(super) live_components: BTreeMap<ContentRef, StructuredLiveComponentContract>,
    pub(super) implementation_manifest: SecretFreeImplementationManifest,
    pub(super) admission_object_refs: Vec<ContentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QualifiedValueIndex {
    objects: BTreeMap<ContentRef, HistoryObject>,
    values: BTreeMap<ContentRef, CanonicalJsonValue>,
}

impl QualifiedValueIndex {
    pub(super) fn object(&self, reference: &ContentRef) -> Option<&HistoryObject> {
        self.objects.get(reference)
    }

    pub(super) fn value(&self, reference: &ContentRef) -> Option<&CanonicalJsonValue> {
        self.values.get(reference)
    }

    pub(super) fn objects(&self) -> &BTreeMap<ContentRef, HistoryObject> {
        &self.objects
    }
}

impl QualifiedRunContext {
    pub(super) fn lexical_slot_ref(
        &self,
        slot: &mfm_spec::structured::LexicalSlot,
    ) -> Result<&ContentRef, StructuredStoreError> {
        self.program.lexical_slot_ref(slot).ok_or_else(invalid)
    }

    pub(super) fn value(&self, reference: &ContentRef) -> Option<&CanonicalJsonValue> {
        self.values.value(reference)
    }

    pub(super) fn value_schema_id(
        &self,
        contract_ref: &ContentRef,
    ) -> Result<SchemaId, StructuredStoreError> {
        if let Some(schema) = self.program.value_schema(contract_ref) {
            return schema.schema_id().map_err(|_| invalid());
        }
        self.program
            .document()
            .component_closure
            .iter()
            .find(|component| &component.content_ref == contract_ref)
            .filter(|component| {
                matches!(
                    component.object_type.as_str(),
                    "structured.lane_outcome_contract" | "structured.fan_out_join_contract"
                )
            })
            .map(|_| contract_ref.schema_id().clone())
            .ok_or_else(invalid)
    }

    pub(super) fn live_component(
        &self,
        reference: &ContentRef,
    ) -> Result<&StructuredLiveComponentContract, StructuredStoreError> {
        self.live_components.get(reference).ok_or_else(invalid)
    }

    pub(super) fn implementation_ref(
        &self,
        kind: mfm_spec::structured::StructuredComponentKind,
        semantic_ref: &ContentRef,
    ) -> Result<&ContentRef, StructuredStoreError> {
        self.implementation_manifest
            .entries
            .iter()
            .find(|entry| {
                entry.component_kind == kind && &entry.semantic_contract_ref == semantic_ref
            })
            .map(|entry| &entry.implementation_contract_ref)
            .ok_or_else(invalid)
    }

    pub(super) fn object(&self, reference: &ContentRef) -> Option<&HistoryObject> {
        self.values.object(reference)
    }

    pub(super) fn validate_value(
        &self,
        contract_ref: &ContentRef,
        value: &CanonicalJsonValue,
    ) -> Result<(), StructuredStoreError> {
        if contains_secret_marker(value.as_json()) {
            return Err(invalid());
        }
        validate_value_shape(self, contract_ref, value.as_json(), 0)
    }
}

pub(super) fn qualify_admission_intent(
    identity: &StructuredStoreIdentity,
    command: StructuredAdmissionCommand,
    programs: &ProgramVerificationRegistry,
) -> Result<Arc<QualifiedRunContext>, StructuredStoreError> {
    let (
        tenant_scope_id,
        invocation_identity,
        entry_point_operation_id,
        document,
        material,
        initial_values,
        _append_request_id,
    ) = command.into_parts();
    let run_id = derive_run_id(
        &identity.store_scope_id,
        &tenant_scope_id,
        &entry_point_operation_id,
        &invocation_identity,
    )
    .map_err(|_| invalid())?;
    let root_ref = document
        .root
        .content_ref()
        .map_err(|_| StructuredStoreError::Certification)?;
    let authored = document
        .component_closure
        .iter()
        .find(|object| object.content_ref == document.root.components.authored_program_ref)
        .map(|object| &object.value)
        .ok_or(StructuredStoreError::Certification)?;
    let program = programs.qualify(
        &entry_point_operation_id,
        &root_ref,
        &document.root,
        authored,
    )?;
    if program.document() != &document
        || program.expanded().input_roots.len() != initial_values.len()
    {
        return Err(StructuredStoreError::Certification);
    }

    let mut objects = vec![HistoryObject::from_persisted(&document.root)
        .map_err(|_| StructuredStoreError::Certification)?];
    for component in &document.component_closure {
        let object = HistoryObject {
            object_type: component.object_type.clone(),
            content_ref: component.content_ref.clone(),
            canonical_json: component
                .value
                .canonical_json()
                .map_err(|_| StructuredStoreError::Certification)?
                .as_str()
                .to_owned(),
        };
        object
            .validate()
            .map_err(|_| StructuredStoreError::Certification)?;
        objects.push(object);
    }
    let admission_material_refs = AdmissionMaterialRefs {
        configuration_ref: material.configuration.content_ref.clone(),
        context_manifest_ref: material.context_manifest.content_ref.clone(),
        prior_run_source_manifest_ref: material.prior_run_source_manifest.content_ref.clone(),
        routing_policy_ref: material.routing_policy.content_ref.clone(),
        stable_resource_lineage_contract_refs: material.stable_resource_lineage_contract_refs,
    };
    objects.extend([
        material.configuration,
        material.context_manifest,
        material.prior_run_source_manifest,
        material.routing_policy,
    ]);
    let mut initial_bindings = Vec::with_capacity(initial_values.len());
    for (slot, proposed) in program.expanded().input_roots.iter().zip(initial_values) {
        let value = canonical_proposal(&proposed)?;
        let schema = program
            .value_schema(&slot.contract_ref)
            .ok_or_else(invalid)?;
        schema
            .validate_canonical_value(proposed.canonical().as_bytes())
            .map_err(|_| invalid())?;
        if contains_secret_marker(value.as_json()) {
            return Err(invalid());
        }
        let schema_id = schema.schema_id().map_err(|_| invalid())?;
        let value_ref = ContentRef::new(
            schema_id,
            ContentDigest::from_digest(
                mfm_ids::DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(proposed.canonical().as_bytes()),
            ),
        )
        .map_err(|_| invalid())?;
        objects.push(HistoryObject {
            object_type: StableId::new(mfm_journal::structured::TYPED_VALUE_OBJECT_TYPE)
                .map_err(|_| invalid())?,
            content_ref: value_ref.clone(),
            canonical_json: proposed.canonical().as_str().to_owned(),
        });
        initial_bindings.push(LexicalValueRef::new(
            program
                .lexical_slot_ref(slot)
                .cloned()
                .ok_or_else(invalid)?,
            TypedValueRef {
                contract_ref: slot.contract_ref.clone(),
                value_ref,
            },
        ));
    }
    let values = Arc::new(qualify_object_index(objects)?);
    require_component_object_closure(values.objects(), program.document())?;
    let live_components = qualify_live_components(program.document())?;
    let component_manifest = component_manifest(values.objects(), &program)?;
    let implementation_manifest = values
        .object(
            &program
                .document()
                .root
                .components
                .secret_free_implementation_manifest_closure_ref,
        )
        .ok_or_else(invalid)?
        .decode_persisted::<SecretFreeImplementationManifest>()
        .map_err(|_| StructuredStoreError::Certification)?;
    let mut admission_object_refs = values.objects().keys().cloned().collect::<Vec<_>>();
    admission_object_refs.sort();
    let admission = QualifiedAdmission {
        store_scope_id: identity.store_scope_id.clone(),
        store_epoch: identity.store_epoch,
        run_id,
        tenant_scope_id,
        invocation_identity,
        entry_point_operation_id,
        certified_program_ref: root_ref,
        admission_material_refs,
        initial_bindings,
        recorded_genesis: None,
    };
    qualify_admission_material_parts(
        &admission.admission_material_refs,
        &admission.initial_bindings,
        values.objects(),
        &program,
        &component_manifest,
        &live_components,
    )?;
    require_exact_admission_object_refs(
        values.objects().keys().cloned().collect(),
        &admission.certified_program_ref,
        &admission.admission_material_refs,
        &admission.initial_bindings,
        &program,
    )?;
    Ok(Arc::new(QualifiedRunContext {
        program,
        admission,
        values,
        live_components,
        implementation_manifest,
        admission_object_refs,
    }))
}

pub(super) fn qualify_existing_intent(
    retained: &Arc<QualifiedRunContext>,
    intent: QualifiedRuntimeIntent,
) -> Result<(Arc<QualifiedRunContext>, QualifiedIntentEvent), StructuredStoreError> {
    let (objects, event) = match intent {
        QualifiedRuntimeIntent::Transition(proposal) => {
            let value = match proposal.value() {
                ProposedTransitionValue::Success { value, facts } => {
                    TransitionValueIntent::Success {
                        value: canonical_proposal(value)?,
                        facts: facts.clone(),
                    }
                }
                ProposedTransitionValue::Failure(value) => {
                    TransitionValueIntent::Failure(canonical_proposal(value)?)
                }
            };
            (Vec::new(), QualifiedIntentEvent::Transition { value })
        }
        QualifiedRuntimeIntent::Authorization(proposal) => (
            vec![proposal.physical_binding_certificate().clone()],
            QualifiedIntentEvent::Authorization {
                state_input_ref: proposal.state_input_ref().clone(),
                request: canonical_proposal(proposal.request())?,
                physical_binding_ref: proposal.physical_binding_certificate().content_ref.clone(),
            },
        ),
        QualifiedRuntimeIntent::Observation(proposal) => {
            let (objects, outcome) = match proposal.outcome() {
                ProposedObservationOutcome::Returned(value) => (
                    Vec::new(),
                    ObservationValueIntent::Returned(canonical_proposal(value)?),
                ),
                ProposedObservationOutcome::SafeFailure(value) => (
                    Vec::new(),
                    ObservationValueIntent::SafeFailure(canonical_proposal(value)?),
                ),
                ProposedObservationOutcome::SupersededBeforeEntry {
                    public_lineage_head,
                    evidence,
                } => (
                    vec![(**public_lineage_head).clone()],
                    ObservationValueIntent::SupersededBeforeEntry {
                        public_lineage_head_ref: public_lineage_head.content_ref.clone(),
                        evidence: canonical_proposal(evidence)?,
                    },
                ),
                ProposedObservationOutcome::EntryUnknown { fault_code } => (
                    Vec::new(),
                    ObservationValueIntent::EntryUnknown {
                        fault_code: fault_code.clone(),
                    },
                ),
                ProposedObservationOutcome::IntegrityFault { fault_code } => (
                    Vec::new(),
                    ObservationValueIntent::IntegrityFault {
                        fault_code: fault_code.clone(),
                    },
                ),
            };
            (
                objects,
                QualifiedIntentEvent::Observation {
                    authorization_ref: proposal.authorization_ref().clone(),
                    outcome,
                },
            )
        }
        QualifiedRuntimeIntent::Admission(_) => {
            return Err(invalid());
        }
    };
    let mut context = (**retained).clone();
    if !objects.is_empty() {
        let mut combined = retained.values.objects.clone();
        for object in objects {
            object.validate().map_err(|_| invalid())?;
            match combined.get(&object.content_ref) {
                Some(existing) if existing == &object => {}
                Some(_) => return Err(invalid()),
                None => {
                    combined.insert(object.content_ref.clone(), object);
                }
            }
        }
        context.values = Arc::new(qualify_object_index(combined.into_values().collect())?);
    }
    Ok((Arc::new(context), event))
}

fn canonical_proposal(
    proposed: &mfm_runtime::history::ProposedCanonicalValue,
) -> Result<CanonicalJsonValue, StructuredStoreError> {
    CanonicalJsonValue::from_exact_bytes(proposed.canonical().as_bytes()).map_err(|_| invalid())
}

fn qualify_object_index(
    objects: Vec<HistoryObject>,
) -> Result<QualifiedValueIndex, StructuredStoreError> {
    let mut qualified_objects = BTreeMap::new();
    let mut values = BTreeMap::new();
    for object in objects {
        object.validate().map_err(|_| invalid())?;
        let value = CanonicalJsonValue::from_exact_bytes(object.canonical_json.as_bytes())
            .map_err(|_| invalid())?;
        match qualified_objects.get(&object.content_ref) {
            Some(existing) if existing == &object => continue,
            Some(_) => return Err(invalid()),
            None => {}
        }
        values.insert(object.content_ref.clone(), value);
        qualified_objects.insert(object.content_ref.clone(), object);
    }
    Ok(QualifiedValueIndex {
        objects: qualified_objects,
        values,
    })
}

fn validate_value_shape(
    context: &QualifiedRunContext,
    contract_ref: &ContentRef,
    value: &serde_json::Value,
    depth: u8,
) -> Result<(), StructuredStoreError> {
    if depth > 8 {
        return Err(invalid());
    }
    if let Some(schema) = context.program.value_schema(contract_ref) {
        let encoded = serde_json::to_string(value).map_err(|_| invalid())?;
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&encoded)
            .map_err(|_| invalid())?;
        return schema
            .validate_canonical_value(canonical.as_bytes())
            .map_err(|_| invalid());
    }
    let component = context
        .program
        .document()
        .component_closure
        .iter()
        .find(|component| &component.content_ref == contract_ref)
        .ok_or_else(invalid)?;
    match component.object_type.as_str() {
        "structured.lane_outcome_contract" => {
            let contract = context
                .object(contract_ref)
                .ok_or_else(invalid)?
                .decode_persisted::<mfm_spec::structured::LaneOutcomeContract>()
                .map_err(|_| invalid())?;
            let object = value
                .as_object()
                .filter(|object| object.len() == 1)
                .ok_or_else(invalid)?;
            if let Some(value) = object.get("Success") {
                validate_value_shape(context, contract.success_contract_ref(), value, depth + 1)
            } else if let Some(value) = object.get("Failure") {
                let mfm_spec::structured::StructuredFailureContract::Typed { contract_ref, .. } =
                    contract.failure_contract()
                else {
                    return Err(invalid());
                };
                validate_value_shape(context, contract_ref, value, depth + 1)
            } else {
                Err(invalid())
            }
        }
        "structured.fan_out_join_contract" => {
            let contract = context
                .object(contract_ref)
                .ok_or_else(invalid)?
                .decode_persisted::<mfm_spec::structured::FanOutJoinContract>()
                .map_err(|_| invalid())?;
            let lane = contract.lane_outcome_contract_ref();
            let object = value
                .as_object()
                .filter(|object| object.len() == 2)
                .ok_or_else(invalid)?;
            validate_value_shape(
                context,
                lane,
                object.get("head").ok_or_else(invalid)?,
                depth + 1,
            )?;
            for value in object
                .get("tail")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(invalid)?
            {
                validate_value_shape(context, lane, value, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(invalid()),
    }
}

fn contains_secret_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => mfm_values::string_contains_secret_marker(value),
        serde_json::Value::Array(values) => values.iter().any(contains_secret_marker),
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            mfm_values::string_contains_secret_marker(key) || contains_secret_marker(value)
        }),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Recorded payloads stay inline so qualification does not allocate per event.
#[allow(clippy::large_enum_variant)]
pub(super) enum QualifiedRecordedEvent {
    Admission {
        admission: RunAdmitted,
        closure: Option<RunClosed>,
    },
    Transition {
        transition: StateTransitionCommitted,
        closure: Option<RunClosed>,
    },
    Authorization(ExternalAccessAuthorized),
    Observation(ExternalAccessObserved),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedAssertions {
    pub(super) records: Vec<AssignedRecord>,
    pub(super) head: JournalHead,
    pub(super) object_refs: Vec<ContentRef>,
    pub(super) candidate_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QualifiedBatch {
    pub(super) committed: CommittedBatch,
    pub(super) event: QualifiedRecordedEvent,
    pub(super) assertions: RecordedAssertions,
}

#[derive(Debug)]
pub(super) struct QualifiedHistory {
    pub(super) context: Arc<QualifiedRunContext>,
    pub(super) run_id: RunId,
    pub(super) store_identity: StructuredStoreIdentity,
    pub(super) batches: Vec<QualifiedBatch>,
    pub(super) objects: Arc<QualifiedValueIndex>,
    pub(super) object_first_seen_sequence: BTreeMap<ContentRef, u64>,
}

impl QualifiedHistory {
    pub(super) fn records(&self) -> impl Iterator<Item = &AssignedRecord> {
        self.batches
            .iter()
            .flat_map(|batch| batch.committed.records.iter())
    }
}

pub(super) fn qualify_recorded_history(
    raw: RawRunHistory,
    programs: &ProgramVerificationRegistry,
) -> Result<Arc<QualifiedHistory>, StructuredStoreError> {
    if raw.batches.is_empty() {
        return Err(invalid());
    }
    let mut previous = None;
    let mut lineage = None;
    for batch in &raw.batches {
        verify_batch_envelope(&raw.run_id, batch, previous.as_ref(), lineage.as_ref())?;
        lineage.get_or_insert_with(|| (batch.store_scope_id.clone(), batch.store_epoch));
        previous = Some(batch.head.clone());
    }

    let first_assigned = raw.batches[0]
        .records
        .first()
        .cloned()
        .ok_or_else(invalid)?;
    let RunRecord::RunAdmitted(admission) = first_assigned.record.clone() else {
        return Err(invalid());
    };
    if admission.run_id != raw.run_id
        || derive_run_id(
            &admission.store_scope_id,
            &admission.tenant_scope_id,
            &admission.entry_point_operation_id,
            &admission.invocation_identity,
        )
        .map_err(|_| invalid())?
            != raw.run_id
    {
        return Err(invalid());
    }

    let mut first_seen = BTreeMap::new();
    let mut retained_refs = BTreeSet::new();
    let mut retained_objects = Vec::new();
    for batch in &raw.batches {
        validate_record_object_closure(batch, &retained_refs)?;
        for object in &batch.objects {
            if retained_refs.insert(object.content_ref.clone()) {
                first_seen.insert(object.content_ref.clone(), batch.head.run_sequence);
            }
            retained_objects.push(object.clone());
        }
    }
    let object_index = Arc::new(qualify_object_index(retained_objects)?);
    let root_object = object_index
        .object(&admission.certified_program_ref)
        .ok_or_else(invalid)?;
    let root = root_object
        .decode_persisted::<CertifiedProgramRoot>()
        .map_err(|_| StructuredStoreError::Certification)?;
    let authored_object = object_index
        .object(&root.components.authored_program_ref)
        .ok_or_else(invalid)?;
    let authored =
        CanonicalJsonValue::from_canonical_json(authored_object.canonical_json.as_bytes())
            .map_err(|_| StructuredStoreError::Certification)?;
    let program = programs.qualify(
        &admission.entry_point_operation_id,
        &root_object.content_ref,
        &root,
        &authored,
    )?;
    require_component_object_closure(object_index.objects(), program.document())?;
    let live_components = qualify_live_components(program.document())?;
    let component_manifest = component_manifest(object_index.objects(), &program)?;
    qualify_admission_material_parts(
        &admission.admission_material_refs,
        &admission.initial_bindings,
        object_index.objects(),
        &program,
        &component_manifest,
        &live_components,
    )?;
    require_exact_admission_object_refs(
        raw.batches[0]
            .objects
            .iter()
            .map(|object| object.content_ref.clone())
            .collect(),
        &admission.certified_program_ref,
        &admission.admission_material_refs,
        &admission.initial_bindings,
        &program,
    )?;
    let implementation_manifest = object_index
        .object(
            &program
                .document()
                .root
                .components
                .secret_free_implementation_manifest_closure_ref,
        )
        .ok_or_else(invalid)?
        .decode_persisted::<SecretFreeImplementationManifest>()
        .map_err(|_| StructuredStoreError::Certification)?;

    let mut batches = Vec::with_capacity(raw.batches.len());
    for (index, committed) in raw.batches.into_iter().enumerate() {
        batches.push(qualify_batch(index == 0, committed)?);
    }
    let (store_scope_id, store_epoch) = lineage.ok_or_else(invalid)?;
    let context = Arc::new(QualifiedRunContext {
        program,
        admission: QualifiedAdmission {
            store_scope_id: admission.store_scope_id.clone(),
            store_epoch: admission.store_epoch,
            run_id: admission.run_id.clone(),
            tenant_scope_id: admission.tenant_scope_id.clone(),
            invocation_identity: admission.invocation_identity.clone(),
            entry_point_operation_id: admission.entry_point_operation_id.clone(),
            certified_program_ref: admission.certified_program_ref.clone(),
            admission_material_refs: admission.admission_material_refs.clone(),
            initial_bindings: admission.initial_bindings.clone(),
            recorded_genesis: Some(admission.genesis_semantic_state_digest.clone()),
        },
        values: Arc::clone(&object_index),
        live_components,
        implementation_manifest,
        admission_object_refs: batches
            .first()
            .ok_or_else(invalid)?
            .assertions
            .object_refs
            .clone(),
    });
    Ok(Arc::new(QualifiedHistory {
        context,
        run_id: raw.run_id,
        store_identity: StructuredStoreIdentity {
            store_scope_id,
            store_epoch,
            physical_target: None,
        },
        batches,
        objects: object_index,
        object_first_seen_sequence: first_seen,
    }))
}

pub(super) fn qualify_recorded_successor(
    prior: &Arc<QualifiedHistory>,
    committed: CommittedBatch,
) -> Result<Arc<QualifiedHistory>, StructuredStoreError> {
    let previous_head = prior
        .batches
        .last()
        .map(|batch| &batch.committed.head)
        .ok_or_else(invalid)?;
    verify_batch_envelope(
        &prior.run_id,
        &committed,
        Some(previous_head),
        Some(&(
            prior.store_identity.store_scope_id.clone(),
            prior.store_identity.store_epoch,
        )),
    )?;
    let retained_refs = prior.objects.objects.keys().cloned().collect();
    validate_record_object_closure(&committed, &retained_refs)?;

    let mut objects = prior.objects.objects.clone();
    let mut values = prior.objects.values.clone();
    let mut first_seen = prior.object_first_seen_sequence.clone();
    for object in &committed.objects {
        if objects.contains_key(&object.content_ref) {
            return Err(invalid());
        }
        let value = CanonicalJsonValue::from_canonical_json(object.canonical_json.as_bytes())
            .map_err(|_| invalid())?;
        first_seen.insert(object.content_ref.clone(), committed.head.run_sequence);
        values.insert(object.content_ref.clone(), value);
        objects.insert(object.content_ref.clone(), object.clone());
    }
    let object_index = Arc::new(QualifiedValueIndex { objects, values });
    let mut batches = prior.batches.clone();
    batches.push(qualify_batch(false, committed)?);
    let mut context = (*prior.context).clone();
    context.values = Arc::clone(&object_index);
    Ok(Arc::new(QualifiedHistory {
        context: Arc::new(context),
        run_id: prior.run_id.clone(),
        store_identity: prior.store_identity.clone(),
        batches,
        objects: object_index,
        object_first_seen_sequence: first_seen,
    }))
}

fn qualify_batch(
    first: bool,
    committed: CommittedBatch,
) -> Result<QualifiedBatch, StructuredStoreError> {
    let event = qualify_recorded_event(first, &committed.records)?;
    let assertions = RecordedAssertions {
        records: committed.records.clone(),
        head: committed.head.clone(),
        object_refs: committed
            .objects
            .iter()
            .map(|object| object.content_ref.clone())
            .collect(),
        candidate_digest: committed.candidate_digest.clone(),
    };
    Ok(QualifiedBatch {
        committed,
        event,
        assertions,
    })
}

fn qualify_recorded_event(
    first_batch: bool,
    records: &[AssignedRecord],
) -> Result<QualifiedRecordedEvent, StructuredStoreError> {
    let first = records.first().ok_or_else(invalid)?;
    let closure = records
        .get(1)
        .map(|record| match &record.record {
            RunRecord::RunClosed(closed) => Ok(closed.clone()),
            _ => Err(invalid()),
        })
        .transpose()?;
    if records.len() > 2 {
        return Err(invalid());
    }
    match &first.record {
        RunRecord::RunAdmitted(admission) if first_batch => Ok(QualifiedRecordedEvent::Admission {
            admission: admission.clone(),
            closure,
        }),
        RunRecord::StateTransitionCommitted(transition) if !first_batch => {
            Ok(QualifiedRecordedEvent::Transition {
                transition: transition.clone(),
                closure,
            })
        }
        RunRecord::ExternalAccessAuthorized(authorization) if !first_batch && closure.is_none() => {
            Ok(QualifiedRecordedEvent::Authorization(authorization.clone()))
        }
        RunRecord::ExternalAccessObserved(observation) if !first_batch && closure.is_none() => {
            Ok(QualifiedRecordedEvent::Observation(observation.clone()))
        }
        _ => Err(invalid()),
    }
}

fn validate_record_object_closure(
    batch: &CommittedBatch,
    prior_object_refs: &BTreeSet<ContentRef>,
) -> Result<(), StructuredStoreError> {
    if batch.objects.len() > MAX_BATCH_OBJECTS
        || batch
            .objects
            .windows(2)
            .any(|pair| pair[0].content_ref >= pair[1].content_ref)
    {
        return Err(invalid());
    }
    let actual = batch
        .objects
        .iter()
        .map(|object| object.content_ref.clone())
        .collect::<BTreeSet<_>>();
    let mut required = BTreeSet::new();
    for assigned in &batch.records {
        match &assigned.record {
            RunRecord::RunAdmitted(admission) => {
                required.extend([
                    admission.certified_program_ref.clone(),
                    admission.admission_material_refs.configuration_ref.clone(),
                    admission
                        .admission_material_refs
                        .context_manifest_ref
                        .clone(),
                    admission
                        .admission_material_refs
                        .prior_run_source_manifest_ref
                        .clone(),
                    admission.admission_material_refs.routing_policy_ref.clone(),
                ]);
                required.extend(
                    admission
                        .admission_material_refs
                        .stable_resource_lineage_contract_refs
                        .iter()
                        .cloned(),
                );
                required.extend(
                    admission
                        .initial_bindings
                        .iter()
                        .map(|binding| binding.value.value_ref.clone()),
                );
            }
            RunRecord::StateTransitionCommitted(transition) => {
                required.extend([
                    transition.input.value.value_ref.clone(),
                    transition.outcome_ref.clone(),
                ]);
                match &transition.outcome {
                    mfm_journal::structured::StateOutcomeRef::Success(value)
                    | mfm_journal::structured::StateOutcomeRef::Failure(value) => {
                        required.insert(value.value.value_ref.clone());
                    }
                }
                for fact in &transition.facts {
                    required.extend([
                        fact.descriptor_ref.clone(),
                        fact.subject.value_ref.clone(),
                        fact.response.value_ref.clone(),
                        fact.claim_ref.clone(),
                    ]);
                }
            }
            RunRecord::ExternalAccessAuthorized(authorization) => {
                required.extend([
                    authorization.state_input_ref.value.value_ref.clone(),
                    authorization.request.value_ref.clone(),
                    authorization.physical_binding_ref.clone(),
                ]);
                required.extend(
                    authorization
                        .stable_resource_lineage_contract_ref
                        .iter()
                        .cloned(),
                );
            }
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                mfm_journal::structured::ObservationOutcome::Returned { value }
                | mfm_journal::structured::ObservationOutcome::SafeFailure { value } => {
                    required.insert(value.value_ref.clone());
                }
                mfm_journal::structured::ObservationOutcome::SupersededBeforeEntry {
                    public_lineage_head_ref,
                    evidence_ref,
                } => {
                    required.extend([public_lineage_head_ref.clone(), evidence_ref.clone()]);
                }
                mfm_journal::structured::ObservationOutcome::EntryUnknown { .. }
                | mfm_journal::structured::ObservationOutcome::IntegrityFault { .. } => {}
            },
            RunRecord::RunClosed(closed) => {
                required.insert(closed.outcome_ref.clone());
            }
        }
    }
    let mut nested = BTreeSet::new();
    for object in &batch.objects {
        if object.canonical_json.len() > MAX_STORED_FRAME_BYTES {
            return Err(invalid());
        }
        let value: serde_json::Value =
            serde_json::from_str(&object.canonical_json).map_err(|_| invalid())?;
        collect_nested_content_refs(&value, 0, &mut nested)?;
    }
    required.extend(
        nested
            .into_iter()
            .filter(|reference| actual.contains(reference)),
    );
    required.retain(|reference| !prior_object_refs.contains(reference));
    if !required.is_subset(&actual) {
        return Err(invalid());
    }
    Ok(())
}

fn collect_nested_content_refs(
    value: &serde_json::Value,
    depth: usize,
    references: &mut BTreeSet<ContentRef>,
) -> Result<(), StructuredStoreError> {
    if depth > mfm_canonical::MAX_CANONICAL_JSON_DEPTH {
        return Err(invalid());
    }
    match value {
        serde_json::Value::Array(items) => {
            if items.len() > mfm_canonical::limits::MAX_ARRAY_ITEMS {
                return Err(invalid());
            }
            for item in items {
                collect_nested_content_refs(item, depth + 1, references)?;
            }
        }
        serde_json::Value::Object(entries) => {
            if entries.len() > mfm_canonical::limits::MAX_OBJECT_ENTRIES {
                return Err(invalid());
            }
            if entries.contains_key("schema_id") && entries.contains_key("content_digest") {
                if let Ok(reference) = serde_json::from_value::<ContentRef>(value.clone()) {
                    references.insert(reference);
                }
            }
            for item in entries.values() {
                collect_nested_content_refs(item, depth + 1, references)?;
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
    Ok(())
}

pub(super) fn verify_batch_envelope(
    run_id: &RunId,
    batch: &CommittedBatch,
    previous_head: Option<&JournalHead>,
    store_identity: Option<&(mfm_ids::StoreScopeId, mfm_ids::StoreEpoch)>,
) -> Result<(), StructuredStoreError> {
    if batch.records.is_empty() || batch.records.len() > MAX_BATCH_RECORDS {
        return Err(invalid());
    }
    let envelope = mfm_journal::structured::canonical_json(batch).map_err(|_| invalid())?;
    if envelope.as_bytes().len() > MAX_STORED_FRAME_BYTES {
        return Err(invalid());
    }
    if batch.predecessor.as_ref() != previous_head {
        return Err(invalid());
    }
    if store_identity
        .is_some_and(|(scope, epoch)| &batch.store_scope_id != scope || batch.store_epoch != *epoch)
    {
        return Err(invalid());
    }
    let sequence = previous_head.map_or(Ok(1), |head| {
        head.run_sequence.checked_add(1).ok_or_else(invalid)
    })?;
    if batch.head.run_sequence != sequence
        || batch
            .objects
            .windows(2)
            .any(|pair| pair[0].content_ref >= pair[1].content_ref)
    {
        return Err(invalid());
    }
    for object in &batch.objects {
        object.validate().map_err(|_| invalid())?;
    }
    let candidate = CommitCandidate {
        run_id: run_id.clone(),
        expected_head: batch.predecessor.clone(),
        append_request_id: batch.append_request_id.clone(),
        tenant_fact_coordinate: batch.tenant_fact_coordinate.clone(),
        records: batch
            .records
            .iter()
            .map(|assigned| assigned.record.clone())
            .collect(),
        objects: batch.objects.clone(),
    };
    if derive_candidate_digest(&candidate).map_err(|_| invalid())? != batch.candidate_digest {
        return Err(invalid());
    }
    for (ordinal, assigned) in batch.records.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| invalid())?;
        if assigned.record_ref.run_id != *run_id
            || assigned.record_ref.run_sequence != sequence
            || assigned.record_ref.ordinal != ordinal
            || derive_record_hash(&RecordHashPreimage {
                run_id,
                run_sequence: sequence,
                ordinal,
                record: &assigned.record,
            })
            .map_err(|_| invalid())?
                != assigned.record_ref.record_hash
        {
            return Err(invalid());
        }
    }
    let commit = derive_commit_digest(&CommitDigestPreimage {
        store_scope_id: &batch.store_scope_id,
        store_epoch: batch.store_epoch,
        predecessor: &batch.predecessor,
        append_request_id: &batch.append_request_id,
        tenant_fact_coordinate: &batch.tenant_fact_coordinate,
        candidate_digest: &batch.candidate_digest,
        record_refs: batch
            .records
            .iter()
            .map(|assigned| &assigned.record_ref)
            .collect(),
        object_refs: batch
            .objects
            .iter()
            .map(|object| &object.content_ref)
            .collect(),
    })
    .map_err(|_| invalid())?;
    if commit != batch.head.commit_digest {
        return Err(invalid());
    }
    Ok(())
}

fn require_component_object_closure(
    objects: &BTreeMap<ContentRef, HistoryObject>,
    document: &CertifiedProgramDocument,
) -> Result<(), StructuredStoreError> {
    for component in &document.component_closure {
        let object = objects.get(&component.content_ref).ok_or_else(invalid)?;
        if object.object_type != component.object_type {
            return Err(invalid());
        }
    }
    Ok(())
}

fn qualify_live_components(
    document: &CertifiedProgramDocument,
) -> Result<BTreeMap<ContentRef, StructuredLiveComponentContract>, StructuredStoreError> {
    let mut qualified = BTreeMap::new();
    for component in &document.component_closure {
        if !matches!(
            component.object_type.as_str(),
            "structured.capability_contract"
                | "structured.adapter_contract"
                | "structured.signer_contract"
                | "structured.resource_contract"
        ) {
            continue;
        }
        let value: StructuredLiveComponentContract =
            serde_json::from_value(component.value.as_json().clone())
                .map_err(|_| StructuredStoreError::Certification)?;
        value
            .validate()
            .map_err(|_| StructuredStoreError::Certification)?;
        qualified.insert(component.content_ref.clone(), value);
    }
    Ok(qualified)
}

fn component_manifest(
    objects: &BTreeMap<ContentRef, HistoryObject>,
    program: &CertifiedProgram,
) -> Result<StateCapabilityAdapterSignerResourceManifest, StructuredStoreError> {
    objects
        .get(
            &program
                .document()
                .root
                .components
                .state_capability_adapter_signer_resource_manifest_closure_ref,
        )
        .ok_or_else(invalid)?
        .decode_persisted()
        .map_err(|_| StructuredStoreError::Certification)
}

fn qualify_admission_material_parts(
    material: &AdmissionMaterialRefs,
    initial_bindings: &[LexicalValueRef],
    objects: &BTreeMap<ContentRef, HistoryObject>,
    program: &CertifiedProgram,
    component_manifest: &StateCapabilityAdapterSignerResourceManifest,
    live_components: &BTreeMap<ContentRef, StructuredLiveComponentContract>,
) -> Result<(), StructuredStoreError> {
    for (reference, expected_kind) in [
        (
            &material.configuration_ref,
            mfm_journal::structured::ADMISSION_CONFIGURATION_OBJECT_TYPE,
        ),
        (
            &material.context_manifest_ref,
            mfm_journal::structured::ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
        ),
        (
            &material.prior_run_source_manifest_ref,
            mfm_journal::structured::ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE,
        ),
        (
            &material.routing_policy_ref,
            mfm_journal::structured::ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
        ),
    ] {
        let object = objects.get(reference).ok_or_else(invalid)?;
        if object.object_type.as_str() != expected_kind {
            return Err(invalid());
        }
    }
    if program.expanded().input_roots.len() != initial_bindings.len() {
        return Err(invalid());
    }
    let mut expected_lineages = BTreeSet::new();
    for entry in component_manifest
        .entries
        .iter()
        .filter(|entry| entry.component_kind == StructuredComponentKind::Capability)
    {
        let protocol = live_components
            .get(&entry.semantic_contract_ref)
            .and_then(|component| component.capability_protocol.as_ref())
            .ok_or_else(invalid)?;
        if let StructuredCapabilityProtocolContract::Effect {
            refresh_contract:
                StructuredEffectRefreshContract::Refreshable {
                    resource_lineage_contract_ref,
                    ..
                },
            ..
        } = protocol
        {
            expected_lineages.insert(resource_lineage_contract_ref.as_ref().clone());
        }
    }
    let expected_lineages = expected_lineages.into_iter().collect::<Vec<_>>();
    if material.stable_resource_lineage_contract_refs != expected_lineages {
        return Err(invalid());
    }
    for binding in initial_bindings {
        if !objects.contains_key(&binding.value.value_ref) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn require_exact_admission_object_refs(
    actual: BTreeSet<ContentRef>,
    certified_program_ref: &ContentRef,
    material: &AdmissionMaterialRefs,
    initial_bindings: &[LexicalValueRef],
    program: &CertifiedProgram,
) -> Result<(), StructuredStoreError> {
    let mut expected = program
        .document()
        .component_closure
        .iter()
        .map(|component| component.content_ref.clone())
        .collect::<BTreeSet<_>>();
    expected.insert(certified_program_ref.clone());
    expected.extend([
        material.configuration_ref.clone(),
        material.context_manifest_ref.clone(),
        material.prior_run_source_manifest_ref.clone(),
        material.routing_policy_ref.clone(),
    ]);
    expected.extend(
        initial_bindings
            .iter()
            .map(|binding| binding.value.value_ref.clone()),
    );
    if actual != expected {
        return Err(invalid());
    }
    Ok(())
}

/// Read-only offline verification through the same qualified/reduced path.
pub fn verify_offline_recorded_history(
    raw: RawRunHistory,
    programs: &ProgramVerificationRegistry,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<super::purpose::OfflineVerifiedRun> {
    super::semantic_open::verify_offline_history(raw, programs, physical)
}
