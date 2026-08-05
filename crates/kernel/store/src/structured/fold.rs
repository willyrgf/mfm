use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_facts::{
    prior_run_fact_selector_contract_ref, FactSelectionReadResponse, FactSelectionRequest, FactSet,
    ProposedFactValue,
};
use mfm_ids::{
    AccessAttemptId, AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm,
    JournalRecordHash, OccurrenceId, RequestDigest, RunId, RunSemanticStateDigest, SchemaId,
    StableId,
};
use mfm_journal::structured::{
    canonical_json, derive_access_attempt_id, derive_commit_digest, derive_record_hash,
    domain_content_digest, AccessKind, AdmissionMaterialRefs, AssignedRecord,
    CertifiedProgramAuditRefs, CommitCandidate, CommittedBatch, CommittedFactRef,
    ExternalAccessAuthorized, ExternalAccessObserved, HistoryObject, JournalHead, LexicalValueRef,
    ObservationOutcome, PriorRunFactScannerBindingCertificate, PriorRunFactSelectionResponse,
    PriorRunFactSourceManifest, RecordLogicalKey, RecordRef, RunAdmitted, RunClosed, RunRecord,
    SemanticHead, StateOutcomeRef, StateTransitionCommitted, StructuralValueOrigin,
    TenantFactCoordinate, TypedValueRef, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE,
    ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_spec::structured::{
    fan_out_join_contract_ref, lane_outcome_contract_ref, prior_run_fact_scanner_adapter_contract,
    prior_run_fact_selection_capability_contract, BlockTail, CertifiedFailureBoundary,
    CertifiedProgramDocument, CertifiedProgramRoot, ExpandedBlock, ExpandedDeclaration,
    ExpandedFanOut, ExpandedFragment, ExpandedMatch, ExpandedStateBinding,
    ExpandedStructuredProgram, FailurePlan, HandlerContinuation, LexicalProducer, LexicalSlot,
    SecretFreeImplementationManifest, StateCapabilityAdapterSignerResourceManifest, StructuralPath,
    StructuredCapabilityProtocolContract, StructuredComponentKind, StructuredEffectRefreshContract,
    StructuredExecutionKind, StructuredFailureContract, StructuredLiveComponentContract,
};
use mfm_spec::CanonicalJsonValue;
use mfm_values::{RetainedValueContract, SchemaIdentity};
use serde::Serialize;
use serde_json::Value;

use super::backend::RawRunHistory;
use super::backend::StructuredStoreIdentity;
use super::canonical_append::MAX_BATCH_RECORDS;
use super::mutation::{
    AccessAuthorizationProposal, AccessObservationProposal, ProposedCanonicalValue,
    ProposedObservationOutcome, ProposedTransitionValue, StateTransitionProposal,
    StructuredAdmissionRequest,
};
use super::qualification::{
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalBindingVerificationMode,
    PublicPhysicalBindingVerifier,
};

/// Stable redaction-safe structured store failure.
pub type StructuredStoreError = mfm_runtime::history::HistoryError;

const CERTIFIED_ROOT_OBJECT_TYPE: &str = "structured.certified_program_root";
const TYPED_VALUE_OBJECT_TYPE: &str = "structured.typed_value";
const STATE_OUTCOME_OBJECT_TYPE: &str = "structured.state_outcome";
const OPERATION_OUTCOME_OBJECT_TYPE: &str = "structured.operation_outcome";
const DATA_CONTRACT_OBJECT_TYPE: &str = "structured.data_contract";
const LANE_OUTCOME_CONTRACT_OBJECT_TYPE: &str = "structured.lane_outcome_contract";
const FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE: &str = "structured.fan_out_join_contract";
const FACT_CLAIM_OBJECT_TYPE: &str = "structured.fact_claim";
const RESOURCE_CONTRACT_OBJECT_TYPE: &str = "structured.resource_contract";

fn invalid(_message: &'static str) -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}

/// Callback-free verified certification data required by the history fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProgramData {
    document: CertifiedProgramDocument,
    expanded: ExpandedStructuredProgram,
    value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
}

impl VerifiedProgramData {
    /// Constructs data only after concrete registry verification rechecked the
    /// exact document. Not public: only the store adapter may mint this value.
    pub(super) const fn new(
        document: CertifiedProgramDocument,
        expanded: ExpandedStructuredProgram,
        value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
    ) -> Self {
        Self {
            document,
            expanded,
            value_schemas,
        }
    }

    /// Returns the exact verified certification document.
    pub const fn document(&self) -> &CertifiedProgramDocument {
        &self.document
    }

    /// Returns the exact verified expanded program.
    pub const fn expanded(&self) -> &ExpandedStructuredProgram {
        &self.expanded
    }

    /// Returns the exact pure schema identity qualified for one retained-value
    /// contract in this certified program.
    pub fn value_schema(&self, contract_ref: &ContentRef) -> Option<&SchemaIdentity> {
        self.value_schemas.get(contract_ref)
    }
}

/// Callback-free persisted-program verification seam for the sole fold.
///
/// Production uses [`mfm_certify::structured::AdmissionVerificationRegistry`]. Offline portable
/// verification supplies the same concrete registry through a store-owned adapter.
pub trait ProgramVerifier: mfm_authority_seal::ProgramVerifierSeal + Send + Sync {
    /// Verifies one certified root against its exact authored program bytes.
    fn verify(
        &self,
        entry_point_id: &StableId,
        root: &CertifiedProgramRoot,
        authored: &CanonicalJsonValue,
    ) -> std::result::Result<Arc<VerifiedProgramData>, StructuredStoreError>;
}

pub use mfm_runtime::history::{
    ActionableState, LaneCursor, ObservationQualification, ProgramCursor, StateLeaf,
    StructuredFrontier,
};

/// One complete callback-free verified structured run view.
pub struct VerifiedStructuredRun {
    state: Box<VerifiedFoldState>,
    program: Arc<VerifiedProgramData>,
    journal_head: JournalHead,
    semantic_head: SemanticHead,
}

#[derive(PartialEq, Eq)]
struct VerifiedFoldState {
    continuation: FoldMachine,
    derived: DerivedProgram,
}

impl std::fmt::Debug for VerifiedStructuredRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedStructuredRun")
            .field("run_id", &self.state.continuation.run_id)
            .field("cursor", &self.state.derived.cursor)
            .field("journal_head", &self.journal_head)
            .finish_non_exhaustive()
    }
}

impl VerifiedStructuredRun {
    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.state.continuation.run_id
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        &self.state.continuation.admission
    }

    /// Returns the exact verified certified program.
    pub fn certified_program(&self) -> &VerifiedProgramData {
        self.program.as_ref()
    }

    /// Returns the fully expanded program selected by admission.
    pub fn expanded_program(&self) -> &ExpandedStructuredProgram {
        self.program.expanded()
    }

    /// Returns the sole callback-free cursor.
    pub const fn cursor(&self) -> &ProgramCursor {
        &self.state.derived.cursor
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        &self.state.derived.frontier
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        &self.semantic_head
    }

    /// Returns every currently live producer-bound lexical value.
    pub fn live_bindings(&self) -> impl ExactSizeIterator<Item = &LexicalValueRef> {
        self.state.derived.bindings.values()
    }

    /// Resolves one exact verified content-addressed history object.
    pub fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.state.continuation.objects.get(content_ref)
    }

    /// Returns every verified content-addressed history object in canonical reference order.
    pub fn objects(&self) -> impl ExactSizeIterator<Item = &HistoryObject> {
        self.state.continuation.objects.values()
    }

    /// Validates one candidate's exact object closure against this successor's
    /// callback-free fold state before backend dispatch.
    pub(super) fn validate_append_object_closure(
        &self,
        batch: &CommittedBatch,
    ) -> super::Result<()> {
        let actual = batch
            .objects
            .iter()
            .map(|object| object.content_ref.clone())
            .collect::<BTreeSet<_>>();
        let prior = batch
            .predecessor
            .as_ref()
            .map_or(0, |head| head.run_sequence);
        let prior_object_refs = self
            .objects_through(prior)
            .map(|object| object.content_ref.clone())
            .collect::<BTreeSet<_>>();
        validate_batch_object_closure(
            &batch.records,
            &actual,
            &prior_object_refs,
            &self.state.continuation,
            &self.state.derived,
        )
    }

    /// Returns verified objects first admitted no later than one physical append sequence.
    pub fn objects_through(&self, run_sequence: u64) -> impl Iterator<Item = &HistoryObject> {
        self.state
            .continuation
            .objects
            .iter()
            .filter(move |(content_ref, _)| {
                self.state
                    .continuation
                    .object_first_seen_sequence
                    .get(*content_ref)
                    .is_some_and(|sequence| *sequence <= run_sequence)
            })
            .map(|(_, object)| object)
    }

    /// Resolves one exact verified access authorization and its record identity.
    pub fn authorization(
        &self,
        access_attempt_id: &AccessAttemptId,
    ) -> Option<(&RecordRef, &ExternalAccessAuthorized)> {
        self.state
            .continuation
            .authorizations
            .get(access_attempt_id)
            .map(|authorization| (&authorization.record_ref, &authorization.record))
    }

    /// Resolves one exact verified access observation and its record identity.
    pub fn observation(
        &self,
        access_attempt_id: &AccessAttemptId,
    ) -> Option<(&RecordRef, &ExternalAccessObserved)> {
        self.state
            .continuation
            .observations
            .get(access_attempt_id)
            .map(|observation| (&observation.record_ref, &observation.record))
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        &self.state.continuation.records
    }

    /// Returns the verified record prefix through the exact semantic head record.
    ///
    /// A terminal transition and `RunClosed` can share one atomic append, so
    /// this cutoff is record-coordinate based rather than append-sequence based.
    pub fn semantic_records(&self) -> impl Iterator<Item = &AssignedRecord> {
        let cutoff = match &self.semantic_head {
            SemanticHead::Genesis { admission_ref, .. } => admission_ref,
            SemanticHead::Transition { transition_ref, .. } => transition_ref,
        };
        self.records().iter().take_while(move |record| {
            record.record_ref.run_sequence < cutoff.run_sequence
                || (record.record_ref.run_sequence == cutoff.run_sequence
                    && record.record_ref.ordinal <= cutoff.ordinal)
        })
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        &self.state.continuation.journal_heads
    }

    /// Returns every exact committed-batch envelope in append order.
    pub fn batches(&self) -> &[CommittedBatch] {
        &self.state.continuation.batches
    }

    /// Derives the distinct prior-run producers referenced by verified fact
    /// selections. The result is computed from folded objects rather than
    /// accepted from an export envelope.
    pub fn direct_source_run_ids(&self) -> super::Result<BTreeSet<RunId>> {
        let mut sources = BTreeSet::new();
        let consumer = self.run_id();
        let fact_response_contract =
            mfm_spec::structured::structured_value_contract_ref::<FactSelectionReadResponse>()
                .map_err(|_| invalid("fact response contract cannot be derived"))?;
        for assigned in self.records() {
            let RunRecord::ExternalAccessObserved(observation) = &assigned.record else {
                continue;
            };
            let ObservationOutcome::Returned { value } = &observation.outcome else {
                continue;
            };
            if value.contract_ref != fact_response_contract {
                continue;
            }
            let Some(object) = self.object(&value.value_ref) else {
                return Err(invalid("fact response object is absent"));
            };
            let returned = object
                .decode::<FactSelectionReadResponse>()
                .map_err(|_| invalid("fact response object is invalid"))?;
            let response = serde_json::from_str::<PriorRunFactSelectionResponse>(
                returned.canonical_response_json(),
            )
            .map_err(|_| invalid("fact selection response is invalid"))?;
            for query in &response.query_results {
                for selected in &query.selected {
                    let producer = &selected.producer_transition_ref.run_id;
                    insert_bounded_source(&mut sources, consumer, producer)?;
                }
            }
        }
        Ok(sources)
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    pub const fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.state.continuation.closed_outcome_ref.as_ref()
    }
}

fn insert_bounded_source(
    sources: &mut BTreeSet<RunId>,
    consumer: &RunId,
    producer: &RunId,
) -> super::Result<()> {
    if producer == consumer || sources.contains(producer) {
        return Ok(());
    }
    if sources.len() >= super::MAX_PORTABLE_SOURCE_RUNS {
        return Err(invalid("fact source count exceeds export bound"));
    }
    sources.insert(producer.clone());
    Ok(())
}

#[cfg(test)]
mod source_bound_tests {
    use std::collections::BTreeSet;

    use mfm_ids::RunId;

    use super::super::MAX_PORTABLE_SOURCE_RUNS;
    use super::insert_bounded_source;

    fn run(index: usize) -> RunId {
        RunId::parse(format!("run:sha256-jcs-v1:{index:064x}")).expect("run id")
    }

    #[test]
    fn distinct_source_bound_rejects_before_insert() {
        let consumer = run(0);
        let mut sources = BTreeSet::new();
        for index in 1..=MAX_PORTABLE_SOURCE_RUNS {
            insert_bounded_source(&mut sources, &consumer, &run(index)).expect("within bound");
        }
        assert_eq!(sources.len(), MAX_PORTABLE_SOURCE_RUNS);
        assert!(
            insert_bounded_source(&mut sources, &consumer, &run(MAX_PORTABLE_SOURCE_RUNS + 1))
                .is_err()
        );
        assert_eq!(sources.len(), MAX_PORTABLE_SOURCE_RUNS);
    }
}

