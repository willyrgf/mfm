use std::collections::{BTreeMap, BTreeSet};

use mfm_capabilities::{BoundaryStage, FailureClass};
use mfm_ids::{AttemptId, ContentRef, EffectKey, RequestDigest};
use mfm_values::RetainedValueContract;

use crate::contract::{
    contract_error, recoverability_contract, validate_reference_effect_identifier,
    AllocationStateRef, EffectIdentity, ExecutorBindingRef, ExecutorRetainedClosureContract,
    FencingRef, ResourceKeyRef, ResourceOwnershipRef, SchemaQualifiedCanonicalValue,
    VerifiedExecutorBinding,
};
use crate::frontier::{
    reference_safe_failure, DeliveryAttemptOutcome, DeliveryAudit, DeliveryAuditFrontier,
    DeliveryAuditFrontierRef, ExecutorEvidenceRecord, ReferenceFailureCode, ReferenceTerminalProof,
    ResourceAllocatedRecord, ReturnedOutcome, TerminalTombstone, TerminalTombstoneRef,
    DELIVERY_FRONTIER_SCHEMA, TERMINAL_PROOF_SCHEMA, TERMINAL_TOMBSTONE_SCHEMA,
};
use crate::{ExecutorError, Result};

/// Result of one recoverable generic executor drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ensure<Evidence> {
    /// The executor has no sufficient terminal proof yet.
    Pending {
        /// Greatest complete retained delivery frontier.
        delivery_audit_ref: DeliveryAuditFrontierRef,
    },
    /// The executor has immutable terminal evidence.
    Terminal {
        /// Structurally verified evidence awaiting domain settlement.
        evidence: Evidence,
    },
}

/// Provenance established by an executor terminal claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofBasis {
    /// Domain evidence is independently verifiable.
    SelfAuthenticatingProof,
    /// The exact admitted executor evidence authority attests the claim.
    ExecutorAttestation {
        /// Exact admitted attestation authority.
        evidence_authority_ref: ContentRef,
    },
    /// The exact admitted trusted observer reports the claim.
    TrustedObserver {
        /// Exact admitted observer authority.
        evidence_authority_ref: ContentRef,
    },
}

/// Closed relation of one executor-owned retained closure member.
///
/// Runtime constructs the outer ensure-result and terminal-evidence values, so
/// neither is a legal executor-owned member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecutorRetainedValueRelation {
    /// Greatest complete delivery frontier named by the ensure result.
    DeliveryAudit,
    /// One predecessor frontier reachable from the delivery-audit head.
    ExecutorFrontier,
    /// Immutable terminal tombstone linked from the delivery audit.
    TerminalTombstone,
    /// Exact-attempt proof linked from the terminal tombstone.
    TerminalProof,
    /// Reviewed domain evidence linked from a returned observation.
    DomainEvidence,
}

/// Producer-free exact bytes and certified metadata claimed for one relation.
///
/// Constructing a member grants no retained-object or target-entry authority.
/// Only membership in a [`VerifiedExecutorRetainedClosure`] establishes that
/// the relation and contract match an exact verified executor binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExecutorRetainedValue {
    relation: ExecutorRetainedValueRelation,
    contract: RetainedValueContract,
    value: SchemaQualifiedCanonicalValue,
}

impl ExecutorRetainedValue {
    /// Constructs an unverified producer-free relation claim.
    pub fn new(
        relation: ExecutorRetainedValueRelation,
        contract: RetainedValueContract,
        value: SchemaQualifiedCanonicalValue,
    ) -> Result<Self> {
        if contract.schema_id() != value.schema_id() {
            return Err(ExecutorError::RetainedValueContractMismatch);
        }
        Ok(Self {
            relation,
            contract,
            value,
        })
    }

    /// Returns the claimed closed relation.
    pub const fn relation(&self) -> ExecutorRetainedValueRelation {
        self.relation
    }

    /// Returns the claimed retained-value contract.
    pub const fn contract(&self) -> &RetainedValueContract {
        &self.contract
    }

    /// Returns the exact schema-qualified canonical bytes.
    pub const fn value(&self) -> &SchemaQualifiedCanonicalValue {
        &self.value
    }

    /// Computes the exact lightweight content identity.
    pub fn reference(&self) -> Result<ContentRef> {
        self.value.reference()
    }
}

/// Complete untrusted executor-owned closure submitted to pure verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorRetainedClosureClaim {
    members: Vec<ExecutorRetainedValue>,
}

impl ExecutorRetainedClosureClaim {
    /// Constructs a canonical relation/content-ordered closure claim.
    pub fn new(members: impl IntoIterator<Item = ExecutorRetainedValue>) -> Result<Self> {
        let mut keyed = BTreeMap::new();
        for member in members {
            let key = (member.relation(), member.reference()?);
            if keyed.insert(key, member).is_some() {
                return Err(ExecutorError::RetainedClosureDuplicate);
            }
        }
        Ok(Self {
            members: keyed.into_values().collect(),
        })
    }

