use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use mfm_canonical::{CanonicalValue, ValidatedCanonicalValue};
use mfm_ids::{AttemptId, ContentRef, EffectKey, TenantScopeId};

use crate::codec::{Decoder, Encoder};
use crate::contract::{
    canonical_object, content_ref, content_ref_value, encode, AllocationStateRef, EffectIdentity,
    ExecutorBindingRef, FencingRef, ResourceKeyRef, ResourceOwnershipRef,
    SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
use crate::engine::{
    ExecutorAppendOutcome, ExecutorEffectSnapshot, ExecutorLedgerAppend, ExecutorLedgerStore,
    ExecutorLedgerStoreIdentity, ExecutorResourceSnapshot, ExecutorStoreSnapshot,
    KeyedExecutorLedger,
};
use crate::frontier::{
    decode_evidence_record, encode_evidence_record, DeliveryAudit, DeliveryAuditFrontier,
    EvidenceBounds, ExecutorEvidenceRecord, TerminalTombstone, TerminalTombstoneRef,
};
use crate::policy::ResourcePolicyBinding;
use crate::{ExecutorError, Result};

const RESOURCE_LEDGER_RECORD_SCHEMA: &str = "mfm.executor-resource-ledger-record.v1";
const RESOURCE_ALLOCATED_SCHEMA: &str = "mfm.executor-resource-allocated.v1";
const LEDGER_CHECKPOINT_MAGIC: &[u8; 8] = b"MFMELG05";
const RESOURCE_RECORD_DURABLE_MAGIC: &[u8; 8] = b"MFMERR01";
const MAX_SNAPSHOT_ITEMS: usize = 1_000_000;

/// Reviewed projection of one durable typed resource allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceAllocationEvidence {
    resource_key_ref: ResourceKeyRef,
    resource_key_value: ValidatedCanonicalValue,
    typed_allocation_state_ref: AllocationStateRef,
    typed_allocation_state: ValidatedCanonicalValue,
    policy_binding: ResourcePolicyBinding,
    fencing_ref: Option<FencingRef>,
}

impl ResourceAllocationEvidence {
    /// Returns the exact resource stream key.
    pub const fn resource_key_ref(&self) -> &ResourceKeyRef {
        &self.resource_key_ref
    }

    /// Returns the exact annex-validated resource-key object.
    pub const fn resource_key_value(&self) -> &ValidatedCanonicalValue {
        &self.resource_key_value
    }

    /// Returns the typed allocation-state commitment.
    pub const fn typed_allocation_state_ref(&self) -> &AllocationStateRef {
        &self.typed_allocation_state_ref
    }

    /// Returns the exact annex-validated allocation state.
    pub const fn typed_allocation_state(&self) -> &ValidatedCanonicalValue {
        &self.typed_allocation_state
    }

    /// Returns the immutable policy/configuration pair.
    pub const fn policy_binding(&self) -> &ResourcePolicyBinding {
        &self.policy_binding
    }

    /// Returns optional destination fencing evidence.
    pub const fn fencing_ref(&self) -> Option<&FencingRef> {
        self.fencing_ref.as_ref()
    }
}

/// Outcome of an idempotent typed resource allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationOutcome<Allocation> {
    /// A new resource record and effect link committed atomically.
    Allocated {
        /// Typed policy result.
        allocation: Allocation,
        /// Reviewed durable allocation evidence.
        evidence: ResourceAllocationEvidence,
        /// New immutable resource stream head.
        resource_head: ResourceLedgerRecordRef,
    },
    /// The exact effect was already bound to this allocation.
    Existing {
        /// Typed policy result reconstructed from retained state.
        allocation: Allocation,
        /// Reviewed durable allocation evidence.
        evidence: ResourceAllocationEvidence,
        /// Current immutable resource stream head.
        resource_head: ResourceLedgerRecordRef,
    },
}

/// One immutable append in a typed resource stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLedgerRecord {
    pub(crate) resource_ownership_ref: ResourceOwnershipRef,
    pub(crate) resource_key_ref: ResourceKeyRef,
    pub(crate) resource_key_value: ValidatedCanonicalValue,
    pub(crate) predecessor: Option<ResourceLedgerRecordRef>,
    pub(crate) effect_key: EffectKey,
    pub(crate) typed_allocation_state_ref: AllocationStateRef,
    pub(crate) typed_allocation_state: ValidatedCanonicalValue,
    pub(crate) policy_binding: ResourcePolicyBinding,
    pub(crate) fencing_ref: Option<FencingRef>,
}

impl ResourceLedgerRecord {
    /// Returns the resource owner.
    pub const fn resource_ownership_ref(&self) -> &ResourceOwnershipRef {
        &self.resource_ownership_ref
    }

    /// Returns the typed resource key.
    pub const fn resource_key_ref(&self) -> &ResourceKeyRef {
        &self.resource_key_ref
    }

    /// Returns the exact annex-validated resource-key object.
    pub const fn resource_key_value(&self) -> &ValidatedCanonicalValue {
        &self.resource_key_value
    }

    /// Returns the immediate immutable predecessor.
    pub const fn predecessor(&self) -> Option<&ResourceLedgerRecordRef> {
        self.predecessor.as_ref()
    }

    /// Returns the effect receiving this permanent allocation.
    pub const fn effect_key(&self) -> &EffectKey {
        &self.effect_key
    }

    /// Returns the typed allocation-state commitment.
    pub const fn typed_allocation_state_ref(&self) -> &AllocationStateRef {
        &self.typed_allocation_state_ref
    }