/// Read-only offline entry to the sole structured-history fold.
///
/// Accepts already-parsed exact batch envelopes and concrete callback-free trust
/// material. It cannot append, expose a writer, or perform ambient IO.
pub fn verify_offline_recorded_history(
    raw: RawRunHistory,
    program_verifier: &dyn ProgramVerifier,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<VerifiedStructuredRun> {
    verify_recorded_history(raw, program_verifier, physical_binding_verifier)
}

#[cfg(test)]
pub(super) fn verified_runs_equivalent(
    left: &VerifiedStructuredRun,
    right: &VerifiedStructuredRun,
) -> bool {
    left.state == right.state
        && left.program == right.program
        && left.journal_head == right.journal_head
        && left.semantic_head == right.semantic_head
}

/// Callback-free verifies a complete raw structured history.
pub(super) fn verify_recorded_history(
    raw: RawRunHistory,
    program_verifier: &dyn ProgramVerifier,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<VerifiedStructuredRun> {
    let (machine, journal_head, derived) =
        fold_recorded_history(raw, program_verifier, physical_binding_verifier)?;
    machine.finish(journal_head, derived)
}

fn fold_recorded_history(
    raw: RawRunHistory,
    program_verifier: &dyn ProgramVerifier,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<(FoldMachine, JournalHead, DerivedProgram)> {
    if raw.batches.is_empty() {
        return Err(invalid("run prefix is empty"));
    }
    let mut previous_head = None;
    let mut store_identity = None;
    for batch in &raw.batches {
        verify_batch_envelope(
            &raw.run_id,
            batch,
            previous_head.as_ref(),
            store_identity.as_ref(),
        )?;
        store_identity.get_or_insert_with(|| (batch.store_scope_id.clone(), batch.store_epoch));
        previous_head = Some(batch.head.clone());
    }

    let RawRunHistory { run_id, batches } = raw;
    let mut batches = batches.into_iter();
    let first_batch = batches
        .next()
        .ok_or_else(|| invalid("run prefix is empty"))?;
    let Some(first_assigned) = first_batch.records.first() else {
        return Err(invalid("admission batch has no record"));
    };
    let RunRecord::RunAdmitted(admission) = &first_assigned.record else {
        return Err(invalid("first record is not RunAdmitted"));
    };
    if admission.run_id != run_id {
        return Err(invalid("admission run identity differs from its envelope"));
    }

    let mut machine =
        FoldMachine::new(run_id, admission.clone(), first_assigned.record_ref.clone());
    let first_object_refs = first_batch
        .objects
        .iter()
        .map(|object| object.content_ref.clone())
        .collect::<BTreeSet<_>>();
    machine.admit_objects(first_batch.objects.clone())?;
    let root_object = machine
        .objects
        .get(&admission.certified_program_root_ref)
        .ok_or_else(|| invalid("certified program root object is missing"))?;
    if root_object.object_type.as_str() != CERTIFIED_ROOT_OBJECT_TYPE {
        return Err(invalid("certified program root object type differs"));
    }
    let root: mfm_spec::structured::CertifiedProgramRoot = root_object
        .decode()
        .map_err(|_| invalid("certified program root object cannot be strictly decoded"))?;
    if root
        .content_ref()
        .map_err(|_| StructuredStoreError::Certification)?
        != admission.certified_program_ref
    {
        return Err(invalid(
            "certified program root reference differs from admission",
        ));
    }
    let authored_object = machine
        .objects
        .get(&root.components.authored_program_ref)
        .ok_or_else(|| invalid("certified authored program object is missing"))?;
    let authored: CanonicalJsonValue = authored_object
        .decode()
        .map_err(|_| invalid("certified authored program object cannot be decoded"))?;
    let program = program_verifier
        .verify(&admission.entry_point_operation_id, &root, &authored)
        .map_err(|_| StructuredStoreError::Certification)?;
    if program.document().root != root
        || program.expanded().operation_id != admission.entry_point_operation_id
    {
        return Err(invalid(
            "qualified program verifier returned different data",
        ));
    }
    validate_admission_audit_refs(admission, program.document())?;
    require_component_object_closure(&machine.objects, program.document())?;
    machine.initialize_program(program)?;
    let mut derived = machine.verify_admission_batch(&first_batch)?;
    validate_batch_object_closure(
        &first_batch.records,
        &first_object_refs,
        &BTreeSet::new(),
        &machine,
        &derived,
    )?;
    machine.record_batch(&first_batch);

    for batch in batches {
        let prior_object_refs = machine.objects.keys().cloned().collect::<BTreeSet<_>>();
        let batch_object_refs = batch
            .objects
            .iter()
            .map(|object| object.content_ref.clone())
            .collect::<BTreeSet<_>>();
        machine.admit_objects(batch.objects.clone())?;
        derived = machine.apply_batch(
            &batch,
            derived,
            PhysicalBindingVerificationMode::RetainedHistory,
            physical_binding_verifier,
        )?;
        validate_batch_object_closure(
            &batch.records,
            &batch_object_refs,
            &prior_object_refs,
            &machine,
            &derived,
        )?;
        machine.record_batch(&batch);
    }
    let journal_head = previous_head.ok_or_else(|| invalid("run prefix has no journal head"))?;
    Ok((machine, journal_head, derived))
}

fn validate_batch_object_closure(
    records: &[AssignedRecord],
    actual: &BTreeSet<ContentRef>,
    prior_object_refs: &BTreeSet<ContentRef>,
    machine: &FoldMachine,
    derived: &DerivedProgram,
) -> super::Result<()> {
    let mut required = derived.required_object_refs.clone();
    for assigned in records {
        match &assigned.record {
            RunRecord::RunAdmitted(admission) => {
                required.insert(admission.certified_program_root_ref.clone());
                required.extend(
                    machine
                        .program()?
                        .document()
                        .component_closure
                        .iter()
                        .map(|component| component.content_ref.clone()),
                );
                required.extend(
                    admission
                        .initial_bindings
                        .iter()
                        .map(|binding| binding.value.value_ref.clone()),
                );
                required.extend([
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
            }
            RunRecord::StateTransitionCommitted(transition) => {
                required.insert(transition.outcome_ref.clone());
                match &transition.outcome {
                    StateOutcomeRef::Success(value) | StateOutcomeRef::Failure(value) => {
                        required.insert(value.value.value_ref.clone());
                    }
                }
                for fact in &transition.facts {
                    required.extend([
                        fact.subject.value_ref.clone(),
                        fact.response.value_ref.clone(),
                        fact.claim_ref.clone(),
                    ]);
                }
            }
            RunRecord::ExternalAccessAuthorized(authorization) => {
                required.extend([
                    authorization.request.value_ref.clone(),
                    authorization.physical_binding_ref.clone(),
                ]);
            }
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                ObservationOutcome::Returned { value }
                | ObservationOutcome::SafeFailure { value } => {
                    required.insert(value.value_ref.clone());
                }
                ObservationOutcome::SupersededBeforeEntry {
                    public_lineage_head_ref,
                    evidence_ref,
                } => {
                    required.extend([public_lineage_head_ref.clone(), evidence_ref.clone()]);
                }
                ObservationOutcome::EntryUnknown { .. }
                | ObservationOutcome::IntegrityFault { .. } => {}
            },
            RunRecord::RunClosed(closed) => {
                required.insert(closed.outcome_ref.clone());
            }
        }
    }
    required.retain(|content_ref| !prior_object_refs.contains(content_ref));
    if actual != &required {
        return Err(invalid(
            "batch object set differs from its exact record closure",
        ));
    }
    Ok(())
}

pub(super) struct PreparedSuccessor {
    pub(super) batch: CommittedBatch,
    pub(super) verified: VerifiedStructuredRun,
}

fn prepare_validated_genesis(
    identity: &StructuredStoreIdentity,
    candidate: CommitCandidate,
    program_verifier: &dyn ProgramVerifier,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<PreparedSuccessor> {
    if candidate.expected_head.is_some() {
        return Err(StructuredStoreError::StaleHead);
    }
    let committed = assign_candidate(identity, candidate)?;
    let preview = RawRunHistory {
        run_id: committed.records[0].record_ref.run_id.clone(),
        batches: vec![committed.clone()],
    };
    let verified = verify_recorded_history(preview, program_verifier, physical_binding_verifier)?;
    Ok(PreparedSuccessor {
        batch: committed,
        verified,
    })
}

fn extend_verified_candidate(
    verified: VerifiedStructuredRun,
    identity: &StructuredStoreIdentity,
    candidate: CommitCandidate,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<PreparedSuccessor> {
    if candidate.run_id != *verified.run_id() {
        return Err(invalid("candidate run differs from verified history"));
    }
    if candidate.expected_head.as_ref() != Some(verified.journal_head()) {
        return Err(StructuredStoreError::StaleHead);
    }
    if verified.admission().store_scope_id != identity.store_scope_id
        || verified.admission().store_epoch != identity.store_epoch
    {
        return Err(invalid(
            "candidate writer lineage differs from verified history",
        ));
    }
    let VerifiedStructuredRun {
        state,
        journal_head,
        ..
    } = verified;
    let VerifiedFoldState {
        mut continuation,
        derived: before,
    } = *state;
    let prior_object_refs = continuation
        .objects
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let committed = assign_candidate(identity, candidate)?;
    verify_batch_envelope(
        &continuation.run_id,
        &committed,
        Some(&journal_head),
        Some(&(identity.store_scope_id.clone(), identity.store_epoch)),
    )?;
    let batch_object_refs = committed
        .objects
        .iter()
        .map(|object| object.content_ref.clone())
        .collect::<BTreeSet<_>>();
    continuation.admit_objects(committed.objects.clone())?;
    let derived = continuation.apply_batch(
        &committed,
        before,
        PhysicalBindingVerificationMode::CurrentCandidate,
        physical_binding_verifier,
    )?;
    validate_batch_object_closure(
        &committed.records,
        &batch_object_refs,
        &prior_object_refs,
        &continuation,
        &derived,
    )?;
    continuation.record_batch(&committed);
    let successor = continuation.finish(committed.head.clone(), derived)?;
    Ok(PreparedSuccessor {
        batch: committed,
        verified: successor,
    })
}

pub(super) fn prepare_admission(
    identity: &StructuredStoreIdentity,
    request: StructuredAdmissionRequest,
    program_verifier: &dyn ProgramVerifier,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<PreparedSuccessor> {
    let authored = request
        .certified_program
        .component_closure
        .iter()
        .find(|object| {
            object.content_ref
                == request
                    .certified_program
                    .root
                    .components
                    .authored_program_ref
        })
        .map(|object| &object.value)
        .ok_or_else(|| invalid("admission certified closure has no authored program"))?;
    let program = program_verifier
        .verify(
            &request.entry_point_operation_id,
            &request.certified_program.root,
            authored,
        )
        .map_err(|_| StructuredStoreError::Certification)?;
    if program.document() != &request.certified_program
        || program.expanded().operation_id != request.entry_point_operation_id
        || program.expanded().input_roots.len() != request.initial_values.len()
    {
        return Err(invalid("admission program or input roots differ"));
    }

    let document = request.certified_program;
    let certified_program_ref = document
        .content_ref()
        .map_err(|_| StructuredStoreError::Certification)?;
    let root_canonical = document
        .root
        .canonical_json()
        .map_err(|_| StructuredStoreError::Certification)?;
    let root_object = HistoryObject::new(
        StableId::new(CERTIFIED_ROOT_OBJECT_TYPE)
            .map_err(|_| invalid("certified root object type is invalid"))?,
        framework_schema_id("mfm.structured-certified-program-root")?,
        root_canonical.as_str(),
    )
    .map_err(|_| invalid("certified root object cannot be constructed"))?;
    let mut objects = vec![root_object.clone()];
    for component in &document.component_closure {
        let canonical = component
            .value
            .canonical_json()
            .map_err(|_| StructuredStoreError::Certification)?;
        let object = HistoryObject::new(
            component.object_type.clone(),
            component.content_ref.schema_id().clone(),
            canonical.as_str(),
        )
        .map_err(|_| StructuredStoreError::Certification)?;
        if object.content_ref != component.content_ref {
            return Err(invalid("certified component object identity differs"));
        }
        objects.push(object);
    }

    let material = request.material;
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

    let mut initial_bindings = Vec::with_capacity(request.initial_values.len());
    for (slot, value) in program
        .expanded()
        .input_roots
        .iter()
        .zip(&request.initial_values)
    {
        let (object, typed) = proposed_typed_object(value, &slot.contract_ref, &program)?;
        objects.push(object);
        initial_bindings.push(LexicalValueRef {
            slot_ref: slot
                .content_ref()
                .map_err(|_| invalid("admission root slot reference cannot be derived"))?,
            value: typed,
            structural_origin: None,
        });
    }

    let components = &document.root.components;
    let mut admission = RunAdmitted {
        store_scope_id: identity.store_scope_id.clone(),
        store_epoch: identity.store_epoch,
        run_id: request.run_id.clone(),
        tenant_scope_id: request.tenant_scope_id,
        invocation_identity: request.invocation_identity,
        entry_point_operation_id: request.entry_point_operation_id,
        certified_program_ref: certified_program_ref.clone(),
        certified_program_root_ref: root_object.content_ref,
        qualified_entry_point_admission_policy_ref: components
            .qualified_entry_point_admission_policy_ref
            .clone(),
        audit_refs: CertifiedProgramAuditRefs {
            authored_program_ref: components.authored_program_ref.clone(),
            expanded_program_ref: components.expanded_program_ref.clone(),
            expansion_profile_ref: components.expansion_profile_ref.clone(),
            expansion_proof_ref: components.expansion_proof_ref.clone(),
            policy_coverage_proof_ref: components.policy_coverage_proof_ref.clone(),
            component_manifest_ref: components
                .state_capability_adapter_signer_resource_manifest_closure_ref
                .clone(),
            implementation_manifest_ref: components
                .secret_free_implementation_manifest_closure_ref
                .clone(),
        },
        admission_material_refs,
        initial_bindings,
        genesis_semantic_state_digest: placeholder_semantic_digest(),
    };
    let mut machine = FoldMachine::new(
        request.run_id.clone(),
        admission.clone(),
        placeholder_record_ref(&request.run_id),
    );
    canonicalize_objects(&mut objects)?;
    machine.admit_objects(objects.clone())?;
    machine.initialize_program(program)?;
    let derived = machine.derive(true)?;
    admission.genesis_semantic_state_digest = semantic_state_digest(
        &certified_program_ref,
        &machine.transitions,
        &derived.bindings,
    )?;
    objects.extend(derived.generated_objects);
    canonicalize_objects(&mut objects)?;
    let mut records = vec![RunRecord::RunAdmitted(admission)];
    if let ProgramCursor::Closed { outcome_ref } = derived.cursor {
        records.push(RunRecord::RunClosed(RunClosed { outcome_ref }));
    }
    prepare_validated_genesis(
        identity,
        CommitCandidate {
            run_id: request.run_id,
            expected_head: None,
            append_request_id: request.append_request_id,
            tenant_fact_coordinate: TenantFactCoordinate::None,
            records,
            objects,
        },
        program_verifier,
        physical_binding_verifier,
    )
}

pub(super) fn prepare_state_transition(
    verified: VerifiedStructuredRun,
    identity: &StructuredStoreIdentity,
    proposal: &StateTransitionProposal,
    tenant_fact_coordinate: TenantFactCoordinate,
) -> super::Result<PreparedSuccessor> {
    let VerifiedStructuredRun {
        state,
        journal_head,
        ..
    } = verified;
    let VerifiedFoldState {
        mut continuation,
        derived: before,
    } = *state;
    let machine = &mut continuation;
    let actionable = minimum_action(&before.frontier)?.clone();
    let state = find_state(machine.program()?.expanded(), &actionable.occurrence_id)?.clone();
    let prior_object_refs = machine.objects.keys().cloned().collect::<BTreeSet<_>>();
    let [input_slot] = state.inputs.as_slice() else {
        return Err(invalid("current state does not have one exact input"));
    };
    let input_ref = input_slot
        .content_ref()
        .map_err(|_| invalid("current input slot reference cannot be derived"))?;
    let input = before
        .bindings
        .get(&input_ref)
        .cloned()
        .ok_or_else(|| invalid("current state input binding is absent"))?;

    let mut objects = Vec::new();
    let (outcome, facts) = match proposal.value() {
        ProposedTransitionValue::Success { value, facts } => {
            let (object, typed) =
                proposed_typed_object(value, &state.output_slot.contract_ref, machine.program()?)?;
            objects.push(object);
            let binding = LexicalValueRef {
                slot_ref: state
                    .output_slot
                    .content_ref()
                    .map_err(|_| invalid("state output slot reference cannot be derived"))?,
                value: typed,
                structural_origin: None,
            };
            let facts = prepare_fact_proposals(facts, &state, machine.program()?, &mut objects)?;
            (StateOutcomeRef::Success(binding), facts)
        }
        ProposedTransitionValue::Failure(value) => {
            let CertifiedFailureBoundary::Typed { source_slot, .. } = &state.failure_boundary
            else {
                return Err(invalid("Never state callback proposed a failure"));
            };
            let (object, typed) =
                proposed_typed_object(value, &source_slot.contract_ref, machine.program()?)?;
            objects.push(object);
            (
                StateOutcomeRef::Failure(LexicalValueRef {
                    slot_ref: source_slot
                        .content_ref()
                        .map_err(|_| invalid("state failure slot reference cannot be derived"))?,
                    value: typed,
                    structural_origin: None,
                }),
                Vec::new(),
            )
        }
    };
    let outcome_json = match &outcome {
        StateOutcomeRef::Success(value) => serde_json::json!({ "Success": value }),
        StateOutcomeRef::Failure(value) => serde_json::json!({ "Failure": value }),
    };
    let outcome_object = framework_object(
        STATE_OUTCOME_OBJECT_TYPE,
        "mfm.structured-state-outcome",
        &outcome_json,
    )?;
    let outcome_ref = outcome_object.content_ref.clone();
    objects.push(outcome_object);
    canonicalize_objects(&mut objects)?;
    filter_existing_objects(&mut objects, &machine.objects)?;
    machine.admit_objects(objects.clone())?;

    let consumed_observation_ref = match &actionable.leaf {
        StateLeaf::ObservedForSettlement {
            observation_ref, ..
        } => Some(observation_ref.clone()),
        StateLeaf::Ready => None,
        _ => return Err(invalid("current state leaf cannot commit a transition")),
    };
    let before_digest = machine.semantic_head()?.semantic_state_digest().clone();
    let mut transition = StateTransitionCommitted {
        occurrence_id: actionable.occurrence_id.clone(),
        occurrence_path_ref: actionable
            .occurrence_path
            .content_ref()
            .map_err(|_| invalid("transition occurrence path cannot be derived"))?,
        semantic_call_id: state.semantic_call_id,
        input,
        consumed_observation_ref,
        outcome_ref,
        outcome,
        facts,
        before_semantic_state_digest: before_digest,
        after_semantic_state_digest: placeholder_semantic_digest(),
    };
    validate_transition(
        &transition,
        &actionable,
        machine.semantic_head()?,
        machine.program()?,
        &machine.objects,
        &before.bindings,
        &machine.authorizations,
        &machine.observations,
    )?;
    machine.transitions.insert(
        transition.occurrence_id.clone(),
        RecordedTransition {
            record: transition.clone(),
        },
    );
    let mut after = machine.derive(true)?;
    transition.after_semantic_state_digest = semantic_state_digest(
        machine.program_ref()?,
        &machine.transitions,
        &after.bindings,
    )?;
    if let Some(recorded) = machine.transitions.get_mut(&transition.occurrence_id) {
        recorded.record = transition.clone();
    }
    let mut generated_objects = std::mem::take(&mut after.generated_objects);
    canonicalize_objects(&mut generated_objects)?;
    filter_existing_objects(&mut generated_objects, &machine.objects)?;
    machine.admit_objects(generated_objects.clone())?;
    objects.extend(generated_objects);
    canonicalize_objects(&mut objects)?;
    let mut records = vec![RunRecord::StateTransitionCommitted(transition)];
    if let ProgramCursor::Closed { outcome_ref } = &after.cursor {
        records.push(RunRecord::RunClosed(RunClosed {
            outcome_ref: outcome_ref.clone(),
        }));
    }
    validate_transition_fact_coordinate(
        identity,
        &machine.admission.tenant_scope_id,
        &records[0],
        &tenant_fact_coordinate,
    )?;
    let committed = assign_candidate(
        identity,
        CommitCandidate {
            run_id: machine.run_id.clone(),
            expected_head: Some(journal_head.clone()),
            append_request_id: proposal.append_request_id().clone(),
            tenant_fact_coordinate,
            records,
            objects,
        },
    )?;
    verify_batch_envelope(
        &machine.run_id,
        &committed,
        Some(&journal_head),
        Some(&(identity.store_scope_id.clone(), identity.store_epoch)),
    )?;
    let [transition, closure @ ..] = committed.records.as_slice() else {
        return Err(invalid("transition candidate has no assigned record"));
    };
    let RunRecord::StateTransitionCommitted(record) = &transition.record else {
        return Err(invalid("transition candidate changed record family"));
    };
    if let Some(recorded) = machine.transitions.get_mut(&record.occurrence_id) {
        recorded.record = record.clone();
    }
    machine.semantic_head = Some(SemanticHead::Transition {
        transition_ref: transition.record_ref.clone(),
        semantic_state_digest: record.after_semantic_state_digest.clone(),
    });
    machine.insert_logical_record(transition)?;
    machine.verify_required_closure(closure.first(), &after)?;
    let batch_object_refs = committed
        .objects
        .iter()
        .map(|object| object.content_ref.clone())
        .collect::<BTreeSet<_>>();
    validate_batch_object_closure(
        &committed.records,
        &batch_object_refs,
        &prior_object_refs,
        machine,
        &after,
    )?;
    continuation.record_batch(&committed);
    let successor = continuation.finish(committed.head.clone(), after)?;
    Ok(PreparedSuccessor {
        batch: committed,
        verified: successor,
    })
}

fn validate_transition_fact_coordinate(
    identity: &StructuredStoreIdentity,
    tenant_scope_id: &mfm_ids::TenantScopeId,
    record: &RunRecord,
    coordinate: &TenantFactCoordinate,
) -> super::Result<()> {
    let RunRecord::StateTransitionCommitted(transition) = record else {
        return Err(invalid(
            "fact publication coordinate does not belong to a transition",
        ));
    };
    match (transition.facts.is_empty(), coordinate) {
        (true, TenantFactCoordinate::None) => Ok(()),
        (false, TenantFactCoordinate::FactPublication { frontier })
            if frontier.store_scope_id == identity.store_scope_id
                && frontier.store_epoch == identity.store_epoch
                && frontier.tenant_scope_id == *tenant_scope_id
                && frontier.fact_order > 0 =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "transition fact coordinate differs from its non-empty publication",
        )),
    }
}

pub(super) fn prepare_authorization(
    verified: VerifiedStructuredRun,
    identity: &StructuredStoreIdentity,
    proposal: &AccessAuthorizationProposal,
    tenant_fact_coordinate: TenantFactCoordinate,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<PreparedSuccessor> {
    let machine = &verified.state.continuation;
    let derived = &verified.state.derived;
    let actionable = minimum_action(&derived.frontier)?;
    let state = find_state(machine.program()?.expanded(), &actionable.occurrence_id)?;
    let (access_kind, capability_contract_ref) = match &state.contract.execution {
        mfm_spec::structured::StructuredStateExecutionContract::Pure => {
            return Err(invalid("Pure state cannot authorize access"));
        }
        mfm_spec::structured::StructuredStateExecutionContract::Read {
            capability_contract_ref,
        } => (AccessKind::Read, capability_contract_ref),
        mfm_spec::structured::StructuredStateExecutionContract::Effect {
            capability_contract_ref,
        } => (AccessKind::Effect, capability_contract_ref),
    };
    let attempt_ordinal = match &actionable.leaf {
        StateLeaf::Ready => 0,
        StateLeaf::Refreshable {
            next_attempt_ordinal,
            ..
        } if access_kind == AccessKind::Effect => *next_attempt_ordinal,
        _ => return Err(invalid("current state leaf cannot authorize access")),
    };
    let capability: StructuredLiveComponentContract =
        decode_component(machine.program()?.document(), capability_contract_ref)?;
    let [adapter_dependency] = capability.dependencies.as_slice() else {
        return Err(invalid(
            "capability does not have one exact adapter dependency",
        ));
    };
    if adapter_dependency.component_kind != StructuredComponentKind::Adapter {
        return Err(invalid("capability dependency is not an adapter"));
    }
    let implementation_manifest: SecretFreeImplementationManifest = decode_component(
        machine.program()?.document(),
        &machine
            .program()?
            .document()
            .root
            .components
            .secret_free_implementation_manifest_closure_ref,
    )?;
    let capability_implementation_ref = implementation_manifest
        .entries
        .iter()
        .find(|entry| {
            entry.component_kind == StructuredComponentKind::Capability
                && entry.semantic_contract_ref == *capability_contract_ref
        })
        .map(|entry| entry.implementation_contract_ref.clone())
        .ok_or_else(|| invalid("capability implementation is absent from manifest"))?;
    let adapter_implementation_ref = implementation_manifest
        .entries
        .iter()
        .find(|entry| {
            entry.component_kind == StructuredComponentKind::Adapter
                && entry.semantic_contract_ref == adapter_dependency.contract_ref
        })
        .map(|entry| entry.implementation_contract_ref.clone())
        .ok_or_else(|| invalid("adapter implementation is absent from manifest"))?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(|| invalid("authorization capability protocol is absent"))?;
    if *proposal.state_input_ref() != actionable.input {
        return Err(invalid(
            "authorization state input differs from the current producer binding",
        ));
    }
    let (request_object, request_ref) = proposed_typed_object(
        proposal.request(),
        protocol.request_contract_ref(),
        machine.program()?,
    )?;
    let request_digest = RequestDigest::from_digest(sha256_digest_bytes(
        request_object.canonical_json.as_bytes(),
    ));
    let stable_resource_lineage_contract_ref =
        actionable.stable_resource_lineage_contract_ref.clone();
    let minimum_lineage_head_ref = match &actionable.leaf {
        StateLeaf::Refreshable {
            public_lineage_head_ref,
            ..
        } => Some(public_lineage_head_ref.clone()),
        _ => None,
    };
    let occurrence_path_ref = actionable
        .occurrence_path
        .content_ref()
        .map_err(|_| invalid("authorization path reference cannot be derived"))?;
    let semantic_head = machine.semantic_head()?.clone();
    let mut authorization = ExternalAccessAuthorized {
        access_attempt_id: AccessAttemptId::from_digest(sha256_digest_bytes(
            b"mfm.structured-placeholder-access-attempt.v1",
        )),
        attempt_ordinal,
        occurrence_id: actionable.occurrence_id.clone(),
        occurrence_path_ref,
        semantic_call_id: state.semantic_call_id.clone(),
        state_input_ref: proposal.state_input_ref().clone(),
        access_kind,
        semantic_head,
        store_scope_id: verified.admission().store_scope_id.clone(),
        store_epoch: verified.admission().store_epoch,
        tenant_scope_id: verified.admission().tenant_scope_id.clone(),
        admitted_routing_policy_ref: verified
            .admission()
            .admission_material_refs
            .routing_policy_ref
            .clone(),
        minimum_lineage_head_ref,
        capability_contract_ref: capability_contract_ref.clone(),
        capability_implementation_ref,
        adapter_contract_ref: adapter_dependency.contract_ref.clone(),
        adapter_implementation_ref,
        request: request_ref,
        request_digest,
        physical_binding_ref: proposal.physical_binding_certificate().content_ref.clone(),
        stable_resource_lineage_contract_ref,
    };
    authorization.access_attempt_id = derive_access_attempt_id(&AccessAttemptPreimage {
        run_id: verified.run_id(),
        occurrence_id: &authorization.occurrence_id,
        occurrence_path_ref: &authorization.occurrence_path_ref,
        semantic_call_id: &authorization.semantic_call_id,
        state_input_ref: &authorization.state_input_ref,
        attempt_ordinal: authorization.attempt_ordinal,
        access_kind: authorization.access_kind,
        semantic_head: &authorization.semantic_head,
        capability_contract_ref: &authorization.capability_contract_ref,
        capability_implementation_ref: &authorization.capability_implementation_ref,
        adapter_contract_ref: &authorization.adapter_contract_ref,
        adapter_implementation_ref: &authorization.adapter_implementation_ref,
        request: &authorization.request,
        request_digest: &authorization.request_digest,
        physical_binding_ref: &authorization.physical_binding_ref,
        stable_resource_lineage_contract_ref: &authorization.stable_resource_lineage_contract_ref,
    })
    .map_err(|_| invalid("authorization access identity cannot be derived"))?;
    let mut objects = vec![
        request_object,
        proposal.physical_binding_certificate().clone(),
    ];
    filter_existing_objects(&mut objects, &machine.objects)?;
    let run_id = verified.run_id().clone();
    let expected_head = verified.journal_head().clone();
    extend_verified_candidate(
        verified,
        identity,
        CommitCandidate {
            run_id,
            expected_head: Some(expected_head),
            append_request_id: proposal.append_request_id().clone(),
            tenant_fact_coordinate,
            records: vec![RunRecord::ExternalAccessAuthorized(authorization)],
            objects,
        },
        physical_binding_verifier,
    )
}

pub(super) fn authorization_requires_fact_selection_barrier(
    verified: &VerifiedStructuredRun,
) -> super::Result<bool> {
    let actionable = minimum_action(&verified.state.derived.frontier)?;
    let Some(capability_contract_ref) = actionable.capability_contract_ref.as_ref() else {
        return Ok(false);
    };
    let expected = prior_run_fact_selection_capability_contract()
        .and_then(|contract| contract.content_ref())
        .map_err(|_| invalid("prior-run fact selection contract cannot be derived"))?;
    Ok(capability_contract_ref == &expected)
}

fn validate_authorization_fact_coordinate(
    identity: &StructuredStoreIdentity,
    tenant_scope_id: &mfm_ids::TenantScopeId,
    is_prior_run_fact_selection: bool,
    coordinate: &TenantFactCoordinate,
) -> super::Result<()> {
    match (is_prior_run_fact_selection, coordinate) {
        (false, TenantFactCoordinate::None) => Ok(()),
        (true, TenantFactCoordinate::FactSelectionBarrier { frontier })
            if frontier.store_scope_id == identity.store_scope_id
                && frontier.store_epoch == identity.store_epoch
                && frontier.tenant_scope_id == *tenant_scope_id =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "authorization fact coordinate differs from exact scanner recognition",
        )),
    }
}

pub(super) enum PreparedObservation {
    ExistingSame(Box<VerifiedStructuredRun>),
    Append(Box<PreparedSuccessor>),
}

pub(super) fn qualify_observation(
    verified: &VerifiedStructuredRun,
    authorization_ref: &RecordRef,
    outcome: &ProposedObservationOutcome,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<ObservationQualification> {
    let machine = &verified.state.continuation;
    let authorization = machine
        .authorizations
        .values()
        .find(|authorization| &authorization.record_ref == authorization_ref)
        .ok_or_else(|| invalid("observation authorization reference is absent"))?;
    let (observation, objects) =
        proposed_observation(machine, authorization, authorization_ref.clone(), outcome)?;
    if let Some(existing) = machine
        .observations
        .get(&authorization.record.access_attempt_id)
    {
        return if existing.record == observation {
            Ok(ObservationQualification::ExistingSame)
        } else {
            Err(invalid(
                "existing observation content differs from pending completion",
            ))
        };
    }
    let complete_objects = HistoryObjectOverlay {
        base: &machine.objects,
        added: &objects,
    };
    if matches!(
        observation.outcome,
        ObservationOutcome::SupersededBeforeEntry { .. }
    ) && validate_proposed_supersession_physical(
        &observation,
        authorization,
        machine.program()?,
        &complete_objects,
        PhysicalBindingVerificationMode::CurrentCandidate,
        physical_binding_verifier,
    )
    .is_err()
    {
        return Ok(ObservationQualification::InvalidSupersessionEvidence);
    }
    validate_observation(
        &observation,
        authorization,
        machine.program()?,
        &complete_objects,
        PhysicalBindingVerificationMode::CurrentCandidate,
        physical_binding_verifier,
    )?;
    Ok(ObservationQualification::Ready)
}

pub(super) fn prepare_observation(
    verified: VerifiedStructuredRun,
    identity: &StructuredStoreIdentity,
    proposal: &AccessObservationProposal,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<PreparedObservation> {
    let (observation, objects) = {
        let machine = &verified.state.continuation;
        let authorization = machine
            .authorizations
            .values()
            .find(|authorization| authorization.record_ref == *proposal.authorization_ref())
            .ok_or_else(|| invalid("observation authorization reference is absent"))?;
        let (observation, objects) = proposed_observation(
            machine,
            authorization,
            proposal.authorization_ref().clone(),
            proposal.outcome(),
        )?;
        if let Some(existing) = machine
            .observations
            .get(&authorization.record.access_attempt_id)
        {
            return if existing.record == observation {
                Ok(PreparedObservation::ExistingSame(Box::new(verified)))
            } else {
                Err(invalid(
                    "existing observation content differs from pending completion",
                ))
            };
        }
        (observation, objects)
    };
    let run_id = verified.run_id().clone();
    let expected_head = verified.journal_head().clone();
    extend_verified_candidate(
        verified,
        identity,
        CommitCandidate {
            run_id,
            expected_head: Some(expected_head),
            append_request_id: proposal.append_request_id().clone(),
            tenant_fact_coordinate: TenantFactCoordinate::None,
            records: vec![RunRecord::ExternalAccessObserved(observation)],
            objects,
        },
        physical_binding_verifier,
    )
    .map(|successor| PreparedObservation::Append(Box::new(successor)))
}

fn proposed_observation(
    machine: &FoldMachine,
    authorization: &RecordedAuthorization,
    authorization_ref: RecordRef,
    proposed_outcome: &ProposedObservationOutcome,
) -> super::Result<(ExternalAccessObserved, Vec<HistoryObject>)> {
    let capability: StructuredLiveComponentContract = decode_component(
        machine.program()?.document(),
        &authorization.record.capability_contract_ref,
    )?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(|| invalid("observation capability protocol is absent"))?;
    let mut objects = Vec::new();
    let outcome = match proposed_outcome {
        ProposedObservationOutcome::Returned(value) => {
            let (object, value) =
                proposed_typed_object(value, protocol.returned_contract_ref(), machine.program()?)?;
            objects.push(object);
            ObservationOutcome::Returned { value }
        }
        ProposedObservationOutcome::SafeFailure(value) => {
            let (object, value) = proposed_typed_object(
                value,
                protocol.safe_failure_contract_ref(),
                machine.program()?,
            )?;
            objects.push(object);
            ObservationOutcome::SafeFailure { value }
        }
        ProposedObservationOutcome::SupersededBeforeEntry {
            public_lineage_head,
            evidence,
        } => {
            let StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        refresh_evidence_contract_ref,
                        ..
                    },
                ..
            } = protocol
            else {
                return Err(invalid("non-refreshable access proposed supersession"));
            };
            let (evidence_object, evidence_ref) =
                proposed_typed_object(evidence, refresh_evidence_contract_ref, machine.program()?)?;
            objects.extend([evidence_object, public_lineage_head.as_ref().clone()]);
            ObservationOutcome::SupersededBeforeEntry {
                public_lineage_head_ref: public_lineage_head.content_ref.clone(),
                evidence_ref: evidence_ref.value_ref,
            }
        }
        ProposedObservationOutcome::EntryUnknown { fault_code } => {
            ObservationOutcome::EntryUnknown {
                fault_code: fault_code.clone(),
            }
        }
        ProposedObservationOutcome::IntegrityFault { fault_code } => {
            ObservationOutcome::IntegrityFault {
                fault_code: fault_code.clone(),
            }
        }
    };
    filter_existing_objects(&mut objects, &machine.objects)?;
    let observation = ExternalAccessObserved {
        authorization_ref,
        access_attempt_id: authorization.record.access_attempt_id.clone(),
        outcome,
    };
    Ok((observation, objects))
}

pub(super) fn assign_candidate(
    identity: &StructuredStoreIdentity,
    candidate: CommitCandidate,
) -> super::Result<CommittedBatch> {
    if candidate.records.is_empty() || candidate.records.len() > MAX_BATCH_RECORDS {
        return Err(invalid("candidate record count exceeds its named bound"));
    }
    let run_sequence = match candidate.expected_head.as_ref() {
        Some(head) => head
            .run_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("candidate sequence overflowed"))?,
        None => 1,
    };
    let candidate_digest = domain_content_digest("mfm.structured-candidate.v1", &candidate)
        .map_err(|_| invalid("candidate digest cannot be derived"))?;
    let records = candidate
        .records
        .iter()
        .enumerate()
        .map(|(ordinal, record)| {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| invalid("candidate record ordinal exceeds u32"))?;
            let record_hash = derive_record_hash(&AssignedRecordHashPreimage {
                run_id: &candidate.run_id,
                run_sequence,
                ordinal,
                record,
            })
            .map_err(|_| invalid("candidate record hash cannot be derived"))?;
            Ok(AssignedRecord {
                record_ref: RecordRef {
                    run_id: candidate.run_id.clone(),
                    run_sequence,
                    ordinal,
                    record_hash,
                },
                record: record.clone(),
            })
        })
        .collect::<super::Result<Vec<_>>>()?;
    let commit_digest = derive_commit_digest(&AssignedCommitDigestPreimage {
        store_scope_id: &identity.store_scope_id,
        store_epoch: identity.store_epoch,
        predecessor: &candidate.expected_head,
        append_request_id: &candidate.append_request_id,
        tenant_fact_coordinate: &candidate.tenant_fact_coordinate,
        candidate_digest: &candidate_digest,
        record_refs: records.iter().map(|record| &record.record_ref).collect(),
        object_refs: candidate
            .objects
            .iter()
            .map(|object| &object.content_ref)
            .collect(),
    })
    .map_err(|_| invalid("candidate commit digest cannot be derived"))?;
    Ok(CommittedBatch {
        store_scope_id: identity.store_scope_id.clone(),
        store_epoch: identity.store_epoch,
        predecessor: candidate.expected_head,
        append_request_id: candidate.append_request_id,
        tenant_fact_coordinate: candidate.tenant_fact_coordinate,
        candidate_digest,
        records,
        objects: candidate.objects,
        head: JournalHead {
            run_sequence,
            commit_digest,
        },
    })
}

#[derive(Serialize)]
struct AssignedRecordHashPreimage<'a> {
    run_id: &'a RunId,
    run_sequence: u64,
    ordinal: u32,
    record: &'a RunRecord,
}

#[derive(Serialize)]
struct AssignedCommitDigestPreimage<'a> {
    store_scope_id: &'a mfm_ids::StoreScopeId,
    store_epoch: mfm_ids::StoreEpoch,
    predecessor: &'a Option<JournalHead>,
    append_request_id: &'a mfm_ids::AppendRequestId,
    tenant_fact_coordinate: &'a TenantFactCoordinate,
    candidate_digest: &'a ContentDigest,
    record_refs: Vec<&'a RecordRef>,
    object_refs: Vec<&'a ContentRef>,
}