    /// Constructs the complete executor-owned claim for an in-memory audit.
    pub fn from_delivery_audit(
        audit: &DeliveryAudit,
        binding: &VerifiedExecutorBinding,
    ) -> Result<Self> {
        let contracts = binding.contract().retained_closure_contract();
        let mut members = Vec::new();
        let head_index = audit
            .frontiers()
            .len()
            .checked_sub(1)
            .ok_or(ExecutorError::InvalidFrontier)?;
        for (index, frontier) in audit.frontiers().iter().enumerate() {
            let (relation, contract) = if index == head_index {
                (
                    ExecutorRetainedValueRelation::DeliveryAudit,
                    contracts.delivery_audit_contract(),
                )
            } else {
                (
                    ExecutorRetainedValueRelation::ExecutorFrontier,
                    contracts.executor_frontier_contract(),
                )
            };
            members.push(ExecutorRetainedValue::new(
                relation,
                contract.clone(),
                SchemaQualifiedCanonicalValue::from_validated(&frontier.validated()?)?,
            )?);
            for record in frontier.appended_records() {
                match record {
                    ExecutorEvidenceRecord::DeliveryAttemptObserved {
                        outcome: DeliveryAttemptOutcome::Returned(returned),
                        ..
                    } => members.push(ExecutorRetainedValue::new(
                        ExecutorRetainedValueRelation::DomainEvidence,
                        contracts.domain_evidence_contract().clone(),
                        returned.safe_result().clone(),
                    )?),
                    ExecutorEvidenceRecord::TerminalTombstone(tombstone) => {
                        members.push(ExecutorRetainedValue::new(
                            ExecutorRetainedValueRelation::TerminalProof,
                            contracts.terminal_proof_contract().clone(),
                            SchemaQualifiedCanonicalValue::from_validated(
                                &tombstone.terminal_proof().validated()?,
                            )?,
                        )?);
                        members.push(ExecutorRetainedValue::new(
                            ExecutorRetainedValueRelation::TerminalTombstone,
                            contracts.terminal_tombstone_contract().clone(),
                            SchemaQualifiedCanonicalValue::from_validated(&tombstone.validated()?)?,
                        )?);
                    }
                    ExecutorEvidenceRecord::EffectBound { .. }
                    | ExecutorEvidenceRecord::ResourceAllocated(_)
                    | ExecutorEvidenceRecord::DeliveryAttemptAuthorized { .. }
                    | ExecutorEvidenceRecord::DeliveryAttemptObserved { .. } => {}
                }
            }
        }
        // Distinct attempts may link the same immutable evidence. The closure
        // carries one member per relation/content identity, not per graph edge.
        let mut unique = BTreeMap::new();
        for member in members {
            let key = (member.relation(), member.reference()?);
            match unique.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(member);
                }
                std::collections::btree_map::Entry::Occupied(entry) if entry.get() == &member => {}
                std::collections::btree_map::Entry::Occupied(_) => {
                    return Err(ExecutorError::RetainedObjectMismatch);
                }
            }
        }
        Ok(Self {
            members: unique.into_values().collect(),
        })
    }

    /// Iterates over every claimed member in relation/content order.
    pub fn members(&self) -> impl ExactSizeIterator<Item = &ExecutorRetainedValue> {
        self.members.iter()
    }

    /// Consumes the claim into canonical relation/content order.
    pub fn into_members(self) -> impl ExactSizeIterator<Item = ExecutorRetainedValue> {
        self.members.into_iter()
    }
}

/// Binding-qualified complete executor-owned retained closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExecutorRetainedClosure {
    executor_binding_ref: ExecutorBindingRef,
    contract: ExecutorRetainedClosureContract,
    members: Vec<ExecutorRetainedValue>,
}

impl VerifiedExecutorRetainedClosure {
    /// Returns the exact executor binding that certified every member contract.
    pub const fn executor_binding_ref(&self) -> &ExecutorBindingRef {
        &self.executor_binding_ref
    }

    /// Returns the exact descriptor-bound contract for this closure.
    pub const fn contract(&self) -> &ExecutorRetainedClosureContract {
        &self.contract
    }

    /// Returns every exact verified member in relation/content order.
    pub fn members(&self) -> impl ExactSizeIterator<Item = &ExecutorRetainedValue> {
        self.members.iter()
    }

    /// Consumes the verified closure into producer-free members.
    pub fn into_members(self) -> impl ExactSizeIterator<Item = ExecutorRetainedValue> {
        self.members.into_iter()
    }
}

struct RetainedValueIndex<'a> {
    members: BTreeMap<(ExecutorRetainedValueRelation, ContentRef), &'a ExecutorRetainedValue>,
    objects: BTreeMap<ContentRef, &'a SchemaQualifiedCanonicalValue>,
}

impl<'a> RetainedValueIndex<'a> {
    fn new(
        binding: &VerifiedExecutorBinding,
        claim: &'a ExecutorRetainedClosureClaim,
    ) -> Result<Self> {
        let contracts = binding.contract().retained_closure_contract();
        let mut members = BTreeMap::new();
        let mut objects = BTreeMap::new();
        for member in claim.members() {
            let expected_contract = match member.relation() {
                ExecutorRetainedValueRelation::DeliveryAudit => contracts.delivery_audit_contract(),
                ExecutorRetainedValueRelation::ExecutorFrontier => {
                    contracts.executor_frontier_contract()
                }
                ExecutorRetainedValueRelation::TerminalTombstone => {
                    contracts.terminal_tombstone_contract()
                }
                ExecutorRetainedValueRelation::TerminalProof => contracts.terminal_proof_contract(),
                ExecutorRetainedValueRelation::DomainEvidence => {
                    contracts.domain_evidence_contract()
                }
            };
            if member.contract() != expected_contract
                || member.value().schema_id() != expected_contract.schema_id()
            {
                return Err(ExecutorError::RetainedValueContractMismatch);
            }
            let reference = member.reference()?;
            match member.relation() {
                ExecutorRetainedValueRelation::DeliveryAudit
                | ExecutorRetainedValueRelation::ExecutorFrontier => {
                    strict_relation_schema(member.value(), DELIVERY_FRONTIER_SCHEMA)?;
                }
                ExecutorRetainedValueRelation::TerminalTombstone => {
                    strict_relation_schema(member.value(), TERMINAL_TOMBSTONE_SCHEMA)?;
                }
                ExecutorRetainedValueRelation::TerminalProof => {
                    strict_relation_schema(member.value(), TERMINAL_PROOF_SCHEMA)?;
                }
                ExecutorRetainedValueRelation::DomainEvidence => {}
            }
            if members
                .insert((member.relation(), reference.clone()), member)
                .is_some()
            {
                return Err(ExecutorError::RetainedClosureDuplicate);
            }
            if let Some(existing) = objects.insert(reference, member.value()) {
                if existing != member.value() {
                    return Err(ExecutorError::RetainedObjectMismatch);
                }
            }
        }
        Ok(Self { members, objects })
    }

