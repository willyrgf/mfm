use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use mfm_canonical::ValidatedCanonicalValueV3;
use mfm_capabilities::{
    BoundaryStage, FailureClass, NonDomainDisposition, NonDomainEntryStatus, NonDomainFailure,
    NonDomainFailureCode,
};
use mfm_ids::{AttemptId, ContentRef, EffectKey, SchemaId, TenantScopeId};

use crate::contract::{
    content_ref, ExecutorBindingRef, ExecutorFuture, ResourceKeyRef, ResourceOwnershipRef,
    SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
use crate::frontier::{
    observation_completion_closure_bytes, reference_safe_failure, DeliveryAttemptOutcome,
    DeliveryAudit, DeliveryAuditFrontier, DeliveryAuditFrontierRef, EvidenceBounds,
    ExecutorEvidenceRecord, ReferenceFailureCode, ResourceAllocatedRecord, TerminalTombstone,
};
use crate::ledger::{
    AllocationOutcome, EffectEntryView, ResourceLedgerRecord, ResourceLedgerRecordRef,
    ResourceStreamView, TargetEntryAuthority,
};
use crate::policy::{ResourcePolicyBinding, TypedResourcePolicy};
use crate::{EffectIdentity, ExecutorError, Result};

const MAX_LOCAL_CAS_ATTEMPTS: usize = 64;

/// Exact non-secret authority identity fixed by one raw executor store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorLedgerStoreIdentity {
    binding_ref: ExecutorBindingRef,
    tenant_scope_id: TenantScopeId,
    durable_ledger_generation_ref: ContentRef,
    evidence_authority_ref: ContentRef,
    resource_ownership_ref: Option<ResourceOwnershipRef>,
    safe_failure_contract_ref: ContentRef,
    domain_evidence_schema_id: SchemaId,
}

impl ExecutorLedgerStoreIdentity {
    /// Reconstructs an exact backend-opened raw-store identity.
    pub fn from_store_parts(
        binding_ref: ExecutorBindingRef,
        tenant_scope_id: TenantScopeId,
        durable_ledger_generation_ref: ContentRef,
        evidence_authority_ref: ContentRef,
        resource_ownership_ref: Option<ResourceOwnershipRef>,
        safe_failure_contract_ref: ContentRef,
        domain_evidence_schema_id: SchemaId,
    ) -> Self {
        Self {
            binding_ref,
            tenant_scope_id,
            durable_ledger_generation_ref,
            evidence_authority_ref,
            resource_ownership_ref,
            safe_failure_contract_ref,
            domain_evidence_schema_id,
        }
    }

    /// Derives the exact raw-store identity selected by a verified binding.
    pub fn from_binding(binding: &VerifiedExecutorBinding) -> Self {
        Self {
            binding_ref: binding.binding_ref().clone(),
            tenant_scope_id: binding.deployment().tenant_scope_id().clone(),
            durable_ledger_generation_ref: binding
                .deployment()
                .durable_ledger_generation_ref()
                .clone(),
            evidence_authority_ref: binding.deployment().evidence_authority_ref().clone(),
            resource_ownership_ref: binding.deployment().resource_ownership_ref().cloned(),
            safe_failure_contract_ref: binding.contract().safe_failure_contract_ref().clone(),
            domain_evidence_schema_id: binding
                .contract()
                .retained_closure_contract()
                .domain_evidence_contract()
                .schema_id()
                .clone(),
        }
    }

    /// Returns the exact executor binding.
    pub const fn binding_ref(&self) -> &ExecutorBindingRef {
        &self.binding_ref
    }

    /// Returns the sole admitted tenant partition.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the non-rollback durable ledger generation.
    pub const fn durable_ledger_generation_ref(&self) -> &ContentRef {
        &self.durable_ledger_generation_ref
    }

    /// Returns the binding-selected evidence authority.
    pub const fn evidence_authority_ref(&self) -> &ContentRef {
        &self.evidence_authority_ref
    }

    /// Returns the admitted resource owner, when resource coordination is enabled.
    pub const fn resource_ownership_ref(&self) -> Option<&ResourceOwnershipRef> {
        self.resource_ownership_ref.as_ref()
    }

    /// Returns the exact safe-failure contract admitted by the binding.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.safe_failure_contract_ref
    }

    /// Returns the only admitted target-return schema.
    pub const fn domain_evidence_schema_id(&self) -> &SchemaId {
        &self.domain_evidence_schema_id
    }
}

/// Complete immutable raw history for one effect key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorEffectSnapshot {
    identity: EffectIdentity,
    frontiers: Vec<DeliveryAuditFrontier>,
    allocation: Option<ResourceLedgerRecord>,
}

impl ExecutorEffectSnapshot {
    /// Reconstructs one backend-loaded effect snapshot.
    ///
    /// The shared engine strictly verifies the snapshot before exposing any
    /// folded view or deriving an append proposal.
    pub fn from_store_parts(
        identity: EffectIdentity,
        frontiers: Vec<DeliveryAuditFrontier>,
        allocation: Option<ResourceLedgerRecord>,
    ) -> Self {
        Self {
            identity,
            frontiers,
            allocation,
        }
    }

    /// Returns the immutable effect identity.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.identity
    }

    /// Returns every predecessor-linked delivery frontier.
    pub fn frontiers(&self) -> &[DeliveryAuditFrontier] {
        &self.frontiers
    }

    /// Returns the linked resource allocation, when present.
    pub const fn allocation(&self) -> Option<&ResourceLedgerRecord> {
        self.allocation.as_ref()
    }

    /// Returns the current immutable effect head.
    pub fn head(&self) -> Result<DeliveryAuditFrontierRef> {
        self.frontiers
            .last()
            .ok_or(ExecutorError::InvalidFrontier)?
            .reference()
    }
}

/// Complete immutable raw history for one owned resource key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorResourceSnapshot {
    resource_ownership_ref: ResourceOwnershipRef,
    resource_key_ref: ResourceKeyRef,
    records: Vec<ResourceLedgerRecord>,
}

impl ExecutorResourceSnapshot {
    /// Reconstructs one backend-loaded resource snapshot.
    ///
    /// An empty record list represents an absent stream under the exact key.
    pub fn from_store_parts(
        resource_ownership_ref: ResourceOwnershipRef,
        resource_key_ref: ResourceKeyRef,
        records: Vec<ResourceLedgerRecord>,
    ) -> Self {
        Self {
            resource_ownership_ref,
            resource_key_ref,
            records,
        }
    }