#[derive(Serialize)]
struct BorrowedCommitCandidate<'a> {
    run_id: &'a RunId,
    expected_head: &'a Option<JournalHead>,
    append_request_id: &'a AppendRequestId,
    tenant_fact_coordinate: &'a TenantFactCoordinate,
    records: Vec<&'a RunRecord>,
    objects: Vec<&'a HistoryObject>,
}

pub(super) fn verify_batch_envelope(
    run_id: &RunId,
    batch: &CommittedBatch,
    previous_head: Option<&JournalHead>,
    store_identity: Option<&(mfm_ids::StoreScopeId, mfm_ids::StoreEpoch)>,
) -> super::Result<()> {
    if batch.records.is_empty() || batch.records.len() > MAX_BATCH_RECORDS {
        return Err(invalid("atomic batch record count exceeds its bound"));
    }
    if batch.predecessor.as_ref() != previous_head {
        return Err(invalid(
            "batch predecessor differs from the exact prior head",
        ));
    }
    if let Some((scope, epoch)) = store_identity {
        if &batch.store_scope_id != scope || batch.store_epoch != *epoch {
            return Err(invalid("batch store lineage changed inside one run"));
        }
    }
    let expected_sequence = match previous_head {
        Some(head) => head
            .run_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("batch sequence overflowed"))?,
        None => 1,
    };
    if batch.head.run_sequence != expected_sequence {
        return Err(invalid("batch sequence is not contiguous"));
    }
    if batch
        .objects
        .windows(2)
        .any(|pair| pair[0].content_ref >= pair[1].content_ref)
    {
        return Err(invalid(
            "batch objects are not unique canonical reference order",
        ));
    }
    for object in &batch.objects {
        object
            .validate()
            .map_err(|_| invalid("batch object bytes are not exact canonical content"))?;
    }
    let candidate = BorrowedCommitCandidate {
        run_id,
        expected_head: &batch.predecessor,
        append_request_id: &batch.append_request_id,
        tenant_fact_coordinate: &batch.tenant_fact_coordinate,
        records: batch
            .records
            .iter()
            .map(|assigned| &assigned.record)
            .collect(),
        objects: batch.objects.iter().collect(),
    };
    let candidate_digest = domain_content_digest("mfm.structured-candidate.v1", &candidate)
        .map_err(|_| invalid("batch candidate digest cannot be recomputed"))?;
    if candidate_digest != batch.candidate_digest {
        return Err(invalid("batch candidate digest differs"));
    }
    for (ordinal, assigned) in batch.records.iter().enumerate() {
        let ordinal =
            u32::try_from(ordinal).map_err(|_| invalid("batch record ordinal exceeds u32"))?;
        if assigned.record_ref.run_id != *run_id
            || assigned.record_ref.run_sequence != expected_sequence
            || assigned.record_ref.ordinal != ordinal
        {
            return Err(invalid("assigned record coordinate differs from its batch"));
        }
        let expected_hash = derive_record_hash(&AssignedRecordHashPreimage {
            run_id,
            run_sequence: expected_sequence,
            ordinal,
            record: &assigned.record,
        })
        .map_err(|_| invalid("record hash cannot be recomputed"))?;
        if expected_hash != assigned.record_ref.record_hash {
            return Err(invalid("assigned record hash differs"));
        }
    }
    let expected_commit = derive_commit_digest(&AssignedCommitDigestPreimage {
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
    .map_err(|_| invalid("commit digest cannot be recomputed"))?;
    if expected_commit != batch.head.commit_digest {
        return Err(invalid("assigned commit digest differs"));
    }
    Ok(())
}

pub(super) fn validate_resolved_batch(
    run_id: &RunId,
    append_request_id: &AppendRequestId,
    candidate_digest: &ContentDigest,
    identity: &StructuredStoreIdentity,
    batch: &CommittedBatch,
) -> super::Result<()> {
    if &batch.append_request_id != append_request_id || &batch.candidate_digest != candidate_digest
    {
        return Err(invalid(
            "resolved append differs from the requested physical identity",
        ));
    }
    verify_batch_envelope(
        run_id,
        batch,
        batch.predecessor.as_ref(),
        Some(&(identity.store_scope_id.clone(), identity.store_epoch)),
    )
}

fn validate_admission_audit_refs(
    admission: &RunAdmitted,
    document: &CertifiedProgramDocument,
) -> super::Result<()> {
    let components = &document.root.components;
    let audit = &admission.audit_refs;
    if admission.qualified_entry_point_admission_policy_ref
        != components.qualified_entry_point_admission_policy_ref
        || audit.authored_program_ref != components.authored_program_ref
        || audit.expanded_program_ref != components.expanded_program_ref
        || audit.expansion_profile_ref != components.expansion_profile_ref
        || audit.expansion_proof_ref != components.expansion_proof_ref
        || audit.policy_coverage_proof_ref != components.policy_coverage_proof_ref
        || audit.component_manifest_ref
            != components.state_capability_adapter_signer_resource_manifest_closure_ref
        || audit.implementation_manifest_ref
            != components.secret_free_implementation_manifest_closure_ref
    {
        return Err(invalid(
            "admission audit projections differ from certified root",
        ));
    }
    Ok(())
}

fn require_component_object_closure(
    objects: &BTreeMap<ContentRef, HistoryObject>,
    document: &CertifiedProgramDocument,
) -> super::Result<()> {
    for component in &document.component_closure {
        let object = objects
            .get(&component.content_ref)
            .ok_or_else(|| invalid("certified component closure object is absent"))?;
        // The batch envelope already revalidated the object's canonical bytes
        // against this exact content reference. The qualified program binds the
        // same component reference, so repeating canonical serialization here
        // would add no check and can duplicate multi-megabyte component work.
        if object.object_type != component.object_type {
            return Err(invalid("certified component closure object differs"));
        }
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct RecordedTransition {
    record: StateTransitionCommitted,
}

#[derive(Clone, PartialEq, Eq)]
struct RecordedAuthorization {
    record_ref: RecordRef,
    record: ExternalAccessAuthorized,
}

#[derive(Clone, PartialEq, Eq)]
struct RecordedObservation {
    record_ref: RecordRef,
    record: ExternalAccessObserved,
}

#[derive(PartialEq, Eq)]
struct FoldMachine {
    run_id: RunId,
    admission: RunAdmitted,
    admission_ref: RecordRef,
    program: Option<Arc<VerifiedProgramData>>,
    objects: BTreeMap<ContentRef, HistoryObject>,
    object_first_seen_sequence: BTreeMap<ContentRef, u64>,
    logical_keys: BTreeMap<RecordLogicalKey, JournalRecordHash>,
    initial_bindings: Vec<LexicalValueRef>,
    transitions: BTreeMap<OccurrenceId, RecordedTransition>,
    authorizations: BTreeMap<AccessAttemptId, RecordedAuthorization>,
    occurrence_attempts: BTreeMap<OccurrenceId, Vec<AccessAttemptId>>,
    observations: BTreeMap<AccessAttemptId, RecordedObservation>,
    semantic_head: Option<SemanticHead>,
    closed_outcome_ref: Option<ContentRef>,
    records: Vec<AssignedRecord>,
    journal_heads: Vec<JournalHead>,
    /// Exact committed-batch envelopes retained for portable export and offline fold entry.
    batches: Vec<CommittedBatch>,
}

struct PhysicalBindingVerification<'a> {
    mode: PhysicalBindingVerificationMode,
    verifier: &'a dyn PublicPhysicalBindingVerifier,
}

impl FoldMachine {
    fn new(run_id: RunId, admission: RunAdmitted, admission_ref: RecordRef) -> Self {
        Self {
            run_id,
            initial_bindings: admission.initial_bindings.clone(),
            admission,
            admission_ref,
            program: None,
            objects: BTreeMap::new(),
            object_first_seen_sequence: BTreeMap::new(),
            logical_keys: BTreeMap::new(),
            transitions: BTreeMap::new(),
            authorizations: BTreeMap::new(),
            occurrence_attempts: BTreeMap::new(),
            observations: BTreeMap::new(),
            semantic_head: None,
            closed_outcome_ref: None,
            records: Vec::new(),
            journal_heads: Vec::new(),
            batches: Vec::new(),
        }
    }

    fn record_batch(&mut self, batch: &CommittedBatch) {
        for object in &batch.objects {
            self.object_first_seen_sequence
                .entry(object.content_ref.clone())
                .or_insert(batch.head.run_sequence);
        }
        self.records.extend(batch.records.iter().cloned());
        self.journal_heads.push(batch.head.clone());
        self.batches.push(batch.clone());
    }

    fn admit_objects(&mut self, objects: Vec<HistoryObject>) -> super::Result<()> {
        for object in objects {
            match self.objects.get(&object.content_ref) {
                Some(existing) if existing == &object => {}
                Some(_) => return Err(invalid("object reference was rebound to different bytes")),
                None => {
                    self.objects.insert(object.content_ref.clone(), object);
                }
            }
        }
        Ok(())
    }

    fn initialize_program(&mut self, program: Arc<VerifiedProgramData>) -> super::Result<()> {
        validate_admission_material_refs(&self.admission, &self.objects, &program)?;
        validate_program_value_schemas(&program)?;
        if program.expanded().input_roots.len() != self.initial_bindings.len() {
            return Err(invalid(
                "admission input count differs from certified roots",
            ));
        }
        for (slot, binding) in program
            .expanded()
            .input_roots
            .iter()
            .zip(&self.initial_bindings)
        {
            validate_lexical_binding(slot, binding, &self.objects, &program)?;
        }
        self.program = Some(program);
        Ok(())
    }

    fn verify_admission_batch(&mut self, batch: &CommittedBatch) -> super::Result<DerivedProgram> {
        let records = batch.records.as_slice();
        if records.len() > 2
            || !matches!(records[0].record, RunRecord::RunAdmitted(_))
            || (records.len() == 2 && !matches!(records[1].record, RunRecord::RunClosed(_)))
        {
            return Err(invalid(
                "admission batch has an illegal record family shape",
            ));
        }
        if self.admission.store_scope_id != batch.store_scope_id
            || self.admission.store_epoch != batch.store_epoch
            || batch.tenant_fact_coordinate != TenantFactCoordinate::None
        {
            return Err(invalid(
                "admission writer lineage or tenant fact coordinate differs from its envelope",
            ));
        }
        self.insert_logical_record(&records[0])?;
        let derived = self.derive(false)?;
        let semantic_digest =
            semantic_state_digest(self.program_ref()?, &self.transitions, &derived.bindings)?;
        if semantic_digest != self.admission.genesis_semantic_state_digest {
            return Err(invalid("admission genesis semantic digest differs"));
        }
        self.semantic_head = Some(SemanticHead::Genesis {
            admission_ref: self.admission_ref.clone(),
            semantic_state_digest: semantic_digest,
        });
        self.verify_required_closure(records.get(1), &derived)?;
        Ok(derived)
    }

    fn apply_batch(
        &mut self,
        batch: &CommittedBatch,
        before: DerivedProgram,
        physical_binding_verification_mode: PhysicalBindingVerificationMode,
        physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
    ) -> super::Result<DerivedProgram> {
        if self.closed_outcome_ref.is_some() {
            return Err(invalid("a record exists after RunClosed"));
        }
        let Some(first) = batch.records.first() else {
            return Err(invalid("atomic batch is empty"));
        };
        let identity = StructuredStoreIdentity {
            store_scope_id: batch.store_scope_id.clone(),
            store_epoch: batch.store_epoch,
            physical_target: None,
        };
        match &first.record {
            RunRecord::RunAdmitted(_) => Err(invalid("RunAdmitted appears after the first record")),
            RunRecord::ExternalAccessAuthorized(record) => {
                if batch.records.len() != 1 {
                    return Err(invalid("authorization batch contains another record"));
                }
                let derived = self.apply_authorization(
                    &first.record_ref,
                    record,
                    before,
                    &identity,
                    &batch.tenant_fact_coordinate,
                    PhysicalBindingVerification {
                        mode: physical_binding_verification_mode,
                        verifier: physical_binding_verifier,
                    },
                )?;
                self.insert_logical_record(first)?;
                Ok(derived)
            }
            RunRecord::ExternalAccessObserved(record) => {
                if batch.records.len() != 1 {
                    return Err(invalid("observation batch contains another record"));
                }
                if batch.tenant_fact_coordinate != TenantFactCoordinate::None {
                    return Err(invalid(
                        "observation batch carries a tenant fact coordinate",
                    ));
                }
                let derived = self.apply_observation(
                    &first.record_ref,
                    record,
                    physical_binding_verification_mode,
                    physical_binding_verifier,
                )?;
                self.insert_logical_record(first)?;
                Ok(derived)
            }
            RunRecord::StateTransitionCommitted(record) => {
                if batch.records.len() > 2
                    || (batch.records.len() == 2
                        && !matches!(batch.records[1].record, RunRecord::RunClosed(_)))
                {
                    return Err(invalid("transition batch has an illegal closure shape"));
                }
                validate_transition_fact_coordinate(
                    &identity,
                    &self.admission.tenant_scope_id,
                    &first.record,
                    &batch.tenant_fact_coordinate,
                )?;
                let derived = self.apply_transition(&first.record_ref, record, before)?;
                self.insert_logical_record(first)?;
                self.verify_required_closure(batch.records.get(1), &derived)?;
                Ok(derived)
            }
            RunRecord::RunClosed(_) => Err(invalid("RunClosed is not adjacent to its cause")),
        }
    }

    fn insert_logical_record(&mut self, assigned: &AssignedRecord) -> super::Result<()> {
        let key = assigned.record.logical_key(&self.run_id);
        match self.logical_keys.get(&key) {
            Some(existing) if existing == &assigned.record_ref.record_hash => {
                Err(invalid("logical record is repeated inside one prefix"))
            }
            Some(_) => Err(invalid("logical record key has conflicting content")),
            None => {
                self.logical_keys
                    .insert(key, assigned.record_ref.record_hash.clone());
                Ok(())
            }
        }
    }

    fn program(&self) -> super::Result<&VerifiedProgramData> {
        self.program
            .as_ref()
            .map(Arc::as_ref)
            .ok_or_else(|| invalid("fold program is not initialized"))
    }

    fn program_ref(&self) -> super::Result<&ContentRef> {
        Ok(&self.admission.certified_program_ref)
    }

    fn semantic_head(&self) -> super::Result<&SemanticHead> {
        self.semantic_head
            .as_ref()
            .ok_or_else(|| invalid("fold semantic head is absent"))
    }

    fn apply_authorization(
        &mut self,
        record_ref: &RecordRef,
        record: &ExternalAccessAuthorized,
        before: DerivedProgram,
        identity: &StructuredStoreIdentity,
        tenant_fact_coordinate: &TenantFactCoordinate,
        physical_binding_verification: PhysicalBindingVerification<'_>,
    ) -> super::Result<DerivedProgram> {
        let state = minimum_action(&before.frontier)?;
        let is_prior_run_fact_selection = validate_authorization(
            record,
            state,
            &self.run_id,
            self.semantic_head()?,
            self.program()?,
            &self.admission,
            &self.objects,
            &before.bindings,
            &self.occurrence_attempts,
            &self.authorizations,
            &self.observations,
            physical_binding_verification.mode,
            physical_binding_verification.verifier,
        )?;
        validate_authorization_fact_coordinate(
            identity,
            &self.admission.tenant_scope_id,
            is_prior_run_fact_selection,
            tenant_fact_coordinate,
        )?;
        if self.authorizations.contains_key(&record.access_attempt_id) {
            return Err(invalid("access attempt identity is duplicated"));
        }
        self.occurrence_attempts
            .entry(record.occurrence_id.clone())
            .or_default()
            .push(record.access_attempt_id.clone());
        self.authorizations.insert(
            record.access_attempt_id.clone(),
            RecordedAuthorization {
                record_ref: record_ref.clone(),
                record: record.clone(),
            },
        );
        self.derive(false)
    }

    fn apply_observation(
        &mut self,
        record_ref: &RecordRef,
        record: &ExternalAccessObserved,
        physical_binding_verification_mode: PhysicalBindingVerificationMode,
        physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
    ) -> super::Result<DerivedProgram> {
        if self.observations.contains_key(&record.access_attempt_id) {
            return Err(invalid("access attempt has more than one observation"));
        }
        let authorization = self
            .authorizations
            .get(&record.access_attempt_id)
            .ok_or_else(|| invalid("observation has no exact authorization"))?;
        validate_observation(
            record,
            authorization,
            self.program()?,
            &self.objects,
            physical_binding_verification_mode,
            physical_binding_verifier,
        )?;
        self.observations.insert(
            record.access_attempt_id.clone(),
            RecordedObservation {
                record_ref: record_ref.clone(),
                record: record.clone(),
            },
        );
        self.derive(false)
    }

    fn apply_transition(
        &mut self,
        record_ref: &RecordRef,
        record: &StateTransitionCommitted,
        before: DerivedProgram,
    ) -> super::Result<DerivedProgram> {
        if self.transitions.contains_key(&record.occurrence_id) {
            return Err(invalid("state occurrence has more than one transition"));
        }
        let state = minimum_action(&before.frontier)?;
        validate_transition(
            record,
            state,
            self.semantic_head()?,
            self.program()?,
            &self.objects,
            &before.bindings,
            &self.authorizations,
            &self.observations,
        )?;
        if record.before_semantic_state_digest != *self.semantic_head()?.semantic_state_digest() {
            return Err(invalid("transition before semantic digest differs"));
        }
        self.transitions.insert(
            record.occurrence_id.clone(),
            RecordedTransition {
                record: record.clone(),
            },
        );
        let after = self.derive(false)?;
        let expected_after =
            semantic_state_digest(self.program_ref()?, &self.transitions, &after.bindings)?;
        if record.after_semantic_state_digest != expected_after {
            return Err(invalid("transition after semantic digest differs"));
        }
        self.semantic_head = Some(SemanticHead::Transition {
            transition_ref: record_ref.clone(),
            semantic_state_digest: expected_after,
        });
        Ok(after)
    }

    fn verify_required_closure(
        &mut self,
        closure: Option<&AssignedRecord>,
        derived: &DerivedProgram,
    ) -> super::Result<()> {
        match (&derived.cursor, closure) {
            (ProgramCursor::Closed { outcome_ref }, Some(assigned)) => {
                let RunRecord::RunClosed(RunClosed {
                    outcome_ref: recorded,
                }) = &assigned.record
                else {
                    return Err(invalid("required adjacent record is not RunClosed"));
                };
                if recorded != outcome_ref {
                    return Err(invalid("RunClosed outcome differs from derived root"));
                }
                self.insert_logical_record(assigned)?;
                self.closed_outcome_ref = Some(outcome_ref.clone());
                Ok(())
            }
            (ProgramCursor::Closed { .. }, None) => Err(invalid(
                "root outcome is derivable without adjacent RunClosed",
            )),
            (_, Some(_)) => Err(invalid("RunClosed appears before root outcome derivation")),
            (_, None) => Ok(()),
        }
    }

    fn derive(&self, allow_generate: bool) -> super::Result<DerivedProgram> {
        derive_program(self, allow_generate)
    }

    fn finish(
        self,
        journal_head: JournalHead,
        derived: DerivedProgram,
    ) -> super::Result<VerifiedStructuredRun> {
        if matches!(derived.cursor, ProgramCursor::Closed { .. })
            != self.closed_outcome_ref.is_some()
        {
            return Err(invalid("folded root completion differs from RunClosed"));
        }
        let program = self
            .program
            .as_ref()
            .cloned()
            .ok_or_else(|| invalid("fold program is not initialized"))?;
        let semantic_head = self
            .semantic_head
            .as_ref()
            .cloned()
            .ok_or_else(|| invalid("fold semantic head is absent"))?;
        Ok(VerifiedStructuredRun {
            state: Box::new(VerifiedFoldState {
                continuation: self,
                derived,
            }),
            program,
            journal_head,
            semantic_head,
        })
    }
}

#[derive(PartialEq, Eq)]
struct DerivedProgram {
    cursor: ProgramCursor,
    frontier: StructuredFrontier,
    bindings: BTreeMap<ContentRef, LexicalValueRef>,
    generated_objects: Vec<HistoryObject>,
    required_object_refs: BTreeSet<ContentRef>,
}

#[derive(Clone)]
struct BoundValue {
    reference: LexicalValueRef,
    value: Value,
}

enum FlowValue {
    Normal(BoundValue),
    ScopeFailure(BoundValue),
}

// These transient fold sums remain inline to avoid allocations at every
// nested block, failure boundary, and state visit.
#[allow(clippy::large_enum_variant)]
enum WalkResult {
    Pending(PendingCursor),
    Complete(FlowValue),
}

#[allow(clippy::large_enum_variant)]
enum PendingCursor {
    AtState(ActionableState),
    InFanOut {
        group_path: StructuralPath,
        lanes: Vec<LaneCursor>,
    },
}

#[allow(clippy::large_enum_variant)]
enum FailureResolution {
    Pending(PendingCursor),
    Recovered(BoundValue),
    ScopeFailure(BoundValue),
}

struct DerivationEngine<'a> {
    machine: &'a FoldMachine,
    bindings: BTreeMap<ContentRef, LexicalValueRef>,
    generated_objects: BTreeMap<ContentRef, HistoryObject>,
    required_object_refs: BTreeSet<ContentRef>,
    allow_generate: bool,
}

fn derive_program(machine: &FoldMachine, allow_generate: bool) -> super::Result<DerivedProgram> {
    let program = machine.program()?;
    let mut engine = DerivationEngine {
        machine,
        bindings: BTreeMap::new(),
        generated_objects: BTreeMap::new(),
        required_object_refs: BTreeSet::new(),
        allow_generate,
    };
    for (slot, binding) in program
        .expanded()
        .input_roots
        .iter()
        .zip(&machine.initial_bindings)
    {
        engine.bind_exact(slot, binding.clone())?;
    }
    let result = walk_block(&mut engine, &program.expanded().root)?;
    let cursor = match result {
        WalkResult::Pending(PendingCursor::AtState(state)) => ProgramCursor::AtState(state),
        WalkResult::Pending(PendingCursor::InFanOut { group_path, lanes }) => {
            ProgramCursor::InFanOut { group_path, lanes }
        }
        WalkResult::Complete(flow) => {
            let (variant, value) = match flow {
                FlowValue::Normal(value) => ("Success", value),
                FlowValue::ScopeFailure(value) => ("Failure", value),
            };
            let outcome = tagged_wrapper(
                variant,
                serde_json::to_value(&value.reference)
                    .map_err(|_| invalid("operation outcome reference cannot be encoded"))?,
            );
            let outcome_ref = engine.framework_object_ref(
                OPERATION_OUTCOME_OBJECT_TYPE,
                "mfm.structured-operation-outcome",
                &outcome,
            )?;
            ProgramCursor::Closed { outcome_ref }
        }
    };
    let frontier = derive_frontier(&cursor)?;
    Ok(DerivedProgram {
        cursor,
        frontier,
        bindings: engine.bindings,
        generated_objects: engine.generated_objects.into_values().collect(),
        required_object_refs: engine.required_object_refs,
    })
}

impl DerivationEngine<'_> {
    fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.machine
            .objects
            .get(content_ref)
            .or_else(|| self.generated_objects.get(content_ref))
    }

    fn typed_value_json(&self, value: &TypedValueRef) -> super::Result<Value> {
        let object = self
            .object(&value.value_ref)
            .ok_or_else(|| invalid("typed value object is absent"))?;
        object
            .decode()
            .map_err(|_| invalid("typed value object cannot be strictly decoded"))
    }

    fn bind_exact(&mut self, slot: &LexicalSlot, binding: LexicalValueRef) -> super::Result<()> {
        let slot_ref = slot
            .content_ref()
            .map_err(|_| invalid("lexical slot reference cannot be derived"))?;
        if binding.slot_ref != slot_ref || binding.value.contract_ref != slot.contract_ref {
            return Err(invalid("lexical binding differs from its certified slot"));
        }
        let object = self
            .object(&binding.value.value_ref)
            .ok_or_else(|| invalid("typed value object is absent"))?;
        validate_typed_value_object(&binding.value, object, self.machine.program()?)?;
        match self.bindings.get(&slot_ref) {
            Some(existing) if existing == &binding => Ok(()),
            Some(_) => Err(invalid("lexical slot was rebound to different content")),
            None => {
                self.bindings.insert(slot_ref, binding);
                Ok(())
            }
        }
    }

    fn bind_alias(&mut self, slot: &LexicalSlot, source: &BoundValue) -> super::Result<BoundValue> {
        self.bind_alias_with_origin(slot, source, None)
    }

    fn bind_alias_with_origin(
        &mut self,
        slot: &LexicalSlot,
        source: &BoundValue,
        structural_origin: Option<StructuralValueOrigin>,
    ) -> super::Result<BoundValue> {
        let binding = LexicalValueRef {
            slot_ref: slot
                .content_ref()
                .map_err(|_| invalid("alias slot reference cannot be derived"))?,
            value: TypedValueRef {
                contract_ref: slot.contract_ref.clone(),
                value_ref: source.reference.value.value_ref.clone(),
            },
            structural_origin,
        };
        self.bind_exact(slot, binding.clone())?;
        Ok(BoundValue {
            reference: binding,
            value: source.value.clone(),
        })
    }

    fn bind_derived(&mut self, slot: &LexicalSlot, value: Value) -> super::Result<BoundValue> {
        self.bind_derived_with_origin(slot, value, None)
    }

    fn bind_derived_with_origin(
        &mut self,
        slot: &LexicalSlot,
        value: Value,
        structural_origin: Option<StructuralValueOrigin>,
    ) -> super::Result<BoundValue> {
        let schema_id = typed_value_schema_id(self.machine.program()?, &slot.contract_ref)?;
        let canonical =
            canonical_json(&value).map_err(|_| invalid("derived typed value is not canonical"))?;
        let object = HistoryObject::new(
            StableId::new(TYPED_VALUE_OBJECT_TYPE)
                .map_err(|_| invalid("typed value object type is invalid"))?,
            schema_id,
            canonical.as_str(),
        )
        .map_err(|_| invalid("derived typed value object cannot be constructed"))?;
        let value_ref = object.content_ref.clone();
        self.required_object_refs.insert(value_ref.clone());
        match self.machine.objects.get(&value_ref) {
            Some(existing) if existing == &object => {}
            Some(_) => return Err(invalid("derived typed value content is rebound")),
            None if self.allow_generate => {
                self.generated_objects.insert(value_ref.clone(), object);
            }
            None => return Err(invalid("required derived typed value object is absent")),
        }
        let binding = LexicalValueRef {
            slot_ref: slot
                .content_ref()
                .map_err(|_| invalid("derived slot reference cannot be derived"))?,
            structural_origin,
            value: TypedValueRef {
                contract_ref: slot.contract_ref.clone(),
                value_ref,
            },
        };
        self.bind_exact(slot, binding.clone())?;
        Ok(BoundValue {
            reference: binding,
            value,
        })
    }

    fn framework_object_ref(
        &mut self,
        object_type: &str,
        schema_name: &str,
        value: &Value,
    ) -> super::Result<ContentRef> {
        let canonical = canonical_json(value)
            .map_err(|_| invalid("derived structural object is not canonical"))?;
        let object = HistoryObject::new(
            StableId::new(object_type).map_err(|_| invalid("derived object type is invalid"))?,
            framework_schema_id(schema_name)?,
            canonical.as_str(),
        )
        .map_err(|_| invalid("derived structural object cannot be constructed"))?;
        self.required_object_refs.insert(object.content_ref.clone());
        match self.machine.objects.get(&object.content_ref) {
            Some(existing) if existing == &object => {}
            Some(_) => return Err(invalid("derived object content is rebound")),
            None if self.allow_generate => {
                self.generated_objects
                    .insert(object.content_ref.clone(), object.clone());
            }
            None => return Err(invalid("required derived structural object is absent")),
        }
        Ok(object.content_ref)
    }

    fn resolve_slot(&mut self, slot: &LexicalSlot) -> super::Result<BoundValue> {
        let slot_ref = slot
            .content_ref()
            .map_err(|_| invalid("lexical slot reference cannot be derived"))?;
        if let Some(binding) = self.bindings.get(&slot_ref).cloned() {
            let value = self.typed_value_json(&binding.value)?;
            return Ok(BoundValue {
                reference: binding,
                value,
            });
        }
        match &slot.producer {
            LexicalProducer::VariantPayload {
                selector,
                canonical_tag,
                payload_path,
            } => {
                let selector = self.resolve_slot(selector)?;
                let selected_tag = selector
                    .value
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("closed-sum selector has no canonical kind"))?;
                if selected_tag != canonical_tag {
                    return Err(invalid("variant payload tag is not active"));
                }
                let mut payload = &selector.value;
                for segment in payload_path {
                    payload = payload
                        .get(segment.as_str())
                        .ok_or_else(|| invalid("variant payload path is absent"))?;
                }
                self.bind_derived(slot, payload.clone())
            }
            LexicalProducer::FragmentInput { source, .. }
            | LexicalProducer::FragmentBoundary { source, .. }
            | LexicalProducer::ArmValue { source, .. } => {
                let source = self.resolve_slot(source)?;
                self.bind_alias(slot, &source)
            }
            LexicalProducer::AdmissionRoot { .. }
            | LexicalProducer::StateOutput { .. }
            | LexicalProducer::MatchMerge { .. }
            | LexicalProducer::ScopeFailureMerge { .. }
            | LexicalProducer::LaneOutcome { .. }
            | LexicalProducer::FanOutJoin { .. }
            | LexicalProducer::AuthoredCallOutput { .. } => Err(invalid(
                "required lexical producer is not committed or selected",
            )),
        }
    }
}

