use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_canonical::{RecoverabilityContractV1, ReferenceTerminalKindV1};
use mfm_facts::{FactCandidate, FactSelectionRequest, FactSubject, FactTopK};
use mfm_ids::{ContentRef, FieldPath, RunId, SemanticDigest, TenantScopeId};
use mfm_journal::v1::{
    AuthorizationRef, ExternalAccessAuthorized, ExternalAccessObserved, FactClaimEnvelope,
    FactContentIdentityPreimage, FactEmission, FactLogicalIdentityPreimage, FactRef,
    FactSelectionCompleteness, FactSelectionResponse, FactSelectionResult,
    FactSelectionScanAttestation, FactSelectionScanContract, FactValueComponent, FrozenReadIntent,
    JournalHead, ObservationOutcome, ObservationOutcomeFields, ObservationRef, ProducerBinding,
    ProducerBindingFields, ReadCapabilityBinding, RunJournalRecordFields, SelectedFact,
    SourceClosurePreimage, TenantFactCoordinateFields, TenantFactFrontier, TransitionRef, ValueRef,
};
use mfm_spec::v1::RetainedValueContract;

use super::objects::PreparedAuthority;
use super::{
    verify_offline_recorded_history, AppendOutcome, AppendRejection, AssignedJournalAppend,
    AsyncStoreFuture, AuthorizeExternalAccess, CommittedAppend, CommittedJournalCommit,
    CommittedObject, Drive, ExistingRunAppendMaterial, NewlyAppended, ObjectAuthorityKey,
    ObservationMaterial, PreparedJournalAppend, PreparedObjectGraph, Replay, Result,
    RunAccessAuthority, RunJournalBackend, RunJournalStore, StoreError, StoreIdentity,
    VerifiedRunView, FACT_SELECTION_OPERATION_ID,
};

/// Maximum publications read by one private fact-scan backend step.
pub const FACT_SCAN_STEP_PUBLICATIONS: usize = 4_096;

/// Maximum individual facts read by one private fact-scan backend step.
pub const FACT_SCAN_STEP_FACTS: usize = 8_192;

/// Maximum unique typed references admitted into one verified fact source closure.
pub const FACT_SOURCE_CLOSURE_MAX_REFERENCES: usize = 65_536;

/// Affine authority to scan one exact reserved fact-selection prefix.
///
/// This value is returned only for a directly observed new authorization. It
/// intentionally implements neither `Clone`, `Copy`, `Debug`, nor Serde.
#[must_use = "the reserved fact scan may run only while this fresh permit is owned"]
pub struct FactScanPermit {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    consuming_run_id: RunId,
    authorization_ref: AuthorizationRef,
    authorization: ExternalAccessAuthorized,
    request: FactSelectionRequest,
    frontier: TenantFactFrontier,
}

impl FactScanPermit {
    fn from_new_authorization(
        store_identity: StoreIdentity,
        authorization: super::NewlyAppendedAuthorization,
        request: FactSelectionRequest,
    ) -> Result<Self> {
        let frontier = authorization
            .committed()
            .fact_frontier()
            .cloned()
            .ok_or(StoreError::FactScanBindingMismatch)?;
        let frontier_fields = frontier.fields()?;
        let authorization_fields = authorization.authorization().fields()?;
        let request_ref = authorization_fields.request_ref.fields()?;
        let request_content_ref = request
            .content_ref()
            .map_err(|_| StoreError::JournalContract)?;
        if authorization_fields.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID
            || request_ref.schema_id != *request_content_ref.schema_id()
            || request_ref.content_digest != *request_content_ref.content_digest()
            || frontier_fields.store_scope_id != *store_identity.store_scope_id()
            || frontier_fields.store_epoch != store_identity.store_epoch()
            || frontier_fields.tenant_scope_id != *authorization.tenant_scope_id()
        {
            return Err(StoreError::FactScanBindingMismatch);
        }
        Ok(Self {
            store_identity,
            tenant_scope_id: authorization.tenant_scope_id().clone(),
            consuming_run_id: authorization.run_id().clone(),
            authorization_ref: authorization.authorization_ref().clone(),
            authorization: authorization.authorization().clone(),
            request,
            frontier,
        })
    }

    /// Returns the exact authoritative store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the consuming run's admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the consuming run excluded from producer selection.
    pub const fn consuming_run_id(&self) -> &RunId {
        &self.consuming_run_id
    }

    /// Returns the exact authorization that fixed the scan barrier.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        &self.authorization_ref
    }

    /// Returns the complete reserved authorization.
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        &self.authorization
    }

    /// Returns the exact state-authored request.
    pub const fn request(&self) -> &FactSelectionRequest {
        &self.request
    }

    /// Returns the exact authoritative frontier fixed by authorization.
    pub const fn frontier(&self) -> &TenantFactFrontier {
        &self.frontier
    }
}

/// Authority-safe result of a reserved fact-selection authorization append.
pub enum FactSelectionAuthorizationOutcome {
    /// The writer directly committed the barrier and returned its sole affine scan permit.
    NewlyAuthorized(Box<FactScanPermit>),
    /// The exact barrier was already committed, so no live scan permit was recreated.
    AlreadyCommitted(CommittedAppend),
    /// The barrier append was deterministically rejected.
    Rejected(AppendRejection),
    /// Commit visibility is ambiguous and no live scan permit exists.
    OutcomeUnknown,
}

/// Exact response created only after one private authoritative scan reached its frozen frontier.
///
/// This token intentionally implements neither `Clone`, `Copy`, `Debug`, nor
/// Serde. Only generic store-owned observation preparation may consume it.
#[must_use = "a complete fact scan must be consumed by store-owned observation preparation"]
pub struct CompletedFactScan {
    permit: FactScanPermit,
    response: FactSelectionResponse,
    sources: VerifiedFactSources,
}

impl CompletedFactScan {
    /// Returns the exact authorization whose barrier was completely scanned.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        self.permit.authorization_ref()
    }

    /// Returns the exact state-authored request.
    pub const fn request(&self) -> &FactSelectionRequest {
        self.permit.request()
    }

    /// Returns the exact authoritative frontier reached by this scan.
    pub const fn frontier(&self) -> &TenantFactFrontier {
        self.permit.frontier()
    }

    /// Returns the complete deterministic response.
    pub const fn response(&self) -> &FactSelectionResponse {
        &self.response
    }

    /// Consumes this affine result into the sole generic observation material.
    ///
    /// The store derives the linked authorization from the sealed scan permit;
    /// callers cannot duplicate or substitute that reference.
    pub fn into_observation_material(self) -> ExistingRunAppendMaterial {
        ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::FactSelection {
            completed_scan: Box::new(self),
        }))
    }
}

/// Same-store fact completeness established against one exact verified run view.
///
/// Construction is private to [`FactSelectionStore`]. The sealed result binds
/// every returned completeness claim to the store lineage, tenant, run, and
/// physical journal head that were rechecked.
pub struct VerifiedFactSelectionCompleteness {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    journal_head: JournalHead,
    fact_selections: Vec<FactSelectionCompleteness>,
}

impl VerifiedFactSelectionCompleteness {
    /// Returns the authoritative store lineage used for prefix verification.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the admitted tenant of the exact verified view.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the admitted run of the exact verified view.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical journal head whose observations were checked.
    pub const fn journal_head(&self) -> &JournalHead {
        &self.journal_head
    }

    /// Returns same-store claims in consuming transition order.
    pub fn fact_selections(&self) -> &[FactSelectionCompleteness] {
        &self.fact_selections
    }
}

struct QualifiedFactScan {
    request: FactSelectionRequest,
    response_contract: RetainedValueContract,
    scan_attestation_contract: RetainedValueContract,
}

struct VerifiedFactPublicationObjects {
    objects: Vec<CommittedObject>,
}

struct CandidateFactSource {
    descriptor_ref: ContentRef,
    claim_ref: ValueRef,
    subject_ref: ValueRef,
    response_ref: ValueRef,
    publication: Arc<VerifiedFactPublicationObjects>,
}

#[derive(Clone)]
struct VerifiedFactSourceRoots {
    descriptor_ref: ContentRef,
    claim_ref: ValueRef,
    subject_ref: ValueRef,
    response_ref: ValueRef,
}

struct VerifiedFactSources {
    roots: Vec<VerifiedFactSourceRoots>,
    dependencies: Vec<ContentRef>,
    objects: Vec<CommittedObject>,
    transport_objects: Vec<CommittedObject>,
}

#[derive(Clone)]
struct ScannedFact {
    selected: SelectedFact,
    source: Arc<CandidateFactSource>,
}

struct FactScanEvaluation {
    response: FactSelectionResponse,
    sources: VerifiedFactSources,
}

struct VerifiedResponseClosure {
    digest: SemanticDigest,
    objects: Vec<CommittedObject>,
}

pub(super) struct RecordedFactSourceRoot {
    pub(super) descriptor_ref: ContentRef,
    pub(super) claim_ref: ValueRef,
    pub(super) subject_ref: ValueRef,
    pub(super) response_ref: ValueRef,
}

struct RecordedFactSelection {
    authorization_ref: AuthorizationRef,
    request: FactSelectionRequest,
    frontier: TenantFactFrontier,
    response_ref: ValueRef,
    response: FactSelectionResponse,
    response_closure_digest: SemanticDigest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClosureVisitState {
    Visiting,
    Complete,
}

enum ClosureVisitFrame {
    EnterValue(ValueRef),
    ExitValue(Vec<u8>),
    EnterContent(ContentRef),
    ExitContent(Vec<u8>),
}

struct VerifiedClosureMaterial {
    dependencies: Vec<ContentRef>,
    objects: Vec<CommittedObject>,
    transport_objects: Vec<CommittedObject>,
}

struct PrefixClosureWalker<'a> {
    contract: &'static RecoverabilityContractV1,
    available_values: BTreeMap<Vec<u8>, &'a CommittedObject>,
    available_content: BTreeMap<Vec<u8>, Vec<&'a CommittedObject>>,
    dependencies: BTreeMap<Vec<u8>, ContentRef>,
    objects: BTreeMap<Vec<u8>, CommittedObject>,
    transport_objects: BTreeMap<Vec<u8>, CommittedObject>,
    value_visit_states: BTreeMap<Vec<u8>, ClosureVisitState>,
    content_visit_states: BTreeMap<Vec<u8>, ClosureVisitState>,
}