    /// Returns the exact annex-validated allocation state.
    pub const fn typed_allocation_state(&self) -> &ValidatedCanonicalValue {
        &self.typed_allocation_state
    }

    /// Returns the immutable policy/configuration pair inherited by the stream.
    pub const fn policy_binding(&self) -> &ResourcePolicyBinding {
        &self.policy_binding
    }

    /// Returns optional destination fencing evidence.
    pub const fn fencing_ref(&self) -> Option<&FencingRef> {
        self.fencing_ref.as_ref()
    }

    /// Returns the exact annex-validated resource-ledger record.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        let value = if self.predecessor.is_none() {
            let mut allocated = vec![
                (
                    "policy_configuration_ref".to_owned(),
                    content_ref_value(self.policy_binding.policy_configuration_ref())?,
                ),
                (
                    "policy_ref".to_owned(),
                    content_ref_value(self.policy_binding.policy_ref())?,
                ),
                (
                    "resource_key_ref".to_owned(),
                    self.resource_key_ref.canonical_value()?,
                ),
                (
                    "resource_ownership_ref".to_owned(),
                    self.resource_ownership_ref.canonical_value()?,
                ),
                (
                    "typed_allocation_state_ref".to_owned(),
                    self.typed_allocation_state_ref.canonical_value()?,
                ),
            ];
            if let Some(fencing_ref) = &self.fencing_ref {
                allocated.push(("fencing_ref".to_owned(), fencing_ref.canonical_value()?));
            }
            let allocated = encode(
                RESOURCE_ALLOCATED_SCHEMA,
                &CanonicalValue::object(allocated).map_err(|_| ExecutorError::CanonicalEncoding)?,
            )?;
            canonical_object([
                (
                    "kind",
                    CanonicalValue::String("resource_allocated".to_owned()),
                ),
                (
                    "record",
                    allocated
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
            ])?
        } else {
            let mut advanced = vec![
                (
                    "kind".to_owned(),
                    CanonicalValue::String("resource_advanced".to_owned()),
                ),
                (
                    "resource_key_ref".to_owned(),
                    self.resource_key_ref.canonical_value()?,
                ),
                (
                    "typed_allocation_state_ref".to_owned(),
                    self.typed_allocation_state_ref.canonical_value()?,
                ),
            ];
            if let Some(fencing_ref) = &self.fencing_ref {
                advanced.push(("fencing_ref".to_owned(), fencing_ref.canonical_value()?));
            }
            CanonicalValue::object(advanced).map_err(|_| ExecutorError::CanonicalEncoding)?
        };
        encode(RESOURCE_LEDGER_RECORD_SCHEMA, &value)
    }

    /// Computes this immutable record's content address.
    pub fn reference(&self) -> Result<ResourceLedgerRecordRef> {
        Ok(ResourceLedgerRecordRef(content_ref(&self.validated()?)?))
    }

    /// Encodes this exact resource record into a bounded checksummed payload.
    pub fn to_durable_bytes(&self) -> Result<Vec<u8>> {
        let mut encoder = Encoder::new(RESOURCE_RECORD_DURABLE_MAGIC);
        encoder.content_ref(self.resource_ownership_ref.as_content_ref())?;
        encoder.content_ref(self.resource_key_ref.as_content_ref())?;
        encoder.validated(&self.resource_key_value)?;
        crate::frontier::encode_optional_content_ref(
            &mut encoder,
            self.predecessor
                .as_ref()
                .map(ResourceLedgerRecordRef::as_content_ref),
        )?;
        encoder.string(self.effect_key.as_str())?;
        encoder.validated(&self.typed_allocation_state)?;
        encoder.content_ref(self.policy_binding.policy_ref())?;
        encoder.content_ref(self.policy_binding.policy_configuration_ref())?;
        crate::frontier::encode_optional_content_ref(
            &mut encoder,
            self.fencing_ref.as_ref().map(FencingRef::as_content_ref),
        )?;
        encoder.finish()
    }

    /// Strictly reconstructs one bounded checksummed resource-record payload.
    pub fn from_durable_bytes(bytes: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, RESOURCE_RECORD_DURABLE_MAGIC)?;
        let resource_ownership_ref =
            ResourceOwnershipRef::from_content_ref(decoder.content_ref()?)?;
        let resource_key_ref = ResourceKeyRef::from_reviewed(decoder.content_ref()?);
        let resource_key_value = decoder.validated()?;
        if ResourceKeyRef::from_reviewed(content_ref(&resource_key_value)?) != resource_key_ref {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let predecessor = crate::frontier::decode_optional_content_ref(&mut decoder)?
            .map(ResourceLedgerRecordRef);
        let effect_key = decoder.effect_key()?;
        let typed_allocation_state = decoder.validated()?;
        let typed_allocation_state_ref =
            AllocationStateRef::from_reviewed(content_ref(&typed_allocation_state)?);
        let policy_binding =
            ResourcePolicyBinding::new(decoder.content_ref()?, decoder.content_ref()?);
        let fencing_ref = crate::frontier::decode_optional_content_ref(&mut decoder)?
            .map(FencingRef::from_reviewed);
        decoder.finish()?;
        let record = Self {
            resource_ownership_ref,
            resource_key_ref,
            resource_key_value,
            predecessor,
            effect_key,
            typed_allocation_state_ref,
            typed_allocation_state,
            policy_binding,
            fencing_ref,
        };
        record.validated()?;
        Ok(record)
    }

    /// Returns the exact executor-owned content closure introduced by this record.
    pub fn content_objects(&self) -> Result<Vec<crate::SchemaQualifiedCanonicalValue>> {
        Ok(vec![
            crate::SchemaQualifiedCanonicalValue::from_validated(&self.validated()?)?,
            crate::SchemaQualifiedCanonicalValue::from_validated(&self.resource_key_value)?,
            crate::SchemaQualifiedCanonicalValue::from_validated(&self.typed_allocation_state)?,
        ])
    }

    pub(crate) fn evidence(&self) -> ResourceAllocationEvidence {
        ResourceAllocationEvidence {
            resource_key_ref: self.resource_key_ref.clone(),
            resource_key_value: self.resource_key_value.clone(),
            typed_allocation_state_ref: self.typed_allocation_state_ref.clone(),
            typed_allocation_state: self.typed_allocation_state.clone(),
            policy_binding: self.policy_binding.clone(),
            fencing_ref: self.fencing_ref.clone(),
        }
    }
}