fn walk_block(
    engine: &mut DerivationEngine<'_>,
    block: &ExpandedBlock,
) -> super::Result<WalkResult> {
    for declaration in &block.declarations {
        match walk_declaration(engine, declaration)? {
            DeclarationResult::Continue => {}
            DeclarationResult::Pending(cursor) => return Ok(WalkResult::Pending(cursor)),
            DeclarationResult::ScopeFailure(value) => {
                return Ok(WalkResult::Complete(FlowValue::ScopeFailure(value)));
            }
        }
    }
    match &block.tail {
        BlockTail::Normal(slot) => Ok(WalkResult::Complete(FlowValue::Normal(
            engine.resolve_slot(slot)?,
        ))),
        BlockTail::ScopeFailure(slot) => Ok(WalkResult::Complete(FlowValue::ScopeFailure(
            engine.resolve_slot(slot)?,
        ))),
    }
}

#[allow(clippy::large_enum_variant)]
enum DeclarationResult {
    Continue,
    Pending(PendingCursor),
    ScopeFailure(BoundValue),
}

fn walk_declaration(
    engine: &mut DerivationEngine<'_>,
    declaration: &ExpandedDeclaration,
) -> super::Result<DeclarationResult> {
    match declaration {
        ExpandedDeclaration::State(state) => match walk_state(engine, state)? {
            StateWalk::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
            StateWalk::Success(value) => {
                engine.bind_exact(&state.output_slot, value.reference)?;
                Ok(DeclarationResult::Continue)
            }
            StateWalk::Failure(failure) => {
                match resolve_failure(engine, &state.failure_boundary, failure)? {
                    FailureResolution::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
                    FailureResolution::Recovered(value) => {
                        engine.bind_alias(&state.output_slot, &value)?;
                        Ok(DeclarationResult::Continue)
                    }
                    FailureResolution::ScopeFailure(value) => {
                        Ok(DeclarationResult::ScopeFailure(value))
                    }
                }
            }
        },
        ExpandedDeclaration::Match(binding) => walk_match(engine, binding),
        ExpandedDeclaration::Fragment(fragment) => walk_fragment(engine, fragment),
        ExpandedDeclaration::FanOut(group) => walk_fan_out(engine, group),
    }
}