    fn require(&self, reference: &ContentRef) -> Result<&'a SchemaQualifiedCanonicalValue> {
        self.objects
            .get(reference)
            .copied()
            .ok_or(ExecutorError::RetainedObjectMissing)
    }

    fn require_relation(
        &self,
        relation: ExecutorRetainedValueRelation,
        reference: &ContentRef,
    ) -> Result<&'a SchemaQualifiedCanonicalValue> {
        self.members
            .get(&(relation, reference.clone()))
            .map(|member| member.value())
            .ok_or(ExecutorError::RetainedClosureIncomplete)
    }

    fn verify_exact(
        &self,
        expected: &BTreeSet<(ExecutorRetainedValueRelation, ContentRef)>,
    ) -> Result<()> {
        let actual = self.members.keys().cloned().collect::<BTreeSet<_>>();
        if expected.difference(&actual).next().is_some() {
            return Err(ExecutorError::RetainedClosureIncomplete);
        }
        if let Some((relation, _)) = actual.difference(expected).next() {
            return Err(match relation {
                ExecutorRetainedValueRelation::DeliveryAudit
                | ExecutorRetainedValueRelation::ExecutorFrontier => ExecutorError::FrontierFork,
                ExecutorRetainedValueRelation::TerminalTombstone
                | ExecutorRetainedValueRelation::TerminalProof
                | ExecutorRetainedValueRelation::DomainEvidence => {
                    ExecutorError::RetainedClosureExtra
                }
            });
        }
        Ok(())
    }
}

fn strict_relation_schema(
    value: &SchemaQualifiedCanonicalValue,
    schema_contract: &str,
) -> Result<()> {
    let validated = recoverability_contract()?
        .strict_decode(schema_contract, value.as_bytes())
        .map_err(contract_error)?;
    if validated.schema_id() != value.schema_id() {
        return Err(ExecutorError::SchemaReferenceMismatch);
    }
    Ok(())
}

/// Untrusted generic result shape returned by an executor implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorEnsureResultClaim {
    /// No structurally sufficient terminal proof exists yet.
    Pending {
        /// Greatest complete retained delivery frontier.
        delivery_audit_ref: DeliveryAuditFrontierRef,
    },
    /// The executor claims immutable terminal evidence.
    Terminal {
        /// Untrusted evidence fields to verify against retained objects.
        evidence: Box<ExecutorTerminalEvidenceClaim>,
    },
}

impl ExecutorEnsureResultClaim {
    /// Constructs a pending claim naming the complete audit head.
    pub fn pending(delivery_audit_ref: DeliveryAuditFrontierRef) -> Self {
        Self::Pending { delivery_audit_ref }
    }

    /// Constructs a terminal claim.
    pub fn terminal(evidence: ExecutorTerminalEvidenceClaim) -> Self {
        Self::Terminal {
            evidence: Box::new(evidence),
        }
    }
}

/// Untrusted pre-journal terminal evidence fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorTerminalEvidenceClaim {
    identity: EffectIdentity,
    delivery_audit_ref: DeliveryAuditFrontierRef,
    terminal_tombstone_ref: TerminalTombstoneRef,
    external_operation_identity: String,
    terminal_outcome: String,
    assurance_policy_ref: ContentRef,
    proof_basis: ProofBasis,
    domain_evidence_ref: ContentRef,
}

impl ExecutorTerminalEvidenceClaim {
    /// Constructs a terminal claim without granting it positive meaning.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: EffectIdentity,
        delivery_audit_ref: DeliveryAuditFrontierRef,
        terminal_tombstone_ref: TerminalTombstoneRef,
        external_operation_identity: impl Into<String>,
        terminal_outcome: impl Into<String>,
        assurance_policy_ref: ContentRef,
        proof_basis: ProofBasis,
        domain_evidence_ref: ContentRef,
    ) -> Result<Self> {
        let external_operation_identity = external_operation_identity.into();
        let terminal_outcome = terminal_outcome.into();
        validate_reference_effect_identifier(&external_operation_identity)?;
        validate_reference_effect_identifier(&terminal_outcome)?;
        Ok(Self {
            identity,
            delivery_audit_ref,
            terminal_tombstone_ref,
            external_operation_identity,
            terminal_outcome,
            assurance_policy_ref,
            proof_basis,
            domain_evidence_ref,
        })
    }

    /// Returns the claimed immutable effect identity.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.identity
    }

    /// Returns the claimed complete audit head.
    pub const fn delivery_audit_ref(&self) -> &DeliveryAuditFrontierRef {
        &self.delivery_audit_ref
    }

    /// Returns the claimed terminal tombstone.
    pub const fn terminal_tombstone_ref(&self) -> &TerminalTombstoneRef {
        &self.terminal_tombstone_ref
    }

    /// Returns the destination-native operation identity.
    pub fn external_operation_identity(&self) -> &str {
        &self.external_operation_identity
    }

    /// Returns the claimed terminal outcome.
    pub fn terminal_outcome(&self) -> &str {
        &self.terminal_outcome
    }

    /// Returns the claimed assurance policy.
    pub const fn assurance_policy_ref(&self) -> &ContentRef {
        &self.assurance_policy_ref
    }

    /// Returns the claimed proof provenance.
    pub const fn proof_basis(&self) -> &ProofBasis {
        &self.proof_basis
    }

    /// Returns the exact domain-evidence object identity.
    pub const fn domain_evidence_ref(&self) -> &ContentRef {
        &self.domain_evidence_ref
    }
}