impl<'a> PrefixClosureWalker<'a> {
    fn new(available_objects: &'a [CommittedObject]) -> Result<Self> {
        let contract = RecoverabilityContractV1::embedded()?;
        let mut available_values = BTreeMap::<Vec<u8>, &'a CommittedObject>::new();
        let mut available_content = BTreeMap::<Vec<u8>, Vec<&'a CommittedObject>>::new();
        for object in available_objects {
            let value_key = object.value_ref().as_bytes().to_vec();
            if let Some(existing) = available_values.get(&value_key) {
                if existing.value_ref() != object.value_ref() || existing.bytes() != object.bytes()
                {
                    return Err(StoreError::InvalidSourceClosure);
                }
            } else {
                available_values.insert(value_key, object);
            }
            let content_ref = value_payload_content_ref(object.value_ref())?;
            let content_key = canonical_content_ref_key(contract, &content_ref)?;
            available_content
                .entry(content_key)
                .or_default()
                .push(object);
        }
        for authorities in available_content.values_mut() {
            authorities.sort_by(|left, right| {
                left.value_ref()
                    .as_bytes()
                    .cmp(right.value_ref().as_bytes())
            });
            if authorities
                .windows(2)
                .any(|pair| pair[0].bytes() != pair[1].bytes())
            {
                return Err(StoreError::InvalidSourceClosure);
            }
        }
        Ok(Self {
            contract,
            available_values,
            available_content,
            dependencies: BTreeMap::new(),
            objects: BTreeMap::new(),
            transport_objects: BTreeMap::new(),
            value_visit_states: BTreeMap::new(),
            content_visit_states: BTreeMap::new(),
        })
    }

    fn verify(
        mut self,
        value_roots: &[ValueRef],
        dependency_roots: &[ContentRef],
    ) -> Result<VerifiedClosureMaterial> {
        let mut stack = Vec::new();
        stack.extend(
            dependency_roots
                .iter()
                .rev()
                .cloned()
                .map(ClosureVisitFrame::EnterContent),
        );
        stack.extend(
            value_roots
                .iter()
                .rev()
                .cloned()
                .map(ClosureVisitFrame::EnterValue),
        );
        self.walk(stack)?;
        Ok(VerifiedClosureMaterial {
            dependencies: self.dependencies.into_values().collect(),
            objects: self.objects.into_values().collect(),
            transport_objects: self.transport_objects.into_values().collect(),
        })
    }

    fn walk(&mut self, mut stack: Vec<ClosureVisitFrame>) -> Result<()> {
        while let Some(frame) = stack.pop() {
            match frame {
                ClosureVisitFrame::EnterValue(value_ref) => {
                    let key = value_ref.as_bytes().to_vec();
                    match self.value_visit_states.get(&key) {
                        Some(ClosureVisitState::Visiting) => {
                            return Err(StoreError::InvalidSourceClosure);
                        }
                        Some(ClosureVisitState::Complete) => continue,
                        None => {}
                    }
                    let object = self
                        .available_values
                        .get(&key)
                        .copied()
                        .ok_or(StoreError::InvalidSourceClosure)?;
                    self.insert_object(object.clone())?;
                    self.value_visit_states
                        .insert(key.clone(), ClosureVisitState::Visiting);

                    let fields = value_ref.fields()?;
                    let value = self
                        .contract
                        .strict_decode_schema_id(&fields.schema_id, object.bytes())?;
                    let mut children = self.payload_reference_frames(&value)?;
                    let evidence_key =
                        canonical_content_ref_key(self.contract, &fields.evidence_contract_ref)?;
                    if !self.content_visit_states.contains_key(&evidence_key) {
                        children.push(ClosureVisitFrame::EnterContent(
                            fields.evidence_contract_ref,
                        ));
                    }
                    stack.push(ClosureVisitFrame::ExitValue(key));
                    stack.extend(children.into_iter().rev());
                }
                ClosureVisitFrame::ExitValue(key) => {
                    let state = self
                        .value_visit_states
                        .get_mut(&key)
                        .ok_or(StoreError::InvalidSourceClosure)?;
                    if *state != ClosureVisitState::Visiting {
                        return Err(StoreError::InvalidSourceClosure);
                    }
                    *state = ClosureVisitState::Complete;
                }
                ClosureVisitFrame::EnterContent(content_ref) => {
                    let key = canonical_content_ref_key(self.contract, &content_ref)?;
                    match self.content_visit_states.get(&key) {
                        Some(ClosureVisitState::Visiting) => {
                            return Err(StoreError::InvalidSourceClosure);
                        }
                        Some(ClosureVisitState::Complete) => continue,
                        None => {}
                    }
                    if !self.dependencies.contains_key(&key) {
                        self.ensure_reference_capacity()?;
                        self.dependencies.insert(key.clone(), content_ref.clone());
                    }
                    let transport = self
                        .available_content
                        .get(&key)
                        .and_then(|authorities| authorities.first())
                        .copied()
                        .ok_or(StoreError::InvalidSourceClosure)?;
                    self.insert_transport_object(transport.clone())?;
                    self.content_visit_states
                        .insert(key.clone(), ClosureVisitState::Visiting);
                    let value = self
                        .contract
                        .strict_decode_schema_id(content_ref.schema_id(), transport.bytes())?;
                    let children = self.payload_reference_frames(&value)?;
                    stack.push(ClosureVisitFrame::ExitContent(key));
                    stack.extend(children.into_iter().rev());
                }
                ClosureVisitFrame::ExitContent(key) => {
                    let state = self
                        .content_visit_states
                        .get_mut(&key)
                        .ok_or(StoreError::InvalidSourceClosure)?;
                    if *state != ClosureVisitState::Visiting {
                        return Err(StoreError::InvalidSourceClosure);
                    }
                    *state = ClosureVisitState::Complete;
                }
            }
        }
        Ok(())
    }

    fn payload_reference_frames(
        &self,
        value: &mfm_canonical::ValidatedCanonicalValueV1,
    ) -> Result<Vec<ClosureVisitFrame>> {
        self.contract
            .reference_edges(value)?
            .into_iter()
            .map(|edge| match edge.terminal_kind() {
                ReferenceTerminalKindV1::ContentRef => {
                    serde_json::from_slice::<ContentRef>(edge.value().as_bytes())
                        .map(ClosureVisitFrame::EnterContent)
                        .map_err(|_| StoreError::InvalidSourceClosure)
                }
                ReferenceTerminalKindV1::ValueRef => {
                    ValueRef::strict_decode(edge.value().as_bytes())
                        .map(ClosureVisitFrame::EnterValue)
                        .map_err(Into::into)
                }
            })
            .collect()
    }

    fn insert_transport_object(&mut self, object: CommittedObject) -> Result<()> {
        let key = object.value_ref().as_bytes().to_vec();
        if let Some(existing) = self.transport_objects.get(&key) {
            if existing != &object {
                return Err(StoreError::InvalidSourceClosure);
            }
        } else {
            self.transport_objects.insert(key, object);
        }
        Ok(())
    }

    fn insert_object(&mut self, object: CommittedObject) -> Result<()> {
        let key = object.value_ref().as_bytes().to_vec();
        if let Some(existing) = self.objects.get(&key) {
            if existing != &object {
                return Err(StoreError::InvalidSourceClosure);
            }
            return Ok(());
        }
        self.ensure_reference_capacity()?;
        self.objects.insert(key, object);
        Ok(())
    }

    fn ensure_reference_capacity(&self) -> Result<()> {
        let retained = self
            .dependencies
            .len()
            .checked_add(self.objects.len())
            .ok_or(StoreError::InvalidSourceClosure)?;
        if retained >= FACT_SOURCE_CLOSURE_MAX_REFERENCES {
            Err(StoreError::InvalidSourceClosure)
        } else {
            Ok(())
        }
    }
}

fn canonical_content_ref_key(
    contract: &RecoverabilityContractV1,
    reference: &ContentRef,
) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(reference).map_err(|_| StoreError::InvalidSourceClosure)?;
    contract.strict_decode("mfm.content-ref.v1", &bytes)?;
    Ok(bytes)
}

fn value_payload_content_ref(value_ref: &ValueRef) -> Result<ContentRef> {
    let fields = value_ref.fields()?;
    ContentRef::new(fields.schema_id, fields.content_digest)
        .map_err(|_| StoreError::InvalidSourceClosure)
}