#[allow(clippy::large_enum_variant)]
enum StateWalk {
    Pending(PendingCursor),
    Success(BoundValue),
    Failure(BoundValue),
}

fn walk_state(
    engine: &mut DerivationEngine<'_>,
    state: &ExpandedStateBinding,
) -> super::Result<StateWalk> {
    let [input_slot] = state.inputs.as_slice() else {
        return Err(invalid("expanded state does not have one exact input"));
    };
    let input = engine.resolve_slot(input_slot)?;
    if let Some(transition) = engine.machine.transitions.get(&state.occurrence_id) {
        validate_transition_shape(
            &transition.record,
            state,
            &input.reference,
            engine.machine.program()?,
            &engine.machine.objects,
        )?;
        let expected_outcome = match &transition.record.outcome {
            StateOutcomeRef::Success(value) => serde_json::json!({ "Success": value }),
            StateOutcomeRef::Failure(value) => serde_json::json!({ "Failure": value }),
        };
        let outcome_ref = engine.framework_object_ref(
            STATE_OUTCOME_OBJECT_TYPE,
            "mfm.structured-state-outcome",
            &expected_outcome,
        )?;
        if outcome_ref != transition.record.outcome_ref {
            return Err(invalid("state outcome object differs from transition"));
        }
        return match &transition.record.outcome {
            StateOutcomeRef::Success(value) => {
                let output = engine.typed_value_json(&value.value)?;
                engine.bind_exact(&state.output_slot, value.clone())?;
                Ok(StateWalk::Success(BoundValue {
                    reference: value.clone(),
                    value: output,
                }))
            }
            StateOutcomeRef::Failure(value) => {
                let failure = engine.typed_value_json(&value.value)?;
                let CertifiedFailureBoundary::Typed { source_slot, .. } = &state.failure_boundary
                else {
                    return Err(invalid("Never state committed a failure"));
                };
                engine.bind_exact(source_slot, value.clone())?;
                Ok(StateWalk::Failure(BoundValue {
                    reference: value.clone(),
                    value: failure,
                }))
            }
        };
    }
    let leaf = state_leaf(
        state,
        &engine.machine.occurrence_attempts,
        &engine.machine.authorizations,
        &engine.machine.observations,
    )?;
    Ok(StateWalk::Pending(PendingCursor::AtState(
        ActionableState {
            occurrence_id: state.occurrence_id.clone(),
            occurrence_path: state.occurrence_path.clone(),
            semantic_call_id: state.semantic_call_id.clone(),
            state_contract_ref: state.contract.state_contract_ref.clone(),
            input: input.reference,
            capability_contract_ref: state.contract.execution.capability_contract_ref().cloned(),
            stable_resource_lineage_contract_ref: state_stable_resource_lineage_contract_ref(
                engine.machine.program()?,
                state,
            )?,
            execution_kind: state.contract.execution.kind(),
            leaf,
        },
    )))
}

fn walk_match(
    engine: &mut DerivationEngine<'_>,
    binding: &ExpandedMatch,
) -> super::Result<DeclarationResult> {
    let selector = engine.resolve_slot(&binding.selector)?;
    let tag = selector
        .value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Match selector has no canonical kind tag"))?;
    let (arm_ordinal, arm) = binding
        .arms
        .iter()
        .enumerate()
        .find(|(_, arm)| arm.canonical_tag == tag)
        .ok_or_else(|| invalid("Match selector tag is not certified"))?;
    match walk_block(engine, &arm.body)? {
        WalkResult::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
        WalkResult::Complete(FlowValue::Normal(value)) => {
            let match_path_ref = binding
                .path
                .content_ref()
                .map_err(|_| invalid("Match path reference cannot be derived"))?;
            let origin = StructuralValueOrigin::MatchArm {
                match_path_ref,
                arm_ordinal: u32::try_from(arm_ordinal)
                    .map_err(|_| invalid("Match arm ordinal overflowed"))?,
                arm_key: arm.label.clone(),
                value_contract_ref: binding.output_slot.contract_ref.clone(),
                source_slot_ref: value.reference.slot_ref.clone(),
                source_value_ref: value.reference.value.value_ref.clone(),
            };
            engine.bind_alias_with_origin(&binding.output_slot, &value, Some(origin))?;
            Ok(DeclarationResult::Continue)
        }
        WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
            Ok(DeclarationResult::ScopeFailure(value))
        }
    }
}

fn walk_fragment(
    engine: &mut DerivationEngine<'_>,
    fragment: &ExpandedFragment,
) -> super::Result<DeclarationResult> {
    for input in &fragment.input_bindings {
        let source = engine.resolve_slot(&input.caller_slot)?;
        let rebound = LexicalSlot {
            lexical_path: fragment.path.clone(),
            contract_ref: input.child_contract_ref.clone(),
            producer: LexicalProducer::FragmentInput {
                boundary_id: fragment.boundary_id.clone(),
                child_root_id: input.child_root_id.clone(),
                source: Box::new(input.caller_slot.clone()),
            },
        };
        engine.bind_alias(&rebound, &source)?;
    }
    match walk_block(engine, &fragment.body)? {
        WalkResult::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
        WalkResult::Complete(FlowValue::Normal(value)) => {
            engine.bind_alias(&fragment.success_slot, &value)?;
            Ok(DeclarationResult::Continue)
        }
        WalkResult::Complete(FlowValue::ScopeFailure(failure)) => {
            match resolve_failure(engine, &fragment.failure_boundary, failure)? {
                FailureResolution::Pending(cursor) => Ok(DeclarationResult::Pending(cursor)),
                FailureResolution::Recovered(value) => {
                    engine.bind_alias(&fragment.success_slot, &value)?;
                    Ok(DeclarationResult::Continue)
                }
                FailureResolution::ScopeFailure(value) => {
                    Ok(DeclarationResult::ScopeFailure(value))
                }
            }
        }
    }
}

fn walk_fan_out(
    engine: &mut DerivationEngine<'_>,
    group: &ExpandedFanOut,
) -> super::Result<DeclarationResult> {
    let parent_bindings = engine.bindings.clone();
    let mut joined_bindings = parent_bindings.clone();
    let mut lane_cursors = Vec::with_capacity(group.lanes.len());
    let mut lane_values = Vec::with_capacity(group.lanes.len());
    let mut any_pending = false;
    let group_path_ref = group
        .path
        .content_ref()
        .map_err(|_| invalid("fan-out group path reference cannot be derived"))?;
    for (lane_ordinal, lane) in group.lanes.iter().enumerate() {
        engine.bindings = parent_bindings.clone();
        match walk_block(engine, &lane.body)? {
            WalkResult::Pending(cursor) => {
                any_pending = true;
                lane_cursors.push(pending_to_lane(cursor));
                lane_values.push(None);
            }
            WalkResult::Complete(flow) => {
                let (variant, value) = match flow {
                    FlowValue::Normal(value) => ("Success", value),
                    FlowValue::ScopeFailure(value) => ("Failure", value),
                };
                let origin = StructuralValueOrigin::FanOutLane {
                    group_path_ref: group_path_ref.clone(),
                    lane_ordinal: u32::try_from(lane_ordinal)
                        .map_err(|_| invalid("fan-out lane ordinal overflowed"))?,
                    lane_key: lane.key.clone(),
                    outcome_contract_ref: lane.outcome_slot.contract_ref.clone(),
                    source_slot_ref: value.reference.slot_ref.clone(),
                    source_value_ref: value.reference.value.value_ref.clone(),
                };
                let lane_json = tagged_wrapper(variant, value.value);
                let lane_value =
                    engine.bind_derived_with_origin(&lane.outcome_slot, lane_json, Some(origin))?;
                lane_cursors.push(LaneCursor::Completed {
                    outcome_ref: lane_value.reference.value.value_ref.clone(),
                });
                lane_values.push(Some(lane_value));
            }
        }
        merge_lane_bindings(&mut joined_bindings, &engine.bindings)?;
    }
    engine.bindings = joined_bindings;
    if any_pending {
        return Ok(DeclarationResult::Pending(PendingCursor::InFanOut {
            group_path: group.path.clone(),
            lanes: lane_cursors,
        }));
    }
    let ordered = lane_values
        .into_iter()
        .map(|value| {
            value
                .map(|value| value.value)
                .ok_or_else(|| invalid("completed fan-out lane has no outcome"))
        })
        .collect::<super::Result<Vec<_>>>()?;
    let (head, tail) = ordered
        .split_first()
        .map(|(head, tail)| (head.clone(), tail.to_vec()))
        .ok_or_else(|| invalid("fan-out join requires a non-empty lane set"))?;
    engine.bind_derived(
        &group.output_slot,
        serde_json::json!({ "head": head, "tail": tail }),
    )?;
    Ok(DeclarationResult::Continue)
}

fn merge_lane_bindings(
    joined: &mut BTreeMap<ContentRef, LexicalValueRef>,
    lane: &BTreeMap<ContentRef, LexicalValueRef>,
) -> super::Result<()> {
    for (slot_ref, binding) in lane {
        match joined.get(slot_ref) {
            Some(existing) if existing == binding => {}
            Some(_) => return Err(invalid("fan-out lanes bind one slot to different content")),
            None => {
                joined.insert(slot_ref.clone(), binding.clone());
            }
        }
    }
    Ok(())
}

fn pending_to_lane(cursor: PendingCursor) -> LaneCursor {
    match cursor {
        PendingCursor::AtState(state) => LaneCursor::AtState(state),
        PendingCursor::InFanOut { group_path, lanes } => LaneCursor::InFanOut { group_path, lanes },
    }
}

fn tagged_wrapper(tag: &str, value: Value) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(tag.to_owned(), value);
    Value::Object(object)
}

fn resolve_failure(
    engine: &mut DerivationEngine<'_>,
    boundary: &CertifiedFailureBoundary,
    failure: BoundValue,
) -> super::Result<FailureResolution> {
    let CertifiedFailureBoundary::Typed {
        source_slot, plan, ..
    } = boundary
    else {
        return Err(invalid("Never boundary received a committed failure"));
    };
    // Rebind the body/child failure value into the certified boundary source
    // slot. The body exit and the FragmentBoundary::TypedFailure (or state
    // failure) source slot are distinct lexical identities; bind_exact would
    // reject a legitimate child failure that must cross the boundary.
    engine.bind_alias(source_slot, &failure)?;
    match plan.as_ref() {
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            boundary_slot,
            source_slot,
            ..
        } => {
            let mut prefix = before_boundary.as_ref().clone();
            prefix.tail = BlockTail::Normal(source_slot.clone());
            let mut current = match walk_block(engine, &prefix)? {
                WalkResult::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                WalkResult::Complete(FlowValue::Normal(value)) => value,
                WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
                    return Ok(FailureResolution::ScopeFailure(value));
                }
            };
            for link in mapping_chain {
                match walk_state(engine, &link.mapper)? {
                    StateWalk::Pending(cursor) => {
                        return Ok(FailureResolution::Pending(cursor));
                    }
                    StateWalk::Success(value) => {
                        engine.bind_exact(&link.output_slot, value.reference.clone())?;
                        current = value;
                    }
                    StateWalk::Failure(_) => {
                        return Err(invalid("certified Pure Never failure mapper failed"));
                    }
                }
            }
            let source = if mapping_chain.is_empty() {
                current
            } else {
                engine.resolve_slot(
                    &mapping_chain
                        .last()
                        .ok_or_else(|| invalid("failure mapping chain disappeared"))?
                        .output_slot,
                )?
            };
            let boundary_value = engine.bind_alias(boundary_slot, &source)?;
            Ok(FailureResolution::ScopeFailure(boundary_value))
        }
        FailurePlan::Handled {
            before_handler,
            handler,
            continuation,
            ..
        } => {
            match walk_block(engine, before_handler)? {
                WalkResult::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
                    return Ok(FailureResolution::ScopeFailure(value));
                }
                WalkResult::Complete(FlowValue::Normal(_)) => {}
            }
            let handled = match walk_state(engine, handler)? {
                StateWalk::Pending(cursor) => return Ok(FailureResolution::Pending(cursor)),
                StateWalk::Success(value) => value,
                StateWalk::Failure(_) => {
                    return Err(invalid("certified Pure Never failure handler failed"));
                }
            };
            engine.bind_exact(&handler.output_slot, handled.reference.clone())?;
            match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation {
                    payload_slot,
                    failure_tail,
                    ..
                } => {
                    let payload = engine.resolve_slot(payload_slot)?;
                    let tail = engine.bind_alias(failure_tail, &payload)?;
                    Ok(FailureResolution::ScopeFailure(tail))
                }
                HandlerContinuation::CustomRecovery { arms, .. } => {
                    let tag = handled
                        .value
                        .get("kind")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("custom failure handler has no canonical tag"))?;
                    let arm = arms
                        .iter()
                        .find(|arm| arm.canonical_tag == tag)
                        .ok_or_else(|| invalid("custom failure route is not certified"))?;
                    match walk_block(engine, &arm.body)? {
                        WalkResult::Pending(cursor) => Ok(FailureResolution::Pending(cursor)),
                        WalkResult::Complete(FlowValue::Normal(value)) => {
                            Ok(FailureResolution::Recovered(value))
                        }
                        WalkResult::Complete(FlowValue::ScopeFailure(value)) => {
                            Ok(FailureResolution::ScopeFailure(value))
                        }
                    }
                }
            }
        }
    }
}

fn state_leaf(
    state: &ExpandedStateBinding,
    occurrence_attempts: &BTreeMap<OccurrenceId, Vec<AccessAttemptId>>,
    authorizations: &BTreeMap<AccessAttemptId, RecordedAuthorization>,
    observations: &BTreeMap<AccessAttemptId, RecordedObservation>,
) -> super::Result<StateLeaf> {
    let Some(attempt_id) = occurrence_attempts
        .get(&state.occurrence_id)
        .and_then(|attempts| attempts.last())
    else {
        return Ok(StateLeaf::Ready);
    };
    let authorization = authorizations
        .get(attempt_id)
        .ok_or_else(|| invalid("occurrence attempt has no authorization"))?;
    let Some(observation) = observations.get(attempt_id) else {
        return Ok(StateLeaf::Authorized {
            access_kind: authorization.record.access_kind,
            access_attempt_id: attempt_id.clone(),
        });
    };
    match &observation.record.outcome {
        ObservationOutcome::Returned { .. } | ObservationOutcome::SafeFailure { .. } => {
            Ok(StateLeaf::ObservedForSettlement {
                access_attempt_id: attempt_id.clone(),
                observation_ref: observation.record_ref.clone(),
            })
        }
        ObservationOutcome::SupersededBeforeEntry {
            public_lineage_head_ref,
            ..
        } => Ok(StateLeaf::Refreshable {
            next_attempt_ordinal: authorization
                .record
                .attempt_ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("access attempt ordinal overflowed"))?,
            public_lineage_head_ref: public_lineage_head_ref.clone(),
        }),
        ObservationOutcome::EntryUnknown { .. } => Ok(StateLeaf::EntryUnknown {
            access_attempt_id: attempt_id.clone(),
        }),
        ObservationOutcome::IntegrityFault { .. } => Ok(StateLeaf::BlockedIntegrity {
            observation_ref: observation.record_ref.clone(),
        }),
    }
}

fn derive_frontier(cursor: &ProgramCursor) -> super::Result<StructuredFrontier> {
    match cursor {
        ProgramCursor::AtState(state) => state_frontier(state),
        ProgramCursor::InFanOut { lanes, .. } => fan_out_frontier(lanes),
        ProgramCursor::Closed { .. } => Ok(StructuredFrontier::Complete),
    }
}

fn state_frontier(state: &ActionableState) -> super::Result<StructuredFrontier> {
    let frontier = match (&state.execution_kind, &state.leaf) {
        (StructuredExecutionKind::Pure, StateLeaf::Ready)
        | (
            StructuredExecutionKind::Read | StructuredExecutionKind::Effect,
            StateLeaf::Ready | StateLeaf::ObservedForSettlement { .. },
        ) => StructuredFrontier::Actions(vec![state.clone()]),
        (StructuredExecutionKind::Effect, StateLeaf::Refreshable { .. }) => {
            StructuredFrontier::Actions(vec![state.clone()])
        }
        (StructuredExecutionKind::Read, StateLeaf::Refreshable { .. }) => {
            return Err(invalid("Read state cannot have a refreshable access leaf"));
        }
        (StructuredExecutionKind::Read, StateLeaf::Authorized { .. }) => {
            StructuredFrontier::WaitingReads
        }
        (StructuredExecutionKind::Effect, StateLeaf::Authorized { .. })
        | (StructuredExecutionKind::Effect, StateLeaf::EntryUnknown { .. }) => {
            StructuredFrontier::PossibleEntry
        }
        (_, StateLeaf::BlockedIntegrity { .. }) => StructuredFrontier::BlockedIntegrity,
        _ => StructuredFrontier::BlockedIntegrity,
    };
    Ok(frontier)
}