/// Content identity of one exact resource-ledger record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceLedgerRecordRef(ContentRef);

impl ResourceLedgerRecordRef {
    /// Returns the lightweight resource-record identity.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
    }
}

/// Immutable resource history supplied to one typed policy decision.
#[derive(Debug, Clone, Copy)]
pub struct ResourceStreamView<'a> {
    records: &'a [ResourceLedgerRecord],
    terminal_effects: &'a std::collections::BTreeSet<EffectKey>,
}

impl<'a> ResourceStreamView<'a> {
    pub(crate) const fn from_verified_parts(
        records: &'a [ResourceLedgerRecord],
        terminal_effects: &'a std::collections::BTreeSet<EffectKey>,
    ) -> Self {
        Self {
            records,
            terminal_effects,
        }
    }

    /// Returns the complete predecessor-linked allocation history.
    pub const fn records(&self) -> &'a [ResourceLedgerRecord] {
        self.records
    }

    /// Returns the current immutable stream head.
    pub fn head(&self) -> Result<Option<ResourceLedgerRecordRef>> {
        self.records
            .last()
            .map(ResourceLedgerRecord::reference)
            .transpose()
    }

    /// Returns whether the linked effect has immutable executor terminal evidence.
    pub fn effect_is_terminal(&self, effect_key: &EffectKey) -> bool {
        self.terminal_effects.contains(effect_key)
    }
}

/// Folded immutable view of one effect entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryView {
    identity: EffectIdentity,
    allocation: Option<(ResourceOwnershipRef, ResourceAllocationEvidence)>,
    delivery_audit: DeliveryAudit,
    terminal_tombstone: Option<(TerminalTombstoneRef, TerminalTombstone)>,
}

impl EffectEntryView {
    pub(crate) fn from_verified_parts(
        identity: EffectIdentity,
        allocation: Option<(ResourceOwnershipRef, ResourceAllocationEvidence)>,
        delivery_audit: DeliveryAudit,
        terminal_tombstone: Option<(TerminalTombstoneRef, TerminalTombstone)>,
    ) -> Self {
        Self {
            identity,
            allocation,
            delivery_audit,
            terminal_tombstone,
        }
    }

    /// Returns the immutable binding/key/request identity.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.identity
    }

    /// Returns the cross-effect allocation, when present.
    pub const fn allocation(&self) -> Option<&(ResourceOwnershipRef, ResourceAllocationEvidence)> {
        self.allocation.as_ref()
    }

    /// Returns the complete bounded delivery audit.
    pub const fn delivery_audit(&self) -> &DeliveryAudit {
        &self.delivery_audit
    }

    /// Returns the immutable terminal tombstone, when present.
    pub const fn terminal_tombstone(&self) -> Option<&(TerminalTombstoneRef, TerminalTombstone)> {
        self.terminal_tombstone.as_ref()
    }
}

/// Affine authority for zero or one target-boundary entry.
///
/// The value is intentionally not `Clone`. Reloading a committed
/// authorization cannot reconstruct this authority. It carries only the four
/// public target-entry fields; completion binding remains private to
/// [`KeyedExecutorLedger::execute_target_once`].
///
/// The authority has no public completion operation:
///
/// ```compile_fail
/// # use mfm_executor::{DeliveryAttemptOutcome, TargetEntryAuthority};
/// # fn cannot_complete(authority: TargetEntryAuthority, outcome: DeliveryAttemptOutcome) {
/// let _ = authority.complete(outcome);
/// # }
/// ```
///
/// No public target receipt exists:
///
/// ```compile_fail
/// use mfm_executor::TargetOperationReceipt;
/// ```
pub struct TargetEntryAuthority {
    identity: EffectIdentity,
    attempt_id: AttemptId,
    target_operation_ref: ContentRef,
    durable_ledger_generation_ref: ContentRef,
}

impl TargetEntryAuthority {
    pub(crate) fn new(
        identity: EffectIdentity,
        attempt_id: AttemptId,
        target_operation_ref: ContentRef,
        durable_ledger_generation_ref: ContentRef,
    ) -> Self {
        Self {
            identity,
            attempt_id,
            target_operation_ref,
            durable_ledger_generation_ref,
        }
    }

    /// Returns the exact immutable effect identity.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.identity
    }

    /// Returns the unique authorized attempt.
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Returns the sole admitted target operation.
    pub const fn target_operation_ref(&self) -> &ContentRef {
        &self.target_operation_ref
    }

    /// Returns the durable generation the destination must accept.
    pub const fn durable_ledger_generation_ref(&self) -> &ContentRef {
        &self.durable_ledger_generation_ref
    }
}