fn verify_selected_fact_sources(
    candidates: &[Arc<CandidateFactSource>],
) -> Result<VerifiedFactSources> {
    let contract = RecoverabilityContractV1::embedded()?;
    let mut roots = BTreeMap::<Vec<Vec<u8>>, VerifiedFactSourceRoots>::new();
    let mut dependencies = BTreeMap::<Vec<u8>, ContentRef>::new();
    let mut objects = BTreeMap::<Vec<u8>, CommittedObject>::new();
    let mut transport_objects = BTreeMap::<Vec<u8>, CommittedObject>::new();
    for candidate in candidates {
        let source_key = vec![
            canonical_content_ref_key(contract, &candidate.descriptor_ref)?,
            candidate.claim_ref.as_bytes().to_vec(),
            candidate.subject_ref.as_bytes().to_vec(),
            candidate.response_ref.as_bytes().to_vec(),
        ];
        if roots.contains_key(&source_key) {
            continue;
        }
        let material = PrefixClosureWalker::new(&candidate.publication.objects)?.verify(
            &[
                candidate.claim_ref.clone(),
                candidate.subject_ref.clone(),
                candidate.response_ref.clone(),
            ],
            std::slice::from_ref(&candidate.descriptor_ref),
        )?;
        for dependency in material.dependencies {
            insert_bounded_dependency(&mut dependencies, &objects, dependency)?;
        }
        for object in material.objects {
            insert_bounded_object(&dependencies, &mut objects, object)?;
        }
        for object in material.transport_objects {
            insert_unbounded_object(&mut transport_objects, object)?;
        }
        roots.insert(
            source_key,
            VerifiedFactSourceRoots {
                descriptor_ref: candidate.descriptor_ref.clone(),
                claim_ref: candidate.claim_ref.clone(),
                subject_ref: candidate.subject_ref.clone(),
                response_ref: candidate.response_ref.clone(),
            },
        );
    }
    Ok(VerifiedFactSources {
        roots: roots.into_values().collect(),
        dependencies: dependencies.into_values().collect(),
        objects: objects.into_values().collect(),
        transport_objects: transport_objects.into_values().collect(),
    })
}

fn insert_bounded_dependency(
    dependencies: &mut BTreeMap<Vec<u8>, ContentRef>,
    objects: &BTreeMap<Vec<u8>, CommittedObject>,
    reference: ContentRef,
) -> Result<()> {
    let contract = RecoverabilityContractV1::embedded()?;
    let key = canonical_content_ref_key(contract, &reference)?;
    if dependencies.contains_key(&key) {
        return Ok(());
    }
    ensure_closure_capacity(dependencies.len(), objects.len())?;
    dependencies.insert(key, reference);
    Ok(())
}

fn insert_bounded_object(
    dependencies: &BTreeMap<Vec<u8>, ContentRef>,
    objects: &mut BTreeMap<Vec<u8>, CommittedObject>,
    object: CommittedObject,
) -> Result<()> {
    let key = object.value_ref().as_bytes().to_vec();
    if let Some(existing) = objects.get(&key) {
        if existing != &object {
            return Err(StoreError::InvalidSourceClosure);
        }
        return Ok(());
    }
    ensure_closure_capacity(dependencies.len(), objects.len())?;
    objects.insert(key, object);
    Ok(())
}

fn insert_unbounded_object(
    objects: &mut BTreeMap<Vec<u8>, CommittedObject>,
    object: CommittedObject,
) -> Result<()> {
    let key = object.value_ref().as_bytes().to_vec();
    if let Some(existing) = objects.get(&key) {
        if existing != &object {
            return Err(StoreError::InvalidSourceClosure);
        }
    } else {
        objects.insert(key, object);
    }
    Ok(())
}

fn ensure_closure_capacity(dependency_count: usize, object_count: usize) -> Result<()> {
    let retained = dependency_count
        .checked_add(object_count)
        .ok_or(StoreError::InvalidSourceClosure)?;
    if retained >= FACT_SOURCE_CLOSURE_MAX_REFERENCES {
        Err(StoreError::InvalidSourceClosure)
    } else {
        Ok(())
    }
}

fn derive_response_closure(
    view: &VerifiedRunView,
    authorization_ref: &AuthorizationRef,
    response_ref: &ValueRef,
    response_bytes: &[u8],
    sources: &VerifiedFactSources,
) -> Result<VerifiedResponseClosure> {
    let entry = view
        .access_audit_entries()
        .find(|entry| entry.authorization_ref() == authorization_ref)
        .ok_or(StoreError::InvalidSourceClosure)?;
    let consumer_prefix = journal_prefix_objects(view, entry.authorization_journal_head())?;
    derive_response_closure_from_material(response_ref, response_bytes, sources, &consumer_prefix)
}

fn derive_response_closure_from_material(
    response_ref: &ValueRef,
    response_bytes: &[u8],
    sources: &VerifiedFactSources,
    consumer_prefix: &[CommittedObject],
) -> Result<VerifiedResponseClosure> {
    let response_fields = response_ref.fields()?;
    let consumer = PrefixClosureWalker::new(&consumer_prefix)?.verify(
        &[],
        std::slice::from_ref(&response_fields.evidence_contract_ref),
    )?;
    CommittedObject::from_persisted(response_ref.clone(), response_bytes.to_vec())?;
    let contract = RecoverabilityContractV1::embedded()?;
    let response_value =
        contract.strict_decode_schema_id(&response_fields.schema_id, response_bytes)?;

    let mut dependencies = BTreeMap::<Vec<u8>, ContentRef>::new();
    let mut objects = BTreeMap::<Vec<u8>, CommittedObject>::new();
    let mut graph_objects = BTreeMap::<Vec<u8>, CommittedObject>::new();
    for dependency in sources
        .dependencies
        .iter()
        .chain(consumer.dependencies.iter())
    {
        insert_bounded_dependency(&mut dependencies, &objects, dependency.clone())?;
    }
    for object in sources.objects.iter().chain(consumer.objects.iter()) {
        insert_bounded_object(&dependencies, &mut objects, object.clone())?;
        insert_unbounded_object(&mut graph_objects, object.clone())?;
    }
    for object in sources
        .transport_objects
        .iter()
        .chain(consumer.transport_objects.iter())
    {
        insert_unbounded_object(&mut graph_objects, object.clone())?;
    }
    for roots in &sources.roots {
        let descriptor_key = canonical_content_ref_key(contract, &roots.descriptor_ref)?;
        if !dependencies.contains_key(&descriptor_key)
            || [&roots.claim_ref, &roots.subject_ref, &roots.response_ref]
                .into_iter()
                .any(|reference| !objects.contains_key(reference.as_bytes()))
        {
            return Err(StoreError::InvalidSourceClosure);
        }
    }
    for edge in contract.reference_edges(&response_value)? {
        match edge.terminal_kind() {
            ReferenceTerminalKindV1::ContentRef => {
                let reference = serde_json::from_slice::<ContentRef>(edge.value().as_bytes())
                    .map_err(|_| StoreError::InvalidSourceClosure)?;
                let key = canonical_content_ref_key(contract, &reference)?;
                if !dependencies.contains_key(&key) {
                    return Err(StoreError::InvalidSourceClosure);
                }
            }
            ReferenceTerminalKindV1::ValueRef => {
                let reference = ValueRef::strict_decode(edge.value().as_bytes())?;
                if !objects.contains_key(reference.as_bytes()) {
                    return Err(StoreError::InvalidSourceClosure);
                }
            }
        }
    }

    let root_content_ref = value_payload_content_ref(response_ref)?;
    let root_content_key = canonical_content_ref_key(contract, &root_content_ref)?;
    if !dependencies.contains_key(&root_content_key) {
        ensure_closure_capacity(dependencies.len(), objects.len())?;
        dependencies.insert(root_content_key.clone(), root_content_ref.clone());
    }
    let root_value_key = response_ref.as_bytes().to_vec();
    if objects.contains_key(&root_value_key) {
        return Err(StoreError::InvalidSourceClosure);
    }
    ensure_closure_capacity(dependencies.len(), objects.len())?;
    let ordered_dependency_refs = dependencies
        .iter()
        .filter_map(|(key, reference)| (key != &root_content_key).then_some(reference.clone()))
        .collect::<Vec<_>>();
    let mut ordered_object_refs = objects
        .values()
        .map(|object| object.value_ref().clone())
        .collect::<Vec<_>>();
    ordered_object_refs.push(response_ref.clone());
    ordered_object_refs.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let digest = SourceClosurePreimage::new(
        &root_content_ref,
        &ordered_dependency_refs,
        &ordered_object_refs,
    )?
    .source_closure_digest()?
    .into_semantic_digest();
    Ok(VerifiedResponseClosure {
        digest,
        objects: graph_objects.into_values().collect(),
    })
}

pub(super) fn derive_recorded_response_closure_digest(
    response_ref: &ValueRef,
    response_bytes: &[u8],
    roots: Vec<RecordedFactSourceRoot>,
    observation_graph: &[CommittedObject],
    consumer_prefix: &[CommittedObject],
) -> Result<SemanticDigest> {
    let contract = RecoverabilityContractV1::embedded()?;
    let mut unique = BTreeMap::<Vec<Vec<u8>>, VerifiedFactSourceRoots>::new();
    for root in roots {
        unique.insert(
            vec![
                canonical_content_ref_key(contract, &root.descriptor_ref)?,
                root.claim_ref.as_bytes().to_vec(),
                root.subject_ref.as_bytes().to_vec(),
                root.response_ref.as_bytes().to_vec(),
            ],
            VerifiedFactSourceRoots {
                descriptor_ref: root.descriptor_ref,
                claim_ref: root.claim_ref,
                subject_ref: root.subject_ref,
                response_ref: root.response_ref,
            },
        );
    }
    let roots = unique.into_values().collect::<Vec<_>>();
    let value_roots = roots
        .iter()
        .flat_map(|root| {
            [
                root.claim_ref.clone(),
                root.subject_ref.clone(),
                root.response_ref.clone(),
            ]
        })
        .collect::<Vec<_>>();
    let dependency_roots = roots
        .iter()
        .map(|root| root.descriptor_ref.clone())
        .collect::<Vec<_>>();
    let source =
        PrefixClosureWalker::new(observation_graph)?.verify(&value_roots, &dependency_roots)?;
    let sources = VerifiedFactSources {
        roots,
        dependencies: source.dependencies,
        objects: source.objects,
        transport_objects: source.transport_objects,
    };
    let closure = derive_response_closure_from_material(
        response_ref,
        response_bytes,
        &sources,
        consumer_prefix,
    )?;
    let expected = closure
        .objects
        .iter()
        .map(|object| (object.value_ref().as_bytes().to_vec(), object))
        .collect::<BTreeMap<_, _>>();
    let actual = observation_graph
        .iter()
        .map(|object| (object.value_ref().as_bytes().to_vec(), object))
        .collect::<BTreeMap<_, _>>();
    if expected.len() != closure.objects.len()
        || actual.len() != observation_graph.len()
        || expected != actual
    {
        return Err(StoreError::InvalidSourceClosure);
    }
    Ok(closure.digest)
}

