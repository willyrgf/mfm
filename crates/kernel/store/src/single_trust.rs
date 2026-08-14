//! Callback-free Store ownership for the strict three-family run protocol.
//!
//! The Store is the only owner of durable semantic evidence.  It compares one exact head,
//! validates one complete frame, and appends it atomically.  It never owns a State callback or a
//! provider handle; a successful preparation append is merely a fact from which Runtime may mint
//! one direct-new execution owner.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use mfm_canonical::raw_content_digest;
use mfm_ids::{
    short_stable_id_fragment, AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, RunId,
    SchemaId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::single_trust::{
    ImmutableObject, PreparationRef, RunFrame, RunRecord, StateConcluded, StateOutcome,
    StatePrepared, ValueRef,
};
use mfm_program::{
    Declaration, ExecutionMode, Program, ProgramCatalog, ProgramDocument, ProgramIngress,
    StateDeclaration,
};
use mfm_values::MfmValue;

const PROGRAM_OBJECT_TYPE: &str = "mfm.program";

/// Process-local identity of one opened semantic Store.
///
/// The value is intentionally private to Store.  Persisted scope, epoch, and tenant fields
/// identify a durable partition, but they are not enough to move an in-memory owner between two
/// independent catalog/Store openings.
#[derive(Debug)]
pub(crate) struct StoreBrand;

/// Result of one mechanical exact-head append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendDisposition {
    /// The exact candidate crossed the Store durability point.
    NewlyCommitted {
        /// The newly committed one-based sequence.
        sequence: u64,
    },
    /// The exact physical request was already found in the retained prefix.
    Found {
        /// The found one-based sequence.
        sequence: u64,
    },
    /// The candidate was evaluated against an old head.
    StaleHead {
        /// The current one-based head sequence.
        actual_sequence: u64,
    },
    /// The database outcome is unknown after submission; the exact append identity must be
    /// resolved before another attempt.
    AcknowledgementUnknown,
}

/// Redaction-safe Store semantic error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The candidate does not belong to this Store identity or tenant partition.
    #[error("store identity is invalid")]
    Identity,
    /// The candidate frame or record violates the three-family protocol.
    #[error("store record is invalid")]
    InvalidRecord,
    /// The candidate conflicts with an existing physical or logical record.
    #[error("store record conflicts with retained history")]
    Conflict,
    /// The run was not found in this tenant partition.
    #[error("run was not found")]
    NotFound,
    /// A frame or run bound was exceeded.
    #[error("store capacity bound exceeded")]
    Capacity,
    /// The selected occurrence has already concluded or another preparation is selected.
    #[error("run occurrence is not actionable")]
    NotActionable,
    /// Retained records do not form one valid sequential prefix.
    #[error("retained run history is invalid")]
    InvalidHistory,
    /// The independent tenant fact-publication head changed while a conclusion was prepared.
    #[error("fact publication frontier changed")]
    FactFrontierChanged,
}

/// Result type for the strict Store API.
pub type Result<T> = std::result::Result<T, StoreError>;

/// A callback-free retained run qualified against one Store scope and writer epoch.
#[derive(Debug, Clone)]
pub struct QualifiedRun {
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    run_id: RunId,
    frames: Vec<RunFrame>,
    head_digest: ContentDigest,
    total_frame_bytes: usize,
    object_map: BTreeMap<ContentRef, mfm_journal::single_trust::ImmutableObject>,
    logical_keys: BTreeSet<mfm_journal::single_trust::RecordLogicalKey>,
    reserved_conclusion_bytes: u64,
    store_brand: Option<Arc<StoreBrand>>,
}

impl QualifiedRun {
    fn new(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        frames: Vec<RunFrame>,
    ) -> Result<Self> {
        if frames.is_empty() || frames.len() > mfm_journal::single_trust::MAX_RUN_FRAMES {
            return Err(StoreError::Capacity);
        }
        let admission = match frames.first().map(RunFrame::record) {
            Some(RunRecord::RunAdmitted(value)) => value,
            _ => return Err(StoreError::InvalidRecord),
        };
        if admission.store_scope_id() != &scope
            || admission.store_epoch() != epoch
            || admission.tenant_scope_id() != &tenant
        {
            return Err(StoreError::Identity);
        }
        let mut total_frame_bytes = 0usize;
        let mut object_refs = BTreeSet::new();
        let mut logical_keys = BTreeSet::new();
        let mut object_map = BTreeMap::new();
        let mut head_digest = None;
        for (index, frame) in frames.iter().enumerate() {
            frame.validate().map_err(|_| StoreError::InvalidRecord)?;
            if frame.expected_sequence() != index as u64 + 1
                || frame.run_id() != admission.run_id()
                || frame.store_scope_id() != &scope
                || frame.store_epoch() != epoch
                || frame.record().is_admission() != (index == 0)
            {
                return Err(StoreError::InvalidRecord);
            }
            total_frame_bytes = total_frame_bytes
                .checked_add(
                    frame
                        .canonical_bytes()
                        .map_err(|_| StoreError::InvalidRecord)?
                        .as_bytes()
                        .len(),
                )
                .ok_or(StoreError::Capacity)?;
            for object in frame.objects() {
                if let Some(previous) = object_map.get(object.content_ref()) {
                    if previous != object {
                        return Err(StoreError::InvalidHistory);
                    }
                }
                object_map.insert(object.content_ref().clone(), object.clone());
                object_refs.insert(object.content_ref().clone());
            }
            head_digest = Some(
                frame
                    .head_digest(head_digest.as_ref())
                    .map_err(|_| StoreError::InvalidHistory)?,
            );
            if total_frame_bytes > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES
                || object_refs.len() > mfm_journal::single_trust::MAX_RUN_OBJECTS
                || !logical_keys.insert(frame.record().logical_key())
            {
                return Err(StoreError::InvalidHistory);
            }
        }
        let reserved = reserved_conclusion_bytes(&frames)?;
        if u64::try_from(total_frame_bytes)
            .map_err(|_| StoreError::Capacity)?
            .checked_add(reserved)
            .ok_or(StoreError::Capacity)?
            > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES as u64
        {
            return Err(StoreError::Capacity);
        }
        validate_prefix_structure(&frames)?;
        validate_object_reachability(&scope, epoch, &tenant, &frames)?;
        Ok(Self {
            scope,
            epoch,
            tenant,
            run_id: admission.run_id().clone(),
            frames,
            head_digest: head_digest.ok_or(StoreError::InvalidHistory)?,
            total_frame_bytes,
            object_map,
            logical_keys,
            reserved_conclusion_bytes: reserved,
            store_brand: None,
        })
    }

    /// Qualifies one complete retained prefix under one Store identity.
    pub(crate) fn qualify_prefix(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        frames: Vec<RunFrame>,
    ) -> Result<Self> {
        Self::new(scope, epoch, tenant, frames)
    }

    /// Validates one complete prefix without returning a transferable Store owner.
    pub fn validate_prefix(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        frames: Vec<RunFrame>,
    ) -> Result<()> {
        Self::new(scope, epoch, tenant, frames).map(|_| ())
    }

    pub(crate) fn bind_store(&mut self, brand: Arc<StoreBrand>) {
        self.store_brand = Some(brand);
    }

    pub(crate) fn belongs_to_store(&self, brand: &Arc<StoreBrand>) -> bool {
        self.store_brand
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(owner, brand))
    }

    /// Returns the qualified Store scope.
    pub const fn scope(&self) -> &StoreScopeId {
        &self.scope
    }

    /// Returns the qualified writer epoch.
    pub const fn epoch(&self) -> StoreEpoch {
        self.epoch
    }

    /// Returns the fixed tenant partition.
    pub const fn tenant(&self) -> &TenantScopeId {
        &self.tenant
    }

    /// Returns the complete retained prefix.
    pub fn frames(&self) -> &[RunFrame] {
        &self.frames
    }

    /// Returns the current exact head sequence.
    pub fn head_sequence(&self) -> u64 {
        self.frames.len() as u64
    }

    pub(crate) const fn total_frame_bytes(&self) -> usize {
        self.total_frame_bytes
    }

    /// Returns the recursive head content address for this qualified prefix.
    pub fn head_digest(&self) -> Result<ContentDigest> {
        Ok(self.head_digest.clone())
    }

    /// Returns the recursive head content identity at one retained record sequence.
    pub fn head_digest_at(&self, sequence: u64) -> Result<ContentDigest> {
        if sequence == 0
            || usize::try_from(sequence)
                .ok()
                .is_none_or(|index| index > self.frames.len())
        {
            return Err(StoreError::InvalidHistory);
        }
        let mut head = None;
        for frame in self
            .frames
            .iter()
            .take(usize::try_from(sequence).map_err(|_| StoreError::InvalidHistory)?)
        {
            head = Some(
                frame
                    .head_digest(head.as_ref())
                    .map_err(|_| StoreError::InvalidHistory)?,
            );
        }
        head.ok_or(StoreError::InvalidHistory)
    }

    /// Returns the complete conclusion capacity currently reserved by unresolved preparations.
    pub fn reserved_conclusion_bytes(&self) -> Result<u64> {
        Ok(self.reserved_conclusion_bytes)
    }

    /// Returns the run identity carried by the admitted record.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns whether one occurrence already has a durable conclusion.
    pub fn has_conclusion(
        &self,
        occurrence: &mfm_journal::single_trust::SequentialControlAddress,
    ) -> bool {
        self.frames.iter().any(|frame| {
            matches!(
                frame.record(),
                RunRecord::StateConcluded(conclusion) if conclusion.occurrence() == occurrence
            )
        })
    }

    /// Returns the selected preparation for one occurrence, if any.
    pub fn selected_preparation(
        &self,
        occurrence: &mfm_journal::single_trust::SequentialControlAddress,
    ) -> Option<(&StatePrepared, PreparationRef)> {
        self.frames
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, frame)| {
                let prepared = match frame.record() {
                    RunRecord::StatePrepared(value) if value.occurrence() == occurrence => value,
                    _ => return None,
                };
                Some((
                    prepared,
                    PreparationRef::new(
                        self.run_id().clone(),
                        frame.expected_sequence(),
                        index as u32,
                    ),
                ))
            })
    }

    /// Extends this already-qualified prefix with one Store-validated append without re-folding
    /// or re-encoding the retained prefix. Cold ingress uses [`Self::qualify_prefix`]; this path
    /// is reserved for a Runtime owner that already holds the predecessor qualification.
    pub(crate) fn append_validated(mut self, frame: RunFrame) -> Result<Self> {
        frame.validate().map_err(|_| StoreError::InvalidRecord)?;
        if frame.run_id() != self.run_id()
            || frame.store_scope_id() != &self.scope
            || frame.store_epoch() != self.epoch
            || frame.expected_sequence() != self.head_sequence().saturating_add(1)
            || frame.record().is_admission()
        {
            return Err(StoreError::Identity);
        }
        validate_append(&self.frames, &frame)?;
        let frame_bytes = frame
            .canonical_bytes()
            .map_err(|_| StoreError::InvalidRecord)?
            .as_bytes()
            .len();
        let total_frame_bytes = self
            .total_frame_bytes
            .checked_add(frame_bytes)
            .ok_or(StoreError::Capacity)?;
        let mut object_map = self.object_map.clone();
        for object in frame.objects() {
            if let Some(previous) = object_map.get(object.content_ref()) {
                if previous != object {
                    return Err(StoreError::InvalidHistory);
                }
            }
            object_map.insert(object.content_ref().clone(), object.clone());
        }
        if frame
            .objects()
            .iter()
            .any(|object| object.object_type().as_str() == PROGRAM_OBJECT_TYPE)
        {
            return Err(StoreError::InvalidHistory);
        }
        validate_record_objects(&object_map, frame.record())?;
        if let RunRecord::StatePrepared(prepared) = frame.record() {
            if let (Some(request), Some(selection)) =
                (prepared.fact_request(), prepared.fact_selection())
            {
                validate_fact_pair(
                    &object_map,
                    request,
                    selection,
                    &self.scope,
                    self.epoch,
                    &self.tenant,
                )?;
            }
        }
        let mut reserved = self.reserved_conclusion_bytes;
        match frame.record() {
            RunRecord::StatePrepared(prepared) => {
                if let Some((previous, _)) = self.selected_preparation(prepared.occurrence()) {
                    reserved = reserved
                        .checked_sub(previous.maximum_conclusion_bytes())
                        .ok_or(StoreError::InvalidHistory)?;
                }
                reserved = reserved
                    .checked_add(prepared.maximum_conclusion_bytes())
                    .ok_or(StoreError::Capacity)?;
            }
            RunRecord::StateConcluded(StateConcluded::Access { occurrence, .. }) => {
                let (prepared, _) = self
                    .selected_preparation(occurrence)
                    .ok_or(StoreError::InvalidHistory)?;
                reserved = reserved
                    .checked_sub(prepared.maximum_conclusion_bytes())
                    .ok_or(StoreError::InvalidHistory)?;
            }
            RunRecord::StateConcluded(StateConcluded::Pure { .. }) | RunRecord::RunAdmitted(_) => {}
        }
        if total_frame_bytes
            .checked_add(usize::try_from(reserved).map_err(|_| StoreError::Capacity)?)
            .is_none_or(|bytes| bytes > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES)
            || object_map.len() > mfm_journal::single_trust::MAX_RUN_OBJECTS
        {
            return Err(StoreError::Capacity);
        }
        let head_digest = frame
            .head_digest(Some(&self.head_digest))
            .map_err(|_| StoreError::InvalidRecord)?;
        self.frames.push(frame.clone());
        self.logical_keys.insert(frame.record().logical_key());
        self.head_digest = head_digest;
        self.total_frame_bytes = total_frame_bytes;
        self.object_map = object_map;
        self.reserved_conclusion_bytes = reserved;
        Ok(self)
    }
}