impl std::fmt::Debug for TargetEntryAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TargetEntryAuthority")
            .field("effect_key", self.identity.effect_key())
            .field("attempt_id", &self.attempt_id)
            .field("target_operation_ref", &self.target_operation_ref)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct EffectStream {
    identity: EffectIdentity,
    frontiers: Vec<DeliveryAuditFrontier>,
    allocation: Option<ResourceLedgerRecord>,
}

#[derive(Debug, Clone, Default)]
struct LedgerState {
    effects: BTreeMap<EffectKey, EffectStream>,
    resources: BTreeMap<(ResourceOwnershipRef, ResourceKeyRef), Vec<ResourceLedgerRecord>>,
    content: BTreeMap<ContentRef, SchemaQualifiedCanonicalValue>,
}

/// Opaque deep checkpoint for exact-generation conformance restart.
#[derive(Debug, Clone)]
pub struct MemoryLedgerCheckpoint {
    binding_ref: ExecutorBindingRef,
    tenant_scope_id: TenantScopeId,
    durable_ledger_generation_ref: ContentRef,
    proof_ref: ContentRef,
    bounds: EvidenceBounds,
    state: LedgerState,
}

impl MemoryLedgerCheckpoint {
    /// Encodes the complete authority into a bounded checksummed snapshot.
    ///
    /// This operational checkpoint is not a production non-rollback fence.
    pub fn to_durable_bytes(&self) -> Result<Vec<u8>> {
        let mut encoder = Encoder::new(LEDGER_CHECKPOINT_MAGIC);
        encoder.content_ref(self.binding_ref.as_content_ref())?;
        encoder.string(self.tenant_scope_id.as_str())?;
        encoder.content_ref(&self.durable_ledger_generation_ref)?;
        encoder.content_ref(&self.proof_ref)?;
        encode_bounds(&mut encoder, &self.bounds)?;

        encoder.usize(self.state.effects.len())?;
        for stream in self.state.effects.values() {
            encode_effect_identity(&mut encoder, &stream.identity)?;
            encoder.usize(stream.frontiers.len())?;
            for frontier in &stream.frontiers {
                encoder.usize(frontier.appended_records().len())?;
                for record in frontier.appended_records() {
                    encode_evidence_record(&mut encoder, record)?;
                }
            }
        }

        encoder.usize(self.state.resources.len())?;
        for ((ownership_ref, resource_key_ref), records) in &self.state.resources {
            encoder.content_ref(ownership_ref.as_content_ref())?;
            encoder.content_ref(resource_key_ref.as_content_ref())?;
            let resource_key_value = records
                .first()
                .ok_or(ExecutorError::InvalidDurableSnapshot)?
                .resource_key_value();
            encoder.validated(resource_key_value)?;
            encoder.usize(records.len())?;
            for record in records {
                encoder.string(record.effect_key.as_str())?;
                encoder.validated(&record.typed_allocation_state)?;
                encoder.content_ref(record.policy_binding.policy_ref())?;
                encoder.content_ref(record.policy_binding.policy_configuration_ref())?;
                crate::frontier::encode_optional_content_ref(
                    &mut encoder,
                    record.fencing_ref.as_ref().map(FencingRef::as_content_ref),
                )?;
            }
        }
        encoder.usize(self.state.content.len())?;
        for value in self.state.content.values() {
            encoder.schema_qualified(value)?;
        }
        encoder.finish()
    }

    /// Decodes and structurally verifies one bounded durable checkpoint.
    pub fn from_durable_bytes(bytes: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, LEDGER_CHECKPOINT_MAGIC)?;
        let binding_ref = ExecutorBindingRef::from_content_ref(decoder.content_ref()?)?;
        let tenant_scope_id = decoder.tenant_scope_id()?;
        let durable_ledger_generation_ref = decoder.content_ref()?;
        let proof_ref = decoder.content_ref()?;
        let bounds = decode_bounds(&mut decoder)?;

        let effect_count = decoder.count(MAX_SNAPSHOT_ITEMS, minimum_effect_bytes())?;
        let mut effects = BTreeMap::new();
        for _ in 0..effect_count {
            let identity = decode_effect_identity(&mut decoder)?;
            if identity.executor_binding_ref() != &binding_ref
                || identity.tenant_scope_id() != &tenant_scope_id
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let frontier_count = decoder.count(bounds.max_records() as usize, 9)?;
            let mut stream = EffectStream {
                identity: identity.clone(),
                frontiers: Vec::new(),
                allocation: None,
            };
            for _ in 0..frontier_count {
                let record_count = decoder.count(256, 1)?;
                if record_count == 0 {
                    return Err(ExecutorError::InvalidDurableSnapshot);
                }
                let mut records = Vec::new();
                for _ in 0..record_count {
                    records.push(decode_evidence_record(&mut decoder)?);
                }
                append_effect_records(&mut stream, records, &bounds, &proof_ref)?;
            }
            if effects
                .insert(identity.effect_key().clone(), stream)
                .is_some()
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }

        let resource_stream_count = decoder.count(MAX_SNAPSHOT_ITEMS, 9)?;
        let mut resources = BTreeMap::new();
        for _ in 0..resource_stream_count {
            let ownership_ref = ResourceOwnershipRef::from_content_ref(decoder.content_ref()?)?;
            let resource_key_ref = ResourceKeyRef::from_reviewed(decoder.content_ref()?);
            let resource_key_value = decoder.validated()?;
            if ResourceKeyRef::from_reviewed(content_ref(&resource_key_value)?) != resource_key_ref
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let record_count = decoder.count(MAX_SNAPSHOT_ITEMS, 9)?;
            if record_count == 0 {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let mut records = Vec::new();
            let mut predecessor = None;
            for _ in 0..record_count {
                let effect_key = decoder.effect_key()?;
                let typed_allocation_state = decoder.validated()?;
                let typed_allocation_state_ref =
                    AllocationStateRef::from_reviewed(content_ref(&typed_allocation_state)?);
                let policy_binding =
                    ResourcePolicyBinding::new(decoder.content_ref()?, decoder.content_ref()?);
                let fencing_ref = crate::frontier::decode_optional_content_ref(&mut decoder)?
                    .map(FencingRef::from_reviewed);
                let record = ResourceLedgerRecord {
                    resource_ownership_ref: ownership_ref.clone(),
                    resource_key_ref: resource_key_ref.clone(),
                    resource_key_value: resource_key_value.clone(),
                    predecessor,
                    effect_key,
                    typed_allocation_state_ref,
                    typed_allocation_state,
                    policy_binding,
                    fencing_ref,
                };
                predecessor = Some(record.reference()?);
                records.push(record);
            }
            if resources
                .insert((ownership_ref, resource_key_ref), records)
                .is_some()
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        let content_count = decoder.count(MAX_SNAPSHOT_ITEMS, 9)?;
        let mut content = BTreeMap::new();
        for _ in 0..content_count {
            let value = decoder.schema_qualified()?;
            let reference = value.reference()?;
            if content.insert(reference, value).is_some() {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        decoder.finish()?;

        attach_allocations(&mut effects, &resources)?;
        let checkpoint = Self {
            binding_ref,
            tenant_scope_id,
            durable_ledger_generation_ref,
            proof_ref,
            bounds,
            state: LedgerState {
                effects,
                resources,
                content,
            },
        };
        validate_checkpoint_state(&checkpoint)?;
        Ok(checkpoint)
    }
}

/// Thread-safe append/CAS conformance backend.
///
/// Clones share one live authority. A checkpoint models exact-generation
/// restart only and does not qualify production rollback or sibling fencing.
#[derive(Debug, Clone)]
pub struct MemoryExecutorStore {
    identity: ExecutorLedgerStoreIdentity,
    bounds: EvidenceBounds,
    state: Arc<Mutex<LedgerState>>,
}

impl MemoryExecutorStore {
    /// Creates an empty raw store for one exact executor binding.
    pub fn new(binding: &VerifiedExecutorBinding) -> Self {
        let bounds = binding.contract().evidence_bounds().clone();
        Self {
            identity: ExecutorLedgerStoreIdentity::from_binding(binding),
            bounds,
            state: Arc::new(Mutex::new(LedgerState::default())),
        }
    }

    /// Restores an independent raw store from an exact-generation checkpoint.
    pub fn restore(
        binding: &VerifiedExecutorBinding,
        checkpoint: MemoryLedgerCheckpoint,
    ) -> Result<Self> {
        require_checkpoint_binding(binding, &checkpoint)?;
        let expected_bounds = binding.contract().evidence_bounds().clone();
        if checkpoint.bounds != expected_bounds {
            return Err(ExecutorError::EvidenceBoundsMismatch);
        }
        validate_binding_state(binding, &checkpoint.state, &checkpoint.bounds)?;
        let store = Self {
            identity: ExecutorLedgerStoreIdentity::from_binding(binding),
            bounds: expected_bounds,
            state: Arc::new(Mutex::new(checkpoint.state)),
        };
        let ledger = KeyedExecutorLedger::new(store.clone(), binding.clone())?;
        ledger.strict_refold(&store.full_snapshot()?)?;
        Ok(store)
    }

    /// Captures a deep immutable checkpoint of all append-only authority.
    pub fn checkpoint(&self) -> Result<MemoryLedgerCheckpoint> {
        let state = self.lock_state()?.clone();
        let checkpoint = MemoryLedgerCheckpoint {
            binding_ref: self.identity.binding_ref().clone(),
            tenant_scope_id: self.identity.tenant_scope_id().clone(),
            durable_ledger_generation_ref: self.identity.durable_ledger_generation_ref().clone(),
            proof_ref: self.identity.evidence_authority_ref().clone(),
            bounds: self.bounds.clone(),
            state,
        };
        validate_checkpoint_state(&checkpoint)?;
        Ok(checkpoint)
    }

    /// Enumerates a complete snapshot for strict shared-engine refolding.
    pub fn full_snapshot(&self) -> Result<ExecutorStoreSnapshot> {
        let state = self.lock_state()?;
        let effects = state
            .effects
            .values()
            .map(effect_snapshot)
            .collect::<Vec<_>>();
        let resources = state
            .resources
            .iter()
            .map(|((ownership, key), records)| {
                ExecutorResourceSnapshot::from_store_parts(
                    ownership.clone(),
                    key.clone(),
                    records.clone(),
                )
            })
            .collect::<Vec<_>>();
        let content = state.content.values().cloned().collect();
        Ok(ExecutorStoreSnapshot::from_store_parts(
            self.identity.clone(),
            effects,
            resources,
            content,
        ))
    }

    /// Loads one raw effect snapshot without applying high-level semantics.
    pub fn load_effect_snapshot(
        &self,
        effect_key: &EffectKey,
    ) -> Result<Option<ExecutorEffectSnapshot>> {
        let state = self.lock_state()?;
        Ok(state.effects.get(effect_key).map(effect_snapshot))
    }

    /// Loads one raw resource snapshot without applying policy semantics.
    pub fn load_resource_snapshot(
        &self,
        resource_ownership_ref: &ResourceOwnershipRef,
        resource_key_ref: &ResourceKeyRef,
    ) -> Result<ExecutorResourceSnapshot> {
        let state = self.lock_state()?;
        Ok(ExecutorResourceSnapshot::from_store_parts(
            resource_ownership_ref.clone(),
            resource_key_ref.clone(),
            state
                .resources
                .get(&(resource_ownership_ref.clone(), resource_key_ref.clone()))
                .cloned()
                .unwrap_or_default(),
        ))
    }

    /// Loads one exact immutable executor-owned content object.
    pub fn load_content_snapshot(
        &self,
        content_ref: &ContentRef,
    ) -> Result<Option<SchemaQualifiedCanonicalValue>> {
        let state = self.lock_state()?;
        Ok(state.content.get(content_ref).cloned())
    }

    /// Applies one raw atomic compare-and-append proposal.
    pub fn compare_and_append_snapshot(
        &self,
        append: ExecutorLedgerAppend,
    ) -> Result<ExecutorAppendOutcome> {
        self.apply_append(append)
    }

    fn require_identity(&self, identity: &EffectIdentity) -> Result<()> {
        if identity.executor_binding_ref() != self.identity.binding_ref() {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        if identity.tenant_scope_id() != self.identity.tenant_scope_id() {
            return Err(ExecutorError::TenantScopeMismatch);
        }
        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, LedgerState>> {
        self.state
            .lock()
            .map_err(|_| ExecutorError::SynchronizationFailure)
    }

    fn apply_append(&self, append: ExecutorLedgerAppend) -> Result<ExecutorAppendOutcome> {
        self.require_identity(append.identity())?;
        append.validate_for_store_identity(&self.identity)?;
        let mut guard = self.lock_state()?;
        let actual_effect_head = guard
            .effects
            .get(append.identity().effect_key())
            .and_then(|stream| stream.frontiers.last())
            .map(DeliveryAuditFrontier::reference)
            .transpose()?;
        let resource_key = append.resource().map(|resource| {
            (
                resource.record().resource_ownership_ref().clone(),
                resource.record().resource_key_ref().clone(),
            )
        });
        let actual_resource_head = resource_key
            .as_ref()
            .and_then(|key| guard.resources.get(key))
            .and_then(|records| records.last())
            .map(ResourceLedgerRecord::reference)
            .transpose()?;
        let resource_expected = append
            .resource()
            .map(|resource| resource.expected_head().cloned());
        let heads_match = actual_effect_head.as_ref() == append.expected_effect_head()
            && match resource_expected {
                None => true,
                Some(expected) => actual_resource_head == expected,
            };
        if !heads_match {
            return if append_is_present(&guard, &append)? {
                Ok(ExecutorAppendOutcome::AlreadyApplied)
            } else {
                Ok(ExecutorAppendOutcome::Conflict)
            };
        }

        let mut staged = guard.clone();
        let stream = staged
            .effects
            .entry(append.identity().effect_key().clone())
            .or_insert_with(|| EffectStream {
                identity: append.identity().clone(),
                frontiers: Vec::new(),
                allocation: None,
            });
        if stream.identity != *append.identity()
            || append.effect_frontier().predecessor_frontier_ref() != append.expected_effect_head()
        {
            return Ok(ExecutorAppendOutcome::Conflict);
        }
        stream.frontiers.push(append.effect_frontier().clone());
        if let Some(resource) = append.resource() {
            if resource.record().effect_key() != append.identity().effect_key()
                || resource.record().predecessor() != resource.expected_head()
                || stream.allocation.is_some()
            {
                return Ok(ExecutorAppendOutcome::Conflict);
            }
            staged
                .resources
                .entry((
                    resource.record().resource_ownership_ref().clone(),
                    resource.record().resource_key_ref().clone(),
                ))
                .or_default()
                .push(resource.record().clone());
            stream.allocation = Some(resource.record().clone());
        }
        for value in append.content_objects() {
            let reference = value.reference()?;
            match staged.content.get(&reference) {
                Some(existing) if existing != value => {
                    return Err(ExecutorError::RetainedObjectMismatch);
                }
                Some(_) => {}
                None => {
                    staged.content.insert(reference, value.clone());
                }
            }
        }
        let checkpoint = MemoryLedgerCheckpoint {
            binding_ref: self.identity.binding_ref().clone(),
            tenant_scope_id: self.identity.tenant_scope_id().clone(),
            durable_ledger_generation_ref: self.identity.durable_ledger_generation_ref().clone(),
            proof_ref: self.identity.evidence_authority_ref().clone(),
            bounds: self.bounds.clone(),
            state: staged.clone(),
        };
        validate_checkpoint_state(&checkpoint)?;
        *guard = staged;
        Ok(ExecutorAppendOutcome::Applied)
    }
}

impl ExecutorLedgerStore for MemoryExecutorStore {
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        &self.identity
    }

    fn load_effect<'a>(
        &'a self,
        effect_key: &'a EffectKey,
    ) -> crate::ExecutorFuture<'a, Result<Option<ExecutorEffectSnapshot>>> {
        Box::pin(async move { self.load_effect_snapshot(effect_key) })
    }

    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> crate::ExecutorFuture<'a, Result<ExecutorResourceSnapshot>> {
        Box::pin(
            async move { self.load_resource_snapshot(resource_ownership_ref, resource_key_ref) },
        )
    }

    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> crate::ExecutorFuture<'a, Result<Option<SchemaQualifiedCanonicalValue>>> {
        Box::pin(async move { self.load_content_snapshot(content_ref) })
    }