fn prepare_fact_observation_graph(
    observation: &ExternalAccessObserved,
    response_ref: &ValueRef,
    response_bytes: &[u8],
    attestation_ref: &ValueRef,
    attestation_bytes: &[u8],
    closure: &VerifiedResponseClosure,
) -> Result<PreparedObjectGraph> {
    let mut authorities = vec![
        PreparedAuthority::produced(response_ref.clone(), response_bytes.to_vec())?,
        PreparedAuthority::produced(attestation_ref.clone(), attestation_bytes.to_vec())?,
    ];
    for object in &closure.objects {
        authorities.push(PreparedAuthority::preexisting(
            object.value_ref().clone(),
            object.bytes().to_vec(),
        )?);
    }
    PreparedObjectGraph::prepare_for_record_values(&[observation.canonical_value()?], authorities)
}

pub(super) fn prepare_fact_selection_observation(
    view: &VerifiedRunView,
    completed: CompletedFactScan,
) -> Result<(
    ExternalAccessObserved,
    PreparedObjectGraph,
    PendingFactScanAttestation,
)> {
    if completed.permit.store_identity() != view.store_identity()
        || completed.permit.tenant_scope_id() != view.tenant_scope_id()
        || completed.permit.consuming_run_id() != view.run_id()
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let entry = view
        .access_audit_entries()
        .find(|entry| entry.authorization_ref() == completed.authorization_ref())
        .ok_or(StoreError::FactScanBindingMismatch)?;
    let retained_frontier = authorization_frontier(view, entry.authorization_journal_head())?;
    if &retained_frontier != completed.frontier() || entry.observation().is_some() {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let qualified = qualify_fact_scan(
        view,
        completed.authorization_ref(),
        Some(completed.request()),
    )?;

    let response_path = FieldPath::new("outcome.result_ref")?;
    let response_producer =
        ProducerBinding::external_observation(completed.authorization_ref(), &response_path)?;
    let response_ref = super::objects::derive_value_ref(
        &qualified.response_contract,
        &response_producer,
        completed.response().as_bytes(),
    )?;
    let closure = derive_response_closure(
        view,
        completed.authorization_ref(),
        &response_ref,
        completed.response().as_bytes(),
        &completed.sources,
    )?;
    let request_digest = completed
        .request()
        .request_digest()
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let attestation = FactSelectionScanAttestation::new(
        view.store_identity().store_scope_id(),
        &view.store_identity().store_epoch(),
        view.tenant_scope_id(),
        completed.authorization_ref(),
        &request_digest,
        completed.frontier(),
        &response_ref,
        &closure.digest,
    )?;
    let attestation_path = FieldPath::new("fact_selection_scan_attestation_ref")?;
    let attestation_producer =
        ProducerBinding::external_observation(completed.authorization_ref(), &attestation_path)?;
    let attestation_ref = super::objects::derive_value_ref(
        &qualified.scan_attestation_contract,
        &attestation_producer,
        attestation.as_bytes(),
    )?;
    let outcome = ObservationOutcome::returned(&response_ref)?;
    let observation = ExternalAccessObserved::new(
        completed.authorization_ref(),
        &outcome,
        Some(&attestation_ref),
    )?;
    let objects = prepare_fact_observation_graph(
        &observation,
        &response_ref,
        completed.response().as_bytes(),
        &attestation_ref,
        attestation.as_bytes(),
        &closure,
    )?;
    let pending = PendingFactScanAttestation::new(
        completed.authorization_ref().clone(),
        response_ref,
        attestation_ref,
    );
    Ok((observation, objects, pending))
}

fn qualify_fact_scan(
    view: &VerifiedRunView,
    authorization_ref: &AuthorizationRef,
    expected_request: Option<&FactSelectionRequest>,
) -> Result<QualifiedFactScan> {
    let authorization = view
        .authorizations()
        .find_map(|(reference, authorization)| {
            (reference == authorization_ref).then_some(authorization)
        })
        .ok_or(StoreError::UnknownAuthorization)?;
    let authorization_fields = authorization.fields()?;
    if authorization_fields.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID {
        return Err(StoreError::FactScanBindingMismatch);
    }

    let capability_binding_ref = authorization_fields.capability_binding_ref.fields()?;
    let admitted_binding = view
        .capability_binding_manifest()
        .entries()
        .iter()
        .find(|entry| entry.operation_id.as_str() == FACT_SELECTION_OPERATION_ID)
        .ok_or(StoreError::FactScanBindingMismatch)?;
    if admitted_binding.binding_ref != capability_binding_ref {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let binding =
        ReadCapabilityBinding::strict_decode(view.retained_content(&capability_binding_ref)?)?;
    let binding_fields = binding.fields()?;
    let scan_contract = FactSelectionScanContract::strict_decode(
        view.retained_content(&binding_fields.capability_contract_ref)?,
    )?;
    let scan_fields = scan_contract.fields()?;
    if scan_fields.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID {
        return Err(StoreError::FactScanBindingMismatch);
    }

    let frozen_read_intent_ref = authorization_fields
        .frozen_read_intent_ref
        .as_ref()
        .ok_or(StoreError::FactScanBindingMismatch)?;
    let frozen_read_intent =
        FrozenReadIntent::strict_decode(view.retained_value(frozen_read_intent_ref)?.bytes())?;
    let frozen_fields = frozen_read_intent.fields()?;
    if frozen_fields.capability_binding_ref != authorization_fields.capability_binding_ref
        || frozen_fields.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID
        || frozen_fields.request_ref != authorization_fields.request_ref
        || frozen_fields.request_contract != scan_fields.request_contract
        || frozen_fields.returned_contract != scan_fields.response_contract
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    authorization_fields
        .request_ref
        .validate_contract(&scan_fields.request_contract)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let request = FactSelectionRequest::from_canonical_json(
        view.retained_value(&authorization_fields.request_ref)?
            .bytes(),
    )
    .map_err(|_| StoreError::FactScanBindingMismatch)?;
    if expected_request.is_some_and(|expected| expected != &request) {
        return Err(StoreError::FactScanBindingMismatch);
    }

    Ok(QualifiedFactScan {
        request,
        response_contract: scan_fields.response_contract,
        scan_attestation_contract: scan_fields.scan_attestation_contract,
    })
}

fn validate_recorded_fact_selection(
    view: &VerifiedRunView,
    row: &PersistedFactScanAttestation,
) -> Result<RecordedFactSelection> {
    let entry = view
        .access_audit_entries()
        .find(|entry| entry.authorization_ref() == row.authorization_ref())
        .ok_or(StoreError::FactScanBindingMismatch)?;
    let authorization_fields = entry.authorization().fields()?;
    if authorization_fields.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let (observation_ref, observation, observation_head) = entry
        .observation()
        .ok_or(StoreError::FactScanBindingMismatch)?;
    if observation_ref != row.observation_ref() || observation_head != row.containing_journal_head()
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let observation_fields = observation.fields()?;
    let response_ref = match observation_fields.outcome.fields()? {
        ObservationOutcomeFields::Returned { result_ref } => result_ref,
        ObservationOutcomeFields::DidNotEnter { .. }
        | ObservationOutcomeFields::Indeterminate { .. } => {
            return Err(StoreError::FactScanBindingMismatch);
        }
    };
    if observation_fields
        .fact_selection_scan_attestation_ref
        .as_ref()
        != Some(row.attestation_ref())
    {
        return Err(StoreError::FactScanBindingMismatch);
    }

    validate_external_observation_binding(
        &response_ref,
        row.authorization_ref(),
        "outcome.result_ref",
    )?;
    validate_external_observation_binding(
        row.attestation_ref(),
        row.authorization_ref(),
        "fact_selection_scan_attestation_ref",
    )?;

    let qualified = qualify_fact_scan(view, row.authorization_ref(), None)?;
    response_ref
        .validate_contract(&qualified.response_contract)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    row.attestation_ref()
        .validate_contract(&qualified.scan_attestation_contract)
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    let response =
        FactSelectionResponse::strict_decode(view.retained_value(&response_ref)?.bytes())?;
    response.validate_request(&qualified.request)?;
    let frontier = authorization_frontier(view, entry.authorization_journal_head())?;
    if response.fields()?.frontier != frontier {
        return Err(StoreError::FactScanBindingMismatch);
    }

    let attestation = FactSelectionScanAttestation::strict_decode(
        view.retained_value(row.attestation_ref())?.bytes(),
    )?;
    let attestation_fields = attestation.fields()?;
    let request_digest = qualified
        .request
        .request_digest()
        .map_err(|_| StoreError::FactScanBindingMismatch)?;
    if attestation_fields.store_scope_id != *view.store_identity().store_scope_id()
        || attestation_fields.store_epoch != view.store_identity().store_epoch()
        || attestation_fields.tenant_scope_id != *view.tenant_scope_id()
        || attestation_fields.authorization_ref != *row.authorization_ref()
        || attestation_fields.request_digest != request_digest
        || attestation_fields.frontier != frontier
        || attestation_fields.response_ref != response_ref
    {
        return Err(StoreError::FactScanBindingMismatch);
    }

    Ok(RecordedFactSelection {
        authorization_ref: row.authorization_ref().clone(),
        request: qualified.request,
        frontier,
        response_ref,
        response,
        response_closure_digest: attestation_fields.response_closure_digest,
    })
}

fn authorization_frontier(
    view: &VerifiedRunView,
    authorization_head: &JournalHead,
) -> Result<TenantFactFrontier> {
    let commit = view
        .journal()
        .commits()
        .iter()
        .find(|commit| {
            commit
                .envelope()
                .journal_head()
                .is_ok_and(|head| &head == authorization_head)
        })
        .ok_or(StoreError::FactScanBindingMismatch)?;
    let coordinate = commit.envelope().fields()?.core.tenant_fact_coordinate;
    let (tenant_scope_id, fact_order) = match coordinate.fields()? {
        TenantFactCoordinateFields::FactSelectionBarrier {
            tenant_scope_id,
            frontier_fact_order,
        } => (tenant_scope_id, frontier_fact_order),
        TenantFactCoordinateFields::None | TenantFactCoordinateFields::FactPublication { .. } => {
            return Err(StoreError::FactScanBindingMismatch);
        }
    };
    if tenant_scope_id != *view.tenant_scope_id() {
        return Err(StoreError::FactScanBindingMismatch);
    }
    TenantFactFrontier::new(
        view.store_identity().store_scope_id(),
        view.store_identity().store_epoch(),
        view.tenant_scope_id(),
        fact_order,
    )
    .map_err(Into::into)
}

fn validate_external_observation_binding(
    value_ref: &ValueRef,
    authorization_ref: &AuthorizationRef,
    expected_field_path: &'static str,
) -> Result<()> {
    let expected_field_path =
        FieldPath::new(expected_field_path).map_err(|_| StoreError::JournalContract)?;
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::ExternalObservation {
            authorization_ref: retained_authorization,
            field_path,
        } if retained_authorization == *authorization_ref && field_path == expected_field_path => {
            Ok(())
        }
        _ => Err(StoreError::FactScanBindingMismatch),
    }
}

struct FactScanSession {
    permit: FactScanPermit,
    next_fact_order: u64,
    accumulators: Vec<FactTopK<ScannedFact>>,
}

enum FactScanStep {
    More(Box<FactScanSession>),
    Complete(Box<CompletedFactScan>),
}

/// One fully verified fact-publication commit loaded by a backend scan step.
///
/// Construction is possible only through [`FactScanPageVerifier::verify_publication`].
pub struct VerifiedFactPublication {
    fact_order: u64,
    selectable: bool,
    producer_view: VerifiedRunView,
    source_objects: Arc<VerifiedFactPublicationObjects>,
    transition_ref: TransitionRef,
    emissions: Vec<FactEmission>,
}

impl VerifiedFactPublication {
    /// Returns the exact dense tenant publication order.
    pub const fn fact_order(&self) -> u64 {
        self.fact_order
    }

    /// Returns the verified producer run.
    pub fn producer_run_id(&self) -> &RunId {
        self.producer_view.run_id()
    }

    /// Returns whether this publication belongs to another run and may be selected.
    ///
    /// The consuming run's own publications remain in the dense verified prefix
    /// but are never eligible for the reserved other-run selection contract.
    pub const fn is_selectable(&self) -> bool {
        self.selectable
    }

    /// Returns the exact producing transition.
    pub const fn transition_ref(&self) -> &TransitionRef {
        &self.transition_ref
    }

    /// Returns every emission in exact callback order.
    pub fn emissions(&self) -> &[FactEmission] {
        &self.emissions
    }

    fn retained_value(&self, value_ref: &ValueRef) -> Result<&CommittedObject> {
        let key = ObjectAuthorityKey::from_value_ref(value_ref)?;
        self.source_objects
            .objects
            .iter()
            .find(|object| object.key() == &key)
            .ok_or(StoreError::CorruptFactHistory)
    }

    fn retained_content(&self, content_ref: &ContentRef) -> Result<&[u8]> {
        self.source_objects
            .objects
            .iter()
            .find_map(|object| {
                object.value_ref().fields().ok().and_then(|fields| {
                    (fields.schema_id == *content_ref.schema_id()
                        && fields.content_digest == *content_ref.content_digest())
                    .then_some(object.bytes())
                })
            })
            .ok_or(StoreError::CorruptFactHistory)
    }

    fn rehydrate_candidates(&self) -> Result<Vec<FactCandidate<ScannedFact>>> {
        if !self.selectable {
            return Ok(Vec::new());
        }
        let publication = Arc::clone(&self.source_objects);
        let transition = self
            .producer_view
            .transition_entries()
            .find(|entry| entry.transition_ref() == &self.transition_ref)
            .ok_or(StoreError::CorruptFactHistory)?;
        let node_id = transition.transition().fields()?.node_id;
        let node = self
            .producer_view
            .certified_spec()
            .nodes()
            .iter()
            .find(|node| node.node_id() == &node_id)
            .ok_or(StoreError::CorruptFactHistory)?;
        let slots = node.settlement_contract().fact_slots();
        let mut counts = vec![0_u32; slots.len()];
        let mut previous_slot = None;
        for emission in &self.emissions {
            let emission = emission.fields()?;
            if previous_slot.is_some_and(|previous| previous > emission.fact_slot_ordinal) {
                return Err(StoreError::CorruptFactHistory);
            }
            previous_slot = Some(emission.fact_slot_ordinal);
            let slot_index = usize::try_from(emission.fact_slot_ordinal)
                .map_err(|_| StoreError::CorruptFactHistory)?;
            let slot = slots
                .get(slot_index)
                .filter(|slot| slot.fact_slot_ordinal() == emission.fact_slot_ordinal)
                .ok_or(StoreError::CorruptFactHistory)?;
            counts[slot_index] = counts[slot_index]
                .checked_add(1)
                .ok_or(StoreError::CorruptFactHistory)?;
            if counts[slot_index] > slot.maximum_emissions() {
                return Err(StoreError::CorruptFactHistory);
            }
        }
        if slots.iter().enumerate().any(|(index, slot)| {
            counts
                .get(index)
                .is_none_or(|count| *count < slot.minimum_emissions())
        }) {
            return Err(StoreError::CorruptFactHistory);
        }
        let mut candidates = Vec::with_capacity(self.emissions.len());
        for emission in &self.emissions {
            let emission = emission.fields()?;
            let slot = usize::try_from(emission.fact_slot_ordinal)
                .ok()
                .and_then(|index| slots.get(index))
                .ok_or(StoreError::CorruptFactHistory)?;
            if slot.fact_descriptor_ref() != &emission.fact_descriptor_ref {
                return Err(StoreError::CorruptFactHistory);
            }
            self.retained_content(&emission.fact_descriptor_ref)?;

            let claim_object = self.retained_value(&emission.claim_ref)?;
            validate_fact_component(
                &emission.claim_ref,
                self.producer_view.run_id(),
                &node_id,
                emission.emission_ordinal,
                FactValueComponent::Claim,
            )?;
            let claim = FactClaimEnvelope::strict_decode(claim_object.bytes())?;
            validate_value_content_ref(&emission.claim_ref, &claim.content_ref()?)?;
            let claim = claim.fields()?;
            if claim.fact_descriptor_ref != emission.fact_descriptor_ref {
                return Err(StoreError::CorruptFactHistory);
            }

            validate_spec_value_contract(slot.subject_contract(), &claim.subject_ref)?;
            validate_fact_component(
                &claim.subject_ref,
                self.producer_view.run_id(),
                &node_id,
                emission.emission_ordinal,
                FactValueComponent::Subject,
            )?;
            let subject_object = self.retained_value(&claim.subject_ref)?;
            let subject = FactSubject::from_canonical_json(subject_object.bytes())
                .map_err(|_| StoreError::CorruptFactHistory)?;
            validate_value_content_ref(
                &claim.subject_ref,
                &subject
                    .content_ref()
                    .map_err(|_| StoreError::CorruptFactHistory)?,
            )?;

            validate_spec_value_contract(slot.response_contract(), &claim.response_ref)?;
            validate_fact_component(
                &claim.response_ref,
                self.producer_view.run_id(),
                &node_id,
                emission.emission_ordinal,
                FactValueComponent::Response,
            )?;
            self.retained_value(&claim.response_ref)?;

            let content_identity = FactContentIdentityPreimage::new(
                &emission.fact_descriptor_ref,
                &claim.subject_ref,
                &claim.response_ref,
            )?
            .fact_content_identity()?;
            if content_identity != emission.fact_content_identity {
                return Err(StoreError::CorruptFactHistory);
            }
            let logical_identity = FactLogicalIdentityPreimage::new(
                &self.transition_ref,
                emission.emission_ordinal,
                &content_identity,
            )?
            .fact_logical_identity()?;
            let fact_ref = FactRef::new(&self.transition_ref, emission.emission_ordinal)?;
            let selected = SelectedFact::new(
                &fact_ref,
                &self.transition_ref,
                &emission.fact_descriptor_ref,
                &claim.subject_ref,
                &claim.response_ref,
                &content_identity,
            )?;
            let descriptor_ref = emission.fact_descriptor_ref.clone();
            candidates.push(FactCandidate::new(
                emission.fact_descriptor_ref,
                subject,
                content_identity,
                logical_identity,
                self.fact_order,
                ScannedFact {
                    selected,
                    source: Arc::new(CandidateFactSource {
                        descriptor_ref,
                        claim_ref: emission.claim_ref,
                        subject_ref: claim.subject_ref,
                        response_ref: claim.response_ref,
                        publication: Arc::clone(&publication),
                    }),
                },
            ));
        }
        Ok(candidates)
    }
}

fn validate_spec_value_contract(
    contract: &mfm_spec::v1::RetainedValueContract,
    value_ref: &ValueRef,
) -> Result<()> {
    let value = value_ref.fields()?;
    if contract.schema_id() == &value.schema_id
        && contract.semantic_type_id() == &value.semantic_type_id
        && contract.role() == &value.role
        && contract.media_type() == value.media_type
        && contract.evidence_contract_ref() == &value.evidence_contract_ref
    {
        Ok(())
    } else {
        Err(StoreError::CorruptFactHistory)
    }
}

fn validate_value_content_ref(
    value_ref: &ValueRef,
    content_ref: &mfm_ids::ContentRef,
) -> Result<()> {
    let fields = value_ref.fields()?;
    if fields.schema_id == *content_ref.schema_id()
        && fields.content_digest == *content_ref.content_digest()
    {
        Ok(())
    } else {
        Err(StoreError::CorruptFactHistory)
    }
}

fn validate_fact_component(
    value_ref: &ValueRef,
    run_id: &RunId,
    node_id: &mfm_ids::NodeId,
    emission_ordinal: u32,
    component: FactValueComponent,
) -> Result<()> {
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::TransitionFact {
            run_id: retained_run_id,
            node_id: retained_node_id,
            emission_ordinal: retained_ordinal,
            component: retained_component,
        } if retained_run_id == *run_id
            && retained_node_id == *node_id
            && retained_ordinal == emission_ordinal
            && retained_component == component =>
        {
            Ok(())
        }
        _ => Err(StoreError::CorruptFactHistory),
    }
}