    /// Returns the exact admitted resource owner.
    pub const fn resource_ownership_ref(&self) -> &ResourceOwnershipRef {
        &self.resource_ownership_ref
    }

    /// Returns the exact typed resource key.
    pub const fn resource_key_ref(&self) -> &ResourceKeyRef {
        &self.resource_key_ref
    }

    /// Returns every predecessor-linked resource record.
    pub fn records(&self) -> &[ResourceLedgerRecord] {
        &self.records
    }

    /// Returns the current immutable resource head.
    pub fn head(&self) -> Result<Option<ResourceLedgerRecordRef>> {
        self.records
            .last()
            .map(ResourceLedgerRecord::reference)
            .transpose()
    }
}

/// Complete backend-private enumeration supplied for strict reopen/refold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorStoreSnapshot {
    store_identity: ExecutorLedgerStoreIdentity,
    effects: Vec<ExecutorEffectSnapshot>,
    resources: Vec<ExecutorResourceSnapshot>,
    content: Vec<SchemaQualifiedCanonicalValue>,
}

impl ExecutorStoreSnapshot {
    /// Reconstructs a complete enumerated raw store snapshot.
    pub fn from_store_parts(
        store_identity: ExecutorLedgerStoreIdentity,
        effects: Vec<ExecutorEffectSnapshot>,
        resources: Vec<ExecutorResourceSnapshot>,
        content: Vec<SchemaQualifiedCanonicalValue>,
    ) -> Self {
        Self {
            store_identity,
            effects,
            resources,
            content,
        }
    }

    /// Returns the exact raw-store identity.
    pub const fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        &self.store_identity
    }

    /// Returns every unique effect snapshot.
    pub fn effects(&self) -> &[ExecutorEffectSnapshot] {
        &self.effects
    }

    /// Returns every unique non-empty resource snapshot.
    pub fn resources(&self) -> &[ExecutorResourceSnapshot] {
        &self.resources
    }

    /// Returns the complete immutable executor-owned content inventory.
    pub fn content(&self) -> &[SchemaQualifiedCanonicalValue] {
        &self.content
    }
}

/// Optional resource-side member of one atomic executor append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorResourceAppend {
    expected_head: Option<ResourceLedgerRecordRef>,
    record: ResourceLedgerRecord,
}

impl ExecutorResourceAppend {
    pub(crate) fn new(
        expected_head: Option<ResourceLedgerRecordRef>,
        record: ResourceLedgerRecord,
    ) -> Self {
        Self {
            expected_head,
            record,
        }
    }

    /// Returns the resource head compared by the raw store.
    pub const fn expected_head(&self) -> Option<&ResourceLedgerRecordRef> {
        self.expected_head.as_ref()
    }

    /// Returns the sole new immutable resource record.
    pub const fn record(&self) -> &ResourceLedgerRecord {
        &self.record
    }
}

/// One raw atomic compare-and-append proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorLedgerAppend {
    identity: EffectIdentity,
    expected_effect_head: Option<DeliveryAuditFrontierRef>,
    effect_frontier: DeliveryAuditFrontier,
    resource: Option<ExecutorResourceAppend>,
    content: Vec<SchemaQualifiedCanonicalValue>,
}

impl ExecutorLedgerAppend {
    fn new(
        identity: EffectIdentity,
        expected_effect_head: Option<DeliveryAuditFrontierRef>,
        effect_frontier: DeliveryAuditFrontier,
        resource: Option<ExecutorResourceAppend>,
        retained_content: Vec<SchemaQualifiedCanonicalValue>,
    ) -> Result<Self> {
        let mut content = effect_frontier.content_objects()?;
        if let Some(resource) = &resource {
            content.extend(resource.record.content_objects()?);
        }
        content.extend(retained_content);
        deduplicate_content(&mut content)?;
        Ok(Self {
            identity,
            expected_effect_head,
            effect_frontier,
            resource,
            content,
        })
    }

    /// Returns the exact effect identity being advanced.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.identity
    }

    /// Returns the effect head compared by the raw store.
    pub const fn expected_effect_head(&self) -> Option<&DeliveryAuditFrontierRef> {
        self.expected_effect_head.as_ref()
    }

    /// Returns the sole new immutable effect frontier.
    pub const fn effect_frontier(&self) -> &DeliveryAuditFrontier {
        &self.effect_frontier
    }

    /// Returns the optional atomically linked resource append.
    pub const fn resource(&self) -> Option<&ExecutorResourceAppend> {
        self.resource.as_ref()
    }

    /// Returns the exact deduplicated content closure introduced by this append.
    pub fn content_objects(&self) -> &[SchemaQualifiedCanonicalValue] {
        &self.content
    }

    /// Reconstructs and validates every supplied target outcome against the
    /// exact raw-store binding before persistence.
    pub fn validate_for_store_identity(
        &self,
        store_identity: &ExecutorLedgerStoreIdentity,
    ) -> Result<()> {
        if self.identity.executor_binding_ref() != store_identity.binding_ref()
            || self.identity.tenant_scope_id() != store_identity.tenant_scope_id()
        {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        for record in self.effect_frontier.appended_records() {
            if let ExecutorEvidenceRecord::DeliveryAttemptObserved { outcome, .. } = record {
                outcome.validate_for_contract(
                    store_identity.safe_failure_contract_ref(),
                    store_identity.domain_evidence_schema_id(),
                )?;
            }
        }
        Ok(())
    }
}

/// Durable result of one raw compare-and-append request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorAppendOutcome {
    /// The complete proposal committed and was positively acknowledged.
    Applied,
    /// The exact proposal was already present before this call.
    AlreadyApplied,
    /// One compared immutable head differed and this proposal is absent.
    Conflict,
    /// The backend cannot prove whether this proposal committed.
    OutcomeUnknown,
}

/// Narrow asynchronous raw store used by the shared keyed-executor engine.
///
/// Implementations persist only immutable snapshots, exact content objects,
/// and atomic compare-and-append proposals. They do not own effect folding,
/// resource policy, attempt derivation, terminal semantics, or target entry.
pub trait ExecutorLedgerStore: Clone + Send + Sync + 'static {
    /// Returns the exact fenced authority identity fixed at store open.
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity;

    /// Loads one complete immutable effect history.
    fn load_effect<'a>(
        &'a self,
        effect_key: &'a EffectKey,
    ) -> ExecutorFuture<'a, Result<Option<ExecutorEffectSnapshot>>>;

    /// Loads one complete immutable resource history.
    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> ExecutorFuture<'a, Result<ExecutorResourceSnapshot>>;

    /// Loads one exact immutable executor-owned content object.
    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> ExecutorFuture<'a, Result<Option<SchemaQualifiedCanonicalValue>>>;

    /// Atomically compares all named heads and appends the complete proposal.
    fn compare_and_append<'a>(
        &'a self,
        append: ExecutorLedgerAppend,
    ) -> ExecutorFuture<'a, Result<ExecutorAppendOutcome>>;
}