fn fan_out_frontier(lanes: &[LaneCursor]) -> super::Result<StructuredFrontier> {
    let mut saw_waiting = false;
    for lane in lanes {
        let frontier = match lane {
            LaneCursor::AtState(state) => {
                if state.execution_kind == StructuredExecutionKind::Effect {
                    return Err(invalid("Effect state cannot appear inside fan-out"));
                }
                state_frontier(state)?
            }
            LaneCursor::InFanOut { lanes, .. } => fan_out_frontier(lanes)?,
            LaneCursor::Completed { .. } => StructuredFrontier::Complete,
        };
        match frontier {
            StructuredFrontier::Complete => {}
            StructuredFrontier::WaitingReads => saw_waiting = true,
            StructuredFrontier::Actions(mut actions) => {
                actions.sort_by(|left, right| left.occurrence_path.cmp(&right.occurrence_path));
                return Ok(StructuredFrontier::Actions(actions));
            }
            StructuredFrontier::PossibleEntry => {
                return Err(invalid(
                    "possible-entry barrier cannot appear inside fan-out",
                ));
            }
            StructuredFrontier::BlockedIntegrity => {
                return Ok(StructuredFrontier::BlockedIntegrity);
            }
        }
    }
    if saw_waiting {
        Ok(StructuredFrontier::WaitingReads)
    } else {
        Ok(StructuredFrontier::Complete)
    }
}

fn minimum_action(frontier: &StructuredFrontier) -> super::Result<&ActionableState> {
    let StructuredFrontier::Actions(actions) = frontier else {
        return Err(invalid("history record targets a non-actionable cursor"));
    };
    actions
        .iter()
        .min_by(|left, right| left.occurrence_path.cmp(&right.occurrence_path))
        .ok_or_else(|| invalid("action frontier is empty"))
}

fn framework_schema_id(name: &str) -> super::Result<SchemaId> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.structured-schema.v1:{name}:1").as_bytes()),
    )
    .map_err(|_| invalid("framework schema identity cannot be derived"))
}

fn framework_object(
    object_type: &str,
    schema_name: &str,
    value: &impl Serialize,
) -> super::Result<HistoryObject> {
    let canonical =
        canonical_json(value).map_err(|_| invalid("framework object cannot be canonicalized"))?;
    HistoryObject::new(
        StableId::new(object_type).map_err(|_| invalid("framework object type is invalid"))?,
        framework_schema_id(schema_name)?,
        canonical.as_str(),
    )
    .map_err(|_| invalid("framework object cannot be constructed"))
}

fn proposed_typed_object(
    value: &ProposedCanonicalValue,
    contract_ref: &ContentRef,
    program: &VerifiedProgramData,
) -> super::Result<(HistoryObject, TypedValueRef)> {
    let contract: RetainedValueContract = decode_component(program.document(), contract_ref)?;
    let object = HistoryObject::new(
        StableId::new(TYPED_VALUE_OBJECT_TYPE)
            .map_err(|_| invalid("typed value object type is invalid"))?,
        contract.schema_id().clone(),
        value.canonical().as_str(),
    )
    .map_err(|_| invalid("proposed typed value object cannot be constructed"))?;
    let typed = TypedValueRef {
        contract_ref: contract_ref.clone(),
        value_ref: object.content_ref.clone(),
    };
    validate_typed_value_object(&typed, &object, program)?;
    Ok((object, typed))
}

fn prepare_fact_proposals(
    proposals: &FactSet,
    state: &ExpandedStateBinding,
    program: &VerifiedProgramData,
    objects: &mut Vec<HistoryObject>,
) -> super::Result<Vec<CommittedFactRef>> {
    let mut proposals = proposals.as_slice().iter().peekable();
    let mut committed = Vec::new();
    for slot in &state.contract.fact_slots {
        let mut slot_count = 0_u32;
        while proposals
            .peek()
            .is_some_and(|proposal| proposal.fact_slot_ordinal() == slot.fact_slot_ordinal())
        {
            if slot_count == slot.maximum_emissions() {
                return Err(invalid("proposed fact slot exceeds its certified maximum"));
            }
            let proposal = proposals
                .next()
                .ok_or_else(|| invalid("proposed fact iterator disappeared"))?;
            if proposal.descriptor_ref() != slot.fact_descriptor_ref() {
                return Err(invalid(
                    "proposed fact descriptor differs from certified slot",
                ));
            }
            let subject_contract_ref =
                mfm_spec::structured::retained_value_contract_ref(slot.subject_contract())
                    .map_err(|_| invalid("fact subject contract reference cannot be derived"))?;
            let response_contract_ref =
                mfm_spec::structured::retained_value_contract_ref(slot.response_contract())
                    .map_err(|_| invalid("fact response contract reference cannot be derived"))?;
            let (subject_object, subject) = proposed_fact_component(
                proposal.subject(),
                slot.subject_contract(),
                &subject_contract_ref,
                program,
            )?;
            let (response_object, response) = proposed_fact_component(
                proposal.response(),
                slot.response_contract(),
                &response_contract_ref,
                program,
            )?;
            objects.extend([subject_object, response_object]);
            let claim = StructuredFactClaimPreimage {
                descriptor_ref: proposal.descriptor_ref(),
                subject: &subject,
                response: &response,
            };
            let claim_object =
                framework_object(FACT_CLAIM_OBJECT_TYPE, "mfm.structured-fact-claim", &claim)?;
            let emission_ordinal = u32::try_from(committed.len())
                .map_err(|_| invalid("fact emission ordinal exceeds u32"))?;
            committed.push(CommittedFactRef {
                emission_ordinal,
                fact_slot_ordinal: slot.fact_slot_ordinal(),
                descriptor_ref: proposal.descriptor_ref().clone(),
                subject,
                response,
                claim_ref: claim_object.content_ref.clone(),
            });
            objects.push(claim_object);
            slot_count = slot_count
                .checked_add(1)
                .ok_or_else(|| invalid("proposed fact slot count overflowed"))?;
        }
        if slot_count < slot.minimum_emissions() {
            return Err(invalid("proposed fact slot is below its certified minimum"));
        }
    }
    if proposals.next().is_some() {
        return Err(invalid("callback proposed an undeclared fact slot"));
    }
    Ok(committed)
}

fn proposed_fact_component(
    proposed: &ProposedFactValue,
    contract: &RetainedValueContract,
    contract_ref: &ContentRef,
    program: &VerifiedProgramData,
) -> super::Result<(HistoryObject, TypedValueRef)> {
    if proposed.schema_id() != contract.schema_id()
        || proposed.semantic_type_id() != contract.semantic_type_id()
        || proposed.role() != contract.role()
        || proposed.media_type() != contract.media_type()
        || proposed.evidence_contract_ref() != contract.evidence_contract_ref()
    {
        return Err(invalid(
            "proposed fact component differs from certified contract",
        ));
    }
    let object = HistoryObject::new(
        StableId::new(TYPED_VALUE_OBJECT_TYPE)
            .map_err(|_| invalid("fact value object type is invalid"))?,
        contract.schema_id().clone(),
        proposed.canonical().as_str(),
    )
    .map_err(|_| invalid("fact value object cannot be constructed"))?;
    if &object.content_ref != proposed.content_ref() {
        return Err(invalid(
            "proposed fact content identity differs from exact bytes",
        ));
    }
    let typed = TypedValueRef {
        contract_ref: contract_ref.clone(),
        value_ref: object.content_ref.clone(),
    };
    validate_typed_value_object(&typed, &object, program)?;
    Ok((object, typed))
}

fn canonicalize_objects(objects: &mut Vec<HistoryObject>) -> super::Result<()> {
    objects.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
    let mut canonical: Vec<HistoryObject> = Vec::with_capacity(objects.len());
    for object in objects.drain(..) {
        match canonical.last() {
            Some(existing) if existing == &object => {}
            Some(existing) if existing.content_ref == object.content_ref => {
                return Err(invalid(
                    "one content reference names different object bytes",
                ));
            }
            _ => canonical.push(object),
        }
    }
    *objects = canonical;
    Ok(())
}

fn filter_existing_objects(
    objects: &mut Vec<HistoryObject>,
    existing: &BTreeMap<ContentRef, HistoryObject>,
) -> super::Result<()> {
    for object in objects.iter() {
        if existing
            .get(&object.content_ref)
            .is_some_and(|persisted| persisted != object)
        {
            return Err(invalid("existing object content differs from proposal"));
        }
    }
    objects.retain(|object| !existing.contains_key(&object.content_ref));
    canonicalize_objects(objects)
}

fn placeholder_semantic_digest() -> RunSemanticStateDigest {
    RunSemanticStateDigest::from_digest(sha256_digest_bytes(
        b"mfm.structured-placeholder-semantic-state.v1",
    ))
}

fn placeholder_record_ref(run_id: &RunId) -> RecordRef {
    RecordRef {
        run_id: run_id.clone(),
        run_sequence: 0,
        ordinal: 0,
        record_hash: JournalRecordHash::from_digest(sha256_digest_bytes(
            b"mfm.structured-placeholder-record.v1",
        )),
    }
}

trait HistoryObjectLookup {
    fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject>;
}

impl HistoryObjectLookup for BTreeMap<ContentRef, HistoryObject> {
    fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.get(content_ref)
    }
}

struct HistoryObjectOverlay<'a> {
    base: &'a BTreeMap<ContentRef, HistoryObject>,
    added: &'a [HistoryObject],
}

impl HistoryObjectLookup for HistoryObjectOverlay<'_> {
    fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.added
            .iter()
            .find(|object| &object.content_ref == content_ref)
            .or_else(|| self.base.get(content_ref))
    }
}

fn validate_lexical_binding(
    slot: &LexicalSlot,
    binding: &LexicalValueRef,
    objects: &BTreeMap<ContentRef, HistoryObject>,
    program: &VerifiedProgramData,
) -> super::Result<()> {
    if binding.slot_ref
        != slot
            .content_ref()
            .map_err(|_| invalid("lexical slot reference cannot be derived"))?
        || binding.value.contract_ref != slot.contract_ref
    {
        return Err(invalid("lexical binding differs from its certified slot"));
    }
    validate_typed_value(&binding.value, objects, program)
}

fn validate_typed_value(
    value: &TypedValueRef,
    objects: &dyn HistoryObjectLookup,
    program: &VerifiedProgramData,
) -> super::Result<()> {
    let object = objects
        .object(&value.value_ref)
        .ok_or_else(|| invalid("typed value object is absent"))?;
    validate_typed_value_object(value, object, program)
}

fn validate_typed_value_object(
    value: &TypedValueRef,
    object: &HistoryObject,
    program: &VerifiedProgramData,
) -> super::Result<()> {
    object
        .validate()
        .map_err(|_| invalid("typed value object is not exact canonical content"))?;
    if object.object_type.as_str() != TYPED_VALUE_OBJECT_TYPE
        || object.content_ref != value.value_ref
    {
        return Err(invalid("typed value object identity differs"));
    }
    let json: Value = object
        .decode()
        .map_err(|_| invalid("typed value object cannot be strictly decoded"))?;
    if typed_value_contains_secret_marker(value, &json)? {
        return Err(invalid("typed value contains a forbidden secret marker"));
    }
    if object.content_ref.schema_id() != &typed_value_schema_id(program, &value.contract_ref)? {
        return Err(invalid(
            "typed value schema differs from certified contract",
        ));
    }
    validate_value_against_contract(program, &value.contract_ref, &json, 0)
}

fn typed_value_contains_secret_marker(value: &TypedValueRef, json: &Value) -> super::Result<bool> {
    let fact_request_contract =
        mfm_spec::structured::structured_value_contract_ref::<FactSelectionRequest>()
            .map_err(|_| invalid("fact request value contract cannot be derived"))?;
    let fact_response_contract =
        mfm_spec::structured::structured_value_contract_ref::<FactSelectionReadResponse>()
            .map_err(|_| invalid("fact response value contract cannot be derived"))?;
    if value.contract_ref != fact_request_contract && value.contract_ref != fact_response_contract {
        return Ok(contains_secret_marker(json));
    }
    contains_secret_marker_in_fact_protocol(json)
}

fn contains_secret_marker_in_fact_protocol(value: &Value) -> super::Result<bool> {
    match value {
        Value::String(value) => Ok(value != "complete_through_authorization_frontier"
            && mfm_values::string_contains_secret_marker(value)),
        Value::Array(values) => values.iter().try_fold(false, |found, value| {
            Ok(found || contains_secret_marker_in_fact_protocol(value)?)
        }),
        Value::Object(values) => values.iter().try_fold(false, |found, (key, value)| {
            if found {
                return Ok(true);
            }
            let key_is_safe_protocol_identity = key == "authorization_ref";
            if !key_is_safe_protocol_identity && mfm_values::string_contains_secret_marker(key) {
                return Ok(true);
            }
            if matches!(
                key.as_str(),
                "canonical_request_base64url" | "canonical_response_base64url"
            ) {
                let Value::String(nested) = value else {
                    return Ok(true);
                };
                let nested = mfm_canonical::CanonicalBytes::from_base64url_no_pad(nested.clone())
                    .map_err(|_| invalid("nested fact base64url is invalid"))?;
                let nested: Value = serde_json::from_slice(nested.as_bytes())
                    .map_err(|_| invalid("nested fact JSON is invalid"))?;
                contains_secret_marker_in_fact_protocol(&nested)
            } else if matches!(
                key.as_str(),
                "subject_canonical_json" | "response_canonical_json" | "claim_canonical_json"
            ) {
                let Value::String(nested) = value else {
                    return Ok(true);
                };
                let nested: Value = serde_json::from_str(nested)
                    .map_err(|_| invalid("nested fact JSON is invalid"))?;
                contains_secret_marker_in_fact_protocol(&nested)
            } else {
                contains_secret_marker_in_fact_protocol(value)
            }
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(false),
    }
}

fn typed_value_schema_id(
    program: &VerifiedProgramData,
    contract_ref: &ContentRef,
) -> super::Result<SchemaId> {
    let component = certified_component(program, contract_ref)?;
    match component.object_type.as_str() {
        DATA_CONTRACT_OBJECT_TYPE => {
            let contract: RetainedValueContract =
                serde_json::from_value(component.value.as_json().clone())
                    .map_err(|_| invalid("retained value contract cannot be decoded"))?;
            let expected = mfm_spec::structured::retained_value_contract_ref(&contract)
                .map_err(|_| invalid("retained value contract identity cannot be derived"))?;
            if &expected != contract_ref {
                return Err(invalid("retained value contract identity differs"));
            }
            Ok(contract.schema_id().clone())
        }
        LANE_OUTCOME_CONTRACT_OBJECT_TYPE => {
            decode_lane_outcome_contract(program, contract_ref)?;
            Ok(contract_ref.schema_id().clone())
        }
        FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE => {
            decode_fan_out_join_contract(program, contract_ref)?;
            Ok(contract_ref.schema_id().clone())
        }
        _ => Err(invalid("typed value contract kind is not value-bearing")),
    }
}

fn validate_value_against_contract(
    program: &VerifiedProgramData,
    contract_ref: &ContentRef,
    value: &Value,
    depth: u8,
) -> super::Result<()> {
    if depth > 8 {
        return Err(invalid(
            "structural value contract nesting exceeds its bound",
        ));
    }
    let component = certified_component(program, contract_ref)?;
    match component.object_type.as_str() {
        DATA_CONTRACT_OBJECT_TYPE => {
            typed_value_schema_id(program, contract_ref)?;
            let schema = program
                .value_schema(contract_ref)
                .ok_or_else(|| invalid("typed value contract has no qualified schema identity"))?;
            let canonical = canonical_json(value)
                .map_err(|_| invalid("typed value cannot be canonicalized"))?;
            schema
                .validate_canonical_value(canonical.as_bytes())
                .map_err(|_| {
                    invalid("typed value differs from its complete certified schema shape")
                })
        }
        LANE_OUTCOME_CONTRACT_OBJECT_TYPE => {
            let (success_contract_ref, failure_contract) =
                decode_lane_outcome_contract(program, contract_ref)?;
            let object = value
                .as_object()
                .filter(|object| object.len() == 1)
                .ok_or_else(|| invalid("lane outcome is not one exact tagged value"))?;
            if let Some(success) = object.get("Success") {
                validate_value_against_contract(program, &success_contract_ref, success, depth + 1)
            } else if let Some(failure) = object.get("Failure") {
                let StructuredFailureContract::Typed { contract_ref, .. } = failure_contract else {
                    return Err(invalid("Never lane contains a failure value"));
                };
                validate_value_against_contract(program, &contract_ref, failure, depth + 1)
            } else {
                Err(invalid("lane outcome tag is not certified"))
            }
        }
        FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE => {
            let lane_contract_ref = decode_fan_out_join_contract(program, contract_ref)?;
            let object = value
                .as_object()
                .ok_or_else(|| invalid("fan-out join is not one exact object"))?;
            // Non-empty head-plus-tail representation; emptiness is unrepresentable.
            let head = object
                .get("head")
                .ok_or_else(|| invalid("fan-out join head is absent"))?;
            let tail = object
                .get("tail")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("fan-out join tail is absent"))?;
            if object.len() != 2 {
                return Err(invalid("fan-out join object has unexpected fields"));
            }
            validate_value_against_contract(program, &lane_contract_ref, head, depth + 1)?;
            for outcome in tail {
                validate_value_against_contract(program, &lane_contract_ref, outcome, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(invalid("typed value contract kind is not value-bearing")),
    }
}

fn certified_component<'a>(
    program: &'a VerifiedProgramData,
    contract_ref: &ContentRef,
) -> super::Result<&'a mfm_spec::structured::CertifiedComponentObject> {
    program
        .document()
        .component_closure
        .iter()
        .find(|component| &component.content_ref == contract_ref)
        .ok_or_else(|| invalid("typed value contract is absent from certified closure"))
}

fn decode_lane_outcome_contract(
    program: &VerifiedProgramData,
    contract_ref: &ContentRef,
) -> super::Result<(ContentRef, StructuredFailureContract)> {
    let component = certified_component(program, contract_ref)?;
    if component.object_type.as_str() != LANE_OUTCOME_CONTRACT_OBJECT_TYPE {
        return Err(invalid("lane outcome contract object type differs"));
    }
    let decoded: (ContentRef, StructuredFailureContract) =
        serde_json::from_value(component.value.as_json().clone())
            .map_err(|_| invalid("lane outcome contract cannot be decoded"))?;
    let expected = lane_outcome_contract_ref(&decoded.0, &decoded.1)
        .map_err(|_| invalid("lane outcome contract identity cannot be derived"))?;
    if &expected != contract_ref {
        return Err(invalid("lane outcome contract identity differs"));
    }
    Ok(decoded)
}

fn decode_fan_out_join_contract(
    program: &VerifiedProgramData,
    contract_ref: &ContentRef,
) -> super::Result<ContentRef> {
    let component = certified_component(program, contract_ref)?;
    if component.object_type.as_str() != FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE {
        return Err(invalid("fan-out join contract object type differs"));
    }
    let lane_contract_ref: ContentRef =
        serde_json::from_value(component.value.as_json().clone())
            .map_err(|_| invalid("fan-out join contract cannot be decoded"))?;
    let (success_contract_ref, failure_contract) =
        decode_lane_outcome_contract(program, &lane_contract_ref)?;
    let expected = fan_out_join_contract_ref(&success_contract_ref, &failure_contract)
        .map_err(|_| invalid("fan-out join contract identity cannot be derived"))?;
    if &expected != contract_ref {
        return Err(invalid("fan-out join contract identity differs"));
    }
    Ok(lane_contract_ref)
}

fn validate_program_value_schemas(program: &VerifiedProgramData) -> super::Result<()> {
    let mut retained_contracts = BTreeMap::new();
    for component in &program.document().component_closure {
        let canonical = component
            .value
            .canonical_json()
            .map_err(|_| invalid("certified component value is not canonical"))?;
        let Ok(contract) = RetainedValueContract::strict_decode(canonical.as_bytes()) else {
            continue;
        };
        let contract_ref = mfm_spec::structured::retained_value_contract_ref(&contract)
            .map_err(|_| invalid("retained value contract identity cannot be derived"))?;
        if contract_ref != component.content_ref {
            return Err(invalid(
                "retained value contract reference differs from its bytes",
            ));
        }
        if retained_contracts.insert(contract_ref, contract).is_some() {
            return Err(invalid(
                "certified closure repeats a retained value contract",
            ));
        }
    }
    if retained_contracts.len() != program.value_schemas.len()
        || retained_contracts.keys().ne(program.value_schemas.keys())
    {
        return Err(invalid(
            "qualified schema identities differ from retained contract closure",
        ));
    }
    for (contract_ref, contract) in retained_contracts {
        let schema = program
            .value_schemas
            .get(&contract_ref)
            .ok_or_else(|| invalid("qualified schema identity is absent"))?;
        if schema
            .schema_id()
            .map_err(|_| invalid("qualified schema identity is invalid"))?
            != *contract.schema_id()
            || schema.semantic_type_id.as_ref() != Some(contract.semantic_type_id())
        {
            return Err(invalid(
                "qualified schema identity differs from retained value contract",
            ));
        }
    }
    Ok(())
}

fn validate_admission_material_refs(
    admission: &RunAdmitted,
    objects: &BTreeMap<ContentRef, HistoryObject>,
    program: &VerifiedProgramData,
) -> super::Result<()> {
    let material = &admission.admission_material_refs;
    validate_admission_material_object(
        &material.configuration_ref,
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        objects,
    )?;
    validate_admission_material_object(
        &material.context_manifest_ref,
        ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
        objects,
    )?;
    validate_admission_material_object(
        &material.prior_run_source_manifest_ref,
        ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE,
        objects,
    )?;
    let source_manifest = objects
        .get(&material.prior_run_source_manifest_ref)
        .ok_or_else(|| invalid("admission prior-run source manifest is absent"))?;
    PriorRunFactSourceManifest::from_history_object(source_manifest)
        .map_err(|_| invalid("admission prior-run source manifest contract differs"))?;
    validate_admission_material_object(
        &material.routing_policy_ref,
        ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
        objects,
    )?;
    let expected_lineages = expected_stable_resource_lineage_contract_refs(program, objects)?;
    if material.stable_resource_lineage_contract_refs != expected_lineages {
        return Err(invalid(
            "admission stable resource lineages differ from the certified program",
        ));
    }
    let mut refs = vec![
        &material.configuration_ref,
        &material.context_manifest_ref,
        &material.prior_run_source_manifest_ref,
        &material.routing_policy_ref,
    ];
    refs.extend(material.stable_resource_lineage_contract_refs.iter());
    refs.sort();
    if refs.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid(
            "one admission object is reused for different material roles",
        ));
    }
    Ok(())
}