/// Store-created verifier for one private dense keyset page.
///
/// A backend may inspect the requested range, but can return a page only by
/// passing every producer journal through this verifier.
pub struct FactScanPageVerifier {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    consuming_run_id: RunId,
    next_fact_order: u64,
    frontier_fact_order: u64,
}

impl FactScanPageVerifier {
    pub(super) fn new(
        store_identity: StoreIdentity,
        tenant_scope_id: TenantScopeId,
        consuming_run_id: RunId,
        next_fact_order: u64,
        frontier_fact_order: u64,
    ) -> Result<Self> {
        if next_fact_order == 0 || next_fact_order > frontier_fact_order {
            return Err(StoreError::FactScanBindingMismatch);
        }
        Ok(Self {
            store_identity,
            tenant_scope_id,
            consuming_run_id,
            next_fact_order,
            frontier_fact_order,
        })
    }

    /// Returns the exact authoritative store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the admitted tenant whose prefix must be scanned.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the consuming run that must be excluded.
    pub const fn consuming_run_id(&self) -> &RunId {
        &self.consuming_run_id
    }

    /// Returns the first dense publication order required by this page.
    pub const fn next_fact_order(&self) -> u64 {
        self.next_fact_order
    }

    /// Returns the inclusive frozen authorization frontier.
    pub const fn frontier_fact_order(&self) -> u64 {
        self.frontier_fact_order
    }