/// Shared storage-neutral high-level keyed-executor ledger.
#[derive(Debug, Clone)]
pub struct KeyedExecutorLedger<Store>
where
    Store: ExecutorLedgerStore,
{
    store: Store,
    binding: VerifiedExecutorBinding,
    bounds: EvidenceBounds,
}

/// Closed result of one exact target-authorization and observation bracket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecuteTargetOutcome {
    /// Fresh target authority returned an outcome and its exact sealed observation is durable.
    Observed(Box<EffectEntryView>),
    /// Another writer advanced the pre-target frontier; no target authority was minted.
    Contended,
}

struct TargetObservationBackoff {
    next_delay_ms: u64,
}

struct AuthorizedTarget {
    authority: TargetEntryAuthority,
    completion_seal: TargetCompletionSeal,
}

struct TargetCompletionSealParts {
    identity: EffectIdentity,
    attempt_id: AttemptId,
    target_operation_ref: ContentRef,
    durable_ledger_generation_ref: ContentRef,
    observation_predecessor_ref: DeliveryAuditFrontierRef,
    evidence_authority_ref: ContentRef,
    max_completion_record_bytes: usize,
    safe_failure_contract_ref: ContentRef,
    domain_evidence_schema_id: SchemaId,
    adapter_contract_violation: DeliveryAttemptOutcome,
    result_unrepresentable: DeliveryAttemptOutcome,
}

/// Private affine completion authority retained across one target callback.
///
/// The callback receives only [`TargetEntryAuthority`] and can return only an
/// unbound [`DeliveryAttemptOutcome`]. This seal cannot leave the crate and is
/// the sole operation that binds a surviving outcome to the current target
/// authorization.
pub(crate) struct TargetCompletionSeal {
    parts: TargetCompletionSealParts,
}

struct BoundTargetObservation {
    identity: EffectIdentity,
    attempt_id: AttemptId,
    target_operation_ref: ContentRef,
    durable_ledger_generation_ref: ContentRef,
    outcome: DeliveryAttemptOutcome,
}

impl TargetCompletionSeal {
    fn new(parts: TargetCompletionSealParts) -> Self {
        Self { parts }
    }

    fn bind(self, outcome: DeliveryAttemptOutcome) -> BoundTargetObservation {
        let parts = self.parts;
        let outcome = if outcome
            .validate_for_contract(
                &parts.safe_failure_contract_ref,
                &parts.domain_evidence_schema_id,
            )
            .is_err()
        {
            parts.adapter_contract_violation
        } else if observation_completion_closure_bytes(
            &parts.identity,
            parts.observation_predecessor_ref,
            parts.attempt_id.clone(),
            outcome.clone(),
            parts.evidence_authority_ref,
        )
        .is_ok_and(|bytes| bytes <= parts.max_completion_record_bytes)
        {
            outcome
        } else {
            parts.result_unrepresentable
        };
        BoundTargetObservation {
            identity: parts.identity,
            attempt_id: parts.attempt_id,
            target_operation_ref: parts.target_operation_ref,
            durable_ledger_generation_ref: parts.durable_ledger_generation_ref,
            outcome,
        }
    }
}

impl TargetObservationBackoff {
    const fn new() -> Self {
        Self { next_delay_ms: 10 }
    }

    const fn reset(&mut self) {
        self.next_delay_ms = 10;
    }

    async fn wait(&mut self) {
        tokio::time::sleep(std::time::Duration::from_millis(self.next_delay_ms)).await;
        self.next_delay_ms = self.next_delay_ms.saturating_mul(2).min(1_000);
    }
}