fn expected_stable_resource_lineage_contract_refs(
    program: &VerifiedProgramData,
    objects: &BTreeMap<ContentRef, HistoryObject>,
) -> super::Result<Vec<ContentRef>> {
    let mut lineage_refs = BTreeSet::new();
    collect_block_resource_lineages(program, &program.expanded().root, &mut lineage_refs)?;
    for lineage_ref in &lineage_refs {
        validate_resource_lineage_contract(program.document(), objects, lineage_ref)?;
    }
    Ok(lineage_refs.into_iter().collect())
}

fn collect_block_resource_lineages(
    program: &VerifiedProgramData,
    block: &ExpandedBlock,
    lineage_refs: &mut BTreeSet<ContentRef>,
) -> super::Result<()> {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state) => {
                collect_state_resource_lineages(program, state, lineage_refs)?;
            }
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    collect_block_resource_lineages(program, &arm.body, lineage_refs)?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    collect_block_resource_lineages(program, &lane.body, lineage_refs)?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                collect_block_resource_lineages(program, &fragment.body, lineage_refs)?;
                collect_failure_boundary_resource_lineages(
                    program,
                    &fragment.failure_boundary,
                    lineage_refs,
                )?;
            }
        }
    }
    Ok(())
}

fn collect_state_resource_lineages(
    program: &VerifiedProgramData,
    state: &ExpandedStateBinding,
    lineage_refs: &mut BTreeSet<ContentRef>,
) -> super::Result<()> {
    if let Some(lineage_ref) = state_stable_resource_lineage_contract_ref(program, state)? {
        lineage_refs.insert(lineage_ref);
    }
    collect_failure_boundary_resource_lineages(program, &state.failure_boundary, lineage_refs)
}

fn collect_failure_boundary_resource_lineages(
    program: &VerifiedProgramData,
    boundary: &CertifiedFailureBoundary,
    lineage_refs: &mut BTreeSet<ContentRef>,
) -> super::Result<()> {
    let CertifiedFailureBoundary::Typed { plan, .. } = boundary else {
        return Ok(());
    };
    match plan.as_ref() {
        FailurePlan::Handled {
            before_handler,
            handler,
            continuation,
            ..
        } => {
            collect_block_resource_lineages(program, before_handler, lineage_refs)?;
            collect_state_resource_lineages(program, handler, lineage_refs)?;
            if let HandlerContinuation::CustomRecovery { arms, .. } = continuation.as_ref() {
                for arm in arms {
                    collect_block_resource_lineages(program, &arm.body, lineage_refs)?;
                }
            }
        }
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            ..
        } => {
            collect_block_resource_lineages(program, before_boundary, lineage_refs)?;
            for link in mapping_chain {
                collect_state_resource_lineages(program, &link.mapper, lineage_refs)?;
            }
        }
    }
    Ok(())
}

fn state_stable_resource_lineage_contract_ref(
    program: &VerifiedProgramData,
    state: &ExpandedStateBinding,
) -> super::Result<Option<ContentRef>> {
    let Some(capability_ref) = state.contract.execution.capability_contract_ref() else {
        return Ok(None);
    };
    let capability: StructuredLiveComponentContract =
        decode_component(program.document(), capability_ref)?;
    capability
        .validate()
        .map_err(|_| invalid("certified capability contract is invalid"))?;
    if capability.component_kind != StructuredComponentKind::Capability
        || capability
            .content_ref()
            .map_err(|_| invalid("certified capability identity cannot be derived"))?
            != *capability_ref
    {
        return Err(invalid("certified capability object identity differs"));
    }
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(|| invalid("certified capability protocol is absent"))?;
    match (state.contract.execution.kind(), protocol) {
        (StructuredExecutionKind::Read, StructuredCapabilityProtocolContract::Read { .. })
        | (
            StructuredExecutionKind::Effect,
            StructuredCapabilityProtocolContract::Effect {
                refresh_contract: StructuredEffectRefreshContract::NoRefresh {},
                ..
            },
        ) => Ok(None),
        (
            StructuredExecutionKind::Effect,
            StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        resource_lineage_contract_ref,
                        ..
                    },
                ..
            },
        ) => Ok(Some(resource_lineage_contract_ref.as_ref().clone())),
        _ => Err(invalid(
            "state execution kind differs from its capability protocol",
        )),
    }
}

fn validate_resource_lineage_contract(
    document: &CertifiedProgramDocument,
    objects: &BTreeMap<ContentRef, HistoryObject>,
    lineage_ref: &ContentRef,
) -> super::Result<()> {
    let manifest: StateCapabilityAdapterSignerResourceManifest = decode_component(
        document,
        &document
            .root
            .components
            .state_capability_adapter_signer_resource_manifest_closure_ref,
    )?;
    let mut entries = manifest
        .entries
        .iter()
        .filter(|entry| &entry.semantic_contract_ref == lineage_ref);
    let entry = entries
        .next()
        .ok_or_else(|| invalid("resource lineage is absent from the certified manifest"))?;
    if entry.component_kind != StructuredComponentKind::Resource || entries.next().is_some() {
        return Err(invalid(
            "resource lineage does not have one exact Resource manifest entry",
        ));
    }

    let component = document
        .component_closure
        .iter()
        .find(|component| &component.content_ref == lineage_ref)
        .ok_or_else(|| invalid("resource lineage contract object is absent"))?;
    if component.object_type.as_str() != RESOURCE_CONTRACT_OBJECT_TYPE {
        return Err(invalid("resource lineage contract object type differs"));
    }
    let contract: StructuredLiveComponentContract =
        serde_json::from_value(component.value.as_json().clone())
            .map_err(|_| invalid("resource lineage contract cannot be decoded"))?;
    contract
        .validate()
        .map_err(|_| invalid("resource lineage contract is invalid"))?;
    if contract.component_kind != StructuredComponentKind::Resource
        || contract
            .content_ref()
            .map_err(|_| invalid("resource lineage contract identity cannot be derived"))?
            != *lineage_ref
    {
        return Err(invalid("resource lineage contract identity differs"));
    }
    validate_admission_material_object(lineage_ref, RESOURCE_CONTRACT_OBJECT_TYPE, objects)
}

fn validate_admission_material_object(
    content_ref: &ContentRef,
    expected_type: &str,
    objects: &BTreeMap<ContentRef, HistoryObject>,
) -> super::Result<()> {
    let object = objects
        .get(content_ref)
        .ok_or_else(|| invalid("admission material object is absent"))?;
    object
        .validate()
        .map_err(|_| invalid("admission material object is invalid"))?;
    if object.object_type.as_str() != expected_type {
        return Err(invalid("admission material object type differs"));
    }
    let value: Value = object
        .decode()
        .map_err(|_| invalid("admission material object cannot be strictly decoded"))?;
    if contains_secret_marker(&value) {
        return Err(invalid(
            "admission material contains a forbidden secret marker",
        ));
    }
    Ok(())
}

