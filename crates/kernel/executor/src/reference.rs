use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use mfm_canonical::{CanonicalValue, ValidatedCanonicalValueV1};
use mfm_capabilities::{BoundaryStage, FailureClass};
use mfm_ids::{AttemptId, ContentRef, RequestDigest};

use crate::codec::{Decoder, Encoder};
use crate::contract::{
    canonical_object, encode, validate_reference_effect_identifier, CanonicalExecutorRequest,
    CommittedEffectRequest, SchemaQualifiedCanonicalValue,
};
use crate::engine::{ExecutorLedgerStore, KeyedExecutorLedger};
use crate::frontier::{
    reference_safe_failure, DeliveryAttemptOutcome, DeliveryAudit, ReferenceFailureCode,
    ReferenceTerminalProof, ReturnedOutcome, TerminalTombstone,
};
use crate::ledger::{EffectEntryView, TargetEntryAuthority, TargetOperationReceipt};
use crate::retained::{
    verify_ensure_result, ExecutorEnsureResultClaim, ExecutorRetainedClosureClaim,
    ExecutorTerminalEvidenceClaim, ProofBasis, VerifiedEnsureResult,
};
use crate::{ExecutorError, Result};

const REFERENCE_REQUEST_SCHEMA: &str = "mfm.executor-reference-queue-request.v1";
const REFERENCE_RESULT_SCHEMA: &str = "mfm.executor-reference-queue-result.v1";
const VALUE_REF_SCHEMA: &str = "mfm.value-ref.v1";
const STABLE_ID_SCHEMA: &str = "mfm.primitive-stable_id.v1";
const DESTINATION_CHECKPOINT_MAGIC: &[u8; 8] = b"MFMEDQ02";
const MAX_DESTINATION_ITEMS: usize = 1_000_000;

/// Pure request used by the convergence-safe reference queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceRequest {
    external_operation_identity: String,
    payload_ref: ValidatedCanonicalValueV1,
    canonical: SchemaQualifiedCanonicalValue,
}