    fn compare_and_append<'a>(
        &'a self,
        append: ExecutorLedgerAppend,
    ) -> crate::ExecutorFuture<'a, Result<ExecutorAppendOutcome>> {
        Box::pin(async move { self.compare_and_append_snapshot(append) })
    }
}

fn append_effect_records(
    stream: &mut EffectStream,
    records: Vec<ExecutorEvidenceRecord>,
    bounds: &EvidenceBounds,
    proof_ref: &ContentRef,
) -> Result<()> {
    let predecessor = stream
        .frontiers
        .last()
        .map(DeliveryAuditFrontier::reference)
        .transpose()?;
    let frontier =
        DeliveryAuditFrontier::append(&stream.identity, predecessor, records, proof_ref.clone())?;
    stream.frontiers.push(frontier);
    DeliveryAudit::from_ledger(stream.frontiers.clone()).verify_structure(
        &stream.identity,
        bounds,
        proof_ref,
    )
}

fn attach_allocations(
    effects: &mut BTreeMap<EffectKey, EffectStream>,
    resources: &BTreeMap<(ResourceOwnershipRef, ResourceKeyRef), Vec<ResourceLedgerRecord>>,
) -> Result<()> {
    for records in resources.values() {
        for record in records {
            let stream = effects
                .get_mut(record.effect_key())
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            if stream.allocation.is_some() {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            stream.allocation = Some(record.clone());
        }
    }
    Ok(())
}

fn effect_snapshot(stream: &EffectStream) -> ExecutorEffectSnapshot {
    ExecutorEffectSnapshot::from_store_parts(
        stream.identity.clone(),
        stream.frontiers.clone(),
        stream.allocation.clone(),
    )
}

fn append_is_present(state: &LedgerState, append: &ExecutorLedgerAppend) -> Result<bool> {
    let frontier_ref = append.effect_frontier().reference()?;
    let effect_present = state
        .effects
        .get(append.identity().effect_key())
        .is_some_and(|stream| {
            stream
                .frontiers
                .iter()
                .any(|frontier| frontier.reference().as_ref() == Ok(&frontier_ref))
        });
    if !effect_present {
        return Ok(false);
    }
    if let Some(resource) = append.resource() {
        let record_ref = resource.record().reference()?;
        if !state
            .resources
            .get(&(
                resource.record().resource_ownership_ref().clone(),
                resource.record().resource_key_ref().clone(),
            ))
            .is_some_and(|records| {
                records
                    .iter()
                    .any(|record| record.reference().as_ref() == Ok(&record_ref))
            })
        {
            return Ok(false);
        }
    }
    append
        .content_objects()
        .iter()
        .try_fold(true, |present, value| {
            let reference = value.reference()?;
            Ok(present && state.content.get(&reference) == Some(value))
        })
}

fn encode_effect_identity(encoder: &mut Encoder, identity: &EffectIdentity) -> Result<()> {
    encoder.string(identity.tenant_scope_id().as_str())?;
    encoder.content_ref(identity.executor_binding_ref().as_content_ref())?;
    encoder.string(identity.effect_key().as_str())?;
    encoder.string(identity.request_digest().as_str())
}

fn decode_effect_identity(decoder: &mut Decoder<'_>) -> Result<EffectIdentity> {
    Ok(EffectIdentity::from_parts(
        decoder.tenant_scope_id()?,
        ExecutorBindingRef::from_content_ref(decoder.content_ref()?)?,
        decoder.effect_key()?,
        decoder.request_digest()?,
    ))
}

fn encode_bounds(encoder: &mut Encoder, bounds: &EvidenceBounds) -> Result<()> {
    encoder.u32(bounds.max_attempts());
    encoder.u32(bounds.max_records());
    encoder.usize(bounds.max_retained_bytes())?;
    encoder.usize(bounds.max_completion_record_bytes())?;
    encoder.u32(bounds.completion_reserve_records());
    encoder.usize(bounds.completion_reserve_bytes())
}

fn decode_bounds(decoder: &mut Decoder<'_>) -> Result<EvidenceBounds> {
    EvidenceBounds::new(
        decoder.u32()?,
        decoder.u32()?,
        decoder.u64()?,
        decoder.u64()?,
        decoder.u32()?,
        decoder.u64()?,
    )
}

const fn minimum_effect_bytes() -> usize {
    9
}

fn require_checkpoint_binding(
    binding: &VerifiedExecutorBinding,
    checkpoint: &MemoryLedgerCheckpoint,
) -> Result<()> {
    if binding.binding_ref() != &checkpoint.binding_ref {
        return Err(ExecutorError::WrongExecutorBinding);
    }
    if binding.deployment().tenant_scope_id() != &checkpoint.tenant_scope_id {
        return Err(ExecutorError::TenantScopeMismatch);
    }
    if binding.deployment().durable_ledger_generation_ref()
        != &checkpoint.durable_ledger_generation_ref
    {
        return Err(ExecutorError::LedgerGenerationMismatch);
    }
    if binding.deployment().evidence_authority_ref() != &checkpoint.proof_ref {
        return Err(ExecutorError::InvalidFrontierProof);
    }
    Ok(())
}

fn validate_checkpoint_state(checkpoint: &MemoryLedgerCheckpoint) -> Result<()> {
    for (effect_key, stream) in &checkpoint.state.effects {
        if effect_key != stream.identity.effect_key()
            || stream.identity.executor_binding_ref() != &checkpoint.binding_ref
            || stream.identity.tenant_scope_id() != &checkpoint.tenant_scope_id
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        DeliveryAudit::from_ledger(stream.frontiers.clone()).verify_structure(
            &stream.identity,
            &checkpoint.bounds,
            &checkpoint.proof_ref,
        )?;
    }
    validate_resource_links(&checkpoint.state)?;
    validate_state_content(&checkpoint.state)
}

fn validate_state_content(state: &LedgerState) -> Result<()> {
    for (reference, value) in &state.content {
        if &value.reference()? != reference {
            return Err(ExecutorError::RetainedObjectMismatch);
        }
    }
    let mut expected = BTreeMap::new();
    let mut opaque = std::collections::BTreeSet::new();
    for stream in state.effects.values() {
        for frontier in &stream.frontiers {
            for value in frontier.content_objects()? {
                insert_expected_content(&mut expected, value)?;
            }
            for record in frontier.appended_records() {
                if let ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
                    target_operation_ref,
                    ..
                } = record
                {
                    opaque.insert(target_operation_ref.clone());
                }
            }
        }
    }
    for records in state.resources.values() {
        for record in records {
            for value in record.content_objects()? {
                insert_expected_content(&mut expected, value)?;
            }
        }
    }
    for (reference, value) in &expected {
        match state.content.get(reference) {
            Some(actual) if actual == value => {}
            Some(_) => return Err(ExecutorError::RetainedObjectMismatch),
            None => return Err(ExecutorError::RetainedClosureIncomplete),
        }
    }
    if opaque
        .iter()
        .any(|reference| !state.content.contains_key(reference))
    {
        return Err(ExecutorError::RetainedClosureIncomplete);
    }
    if state
        .content
        .keys()
        .any(|reference| !expected.contains_key(reference) && !opaque.contains(reference))
    {
        return Err(ExecutorError::RetainedClosureExtra);
    }
    Ok(())
}