/// Structurally verified terminal evidence awaiting domain finality.
///
/// This proves the generic frontier, tombstone, exact returned observation,
/// result identity, and admitted evidence authority. The effect's pure domain
/// callback remains solely responsible for deciding settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTerminalEvidence {
    claim: ExecutorTerminalEvidenceClaim,
    tombstone: TerminalTombstone,
    terminal_proof: ReferenceTerminalProof,
    domain_evidence: SchemaQualifiedCanonicalValue,
}

impl VerifiedTerminalEvidence {
    /// Returns the verified pre-journal claim fields.
    pub const fn claim(&self) -> &ExecutorTerminalEvidenceClaim {
        &self.claim
    }

    /// Returns the exact verified terminal tombstone.
    pub const fn terminal_tombstone(&self) -> &TerminalTombstone {
        &self.tombstone
    }

    /// Returns the exact-attempt generic v1 terminal proof.
    pub const fn terminal_proof(&self) -> &ReferenceTerminalProof {
        &self.terminal_proof
    }

    /// Returns the exact schema-qualified domain evidence.
    pub const fn domain_evidence(&self) -> &SchemaQualifiedCanonicalValue {
        &self.domain_evidence
    }
}

/// Generic verified executor result plus one exact binding-qualified closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEnsureResult {
    fields: Box<VerifiedEnsureResultFields>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedEnsureResultFields {
    identity: EffectIdentity,
    outcome: Ensure<VerifiedTerminalEvidence>,
    delivery_audit: DeliveryAudit,
    retained_closure: VerifiedExecutorRetainedClosure,
}

/// Closed surviving outcome of one audited executor `ensure` invocation.
///
/// Integrity failures and unresolved append ambiguity remain typed errors. Every reviewed
/// returned or safe boundary failure is instead an observation-bearing value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectExecutorOutcome {
    kind: EffectExecutorOutcomeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EffectExecutorOutcomeKind {
    Returned(VerifiedEnsureResult),
    DidNotEnter(crate::ReferenceSafeFailure),
    Indeterminate(crate::ReferenceSafeFailure),
}

/// Borrowed closed view of one surviving executor outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectExecutorOutcomeView<'a> {
    /// The executor returned one binding-verified pending or terminal result.
    Returned(&'a VerifiedEnsureResult),
    /// The executor proved that target boundary entry did not occur.
    DidNotEnter(&'a crate::ReferenceSafeFailure),
    /// Target boundary entry or the terminal outcome remains indeterminate.
    Indeterminate(&'a crate::ReferenceSafeFailure),
}

impl EffectExecutorOutcome {
    /// Wraps one binding-verified returned executor result.
    pub const fn returned(result: VerifiedEnsureResult) -> Self {
        Self {
            kind: EffectExecutorOutcomeKind::Returned(result),
        }
    }

    /// Constructs one outcome-specific proven-not-entered failure.
    pub fn did_not_enter(failure: crate::ReferenceSafeFailure) -> Result<Self> {
        DeliveryAttemptOutcome::did_not_enter(failure.clone())?;
        Ok(Self {
            kind: EffectExecutorOutcomeKind::DidNotEnter(failure),
        })
    }

    /// Constructs one outcome-specific indeterminate failure.
    pub fn indeterminate(failure: crate::ReferenceSafeFailure) -> Result<Self> {
        DeliveryAttemptOutcome::indeterminate(failure.clone())?;
        Ok(Self {
            kind: EffectExecutorOutcomeKind::Indeterminate(failure),
        })
    }

    /// Returns the borrowed closed outcome.
    pub const fn view(&self) -> EffectExecutorOutcomeView<'_> {
        match &self.kind {
            EffectExecutorOutcomeKind::Returned(result) => {
                EffectExecutorOutcomeView::Returned(result)
            }
            EffectExecutorOutcomeKind::DidNotEnter(failure) => {
                EffectExecutorOutcomeView::DidNotEnter(failure)
            }
            EffectExecutorOutcomeKind::Indeterminate(failure) => {
                EffectExecutorOutcomeView::Indeterminate(failure)
            }
        }
    }

    /// Splits the outcome into a returned result or one validated safe-failure tuple.
    pub fn into_parts(
        self,
    ) -> std::result::Result<
        VerifiedEnsureResult,
        (
            mfm_capabilities::SafeFailureOutcome,
            crate::ReferenceSafeFailure,
        ),
    > {
        match self.kind {
            EffectExecutorOutcomeKind::Returned(result) => Ok(result),
            EffectExecutorOutcomeKind::DidNotEnter(failure) => {
                Err((mfm_capabilities::SafeFailureOutcome::DidNotEnter, failure))
            }
            EffectExecutorOutcomeKind::Indeterminate(failure) => {
                Err((mfm_capabilities::SafeFailureOutcome::Indeterminate, failure))
            }
        }
    }
}

impl VerifiedEnsureResult {
    /// Returns the exact immutable effect identity.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.fields.identity
    }

    /// Returns the structurally verified pending or terminal outcome.
    pub const fn outcome(&self) -> &Ensure<VerifiedTerminalEvidence> {
        &self.fields.outcome
    }

    /// Returns the complete verified delivery audit.
    pub const fn delivery_audit(&self) -> &DeliveryAudit {
        &self.fields.delivery_audit
    }

    /// Returns the one binding-qualified complete producer-free closure.
    pub const fn retained_closure(&self) -> &VerifiedExecutorRetainedClosure {
        &self.fields.retained_closure
    }

    /// Splits the verified result for runtime's sole journal promotion.
    pub fn into_parts(
        self,
    ) -> (
        EffectIdentity,
        Ensure<VerifiedTerminalEvidence>,
        DeliveryAudit,
        VerifiedExecutorRetainedClosure,
    ) {
        let VerifiedEnsureResultFields {
            identity,
            outcome,
            delivery_audit,
            retained_closure,
        } = *self.fields;
        (identity, outcome, delivery_audit, retained_closure)
    }
}