    /// Verifies one complete producer run and selects its exact publication.
    ///
    /// Durable backends call this with native producer rows and the exact
    /// reachable object closure loaded from the authoritative writer.
    pub fn verify_publication(
        &self,
        fact_order: u64,
        producer_run_id: RunId,
        commits: Vec<CommittedJournalCommit>,
        objects: Vec<CommittedObject>,
    ) -> Result<VerifiedFactPublication> {
        if fact_order < self.next_fact_order || fact_order > self.frontier_fact_order {
            return Err(StoreError::FactScanBindingMismatch);
        }
        let selectable = producer_run_id != self.consuming_run_id;
        let view = verify_offline_recorded_history(
            self.store_identity.clone(),
            self.tenant_scope_id.clone(),
            producer_run_id,
            commits,
            objects,
        )?;
        let mut matching_heads = view.journal().commits().iter().filter_map(|commit| {
            let fields = commit.envelope().fields().ok()?;
            match fields.core.tenant_fact_coordinate.fields().ok()? {
                TenantFactCoordinateFields::FactPublication {
                    tenant_scope_id,
                    fact_order: retained_order,
                } if tenant_scope_id == self.tenant_scope_id && retained_order == fact_order => {
                    commit.envelope().journal_head().ok()
                }
                TenantFactCoordinateFields::None
                | TenantFactCoordinateFields::FactPublication { .. }
                | TenantFactCoordinateFields::FactSelectionBarrier { .. } => None,
            }
        });
        let containing_head = matching_heads
            .next()
            .ok_or(StoreError::CorruptFactHistory)?;
        if matching_heads.next().is_some() {
            return Err(StoreError::CorruptFactHistory);
        }
        let (transition_ref, emissions) = {
            let mut matching_transitions = view
                .transition_entries()
                .filter(|entry| entry.containing_journal_head() == &containing_head);
            let transition = matching_transitions
                .next()
                .ok_or(StoreError::CorruptFactHistory)?;
            if matching_transitions.next().is_some() {
                return Err(StoreError::CorruptFactHistory);
            }
            (
                transition.transition_ref().clone(),
                transition.transition().fact_emissions()?,
            )
        };
        if emissions.is_empty()
            || emissions.iter().enumerate().any(|(expected, emission)| {
                emission.fields().ok().is_none_or(|fields| {
                    usize::try_from(fields.emission_ordinal).ok() != Some(expected)
                })
            })
        {
            return Err(StoreError::CorruptFactHistory);
        }
        let source_objects = publication_prefix_objects(&view, &containing_head)?;
        Ok(VerifiedFactPublication {
            fact_order,
            selectable,
            source_objects,
            transition_ref,
            emissions,
            producer_view: view,
        })
    }

    /// Completes one bounded contiguous page.
    pub fn complete(self, publications: Vec<VerifiedFactPublication>) -> Result<FactScanPage> {
        if publications.is_empty() || publications.len() > FACT_SCAN_STEP_PUBLICATIONS {
            return Err(StoreError::CorruptFactHistory);
        }
        let mut expected_order = self.next_fact_order;
        let mut fact_count = 0usize;
        for publication in &publications {
            if publication.fact_order != expected_order
                || publication.fact_order > self.frontier_fact_order
            {
                return Err(StoreError::CorruptFactHistory);
            }
            fact_count = fact_count.checked_add(publication.emissions.len()).ok_or(
                StoreError::FactOrderOverflow {
                    tenant_scope_id: self.tenant_scope_id.clone(),
                },
            )?;
            if fact_count > FACT_SCAN_STEP_FACTS {
                return Err(StoreError::FactSelectionLimitExceeded);
            }
            expected_order =
                expected_order
                    .checked_add(1)
                    .ok_or(StoreError::FactOrderOverflow {
                        tenant_scope_id: self.tenant_scope_id.clone(),
                    })?;
        }
        Ok(FactScanPage {
            publications,
            next_fact_order: expected_order,
            frontier_fact_order: self.frontier_fact_order,
        })
    }
}

fn publication_prefix_objects(
    view: &VerifiedRunView,
    containing_head: &JournalHead,
) -> Result<Arc<VerifiedFactPublicationObjects>> {
    Ok(Arc::new(VerifiedFactPublicationObjects {
        objects: journal_prefix_objects(view, containing_head)?,
    }))
}