fn insert_expected_content(
    expected: &mut BTreeMap<ContentRef, SchemaQualifiedCanonicalValue>,
    value: SchemaQualifiedCanonicalValue,
) -> Result<()> {
    let reference = value.reference()?;
    match expected.get(&reference) {
        Some(existing) if existing != &value => Err(ExecutorError::RetainedObjectMismatch),
        Some(_) => Ok(()),
        None => {
            expected.insert(reference, value);
            Ok(())
        }
    }
}

fn validate_binding_state(
    binding: &VerifiedExecutorBinding,
    state: &LedgerState,
    bounds: &EvidenceBounds,
) -> Result<()> {
    let checkpoint = MemoryLedgerCheckpoint {
        binding_ref: binding.binding_ref().clone(),
        tenant_scope_id: binding.deployment().tenant_scope_id().clone(),
        durable_ledger_generation_ref: binding.deployment().durable_ledger_generation_ref().clone(),
        proof_ref: binding.deployment().evidence_authority_ref().clone(),
        bounds: bounds.clone(),
        state: state.clone(),
    };
    validate_checkpoint_state(&checkpoint)?;
    for stream in state.effects.values() {
        DeliveryAudit::from_ledger(stream.frontiers.clone()).verify(&stream.identity, binding)?;
    }
    let admitted_owner = binding.deployment().resource_ownership_ref();
    if state
        .resources
        .keys()
        .any(|(owner, _)| Some(owner) != admitted_owner)
    {
        return Err(ExecutorError::ResourceOwnershipReferenceMismatch);
    }
    Ok(())
}