impl ReferenceRequest {
    /// Constructs the exact queue request from one producer-bound value-ref
    /// object.
    pub fn new(
        external_operation_identity: impl Into<String>,
        payload_ref: ValidatedCanonicalValueV1,
    ) -> Result<Self> {
        if payload_ref.schema_contract() != VALUE_REF_SCHEMA {
            return Err(ExecutorError::SchemaReferenceMismatch);
        }
        let external_operation_identity = external_operation_identity.into();
        validate_reference_effect_identifier(&external_operation_identity)?;
        let validated = encode(
            REFERENCE_REQUEST_SCHEMA,
            &canonical_object([
                (
                    "external_operation_identity",
                    CanonicalValue::String(external_operation_identity.clone()),
                ),
                (
                    "payload_ref",
                    payload_ref
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
                (
                    "version",
                    CanonicalValue::String(REFERENCE_REQUEST_SCHEMA.to_owned()),
                ),
            ])?,
        )?;
        Ok(Self {
            external_operation_identity,
            payload_ref,
            canonical: SchemaQualifiedCanonicalValue::from_validated(&validated)?,
        })
    }

    /// Strictly reconstructs a request from the exact frozen queue schema.
    pub fn from_validated(validated: ValidatedCanonicalValueV1) -> Result<Self> {
        if validated.schema_contract() != REFERENCE_REQUEST_SCHEMA {
            return Err(ExecutorError::SchemaReferenceMismatch);
        }
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let external_operation_identity = json
            .get("external_operation_identity")
            .and_then(serde_json::Value::as_str)
            .ok_or(ExecutorError::CanonicalEncoding)?
            .to_owned();
        validate_reference_effect_identifier(&external_operation_identity)?;
        let payload_json = json
            .get("payload_ref")
            .ok_or(ExecutorError::CanonicalEncoding)?;
        let payload_bytes =
            serde_json::to_vec(payload_json).map_err(|_| ExecutorError::CanonicalEncoding)?;
        let payload_ref = crate::contract::recoverability_contract()?
            .strict_decode(VALUE_REF_SCHEMA, &payload_bytes)
            .map_err(crate::contract::contract_error)?;
        let rebuilt = Self::new(external_operation_identity, payload_ref)?;
        if rebuilt.canonical.as_bytes() != validated.as_bytes() {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(rebuilt)
    }

    /// Returns the destination-native semantic operation identity.
    pub fn external_operation_identity(&self) -> &str {
        &self.external_operation_identity
    }

    /// Returns the complete producer-bound payload reference.
    pub const fn payload_ref(&self) -> &ValidatedCanonicalValueV1 {
        &self.payload_ref
    }
}

impl CanonicalExecutorRequest for ReferenceRequest {
    fn canonical_request(&self) -> &SchemaQualifiedCanonicalValue {
        &self.canonical
    }
}

/// Exact reviewed operation and evidence refs for the reference executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceContract {
    destination_domain_ref: ContentRef,
    enqueue_operation: SchemaQualifiedCanonicalValue,
    enqueue_operation_ref: ContentRef,
    terminal_outcome: String,
    assurance_policy_ref: ContentRef,
    safe_failure_contract_ref: ContentRef,
}

impl ReferenceContract {
    /// Constructs one immutable reference-executor contract.
    pub fn new(
        destination_domain_ref: ContentRef,
        enqueue_operation: SchemaQualifiedCanonicalValue,
        terminal_outcome: impl Into<String>,
        assurance_policy_ref: ContentRef,
        safe_failure_contract_ref: ContentRef,
    ) -> Result<Self> {
        let terminal_outcome = terminal_outcome.into();
        encode(
            STABLE_ID_SCHEMA,
            &CanonicalValue::String(terminal_outcome.clone()),
        )?;
        let enqueue_operation_ref = enqueue_operation.reference()?;
        Ok(Self {
            destination_domain_ref,
            enqueue_operation,
            enqueue_operation_ref,
            terminal_outcome,
            assurance_policy_ref,
            safe_failure_contract_ref,
        })
    }

    /// Returns the convergence-safe semantic destination domain.
    pub const fn destination_domain_ref(&self) -> &ContentRef {
        &self.destination_domain_ref
    }

    /// Returns the sole reviewed target operation.
    pub const fn enqueue_operation_ref(&self) -> &ContentRef {
        &self.enqueue_operation_ref
    }

    /// Returns the exact retained target-operation descriptor.
    pub const fn enqueue_operation(&self) -> &SchemaQualifiedCanonicalValue {
        &self.enqueue_operation
    }

    /// Returns the fixed terminal outcome spelling.
    pub fn terminal_outcome(&self) -> &str {
        &self.terminal_outcome
    }

    /// Returns the admitted terminal assurance policy.
    pub const fn assurance_policy_ref(&self) -> &ContentRef {
        &self.assurance_policy_ref
    }

    /// Returns the exact closed safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.safe_failure_contract_ref
    }
}

/// Conformance behavior selected below the reference target boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceTargetBehavior {
    /// Enter the convergence-safe queue normally.
    Available,
    /// Prove transport failure before target entry.
    UnavailableBeforeEntry,
    /// Lose transport certainty during target entry.
    UnavailableDuringEntry,
    /// Prove cooperative cancellation before target entry.
    CancelledBeforeEntry,
    /// Lose cancellation certainty during target entry.
    CancelledDuringEntry,
}

/// One destination call's receipt plus any exact safe queue-result object.
pub struct ReferenceDestinationReturn {
    receipt: TargetOperationReceipt,
    safe_result: Option<ValidatedCanonicalValueV1>,
}

impl ReferenceDestinationReturn {
    /// Returns the affine exact-attempt receipt.
    pub const fn receipt(&self) -> &TargetOperationReceipt {
        &self.receipt
    }