fn journal_prefix_objects(
    view: &VerifiedRunView,
    containing_head: &JournalHead,
) -> Result<Vec<CommittedObject>> {
    let mut reachable = BTreeSet::new();
    let mut found = false;
    for commit in view.journal().commits() {
        let fields = commit.envelope().fields()?;
        for intent in fields.core.artifact_admission_intents {
            reachable.insert(ObjectAuthorityKey::from_value_ref(
                &intent.fields()?.value_ref,
            )?);
        }
        if &commit.envelope().journal_head()? == containing_head {
            found = true;
            break;
        }
    }
    if !found {
        return Err(StoreError::CorruptFactHistory);
    }
    let objects = reachable
        .into_iter()
        .map(|key| {
            view.object(&key)
                .cloned()
                .ok_or(StoreError::CorruptFactHistory)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(objects)
}

/// One private verified scan page.
pub struct FactScanPage {
    publications: Vec<VerifiedFactPublication>,
    next_fact_order: u64,
    frontier_fact_order: u64,
}

impl FactScanPage {
    pub(super) fn into_parts(self) -> (Vec<VerifiedFactPublication>, u64, bool) {
        let complete = self.next_fact_order > self.frontier_fact_order;
        (self.publications, self.next_fact_order, complete)
    }
}

impl FactScanSession {
    fn new(permit: FactScanPermit) -> Self {
        let accumulators = fact_accumulators(permit.request());
        Self {
            permit,
            next_fact_order: 1,
            accumulators,
        }
    }

    fn consume_page(mut self, page: FactScanPage) -> Result<FactScanStep> {
        let (publications, next_fact_order, complete) = page.into_parts();
        consume_fact_publications(&mut self.accumulators, publications)?;
        self.next_fact_order = next_fact_order;
        if complete {
            self.finish().map(Box::new).map(FactScanStep::Complete)
        } else {
            Ok(FactScanStep::More(Box::new(self)))
        }
    }

    fn finish(self) -> Result<CompletedFactScan> {
        let evaluation = finish_fact_response(
            self.permit.request(),
            self.permit.frontier(),
            self.accumulators,
        )?;
        Ok(CompletedFactScan {
            permit: self.permit,
            response: evaluation.response,
            sources: evaluation.sources,
        })
    }
}

fn fact_accumulators(request: &FactSelectionRequest) -> Vec<FactTopK<ScannedFact>> {
    request
        .queries()
        .iter()
        .cloned()
        .map(FactTopK::new)
        .collect()
}

fn consume_fact_publications(
    accumulators: &mut [FactTopK<ScannedFact>],
    publications: Vec<VerifiedFactPublication>,
) -> Result<()> {
    for publication in publications {
        for candidate in publication.rehydrate_candidates()? {
            for accumulator in &mut *accumulators {
                accumulator.consider(candidate.clone());
            }
        }
    }
    Ok(())
}

fn finish_fact_response(
    request: &FactSelectionRequest,
    frontier: &TenantFactFrontier,
    accumulators: Vec<FactTopK<ScannedFact>>,
) -> Result<FactScanEvaluation> {
    let request_digest = request
        .request_digest()
        .map_err(|_| StoreError::JournalContract)?;
    let mut candidate_sources = Vec::<Arc<CandidateFactSource>>::new();
    let results = accumulators
        .into_iter()
        .enumerate()
        .map(|(ordinal, accumulator)| {
            let ordinal =
                u32::try_from(ordinal).map_err(|_| StoreError::FactSelectionLimitExceeded)?;
            let selected = accumulator
                .finish()
                .into_iter()
                .map(FactCandidate::into_value)
                .map(|scanned| {
                    candidate_sources.push(scanned.source);
                    scanned.selected
                })
                .collect::<Vec<_>>();
            FactSelectionResult::new(ordinal, &selected).map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    let response = FactSelectionResponse::new(&request_digest, frontier, &results)?;
    response.validate_request(request)?;
    let sources = verify_selected_fact_sources(&candidate_sources)?;
    Ok(FactScanEvaluation { response, sources })
}

async fn scan_verified_fact_response<B: FactScanBackend>(
    store: &B,
    store_identity: &StoreIdentity,
    tenant_scope_id: &TenantScopeId,
    consuming_run_id: &RunId,
    request: &FactSelectionRequest,
    frontier: &TenantFactFrontier,
) -> std::result::Result<FactScanEvaluation, B::Error> {
    let frontier_fact_order = frontier
        .fields()
        .map_err(StoreError::from)
        .map_err(B::Error::from)?
        .fact_order;
    let mut accumulators = fact_accumulators(request);
    if frontier_fact_order == 0 {
        return finish_fact_response(request, frontier, accumulators).map_err(B::Error::from);
    }

    let mut next_fact_order = 1;
    loop {
        let verifier = FactScanPageVerifier::new(
            store_identity.clone(),
            tenant_scope_id.clone(),
            consuming_run_id.clone(),
            next_fact_order,
            frontier_fact_order,
        )
        .map_err(B::Error::from)?;
        let page = store.backend_fact_scan_page(verifier).await?;
        let (publications, next, complete) = page.into_parts();
        consume_fact_publications(&mut accumulators, publications).map_err(B::Error::from)?;
        if complete {
            return finish_fact_response(request, frontier, accumulators).map_err(B::Error::from);
        }
        next_fact_order = next;
    }
}

/// Immutable private routing row coupled atomically to a reserved observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedFactScanAttestation {
    authorization_ref: AuthorizationRef,
    attestation_ref: ValueRef,
    observation_ref: ObservationRef,
    containing_journal_head: JournalHead,
}

impl PersistedFactScanAttestation {
    /// Reconstructs one backend row for store-owned validation.
    #[doc(hidden)]
    pub fn from_persisted(
        authorization_ref: AuthorizationRef,
        attestation_ref: ValueRef,
        observation_ref: ObservationRef,
        containing_journal_head: JournalHead,
    ) -> Self {
        Self {
            authorization_ref,
            attestation_ref,
            observation_ref,
            containing_journal_head,
        }
    }

    /// Returns the unique reserved authorization key.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        &self.authorization_ref
    }

    /// Returns the exact retained scan-attestation authority.
    pub const fn attestation_ref(&self) -> &ValueRef {
        &self.attestation_ref
    }

    /// Returns the derived committed observation.
    pub const fn observation_ref(&self) -> &ObservationRef {
        &self.observation_ref
    }

    /// Returns the commit that atomically published the observation and row.
    pub const fn containing_journal_head(&self) -> &JournalHead {
        &self.containing_journal_head
    }
}

/// Pending private attestation routing owned by a specialized observation.
pub struct PendingFactScanAttestation {
    authorization_ref: AuthorizationRef,
    response_ref: ValueRef,
    attestation_ref: ValueRef,
}

impl PendingFactScanAttestation {
    pub(super) const fn new(
        authorization_ref: AuthorizationRef,
        response_ref: ValueRef,
        attestation_ref: ValueRef,
    ) -> Self {
        Self {
            authorization_ref,
            response_ref,
            attestation_ref,
        }
    }

    /// Reconstructs the exact private row expected for an already assigned commit.
    #[doc(hidden)]
    pub fn persisted_for_commit(
        &self,
        commit: &CommittedJournalCommit,
    ) -> Result<PersistedFactScanAttestation> {
        persisted_attestation_for_commit(
            self.authorization_ref.clone(),
            self.response_ref.clone(),
            self.attestation_ref.clone(),
            commit,
        )
    }

    /// Assigns immutable observation and containing-commit coordinates.
    pub fn assign(self, assigned: &AssignedJournalAppend) -> Result<PersistedFactScanAttestation> {
        persisted_attestation_for_commit(
            self.authorization_ref,
            self.response_ref,
            self.attestation_ref,
            assigned.commit(),
        )
    }
}

fn persisted_attestation_for_commit(
    authorization_ref: AuthorizationRef,
    response_ref: ValueRef,
    attestation_ref: ValueRef,
    commit: &CommittedJournalCommit,
) -> Result<PersistedFactScanAttestation> {
    let envelope = commit.envelope().fields()?;
    let record = commit.records().first().ok_or(StoreError::EmptyJournal)?;
    if commit.records().len() != 1 {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let observation = match record.candidate().fields()?.payload.fields()? {
        RunJournalRecordFields::ExternalAccessObserved(observation) => observation,
        RunJournalRecordFields::RunAdmitted(_)
        | RunJournalRecordFields::StateTransitionCommitted(_)
        | RunJournalRecordFields::ExternalAccessAuthorized(_)
        | RunJournalRecordFields::RunClosed(_) => {
            return Err(StoreError::FactScanBindingMismatch);
        }
    };
    let observation_fields = observation.fields()?;
    let retained_response_ref = match observation_fields.outcome.fields()? {
        ObservationOutcomeFields::Returned { result_ref } => result_ref,
        ObservationOutcomeFields::DidNotEnter { .. }
        | ObservationOutcomeFields::Indeterminate { .. } => {
            return Err(StoreError::FactScanBindingMismatch);
        }
    };
    if observation_fields.authorization_ref != authorization_ref
        || retained_response_ref != response_ref
        || observation_fields
            .fact_selection_scan_attestation_ref
            .as_ref()
            != Some(&attestation_ref)
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    let observation_ref = ObservationRef::new(
        &record.record_ref(&envelope.core.run_id, envelope.core.run_sequence)?,
    )?;
    Ok(PersistedFactScanAttestation {
        authorization_ref,
        attestation_ref,
        observation_ref,
        containing_journal_head: commit.envelope().journal_head()?,
    })
}

/// Store-created target for loading private attestation rows for one run.
pub struct FactAttestationLoadVerifier {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
}

impl FactAttestationLoadVerifier {
    pub(super) const fn new(
        store_identity: StoreIdentity,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> Self {
        Self {
            store_identity,
            tenant_scope_id,
            run_id,
        }
    }

    /// Returns the exact qualified store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the consuming run whose rows may be loaded.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

/// Trusted durable-backend seam for same-store fact completeness.
pub trait FactScanBackend: RunJournalBackend {
    /// Reads and verifies one bounded dense keyset page from the authoritative writer.
    fn backend_fact_scan_page<'a>(
        &'a self,
        verifier: FactScanPageVerifier,
    ) -> AsyncStoreFuture<'a, FactScanPage, <Self as RunJournalBackend>::Error>;

    /// Loads immutable private attestation rows for one authorized consuming run.
    fn backend_load_fact_attestations<'a>(
        &'a self,
        verifier: FactAttestationLoadVerifier,
    ) -> AsyncStoreFuture<'a, Vec<PersistedFactScanAttestation>, <Self as RunJournalBackend>::Error>;
}

async fn verify_fact_selection_rows<B: FactScanBackend>(
    store: &B,
    view: &VerifiedRunView,
) -> std::result::Result<BTreeMap<Vec<u8>, FactSelectionCompleteness>, B::Error> {
    let row_verifier = FactAttestationLoadVerifier::new(
        view.store_identity().clone(),
        view.tenant_scope_id().clone(),
        view.run_id().clone(),
    );
    let rows = store.backend_load_fact_attestations(row_verifier).await?;
    let mut rows_by_authorization = BTreeMap::new();
    for row in rows {
        if rows_by_authorization
            .insert(row.authorization_ref().as_bytes().to_vec(), row)
            .is_some()
        {
            return Err(B::Error::from(StoreError::FactScanBindingMismatch));
        }
    }

    let mut verified_by_observation = BTreeMap::new();
    for entry in view.access_audit_entries() {
        let authorization_fields = entry
            .authorization()
            .fields()
            .map_err(StoreError::from)
            .map_err(B::Error::from)?;
        let reserved =
            authorization_fields.capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID;
        let returned = entry.observation().is_some_and(|(_, observation, _)| {
            observation.fields().is_ok_and(|fields| {
                matches!(
                    fields.outcome.fields(),
                    Ok(ObservationOutcomeFields::Returned { .. })
                )
            })
        });
        let row = rows_by_authorization.remove(entry.authorization_ref().as_bytes());
        if !reserved || !returned {
            if row.is_some() {
                return Err(B::Error::from(StoreError::FactScanBindingMismatch));
            }
            continue;
        }
        let row = row
            .ok_or(StoreError::FactScanBindingMismatch)
            .map_err(B::Error::from)?;
        let recorded = validate_recorded_fact_selection(view, &row).map_err(B::Error::from)?;
        let expected = scan_verified_fact_response(
            store,
            view.store_identity(),
            view.tenant_scope_id(),
            view.run_id(),
            &recorded.request,
            &recorded.frontier,
        )
        .await?;
        if expected.response.as_bytes() != recorded.response.as_bytes() {
            return Err(B::Error::from(StoreError::FactScanBindingMismatch));
        }
        let closure = derive_response_closure(
            view,
            &recorded.authorization_ref,
            &recorded.response_ref,
            recorded.response.as_bytes(),
            &expected.sources,
        )
        .map_err(B::Error::from)?;
        if closure.digest != recorded.response_closure_digest {
            return Err(B::Error::from(StoreError::InvalidSourceClosure));
        }
        let (observation_ref, _, _) = entry
            .observation()
            .ok_or(StoreError::FactScanBindingMismatch)
            .map_err(B::Error::from)?;
        let completeness = FactSelectionCompleteness::same_store_verified(
            &recorded.authorization_ref,
            &recorded.frontier,
        )
        .map_err(StoreError::from)
        .map_err(B::Error::from)?;
        if verified_by_observation
            .insert(observation_ref.as_bytes().to_vec(), completeness)
            .is_some()
        {
            return Err(B::Error::from(StoreError::FactScanBindingMismatch));
        }
    }
    if !rows_by_authorization.is_empty() {
        return Err(B::Error::from(StoreError::FactScanBindingMismatch));
    }
    Ok(verified_by_observation)
}

/// Dedicated same-store fact-selection operations.
pub trait FactSelectionStore: FactScanBackend {
    /// Appends the reserved authorization and returns its affine permit only on a fresh commit.
    fn append_fact_selection_authorization<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        append: AuthorizeExternalAccess,
        request: FactSelectionRequest,
    ) -> AsyncStoreFuture<'a, FactSelectionAuthorizationOutcome, <Self as RunJournalBackend>::Error>;

    /// Consumes one fresh affine permit and scans its authoritative prefix to completion.
    fn scan_fact_selection<'a>(
        &'a self,
        permit: FactScanPermit,
    ) -> AsyncStoreFuture<'a, CompletedFactScan, <Self as RunJournalBackend>::Error>;

    /// Rechecks exact returned observations selected for one live reducer invocation.
    ///
    /// References must be unique and in physical journal order. The result is
    /// intentionally unit: verification grants no reusable completeness token.
    fn verify_drive_fact_selection_observations<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        view: &'a VerifiedRunView,
        observation_refs: &'a [ObservationRef],
    ) -> AsyncStoreFuture<'a, (), <Self as RunJournalBackend>::Error>;

    /// Rechecks every retained reserved response against its fixed writer prefix.
    fn verify_fact_selection_completeness<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Replay>,
        view: &'a VerifiedRunView,
    ) -> AsyncStoreFuture<'a, VerifiedFactSelectionCompleteness, <Self as RunJournalBackend>::Error>;
}