/// Validates one complete retained prefix without returning a transferable Store owner.
pub fn validate_prefix(
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    frames: Vec<RunFrame>,
) -> Result<()> {
    QualifiedRun::new(scope, epoch, tenant, frames).map(|_| ())
}

/// Loads the exact Program document retained in an admitted run under one catalog.
///
/// Qualification first proves that the admission closure contains one canonical `mfm.program`
/// object matching the admission reference and entry point. This function then applies the
/// catalog's strict associations, so a document cannot become executable under a foreign catalog.
pub fn retained_program(run: &QualifiedRun, catalog: &ProgramCatalog) -> Result<Program> {
    let admission = admitted_record(run.frames())?;
    let object = admitted_program_object(run.frames())?;
    let program = ProgramIngress::new(catalog)
        .decode(object.canonical_json().as_bytes())
        .map_err(|_| StoreError::InvalidHistory)?;
    if program.program_ref().content_ref() != admission.program_ref()
        || program.document().entry_point_id() != admission.entry_point_id()
    {
        return Err(StoreError::InvalidHistory);
    }
    Ok(program)
}

/// Computes callback-free terminality for replay without minting mutation authority.
pub fn replay_terminality(run: &QualifiedRun, catalog: &ProgramCatalog) -> Result<bool> {
    let program = retained_program(run, catalog)?;
    let selection = RunReducer::new(program.document().clone()).reduce(run)?;
    Ok(matches!(
        selection.action(),
        RunAction::ZeroStateTerminal { .. } | RunAction::Terminal { .. } | RunAction::Failed { .. }
    ))
}

/// A secret-free Store-owned owner for one conclusion append.
#[derive(Debug)]
pub(crate) struct PreparedConclusion {
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    frame: RunFrame,
    document: ProgramDocument,
    expected_action: RunAction,
    fact_rebind_ordinal: u16,
    store_brand: Option<Arc<StoreBrand>>,
}

impl PreparedConclusion {
    fn new(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        frame: RunFrame,
        document: ProgramDocument,
        expected_action: RunAction,
    ) -> Self {
        Self {
            scope,
            epoch,
            tenant,
            frame,
            document,
            expected_action,
            fact_rebind_ordinal: 0,
            store_brand: None,
        }
    }

    pub(crate) fn bind_fact_publication(&mut self, publication_sequence: u64) -> Result<()> {
        let proposal_set_ref = self
            .frame
            .record()
            .fact_proposals()
            .cloned()
            .ok_or(StoreError::InvalidRecord)?;
        let publication =
            mfm_journal::single_trust::FactPublication::new(publication_sequence, proposal_set_ref)
                .map_err(|_| StoreError::InvalidRecord)?;
        let conclusion = match self.frame.record() {
            RunRecord::StateConcluded(conclusion) => conclusion.clone(),
            _ => return Err(StoreError::InvalidRecord),
        };
        let conclusion = conclusion
            .with_fact_publication(Some(publication))
            .map_err(|_| StoreError::InvalidRecord)?;
        self.frame = RunFrame::new(
            self.frame.run_id().clone(),
            self.frame.store_scope_id().clone(),
            self.frame.store_epoch(),
            self.frame.expected_sequence(),
            self.frame.append_request_id().clone(),
            RunRecord::StateConcluded(conclusion),
            self.frame.objects().to_vec(),
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        Ok(())
    }

    pub(crate) fn rebind_fact_publication(&mut self) -> Result<()> {
        if self.frame.record().fact_publication().is_none() {
            return Ok(());
        }
        let conclusion = match self.frame.record() {
            RunRecord::StateConcluded(conclusion) => conclusion.clone(),
            _ => return Err(StoreError::InvalidRecord),
        };
        let conclusion = conclusion
            .with_fact_publication(None)
            .map_err(|_| StoreError::InvalidRecord)?;
        let ordinal = self
            .fact_rebind_ordinal
            .checked_add(1)
            .ok_or(StoreError::Capacity)?;
        self.frame = RunFrame::new(
            self.frame.run_id().clone(),
            self.frame.store_scope_id().clone(),
            self.frame.store_epoch(),
            self.frame.expected_sequence(),
            AppendRequestId::new(format!(
                "conclusion-{}-{}-fact-{}",
                short_stable_id_fragment(self.frame.run_id().as_str(), 96),
                self.frame.expected_sequence(),
                ordinal
            ))
            .map_err(|_| StoreError::InvalidRecord)?,
            RunRecord::StateConcluded(conclusion),
            self.frame.objects().to_vec(),
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        self.fact_rebind_ordinal = ordinal;
        Ok(())
    }

    /// Returns the exact candidate frame without exposing mutable append authority.
    pub const fn frame(&self) -> &RunFrame {
        &self.frame
    }

    /// Returns the durable run identity carried by this conclusion owner.
    pub const fn run_id(&self) -> &RunId {
        self.frame.run_id()
    }

    pub(crate) const fn expected_action(&self) -> &RunAction {
        &self.expected_action
    }

    pub(crate) const fn document(&self) -> &ProgramDocument {
        &self.document
    }

    pub(crate) fn bind_store(&mut self, brand: Arc<StoreBrand>) {
        self.store_brand = Some(brand);
    }

    pub(crate) fn belongs_to(
        &self,
        identity: &crate::backend::StructuredStoreIdentity,
        brand: &Arc<StoreBrand>,
    ) -> bool {
        self.scope == *identity.scope()
            && self.epoch == identity.epoch()
            && self.tenant == *identity.tenant()
            && self
                .store_brand
                .as_ref()
                .is_some_and(|owner| Arc::ptr_eq(owner, brand))
    }

    #[cfg(test)]
    pub(crate) fn commit(self, store: &SemanticStore) -> Result<AppendDisposition> {
        if self.scope != store.scope || self.epoch != store.epoch || self.tenant != store.tenant {
            return Err(StoreError::Identity);
        }
        store.append(self.frame)
    }
}

/// Result of a preparation append.  Runtime may create a call only for `NewlyCommitted`.
#[derive(Debug)]
pub(crate) struct PreparationAppend {
    disposition: AppendDisposition,
    preparation: Option<PreparationRef>,
    fact_continuation: Option<FactContinuation>,
}

/// One-use preparation-bound prior-fact continuation.
///
/// It carries only the qualified structural identity. The selected fact object itself remains in
/// the append-atomic object closure and is never fetched by Runtime or a State implementation.
#[derive(Debug)]
pub struct FactContinuation {
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    run_id: RunId,
    preparation: PreparationRef,
    request: ValueRef,
    selection_ref: ValueRef,
    selection: mfm_facts::FactSelection,
    store_brand: Option<Arc<StoreBrand>>,
}

impl FactContinuation {
    #[allow(clippy::too_many_arguments)]
    fn new(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        run_id: RunId,
        preparation: PreparationRef,
        request: ValueRef,
        selection_ref: ValueRef,
        selection: mfm_facts::FactSelection,
    ) -> Self {
        Self {
            scope,
            epoch,
            tenant,
            run_id,
            preparation,
            request,
            selection_ref,
            selection,
            store_brand: None,
        }
    }

    pub(crate) fn bind_store(&mut self, brand: Arc<StoreBrand>) {
        self.store_brand = Some(brand);
    }

    pub(crate) fn belongs_to_store(&self, brand: &Arc<StoreBrand>) -> bool {
        self.store_brand
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(owner, brand))
    }

    /// Returns the fixed prior-fact request identity without exposing its bytes.
    pub const fn request(&self) -> &ValueRef {
        &self.request
    }

    /// Returns the Store-selected, callback-free fact response.
    pub const fn selection(&self) -> &mfm_facts::FactSelection {
        &self.selection
    }

    /// Returns the content identity of the selected response.
    pub const fn selection_ref(&self) -> &ValueRef {
        &self.selection_ref
    }

    /// Consumes the continuation into the callback-free selected response.
    pub fn into_selection(self) -> mfm_facts::FactSelection {
        self.selection
    }

    /// Returns the Store identity bound to the one-use continuation.
    pub const fn scope(&self) -> &StoreScopeId {
        &self.scope
    }

    /// Returns the writer epoch bound to the one-use continuation.
    pub const fn epoch(&self) -> StoreEpoch {
        self.epoch
    }

    /// Returns the tenant partition bound to the one-use continuation.
    pub const fn tenant(&self) -> &TenantScopeId {
        &self.tenant
    }

    /// Returns the run bound to the one-use continuation.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact preparation bound to the one-use continuation.
    pub const fn preparation(&self) -> &PreparationRef {
        &self.preparation
    }
}

impl PreparationAppend {
    pub(crate) fn bind_store(&mut self, brand: Arc<StoreBrand>) {
        if let Some(fact_continuation) = self.fact_continuation.as_mut() {
            fact_continuation.bind_store(brand);
        }
    }