    /// Returns the exact safe queue result when one survived.
    pub const fn safe_result(&self) -> Option<&ValidatedCanonicalValueV1> {
        self.safe_result.as_ref()
    }

    /// Consumes the wrapper into its one-shot target receipt.
    pub fn into_target_receipt(self) -> TargetOperationReceipt {
        self.receipt
    }
}

impl std::fmt::Debug for ReferenceDestinationReturn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferenceDestinationReturn")
            .field("receipt", &self.receipt)
            .field("has_safe_result", &self.safe_result.is_some())
            .finish()
    }
}

/// Convergence-safe semantic destination used by the reference executor.
pub trait ReferenceDestination: Clone + Send + Sync + 'static {
    /// Consumes one affine authority at the transactional queue boundary.
    fn enqueue<'a>(
        &'a self,
        authority: TargetEntryAuthority,
        request: &'a ReferenceRequest,
        contract: &'a ReferenceContract,
        behavior: ReferenceTargetBehavior,
    ) -> crate::ExecutorFuture<'a, Result<ReferenceDestinationReturn>>;
}

#[derive(Debug, Clone)]
struct QueueEntry {
    request_digest: RequestDigest,
    queue_position: u64,
}

#[derive(Debug, Clone, Default)]
struct DestinationState {
    active_generation: Option<ContentRef>,
    fenced_generations: BTreeSet<ContentRef>,
    queue: BTreeMap<String, QueueEntry>,
    seen_attempts: BTreeSet<AttemptId>,
    target_entry_count: u64,
}

/// Opaque exact-generation checkpoint for the memory queue.
#[derive(Debug, Clone)]
pub struct MemoryDestinationCheckpoint {
    state: DestinationState,
}

impl MemoryDestinationCheckpoint {
    /// Encodes the complete queue authority into a bounded checksum snapshot.
    pub fn to_durable_bytes(&self) -> Result<Vec<u8>> {
        let mut encoder = Encoder::new(DESTINATION_CHECKPOINT_MAGIC);
        crate::frontier::encode_optional_content_ref(
            &mut encoder,
            self.state.active_generation.as_ref(),
        )?;
        encoder.usize(self.state.fenced_generations.len())?;
        for generation in &self.state.fenced_generations {
            encoder.content_ref(generation)?;
        }
        encoder.usize(self.state.queue.len())?;
        for (key, entry) in &self.state.queue {
            encoder.string(key)?;
            encoder.string(entry.request_digest.as_str())?;
            encoder.u64(entry.queue_position);
        }
        encoder.usize(self.state.seen_attempts.len())?;
        for attempt_id in &self.state.seen_attempts {
            encoder.string(attempt_id.as_str())?;
        }
        encoder.u64(self.state.target_entry_count);
        encoder.finish()
    }