/// Purely verifies a generic claim from exact retained objects.
///
/// No live executor, destination, callback, filesystem, or network authority
/// is consulted.
pub fn verify_ensure_result(
    identity: EffectIdentity,
    binding: &VerifiedExecutorBinding,
    claim: ExecutorEnsureResultClaim,
    retained_closure: ExecutorRetainedClosureClaim,
) -> Result<VerifiedEnsureResult> {
    if identity.executor_binding_ref() != binding.binding_ref() {
        return Err(ExecutorError::WrongExecutorBinding);
    }
    if identity.tenant_scope_id() != binding.deployment().tenant_scope_id() {
        return Err(ExecutorError::TenantScopeMismatch);
    }
    let head = match &claim {
        ExecutorEnsureResultClaim::Pending { delivery_audit_ref } => delivery_audit_ref,
        ExecutorEnsureResultClaim::Terminal { evidence } => {
            if evidence.identity() != &identity {
                return Err(ExecutorError::TerminalProofMismatch);
            }
            evidence.delivery_audit_ref()
        }
    };
    let index = RetainedValueIndex::new(binding, &retained_closure)?;
    let (delivery_audit, expected_members) =
        verify_retained_delivery_audit_index(&identity, binding, head.as_content_ref(), &index)?;
    let outcome = match claim {
        ExecutorEnsureResultClaim::Pending { delivery_audit_ref } => {
            if audit_tombstone(&delivery_audit).is_some() {
                return Err(ExecutorError::TerminalTombstoneConflict);
            }
            Ensure::Pending { delivery_audit_ref }
        }
        ExecutorEnsureResultClaim::Terminal { evidence } => {
            let verified = verify_retained_terminal_evidence_index(
                binding,
                &delivery_audit,
                *evidence,
                &index,
            )?;
            Ensure::Terminal { evidence: verified }
        }
    };
    index.verify_exact(&expected_members)?;
    drop(index);
    Ok(VerifiedEnsureResult {
        fields: Box::new(VerifiedEnsureResultFields {
            identity,
            outcome,
            delivery_audit,
            retained_closure: VerifiedExecutorRetainedClosure {
                executor_binding_ref: binding.binding_ref().clone(),
                contract: binding.contract().retained_closure_contract().clone(),
                members: retained_closure.into_members().collect(),
            },
        }),
    })
}

/// Purely reconstructs and verifies one predecessor-linked retained audit.
pub fn verify_retained_delivery_audit(
    identity: &EffectIdentity,
    binding: &VerifiedExecutorBinding,
    head_ref: &ContentRef,
    retained_closure: &ExecutorRetainedClosureClaim,
) -> Result<DeliveryAudit> {
    let index = RetainedValueIndex::new(binding, retained_closure)?;
    let (audit, expected_members) =
        verify_retained_delivery_audit_index(identity, binding, head_ref, &index)?;
    index.verify_exact(&expected_members)?;
    Ok(audit)
}