fn validate_resource_links(state: &LedgerState) -> Result<()> {
    let mut allocation_links = BTreeMap::<EffectKey, usize>::new();
    for ((ownership_ref, resource_key_ref), records) in &state.resources {
        let mut predecessor = None;
        let mut policy_binding = None;
        let mut resource_key_value = None;
        for record in records {
            if &record.resource_ownership_ref != ownership_ref
                || &record.resource_key_ref != resource_key_ref
                || record.predecessor != predecessor
                || ResourceKeyRef::from_reviewed(content_ref(&record.resource_key_value)?)
                    != *resource_key_ref
                || AllocationStateRef::from_reviewed(content_ref(&record.typed_allocation_state)?)
                    != record.typed_allocation_state_ref
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            match &policy_binding {
                None => {
                    policy_binding = Some(record.policy_binding.clone());
                    resource_key_value = Some(record.resource_key_value.clone());
                }
                Some(expected)
                    if expected == &record.policy_binding
                        && resource_key_value.as_ref() == Some(&record.resource_key_value) => {}
                Some(_) => {
                    return Err(ExecutorError::ResourcePolicyNotRevalidated);
                }
            }
            predecessor = Some(record.reference()?);
            let links = allocation_links
                .entry(record.effect_key.clone())
                .or_default();
            *links = links
                .checked_add(1)
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
        }
    }

    for (effect_key, stream) in &state.effects {
        match (
            stream.allocation.as_ref(),
            allocation_links.get(effect_key).copied(),
        ) {
            (None, None) => {}
            (Some(_), Some(1)) => {}
            _ => {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
    }
    if allocation_links
        .keys()
        .any(|effect_key| !state.effects.contains_key(effect_key))
    {
        return Err(ExecutorError::InvalidDurableSnapshot);
    }
    Ok(())
}