    #[cfg(test)]
    pub(crate) fn with_disposition(self, disposition: AppendDisposition) -> Self {
        let direct_new = matches!(disposition, AppendDisposition::NewlyCommitted { .. });
        Self {
            disposition,
            preparation: matches!(
                disposition,
                AppendDisposition::NewlyCommitted { .. } | AppendDisposition::Found { .. }
            )
            .then_some(self.preparation)
            .flatten(),
            fact_continuation: direct_new.then_some(self.fact_continuation).flatten(),
        }
    }

    /// Returns the mechanical append disposition.
    pub const fn disposition(&self) -> AppendDisposition {
        self.disposition
    }

    /// Returns the Store-assigned preparation identity when the append is known durable.
    pub const fn preparation(&self) -> Option<&PreparationRef> {
        self.preparation.as_ref()
    }

    /// Consumes the append result's one-use prior-fact continuation.
    pub fn into_fact_continuation(self) -> Option<FactContinuation> {
        self.fact_continuation
    }
}

pub(crate) struct AccessPreparationCandidate {
    pub(crate) append: PreparationAppend,
    pub(crate) frame: Option<RunFrame>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_access_from_current(
    scope: &StoreScopeId,
    epoch: StoreEpoch,
    tenant: &TenantScopeId,
    current: &QualifiedRun,
    run_id: &RunId,
    document: &ProgramDocument,
    selected: &RunSelection,
    expected_sequence: u64,
    append_request_id: AppendRequestId,
    prepared: StatePrepared,
    objects: Vec<mfm_journal::single_trust::ImmutableObject>,
) -> Result<AccessPreparationCandidate> {
    if current.run_id() != run_id {
        return Err(StoreError::Identity);
    }
    let successor_sequence = expected_sequence
        .checked_add(1)
        .ok_or(StoreError::Capacity)?;
    let mut objects = objects;
    if !current
        .frames()
        .iter()
        .flat_map(|frame| frame.objects())
        .any(|object| object == &selected.latest_context_object)
        && !objects
            .iter()
            .any(|object| object == &selected.latest_context_object)
    {
        objects.push(selected.latest_context_object.clone());
    }
    let candidate_frame = RunFrame::new(
        run_id.clone(),
        scope.clone(),
        epoch,
        successor_sequence,
        append_request_id,
        RunRecord::StatePrepared(prepared.clone()),
        objects,
    )
    .map_err(|_| StoreError::Capacity)?;
    if let Some((index, existing)) = current
        .frames()
        .iter()
        .enumerate()
        .find(|(_, frame)| frame.append_request_id() == candidate_frame.append_request_id())
    {
        if existing != &candidate_frame {
            return Err(StoreError::Conflict);
        }
        return Ok(AccessPreparationCandidate {
            append: PreparationAppend {
                disposition: AppendDisposition::Found {
                    sequence: existing.expected_sequence(),
                },
                preparation: Some(PreparationRef::new(
                    run_id.clone(),
                    existing.expected_sequence(),
                    index as u32,
                )),
                fact_continuation: None,
            },
            frame: None,
        });
    }
    if current.head_sequence() != expected_sequence {
        return Ok(AccessPreparationCandidate {
            append: PreparationAppend {
                disposition: AppendDisposition::StaleHead {
                    actual_sequence: current.head_sequence(),
                },
                preparation: None,
                fact_continuation: None,
            },
            frame: None,
        });
    }
    let expected_input = match &selected.action {
        RunAction::ReadyAccess {
            occurrence, input, ..
        } if occurrence == prepared.occurrence() => Some(input),
        RunAction::WaitingPreparation { occurrence, .. } if occurrence == prepared.occurrence() => {
            current
                .selected_preparation(prepared.occurrence())
                .map(|(previous, _)| previous.input())
        }
        _ => None,
    };
    if expected_input != Some(prepared.input()) {
        return Err(StoreError::NotActionable);
    }
    let Some(Declaration::State(state)) = document.declaration(prepared.occurrence()) else {
        return Err(StoreError::InvalidRecord);
    };
    validate_preparation_contract(&prepared, state)?;
    if current.has_conclusion(prepared.occurrence()) {
        return Err(StoreError::NotActionable);
    }
    if let Some((previous, previous_ref)) = current.selected_preparation(prepared.occurrence()) {
        if !previous.mode().permits_replacement()
            || prepared.mode() != previous.mode()
            || prepared.replaces() != Some(&previous_ref)
            || prepared.preparation_ordinal() != previous.preparation_ordinal().saturating_add(1)
            || prepared.preparation_ordinal() >= prepared.mode().total_attempt_bound()
        {
            return Err(StoreError::NotActionable);
        }
    } else if prepared.preparation_ordinal() != 0 || prepared.replaces().is_some() {
        return Err(StoreError::InvalidRecord);
    }
    let fact_request = prepared.fact_request().cloned();
    let fact_selection = prepared.fact_selection().cloned();
    ensure_run_capacity(current.frames(), &candidate_frame)?;
    let mut candidate_prefix = current.frames().to_vec();
    candidate_prefix.push(candidate_frame.clone());
    validate_candidate_prefix(scope, epoch, tenant, &candidate_prefix)?;
    let record_ordinal = u32::try_from(expected_sequence).map_err(|_| StoreError::Capacity)?;
    let preparation = PreparationRef::new(run_id.clone(), successor_sequence, record_ordinal);
    let fact_continuation = match fact_request.zip(fact_selection) {
        None => None,
        Some((request, selection_ref)) => {
            let object = candidate_frame
                .objects()
                .iter()
                .find(|object| object.content_ref() == selection_ref.value_ref())
                .ok_or(StoreError::InvalidHistory)?;
            let selection = serde_json::from_str(object.canonical_json())
                .map_err(|_| StoreError::InvalidHistory)?;
            Some(FactContinuation::new(
                scope.clone(),
                epoch,
                tenant.clone(),
                run_id.clone(),
                preparation.clone(),
                request,
                selection_ref,
                selection,
            ))
        }
    };
    Ok(AccessPreparationCandidate {
        append: PreparationAppend {
            disposition: AppendDisposition::NewlyCommitted {
                sequence: successor_sequence,
            },
            preparation: Some(preparation),
            fact_continuation,
        },
        frame: Some(candidate_frame),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_conclusion_from_current(
    scope: &StoreScopeId,
    epoch: StoreEpoch,
    tenant: &TenantScopeId,
    current: &QualifiedRun,
    run_id: &RunId,
    document: &ProgramDocument,
    selected: &RunSelection,
    expected_sequence: u64,
    append_request_id: AppendRequestId,
    conclusion: StateConcluded,
    objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    maximum_conclusion_bytes: u64,
) -> Result<PreparedConclusion> {
    if conclusion.fact_publication().is_some() {
        return Err(StoreError::InvalidRecord);
    }
    if current.run_id() != run_id {
        return Err(StoreError::Identity);
    }
    let successor_sequence = expected_sequence
        .checked_add(1)
        .ok_or(StoreError::Capacity)?;
    let candidate_frame = RunFrame::new(
        run_id.clone(),
        scope.clone(),
        epoch,
        successor_sequence,
        append_request_id,
        RunRecord::StateConcluded(conclusion.clone()),
        objects,
    )
    .map_err(|_| StoreError::Capacity)?;
    if let Some(existing) = current
        .frames()
        .iter()
        .find(|frame| frame.append_request_id() == candidate_frame.append_request_id())
    {
        if existing != &candidate_frame {
            return Err(StoreError::Conflict);
        }
        return Ok(PreparedConclusion::new(
            scope.clone(),
            epoch,
            tenant.clone(),
            existing.clone(),
            document.clone(),
            selected.action().clone(),
        ));
    }
    if current.head_sequence() != expected_sequence
        || current.has_conclusion(conclusion.occurrence())
    {
        return Err(StoreError::NotActionable);
    }
    let actionable = match (&conclusion, &selected.action) {
        (
            StateConcluded::Pure { occurrence, .. },
            RunAction::ReadyPure {
                occurrence: ready, ..
            },
        ) => occurrence == ready,
        (
            StateConcluded::Access {
                occurrence,
                preparation,
                ..
            },
            RunAction::WaitingPreparation {
                occurrence: ready,
                preparation: selected,
            },
        ) => occurrence == ready && preparation == selected,
        _ => false,
    };
    if !actionable {
        return Err(StoreError::NotActionable);
    }
    let occurrence = conclusion.occurrence().clone();
    if let StateConcluded::Access {
        preparation,
        fact_selection,
        ..
    } = &conclusion
    {
        let selected = current
            .selected_preparation(conclusion.occurrence())
            .ok_or(StoreError::NotActionable)?;
        if &selected.1 != preparation || selected.0.fact_selection() != fact_selection.as_ref() {
            return Err(StoreError::NotActionable);
        }
    }
    validate_conclusion_contract(document, &conclusion)?;
    let candidate_bytes = candidate_frame
        .canonical_bytes()
        .map_err(|_| StoreError::Capacity)?
        .as_bytes()
        .len();
    let state_maximum = match document.declaration(conclusion.occurrence()) {
        Some(Declaration::State(state)) => state.maximum_conclusion_bytes(),
        _ => return Err(StoreError::InvalidRecord),
    };
    if candidate_bytes
        > usize::try_from(maximum_conclusion_bytes).map_err(|_| StoreError::Capacity)?
        || candidate_bytes > usize::try_from(state_maximum).map_err(|_| StoreError::Capacity)?
    {
        return Err(StoreError::Capacity);
    }
    ensure_run_capacity(current.frames(), &candidate_frame)?;
    if let Some((prepared, _)) = current.selected_preparation(&occurrence) {
        if candidate_bytes
            > usize::try_from(prepared.maximum_conclusion_bytes())
                .map_err(|_| StoreError::Capacity)?
        {
            return Err(StoreError::Capacity);
        }
    }
    let mut candidate_prefix = current.frames().to_vec();
    candidate_prefix.push(candidate_frame.clone());
    validate_candidate_prefix(scope, epoch, tenant, &candidate_prefix)?;
    Ok(PreparedConclusion::new(
        scope.clone(),
        epoch,
        tenant.clone(),
        candidate_frame,
        document.clone(),
        selected.action().clone(),
    ))
}

pub(crate) fn reduce_qualified(
    current: &QualifiedRun,
    document: ProgramDocument,
) -> Result<RunSelection> {
    RunReducer::new(document).reduce(current)
}

#[cfg(test)]
struct MemoryState {
    runs: BTreeMap<RunId, Vec<RunFrame>>,
    fact_heads: BTreeMap<TenantScopeId, u64>,
}

/// In-memory exact-head Store test double used by semantic unit tests.
#[cfg(test)]
pub(crate) struct SemanticStore {
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    state: Arc<Mutex<MemoryState>>,
}

#[cfg(test)]
impl SemanticStore {
    /// Creates one immutable Store identity.  The mutex protects backend atomicity, not execution
    /// ownership; multiple qualified workers may call this Store concurrently and race by head.
    pub fn memory(scope: StoreScopeId, epoch: StoreEpoch, tenant: TenantScopeId) -> Self {
        Self {
            scope,
            epoch,
            tenant,
            state: Arc::new(Mutex::new(MemoryState {
                runs: BTreeMap::new(),
                fact_heads: BTreeMap::new(),
            })),
        }
    }

    /// Returns the fixed Store scope.
    #[cfg(test)]
    pub const fn scope(&self) -> &StoreScopeId {
        &self.scope
    }

    /// Returns the immutable writer epoch.
    #[cfg(test)]
    #[allow(dead_code)]
    pub const fn epoch(&self) -> StoreEpoch {
        self.epoch
    }

    /// Returns the fixed tenant partition.
    #[cfg(test)]
    #[allow(dead_code)]
    pub const fn tenant(&self) -> &TenantScopeId {
        &self.tenant
    }

    /// Loads and qualifies one complete retained prefix for this tenant.
    pub fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
        let state = self.state.lock().map_err(|_| StoreError::Identity)?;
        let frames = state
            .runs
            .get(run_id)
            .cloned()
            .ok_or(StoreError::NotFound)?;
        QualifiedRun::new(self.scope.clone(), self.epoch, self.tenant.clone(), frames)
    }

    /// Appends the sole admission frame, or returns found-same for an identical retry.
    #[cfg(test)]
    fn append_admission_fixture(&self, frame: RunFrame) -> Result<AppendDisposition> {
        if frame.expected_sequence() != 1 || !frame.record().is_admission() {
            return Err(StoreError::InvalidRecord);
        }
        self.append(frame)
    }

    /// Performs one exact-head compare-and-append with per-append validation.
    pub(crate) fn append(&self, frame: RunFrame) -> Result<AppendDisposition> {
        frame.validate().map_err(|_| StoreError::InvalidRecord)?;
        if frame.store_scope_id() != &self.scope || frame.store_epoch() != self.epoch {
            return Err(StoreError::Identity);
        }
        let mut state = self.state.lock().map_err(|_| StoreError::Identity)?;
        let run_id = frame.run_id().clone();
        let publication_sequence = frame
            .record()
            .fact_publication()
            .map(|publication| publication.publication_sequence());
        let fact_head = state.fact_heads.get(&self.tenant).copied().unwrap_or(0);
        let entry = state.runs.entry(run_id.clone()).or_default();
        if entry.is_empty() {
            if frame.expected_sequence() != 1 || !frame.record().is_admission() {
                return Err(StoreError::InvalidRecord);
            }
            let admission = match frame.record() {
                RunRecord::RunAdmitted(value) => value,
                _ => return Err(StoreError::InvalidRecord),
            };
            if admission.tenant_scope_id() != &self.tenant
                || admission.run_id() != &run_id
                || admission.store_scope_id() != &self.scope
                || admission.store_epoch() != self.epoch
            {
                return Err(StoreError::Identity);
            }
            ensure_run_capacity(&[], &frame)?;
            validate_candidate_prefix(
                &self.scope,
                self.epoch,
                &self.tenant,
                std::slice::from_ref(&frame),
            )?;
            entry.push(frame);
            advance_fact_head(&mut state, &self.tenant, publication_sequence);
            return Ok(AppendDisposition::NewlyCommitted { sequence: 1 });
        }

        if let Some((sequence, existing)) =
            entry.iter().enumerate().find_map(|(index, existing)| {
                (existing.append_request_id() == frame.append_request_id())
                    .then_some((index as u64 + 1, existing))
            })
        {
            return if existing == &frame {
                Ok(AppendDisposition::Found { sequence })
            } else {
                Err(StoreError::Conflict)
            };
        }

        let actual_sequence = entry.len() as u64;
        if frame.expected_sequence() != actual_sequence + 1 {
            return Ok(AppendDisposition::StaleHead { actual_sequence });
        }
        validate_fact_publication(fact_head, publication_sequence)?;
        validate_append(entry, &frame)?;
        ensure_run_capacity(entry, &frame)?;
        let mut candidate = entry.clone();
        candidate.push(frame.clone());
        validate_candidate_prefix(&self.scope, self.epoch, &self.tenant, &candidate)?;
        entry.push(frame);
        advance_fact_head(&mut state, &self.tenant, publication_sequence);
        Ok(AppendDisposition::NewlyCommitted {
            sequence: actual_sequence + 1,
        })
    }

    /// Creates one Access preparation candidate without invoking any implementation.
    #[allow(clippy::too_many_arguments)]
    fn prepare_access_fixture(
        &self,
        run_id: &RunId,
        document: &ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        prepared: StatePrepared,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<PreparationAppend> {
        let current = self.load(run_id)?;
        let selected = reduce_qualified(&current, document.clone())?;
        let candidate = prepare_access_from_current(
            &self.scope,
            self.epoch,
            &self.tenant,
            &current,
            run_id,
            document,
            &selected,
            expected_sequence,
            append_request_id,
            prepared,
            objects,
        )?;
        match candidate.frame {
            Some(frame) => {
                let disposition = self.append(frame)?;
                Ok(candidate.append.with_disposition(disposition))
            }
            None => Ok(candidate.append),
        }
    }

    /// Reserves and returns one conclusion owner while the current exact head remains known.
    #[allow(clippy::too_many_arguments)]
    fn prepare_conclusion_fixture(
        &self,
        run_id: &RunId,
        document: &ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        conclusion: StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<PreparedConclusion> {
        let current = self.load(run_id)?;
        let selected = reduce_qualified(&current, document.clone())?;
        prepare_conclusion_from_current(
            &self.scope,
            self.epoch,
            &self.tenant,
            &current,
            run_id,
            document,
            &selected,
            expected_sequence,
            append_request_id,
            conclusion,
            objects,
            maximum_conclusion_bytes,
        )
    }

    /// Reduces one qualified prefix against the exact callback-free Program document.
    fn reduce_fixture(&self, run_id: &RunId, document: ProgramDocument) -> Result<RunSelection> {
        let run = self.load(run_id)?;
        reduce_qualified(&run, document)
    }
}

/// Callback-free sequential reducer action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    /// Empty expanded control form returns the exact admitted `C0`.
    ZeroStateTerminal {
        /// Exact admitted context/root result.
        result: mfm_journal::single_trust::ValueRef,
    },
    /// A Pure State is ready for one deterministic evaluation.
    ReadyPure {
        /// Exact State occurrence.
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        /// Complete predecessor context.
        input: mfm_journal::single_trust::ValueRef,
    },
    /// An Access State is ready to borrow its exact predecessor context for preparation.
    ReadyAccess {
        /// Exact State occurrence.
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        /// Complete predecessor context.
        input: mfm_journal::single_trust::ValueRef,
        /// Read or Effect mode.
        mode: AccessActionMode,
    },
    /// One preparation is durable and awaits its call-bound conclusion.
    WaitingPreparation {
        /// Exact State occurrence.
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        /// Selected preparation identity.
        preparation: PreparationRef,
    },
    /// A successful terminal State has a durable root result.
    Terminal {
        /// Exact terminal occurrence.
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        /// Qualified root result.
        result: mfm_journal::single_trust::ValueRef,
    },
    /// A fail-fast State failure is durable.
    Failed {
        /// Exact failure occurrence.
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        /// Typed failure value.
        failure: mfm_journal::single_trust::ValueRef,
    },
}

/// Access mode selected from the immutable Program declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessActionMode {
    /// Non-mutating Read.
    Read,
    /// Mutating Effect.
    Effect,
}

/// Internal callback-free result of one complete-prefix reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunSelection {
    /// Exact latest complete cumulative context.
    latest_context: mfm_journal::single_trust::ValueRef,
    /// Immutable object carrying the latest context bytes.  Match selection may materialize this
    /// object for the next append's closure.
    latest_context_object: mfm_journal::single_trust::ImmutableObject,
    /// Sole next action or durable terminal result.
    action: RunAction,
}

impl RunSelection {
    fn new(
        _run: &QualifiedRun,
        latest_context: mfm_journal::single_trust::ValueRef,
        latest_context_object: mfm_journal::single_trust::ImmutableObject,
        action: RunAction,
    ) -> Self {
        Self {
            latest_context,
            latest_context_object,
            action,
        }
    }

    /// Returns the latest complete cumulative context identity.
    pub const fn latest_context(&self) -> &mfm_journal::single_trust::ValueRef {
        &self.latest_context
    }

    /// Returns the retained object carrying the latest context bytes.
    pub const fn latest_context_object(&self) -> &mfm_journal::single_trust::ImmutableObject {
        &self.latest_context_object
    }

    /// Returns the sole selected action or durable terminal result.
    pub const fn action(&self) -> &RunAction {
        &self.action
    }

    pub(crate) fn waiting_preparation(
        self,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        preparation: PreparationRef,
    ) -> Self {
        Self {
            latest_context: self.latest_context,
            latest_context_object: self.latest_context_object,
            action: RunAction::WaitingPreparation {
                occurrence,
                preparation,
            },
        }
    }
}

/// The sole affine Store-selected authority for one exact run head and Program.
///
/// A selected run cannot be cloned, serialized, or constructed by callers. Consuming it into
/// [`QualifiedRun`] gives up all mutation authority while preserving callback-free evidence.
#[derive(Debug)]
pub struct SelectedRun {
    run: QualifiedRun,
    program: Program,
    selection: RunSelection,
    store_brand: Arc<StoreBrand>,
}

impl SelectedRun {
    pub(crate) fn new(
        run: QualifiedRun,
        program: Program,
        selection: RunSelection,
        store_brand: Arc<StoreBrand>,
    ) -> Self {
        Self {
            run,
            program,
            selection,
            store_brand,
        }
    }

    pub(crate) fn belongs_to_store(&self, brand: &Arc<StoreBrand>) -> bool {
        Arc::ptr_eq(&self.store_brand, brand) && self.run.belongs_to_store(brand)
    }

    pub(crate) const fn document(&self) -> &ProgramDocument {
        self.program.document()
    }

    pub(crate) const fn selection(&self) -> &RunSelection {
        &self.selection
    }

    pub(crate) fn into_parts(self) -> (QualifiedRun, Program, RunSelection) {
        (self.run, self.program, self.selection)
    }

    pub(crate) const fn qualified_run(&self) -> &QualifiedRun {
        &self.run
    }

    /// Returns the durable run identity.
    pub fn run_id(&self) -> &RunId {
        self.run.run_id()
    }

    /// Returns the retained exact Program content identity selected for this run.
    pub fn program_ref(&self) -> &ContentRef {
        self.program.program_ref().content_ref()
    }

    /// Returns the exact retained Program selected for this run.
    ///
    /// The Program is callback-free and remains coupled to this affine Store selection; callers
    /// receive only a shared reference and cannot replace it while retaining mutation authority.
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Returns the exact selected head sequence.
    pub fn head_sequence(&self) -> u64 {
        self.run.head_sequence()
    }

    /// Returns the latest complete cumulative context identity.
    pub const fn latest_context(&self) -> &mfm_journal::single_trust::ValueRef {
        self.selection.latest_context()
    }

    /// Returns the retained object carrying the latest context bytes.
    pub const fn latest_context_object(&self) -> &mfm_journal::single_trust::ImmutableObject {
        self.selection.latest_context_object()
    }

    /// Returns the sole selected action or durable terminal result.
    pub const fn action(&self) -> &RunAction {
        self.selection.action()
    }

    /// Consumes this mutation owner into cloneable callback-free evidence.
    pub fn into_qualified_run(self) -> QualifiedRun {
        self.run
    }
}

/// Store-owned sequential reducer and binder.
pub(crate) struct RunReducer {
    document: ProgramDocument,
}

impl RunReducer {
    /// Creates one reducer for an already normalized State/Match Program document.
    pub(crate) fn new(document: ProgramDocument) -> Self {
        Self { document }
    }

    /// Folds one complete retained prefix without invoking any callback.
    pub(crate) fn reduce(&self, run: &QualifiedRun) -> Result<RunSelection> {
        let admission = match run.frames().first().map(RunFrame::record) {
            Some(RunRecord::RunAdmitted(value)) => value,
            _ => return Err(StoreError::InvalidHistory),
        };
        if admission.program_ref()
            != &self
                .document
                .program_ref()
                .map_err(|_| StoreError::InvalidHistory)?
            || admission.admitted_context().contract_ref()
                != self.document.admitted_context_contract_ref()
        {
            return Err(StoreError::InvalidHistory);
        }
        if self.document.declarations().is_empty() {
            let latest = admission.admitted_context().clone();
            if self.document.root_contract_ref() != latest.contract_ref()
                || run.frames()[0].objects().len() != 2
                || admitted_program_object(run.frames()).is_err()
                || object_for_value(run, &latest)?.content_ref() != latest.value_ref()
            {
                return Err(StoreError::InvalidHistory);
            }
            if run.frames().len() != 1 {
                return Err(StoreError::InvalidHistory);
            }
            return Ok(RunSelection::new(
                run,
                latest.clone(),
                object_for_value(run, &latest)?,
                RunAction::ZeroStateTerminal { result: latest },
            ));
        }

        validate_record_addresses(run, self.document.declarations())?;
        let mut latest = admission.admitted_context().clone();
        let mut latest_object = object_for_value(run, &latest)?;
        let mut cursor = self
            .document
            .entry_address()
            .map_err(|_| StoreError::InvalidHistory)?
            .clone();
        let mut consumed = BTreeSet::from([0_usize]);

        loop {
            let declaration = self
                .document
                .declaration(&cursor)
                .ok_or(StoreError::InvalidHistory)?;
            match declaration {
                Declaration::Match(match_declaration) => {
                    if match_declaration.selector_contract_ref() != latest.contract_ref() {
                        return Err(StoreError::InvalidHistory);
                    }
                    let (payload, next_address, payload_object) =
                        select_match_payload(run, &latest, match_declaration)?;
                    latest = payload;
                    latest_object = payload_object;
                    cursor = next_address;
                }
                Declaration::State(state) => {
                    if state.input_contract_ref() != latest.contract_ref() {
                        return Err(StoreError::InvalidHistory);
                    }
                    let occurrence = state.address().clone();
                    let conclusion = find_conclusion(run, &occurrence);
                    let preparation = find_preparation(run, &occurrence);
                    match (conclusion, preparation, state.execution()) {
                        (Some((_, conclusion)), preparation, ExecutionMode::Pure) => {
                            mark_occurrence_frames(run, &occurrence, &mut consumed);
                            if preparation.is_some()
                                || !matches!(conclusion, StateConcluded::Pure { .. })
                            {
                                return Err(StoreError::InvalidHistory);
                            }
                            let outcome = match conclusion {
                                StateConcluded::Pure { outcome, .. } => outcome,
                                StateConcluded::Access { .. } => {
                                    return Err(StoreError::InvalidHistory)
                                }
                            };
                            latest = apply_success_or_failure(state, &latest, outcome)?;
                            latest_object = object_for_value(run, &latest)?;
                            match conclusion_transition(state, &self.document, outcome)? {
                                ConclusionTransition::Terminal => {
                                    ensure_all_consumed(run, &consumed)?;
                                    return terminal_state(
                                        run,
                                        state,
                                        outcome,
                                        latest,
                                        latest_object,
                                    );
                                }
                                ConclusionTransition::Continue(next) => {
                                    cursor = next;
                                }
                            }
                        }
                        (
                            Some((_, conclusion)),
                            Some((_, _, preparation_ref)),
                            ExecutionMode::Read { .. } | ExecutionMode::Effect { .. },
                        ) => {
                            mark_occurrence_frames(run, &occurrence, &mut consumed);
                            let StateConcluded::Access {
                                preparation,
                                fact_selection,
                                outcome,
                                ..
                            } = conclusion
                            else {
                                return Err(StoreError::InvalidHistory);
                            };
                            if preparation != &preparation_ref {
                                return Err(StoreError::InvalidHistory);
                            }
                            let (_, prepared, _) = find_preparation(run, &occurrence)
                                .ok_or(StoreError::InvalidHistory)?;
                            if prepared.input() != &latest
                                || prepared.fact_selection() != fact_selection.as_ref()
                            {
                                return Err(StoreError::InvalidHistory);
                            }
                            latest = apply_success_or_failure(state, &latest, outcome)?;
                            latest_object = object_for_value(run, &latest)?;
                            match conclusion_transition(state, &self.document, outcome)? {
                                ConclusionTransition::Terminal => {
                                    ensure_all_consumed(run, &consumed)?;
                                    return terminal_state(
                                        run,
                                        state,
                                        outcome,
                                        latest,
                                        latest_object,
                                    );
                                }
                                ConclusionTransition::Continue(next) => {
                                    cursor = next;
                                }
                            }
                        }
                        (
                            Some(_),
                            None,
                            ExecutionMode::Read { .. } | ExecutionMode::Effect { .. },
                        ) => return Err(StoreError::InvalidHistory),
                        (None, Some((_, _, _)), ExecutionMode::Pure) => {
                            return Err(StoreError::InvalidHistory)
                        }
                        (
                            None,
                            Some((_, prepared, preparation_ref)),
                            ExecutionMode::Read { .. } | ExecutionMode::Effect { .. },
                        ) => {
                            mark_occurrence_frames(run, &occurrence, &mut consumed);
                            if prepared.input() != &latest {
                                return Err(StoreError::InvalidHistory);
                            }
                            ensure_all_consumed(run, &consumed)?;
                            return Ok(RunSelection::new(
                                run,
                                latest,
                                latest_object,
                                RunAction::WaitingPreparation {
                                    occurrence,
                                    preparation: preparation_ref,
                                },
                            ));
                        }
                        (None, None, ExecutionMode::Pure) => {
                            ensure_all_consumed(run, &consumed)?;
                            return Ok(RunSelection::new(
                                run,
                                latest.clone(),
                                latest_object,
                                RunAction::ReadyPure {
                                    occurrence,
                                    input: latest,
                                },
                            ));
                        }
                        (None, None, ExecutionMode::Read { .. }) => {
                            ensure_all_consumed(run, &consumed)?;
                            return Ok(RunSelection::new(
                                run,
                                latest.clone(),
                                latest_object,
                                RunAction::ReadyAccess {
                                    occurrence,
                                    input: latest,
                                    mode: AccessActionMode::Read,
                                },
                            ));
                        }
                        (None, None, ExecutionMode::Effect { .. }) => {
                            ensure_all_consumed(run, &consumed)?;
                            return Ok(RunSelection::new(
                                run,
                                latest.clone(),
                                latest_object,
                                RunAction::ReadyAccess {
                                    occurrence,
                                    input: latest,
                                    mode: AccessActionMode::Effect,
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
}

/// Advances one retained reducer result across the one conclusion frame just appended by a hot
/// Runtime session. This follows only the suffix after the already-selected occurrence; it never
/// folds the retained prefix again.
pub(crate) fn advance_selected(
    selected: &RunSelection,
    previous: &QualifiedRun,
    next: &QualifiedRun,
    document: &ProgramDocument,
) -> Result<RunSelection> {
    if next.run_id() != previous.run_id()
        || next.head_sequence() != previous.head_sequence().saturating_add(1)
    {
        return Err(StoreError::Identity);
    }
    let conclusion = match next.frames().last().map(RunFrame::record) {
        Some(RunRecord::StateConcluded(value)) => value,
        _ => return Err(StoreError::InvalidRecord),
    };
    let occurrence = conclusion.occurrence().clone();
    match &selected.action {
        RunAction::ReadyPure {
            occurrence: ready, ..
        }
        | RunAction::WaitingPreparation {
            occurrence: ready, ..
        } if ready == &occurrence => {}
        _ => return Err(StoreError::NotActionable),
    }
    let Some(Declaration::State(state)) = document.declaration(&occurrence) else {
        return Err(StoreError::InvalidHistory);
    };
    let mut latest =
        apply_success_or_failure(state, &selected.latest_context, conclusion.outcome())?;
    let mut latest_object = object_for_value(next, &latest)?;
    let mut cursor = match conclusion_transition(state, document, conclusion.outcome())? {
        ConclusionTransition::Terminal => {
            return terminal_state(next, state, conclusion.outcome(), latest, latest_object)
        }
        ConclusionTransition::Continue(next) => next,
    };
    loop {
        let declaration = document
            .declaration(&cursor)
            .ok_or(StoreError::InvalidHistory)?;
        match declaration {
            Declaration::Match(match_declaration) => {
                if match_declaration.selector_contract_ref() != latest.contract_ref() {
                    return Err(StoreError::InvalidHistory);
                }
                let (payload, next_address, payload_object) =
                    select_match_payload(next, &latest, match_declaration)?;
                cursor = next_address;
                latest = payload;
                latest_object = payload_object;
            }
            Declaration::State(state) => {
                if state.input_contract_ref() != latest.contract_ref() {
                    return Err(StoreError::InvalidHistory);
                }
                let occurrence = state.address().clone();
                let action = match state.execution() {
                    ExecutionMode::Pure => RunAction::ReadyPure {
                        occurrence,
                        input: latest.clone(),
                    },
                    ExecutionMode::Read { .. } => RunAction::ReadyAccess {
                        occurrence,
                        input: latest.clone(),
                        mode: AccessActionMode::Read,
                    },
                    ExecutionMode::Effect { .. } => RunAction::ReadyAccess {
                        occurrence,
                        input: latest.clone(),
                        mode: AccessActionMode::Effect,
                    },
                };
                return Ok(RunSelection::new(next, latest, latest_object, action));
            }
        }
    }
}

fn terminal_state(
    run: &QualifiedRun,
    state: &StateDeclaration,
    outcome: &StateOutcome,
    latest: mfm_journal::single_trust::ValueRef,
    latest_object: mfm_journal::single_trust::ImmutableObject,
) -> Result<RunSelection> {
    match outcome {
        StateOutcome::Success(result) => Ok(RunSelection::new(
            run,
            latest,
            latest_object,
            RunAction::Terminal {
                occurrence: state.address().clone(),
                result: result.clone(),
            },
        )),
        StateOutcome::Failure(failure) => Ok(RunSelection::new(
            run,
            latest,
            latest_object,
            RunAction::Failed {
                occurrence: state.address().clone(),
                failure: failure.clone(),
            },
        )),
    }
}

enum ConclusionTransition {
    Continue(mfm_journal::single_trust::SequentialControlAddress),
    Terminal,
}

fn conclusion_transition(
    state: &StateDeclaration,
    _document: &ProgramDocument,
    outcome: &StateOutcome,
) -> Result<ConclusionTransition> {
    match outcome {
        StateOutcome::Success(_) => {
            if state.terminal() {
                Ok(ConclusionTransition::Terminal)
            } else {
                state
                    .next_address()
                    .cloned()
                    .map(ConclusionTransition::Continue)
                    .ok_or(StoreError::InvalidHistory)
            }
        }
        StateOutcome::Failure(value) => {
            if state.failure_contract_ref() != Some(value.contract_ref()) {
                return Err(StoreError::InvalidHistory);
            }
            if let Some(next) = state.failure_next_address() {
                return Ok(ConclusionTransition::Continue(next.clone()));
            }
            Ok(ConclusionTransition::Terminal)
        }
    }
}

fn apply_success_or_failure(
    state: &StateDeclaration,
    latest: &mfm_journal::single_trust::ValueRef,
    outcome: &StateOutcome,
) -> Result<mfm_journal::single_trust::ValueRef> {
    if latest.contract_ref() != state.input_contract_ref() {
        return Err(StoreError::InvalidHistory);
    }
    match outcome {
        StateOutcome::Success(value) if value.contract_ref() == state.output_contract_ref() => {
            Ok(value.clone())
        }
        StateOutcome::Failure(value)
            if state.failure_contract_ref() == Some(value.contract_ref()) =>
        {
            Ok(value.clone())
        }
        _ => Err(StoreError::InvalidHistory),
    }
}

fn find_conclusion<'a>(
    run: &'a QualifiedRun,
    occurrence: &mfm_journal::single_trust::SequentialControlAddress,
) -> Option<(usize, &'a StateConcluded)> {
    run.frames()
        .iter()
        .enumerate()
        .find_map(|(index, frame)| match frame.record() {
            RunRecord::StateConcluded(value) if value.occurrence() == occurrence => {
                Some((index, value))
            }
            _ => None,
        })
}

fn find_preparation<'a>(
    run: &'a QualifiedRun,
    occurrence: &mfm_journal::single_trust::SequentialControlAddress,
) -> Option<(usize, &'a StatePrepared, PreparationRef)> {
    run.frames()
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, frame)| match frame.record() {
            RunRecord::StatePrepared(value) if value.occurrence() == occurrence => Some((
                index,
                value,
                PreparationRef::new(
                    run.run_id().clone(),
                    frame.expected_sequence(),
                    index as u32,
                ),
            )),
            _ => None,
        })
}

fn mark_occurrence_frames(
    run: &QualifiedRun,
    occurrence: &mfm_journal::single_trust::SequentialControlAddress,
    consumed: &mut BTreeSet<usize>,
) {
    for (index, frame) in run.frames().iter().enumerate() {
        let matches = match frame.record() {
            RunRecord::StatePrepared(prepared) => prepared.occurrence() == occurrence,
            RunRecord::StateConcluded(conclusion) => conclusion.occurrence() == occurrence,
            RunRecord::RunAdmitted(_) => false,
        };
        if matches {
            consumed.insert(index);
        }
    }
}

fn ensure_all_consumed(run: &QualifiedRun, consumed: &BTreeSet<usize>) -> Result<()> {
    if consumed.len() == run.frames().len() {
        Ok(())
    } else {
        Err(StoreError::InvalidHistory)
    }
}

fn select_match_payload(
    run: &QualifiedRun,
    input: &ValueRef,
    declaration: &mfm_program::single_trust::MatchDeclaration,
) -> Result<(
    ValueRef,
    mfm_journal::single_trust::SequentialControlAddress,
    mfm_journal::single_trust::ImmutableObject,
)> {
    let object = run
        .frames()
        .iter()
        .flat_map(|frame| frame.objects())
        .find(|object| object.content_ref() == input.value_ref())
        .ok_or(StoreError::InvalidHistory)?;
    let value: serde_json::Value =
        serde_json::from_str(object.canonical_json()).map_err(|_| StoreError::InvalidHistory)?;
    let object = value.as_object().ok_or(StoreError::InvalidHistory)?;
    if object.len() != 2 || !object.contains_key("kind") || !object.contains_key("value") {
        return Err(StoreError::InvalidHistory);
    }
    let tag = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or(StoreError::InvalidHistory)?;
    let payload = object.get("value").ok_or(StoreError::InvalidHistory)?;
    let variant = declaration
        .variants()
        .iter()
        .find(|variant| variant.tag().as_str() == tag)
        .ok_or(StoreError::InvalidHistory)?;
    let payload = mfm_journal::single_trust::canonical_json(payload)
        .map_err(|_| StoreError::InvalidHistory)?;
    let value_ref = ContentRef::new(
        variant.payload_contract_ref().schema_id().clone(),
        raw_content_digest(payload.as_bytes()),
    )
    .map_err(|_| StoreError::InvalidHistory)?;
    let selected = ValueRef::new(variant.payload_contract_ref().clone(), value_ref);
    let existing_object = run
        .frames()
        .iter()
        .flat_map(|frame| frame.objects())
        .find(|object| object.content_ref() == selected.value_ref())
        .cloned();
    let selected_object = match existing_object {
        Some(object) => object,
        None => mfm_journal::single_trust::ImmutableObject::new(
            mfm_ids::StableId::new("mfm.value").map_err(|_| StoreError::InvalidHistory)?,
            selected.value_ref().clone(),
            payload.as_str().to_owned(),
        )
        .map_err(|_| StoreError::InvalidHistory)?,
    };
    Ok((selected, variant.entry_address().clone(), selected_object))
}

pub(crate) fn object_for_value(
    run: &QualifiedRun,
    value: &ValueRef,
) -> Result<mfm_journal::single_trust::ImmutableObject> {
    run.frames()
        .iter()
        .flat_map(|frame| frame.objects())
        .find(|object| object.content_ref() == value.value_ref())
        .cloned()
        .ok_or(StoreError::InvalidHistory)
}

fn validate_preparation_contract(
    prepared: &StatePrepared,
    state: &mfm_program::single_trust::StateDeclaration,
) -> Result<()> {
    let binding_ref = state
        .execution_binding()
        .ok_or(StoreError::InvalidRecord)?
        .content_ref()
        .map_err(|_| StoreError::InvalidRecord)?;
    if state.execution().is_pure()
        || prepared.input().contract_ref() != state.input_contract_ref()
        || binding_ref != *prepared.execution_binding_ref()
        || prepared.maximum_conclusion_bytes() != state.maximum_conclusion_bytes()
        || prepared.fact_request().is_some() != prepared.fact_selection().is_some()
        || prepared.fact_request().is_some() != state.fact_selection_required()
    {
        return Err(StoreError::InvalidRecord);
    }
    match (state.execution(), prepared.mode()) {
        (
            ExecutionMode::Read {
                total_attempt_bound,
                ..
            },
            mfm_journal::single_trust::PreparationMode::Read {
                total_attempt_bound: prepared_bound,
            },
        ) if total_attempt_bound == prepared_bound => Ok(()),
        (ExecutionMode::Effect { .. }, mfm_journal::single_trust::PreparationMode::Effect) => {
            Ok(())
        }
        _ => Err(StoreError::InvalidRecord),
    }
}

fn validate_conclusion_contract(
    document: &ProgramDocument,
    conclusion: &StateConcluded,
) -> Result<()> {
    let Some(Declaration::State(state)) = document.declaration(conclusion.occurrence()) else {
        return Err(StoreError::InvalidRecord);
    };
    match conclusion {
        StateConcluded::Pure { outcome, .. } if !state.execution().is_pure() => {
            let _ = outcome;
            Err(StoreError::InvalidRecord)
        }
        StateConcluded::Access { outcome, .. } if state.execution().is_pure() => {
            let _ = outcome;
            Err(StoreError::InvalidRecord)
        }
        StateConcluded::Pure { outcome, .. } => {
            match outcome {
                StateOutcome::Success(value)
                    if value.contract_ref() == state.output_contract_ref() => {}
                StateOutcome::Failure(value)
                    if state.failure_contract_ref() == Some(value.contract_ref()) => {}
                _ => return Err(StoreError::InvalidRecord),
            }
            conclusion_transition(state, document, outcome).map(|_| ())
        }
        StateConcluded::Access {
            outcome,
            fact_publication,
            ..
        } => {
            if fact_publication.as_ref().is_some_and(|publication| {
                Some(publication.proposal_set_ref()) != conclusion.fact_proposals()
            }) {
                return Err(StoreError::InvalidRecord);
            }
            match outcome {
                StateOutcome::Success(value)
                    if value.contract_ref() == state.output_contract_ref() => {}
                StateOutcome::Failure(value)
                    if state.failure_contract_ref() == Some(value.contract_ref()) => {}
                _ => return Err(StoreError::InvalidRecord),
            }
            conclusion_transition(state, document, outcome).map(|_| ())
        }
    }
}

fn validate_record_addresses(run: &QualifiedRun, declarations: &[Declaration]) -> Result<()> {
    for frame in run.frames().iter().skip(1) {
        match frame.record() {
            RunRecord::StatePrepared(prepared) => {
                let Some(Declaration::State(state)) =
                    declarations.iter().find(|declaration| match declaration {
                        Declaration::State(state) => state.address() == prepared.occurrence(),
                        Declaration::Match(_) => false,
                    })
                else {
                    return Err(StoreError::InvalidHistory);
                };
                validate_preparation_contract(prepared, state)
                    .map_err(|_| StoreError::InvalidHistory)?;
            }
            RunRecord::StateConcluded(conclusion) => {
                if !declarations.iter().any(|declaration| {
                    matches!(declaration, Declaration::State(state) if state.address() == conclusion.occurrence())
                }) {
                    return Err(StoreError::InvalidHistory);
                }
            }
            RunRecord::RunAdmitted(_) => return Err(StoreError::InvalidHistory),
        }
    }
    Ok(())
}

fn validate_prefix_structure(frames: &[RunFrame]) -> Result<()> {
    let admission = match frames.first().map(RunFrame::record) {
        Some(RunRecord::RunAdmitted(value)) => value,
        _ => return Err(StoreError::InvalidHistory),
    };
    let mut selected: BTreeMap<
        mfm_journal::single_trust::SequentialControlAddress,
        (
            u16,
            PreparationRef,
            mfm_journal::single_trust::PreparationMode,
            Option<ValueRef>,
        ),
    > = BTreeMap::new();
    let mut concluded = BTreeSet::new();
    for (index, frame) in frames.iter().enumerate().skip(1) {
        match frame.record() {
            RunRecord::StatePrepared(prepared) => {
                if concluded.contains(prepared.occurrence()) {
                    return Err(StoreError::InvalidHistory);
                }
                let preparation_ref = PreparationRef::new(
                    admission.run_id().clone(),
                    frame.expected_sequence(),
                    index as u32,
                );
                match selected.get(prepared.occurrence()) {
                    None if prepared.preparation_ordinal() == 0
                        && prepared.replaces().is_none() => {}
                    Some((ordinal, previous, mode, _))
                        if prepared.preparation_ordinal() == ordinal.saturating_add(1)
                            && prepared.replaces() == Some(previous)
                            && prepared.mode() == mode
                            && prepared.mode().permits_replacement()
                            && prepared.preparation_ordinal()
                                < prepared.mode().total_attempt_bound() => {}
                    _ => return Err(StoreError::InvalidHistory),
                }
                selected.insert(
                    prepared.occurrence().clone(),
                    (
                        prepared.preparation_ordinal(),
                        preparation_ref,
                        *prepared.mode(),
                        prepared.fact_selection().cloned(),
                    ),
                );
            }
            RunRecord::StateConcluded(conclusion) => {
                if !concluded.insert(conclusion.occurrence().clone()) {
                    return Err(StoreError::InvalidHistory);
                }
                match conclusion {
                    StateConcluded::Pure { .. } => {
                        if selected.contains_key(conclusion.occurrence()) {
                            return Err(StoreError::InvalidHistory);
                        }
                    }
                    StateConcluded::Access { preparation, .. } => {
                        let Some((_, selected_ref, _, selected_fact_selection)) =
                            selected.get(conclusion.occurrence())
                        else {
                            return Err(StoreError::InvalidHistory);
                        };
                        if selected_ref != preparation
                            || selected_fact_selection.as_ref() != conclusion.fact_selection()
                        {
                            return Err(StoreError::InvalidHistory);
                        }
                    }
                }
            }
            RunRecord::RunAdmitted(_) => return Err(StoreError::InvalidHistory),
        }
    }
    Ok(())
}

fn admitted_record(frames: &[RunFrame]) -> Result<&mfm_journal::single_trust::RunAdmitted> {
    match frames.first().map(RunFrame::record) {
        Some(RunRecord::RunAdmitted(admission)) => Ok(admission),
        _ => Err(StoreError::InvalidHistory),
    }
}

fn program_object_type() -> Result<StableId> {
    StableId::new(PROGRAM_OBJECT_TYPE).map_err(|_| StoreError::InvalidRecord)
}

/// Returns the sole exact Program object in one admission closure after structural validation.
fn admitted_program_object(frames: &[RunFrame]) -> Result<&ImmutableObject> {
    let admission = admitted_record(frames)?;
    let object_type = program_object_type()?;
    let admission_frame = frames.first().ok_or(StoreError::InvalidHistory)?;
    let mut program_objects = admission_frame
        .objects()
        .iter()
        .filter(|object| object.object_type() == &object_type);
    let object = program_objects.next().ok_or(StoreError::InvalidHistory)?;
    if program_objects.next().is_some()
        || frames
            .iter()
            .skip(1)
            .flat_map(RunFrame::objects)
            .any(|candidate| candidate.object_type() == &object_type)
    {
        return Err(StoreError::InvalidHistory);
    }
    let document = ProgramDocument::decode_canonical(object.canonical_json().as_bytes())
        .map_err(|_| StoreError::InvalidHistory)?;
    let document_ref = document
        .program_ref()
        .map_err(|_| StoreError::InvalidHistory)?;
    if object.content_ref() != &document_ref
        || object.content_ref() != admission.program_ref()
        || document.entry_point_id() != admission.entry_point_id()
    {
        return Err(StoreError::InvalidHistory);
    }
    Ok(object)
}

fn validate_object_reachability(
    scope: &StoreScopeId,
    epoch: StoreEpoch,
    tenant: &TenantScopeId,
    frames: &[RunFrame],
) -> Result<()> {
    let program_object = admitted_program_object(frames)?;
    let mut objects = BTreeMap::new();
    let mut referenced = BTreeSet::from([program_object.content_ref().clone()]);
    for frame in frames {
        for object in frame.objects() {
            if let Some(previous) = objects.get(object.content_ref()) {
                if previous != object {
                    return Err(StoreError::InvalidHistory);
                }
            } else {
                objects.insert(object.content_ref().clone(), object.clone());
            }
        }
        validate_record_objects(&objects, frame.record())?;
        referenced.extend(
            record_value_refs(frame.record())
                .into_iter()
                .map(|value| value.value_ref().clone()),
        );
    }
    if objects
        .keys()
        .any(|content_ref| !referenced.contains(content_ref))
    {
        return Err(StoreError::InvalidHistory);
    }
    for frame in frames {
        if let RunRecord::StatePrepared(prepared) = frame.record() {
            if let (Some(request), Some(selection)) =
                (prepared.fact_request(), prepared.fact_selection())
            {
                validate_fact_pair(&objects, request, selection, scope, epoch, tenant)?;
            }
        }
    }
    Ok(())
}

fn validate_record_objects(
    objects: &BTreeMap<ContentRef, mfm_journal::single_trust::ImmutableObject>,
    record: &RunRecord,
) -> Result<()> {
    for value in record_value_refs(record) {
        let Some(object) = objects.get(value.value_ref()) else {
            return Err(StoreError::InvalidHistory);
        };
        if !value.is_schema_bound()
            || object.content_ref().schema_id() != value.value_ref().schema_id()
        {
            return Err(StoreError::InvalidHistory);
        }
    }
    if let RunRecord::StateConcluded(conclusion) = record {
        if let Some(proposals) = conclusion.fact_proposals() {
            validate_fact_proposals(objects, proposals)?;
        }
    }
    Ok(())
}

fn validate_fact_proposals(
    objects: &BTreeMap<ContentRef, mfm_journal::single_trust::ImmutableObject>,
    proposals_ref: &ValueRef,
) -> Result<()> {
    if proposals_ref.contract_ref().schema_id()
        != &mfm_facts::FactProposalSet::schema_id().map_err(|_| StoreError::InvalidRecord)?
    {
        return Err(StoreError::InvalidRecord);
    }
    let object = objects
        .get(proposals_ref.value_ref())
        .ok_or(StoreError::InvalidHistory)?;
    let proposals: mfm_facts::FactProposalSet =
        serde_json::from_str(object.canonical_json()).map_err(|_| StoreError::InvalidRecord)?;
    proposals
        .validate()
        .map_err(|_| StoreError::InvalidRecord)?;
    let canonical = mfm_journal::single_trust::canonical_json(&proposals)
        .map_err(|_| StoreError::InvalidRecord)?;
    if object.content_ref() != proposals_ref.value_ref()
        || object.content_ref().content_digest() != &raw_content_digest(canonical.as_bytes())
        || object.canonical_json() != canonical.as_str()
    {
        return Err(StoreError::InvalidRecord);
    }
    Ok(())
}

fn validate_fact_pair(
    objects: &BTreeMap<ContentRef, mfm_journal::single_trust::ImmutableObject>,
    request_ref: &ValueRef,
    selection_ref: &ValueRef,
    scope: &StoreScopeId,
    epoch: StoreEpoch,
    tenant: &TenantScopeId,
) -> Result<()> {
    if request_ref.contract_ref().schema_id()
        != &mfm_facts::FactSelectionRequest::schema_id().map_err(|_| StoreError::InvalidRecord)?
        || selection_ref.contract_ref().schema_id()
            != &mfm_facts::FactSelection::schema_id().map_err(|_| StoreError::InvalidRecord)?
    {
        return Err(StoreError::InvalidRecord);
    }
    let request_object = objects
        .get(request_ref.value_ref())
        .ok_or(StoreError::InvalidHistory)?;
    let selection_object = objects
        .get(selection_ref.value_ref())
        .ok_or(StoreError::InvalidHistory)?;
    let request: mfm_facts::FactSelectionRequest =
        serde_json::from_str(request_object.canonical_json())
            .map_err(|_| StoreError::InvalidRecord)?;
    let selection: mfm_facts::FactSelection =
        serde_json::from_str(selection_object.canonical_json())
            .map_err(|_| StoreError::InvalidRecord)?;
    request.validate().map_err(|_| StoreError::InvalidRecord)?;
    selection
        .validate_for(&request)
        .map_err(|_| StoreError::InvalidRecord)?;
    if selection.frontier.stream_ref != fact_stream_ref(scope, epoch, tenant)? {
        return Err(StoreError::InvalidRecord);
    }
    Ok(())
}

pub(crate) fn fact_stream_ref(
    scope: &StoreScopeId,
    epoch: StoreEpoch,
    tenant: &TenantScopeId,
) -> Result<ContentRef> {
    let schema = SchemaId::new(
        "mfm.fact-stream",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| StoreError::InvalidRecord)?;
    let material =
        mfm_journal::single_trust::canonical_json(&(scope.as_str(), epoch.get(), tenant.as_str()))
            .map_err(|_| StoreError::InvalidRecord)?;
    ContentRef::new(schema, raw_content_digest(material.as_bytes()))
        .map_err(|_| StoreError::InvalidRecord)
}

fn record_value_refs(record: &RunRecord) -> Vec<&mfm_journal::single_trust::ValueRef> {
    match record {
        RunRecord::RunAdmitted(admission) => vec![admission.admitted_context()],
        RunRecord::StatePrepared(prepared) => {
            let mut values = vec![prepared.input(), prepared.intent()];
            if let Some(request) = prepared.fact_request() {
                values.push(request);
            }
            if let Some(selection) = prepared.fact_selection() {
                values.push(selection);
            }
            values
        }
        RunRecord::StateConcluded(conclusion) => {
            let mut values = Vec::new();
            if let Some(proposals) = conclusion.fact_proposals() {
                values.push(proposals);
            }
            if let StateConcluded::Access {
                evidence,
                fact_selection,
                fact_publication,
                ..
            } = conclusion
            {
                values.push(evidence);
                if let Some(selection) = fact_selection {
                    values.push(selection);
                }
                if let Some(publication) = fact_publication {
                    values.push(publication.proposal_set_ref());
                }
            } else if let StateConcluded::Pure {
                fact_publication: Some(publication),
                ..
            } = conclusion
            {
                values.push(publication.proposal_set_ref());
            }
            values.push(match conclusion.outcome() {
                StateOutcome::Success(value) | StateOutcome::Failure(value) => value,
            });
            values
        }
    }
}

fn validate_append(existing: &[RunFrame], candidate: &RunFrame) -> Result<()> {
    let record = candidate.record();
    if record.is_admission() || candidate.expected_sequence() == 1 {
        return Err(StoreError::InvalidRecord);
    }
    let key = record.logical_key();
    if existing
        .iter()
        .any(|frame| frame.record().logical_key() == key)
    {
        return Err(StoreError::Conflict);
    }
    match record {
        RunRecord::StatePrepared(prepared) => {
            let previous = existing.iter().filter_map(|frame| match frame.record() {
                RunRecord::StatePrepared(value) if value.occurrence() == prepared.occurrence() => {
                    Some(value)
                }
                _ => None,
            });
            let expected_ordinal = previous
                .map(StatePrepared::preparation_ordinal)
                .max()
                .map_or(0, |ordinal| ordinal.saturating_add(1));
            if prepared.preparation_ordinal() != expected_ordinal
                || (prepared.replaces().is_some() != (expected_ordinal > 0))
            {
                return Err(StoreError::InvalidRecord);
            }
            if let Some(previous) = existing
                .iter()
                .rev()
                .find_map(|frame| match frame.record() {
                    RunRecord::StatePrepared(value)
                        if value.occurrence() == prepared.occurrence() =>
                    {
                        Some(value)
                    }
                    _ => None,
                })
            {
                if !previous.mode().permits_replacement()
                    || previous.mode() != prepared.mode()
                    || prepared.preparation_ordinal() >= prepared.mode().total_attempt_bound()
                {
                    return Err(StoreError::InvalidRecord);
                }
                let (parent_index, parent_frame) = existing
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, frame)| match frame.record() {
                        RunRecord::StatePrepared(value) => {
                            value.occurrence() == prepared.occurrence()
                                && value.preparation_ordinal().saturating_add(1)
                                    == prepared.preparation_ordinal()
                        }
                        _ => false,
                    })
                    .ok_or(StoreError::InvalidRecord)?;
                let previous_ref = PreparationRef::new(
                    candidate.run_id().clone(),
                    parent_frame.expected_sequence(),
                    parent_index as u32,
                );
                if prepared.replaces() != Some(&previous_ref) {
                    return Err(StoreError::InvalidRecord);
                }
            }
        }
        RunRecord::StateConcluded(conclusion) => {
            if existing.iter().any(|frame| {
                matches!(
                    frame.record(),
                    RunRecord::StateConcluded(other) if other.occurrence() == conclusion.occurrence()
                )
            }) {
                return Err(StoreError::Conflict);
            }
            if matches!(conclusion, StateConcluded::Pure { .. }) {
                // Pure conclusions are intentionally preparation-free.
            } else if let StateConcluded::Access {
                preparation,
                fact_selection,
                ..
            } = conclusion
            {
                let selected =
                    existing
                        .iter()
                        .enumerate()
                        .find_map(|(index, frame)| match frame.record() {
                            RunRecord::StatePrepared(value)
                                if value.occurrence() == conclusion.occurrence()
                                    && PreparationRef::new(
                                        candidate.run_id().clone(),
                                        frame.expected_sequence(),
                                        index as u32,
                                    ) == *preparation =>
                            {
                                Some(value)
                            }
                            _ => None,
                        });
                if selected.is_none()
                    || selected.and_then(StatePrepared::fact_selection) != fact_selection.as_ref()
                {
                    return Err(StoreError::InvalidRecord);
                }
            }
        }
        RunRecord::RunAdmitted(_) => return Err(StoreError::InvalidRecord),
    }
    Ok(())
}

fn ensure_run_capacity(existing: &[RunFrame], candidate: &RunFrame) -> Result<()> {
    let current_bytes = existing.iter().try_fold(0usize, |total, frame| {
        let bytes = frame
            .canonical_bytes()
            .map_err(|_| StoreError::InvalidHistory)?
            .as_bytes()
            .len();
        total.checked_add(bytes).ok_or(StoreError::Capacity)
    })?;
    let candidate_bytes = candidate
        .canonical_bytes()
        .map_err(|_| StoreError::InvalidRecord)?
        .as_bytes()
        .len();
    let mut object_refs = BTreeSet::new();
    for frame in existing {
        object_refs.extend(frame.objects().iter().map(|object| object.content_ref()));
    }
    object_refs.extend(
        candidate
            .objects()
            .iter()
            .map(|object| object.content_ref()),
    );
    let objects = object_refs.len();
    let mut combined = existing.to_vec();
    combined.push(candidate.clone());
    let reserved = reserved_conclusion_bytes(&combined)?;
    if current_bytes
        .checked_add(candidate_bytes)
        .and_then(|bytes| bytes.checked_add(usize::try_from(reserved).ok()?))
        .is_none_or(|bytes| bytes > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES)
        || objects > mfm_journal::single_trust::MAX_RUN_OBJECTS
    {
        return Err(StoreError::Capacity);
    }
    Ok(())
}

fn reserved_conclusion_bytes(frames: &[RunFrame]) -> Result<u64> {
    let mut selected: BTreeMap<mfm_journal::single_trust::SequentialControlAddress, (u16, u64)> =
        BTreeMap::new();
    let mut concluded = BTreeSet::new();
    for frame in frames {
        match frame.record() {
            RunRecord::StatePrepared(prepared) => {
                selected.insert(
                    prepared.occurrence().clone(),
                    (
                        prepared.preparation_ordinal(),
                        prepared.maximum_conclusion_bytes(),
                    ),
                );
            }
            RunRecord::StateConcluded(conclusion) => {
                concluded.insert(conclusion.occurrence().clone());
            }
            RunRecord::RunAdmitted(_) => {}
        }
    }
    selected
        .into_iter()
        .filter(|(occurrence, _)| !concluded.contains(occurrence))
        .try_fold(0_u64, |total, (_, (_, bytes))| {
            total.checked_add(bytes).ok_or(StoreError::Capacity)
        })
}

#[cfg(test)]
fn validate_fact_publication(current: u64, publication: Option<u64>) -> Result<()> {
    let Some(publication) = publication else {
        return Ok(());
    };
    let expected = current.checked_add(1).ok_or(StoreError::Capacity)?;
    (publication == expected)
        .then_some(())
        .ok_or(StoreError::Conflict)
}

#[cfg(test)]
fn advance_fact_head(state: &mut MemoryState, tenant: &TenantScopeId, publication: Option<u64>) {
    if let Some(publication) = publication {
        state.fact_heads.insert(tenant.clone(), publication);
    }
}

fn validate_candidate_prefix(
    scope: &StoreScopeId,
    epoch: StoreEpoch,
    tenant: &TenantScopeId,
    frames: &[RunFrame],
) -> Result<()> {
    QualifiedRun::new(scope.clone(), epoch, tenant.clone(), frames.to_vec())
        .map(|_| ())
        .map_err(|error| match error {
            StoreError::Capacity => StoreError::Capacity,
            StoreError::Identity => StoreError::Identity,
            _ => StoreError::InvalidRecord,
        })
}

/// Mechanical result of one exact-head configuration append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationAppendDisposition {
    /// The candidate became the next durable revision.
    NewlyCommitted {
        /// Newly assigned revision sequence.
        sequence: u64,
    },
    /// The same append request already has the same revision.
    Found {
        /// Existing revision sequence.
        sequence: u64,
    },
    /// The caller's configuration head was stale.
    StaleHead {
        /// Current revision sequence.
        actual_sequence: u64,
    },
    /// The backend acknowledgement was lost after submission.
    AcknowledgementUnknown,
}

#[cfg(test)]
#[path = "../tests/single_trust_unit.rs"]
mod tests;