fn verify_retained_delivery_audit_index(
    identity: &EffectIdentity,
    binding: &VerifiedExecutorBinding,
    head_ref: &ContentRef,
    index: &RetainedValueIndex<'_>,
) -> Result<(
    DeliveryAudit,
    BTreeSet<(ExecutorRetainedValueRelation, ContentRef)>,
)> {
    let mut current = Some(head_ref.clone());
    let mut seen = BTreeSet::new();
    let mut reversed = Vec::new();
    let mut expected_members = BTreeSet::new();
    let mut relation = ExecutorRetainedValueRelation::DeliveryAudit;
    while let Some(reference) = current {
        if !seen.insert(reference.clone()) {
            return Err(ExecutorError::FrontierFork);
        }
        if reversed.len() >= binding.contract().evidence_bounds().max_records() as usize {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        index.require_relation(relation, &reference)?;
        expected_members.insert((relation, reference.clone()));
        let frontier = decode_frontier(identity, &reference, index)?;
        current = frontier
            .predecessor_frontier_ref()
            .map(|value| value.as_content_ref().clone());
        reversed.push(frontier);
        relation = ExecutorRetainedValueRelation::ExecutorFrontier;
    }
    reversed.reverse();
    let audit = DeliveryAudit::from_ledger(reversed);
    audit.verify(
        identity,
        binding.contract().evidence_bounds(),
        binding.deployment().evidence_authority_ref(),
    )?;
    verify_contract_records(&audit, binding)?;
    append_linked_members(&audit, &mut expected_members)?;
    Ok((audit, expected_members))
}

/// Purely verifies a terminal tombstone, its frozen generic v1 exact-attempt
/// proof, returned domain evidence, and admitted proof authority.
pub fn verify_retained_terminal_evidence(
    binding: &VerifiedExecutorBinding,
    audit: &DeliveryAudit,
    claim: ExecutorTerminalEvidenceClaim,
    retained_closure: &ExecutorRetainedClosureClaim,
) -> Result<VerifiedTerminalEvidence> {
    if claim.identity().executor_binding_ref() != binding.binding_ref() {
        return Err(ExecutorError::WrongExecutorBinding);
    }
    if claim.identity().tenant_scope_id() != binding.deployment().tenant_scope_id() {
        return Err(ExecutorError::TenantScopeMismatch);
    }
    audit.verify(
        claim.identity(),
        binding.contract().evidence_bounds(),
        binding.deployment().evidence_authority_ref(),
    )?;
    verify_contract_records(audit, binding)?;
    let index = RetainedValueIndex::new(binding, retained_closure)?;
    let head = audit.head_ref()?;
    if claim.delivery_audit_ref() != &head {
        return Err(ExecutorError::TerminalProofMismatch);
    }
    let expected_members = expected_members_for_audit(audit)?;
    let verified = verify_retained_terminal_evidence_index(binding, audit, claim, &index)?;
    index.verify_exact(&expected_members)?;
    Ok(verified)
}

fn verify_retained_terminal_evidence_index(
    binding: &VerifiedExecutorBinding,
    audit: &DeliveryAudit,
    claim: ExecutorTerminalEvidenceClaim,
    index: &RetainedValueIndex<'_>,
) -> Result<VerifiedTerminalEvidence> {
    index.require_relation(
        ExecutorRetainedValueRelation::TerminalTombstone,
        claim.terminal_tombstone_ref().as_content_ref(),
    )?;
    let tombstone_value = strict_fixed(
        claim.terminal_tombstone_ref().as_content_ref(),
        TERMINAL_TOMBSTONE_SCHEMA,
        index,
    )?;
    let tombstone = decode_tombstone(tombstone_value.as_bytes(), index)?;
    if tombstone.reference()? != *claim.terminal_tombstone_ref()
        || tombstone.external_operation_identity() != claim.external_operation_identity()
        || tombstone.terminal_outcome() != claim.terminal_outcome()
    {
        return Err(ExecutorError::TerminalTombstoneConflict);
    }
    let matching_tombstone = audit.frontiers().iter().any(|frontier| {
        frontier.appended_records().iter().any(|record| {
            matches!(
                record,
                ExecutorEvidenceRecord::TerminalTombstone(candidate)
                    if candidate == &tombstone
            )
        })
    });
    if !matching_tombstone {
        return Err(ExecutorError::TerminalTombstoneConflict);
    }
    let proof = tombstone.terminal_proof().clone();
    if proof.returned_outcome().safe_result_ref() != claim.domain_evidence_ref() {
        return Err(ExecutorError::TerminalProofMismatch);
    }
    index.require_relation(
        ExecutorRetainedValueRelation::DomainEvidence,
        claim.domain_evidence_ref(),
    )?;
    let domain_evidence = index.require(claim.domain_evidence_ref())?.clone();
    if domain_evidence.reference()? != *claim.domain_evidence_ref()
        || &domain_evidence != proof.returned_outcome().safe_result()
    {
        return Err(ExecutorError::RetainedObjectMismatch);
    }
    let expected_authority = binding.deployment().evidence_authority_ref();
    match claim.proof_basis() {
        ProofBasis::SelfAuthenticatingProof => {}
        ProofBasis::ExecutorAttestation {
            evidence_authority_ref,
        }
        | ProofBasis::TrustedObserver {
            evidence_authority_ref,
        } if evidence_authority_ref == expected_authority => {}
        _ => return Err(ExecutorError::InvalidFrontierProof),
    }
    Ok(VerifiedTerminalEvidence {
        claim,
        tombstone,
        terminal_proof: proof,
        domain_evidence,
    })
}

fn expected_members_for_audit(
    audit: &DeliveryAudit,
) -> Result<BTreeSet<(ExecutorRetainedValueRelation, ContentRef)>> {
    let mut expected = BTreeSet::new();
    let head_index = audit
        .frontiers()
        .len()
        .checked_sub(1)
        .ok_or(ExecutorError::InvalidFrontier)?;
    for (index, frontier) in audit.frontiers().iter().enumerate() {
        let relation = if index == head_index {
            ExecutorRetainedValueRelation::DeliveryAudit
        } else {
            ExecutorRetainedValueRelation::ExecutorFrontier
        };
        expected.insert((relation, frontier.reference()?.as_content_ref().clone()));
    }
    append_linked_members(audit, &mut expected)?;
    Ok(expected)
}

fn append_linked_members(
    audit: &DeliveryAudit,
    expected: &mut BTreeSet<(ExecutorRetainedValueRelation, ContentRef)>,
) -> Result<()> {
    for record in audit.records() {
        match record {
            ExecutorEvidenceRecord::DeliveryAttemptObserved {
                outcome: DeliveryAttemptOutcome::Returned(returned),
                ..
            } => {
                expected.insert((
                    ExecutorRetainedValueRelation::DomainEvidence,
                    returned.safe_result_ref().clone(),
                ));
            }
            ExecutorEvidenceRecord::TerminalTombstone(tombstone) => {
                expected.insert((
                    ExecutorRetainedValueRelation::TerminalTombstone,
                    tombstone.reference()?.as_content_ref().clone(),
                ));
                expected.insert((
                    ExecutorRetainedValueRelation::TerminalProof,
                    tombstone.terminal_proof().reference()?,
                ));
                expected.insert((
                    ExecutorRetainedValueRelation::DomainEvidence,
                    tombstone
                        .terminal_proof()
                        .returned_outcome()
                        .safe_result_ref()
                        .clone(),
                ));
            }
            ExecutorEvidenceRecord::EffectBound { .. }
            | ExecutorEvidenceRecord::ResourceAllocated(_)
            | ExecutorEvidenceRecord::DeliveryAttemptAuthorized { .. }
            | ExecutorEvidenceRecord::DeliveryAttemptObserved { .. } => {}
        }
    }
    Ok(())
}

fn audit_tombstone(audit: &DeliveryAudit) -> Option<&TerminalTombstone> {
    audit.frontiers().iter().find_map(|frontier| {
        frontier.appended_records().iter().find_map(|record| {
            if let ExecutorEvidenceRecord::TerminalTombstone(tombstone) = record {
                Some(tombstone)
            } else {
                None
            }
        })
    })
}

fn verify_contract_records(audit: &DeliveryAudit, binding: &VerifiedExecutorBinding) -> Result<()> {
    for record in audit.records() {
        match record {
            ExecutorEvidenceRecord::ResourceAllocated(record) => {
                let expected = binding
                    .resource_ownership()
                    .ok_or(ExecutorError::ResourceOwnershipRequired)?
                    .reference()?;
                if binding.contract().resource_domain_requirement().is_none()
                    || record.resource_ownership_ref() != &expected
                {
                    return Err(ExecutorError::ResourceDomainMismatch);
                }
            }
            ExecutorEvidenceRecord::DeliveryAttemptObserved { outcome, .. } => {
                let failure = match outcome {
                    DeliveryAttemptOutcome::DidNotEnter(failure)
                    | DeliveryAttemptOutcome::Indeterminate(failure) => Some(failure),
                    DeliveryAttemptOutcome::Returned(_) => None,
                };
                if failure.is_some_and(|failure| {
                    failure.safe_failure_contract_ref()
                        != binding.contract().safe_failure_contract_ref()
                }) {
                    return Err(ExecutorError::InvalidSafeFailure);
                }
            }
            ExecutorEvidenceRecord::EffectBound { .. }
            | ExecutorEvidenceRecord::DeliveryAttemptAuthorized { .. }
            | ExecutorEvidenceRecord::TerminalTombstone(_) => {}
        }
    }
    Ok(())
}

fn decode_frontier(
    admitted_identity: &EffectIdentity,
    reference: &ContentRef,
    index: &RetainedValueIndex<'_>,
) -> Result<DeliveryAuditFrontier> {
    let retained = strict_fixed(reference, DELIVERY_FRONTIER_SCHEMA, index)?;
    let json = parse_json(retained.as_bytes())?;
    let binding_ref =
        ExecutorBindingRef::from_content_ref(content_field(&json, "executor_binding_ref")?)?;
    let effect_key = EffectKey::parse(string_field(&json, "effect_key")?)
        .map_err(|_| ExecutorError::CanonicalEncoding)?;
    let request_digest = RequestDigest::parse(string_field(&json, "request_digest")?)
        .map_err(|_| ExecutorError::CanonicalEncoding)?;
    let predecessor = json
        .get("predecessor_frontier_ref")
        .map(decode_content_ref)
        .transpose()?
        .map(DeliveryAuditFrontierRef::from_content_ref)
        .transpose()?;
    let proof_ref = content_field(&json, "proof_ref")?;
    let records = array_field(&json, "appended_records")?
        .iter()
        .map(|record| decode_record(record, index))
        .collect::<Result<Vec<_>>>()?;
    let retained_identity = EffectIdentity::from_parts(
        admitted_identity.tenant_scope_id().clone(),
        binding_ref,
        effect_key,
        request_digest,
    );
    let frontier =
        DeliveryAuditFrontier::append(&retained_identity, predecessor, records, proof_ref)?;
    if frontier.reference()?.as_content_ref() != reference
        || frontier.validated()?.as_bytes() != retained.as_bytes()
    {
        return Err(ExecutorError::RetainedObjectMismatch);
    }
    Ok(frontier)
}

fn decode_record(
    wrapper: &serde_json::Value,
    index: &RetainedValueIndex<'_>,
) -> Result<ExecutorEvidenceRecord> {
    let kind = string_field(wrapper, "kind")?;
    let record = wrapper
        .get("record")
        .ok_or(ExecutorError::CanonicalEncoding)?;
    match kind {
        "effect_bound" => Ok(ExecutorEvidenceRecord::EffectBound {
            executor_binding_ref: ExecutorBindingRef::from_content_ref(content_field(
                record,
                "executor_binding_ref",
            )?)?,
            effect_key: EffectKey::parse(string_field(record, "effect_key")?)
                .map_err(|_| ExecutorError::CanonicalEncoding)?,
            request_digest: RequestDigest::parse(string_field(record, "request_digest")?)
                .map_err(|_| ExecutorError::CanonicalEncoding)?,
        }),
        "resource_allocated" => Ok(ExecutorEvidenceRecord::ResourceAllocated(Box::new(
            ResourceAllocatedRecord::new(
                ResourceOwnershipRef::from_content_ref(content_field(
                    record,
                    "resource_ownership_ref",
                )?)?,
                ResourceKeyRef::from_reviewed(content_field(record, "resource_key_ref")?),
                AllocationStateRef::from_reviewed(content_field(
                    record,
                    "typed_allocation_state_ref",
                )?),
                content_field(record, "policy_ref")?,
                content_field(record, "policy_configuration_ref")?,
                record
                    .get("fencing_ref")
                    .map(decode_content_ref)
                    .transpose()?
                    .map(FencingRef::from_reviewed),
            ),
        ))),
        "delivery_attempt_authorized" => Ok(ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: u32::try_from(unsigned_field(record, "attempt_ordinal")?)
                .map_err(|_| ExecutorError::CanonicalEncoding)?,
            attempt_id: AttemptId::parse(string_field(record, "attempt_id")?)
                .map_err(|_| ExecutorError::CanonicalEncoding)?,
            target_operation_ref: content_field(record, "target_operation_ref")?,
        }),
        "delivery_attempt_observed" => Ok(ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id: AttemptId::parse(string_field(record, "attempt_id")?)
                .map_err(|_| ExecutorError::CanonicalEncoding)?,
            outcome: decode_outcome(
                record
                    .get("outcome")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
                index,
            )?,
        }),
        "terminal_tombstone" => Ok(ExecutorEvidenceRecord::TerminalTombstone(
            decode_tombstone_value(record, index)?,
        )),
        _ => Err(ExecutorError::CanonicalEncoding),
    }
}