    /// Strictly decodes one bounded queue checkpoint.
    pub fn from_durable_bytes(bytes: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, DESTINATION_CHECKPOINT_MAGIC)?;
        let active_generation = crate::frontier::decode_optional_content_ref(&mut decoder)?;
        let fenced_count = decoder.count(MAX_DESTINATION_ITEMS, 9)?;
        let mut fenced_generations = BTreeSet::new();
        for _ in 0..fenced_count {
            if !fenced_generations.insert(decoder.content_ref()?) {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        if active_generation
            .as_ref()
            .is_some_and(|value| fenced_generations.contains(value))
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }

        let queue_count = decoder.count(MAX_DESTINATION_ITEMS, 25)?;
        let mut queue = BTreeMap::new();
        let mut queue_positions = BTreeSet::new();
        let max_queue_position =
            u64::try_from(queue_count).map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        for _ in 0..queue_count {
            let key = decoder.string()?;
            validate_reference_effect_identifier(&key)?;
            let entry = QueueEntry {
                request_digest: decoder.request_digest()?,
                queue_position: decoder.u64()?,
            };
            if entry.queue_position == 0
                || entry.queue_position > max_queue_position
                || !queue_positions.insert(entry.queue_position)
                || queue.insert(key, entry).is_some()
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        let attempt_count = decoder.count(MAX_DESTINATION_ITEMS, 9)?;
        let mut seen_attempts = BTreeSet::new();
        for _ in 0..attempt_count {
            if !seen_attempts.insert(decoder.attempt_id()?) {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        let target_entry_count = decoder.u64()?;
        decoder.finish()?;
        if target_entry_count
            != u64::try_from(seen_attempts.len())
                .map_err(|_| ExecutorError::InvalidDurableSnapshot)?
            || (active_generation.is_none() && (!queue.is_empty() || !seen_attempts.is_empty()))
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(Self {
            state: DestinationState {
                active_generation,
                fenced_generations,
                queue,
                seen_attempts,
                target_entry_count,
            },
        })
    }
}

/// In-memory transactional convergence-safe queue conformance destination.
#[derive(Debug, Clone, Default)]
pub struct MemoryConvergentDestination {
    state: Arc<Mutex<DestinationState>>,
}

impl MemoryConvergentDestination {
    /// Creates an empty destination with no active executor generation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Activates the first exact durable executor generation.
    pub fn activate_generation(&self, generation: ContentRef) -> Result<()> {
        let mut guard = self.lock_state()?;
        if guard.fenced_generations.contains(&generation) {
            return Err(ExecutorError::DestinationGenerationFenced);
        }
        match &guard.active_generation {
            None => guard.active_generation = Some(generation),
            Some(active) if active == &generation => {}
            Some(_) => {
                return Err(ExecutorError::DestinationFenceMismatch);
            }
        }
        Ok(())
    }

    /// Permanently fences one active generation and activates its successor.
    pub fn fence_and_activate(
        &self,
        expected_active: &ContentRef,
        successor: ContentRef,
    ) -> Result<()> {
        let mut guard = self.lock_state()?;
        if guard.active_generation.as_ref() != Some(expected_active)
            || &successor == expected_active
            || guard.fenced_generations.contains(&successor)
        {
            return Err(ExecutorError::DestinationFenceMismatch);
        }
        guard.fenced_generations.insert(expected_active.clone());
        guard.active_generation = Some(successor);
        Ok(())
    }

    /// Returns the number of independently entered target attempts.
    pub fn target_entry_count(&self) -> Result<u64> {
        Ok(self.lock_state()?.target_entry_count)
    }

    /// Returns the number of distinct convergence-key mutations.
    pub fn semantic_mutation_count(&self) -> Result<u64> {
        u64::try_from(self.lock_state()?.queue.len())
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    /// Captures a deep exact-generation checkpoint.
    pub fn checkpoint(&self) -> Result<MemoryDestinationCheckpoint> {
        Ok(MemoryDestinationCheckpoint {
            state: self.lock_state()?.clone(),
        })
    }

    /// Restores an independent process from an exact checkpoint.
    pub fn restore(checkpoint: MemoryDestinationCheckpoint) -> Self {
        Self {
            state: Arc::new(Mutex::new(checkpoint.state)),
        }
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, DestinationState>> {
        self.state
            .lock()
            .map_err(|_| ExecutorError::SynchronizationFailure)
    }

    /// Enters the in-memory destination synchronously for blocking conformance stores.
    pub fn enter(
        &self,
        authority: TargetEntryAuthority,
        request: &ReferenceRequest,
        contract: &ReferenceContract,
        behavior: ReferenceTargetBehavior,
    ) -> Result<ReferenceDestinationReturn> {
        if authority.target_operation_ref() != contract.enqueue_operation_ref() {
            return Err(ExecutorError::TargetOperationMismatch);
        }
        validate_reference_effect_identifier(request.external_operation_identity())?;

        let failure = |code, class, stage| {
            reference_safe_failure(
                contract.safe_failure_contract_ref().clone(),
                code,
                class,
                stage,
            )
        };
        match behavior {
            ReferenceTargetBehavior::UnavailableBeforeEntry => {
                let outcome = DeliveryAttemptOutcome::did_not_enter(failure(
                    ReferenceFailureCode::DestinationUnavailable,
                    FailureClass::Transport,
                    BoundaryStage::BeforeBoundaryEntry,
                )?)?;
                return Ok(ReferenceDestinationReturn {
                    receipt: authority.complete(outcome),
                    safe_result: None,
                });
            }
            ReferenceTargetBehavior::UnavailableDuringEntry => {
                let outcome = DeliveryAttemptOutcome::indeterminate(failure(
                    ReferenceFailureCode::DestinationUnavailable,
                    FailureClass::Transport,
                    BoundaryStage::BoundaryEntry,
                )?)?;
                return Ok(ReferenceDestinationReturn {
                    receipt: authority.complete(outcome),
                    safe_result: None,
                });
            }
            ReferenceTargetBehavior::CancelledBeforeEntry => {
                let outcome = DeliveryAttemptOutcome::did_not_enter(failure(
                    ReferenceFailureCode::AccessCancelled,
                    FailureClass::Cancellation,
                    BoundaryStage::BeforeBoundaryEntry,
                )?)?;
                return Ok(ReferenceDestinationReturn {
                    receipt: authority.complete(outcome),
                    safe_result: None,
                });
            }
            ReferenceTargetBehavior::CancelledDuringEntry => {
                let outcome = DeliveryAttemptOutcome::indeterminate(failure(
                    ReferenceFailureCode::AccessCancelled,
                    FailureClass::Cancellation,
                    BoundaryStage::BoundaryEntry,
                )?)?;
                return Ok(ReferenceDestinationReturn {
                    receipt: authority.complete(outcome),
                    safe_result: None,
                });
            }
            ReferenceTargetBehavior::Available => {}
        }

        let mut guard = self.lock_state()?;
        let generation = authority.durable_ledger_generation_ref().clone();
        if guard.fenced_generations.contains(&generation)
            || guard.active_generation.as_ref() != Some(&generation)
        {
            drop(guard);
            let outcome = DeliveryAttemptOutcome::did_not_enter(failure(
                ReferenceFailureCode::GenerationFenced,
                FailureClass::Authorization,
                BoundaryStage::BeforeBoundaryEntry,
            )?)?;
            return Ok(ReferenceDestinationReturn {
                receipt: authority.complete(outcome),
                safe_result: None,
            });
        }
        if guard.seen_attempts.contains(authority.attempt_id()) {
            return Err(ExecutorError::TargetAuthorityConsumed);
        }

        let mut staged = guard.clone();
        staged.seen_attempts.insert(authority.attempt_id().clone());
        staged.target_entry_count = staged
            .target_entry_count
            .checked_add(1)
            .ok_or(ExecutorError::InvalidDurableSnapshot)?;
        let key = request.external_operation_identity().to_owned();
        let (safe_result, outcome) = match staged.queue.get(&key) {
            Some(entry) if entry.request_digest == *authority.identity().request_digest() => {
                let safe_result = already_enqueued_result(&key)?;
                let outcome = DeliveryAttemptOutcome::returned(
                    SchemaQualifiedCanonicalValue::from_validated(&safe_result)?,
                )?;
                (safe_result, outcome)
            }
            Some(_) => {
                let safe_failure = failure(
                    ReferenceFailureCode::RequestConflict,
                    FailureClass::Destination,
                    BoundaryStage::BoundaryObservation,
                )?;
                let safe_result = request_conflict_result(&safe_failure)?;
                let outcome = DeliveryAttemptOutcome::indeterminate(safe_failure)?;
                (safe_result, outcome)
            }
            None => {
                let queue_position = u64::try_from(staged.queue.len())
                    .map_err(|_| ExecutorError::InvalidDurableSnapshot)?
                    .checked_add(1)
                    .ok_or(ExecutorError::InvalidDurableSnapshot)?;
                staged.queue.insert(
                    key.clone(),
                    QueueEntry {
                        request_digest: authority.identity().request_digest().clone(),
                        queue_position,
                    },
                );
                let safe_result = enqueued_result(&key, queue_position)?;
                let outcome = DeliveryAttemptOutcome::returned(
                    SchemaQualifiedCanonicalValue::from_validated(&safe_result)?,
                )?;
                (safe_result, outcome)
            }
        };
        *guard = staged;
        Ok(ReferenceDestinationReturn {
            receipt: authority.complete(outcome),
            safe_result: Some(safe_result),
        })
    }
}

impl ReferenceDestination for MemoryConvergentDestination {
    fn enqueue<'a>(
        &'a self,
        authority: TargetEntryAuthority,
        request: &'a ReferenceRequest,
        contract: &'a ReferenceContract,
        behavior: ReferenceTargetBehavior,
    ) -> crate::ExecutorFuture<'a, Result<ReferenceDestinationReturn>> {
        Box::pin(async move { self.enter(authority, request, contract, behavior) })
    }
}

/// Deterministic conformance crash boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceCrashPoint {
    /// Authorization committed but the target was not entered.
    BeforeTargetEntry,
    /// Queue mutation committed but the receipt was lost.
    AfterTargetMutationBeforeObservation,
    /// Returned observation committed but no tombstone committed.
    AfterObservationBeforeTombstone,
    /// Tombstone committed but the executor response was lost.
    AfterTombstoneBeforeReturn,
}

/// Observable result of one conformance drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceDriveOutcome {
    /// A complete pending or terminal result returned.
    Returned(Box<VerifiedEnsureResult>),
    /// The injected process crash stopped the drive at this boundary.
    Crashed(ReferenceCrashPoint),
}

/// Durable reference executor over one immutable ledger and semantic queue.
#[derive(Debug, Clone)]
pub struct ReferenceExecutor<Store, Destination>
where
    Store: ExecutorLedgerStore,
    Destination: ReferenceDestination,
{
    ledger: KeyedExecutorLedger<Store>,
    destination: Destination,
    contract: ReferenceContract,
}

impl<Store, Destination> ReferenceExecutor<Store, Destination>
where
    Store: ExecutorLedgerStore,
    Destination: ReferenceDestination,
{
    /// Constructs the reference executor for one exact binding.
    pub fn new(
        ledger: KeyedExecutorLedger<Store>,
        destination: Destination,
        contract: ReferenceContract,
    ) -> Self {
        Self {
            ledger,
            destination,
            contract,
        }
    }

    /// Returns the exact executor ledger.
    pub const fn ledger(&self) -> &KeyedExecutorLedger<Store> {
        &self.ledger
    }

    /// Returns the convergence-safe semantic destination.
    pub const fn destination(&self) -> &Destination {
        &self.destination
    }

    /// Drives one committed request without conformance fault injection.
    pub async fn drive(
        &self,
        request: &CommittedEffectRequest<ReferenceRequest>,
    ) -> Result<VerifiedEnsureResult> {
        match self
            .drive_conformance(request, ReferenceTargetBehavior::Available, None)
            .await?
        {
            ReferenceDriveOutcome::Returned(value) => Ok(*value),
            ReferenceDriveOutcome::Crashed(_) => Err(ExecutorError::ReferenceCrashInjected),
        }
    }

    /// Drives one committed request with reviewed target and crash injection.
    pub async fn drive_conformance(
        &self,
        request: &CommittedEffectRequest<ReferenceRequest>,
        behavior: ReferenceTargetBehavior,
        crash_point: Option<ReferenceCrashPoint>,
    ) -> Result<ReferenceDriveOutcome> {
        let identity = request.identity();
        let view = self.ledger.bind_effect(identity).await?;
        if view.terminal_tombstone().is_some() {
            return Ok(ReferenceDriveOutcome::Returned(Box::new(
                self.terminal_return(view)?,
            )));
        }

        let authority = match self
            .ledger
            .authorize_target(identity, self.contract.enqueue_operation().clone(), None)
            .await
        {
            Ok(authority) => authority,
            Err(ExecutorError::EvidenceBoundsExhausted) => {
                let view = self
                    .ledger
                    .effect_view(identity)
                    .await?
                    .ok_or(ExecutorError::EffectNotBound)?;
                return Ok(ReferenceDriveOutcome::Returned(Box::new(pending_return(
                    view,
                    self.ledger.exact_binding(),
                )?)));
            }
            Err(ExecutorError::EffectAlreadyTerminal) => {
                let view = self
                    .ledger
                    .effect_view(identity)
                    .await?
                    .ok_or(ExecutorError::EffectNotBound)?;
                return Ok(ReferenceDriveOutcome::Returned(Box::new(
                    self.terminal_return(view)?,
                )));
            }
            Err(error) => return Err(error),
        };
        if crash_point == Some(ReferenceCrashPoint::BeforeTargetEntry) {
            return Ok(ReferenceDriveOutcome::Crashed(
                ReferenceCrashPoint::BeforeTargetEntry,
            ));
        }

        let destination_return = self
            .destination
            .enqueue(authority, request.request(), &self.contract, behavior)
            .await?;
        if crash_point == Some(ReferenceCrashPoint::AfterTargetMutationBeforeObservation) {
            return Ok(ReferenceDriveOutcome::Crashed(
                ReferenceCrashPoint::AfterTargetMutationBeforeObservation,
            ));
        }

        let receipt = destination_return.into_target_receipt();
        let attempt_id = receipt.attempt_id().clone();
        let returned = receipt.outcome().returned_outcome().cloned();
        let view = self.ledger.observe_target(receipt).await?;
        let Some(returned) = returned else {
            return Ok(ReferenceDriveOutcome::Returned(Box::new(pending_return(
                view,
                self.ledger.exact_binding(),
            )?)));
        };
        if crash_point == Some(ReferenceCrashPoint::AfterObservationBeforeTombstone) {
            return Ok(ReferenceDriveOutcome::Crashed(
                ReferenceCrashPoint::AfterObservationBeforeTombstone,
            ));
        }

        if view.terminal_tombstone().is_some() {
            return Ok(ReferenceDriveOutcome::Returned(Box::new(
                self.terminal_return(view)?,
            )));
        }
        let observation_ref =
            returned_observation_ref(view.delivery_audit(), &attempt_id, &returned)?;
        let proof = ReferenceTerminalProof::new(attempt_id, returned, observation_ref)?;
        let tombstone = TerminalTombstone::new(
            request.request().external_operation_identity().to_owned(),
            self.contract.terminal_outcome().to_owned(),
            proof,
        )?;
        let view = match self
            .ledger
            .append_terminal_tombstone(identity, tombstone)
            .await
        {
            Ok(view) => view,
            Err(ExecutorError::TerminalTombstoneConflict) => self
                .ledger
                .effect_view(identity)
                .await?
                .ok_or(ExecutorError::EffectNotBound)?,
            Err(error) => return Err(error),
        };
        if crash_point == Some(ReferenceCrashPoint::AfterTombstoneBeforeReturn) {
            return Ok(ReferenceDriveOutcome::Crashed(
                ReferenceCrashPoint::AfterTombstoneBeforeReturn,
            ));
        }
        Ok(ReferenceDriveOutcome::Returned(Box::new(
            self.terminal_return(view)?,
        )))
    }

    fn terminal_return(&self, view: EffectEntryView) -> Result<VerifiedEnsureResult> {
        let (tombstone_ref, tombstone) = view
            .terminal_tombstone()
            .cloned()
            .ok_or(ExecutorError::TerminalEvidenceMissing)?;
        let proof = tombstone.terminal_proof().clone();
        let audit = view.delivery_audit().clone();
        let claim = ExecutorTerminalEvidenceClaim::new(
            view.identity().clone(),
            audit.head_ref()?,
            tombstone_ref,
            tombstone.external_operation_identity(),
            tombstone.terminal_outcome(),
            self.contract.assurance_policy_ref().clone(),
            ProofBasis::ExecutorAttestation {
                evidence_authority_ref: self
                    .ledger
                    .exact_binding()
                    .deployment()
                    .evidence_authority_ref()
                    .clone(),
            },
            proof.returned_outcome().safe_result_ref().clone(),
        )?;
        let retained_closure =
            ExecutorRetainedClosureClaim::from_delivery_audit(&audit, self.ledger.exact_binding())?;
        verify_ensure_result(
            view.identity().clone(),
            self.ledger.exact_binding(),
            ExecutorEnsureResultClaim::terminal(claim),
            retained_closure,
        )
    }
}

fn pending_return(
    view: EffectEntryView,
    binding: &crate::VerifiedExecutorBinding,
) -> Result<VerifiedEnsureResult> {
    let audit = view.delivery_audit().clone();
    let delivery_audit_ref = audit.head_ref()?;
    let retained_closure = ExecutorRetainedClosureClaim::from_delivery_audit(&audit, binding)?;
    verify_ensure_result(
        view.identity().clone(),
        binding,
        ExecutorEnsureResultClaim::pending(delivery_audit_ref),
        retained_closure,
    )
}

fn returned_observation_ref(
    audit: &DeliveryAudit,
    attempt_id: &AttemptId,
    returned: &ReturnedOutcome,
) -> Result<ContentRef> {
    audit
        .attempts()?
        .into_iter()
        .find(|attempt| {
            attempt.attempt_id() == attempt_id
                && matches!(
                    attempt.outcome(),
                    Some(DeliveryAttemptOutcome::Returned(candidate)) if candidate == returned
                )
        })
        .and_then(|attempt| attempt.returned_observation_ref().cloned())
        .ok_or(ExecutorError::TerminalProofMismatch)
}

fn enqueued_result(
    destination_key: &str,
    queue_position: u64,
) -> Result<ValidatedCanonicalValueV1> {
    encode(
        REFERENCE_RESULT_SCHEMA,
        &canonical_object([
            (
                "destination_key",
                CanonicalValue::String(destination_key.to_owned()),
            ),
            ("kind", CanonicalValue::String("enqueued".to_owned())),
            (
                "queue_position",
                CanonicalValue::String(queue_position.to_string()),
            ),
        ])?,
    )
}

fn already_enqueued_result(destination_key: &str) -> Result<ValidatedCanonicalValueV1> {
    encode(
        REFERENCE_RESULT_SCHEMA,
        &canonical_object([
            (
                "destination_key",
                CanonicalValue::String(destination_key.to_owned()),
            ),
            (
                "kind",
                CanonicalValue::String("already_enqueued".to_owned()),
            ),
        ])?,
    )
}

fn request_conflict_result(
    failure: &crate::ReferenceSafeFailure,
) -> Result<ValidatedCanonicalValueV1> {
    let outcome = DeliveryAttemptOutcome::indeterminate(failure.clone())?;
    let DeliveryAttemptOutcome::Indeterminate(failure) = outcome else {
        return Err(ExecutorError::InvalidSafeFailure);
    };
    let safe_failure = crate::frontier::validated_safe_failure_for_result(&failure)?;
    encode(
        REFERENCE_RESULT_SCHEMA,
        &canonical_object([
            (
                "kind",
                CanonicalValue::String("request_conflict".to_owned()),
            ),
            (
                "safe_failure",
                safe_failure
                    .canonical_value()
                    .map_err(crate::contract::contract_error)?,
            ),
        ])?,
    )
}