impl<B: FactScanBackend> FactSelectionStore for B {
    fn append_fact_selection_authorization<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        append: AuthorizeExternalAccess,
        request: FactSelectionRequest,
    ) -> AsyncStoreFuture<'a, FactSelectionAuthorizationOutcome, <Self as RunJournalBackend>::Error>
    {
        let store_identity = self.store_authority_context().store_identity().clone();
        let checked = (|| {
            let fields = append.authorization().fields()?;
            let request_fields = fields.request_ref.fields()?;
            let request_ref = request
                .content_ref()
                .map_err(|_| StoreError::JournalContract)?;
            if fields.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID
                || request_fields.schema_id != *request_ref.schema_id()
                || request_fields.content_digest != *request_ref.content_digest()
            {
                return Err(StoreError::FactScanBindingMismatch);
            }
            Ok(())
        })();
        if let Err(error) = checked {
            return Box::pin(async move { Err(error.into()) });
        }
        let future = RunJournalStore::append(
            self,
            authority,
            PreparedJournalAppend::AuthorizeExternalAccess(append),
        );
        Box::pin(async move {
            let outcome = future.await?;
            let outcome = match outcome {
                AppendOutcome::NewlyAppended(NewlyAppended::Authorization(authorization)) => {
                    let permit = FactScanPermit::from_new_authorization(
                        store_identity,
                        *authorization,
                        request,
                    )
                    .map_err(B::Error::from)?;
                    let view = RunJournalStore::load_committed_journal(self, authority)
                        .await?
                        .verify_recorded_history()
                        .map_err(B::Error::from)?;
                    qualify_fact_scan(&view, permit.authorization_ref(), Some(permit.request()))
                        .map_err(B::Error::from)?;
                    FactSelectionAuthorizationOutcome::NewlyAuthorized(Box::new(permit))
                }
                AppendOutcome::NewlyAppended(
                    NewlyAppended::RunAdmitted(_)
                    | NewlyAppended::Transition(_)
                    | NewlyAppended::Observation(_),
                ) => return Err(StoreError::FactScanBindingMismatch.into()),
                AppendOutcome::AlreadyCommitted(committed) => {
                    FactSelectionAuthorizationOutcome::AlreadyCommitted(committed)
                }
                AppendOutcome::Rejected(rejection) => {
                    FactSelectionAuthorizationOutcome::Rejected(rejection)
                }
                AppendOutcome::OutcomeUnknown => FactSelectionAuthorizationOutcome::OutcomeUnknown,
            };
            Ok(outcome)
        })
    }

    fn scan_fact_selection<'a>(
        &'a self,
        permit: FactScanPermit,
    ) -> AsyncStoreFuture<'a, CompletedFactScan, <Self as RunJournalBackend>::Error> {
        Box::pin(async move {
            let frontier = permit
                .frontier()
                .fields()
                .map_err(StoreError::from)
                .map_err(B::Error::from)?
                .fact_order;
            let mut session = FactScanSession::new(permit);
            if frontier == 0 {
                return session.finish().map_err(B::Error::from);
            }
            loop {
                let verifier = FactScanPageVerifier::new(
                    session.permit.store_identity().clone(),
                    session.permit.tenant_scope_id().clone(),
                    session.permit.consuming_run_id().clone(),
                    session.next_fact_order,
                    frontier,
                )
                .map_err(B::Error::from)?;
                let page = self.backend_fact_scan_page(verifier).await?;
                match session.consume_page(page).map_err(B::Error::from)? {
                    FactScanStep::More(next) => session = *next,
                    FactScanStep::Complete(completed) => return Ok(*completed),
                }
            }
        })
    }

    fn verify_drive_fact_selection_observations<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        view: &'a VerifiedRunView,
        observation_refs: &'a [ObservationRef],
    ) -> AsyncStoreFuture<'a, (), <Self as RunJournalBackend>::Error> {
        let target = match self.store_authority_context().validate_run(authority) {
            Ok(target) => target,
            Err(error) => return Box::pin(async move { Err(error.into()) }),
        };
        if view.store_identity() != self.store_authority_context().store_identity()
            || view.tenant_scope_id() != &target.tenant_scope_id
            || view.run_id() != &target.run_id
        {
            return Box::pin(
                async move { Err(StoreError::AccessDenied { purpose: "drive" }.into()) },
            );
        }
        Box::pin(async move {
            let mut verified_by_observation = verify_fact_selection_rows(self, view).await?;
            let mut journal_order_by_observation = BTreeMap::new();
            for entry in view.access_audit_entries() {
                let Some((observation_ref, _, containing_head)) = entry.observation() else {
                    continue;
                };
                let run_sequence = containing_head
                    .fields()
                    .map_err(StoreError::from)
                    .map_err(B::Error::from)?
                    .run_sequence;
                if journal_order_by_observation
                    .insert(observation_ref.as_bytes().to_vec(), run_sequence)
                    .is_some()
                {
                    return Err(StoreError::FactScanBindingMismatch.into());
                }
            }

            let mut previous_order = None;
            for observation_ref in observation_refs {
                let journal_order = journal_order_by_observation
                    .get(observation_ref.as_bytes())
                    .copied()
                    .ok_or(StoreError::FactScanBindingMismatch)
                    .map_err(B::Error::from)?;
                if previous_order.is_some_and(|previous| previous >= journal_order) {
                    return Err(StoreError::FactScanBindingMismatch.into());
                }
                previous_order = Some(journal_order);
                verified_by_observation
                    .remove(observation_ref.as_bytes())
                    .ok_or(StoreError::FactScanBindingMismatch)
                    .map_err(B::Error::from)?;
            }
            Ok(())
        })
    }

    fn verify_fact_selection_completeness<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Replay>,
        view: &'a VerifiedRunView,
    ) -> AsyncStoreFuture<'a, VerifiedFactSelectionCompleteness, <Self as RunJournalBackend>::Error>
    {
        let target = match self.store_authority_context().validate_run(authority) {
            Ok(target) => target,
            Err(error) => return Box::pin(async move { Err(error.into()) }),
        };
        if view.store_identity() != self.store_authority_context().store_identity()
            || view.tenant_scope_id() != &target.tenant_scope_id
            || view.run_id() != &target.run_id
        {
            return Box::pin(
                async move { Err(StoreError::AccessDenied { purpose: "replay" }.into()) },
            );
        }
        Box::pin(async move {
            let mut verified_by_observation = verify_fact_selection_rows(self, view).await?;
            let fact_observations = verified_by_observation
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut completeness = Vec::new();
            for transition in view.transition_entries() {
                let fields = transition
                    .transition()
                    .fields()
                    .map_err(StoreError::from)
                    .map_err(B::Error::from)?;
                let mfm_journal::v1::TransitionBodyFields::ReadSettled {
                    consumed_observation_ref,
                    ..
                } = fields
                    .body
                    .fields()
                    .map_err(StoreError::from)
                    .map_err(B::Error::from)?
                else {
                    continue;
                };
                if !fact_observations.contains(consumed_observation_ref.as_bytes()) {
                    continue;
                }
                completeness.push(
                    verified_by_observation
                        .remove(consumed_observation_ref.as_bytes())
                        .ok_or(StoreError::FactScanBindingMismatch)
                        .map_err(B::Error::from)?,
                );
            }
            Ok(VerifiedFactSelectionCompleteness {
                store_identity: view.store_identity().clone(),
                tenant_scope_id: view.tenant_scope_id().clone(),
                run_id: view.run_id().clone(),
                journal_head: view.journal_head().clone(),
                fact_selections: completeness,
            })
        })
    }
}

#[cfg(test)]
#[path = "fact_scan_tests.rs"]
mod tests;