fn decode_outcome(
    value: &serde_json::Value,
    index: &RetainedValueIndex<'_>,
) -> Result<DeliveryAttemptOutcome> {
    match string_field(value, "kind")? {
        "returned" => {
            let returned = value
                .get("returned_outcome")
                .ok_or(ExecutorError::CanonicalEncoding)?;
            let safe_result_ref = content_field(returned, "safe_result_ref")?;
            index.require_relation(
                ExecutorRetainedValueRelation::DomainEvidence,
                &safe_result_ref,
            )?;
            let safe_result = index.require(&safe_result_ref)?.clone();
            if safe_result.reference()? != safe_result_ref {
                return Err(ExecutorError::RetainedObjectMismatch);
            }
            DeliveryAttemptOutcome::returned(safe_result)
        }
        "did_not_enter" => DeliveryAttemptOutcome::did_not_enter(decode_safe_failure(
            value
                .get("safe_failure")
                .ok_or(ExecutorError::CanonicalEncoding)?,
        )?),
        "indeterminate" => DeliveryAttemptOutcome::indeterminate(decode_safe_failure(
            value
                .get("safe_failure")
                .ok_or(ExecutorError::CanonicalEncoding)?,
        )?),
        _ => Err(ExecutorError::CanonicalEncoding),
    }
}

