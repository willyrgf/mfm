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

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, RunId, SchemaId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::single_trust::{
    PreparationRef, RunFrame, RunRecord, StateConcluded, StateOutcome, StatePrepared, ValueRef,
};
use mfm_program::single_trust::{Declaration, ExecutionMode, ProgramDocument, StateDeclaration};
use mfm_values::MfmValue;

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
        validate_object_reachability(&frames)?;
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
        })
    }

    /// Qualifies one complete retained prefix under one Store identity.
    pub fn qualify_prefix(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        frames: Vec<RunFrame>,
    ) -> Result<Self> {
        Self::new(scope, epoch, tenant, frames)
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

    /// Returns the recursive head content address for this qualified prefix.
    pub fn head_digest(&self) -> Result<ContentDigest> {
        Ok(self.head_digest.clone())
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
        validate_record_objects(&object_map, frame.record())?;
        if let RunRecord::StatePrepared(prepared) = frame.record() {
            if let (Some(request), Some(selection)) =
                (prepared.fact_request(), prepared.fact_selection())
            {
                validate_fact_pair(&object_map, request, selection)?;
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

/// A secret-free Store-owned owner for one conclusion append.
#[derive(Debug)]
pub struct PreparedConclusion {
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    frame: RunFrame,
    store_brand: Option<Arc<crate::backend::StoreBrand>>,
}

impl PreparedConclusion {
    fn new(scope: StoreScopeId, epoch: StoreEpoch, tenant: TenantScopeId, frame: RunFrame) -> Self {
        Self {
            scope,
            epoch,
            tenant,
            frame,
            store_brand: None,
        }
    }

    /// Returns the exact candidate frame without exposing mutable append authority.
    pub const fn frame(&self) -> &RunFrame {
        &self.frame
    }

    pub(crate) fn bind_store(&mut self, brand: Arc<crate::backend::StoreBrand>) {
        self.store_brand = Some(brand);
    }

    pub(crate) fn belongs_to(
        &self,
        identity: &crate::backend::StructuredStoreIdentity,
        brand: &Arc<crate::backend::StoreBrand>,
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
pub struct PreparationAppend {
    disposition: AppendDisposition,
    preparation: Option<PreparationRef>,
    fact_continuation: Option<FactContinuation>,
    frame: Option<RunFrame>,
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
    selection: ValueRef,
}

impl FactContinuation {
    fn new(
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        run_id: RunId,
        preparation: PreparationRef,
        request: ValueRef,
        selection: ValueRef,
    ) -> Self {
        Self {
            scope,
            epoch,
            tenant,
            run_id,
            preparation,
            request,
            selection,
        }
    }

    /// Returns the fixed prior-fact request identity without exposing its bytes.
    pub const fn request(&self) -> &ValueRef {
        &self.request
    }

    /// Returns the selected fact response identity without exposing its bytes.
    pub const fn selection(&self) -> &ValueRef {
        &self.selection
    }

    /// Consumes the continuation into the callback-free selected response identity.
    pub fn into_selection(self) -> ValueRef {
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
            frame: self.frame,
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

    /// Returns the one-use prior-fact continuation for a direct-new append.
    pub const fn fact_continuation(&self) -> Option<&FactContinuation> {
        self.fact_continuation.as_ref()
    }

    /// Consumes the append result's one-use prior-fact continuation.
    pub fn into_fact_continuation(self) -> Option<FactContinuation> {
        self.fact_continuation
    }

    pub(crate) fn set_frame(&mut self, frame: RunFrame) {
        self.frame = Some(frame);
    }

    /// Returns the exact candidate frame when Store constructed a new physical append.
    pub fn committed_frame(&self) -> Option<&RunFrame> {
        self.frame.as_ref()
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
    reduced: &ReducedRunState,
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
    if !reduced.belongs_to(current) {
        return Err(StoreError::Identity);
    }
    let mut objects = objects;
    if !current
        .frames()
        .iter()
        .flat_map(|frame| frame.objects())
        .any(|object| object == &reduced.latest_context_object)
        && !objects
            .iter()
            .any(|object| object == &reduced.latest_context_object)
    {
        objects.push(reduced.latest_context_object.clone());
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
                frame: None,
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
                frame: None,
            },
            frame: None,
        });
    }
    let expected_input = match &reduced.action {
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
    let fact_continuation = fact_request
        .zip(fact_selection)
        .map(|(request, selection)| {
            FactContinuation::new(
                scope.clone(),
                epoch,
                tenant.clone(),
                run_id.clone(),
                preparation.clone(),
                request,
                selection,
            )
        });
    Ok(AccessPreparationCandidate {
        append: PreparationAppend {
            disposition: AppendDisposition::NewlyCommitted {
                sequence: successor_sequence,
            },
            preparation: Some(preparation),
            fact_continuation,
            frame: None,
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
    reduced: &ReducedRunState,
    expected_sequence: u64,
    append_request_id: AppendRequestId,
    conclusion: StateConcluded,
    objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    maximum_conclusion_bytes: u64,
) -> Result<PreparedConclusion> {
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
        ));
    }
    if current.head_sequence() != expected_sequence
        || current.has_conclusion(conclusion.occurrence())
    {
        return Err(StoreError::NotActionable);
    }
    if !reduced.belongs_to(current) {
        return Err(StoreError::Identity);
    }
    let actionable = match (&conclusion, &reduced.action) {
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
    ))
}

pub(crate) fn reduce_qualified(
    current: &QualifiedRun,
    document: ProgramDocument,
) -> Result<ReducedRunState> {
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
    pub fn admit(&self, frame: RunFrame) -> Result<AppendDisposition> {
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
    pub fn prepare_access(
        &self,
        run_id: &RunId,
        document: &ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        prepared: StatePrepared,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<PreparationAppend> {
        let current = self.load(run_id)?;
        let reduced = reduce_qualified(&current, document.clone())?;
        let candidate = prepare_access_from_current(
            &self.scope,
            self.epoch,
            &self.tenant,
            &current,
            run_id,
            document,
            &reduced,
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
    pub fn prepare_conclusion(
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
        let reduced = reduce_qualified(&current, document.clone())?;
        prepare_conclusion_from_current(
            &self.scope,
            self.epoch,
            &self.tenant,
            &current,
            run_id,
            document,
            &reduced,
            expected_sequence,
            append_request_id,
            conclusion,
            objects,
            maximum_conclusion_bytes,
        )
    }

    /// Reduces one qualified prefix against the exact callback-free Program document.
    pub fn reduce(&self, run_id: &RunId, document: ProgramDocument) -> Result<ReducedRunState> {
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

/// Callback-free result of one complete-prefix reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReducedRunState {
    /// Exact latest complete cumulative context.
    latest_context: mfm_journal::single_trust::ValueRef,
    /// Immutable object carrying the latest context bytes.  Match selection may materialize this
    /// object for the next append's closure.
    latest_context_object: mfm_journal::single_trust::ImmutableObject,
    /// Sole next action or durable terminal result.
    action: RunAction,
    run_id: RunId,
    head_sequence: u64,
}

impl ReducedRunState {
    fn new(
        run: &QualifiedRun,
        latest_context: mfm_journal::single_trust::ValueRef,
        latest_context_object: mfm_journal::single_trust::ImmutableObject,
        action: RunAction,
    ) -> Self {
        Self {
            latest_context,
            latest_context_object,
            action,
            run_id: run.run_id().clone(),
            head_sequence: run.head_sequence(),
        }
    }

    pub(crate) fn belongs_to(&self, run: &QualifiedRun) -> bool {
        self.run_id == *run.run_id() && self.head_sequence == run.head_sequence()
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

    /// Carries the retained latest context across a direct-new preparation append.
    pub fn waiting_preparation(
        &self,
        run: &QualifiedRun,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        preparation: PreparationRef,
    ) -> Result<Self> {
        if self.run_id != *run.run_id()
            || self.head_sequence.saturating_add(1) != run.head_sequence()
            || preparation.run_sequence() != run.head_sequence()
        {
            return Err(StoreError::Identity);
        }
        Ok(Self::new(
            run,
            self.latest_context.clone(),
            self.latest_context_object.clone(),
            RunAction::WaitingPreparation {
                occurrence,
                preparation,
            },
        ))
    }
}

/// Store-owned sequential reducer and binder.
pub struct RunReducer {
    document: ProgramDocument,
}

impl RunReducer {
    /// Creates one reducer for an already normalized State/Match Program document.
    pub fn new(document: ProgramDocument) -> Self {
        Self { document }
    }

    /// Folds one complete retained prefix without invoking any callback.
    pub fn reduce(&self, run: &QualifiedRun) -> Result<ReducedRunState> {
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
                || run.frames()[0].objects().len() != 1
                || run.frames()[0].objects()[0].content_ref() != latest.value_ref()
            {
                return Err(StoreError::InvalidHistory);
            }
            if run.frames().len() != 1 {
                return Err(StoreError::InvalidHistory);
            }
            return Ok(ReducedRunState::new(
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
                            return Ok(ReducedRunState::new(
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
                            return Ok(ReducedRunState::new(
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
                            return Ok(ReducedRunState::new(
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
                            return Ok(ReducedRunState::new(
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
pub(crate) fn advance_reduced(
    reduced: &ReducedRunState,
    previous: &QualifiedRun,
    next: &QualifiedRun,
    document: &ProgramDocument,
) -> Result<ReducedRunState> {
    if !reduced.belongs_to(previous)
        || next.run_id() != previous.run_id()
        || next.head_sequence() != previous.head_sequence().saturating_add(1)
    {
        return Err(StoreError::Identity);
    }
    let conclusion = match next.frames().last().map(RunFrame::record) {
        Some(RunRecord::StateConcluded(value)) => value,
        _ => return Err(StoreError::InvalidRecord),
    };
    let occurrence = conclusion.occurrence().clone();
    match &reduced.action {
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
        apply_success_or_failure(state, &reduced.latest_context, conclusion.outcome())?;
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
                return Ok(ReducedRunState::new(next, latest, latest_object, action));
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
) -> Result<ReducedRunState> {
    match outcome {
        StateOutcome::Success(result) => Ok(ReducedRunState::new(
            run,
            latest,
            latest_object,
            RunAction::Terminal {
                occurrence: state.address().clone(),
                result: result.clone(),
            },
        )),
        StateOutcome::Failure(failure) => Ok(ReducedRunState::new(
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
    document: &ProgramDocument,
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
            if state.terminal() && document.root_contract_ref() == value.contract_ref() {
                Ok(ConclusionTransition::Terminal)
            } else {
                Err(StoreError::InvalidHistory)
            }
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
    let object_type =
        mfm_ids::StableId::new("mfm.match-payload").map_err(|_| StoreError::InvalidHistory)?;
    let selected_object = mfm_journal::single_trust::ImmutableObject::new(
        object_type,
        selected.value_ref().clone(),
        payload.as_str().to_owned(),
    )
    .map_err(|_| StoreError::InvalidHistory)?;
    Ok((selected, variant.entry_address().clone(), selected_object))
}

fn object_for_value(
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
    let binding_ref = prepared
        .binding()
        .content_ref()
        .map_err(|_| StoreError::InvalidRecord)?;
    if state.execution().is_pure()
        || prepared.input().contract_ref() != state.input_contract_ref()
        || prepared.execution_binding_ref()
            != state
                .execution_binding_ref()
                .ok_or(StoreError::InvalidRecord)?
        || prepared.binding().state_implementation_ref() != state.state_implementation_ref()
        || binding_ref != *prepared.execution_binding_ref()
        || prepared.maximum_conclusion_bytes() != state.maximum_conclusion_bytes()
        || prepared.fact_request().is_some() != prepared.fact_selection().is_some()
        || prepared.fact_request().is_some() != state.fact_selection_required()
    {
        return Err(StoreError::InvalidRecord);
    }
    let capability_ref = prepared
        .binding()
        .capability_contract_ref()
        .ok_or(StoreError::InvalidRecord)?;
    if prepared.binding().adapter_implementation_ref().is_none() {
        return Err(StoreError::InvalidRecord);
    }
    match (state.execution(), prepared.mode()) {
        (
            ExecutionMode::Read {
                capability_contract_ref,
                total_attempt_bound,
                ..
            },
            mfm_journal::single_trust::PreparationMode::Read {
                total_attempt_bound: prepared_bound,
            },
        ) if capability_contract_ref == capability_ref
            && total_attempt_bound == prepared_bound
            && prepared.binding().effect_domain().is_none() =>
        {
            Ok(())
        }
        (
            ExecutionMode::Effect {
                capability_contract_ref,
                total_attempt_bound,
                absorbing,
                effect_domain,
                ..
            },
            mfm_journal::single_trust::PreparationMode::Effect {
                total_attempt_bound: prepared_bound,
                absorbing: prepared_absorbing,
            },
        ) if capability_contract_ref == capability_ref
            && total_attempt_bound == prepared_bound
            && absorbing == prepared_absorbing
            && prepared.binding().effect_domain() == Some(effect_domain) =>
        {
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
            fact_selection,
            fact_publication,
            ..
        } => {
            if fact_publication
                .as_ref()
                .is_some_and(|publication| Some(publication.selection()) != fact_selection.as_ref())
            {
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

fn validate_object_reachability(frames: &[RunFrame]) -> Result<()> {
    let mut objects = BTreeMap::new();
    let mut referenced = BTreeSet::new();
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
                validate_fact_pair(&objects, request, selection)?;
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
    Ok(())
}

fn validate_fact_pair(
    objects: &BTreeMap<ContentRef, mfm_journal::single_trust::ImmutableObject>,
    request_ref: &ValueRef,
    selection_ref: &ValueRef,
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
                    values.push(publication.selection());
                }
            } else if let StateConcluded::Pure {
                fact_publication: Some(publication),
                ..
            } = conclusion
            {
                values.push(publication.selection());
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

/// One canonical configuration revision retained outside the run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationRevision {
    sequence: u64,
    append_request_id: AppendRequestId,
    canonical_json: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}

impl ConfigurationRevision {
    /// Constructs one validated configuration revision from source JSON.
    pub fn new(
        sequence: u64,
        append_request_id: AppendRequestId,
        canonical_json: String,
    ) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_json_str(&canonical_json)
            .map_err(|_| StoreError::InvalidRecord)?;
        let value: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).map_err(|_| StoreError::InvalidRecord)?;
        if sequence == 0
            || canonical.as_bytes().len()
                > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
            || contains_secret_marker(&value)
        {
            return Err(StoreError::InvalidRecord);
        }
        let content_ref = configuration_content_ref(canonical.as_bytes())?;
        Ok(Self {
            sequence,
            append_request_id,
            canonical_json: canonical,
            content_ref,
        })
    }

    /// Constructs one retained revision after checking its stored content identity.
    pub fn from_parts(
        sequence: u64,
        append_request_id: AppendRequestId,
        canonical_json: String,
        content_ref: ContentRef,
    ) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_json_str(&canonical_json)
            .map_err(|_| StoreError::InvalidRecord)?;
        if canonical.as_str() != canonical_json {
            return Err(StoreError::InvalidRecord);
        }
        let revision = Self::new(sequence, append_request_id, canonical_json)?;
        if revision.content_ref != content_ref {
            return Err(StoreError::InvalidRecord);
        }
        Ok(revision)
    }

    /// Returns the one-based revision sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the physical append identity that created this revision.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the canonical revision bytes.
    pub fn canonical_json(&self) -> &str {
        self.canonical_json.as_str()
    }

    /// Returns the content identity of the canonical revision.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

/// One fixed, fully qualified configuration snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationSnapshot {
    revisions: Vec<ConfigurationRevision>,
    total_bytes: usize,
}

impl ConfigurationSnapshot {
    pub(crate) fn from_revisions(revisions: Vec<ConfigurationRevision>) -> Result<Self> {
        if revisions.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
            || revisions
                .iter()
                .enumerate()
                .any(|(index, revision)| revision.sequence() != index as u64 + 1)
        {
            return Err(StoreError::InvalidHistory);
        }
        let total_bytes = revisions.iter().try_fold(0usize, |total, revision| {
            total
                .checked_add(revision.canonical_json().len())
                .filter(|bytes| *bytes <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES)
                .ok_or(StoreError::Capacity)
        })?;
        Ok(Self {
            revisions,
            total_bytes,
        })
    }

    /// Returns the current one-based configuration head, or zero for an empty stream.
    pub const fn head_sequence(&self) -> u64 {
        self.revisions.len() as u64
    }

    /// Returns the cumulative canonical bytes in this snapshot.
    pub const fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Returns the dense validated revisions in order.
    pub fn revisions(&self) -> &[ConfigurationRevision] {
        &self.revisions
    }

    /// Returns the latest revision, when the stream is non-empty.
    pub fn latest(&self) -> Option<&ConfigurationRevision> {
        self.revisions.last()
    }

    /// Promotes one direct successor into a new local snapshot without reading the backend.
    pub fn with_successor(&self, revision: ConfigurationRevision) -> Result<Self> {
        if revision.sequence() != self.head_sequence().saturating_add(1) {
            return Err(StoreError::Conflict);
        }
        let mut revisions = self.revisions.clone();
        revisions.push(revision);
        Self::from_revisions(revisions)
    }
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

/// One test-only affine configuration successor for the in-memory semantic history tests.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct PreparedConfigurationAppend {
    pub(crate) expected_sequence: u64,
    pub(crate) append_request_id: AppendRequestId,
    pub(crate) revision: ConfigurationRevision,
}

#[cfg(test)]
impl PreparedConfigurationAppend {
    /// Consumes this owner through the one configuration append path.
    pub(crate) fn commit(
        self,
        history: &mut ConfigurationHistory,
    ) -> Result<ConfigurationAppendDisposition> {
        history.commit(self)
    }
}

/// Strict bounded configuration history used by Store's semantic unit tests.
#[cfg(test)]
pub(crate) struct ConfigurationHistory {
    revisions: Vec<ConfigurationRevision>,
    total_bytes: usize,
    append_requests: BTreeMap<AppendRequestId, u64>,
}

#[cfg(test)]
impl Default for ConfigurationHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl ConfigurationHistory {
    /// Creates an empty bounded configuration stream.
    pub(crate) const fn new() -> Self {
        Self {
            revisions: Vec::new(),
            total_bytes: 0,
            append_requests: BTreeMap::new(),
        }
    }

    /// Prepares one canonical configuration successor without changing history.
    pub(crate) fn prepare_append(
        &self,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        revision: String,
    ) -> Result<PreparedConfigurationAppend> {
        let successor = ConfigurationRevision::new(
            self.revisions.len() as u64 + 1,
            append_request_id.clone(),
            revision,
        )?;
        let bytes = successor.canonical_json.as_bytes().len();
        let duplicate_request = self.append_requests.contains_key(&append_request_id);
        if !duplicate_request
            && (bytes > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
                || self.revisions.len() >= mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
                || self.total_bytes.saturating_add(bytes)
                    > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES)
        {
            return Err(StoreError::Capacity);
        }
        Ok(PreparedConfigurationAppend {
            expected_sequence,
            append_request_id,
            revision: successor,
        })
    }

    fn commit(
        &mut self,
        prepared: PreparedConfigurationAppend,
    ) -> Result<ConfigurationAppendDisposition> {
        if let Some(sequence) = self.append_requests.get(&prepared.append_request_id) {
            let existing = self
                .revisions
                .get((*sequence).saturating_sub(1) as usize)
                .ok_or(StoreError::InvalidHistory)?;
            return if existing.canonical_json == prepared.revision.canonical_json
                && existing.content_ref == prepared.revision.content_ref
            {
                Ok(ConfigurationAppendDisposition::Found {
                    sequence: *sequence,
                })
            } else {
                Err(StoreError::Conflict)
            };
        }
        let actual_sequence = self.revisions.len() as u64;
        if prepared.expected_sequence != actual_sequence {
            return Ok(ConfigurationAppendDisposition::StaleHead { actual_sequence });
        }
        let bytes = prepared.revision.canonical_json.as_bytes().len();
        if bytes > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
            || self.revisions.len() >= mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
            || self.total_bytes.saturating_add(bytes)
                > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
        {
            return Err(StoreError::Capacity);
        }
        let sequence = actual_sequence.saturating_add(1);
        let mut revision = prepared.revision;
        revision.sequence = sequence;
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes)
            .ok_or(StoreError::Capacity)?;
        self.append_requests
            .insert(prepared.append_request_id, sequence);
        self.revisions.push(revision);
        Ok(ConfigurationAppendDisposition::NewlyCommitted { sequence })
    }

    /// Returns the current configuration head sequence.
    pub(crate) const fn head_sequence(&self) -> u64 {
        self.revisions.len() as u64
    }

    /// Returns the cumulative canonical bytes retained by this stream.
    pub(crate) const fn total_bytes(&self) -> usize {
        self.total_bytes
    }
}

fn configuration_content_ref(bytes: &[u8]) -> Result<ContentRef> {
    let schema = SchemaId::new(
        "mfm.configuration",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| StoreError::InvalidRecord)?;
    ContentRef::new(schema, raw_content_digest(bytes)).map_err(|_| StoreError::InvalidRecord)
}

fn contains_secret_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let text = text.to_ascii_lowercase();
            [
                "password",
                "passphrase",
                "mnemonic",
                "private_key",
                "privatekey",
                "secret",
                "access_token",
                "api_key",
            ]
            .iter()
            .any(|marker| text.contains(marker))
        }
        serde_json::Value::Array(values) => values.iter().any(contains_secret_marker),
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            contains_secret_marker(&serde_json::Value::String(key.clone()))
                || contains_secret_marker(value)
        }),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_canonical::raw_content_digest;
    use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
    use mfm_journal::single_trust::{
        BindingDescriptor, ImmutableObject, PreparationMode, RunAdmitted, SequentialControlAddress,
        StateOutcome, ValueRef,
    };

    fn content(seed: u8) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                "mfm.test.value",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0; 32]),
            )
            .expect("schema"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                DigestBytes::from_array([seed; 32]),
            ),
        )
        .expect("content")
    }

    fn ids() -> (StoreScopeId, TenantScopeId, RunId) {
        (
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").expect("scope"),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").expect("tenant"),
            RunId::parse("run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .expect("run"),
        )
    }

    fn context_value_ref() -> ContentRef {
        ContentRef::new(content(2).schema_id().clone(), raw_content_digest(b"null"))
            .expect("context value")
    }

    fn value_ref(contract: &ContentRef, json: &str) -> ValueRef {
        ValueRef::new(
            contract.clone(),
            ContentRef::new(
                contract.schema_id().clone(),
                raw_content_digest(json.as_bytes()),
            )
            .expect("value ref"),
        )
    }

    fn value_object(value: &ValueRef, json: &str) -> ImmutableObject {
        ImmutableObject::new(
            StableId::new("mfm.value").expect("object type"),
            value.value_ref().clone(),
            json.to_owned(),
        )
        .expect("value object")
    }

    fn pure_state(
        ordinal: u32,
        input: ContentRef,
        output: ContentRef,
        terminal: bool,
        next: Option<SequentialControlAddress>,
    ) -> Declaration {
        let address = SequentialControlAddress::new(ordinal, Vec::new()).expect("address");
        let implementation = content(ordinal as u8 + 20);
        let state = match next {
            Some(next) => StateDeclaration::with_next(
                address,
                implementation,
                input,
                output,
                None,
                ExecutionMode::Pure,
                next,
            )
            .expect("state"),
            None => StateDeclaration::new(
                address,
                implementation,
                input,
                output,
                None,
                ExecutionMode::Pure,
                terminal,
            )
            .expect("state"),
        };
        Declaration::State(Box::new(state))
    }

    fn admission(scope: StoreScopeId, tenant: TenantScopeId, run: RunId) -> RunFrame {
        let context_ref = context_value_ref();
        RunFrame::new(
            run.clone(),
            scope.clone(),
            StoreEpoch::new(1),
            1,
            AppendRequestId::new("append-request-0123456789abcdef").expect("append"),
            RunRecord::RunAdmitted(
                RunAdmitted::new(
                    scope,
                    StoreEpoch::new(1),
                    run,
                    tenant,
                    StableId::new("mfm.test-entry-1").expect("entry"),
                    content(1),
                    ValueRef::new(content(2), context_ref.clone()),
                    content(4),
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![ImmutableObject::new(
                StableId::new("mfm.value").expect("object type"),
                context_ref,
                "null".to_owned(),
            )
            .expect("context object")],
        )
        .expect("frame")
    }

    #[test]
    fn duplicate_admission_is_found_and_distinct_head_is_stale() {
        let (scope, tenant, run) = ids();
        let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
        let first = admission(scope.clone(), tenant.clone(), run.clone());
        assert_eq!(
            store.admit(first.clone()),
            Ok(AppendDisposition::NewlyCommitted { sequence: 1 })
        );
        assert_eq!(
            store.admit(first),
            Ok(AppendDisposition::Found { sequence: 1 })
        );
        let stale = {
            let admitted = admission(scope, tenant, run);
            RunFrame::new(
                admitted.run_id().clone(),
                admitted.store_scope_id().clone(),
                admitted.store_epoch(),
                3,
                AppendRequestId::new("append-request-2-0123456789abcdef").expect("append"),
                RunRecord::StateConcluded(StateConcluded::Pure {
                    occurrence: SequentialControlAddress::new(0, Vec::new()).expect("occurrence"),
                    outcome: StateOutcome::Success(ValueRef::new(content(5), content(6))),
                    fact_publication: None,
                }),
                Vec::new(),
            )
            .expect("candidate")
        };
        assert_eq!(
            store.append(stale),
            Ok(AppendDisposition::StaleHead { actual_sequence: 1 })
        );
    }

    #[test]
    fn configuration_writer_promotes_only_direct_successors() {
        let mut history = ConfigurationHistory::new();
        let request = AppendRequestId::new("config-append-0123456789abcdef").expect("request");
        let prepared = history
            .prepare_append(0, request.clone(), "{\"quote\":\"usd\"}".to_owned())
            .expect("prepare");
        assert_eq!(
            prepared.commit(&mut history),
            Ok(ConfigurationAppendDisposition::NewlyCommitted { sequence: 1 })
        );
        assert_eq!(history.head_sequence(), 1);
        assert_eq!(history.total_bytes(), 15);

        let retry = history
            .prepare_append(0, request, "{\"quote\":\"usd\"}".to_owned())
            .expect("retry prepare");
        assert_eq!(
            retry.commit(&mut history),
            Ok(ConfigurationAppendDisposition::Found { sequence: 1 })
        );
        let stale = history
            .prepare_append(
                0,
                AppendRequestId::new("config-append-2-0123456789ab").expect("request"),
                "{\"quote\":\"eur\"}".to_owned(),
            )
            .expect("stale prepare");
        assert_eq!(
            stale.commit(&mut history),
            Ok(ConfigurationAppendDisposition::StaleHead { actual_sequence: 1 })
        );
        assert_eq!(
            history
                .prepare_append(
                    1,
                    AppendRequestId::new("config-secret-0123456789ab").expect("request"),
                    "{\"secret\":\"x\"}".to_owned(),
                )
                .unwrap_err(),
            StoreError::InvalidRecord
        );
    }

    #[test]
    fn pure_conclusion_has_no_preparation_path() {
        let (scope, tenant, run) = ids();
        let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
        let occurrence = SequentialControlAddress::new(0, Vec::new()).expect("occurrence");
        let contract = content(2);
        let document = ProgramDocument::new(
            StableId::new("mfm.test-entry-1").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![pure_state(
                0,
                contract.clone(),
                contract.clone(),
                true,
                None,
            )],
        )
        .expect("document");
        let context_ref = context_value_ref();
        let admitted = RunFrame::new(
            run.clone(),
            scope.clone(),
            StoreEpoch::new(1),
            1,
            AppendRequestId::new("append-pure-admission").expect("append"),
            RunRecord::RunAdmitted(
                RunAdmitted::new(
                    scope.clone(),
                    StoreEpoch::new(1),
                    run.clone(),
                    tenant.clone(),
                    StableId::new("mfm.test-entry-1").expect("entry"),
                    document.program_ref().expect("program"),
                    ValueRef::new(content(2), context_ref.clone()),
                    content(4),
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![value_object(
                &ValueRef::new(content(2), context_ref.clone()),
                "null",
            )],
        )
        .expect("frame");
        store.admit(admitted).expect("admit");
        let conclusion = StateConcluded::Pure {
            occurrence,
            outcome: StateOutcome::Success(ValueRef::new(contract, context_value_ref())),
            fact_publication: None,
        };
        let owner = store
            .prepare_conclusion(
                &run,
                &document,
                1,
                AppendRequestId::new("append-conclusion-0123456789ab").expect("append"),
                conclusion,
                Vec::new(),
                mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
            )
            .expect("prepare");
        assert_eq!(
            owner.commit(&store),
            Ok(AppendDisposition::NewlyCommitted { sequence: 2 })
        );
    }

    #[test]
    fn direct_new_preparation_matches_qualified_record_ordinal() {
        let (scope, tenant, run) = ids();
        let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
        let input_contract = content(70);
        let output_contract = content(71);
        let capability_contract = content(72);
        let state_implementation = content(73);
        let adapter_implementation = content(74);
        let physical_target = content(75);
        let binding = BindingDescriptor::new(
            state_implementation.clone(),
            Some(capability_contract.clone()),
            Some(adapter_implementation),
            physical_target,
            None,
            None,
        )
        .expect("binding");
        let execution_binding_ref = binding.content_ref().expect("binding ref");
        let state = StateDeclaration::new(
            SequentialControlAddress::new(0, Vec::new()).expect("address"),
            state_implementation,
            input_contract.clone(),
            output_contract,
            None,
            ExecutionMode::Read {
                capability_contract_ref: capability_contract,
                total_attempt_bound: 1,
                fact_selection_required: false,
            },
            true,
        )
        .expect("state")
        .with_execution_binding(execution_binding_ref.clone())
        .expect("execution binding");
        let document = ProgramDocument::new(
            StableId::new("mfm.test-access").expect("entry"),
            state.output_contract_ref().clone(),
            input_contract.clone(),
            vec![Declaration::State(Box::new(state))],
        )
        .expect("document");
        let input = value_ref(&input_contract, "null");
        let intent_contract = content(76);
        let intent = value_ref(&intent_contract, "null");
        let admission = RunFrame::new(
            run.clone(),
            scope.clone(),
            StoreEpoch::new(1),
            1,
            AppendRequestId::new("append-access-admission").expect("append"),
            RunRecord::RunAdmitted(
                RunAdmitted::new(
                    scope.clone(),
                    StoreEpoch::new(1),
                    run.clone(),
                    tenant.clone(),
                    StableId::new("mfm.test-access").expect("entry"),
                    document.program_ref().expect("program"),
                    input.clone(),
                    content(77),
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![value_object(&input, "null")],
        )
        .expect("admission frame");
        store.admit(admission).expect("admit");

        let prepared = StatePrepared::new(
            SequentialControlAddress::new(0, Vec::new()).expect("address"),
            0,
            input,
            intent,
            None,
            None,
            PreparationMode::Read {
                total_attempt_bound: 1,
            },
            binding,
            execution_binding_ref,
            None,
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("prepared");
        let append = store
            .prepare_access(
                &run,
                &document,
                1,
                AppendRequestId::new("append-access-preparation").expect("append"),
                prepared,
                vec![value_object(&value_ref(&intent_contract, "null"), "null")],
            )
            .expect("prepare");
        let assigned = append.preparation().cloned().expect("assigned preparation");
        let retained = store.load(&run).expect("retained");
        let (_, selected) = retained
            .selected_preparation(&SequentialControlAddress::new(0, Vec::new()).expect("address"))
            .expect("selected preparation");
        assert_eq!(assigned, selected);
        assert_eq!(assigned.record_ordinal(), 1);
    }

    #[test]
    fn reducer_advances_the_exact_direct_successor_context() {
        let (scope, tenant, run) = ids();
        let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
        let input_contract = content(30);
        let middle_contract = content(31);
        let root_value = value_ref(&input_contract, "{\"step\":0}");
        let middle_value = value_ref(&middle_contract, "{\"step\":1}");
        let final_value = value_ref(&middle_contract, "{\"step\":2}");
        let fact_selection = value_ref(&content(60), "null");
        let second_address = SequentialControlAddress::new(2, Vec::new()).expect("address");
        let document = ProgramDocument::new(
            StableId::new("mfm.test-sequential").expect("entry"),
            middle_contract.clone(),
            input_contract.clone(),
            vec![
                pure_state(
                    1,
                    input_contract.clone(),
                    middle_contract.clone(),
                    false,
                    Some(second_address.clone()),
                ),
                pure_state(
                    2,
                    middle_contract.clone(),
                    middle_contract.clone(),
                    true,
                    None,
                ),
            ],
        )
        .expect("document");
        let admission = RunFrame::new(
            run.clone(),
            scope.clone(),
            StoreEpoch::new(1),
            1,
            AppendRequestId::new("append-sequential-admission").expect("append"),
            RunRecord::RunAdmitted(
                RunAdmitted::new(
                    scope.clone(),
                    StoreEpoch::new(1),
                    run.clone(),
                    tenant.clone(),
                    StableId::new("mfm.test-sequential").expect("entry"),
                    document.program_ref().expect("program"),
                    root_value.clone(),
                    content(41),
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![value_object(&root_value, "{\"step\":0}")],
        )
        .expect("admission frame");
        store.admit(admission).expect("admit");
        let ready = store.reduce(&run, document.clone()).expect("ready");
        assert!(matches!(ready.action, RunAction::ReadyPure { .. }));

        let first = RunFrame::new(
            run.clone(),
            scope.clone(),
            StoreEpoch::new(1),
            2,
            AppendRequestId::new("append-sequential-first").expect("append"),
            RunRecord::StateConcluded(StateConcluded::Pure {
                occurrence: SequentialControlAddress::new(1, Vec::new()).expect("address"),
                outcome: StateOutcome::Success(middle_value.clone()),
                fact_publication: Some(
                    mfm_journal::single_trust::FactPublication::new(1, fact_selection.clone())
                        .expect("publication"),
                ),
            }),
            vec![
                value_object(&middle_value, "{\"step\":1}"),
                value_object(&fact_selection, "null"),
            ],
        )
        .expect("first frame");
        assert_eq!(
            store.append(first.clone()),
            Ok(AppendDisposition::NewlyCommitted { sequence: 2 })
        );
        assert_eq!(
            store.append(first),
            Ok(AppendDisposition::Found { sequence: 2 })
        );
        let next = store.reduce(&run, document.clone()).expect("next");
        assert_eq!(next.latest_context, middle_value);
        assert!(matches!(next.action, RunAction::ReadyPure { .. }));

        let second = RunFrame::new(
            run.clone(),
            scope,
            StoreEpoch::new(1),
            3,
            AppendRequestId::new("append-sequential-second").expect("append"),
            RunRecord::StateConcluded(StateConcluded::Pure {
                occurrence: second_address,
                outcome: StateOutcome::Success(final_value.clone()),
                fact_publication: None,
            }),
            vec![value_object(&final_value, "{\"step\":2}")],
        )
        .expect("second frame");
        store.append(second).expect("second conclusion");
        let terminal = store.reduce(&run, document).expect("terminal");
        assert_eq!(terminal.latest_context, final_value);
        assert!(matches!(terminal.action, RunAction::Terminal { .. }));
    }

    #[test]
    fn reducer_materializes_one_selected_match_payload() {
        let (scope, tenant, run) = ids();
        let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
        let selector_contract = content(50);
        let payload_contract = content(51);
        let result_contract = content(52);
        let selector_value = value_ref(
            &selector_contract,
            "{\"kind\":\"left\",\"value\":{\"n\":1}}",
        );
        let payload_value = value_ref(&payload_contract, "{\"n\":1}");
        let result_value = value_ref(&result_contract, "{\"result\":2}");
        let state_address = SequentialControlAddress::new(2, Vec::new()).expect("address");
        let selector = mfm_program::MatchDeclaration::new(
            SequentialControlAddress::new(1, Vec::new()).expect("address"),
            selector_contract.clone(),
            vec![mfm_program::MatchVariant::new(
                StableId::new("left").expect("tag"),
                payload_contract.clone(),
                result_contract.clone(),
                state_address.clone(),
            )],
        )
        .expect("selector");
        let document = ProgramDocument::new(
            StableId::new("mfm.test-match").expect("entry"),
            result_contract.clone(),
            selector_contract.clone(),
            vec![
                Declaration::Match(selector),
                pure_state(2, payload_contract.clone(), result_contract, true, None),
            ],
        )
        .expect("document");
        let admission = RunFrame::new(
            run.clone(),
            scope,
            StoreEpoch::new(1),
            1,
            AppendRequestId::new("append-match-admission").expect("append"),
            RunRecord::RunAdmitted(
                RunAdmitted::new(
                    store.scope().clone(),
                    StoreEpoch::new(1),
                    run.clone(),
                    tenant,
                    StableId::new("mfm.test-match").expect("entry"),
                    document.program_ref().expect("program"),
                    selector_value.clone(),
                    content(53),
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![ImmutableObject::new(
                StableId::new("mfm.value").expect("object type"),
                selector_value.value_ref().clone(),
                "{\"kind\":\"left\",\"value\":{\"n\":1}}".to_owned(),
            )
            .expect("selector object")],
        )
        .expect("admission frame");
        store.admit(admission).expect("admit");
        let selected = store.reduce(&run, document.clone()).expect("selected arm");
        assert_eq!(selected.latest_context, payload_value);
        assert_eq!(selected.latest_context_object.canonical_json(), "{\"n\":1}");
        assert!(matches!(selected.action, RunAction::ReadyPure { .. }));
        let owner = store
            .prepare_conclusion(
                &run,
                &document,
                1,
                AppendRequestId::new("append-match-conclusion").expect("append"),
                StateConcluded::Pure {
                    occurrence: state_address,
                    outcome: StateOutcome::Success(result_value.clone()),
                    fact_publication: None,
                },
                vec![value_object(&result_value, "{\"result\":2}")],
                mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
            )
            .expect("match conclusion");
        assert_eq!(owner.frame().objects().len(), 1);
        assert_eq!(
            owner.frame().objects()[0].canonical_json(),
            "{\"result\":2}"
        );
        owner.commit(&store).expect("commit match conclusion");
        let terminal = store.reduce(&run, document).expect("terminal arm");
        assert!(matches!(terminal.action, RunAction::Terminal { .. }));
    }
}