fn contains_secret_marker(value: &Value) -> bool {
    match value {
        Value::String(value) => mfm_values::string_contains_secret_marker(value),
        Value::Array(values) => values.iter().any(contains_secret_marker),
        Value::Object(values) => values.iter().any(|(key, value)| {
            mfm_values::string_contains_secret_marker(key) || contains_secret_marker(value)
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

#[derive(Serialize)]
struct AccessAttemptPreimage<'a> {
    run_id: &'a RunId,
    occurrence_id: &'a OccurrenceId,
    occurrence_path_ref: &'a ContentRef,
    semantic_call_id: &'a mfm_ids::SemanticCallId,
    state_input_ref: &'a LexicalValueRef,
    attempt_ordinal: u64,
    access_kind: AccessKind,
    semantic_head: &'a SemanticHead,
    capability_contract_ref: &'a ContentRef,
    capability_implementation_ref: &'a ContentRef,
    adapter_contract_ref: &'a ContentRef,
    adapter_implementation_ref: &'a ContentRef,
    request: &'a TypedValueRef,
    request_digest: &'a RequestDigest,
    physical_binding_ref: &'a ContentRef,
    stable_resource_lineage_contract_ref: &'a Option<ContentRef>,
}

#[allow(clippy::too_many_arguments)]
fn validate_authorization(
    record: &ExternalAccessAuthorized,
    state: &ActionableState,
    run_id: &RunId,
    semantic_head: &SemanticHead,
    program: &VerifiedProgramData,
    admission: &RunAdmitted,
    objects: &BTreeMap<ContentRef, HistoryObject>,
    bindings: &BTreeMap<ContentRef, LexicalValueRef>,
    occurrence_attempts: &BTreeMap<OccurrenceId, Vec<AccessAttemptId>>,
    authorizations: &BTreeMap<AccessAttemptId, RecordedAuthorization>,
    observations: &BTreeMap<AccessAttemptId, RecordedObservation>,
    physical_binding_verification_mode: PhysicalBindingVerificationMode,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<bool> {
    let document = program.document();
    let expanded = program.expanded();
    if record.occurrence_id != state.occurrence_id
        || record.occurrence_path_ref
            != state
                .occurrence_path
                .content_ref()
                .map_err(|_| invalid("authorization path reference cannot be derived"))?
        || &record.semantic_head != semantic_head
    {
        return Err(invalid(
            "authorization does not target the exact current cursor",
        ));
    }
    if record.state_input_ref != state.input {
        return Err(invalid(
            "authorization state input differs from the current producer binding",
        ));
    }
    let state_binding = find_state(expanded, &record.occurrence_id)?;
    if state_binding.semantic_call_id != record.semantic_call_id {
        return Err(invalid(
            "authorization semantic call differs from current state",
        ));
    }
    let expected_kind = match state_binding.contract.execution.kind() {
        StructuredExecutionKind::Read => AccessKind::Read,
        StructuredExecutionKind::Effect => AccessKind::Effect,
        StructuredExecutionKind::Pure => {
            return Err(invalid("Pure state cannot authorize external access"));
        }
    };
    if record.access_kind != expected_kind {
        return Err(invalid(
            "authorization access kind differs from certified state",
        ));
    }
    let expected_ordinal = match &state.leaf {
        StateLeaf::Ready => 0,
        StateLeaf::Refreshable {
            next_attempt_ordinal,
            ..
        } => *next_attempt_ordinal,
        _ => {
            return Err(invalid(
                "current access leaf cannot authorize another attempt",
            ))
        }
    };
    if record.attempt_ordinal != expected_ordinal {
        return Err(invalid(
            "authorization attempt ordinal differs from folded ordinal",
        ));
    }
    if occurrence_attempts
        .get(&record.occurrence_id)
        .is_some_and(|attempts| {
            attempts.iter().any(|attempt| {
                authorizations.contains_key(attempt) && !observations.contains_key(attempt)
            })
        })
    {
        return Err(invalid(
            "occurrence already has an unresolved authorization",
        ));
    }
    let capability_ref = state_binding
        .contract
        .execution
        .capability_contract_ref()
        .ok_or_else(|| invalid("access state has no capability contract"))?;
    if capability_ref != &record.capability_contract_ref {
        return Err(invalid(
            "authorization capability differs from certified state",
        ));
    }
    let capability: StructuredLiveComponentContract = decode_component(document, capability_ref)?;
    let [adapter_dependency] = capability.dependencies.as_slice() else {
        return Err(invalid(
            "capability does not have one exact adapter dependency",
        ));
    };
    if adapter_dependency.component_kind != StructuredComponentKind::Adapter
        || adapter_dependency.contract_ref != record.adapter_contract_ref
    {
        return Err(invalid(
            "authorization adapter differs from capability contract",
        ));
    }
    let implementation_manifest: SecretFreeImplementationManifest = decode_component(
        document,
        &document
            .root
            .components
            .secret_free_implementation_manifest_closure_ref,
    )?;
    let expected_capability_implementation = implementation_manifest
        .entries
        .iter()
        .find(|entry| {
            entry.component_kind == StructuredComponentKind::Capability
                && entry.semantic_contract_ref == record.capability_contract_ref
        })
        .map(|entry| &entry.implementation_contract_ref)
        .ok_or_else(|| invalid("capability implementation is absent from manifest"))?;
    let expected_adapter_implementation = implementation_manifest
        .entries
        .iter()
        .find(|entry| {
            entry.component_kind == StructuredComponentKind::Adapter
                && entry.semantic_contract_ref == record.adapter_contract_ref
        })
        .map(|entry| &entry.implementation_contract_ref)
        .ok_or_else(|| invalid("adapter implementation is absent from manifest"))?;
    if expected_capability_implementation != &record.capability_implementation_ref
        || expected_adapter_implementation != &record.adapter_implementation_ref
    {
        return Err(invalid(
            "authorization implementation differs from certified manifest",
        ));
    }
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(|| invalid("capability protocol is absent"))?;
    if protocol.request_contract_ref() != &record.request.contract_ref {
        return Err(invalid(
            "authorization request contract differs from capability",
        ));
    }
    validate_typed_value(&record.request, objects, program)?;
    let request_object = objects
        .get(&record.request.value_ref)
        .ok_or_else(|| invalid("authorization request object is absent"))?;
    let expected_request_digest = RequestDigest::from_digest(sha256_digest_bytes(
        request_object.canonical_json.as_bytes(),
    ));
    if record.request_digest != expected_request_digest {
        return Err(invalid(
            "authorization request digest differs from exact bytes",
        ));
    }
    let expected_lineage = state.stable_resource_lineage_contract_ref.as_ref();
    if record.stable_resource_lineage_contract_ref.as_ref() != expected_lineage {
        return Err(invalid(
            "authorization refresh lineage differs from capability",
        ));
    }
    if expected_lineage.is_some_and(|lineage| {
        admission
            .admission_material_refs
            .stable_resource_lineage_contract_refs
            .binary_search(lineage)
            .is_err()
    }) {
        return Err(invalid(
            "authorization refresh lineage was not admitted for this run",
        ));
    }
    let binding_object = objects
        .get(&record.physical_binding_ref)
        .ok_or_else(|| invalid("authorization physical binding certificate is absent"))?;
    binding_object
        .validate()
        .map_err(|_| invalid("authorization physical binding certificate is invalid"))?;
    let minimum_lineage_head_ref = match &state.leaf {
        StateLeaf::Refreshable {
            public_lineage_head_ref,
            ..
        } => Some(public_lineage_head_ref),
        _ => None,
    };
    if record.store_scope_id != admission.store_scope_id
        || record.store_epoch != admission.store_epoch
        || record.tenant_scope_id != admission.tenant_scope_id
        || record.admitted_routing_policy_ref
            != admission.admission_material_refs.routing_policy_ref
        || record.minimum_lineage_head_ref.as_ref() != minimum_lineage_head_ref
    {
        return Err(invalid(
            "authorization lineage scope differs from admission",
        ));
    }
    let previous_physical_binding_ref = match &state.leaf {
        StateLeaf::Refreshable { .. } => {
            let previous_attempt = occurrence_attempts
                .get(&record.occurrence_id)
                .and_then(|attempts| attempts.last())
                .ok_or_else(|| invalid("refresh authorization has no preceding attempt"))?;
            let previous_authorization = authorizations
                .get(previous_attempt)
                .ok_or_else(|| invalid("refresh authorization predecessor is absent"))?;
            if !observations.contains_key(previous_attempt) {
                return Err(invalid(
                    "refresh authorization predecessor has no observation",
                ));
            }
            Some(&previous_authorization.record.physical_binding_ref)
        }
        _ => None,
    };
    let is_prior_run_fact_selection = validate_prior_run_fact_selection_authorization(
        record,
        &capability,
        admission,
        request_object,
        binding_object,
    )?;
    if !is_prior_run_fact_selection {
        physical_binding_verifier.verify_authorization(
            &PhysicalBindingAuthorization {
                verification_mode: physical_binding_verification_mode,
                access_kind: record.access_kind,
                capability_contract_ref: &record.capability_contract_ref,
                capability_implementation_ref: &record.capability_implementation_ref,
                adapter_contract_ref: &record.adapter_contract_ref,
                adapter_implementation_ref: &record.adapter_implementation_ref,
                admitted_routing_policy_ref: &admission.admission_material_refs.routing_policy_ref,
                stable_resource_lineage_contract_ref: record
                    .stable_resource_lineage_contract_ref
                    .as_ref(),
                minimum_lineage_head_ref,
                previous_physical_binding_ref,
            },
            binding_object,
        )?;
    }
    let expected_attempt = derive_access_attempt_id(&AccessAttemptPreimage {
        run_id,
        occurrence_id: &record.occurrence_id,
        occurrence_path_ref: &record.occurrence_path_ref,
        semantic_call_id: &record.semantic_call_id,
        state_input_ref: &record.state_input_ref,
        attempt_ordinal: record.attempt_ordinal,
        access_kind: record.access_kind,
        semantic_head: &record.semantic_head,
        capability_contract_ref: &record.capability_contract_ref,
        capability_implementation_ref: &record.capability_implementation_ref,
        adapter_contract_ref: &record.adapter_contract_ref,
        adapter_implementation_ref: &record.adapter_implementation_ref,
        request: &record.request,
        request_digest: &record.request_digest,
        physical_binding_ref: &record.physical_binding_ref,
        stable_resource_lineage_contract_ref: &record.stable_resource_lineage_contract_ref,
    })
    .map_err(|_| invalid("access attempt identity cannot be derived"))?;
    if record.access_attempt_id != expected_attempt {
        return Err(invalid("authorization access attempt identity differs"));
    }
    let [input_slot] = state_binding.inputs.as_slice() else {
        return Err(invalid("access state does not have one exact input"));
    };
    let input_ref = input_slot
        .content_ref()
        .map_err(|_| invalid("access state input slot reference cannot be derived"))?;
    if !bindings.contains_key(&input_ref) {
        return Err(invalid("authorization state input is not live"));
    }
    Ok(is_prior_run_fact_selection)
}

fn validate_prior_run_fact_selection_authorization(
    record: &ExternalAccessAuthorized,
    capability: &StructuredLiveComponentContract,
    admission: &RunAdmitted,
    request_object: &HistoryObject,
    binding_object: &HistoryObject,
) -> super::Result<bool> {
    let expected_capability = prior_run_fact_selection_capability_contract()
        .map_err(|_| invalid("prior-run fact selection contract cannot be derived"))?;
    let expected_capability_ref = expected_capability
        .content_ref()
        .map_err(|_| invalid("prior-run fact selection identity cannot be derived"))?;
    if record.capability_contract_ref != expected_capability_ref {
        return Ok(false);
    }
    let expected_adapter_ref = prior_run_fact_scanner_adapter_contract()
        .and_then(|contract| contract.content_ref())
        .map_err(|_| invalid("prior-run fact scanner identity cannot be derived"))?;
    if record.access_kind != AccessKind::Read
        || capability != &expected_capability
        || record.adapter_contract_ref != expected_adapter_ref
        || record.stable_resource_lineage_contract_ref.is_some()
    {
        return Err(invalid(
            "prior-run fact selection authorization differs from its reserved contract",
        ));
    }
    let request: FactSelectionRequest = request_object
        .decode()
        .map_err(|_| invalid("prior-run fact selection request cannot be decoded"))?;
    if request
        .admitted_source_manifest_ref()
        .map_err(|_| invalid("prior-run fact selection source cannot be decoded"))?
        != admission
            .admission_material_refs
            .prior_run_source_manifest_ref
        || request
            .selector_contract_ref()
            .map_err(|_| invalid("prior-run fact selection selector cannot be decoded"))?
            != prior_run_fact_selector_contract_ref()
                .map_err(|_| invalid("prior-run fact selector identity cannot be derived"))?
    {
        return Err(invalid(
            "prior-run fact selection request exceeds admitted authority",
        ));
    }
    let certificate = PriorRunFactScannerBindingCertificate::from_history_object(binding_object)
        .map_err(|_| invalid("prior-run fact scanner binding certificate is invalid"))?;
    if certificate.store_scope_id() != &admission.store_scope_id
        || certificate.store_epoch() != admission.store_epoch
        || certificate.tenant_scope_id() != &admission.tenant_scope_id
        || certificate.admitted_source_manifest_ref()
            != &admission
                .admission_material_refs
                .prior_run_source_manifest_ref
        || certificate.selector_contract_ref()
            != &prior_run_fact_selector_contract_ref()
                .map_err(|_| invalid("prior-run fact selector identity cannot be derived"))?
        || certificate.capability_contract_ref() != &record.capability_contract_ref
        || certificate.capability_implementation_ref() != &record.capability_implementation_ref
        || certificate.adapter_contract_ref() != &record.adapter_contract_ref
        || certificate.adapter_implementation_ref() != &record.adapter_implementation_ref
    {
        return Err(invalid(
            "prior-run fact scanner binding certificate differs from authorization",
        ));
    }
    Ok(true)
}

fn validate_proposed_supersession_physical(
    observation: &ExternalAccessObserved,
    authorization: &RecordedAuthorization,
    program: &VerifiedProgramData,
    objects: &dyn HistoryObjectLookup,
    physical_binding_verification_mode: PhysicalBindingVerificationMode,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<()> {
    let ObservationOutcome::SupersededBeforeEntry {
        public_lineage_head_ref,
        evidence_ref,
    } = &observation.outcome
    else {
        return Err(invalid("proposed observation is not a supersession"));
    };
    let capability: StructuredLiveComponentContract = decode_component(
        program.document(),
        &authorization.record.capability_contract_ref,
    )?;
    let Some(StructuredCapabilityProtocolContract::Effect {
        refresh_contract:
            StructuredEffectRefreshContract::Refreshable {
                resource_lineage_contract_ref,
                ..
            },
        ..
    }) = capability.capability_protocol.as_ref()
    else {
        return Err(invalid("non-refreshable access proposed supersession"));
    };
    validate_supersession_physical(
        authorization,
        resource_lineage_contract_ref,
        public_lineage_head_ref,
        evidence_ref,
        objects,
        physical_binding_verification_mode,
        physical_binding_verifier,
    )
}

fn validate_supersession_physical(
    authorization: &RecordedAuthorization,
    resource_lineage_contract_ref: &ContentRef,
    public_lineage_head_ref: &ContentRef,
    evidence_ref: &ContentRef,
    objects: &dyn HistoryObjectLookup,
    physical_binding_verification_mode: PhysicalBindingVerificationMode,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<()> {
    if authorization.record.access_kind != AccessKind::Effect
        || authorization
            .record
            .stable_resource_lineage_contract_ref
            .as_ref()
            != Some(resource_lineage_contract_ref)
    {
        return Err(invalid("supersession lineage differs from authorization"));
    }
    let public_lineage_head = objects
        .object(public_lineage_head_ref)
        .ok_or_else(|| invalid("supersession public lineage head is absent"))?;
    public_lineage_head
        .validate()
        .map_err(|_| invalid("supersession public lineage head is invalid"))?;
    let evidence_object = objects
        .object(evidence_ref)
        .ok_or_else(|| invalid("supersession evidence object is absent"))?;
    physical_binding_verifier.verify_supersession(
        &PhysicalBindingSupersession {
            verification_mode: physical_binding_verification_mode,
            capability_contract_ref: &authorization.record.capability_contract_ref,
            adapter_contract_ref: &authorization.record.adapter_contract_ref,
            adapter_implementation_ref: &authorization.record.adapter_implementation_ref,
            authorized_binding_ref: &authorization.record.physical_binding_ref,
            stable_resource_lineage_contract_ref: resource_lineage_contract_ref,
            public_lineage_head_ref,
        },
        public_lineage_head,
        evidence_object,
    )
}

fn validate_observation(
    observation: &ExternalAccessObserved,
    authorization: &RecordedAuthorization,
    program: &VerifiedProgramData,
    objects: &dyn HistoryObjectLookup,
    physical_binding_verification_mode: PhysicalBindingVerificationMode,
    physical_binding_verifier: &dyn PublicPhysicalBindingVerifier,
) -> super::Result<()> {
    let document = program.document();
    if observation.authorization_ref != authorization.record_ref
        || observation.access_attempt_id != authorization.record.access_attempt_id
    {
        return Err(invalid("observation linkage differs from authorization"));
    }
    let capability: StructuredLiveComponentContract =
        decode_component(document, &authorization.record.capability_contract_ref)?;
    let protocol = capability
        .capability_protocol
        .as_ref()
        .ok_or_else(|| invalid("observed capability protocol is absent"))?;
    match &observation.outcome {
        ObservationOutcome::Returned { value } => {
            if value.contract_ref != *protocol.returned_contract_ref() {
                return Err(invalid("returned observation contract differs"));
            }
            validate_typed_value(value, objects, program)
        }
        ObservationOutcome::SafeFailure { value } => {
            if value.contract_ref != *protocol.safe_failure_contract_ref() {
                return Err(invalid("safe-failure observation contract differs"));
            }
            validate_typed_value(value, objects, program)
        }
        ObservationOutcome::SupersededBeforeEntry {
            public_lineage_head_ref,
            evidence_ref,
        } => {
            let StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        refresh_evidence_contract_ref,
                        resource_lineage_contract_ref,
                    },
                ..
            } = protocol
            else {
                return Err(invalid("non-refreshable access observed supersession"));
            };
            let evidence = TypedValueRef {
                contract_ref: refresh_evidence_contract_ref.clone(),
                value_ref: evidence_ref.clone(),
            };
            validate_typed_value(&evidence, objects, program)?;
            validate_supersession_physical(
                authorization,
                resource_lineage_contract_ref,
                public_lineage_head_ref,
                evidence_ref,
                objects,
                physical_binding_verification_mode,
                physical_binding_verifier,
            )
        }
        ObservationOutcome::EntryUnknown { .. } => {
            if authorization.record.access_kind != AccessKind::Effect {
                return Err(invalid("Read observation cannot be EntryUnknown"));
            }
            Ok(())
        }
        ObservationOutcome::IntegrityFault { .. } => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_transition(
    record: &StateTransitionCommitted,
    state: &ActionableState,
    _semantic_head: &SemanticHead,
    program: &VerifiedProgramData,
    objects: &BTreeMap<ContentRef, HistoryObject>,
    bindings: &BTreeMap<ContentRef, LexicalValueRef>,
    authorizations: &BTreeMap<AccessAttemptId, RecordedAuthorization>,
    observations: &BTreeMap<AccessAttemptId, RecordedObservation>,
) -> super::Result<()> {
    let expanded = program.expanded();
    if record.occurrence_id != state.occurrence_id
        || record.occurrence_path_ref
            != state
                .occurrence_path
                .content_ref()
                .map_err(|_| invalid("transition path reference cannot be derived"))?
    {
        return Err(invalid(
            "transition does not target the minimum actionable state",
        ));
    }
    let state_binding = find_state(expanded, &record.occurrence_id)?;
    let [input_slot] = state_binding.inputs.as_slice() else {
        return Err(invalid("transition state does not have one exact input"));
    };
    let input_ref = input_slot
        .content_ref()
        .map_err(|_| invalid("transition input slot reference cannot be derived"))?;
    let expected_input = bindings
        .get(&input_ref)
        .ok_or_else(|| invalid("transition input is not a live binding"))?;
    if &record.input != expected_input {
        return Err(invalid("transition input differs from folded state frame"));
    }
    match (&state.execution_kind, &state.leaf) {
        (StructuredExecutionKind::Pure, StateLeaf::Ready) => {
            if record.consumed_observation_ref.is_some() {
                return Err(invalid("Pure transition consumes an observation"));
            }
        }
        (
            StructuredExecutionKind::Read | StructuredExecutionKind::Effect,
            StateLeaf::ObservedForSettlement {
                access_attempt_id,
                observation_ref,
            },
        ) => {
            if record.consumed_observation_ref.as_ref() != Some(observation_ref) {
                return Err(invalid("access transition consumes the wrong observation"));
            }
            let authorization = authorizations
                .get(access_attempt_id)
                .ok_or_else(|| invalid("settlement authorization is absent"))?;
            let observation = observations
                .get(access_attempt_id)
                .ok_or_else(|| invalid("settlement observation is absent"))?;
            if observation.record_ref != *observation_ref
                || authorization.record.access_kind
                    != match state.execution_kind {
                        StructuredExecutionKind::Read => AccessKind::Read,
                        StructuredExecutionKind::Effect => AccessKind::Effect,
                        StructuredExecutionKind::Pure => {
                            return Err(invalid("Pure state reached access settlement"));
                        }
                    }
            {
                return Err(invalid("settlement access linkage differs"));
            }
        }
        _ => return Err(invalid("transition targets a non-settleable state leaf")),
    }
    validate_transition_shape(record, state_binding, expected_input, program, objects)
}

fn validate_transition_shape(
    record: &StateTransitionCommitted,
    state: &ExpandedStateBinding,
    expected_input: &LexicalValueRef,
    program: &VerifiedProgramData,
    objects: &BTreeMap<ContentRef, HistoryObject>,
) -> super::Result<()> {
    if record.semantic_call_id != state.semantic_call_id
        || record.occurrence_path_ref
            != state
                .occurrence_path
                .content_ref()
                .map_err(|_| invalid("state path reference cannot be derived"))?
        || &record.input != expected_input
    {
        return Err(invalid("transition state identity or input differs"));
    }
    match &record.outcome {
        StateOutcomeRef::Success(value) => {
            validate_lexical_binding(&state.output_slot, value, objects, program)?;
        }
        StateOutcomeRef::Failure(value) => {
            let CertifiedFailureBoundary::Typed {
                failure_contract,
                source_slot,
                ..
            } = &state.failure_boundary
            else {
                return Err(invalid("Never state committed a typed failure"));
            };
            if !matches!(
                failure_contract.as_ref(),
                StructuredFailureContract::Typed { .. }
            ) {
                return Err(invalid("typed failure boundary has no typed contract"));
            }
            validate_lexical_binding(source_slot, value, objects, program)?;
        }
    }
    validate_committed_facts(record, state, program, objects)?;
    if !objects.contains_key(&record.outcome_ref) {
        return Err(invalid("nominal state outcome object is absent"));
    }
    Ok(())
}

#[derive(Serialize)]
struct StructuredFactClaimPreimage<'a> {
    descriptor_ref: &'a ContentRef,
    subject: &'a TypedValueRef,
    response: &'a TypedValueRef,
}

fn validate_committed_facts(
    record: &StateTransitionCommitted,
    state: &ExpandedStateBinding,
    program: &VerifiedProgramData,
    objects: &BTreeMap<ContentRef, HistoryObject>,
) -> super::Result<()> {
    if matches!(record.outcome, StateOutcomeRef::Failure(_)) && !record.facts.is_empty() {
        return Err(invalid("failed state transition emits facts"));
    }
    let mut facts = record.facts.iter().peekable();
    let mut emission_ordinal = 0_u32;
    for slot in &state.contract.fact_slots {
        let mut slot_count = 0_u32;
        while facts
            .peek()
            .is_some_and(|fact| fact.fact_slot_ordinal == slot.fact_slot_ordinal())
        {
            if slot_count == slot.maximum_emissions() {
                return Err(invalid("fact slot exceeds its certified maximum"));
            }
            let fact = facts
                .next()
                .ok_or_else(|| invalid("fact slot iterator disappeared"))?;
            if fact.emission_ordinal != emission_ordinal
                || fact.descriptor_ref != *slot.fact_descriptor_ref()
            {
                return Err(invalid(
                    "fact ordinal or descriptor differs from certified slot",
                ));
            }
            let subject_contract_ref =
                mfm_spec::structured::retained_value_contract_ref(slot.subject_contract())
                    .map_err(|_| invalid("fact subject contract reference cannot be derived"))?;
            let response_contract_ref =
                mfm_spec::structured::retained_value_contract_ref(slot.response_contract())
                    .map_err(|_| invalid("fact response contract reference cannot be derived"))?;
            if fact.subject.contract_ref != subject_contract_ref
                || fact.response.contract_ref != response_contract_ref
            {
                return Err(invalid("fact value contracts differ from certified slot"));
            }
            validate_typed_value(&fact.subject, objects, program)?;
            validate_typed_value(&fact.response, objects, program)?;
            let claim_value = StructuredFactClaimPreimage {
                descriptor_ref: &fact.descriptor_ref,
                subject: &fact.subject,
                response: &fact.response,
            };
            let canonical = canonical_json(&claim_value)
                .map_err(|_| invalid("fact claim cannot be canonicalized"))?;
            let expected = HistoryObject::new(
                StableId::new(FACT_CLAIM_OBJECT_TYPE)
                    .map_err(|_| invalid("fact claim object type is invalid"))?,
                framework_schema_id("mfm.structured-fact-claim")?,
                canonical.as_str(),
            )
            .map_err(|_| invalid("fact claim object cannot be constructed"))?;
            if expected.content_ref != fact.claim_ref
                || objects.get(&fact.claim_ref) != Some(&expected)
            {
                return Err(invalid("fact claim closure differs from committed values"));
            }
            slot_count = slot_count
                .checked_add(1)
                .ok_or_else(|| invalid("fact slot count overflowed"))?;
            emission_ordinal = emission_ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("fact emission ordinal overflowed"))?;
        }
        if slot_count < slot.minimum_emissions() {
            return Err(invalid("fact slot is below its certified minimum"));
        }
    }
    if facts.next().is_some() {
        return Err(invalid("transition contains an undeclared fact slot"));
    }
    Ok(())
}

fn decode_component<T: serde::de::DeserializeOwned>(
    document: &CertifiedProgramDocument,
    content_ref: &ContentRef,
) -> super::Result<T> {
    let component = document
        .component_closure
        .iter()
        .find(|component| &component.content_ref == content_ref)
        .ok_or_else(|| invalid("certified component is absent"))?;
    serde_json::from_value(component.value.as_json().clone())
        .map_err(|_| invalid("certified component cannot be decoded"))
}

fn find_state<'a>(
    expanded: &'a ExpandedStructuredProgram,
    occurrence_id: &OccurrenceId,
) -> super::Result<&'a ExpandedStateBinding> {
    find_state_in_block(&expanded.root, occurrence_id)
        .ok_or_else(|| invalid("record occurrence is absent from expanded program"))
}

fn find_state_in_block<'a>(
    block: &'a ExpandedBlock,
    occurrence_id: &OccurrenceId,
) -> Option<&'a ExpandedStateBinding> {
    block
        .declarations
        .iter()
        .find_map(|declaration| find_state_in_declaration(declaration, occurrence_id))
}

fn find_state_in_declaration<'a>(
    declaration: &'a ExpandedDeclaration,
    occurrence_id: &OccurrenceId,
) -> Option<&'a ExpandedStateBinding> {
    match declaration {
        ExpandedDeclaration::State(state) => {
            if &state.occurrence_id == occurrence_id {
                Some(state)
            } else {
                find_state_in_failure_boundary(&state.failure_boundary, occurrence_id)
            }
        }
        ExpandedDeclaration::Match(binding) => binding
            .arms
            .iter()
            .find_map(|arm| find_state_in_block(&arm.body, occurrence_id)),
        ExpandedDeclaration::FanOut(group) => group
            .lanes
            .iter()
            .find_map(|lane| find_state_in_block(&lane.body, occurrence_id)),
        ExpandedDeclaration::Fragment(fragment) => {
            find_state_in_block(&fragment.body, occurrence_id).or_else(|| {
                find_state_in_failure_boundary(&fragment.failure_boundary, occurrence_id)
            })
        }
    }
}

fn find_state_in_failure_boundary<'a>(
    boundary: &'a CertifiedFailureBoundary,
    occurrence_id: &OccurrenceId,
) -> Option<&'a ExpandedStateBinding> {
    let CertifiedFailureBoundary::Typed { plan, .. } = boundary else {
        return None;
    };
    match plan.as_ref() {
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            ..
        } => find_state_in_block(before_boundary, occurrence_id).or_else(|| {
            mapping_chain.iter().find_map(|link| {
                if &link.mapper.occurrence_id == occurrence_id {
                    Some(link.mapper.as_ref())
                } else {
                    find_state_in_failure_boundary(&link.mapper.failure_boundary, occurrence_id)
                }
            })
        }),
        FailurePlan::Handled {
            before_handler,
            handler,
            continuation,
            ..
        } => find_state_in_block(before_handler, occurrence_id)
            .or_else(|| (&handler.occurrence_id == occurrence_id).then_some(handler.as_ref()))
            .or_else(|| find_state_in_failure_boundary(&handler.failure_boundary, occurrence_id))
            .or_else(|| match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation { .. } => None,
                HandlerContinuation::CustomRecovery { arms, .. } => arms
                    .iter()
                    .find_map(|arm| find_state_in_block(&arm.body, occurrence_id)),
            }),
    }
}

#[derive(Serialize)]
struct SemanticStatePreimage<'a> {
    certified_program_ref: &'a ContentRef,
    transitions: Vec<SemanticTransitionPreimage<'a>>,
    live_bindings: Vec<&'a LexicalValueRef>,
}

#[derive(Serialize)]
struct SemanticTransitionPreimage<'a> {
    occurrence_id: &'a OccurrenceId,
    outcome_ref: &'a ContentRef,
    facts: &'a [mfm_journal::structured::CommittedFactRef],
}

fn semantic_state_digest(
    certified_program_ref: &ContentRef,
    transitions: &BTreeMap<OccurrenceId, RecordedTransition>,
    bindings: &BTreeMap<ContentRef, LexicalValueRef>,
) -> super::Result<RunSemanticStateDigest> {
    let preimage = SemanticStatePreimage {
        certified_program_ref,
        transitions: transitions
            .values()
            .map(|transition| SemanticTransitionPreimage {
                occurrence_id: &transition.record.occurrence_id,
                outcome_ref: &transition.record.outcome_ref,
                facts: &transition.record.facts,
            })
            .collect(),
        live_bindings: bindings.values().collect(),
    };
    let canonical = canonical_json(&preimage)
        .map_err(|_| invalid("semantic state preimage is not canonical"))?;
    let mut bytes = b"mfm.structured-semantic-state.v1\0".to_vec();
    bytes.extend_from_slice(canonical.as_bytes());
    Ok(RunSemanticStateDigest::from_digest(sha256_digest_bytes(
        &bytes,
    )))
}