fn decode_safe_failure(value: &serde_json::Value) -> Result<crate::ReferenceSafeFailure> {
    if !value
        .get("coarse_size_class")
        .is_some_and(serde_json::Value::is_null)
        || !value
            .get("diagnostic_ref")
            .is_some_and(serde_json::Value::is_null)
    {
        return Err(ExecutorError::InvalidSafeFailure);
    }
    let code = match string_field(value, "stable_code")? {
        "generation_fenced" => ReferenceFailureCode::GenerationFenced,
        "destination_unavailable" => ReferenceFailureCode::DestinationUnavailable,
        "request_conflict" => ReferenceFailureCode::RequestConflict,
        "access_cancelled" => ReferenceFailureCode::AccessCancelled,
        "unclassified_failure" => ReferenceFailureCode::UnclassifiedFailure,
        _ => return Err(ExecutorError::InvalidSafeFailure),
    };
    let class = match string_field(value, "failure_class")? {
        "authorization" => FailureClass::Authorization,
        "transport" => FailureClass::Transport,
        "destination" => FailureClass::Destination,
        "cancellation" => FailureClass::Cancellation,
        "unclassified" => FailureClass::Unclassified,
        _ => return Err(ExecutorError::InvalidSafeFailure),
    };
    let stage = match string_field(value, "boundary_stage")? {
        "before_boundary_entry" => BoundaryStage::BeforeBoundaryEntry,
        "boundary_entry" => BoundaryStage::BoundaryEntry,
        "boundary_observation" => BoundaryStage::BoundaryObservation,
        _ => return Err(ExecutorError::InvalidSafeFailure),
    };
    reference_safe_failure(
        content_field(value, "safe_failure_contract_ref")?,
        code,
        class,
        stage,
    )
}

fn decode_tombstone(bytes: &[u8], index: &RetainedValueIndex<'_>) -> Result<TerminalTombstone> {
    decode_tombstone_value(&parse_json(bytes)?, index)
}

fn decode_tombstone_value(
    value: &serde_json::Value,
    index: &RetainedValueIndex<'_>,
) -> Result<TerminalTombstone> {
    let proof_ref = content_field(value, "terminal_proof_ref")?;
    index.require_relation(ExecutorRetainedValueRelation::TerminalProof, &proof_ref)?;
    let proof_value = strict_fixed(&proof_ref, TERMINAL_PROOF_SCHEMA, index)?;
    let proof_json = parse_json(proof_value.as_bytes())?;
    let returned = proof_json
        .get("returned_outcome")
        .ok_or(ExecutorError::CanonicalEncoding)?;
    let safe_result_ref = content_field(returned, "safe_result_ref")?;
    index.require_relation(
        ExecutorRetainedValueRelation::DomainEvidence,
        &safe_result_ref,
    )?;
    let safe_result = index.require(&safe_result_ref)?.clone();
    let returned_outcome = ReturnedOutcome::new(safe_result)?;
    if returned_outcome.safe_result_ref() != &safe_result_ref {
        return Err(ExecutorError::RetainedObjectMismatch);
    }
    let proof = ReferenceTerminalProof::new(
        AttemptId::parse(string_field(&proof_json, "attempt_id")?)
            .map_err(|_| ExecutorError::CanonicalEncoding)?,
        returned_outcome,
        content_field(&proof_json, "returned_observation_ref")?,
    )?;
    if proof.reference()? != proof_ref || proof.validated()?.as_bytes() != proof_value.as_bytes() {
        return Err(ExecutorError::TerminalProofMismatch);
    }
    TerminalTombstone::new(
        string_field(value, "external_operation_identity")?,
        string_field(value, "terminal_outcome")?,
        proof,
    )
}

fn strict_fixed<'a>(
    reference: &ContentRef,
    schema_contract: &str,
    index: &'a RetainedValueIndex<'_>,
) -> Result<&'a SchemaQualifiedCanonicalValue> {
    let value = index.require(reference)?;
    if value.reference()? != *reference {
        return Err(ExecutorError::RetainedObjectMismatch);
    }
    let validated = recoverability_contract()?
        .strict_decode(schema_contract, value.as_bytes())
        .map_err(contract_error)?;
    if validated.schema_id() != reference.schema_id() {
        return Err(ExecutorError::SchemaReferenceMismatch);
    }
    Ok(value)
}

fn parse_json(bytes: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(bytes).map_err(|_| ExecutorError::CanonicalEncoding)
}

fn string_field<'a>(value: &'a serde_json::Value, name: &str) -> Result<&'a str> {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or(ExecutorError::CanonicalEncoding)
}

fn unsigned_field(value: &serde_json::Value, name: &str) -> Result<u64> {
    value
        .get(name)
        .and_then(serde_json::Value::as_u64)
        .ok_or(ExecutorError::CanonicalEncoding)
}

fn array_field<'a>(value: &'a serde_json::Value, name: &str) -> Result<&'a Vec<serde_json::Value>> {
    value
        .get(name)
        .and_then(serde_json::Value::as_array)
        .ok_or(ExecutorError::CanonicalEncoding)
}

fn content_field(value: &serde_json::Value, name: &str) -> Result<ContentRef> {
    decode_content_ref(value.get(name).ok_or(ExecutorError::CanonicalEncoding)?)
}

fn decode_content_ref(value: &serde_json::Value) -> Result<ContentRef> {
    serde_json::from_value(value.clone()).map_err(|_| ExecutorError::CanonicalEncoding)
}