impl<Store> KeyedExecutorLedger<Store>
where
    Store: ExecutorLedgerStore,
{
    /// Binds one fenced raw store to its exact verified executor contract.
    pub fn new(store: Store, binding: VerifiedExecutorBinding) -> Result<Self> {
        let expected = ExecutorLedgerStoreIdentity::from_binding(&binding);
        if store.store_identity() != &expected {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        let bounds = binding.contract().evidence_bounds().clone();
        Ok(Self {
            store,
            binding,
            bounds,
        })
    }

    /// Returns the exact verified executor binding.
    pub const fn exact_binding(&self) -> &VerifiedExecutorBinding {
        &self.binding
    }

    /// Returns the contract's finite evidence bounds.
    pub const fn evidence_bounds(&self) -> &EvidenceBounds {
        &self.bounds
    }

    /// Returns the raw store selected by this engine.
    pub const fn store(&self) -> &Store {
        &self.store
    }

    /// Idempotently binds one exact effect/request identity.
    pub async fn bind_effect(&self, identity: &EffectIdentity) -> Result<EffectEntryView> {
        self.require_identity(identity)?;
        for _ in 0..MAX_LOCAL_CAS_ATTEMPTS {
            if let Some(snapshot) = self.load_effect(identity).await? {
                return fold_effect(&snapshot);
            }
            let frontier =
                self.next_frontier(identity, None, vec![effect_bound_record(identity)])?;
            let append =
                ExecutorLedgerAppend::new(identity.clone(), None, frontier, None, Vec::new())?;
            match self.store.compare_and_append(append).await? {
                ExecutorAppendOutcome::Applied
                | ExecutorAppendOutcome::AlreadyApplied
                | ExecutorAppendOutcome::Conflict => {}
                ExecutorAppendOutcome::OutcomeUnknown => {
                    return Err(ExecutorError::DurableAppendOutcomeUnknown);
                }
            }
        }
        self.load_effect(identity)
            .await?
            .as_ref()
            .map(fold_effect)
            .transpose()?
            .ok_or(ExecutorError::LocalContention)
    }

    /// Loads one folded immutable effect view under the exact identity.
    pub async fn effect_view(&self, identity: &EffectIdentity) -> Result<Option<EffectEntryView>> {
        self.require_identity(identity)?;
        self.load_effect(identity)
            .await?
            .as_ref()
            .map(fold_effect)
            .transpose()
    }

    /// Returns the current immutable resource stream head.
    pub async fn resource_head(
        &self,
        resource_key_ref: &ResourceKeyRef,
    ) -> Result<Option<ResourceLedgerRecordRef>> {
        let ownership = self.required_resource_owner()?;
        self.load_resource(ownership, resource_key_ref)
            .await?
            .head()
    }

    /// Returns one complete immutable typed resource stream.
    pub async fn resource_records(
        &self,
        resource_key_ref: &ResourceKeyRef,
    ) -> Result<Vec<ResourceLedgerRecord>> {
        let ownership = self.required_resource_owner()?;
        Ok(self
            .load_resource(ownership, resource_key_ref)
            .await?
            .records
            .clone())
    }

    /// Atomically binds or upgrades an effect and appends one policy allocation.
    pub async fn try_bind_and_allocate<Policy>(
        &self,
        identity: &EffectIdentity,
        expected_resource_head: Option<&ResourceLedgerRecordRef>,
        policy: &Policy,
        request: &Policy::Request,
    ) -> Result<AllocationOutcome<Policy::Allocation>>
    where
        Policy: TypedResourcePolicy,
    {
        self.require_identity(identity)?;
        let ownership_ref = self.required_resource_owner()?.clone();
        let resource_key_value = policy
            .resource_key_value(request)
            .map_err(ExecutorError::ResourcePolicy)?;
        let resource_key_ref = ResourceKeyRef::from_reviewed(content_ref(&resource_key_value)?);
        let mut final_applied = None;

        for attempt in 0..MAX_LOCAL_CAS_ATTEMPTS {
            let existing_effect = self.load_effect(identity).await?;
            if let Some(snapshot) = &existing_effect {
                if let Some(record) = snapshot.allocation() {
                    return self
                        .restore_existing_allocation(
                            identity,
                            &resource_key_ref,
                            &resource_key_value,
                            policy,
                            request,
                            record,
                        )
                        .await;
                }
                let audit = DeliveryAudit::from_ledger(snapshot.frontiers.clone());
                if audit.attempt_count() != 0 {
                    return Err(ExecutorError::ResourceAllocationConflict);
                }
            }

            let (resource, terminal_effects) = self
                .load_resource_history(&ownership_ref, &resource_key_ref)
                .await?;
            let actual_head = resource.head()?;
            if actual_head.as_ref() != expected_resource_head {
                return Err(ExecutorError::ResourceCasMismatch);
            }
            let history =
                ResourceStreamView::from_verified_parts(resource.records(), &terminal_effects);
            let decision = policy
                .allocate(identity, request, history)
                .map_err(ExecutorError::ResourcePolicy)?;
            let typed_allocation_state_ref =
                crate::AllocationStateRef::from_reviewed(content_ref(&decision.allocation_state)?);
            let record = ResourceLedgerRecord {
                resource_ownership_ref: ownership_ref.clone(),
                resource_key_ref: resource_key_ref.clone(),
                resource_key_value: resource_key_value.clone(),
                predecessor: actual_head.clone(),
                effect_key: identity.effect_key().clone(),
                typed_allocation_state_ref: typed_allocation_state_ref.clone(),
                typed_allocation_state: decision.allocation_state,
                policy_binding: policy.binding().clone(),
                fencing_ref: decision.fencing_ref,
            };
            let evidence = record.evidence();
            let resource_record =
                ExecutorEvidenceRecord::ResourceAllocated(Box::new(ResourceAllocatedRecord::new(
                    ownership_ref.clone(),
                    resource_key_ref.clone(),
                    typed_allocation_state_ref,
                    policy.binding().policy_ref().clone(),
                    policy.binding().policy_configuration_ref().clone(),
                    evidence.fencing_ref().cloned(),
                )));
            let mut records = Vec::with_capacity(2);
            if existing_effect.is_none() {
                records.push(effect_bound_record(identity));
            }
            records.push(resource_record);
            let expected_effect_head = existing_effect
                .as_ref()
                .map(ExecutorEffectSnapshot::head)
                .transpose()?;
            let frontier = self.next_frontier(identity, existing_effect.as_ref(), records)?;
            let resource_head = record.reference()?;
            let append = ExecutorLedgerAppend::new(
                identity.clone(),
                expected_effect_head,
                frontier,
                Some(ExecutorResourceAppend::new(actual_head, record)),
                Vec::new(),
            )?;
            match self.store.compare_and_append(append).await? {
                ExecutorAppendOutcome::Applied => {
                    let allocated = AllocationOutcome::Allocated {
                        allocation: decision.allocation,
                        evidence,
                        resource_head,
                    };
                    if attempt + 1 == MAX_LOCAL_CAS_ATTEMPTS {
                        final_applied = Some(allocated);
                    } else {
                        return Ok(allocated);
                    }
                }
                ExecutorAppendOutcome::AlreadyApplied | ExecutorAppendOutcome::Conflict => {}
                ExecutorAppendOutcome::OutcomeUnknown => {
                    return Err(ExecutorError::DurableAppendOutcomeUnknown);
                }
            }
        }
        let existing_effect = self.load_effect(identity).await?;
        if let Some(snapshot) = &existing_effect {
            if let Some(record) = snapshot.allocation() {
                let existing = self
                    .restore_existing_allocation(
                        identity,
                        &resource_key_ref,
                        &resource_key_value,
                        policy,
                        request,
                        record,
                    )
                    .await?;
                return Ok(final_applied.unwrap_or(existing));
            }
            let audit = DeliveryAudit::from_ledger(snapshot.frontiers.clone());
            if audit.attempt_count() != 0 {
                return Err(ExecutorError::ResourceAllocationConflict);
            }
        }
        let (resource, _) = self
            .load_resource_history(&ownership_ref, &resource_key_ref)
            .await?;
        if resource.head()?.as_ref() != expected_resource_head {
            return Err(ExecutorError::ResourceCasMismatch);
        }
        Err(ExecutorError::LocalContention)
    }

    /// Attempts one authorization against the exact delivery head used to plan it.
    ///
    /// `Ok(None)` means another writer advanced the effect before this caller
    /// committed its authority. The caller must refold and replan; it must not
    /// enter the target boundary from the stale decision.
    async fn try_authorize_target(
        &self,
        identity: &EffectIdentity,
        expected_head: &DeliveryAuditFrontierRef,
        target_operation: SchemaQualifiedCanonicalValue,
        policy_binding: Option<&ResourcePolicyBinding>,
    ) -> Result<Option<AuthorizedTarget>> {
        self.require_identity(identity)?;
        if target_operation.as_bytes().len() > self.bounds.max_retained_bytes() {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        let target_operation_ref = target_operation.reference()?;
        let snapshot = self
            .load_effect(identity)
            .await?
            .ok_or(ExecutorError::EffectNotBound)?;
        if snapshot.head()? != *expected_head {
            return Ok(None);
        }
        let view = fold_effect(&snapshot)?;
        if view.terminal_tombstone().is_some() {
            return Err(ExecutorError::EffectAlreadyTerminal);
        }
        match (view.allocation(), policy_binding) {
            (None, None) => {}
            (Some((_, evidence)), Some(candidate)) if evidence.policy_binding() == candidate => {}
            _ => return Err(ExecutorError::ResourcePolicyNotRevalidated),
        }
        if let Some(allocation) = snapshot.allocation() {
            self.require_linked_allocation(allocation).await?;
        }
        let ordinal = u32::try_from(view.delivery_audit().attempt_count())
            .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?;
        if ordinal >= self.bounds.max_attempts() {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        let attempt_id = crate::derive_attempt_id(identity, ordinal, &target_operation_ref)?;
        let result_unrepresentable =
            DeliveryAttemptOutcome::indeterminate(reference_safe_failure(
                self.binding.contract().safe_failure_contract_ref().clone(),
                ReferenceFailureCode::ResultUnrepresentable,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation,
            )?)?;
        let adapter_contract_violation = DeliveryAttemptOutcome::non_domain_failure(
            NonDomainFailure::new(
                NonDomainEntryStatus::MayHaveEntered,
                NonDomainDisposition::IntegrityBlocked,
                NonDomainFailureCode::AdapterContractViolation,
            )
            .map_err(|_| ExecutorError::InvalidSafeFailure)?,
        )?;
        result_unrepresentable.validate_for_binding(&self.binding)?;
        adapter_contract_violation.validate_for_binding(&self.binding)?;
        let frontier = self.next_frontier(
            identity,
            Some(&snapshot),
            vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
                attempt_ordinal: ordinal,
                attempt_id: attempt_id.clone(),
                target_operation_ref: target_operation_ref.clone(),
            }],
        )?;
        let observation_predecessor_ref = frontier.reference()?;
        for fallback in [&result_unrepresentable, &adapter_contract_violation] {
            let fallback_bytes = observation_completion_closure_bytes(
                identity,
                observation_predecessor_ref.clone(),
                attempt_id.clone(),
                fallback.clone(),
                self.binding.deployment().evidence_authority_ref().clone(),
            )?;
            if fallback_bytes > self.bounds.max_completion_record_bytes() {
                return Err(ExecutorError::EvidenceBoundsExhausted);
            }
        }
        let append = ExecutorLedgerAppend::new(
            identity.clone(),
            Some(expected_head.clone()),
            frontier,
            None,
            vec![target_operation],
        )?;
        match self.store.compare_and_append(append).await? {
            ExecutorAppendOutcome::Applied => {
                let durable_ledger_generation_ref = self
                    .binding
                    .deployment()
                    .durable_ledger_generation_ref()
                    .clone();
                let authority = TargetEntryAuthority::new(
                    identity.clone(),
                    attempt_id.clone(),
                    target_operation_ref.clone(),
                    durable_ledger_generation_ref.clone(),
                );
                let completion_seal = TargetCompletionSeal::new(TargetCompletionSealParts {
                    identity: identity.clone(),
                    attempt_id,
                    target_operation_ref,
                    durable_ledger_generation_ref,
                    observation_predecessor_ref,
                    evidence_authority_ref: self
                        .binding
                        .deployment()
                        .evidence_authority_ref()
                        .clone(),
                    max_completion_record_bytes: self.bounds.max_completion_record_bytes(),
                    safe_failure_contract_ref: self
                        .binding
                        .contract()
                        .safe_failure_contract_ref()
                        .clone(),
                    domain_evidence_schema_id: self
                        .binding
                        .contract()
                        .retained_closure_contract()
                        .domain_evidence_contract()
                        .schema_id()
                        .clone(),
                    adapter_contract_violation,
                    result_unrepresentable,
                });
                Ok(Some(AuthorizedTarget {
                    authority,
                    completion_seal,
                }))
            }
            ExecutorAppendOutcome::AlreadyApplied | ExecutorAppendOutcome::Conflict => Ok(None),
            ExecutorAppendOutcome::OutcomeUnknown => {
                Err(ExecutorError::DurableAppendOutcomeUnknown)
            }
        }
    }

    /// Authorizes, invokes, and durably observes exactly one target operation.
    ///
    /// The invoker is called only after a fresh target authorization append.
    /// A pre-target compare-and-swap conflict returns [`ExecuteTargetOutcome::Contended`]
    /// without minting authority. The private affine completion seal remains
    /// in this method while the invoker receives only the public four-field
    /// authority and returns an unbound outcome. Once the invoker returns,
    /// completion is infallibly normalized and this method owns observation
    /// persistence until the identical outcome is durably resolved. Dropping
    /// this future before callback return or during persistence is the only
    /// unmatched target path.
    pub async fn execute_target_once<Invoke, Invocation>(
        &self,
        identity: &EffectIdentity,
        expected_head: &DeliveryAuditFrontierRef,
        target_operation: SchemaQualifiedCanonicalValue,
        policy_binding: Option<&ResourcePolicyBinding>,
        invoke: Invoke,
    ) -> Result<ExecuteTargetOutcome>
    where
        Invoke: FnOnce(TargetEntryAuthority) -> Invocation,
        Invocation: Future<Output = DeliveryAttemptOutcome>,
    {
        let Some(authorized) = self
            .try_authorize_target(identity, expected_head, target_operation, policy_binding)
            .await?
        else {
            return Ok(ExecuteTargetOutcome::Contended);
        };
        let outcome = invoke(authorized.authority).await;
        let observation = authorized.completion_seal.bind(outcome);
        Ok(ExecuteTargetOutcome::Observed(Box::new(
            self.observe_target(observation).await,
        )))
    }

    /// Loads the exact retained target-operation descriptor for one attempt.
    pub async fn target_operation(
        &self,
        identity: &EffectIdentity,
        attempt_id: &AttemptId,
    ) -> Result<SchemaQualifiedCanonicalValue> {
        let snapshot = self
            .load_effect(identity)
            .await?
            .ok_or(ExecutorError::EffectNotBound)?;
        let audit = DeliveryAudit::from_ledger(snapshot.frontiers.clone());
        let target_operation_ref = audit
            .attempts()?
            .into_iter()
            .find(|attempt| attempt.attempt_id() == attempt_id)
            .map(|attempt| attempt.target_operation_ref().clone())
            .ok_or(ExecutorError::AttemptIdentityMismatch)?;
        let value = self
            .store
            .load_content(&target_operation_ref)
            .await?
            .ok_or(ExecutorError::RetainedClosureIncomplete)?;
        if value.reference()? != target_operation_ref {
            return Err(ExecutorError::RetainedObjectMismatch);
        }
        Ok(value)
    }

    /// Consumes one privately bound target result into exactly one observation view.
    ///
    /// Once entered, every load, projection, construction, and append failure
    /// is treated as unresolved persistence and retried. A non-identical
    /// existing observation is also unresolved here: no second callback is
    /// invoked and cancellation remains the only escape from a permanently
    /// hostile backend.
    async fn observe_target(&self, observation: BoundTargetObservation) -> EffectEntryView {
        let BoundTargetObservation {
            identity,
            attempt_id,
            target_operation_ref,
            durable_ledger_generation_ref: _durable_ledger_generation_ref,
            outcome,
        } = observation;
        let mut last_verified_head: Option<DeliveryAuditFrontierRef> = None;
        let mut backoff = TargetObservationBackoff::new();
        loop {
            let snapshot = match self.load_effect(&identity).await {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) | Err(_) => {
                    backoff.wait().await;
                    continue;
                }
            };
            let loaded_head = match snapshot.head() {
                Ok(head) => head,
                Err(_) => {
                    backoff.wait().await;
                    continue;
                }
            };
            if last_verified_head
                .as_ref()
                .is_some_and(|head| head != &loaded_head)
            {
                backoff.reset();
            }
            last_verified_head = Some(loaded_head);
            let audit = DeliveryAudit::from_ledger(snapshot.frontiers.clone());
            let existing = match audit.attempts() {
                Ok(attempts) => {
                    let Some(attempt) = attempts
                        .into_iter()
                        .find(|attempt| attempt.attempt_id() == &attempt_id)
                    else {
                        backoff.wait().await;
                        continue;
                    };
                    if attempt.target_operation_ref() != &target_operation_ref {
                        backoff.wait().await;
                        continue;
                    }
                    attempt.outcome().cloned()
                }
                Err(_) => {
                    backoff.wait().await;
                    continue;
                }
            };
            if let Some(existing) = existing {
                if existing != outcome {
                    backoff.wait().await;
                    continue;
                }
                match fold_effect(&snapshot) {
                    Ok(view) => return view,
                    Err(_) => {
                        backoff.wait().await;
                        continue;
                    }
                }
            }
            let expected_head = match snapshot.head() {
                Ok(head) => head,
                Err(_) => {
                    backoff.wait().await;
                    continue;
                }
            };
            let frontier = match self.next_frontier(
                &identity,
                Some(&snapshot),
                vec![ExecutorEvidenceRecord::DeliveryAttemptObserved {
                    attempt_id: attempt_id.clone(),
                    outcome: outcome.clone(),
                }],
            ) {
                Ok(frontier) => frontier,
                Err(_) => {
                    backoff.wait().await;
                    continue;
                }
            };
            let append = match ExecutorLedgerAppend::new(
                identity.clone(),
                Some(expected_head),
                frontier,
                None,
                Vec::new(),
            ) {
                Ok(append) => append,
                Err(_) => {
                    backoff.wait().await;
                    continue;
                }
            };
            match self.store.compare_and_append(append).await {
                Ok(ExecutorAppendOutcome::Applied | ExecutorAppendOutcome::AlreadyApplied) => {
                    backoff.reset()
                }
                Ok(ExecutorAppendOutcome::Conflict | ExecutorAppendOutcome::OutcomeUnknown)
                | Err(_) => {
                    backoff.wait().await;
                }
            }
        }
    }

    /// Appends one immutable tombstone or returns the identical existing one.
    pub async fn append_terminal_tombstone(
        &self,
        identity: &EffectIdentity,
        tombstone: TerminalTombstone,
    ) -> Result<EffectEntryView> {
        self.require_identity(identity)?;
        for _ in 0..MAX_LOCAL_CAS_ATTEMPTS {
            let snapshot = self
                .load_effect(identity)
                .await?
                .ok_or(ExecutorError::EffectNotBound)?;
            let view = fold_effect(&snapshot)?;
            if let Some((_, existing)) = view.terminal_tombstone() {
                if existing == &tombstone {
                    return Ok(view);
                }
                return Err(ExecutorError::TerminalTombstoneConflict);
            }
            let expected_head = snapshot.head()?;
            let frontier = self.next_frontier(
                identity,
                Some(&snapshot),
                vec![ExecutorEvidenceRecord::TerminalTombstone(tombstone.clone())],
            )?;
            let append = ExecutorLedgerAppend::new(
                identity.clone(),
                Some(expected_head),
                frontier,
                None,
                Vec::new(),
            )?;
            match self.store.compare_and_append(append).await? {
                ExecutorAppendOutcome::Applied
                | ExecutorAppendOutcome::AlreadyApplied
                | ExecutorAppendOutcome::Conflict => {}
                ExecutorAppendOutcome::OutcomeUnknown => {
                    return Err(ExecutorError::DurableAppendOutcomeUnknown);
                }
            }
        }
        let snapshot = self
            .load_effect(identity)
            .await?
            .ok_or(ExecutorError::LocalContention)?;
        let view = fold_effect(&snapshot)?;
        match view.terminal_tombstone() {
            Some((_, existing)) if existing == &tombstone => Ok(view),
            Some(_) => Err(ExecutorError::TerminalTombstoneConflict),
            None => Err(ExecutorError::LocalContention),
        }
    }

    /// Strictly refolds a backend-private complete store enumeration.
    pub fn strict_refold(&self, snapshot: &ExecutorStoreSnapshot) -> Result<()> {
        if snapshot.store_identity() != self.store.store_identity() {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        let mut effects = BTreeMap::new();
        for effect in snapshot.effects() {
            self.verify_effect_snapshot(effect)?;
            if effects
                .insert(effect.identity().effect_key().clone(), effect)
                .is_some()
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        let mut allocation_links = BTreeMap::<EffectKey, usize>::new();
        let mut resources = BTreeSet::new();
        for resource in snapshot.resources() {
            self.verify_resource_snapshot(resource)?;
            if resource.records().is_empty()
                || !resources.insert((
                    resource.resource_ownership_ref().clone(),
                    resource.resource_key_ref().clone(),
                ))
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            for record in resource.records() {
                let effect = effects
                    .get(record.effect_key())
                    .ok_or(ExecutorError::InvalidDurableSnapshot)?;
                if effect.allocation() != Some(record) {
                    return Err(ExecutorError::InvalidDurableSnapshot);
                }
                let count = allocation_links
                    .entry(record.effect_key().clone())
                    .or_default();
                *count = count
                    .checked_add(1)
                    .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            }
        }
        for (effect_key, effect) in effects {
            match (
                effect.allocation(),
                allocation_links.get(&effect_key).copied(),
            ) {
                (None, None) | (Some(_), Some(1)) => {}
                _ => return Err(ExecutorError::InvalidDurableSnapshot),
            }
        }
        self.verify_content_inventory(snapshot)
    }

    fn verify_content_inventory(&self, snapshot: &ExecutorStoreSnapshot) -> Result<()> {
        let mut actual = BTreeMap::<ContentRef, &SchemaQualifiedCanonicalValue>::new();
        for value in snapshot.content() {
            let reference = value.reference()?;
            if let Some(existing) = actual.insert(reference, value) {
                if existing != value {
                    return Err(ExecutorError::RetainedObjectMismatch);
                }
                return Err(ExecutorError::RetainedClosureDuplicate);
            }
        }
        let mut expected = Vec::new();
        let mut opaque = BTreeSet::new();
        for effect in snapshot.effects() {
            for frontier in effect.frontiers() {
                expected.extend(frontier.content_objects()?);
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
        for resource in snapshot.resources() {
            for record in resource.records() {
                expected.extend(record.content_objects()?);
            }
        }
        deduplicate_content(&mut expected)?;
        let expected = expected
            .into_iter()
            .map(|value| Ok((value.reference()?, value)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        for (reference, value) in &expected {
            let Some(actual_value) = actual.get(reference).copied() else {
                return Err(ExecutorError::RetainedClosureIncomplete);
            };
            if actual_value != value {
                return Err(ExecutorError::RetainedObjectMismatch);
            }
        }
        for reference in &opaque {
            if !actual.contains_key(reference) {
                return Err(ExecutorError::RetainedClosureIncomplete);
            }
        }
        for reference in actual.keys() {
            if !expected.contains_key(reference) && !opaque.contains(reference) {
                return Err(ExecutorError::RetainedClosureExtra);
            }
        }
        Ok(())
    }

    async fn restore_existing_allocation<Policy>(
        &self,
        identity: &EffectIdentity,
        resource_key_ref: &ResourceKeyRef,
        resource_key_value: &ValidatedCanonicalValueV3,
        policy: &Policy,
        request: &Policy::Request,
        allocation_record: &ResourceLedgerRecord,
    ) -> Result<AllocationOutcome<Policy::Allocation>>
    where
        Policy: TypedResourcePolicy,
    {
        let ownership_ref = self.required_resource_owner()?;
        if allocation_record.resource_ownership_ref() != ownership_ref
            || allocation_record.resource_key_ref() != resource_key_ref
            || allocation_record.resource_key_value() != resource_key_value
            || allocation_record.policy_binding() != policy.binding()
            || allocation_record.effect_key() != identity.effect_key()
        {
            return Err(ExecutorError::ResourceAllocationConflict);
        }
        let (resource, terminal_effects) = self
            .load_resource_history(ownership_ref, resource_key_ref)
            .await?;
        let history =
            ResourceStreamView::from_verified_parts(resource.records(), &terminal_effects);
        policy
            .validate_history(history)
            .map_err(ExecutorError::ResourcePolicy)?;
        let stored = resource
            .records()
            .iter()
            .find(|record| record.effect_key() == identity.effect_key())
            .ok_or(ExecutorError::ResourceAllocationConflict)?;
        if stored != allocation_record {
            return Err(ExecutorError::ResourceAllocationConflict);
        }
        let allocation = policy
            .restore_allocation(identity, request, stored)
            .map_err(ExecutorError::ResourcePolicy)?;
        let resource_head = resource
            .head()?
            .ok_or(ExecutorError::ResourceAllocationConflict)?;
        Ok(AllocationOutcome::Existing {
            allocation,
            evidence: stored.evidence(),
            resource_head,
        })
    }

    async fn load_effect(
        &self,
        identity: &EffectIdentity,
    ) -> Result<Option<ExecutorEffectSnapshot>> {
        let snapshot = self.store.load_effect(identity.effect_key()).await?;
        if let Some(snapshot) = &snapshot {
            if snapshot.identity() != identity {
                return Err(ExecutorError::EffectBindingConflict);
            }
            self.verify_effect_snapshot(snapshot)?;
        }
        Ok(snapshot)
    }

    async fn load_resource(
        &self,
        ownership_ref: &ResourceOwnershipRef,
        resource_key_ref: &ResourceKeyRef,
    ) -> Result<ExecutorResourceSnapshot> {
        let snapshot = self
            .store
            .load_resource(ownership_ref, resource_key_ref)
            .await?;
        if snapshot.resource_ownership_ref() != ownership_ref
            || snapshot.resource_key_ref() != resource_key_ref
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        self.verify_resource_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    async fn load_resource_history(
        &self,
        ownership_ref: &ResourceOwnershipRef,
        resource_key_ref: &ResourceKeyRef,
    ) -> Result<(ExecutorResourceSnapshot, BTreeSet<EffectKey>)> {
        let resource = self.load_resource(ownership_ref, resource_key_ref).await?;
        let mut terminal_effects = BTreeSet::new();
        for record in resource.records() {
            let effect = self
                .store
                .load_effect(record.effect_key())
                .await?
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            if effect.identity().effect_key() != record.effect_key() {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            self.verify_effect_snapshot(&effect)?;
            if effect.allocation() != Some(record) {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            if fold_effect(&effect)?.terminal_tombstone().is_some() {
                terminal_effects.insert(record.effect_key().clone());
            }
        }
        Ok((resource, terminal_effects))
    }

    fn verify_effect_snapshot(&self, snapshot: &ExecutorEffectSnapshot) -> Result<()> {
        self.require_identity(snapshot.identity())?;
        if let Some(allocation) = snapshot.allocation() {
            if allocation.effect_key() != snapshot.identity().effect_key()
                || Some(allocation.resource_ownership_ref())
                    != self.binding.deployment().resource_ownership_ref()
                || ResourceKeyRef::from_reviewed(content_ref(allocation.resource_key_value())?)
                    != *allocation.resource_key_ref()
                || crate::AllocationStateRef::from_reviewed(content_ref(
                    allocation.typed_allocation_state(),
                )?) != *allocation.typed_allocation_state_ref()
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        let view = fold_effect(snapshot)?;
        view.delivery_audit()
            .verify(snapshot.identity(), &self.binding)?;
        Ok(())
    }

    async fn require_linked_allocation(&self, allocation: &ResourceLedgerRecord) -> Result<()> {
        let resource = self
            .load_resource(
                allocation.resource_ownership_ref(),
                allocation.resource_key_ref(),
            )
            .await?;
        if !resource.records().contains(allocation) {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(())
    }

    fn verify_resource_snapshot(&self, snapshot: &ExecutorResourceSnapshot) -> Result<()> {
        if Some(snapshot.resource_ownership_ref())
            != self.binding.deployment().resource_ownership_ref()
        {
            return Err(ExecutorError::ResourceOwnershipReferenceMismatch);
        }
        let mut predecessor = None;
        let mut policy_binding = None;
        let mut resource_key_value = None;
        let mut effects = BTreeSet::new();
        for record in snapshot.records() {
            if record.resource_ownership_ref() != snapshot.resource_ownership_ref()
                || record.resource_key_ref() != snapshot.resource_key_ref()
                || record.predecessor() != predecessor.as_ref()
                || ResourceKeyRef::from_reviewed(content_ref(record.resource_key_value())?)
                    != *snapshot.resource_key_ref()
                || crate::AllocationStateRef::from_reviewed(content_ref(
                    record.typed_allocation_state(),
                )?) != *record.typed_allocation_state_ref()
                || !effects.insert(record.effect_key().clone())
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            match &policy_binding {
                None => {
                    policy_binding = Some(record.policy_binding().clone());
                    resource_key_value = Some(record.resource_key_value().clone());
                }
                Some(expected)
                    if expected == record.policy_binding()
                        && resource_key_value.as_ref() == Some(record.resource_key_value()) => {}
                Some(_) => return Err(ExecutorError::ResourcePolicyNotRevalidated),
            }
            predecessor = Some(record.reference()?);
        }
        Ok(())
    }

    fn next_frontier(
        &self,
        identity: &EffectIdentity,
        existing: Option<&ExecutorEffectSnapshot>,
        records: Vec<ExecutorEvidenceRecord>,
    ) -> Result<DeliveryAuditFrontier> {
        let predecessor = existing.map(ExecutorEffectSnapshot::head).transpose()?;
        let frontier = DeliveryAuditFrontier::append(
            identity,
            predecessor,
            records,
            self.binding.deployment().evidence_authority_ref().clone(),
        )?;
        let mut complete = existing
            .map(|snapshot| snapshot.frontiers.clone())
            .unwrap_or_default();
        complete.push(frontier.clone());
        DeliveryAudit::from_ledger(complete).verify(identity, &self.binding)?;
        Ok(frontier)
    }

    fn require_identity(&self, identity: &EffectIdentity) -> Result<()> {
        if identity.executor_binding_ref() != self.binding.binding_ref() {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        if identity.tenant_scope_id() != self.binding.deployment().tenant_scope_id() {
            return Err(ExecutorError::TenantScopeMismatch);
        }
        Ok(())
    }

    fn required_resource_owner(&self) -> Result<&ResourceOwnershipRef> {
        self.binding
            .deployment()
            .resource_ownership_ref()
            .ok_or(ExecutorError::ResourceOwnershipRequired)
    }
}

fn effect_bound_record(identity: &EffectIdentity) -> ExecutorEvidenceRecord {
    ExecutorEvidenceRecord::EffectBound {
        executor_binding_ref: identity.executor_binding_ref().clone(),
        effect_key: identity.effect_key().clone(),
        request_digest: identity.request_digest().clone(),
    }
}

fn fold_effect(snapshot: &ExecutorEffectSnapshot) -> Result<EffectEntryView> {
    let delivery_audit = DeliveryAudit::from_ledger(snapshot.frontiers.clone());
    let mut tombstone = None;
    let mut allocation_projection = None;
    for record in delivery_audit.records() {
        match record {
            ExecutorEvidenceRecord::ResourceAllocated(record) => {
                if allocation_projection
                    .replace((
                        record.resource_ownership_ref(),
                        record.resource_key_ref(),
                        record.typed_allocation_state_ref(),
                        record.policy_ref(),
                        record.policy_configuration_ref(),
                        record.fencing_ref(),
                    ))
                    .is_some()
                {
                    return Err(ExecutorError::InvalidFrontier);
                }
            }
            ExecutorEvidenceRecord::TerminalTombstone(value) => {
                tombstone = Some((value.reference()?, value.clone()));
            }
            _ => {}
        }
    }
    let allocation = snapshot
        .allocation
        .as_ref()
        .map(|record| (record.resource_ownership_ref().clone(), record.evidence()));
    match (allocation_projection, allocation.as_ref()) {
        (None, None) => {}
        (
            Some((owner, key, state, policy, configuration, fence)),
            Some((stored_owner, evidence)),
        ) if owner == stored_owner
            && key == evidence.resource_key_ref()
            && state == evidence.typed_allocation_state_ref()
            && policy == evidence.policy_binding().policy_ref()
            && configuration == evidence.policy_binding().policy_configuration_ref()
            && fence == evidence.fencing_ref() => {}
        _ => return Err(ExecutorError::InvalidDurableSnapshot),
    }
    Ok(EffectEntryView::from_verified_parts(
        snapshot.identity.clone(),
        allocation,
        delivery_audit,
        tombstone,
    ))
}

fn deduplicate_content(values: &mut Vec<SchemaQualifiedCanonicalValue>) -> Result<()> {
    let mut unique = BTreeMap::<ContentRef, SchemaQualifiedCanonicalValue>::new();
    for value in values.drain(..) {
        let reference = value.reference()?;
        match unique.get(&reference) {
            Some(existing) if existing != &value => {
                return Err(ExecutorError::RetainedObjectMismatch);
            }
            Some(_) => {}
            None => {
                unique.insert(reference, value);
            }
        }
    }
    values.extend(unique.into_values());
    Ok(())
}
