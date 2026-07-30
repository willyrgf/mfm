use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{BoundaryStage, CoarseSizeClass, FailureClass};
use mfm_executor::EffectExecutorOutcome;
use mfm_facts::FactProposal;
use mfm_ids::{ContentRef, EffectKey, FieldPath, NodeId, RequestDigest, StableId, TenantScopeId};
use mfm_journal::{
    AuthorizationRef, CapabilityBindingRef, CrossRunSourceRef, InputManifest, InputManifestRef,
    JournalHead, ObservationRef, TransitionRef, ValueRef,
};
use mfm_spec::{CertifiedAdmissionArtifacts, RetainedValueContract};

use super::objects::PreparedAuthority;
use super::{
    AdmittedSupportGraph, Result, StoreAuthorityContext, StoreError, VerifiedConfiguredValue,
};

/// Producer-free canonical entry-point input proposed for one admission.
pub struct ProposedAdmissionInput {
    canonical: PlainCanonicalJsonBytes,
    value_contract: RetainedValueContract,
}

impl ProposedAdmissionInput {
    /// Constructs one exact producer-free admission input.
    pub const fn new(
        canonical: PlainCanonicalJsonBytes,
        value_contract: RetainedValueContract,
    ) -> Self {
        Self {
            canonical,
            value_contract,
        }
    }

    /// Returns exact canonical input bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the complete certified input contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }
}

pub(super) struct VerifiedAdmissionSource {
    pub(super) source: CrossRunSourceRef,
    pub(super) value_ref: ValueRef,
    pub(super) bytes: Vec<u8>,
    pub(super) value_contract: RetainedValueContract,
    pub(super) source_role_ref: ContentRef,
}

/// Store-sealed exact cross-run source set for one admission.
///
/// The token is non-cloneable and has no public constructor. An empty set must still be minted by
/// the target store so admission never treats the absence of sources as an authority bypass.
pub struct VerifiedAdmissionSources {
    pub(super) authority: StoreAuthorityContext,
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) entries: BTreeMap<FieldPath, VerifiedAdmissionSource>,
}

impl VerifiedAdmissionSources {
    pub(super) const fn empty(
        authority: StoreAuthorityContext,
        tenant_scope_id: TenantScopeId,
    ) -> Self {
        Self {
            authority,
            tenant_scope_id,
            entries: BTreeMap::new(),
        }
    }

    /// Returns whether this exact verified source set is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of exact verified direct source roots.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Complete producer-free material accepted by store-owned admission preparation.
pub struct AdmissionMaterial<'a> {
    artifacts: CertifiedAdmissionArtifacts,
    input: ProposedAdmissionInput,
    configured: &'a VerifiedConfiguredValue,
    support: &'a AdmittedSupportGraph,
    sources: &'a VerifiedAdmissionSources,
}

impl<'a> AdmissionMaterial<'a> {
    /// Constructs one complete admission proposal from sealed prerequisite resolutions.
    pub const fn new(
        artifacts: CertifiedAdmissionArtifacts,
        input: ProposedAdmissionInput,
        configured: &'a VerifiedConfiguredValue,
        support: &'a AdmittedSupportGraph,
        sources: &'a VerifiedAdmissionSources,
    ) -> Self {
        Self {
            artifacts,
            input,
            configured,
            support,
            sources,
        }
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        CertifiedAdmissionArtifacts,
        ProposedAdmissionInput,
        &'a VerifiedConfiguredValue,
        &'a AdmittedSupportGraph,
        &'a VerifiedAdmissionSources,
    ) {
        (
            self.artifacts,
            self.input,
            self.configured,
            self.support,
            self.sources,
        )
    }
}

/// Producer-free canonical root proposed by runtime callback material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedObjectRoot {
    value_contract: RetainedValueContract,
    canonical: PlainCanonicalJsonBytes,
}

impl ProducedObjectRoot {
    /// Constructs one producer-free retained root.
    pub const fn new(
        value_contract: RetainedValueContract,
        canonical: PlainCanonicalJsonBytes,
    ) -> Self {
        Self {
            value_contract,
            canonical,
        }
    }

    /// Returns the complete certified producer-independent contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns exact canonical root bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }
}

/// One named producer-free member of a recursive object-graph proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectGraphMember {
    field_path: FieldPath,
    root: ProducedObjectRoot,
}

impl ObjectGraphMember {
    /// Constructs one exact named proposal.
    pub const fn new(field_path: FieldPath, root: ProducedObjectRoot) -> Self {
        Self { field_path, root }
    }

    /// Returns the stable producer field path.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns producer-free retained material.
    pub const fn root(&self) -> &ProducedObjectRoot {
        &self.root
    }
}

/// Complete producer-free recursive object-graph material supplied to store preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectGraphProposal {
    members: Vec<ObjectGraphMember>,
}

impl ObjectGraphProposal {
    /// Constructs a canonically ordered graph with unique named roots.
    pub fn new(mut members: Vec<ObjectGraphMember>) -> Result<Self> {
        members.sort_by(|left, right| left.field_path.cmp(&right.field_path));
        if members
            .windows(2)
            .any(|pair| pair[0].field_path == pair[1].field_path)
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "object graph contains a duplicate producer field path",
            });
        }
        Ok(Self { members })
    }

    /// Returns the empty producer-free graph.
    pub const fn empty() -> Self {
        Self {
            members: Vec::new(),
        }
    }

    /// Returns named roots in canonical field-path order.
    pub fn members(&self) -> &[ObjectGraphMember] {
        &self.members
    }

    pub(super) fn into_members(self) -> Vec<ObjectGraphMember> {
        self.members
    }
}

/// One exact value materialized by store-owned input assembly.
pub struct PreparedValue {
    value_ref: ValueRef,
    bytes: Vec<u8>,
}

impl PreparedValue {
    pub(super) const fn new(value_ref: ValueRef, bytes: Vec<u8>) -> Self {
        Self { value_ref, bytes }
    }

    /// Returns complete producer-bound retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns exact verified canonical bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Sealed exact callback frame prepared from one verified predecessor.
///
/// The value is intentionally non-cloneable. Returning it inside [`TransitionMaterial`] proves
/// the callback result is committed against the exact manifest that supplied the callback.
pub struct PreparedFrame {
    authority: StoreAuthorityContext,
    tenant_scope_id: TenantScopeId,
    run_id: mfm_ids::RunId,
    journal_head: JournalHead,
    node_id: NodeId,
    input_manifest: InputManifest,
    input_manifest_ref: InputManifestRef,
    config: PreparedValue,
    context: Option<PreparedValue>,
    input: PreparedValue,
    authorities: Vec<PreparedAuthority>,
}

pub(super) struct PreparedFrameMaterial {
    pub(super) authority: StoreAuthorityContext,
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) run_id: mfm_ids::RunId,
    pub(super) journal_head: JournalHead,
    pub(super) node_id: NodeId,
    pub(super) input_manifest: InputManifest,
    pub(super) input_manifest_ref: InputManifestRef,
    pub(super) config: PreparedValue,
    pub(super) context: Option<PreparedValue>,
    pub(super) input: PreparedValue,
    pub(super) authorities: Vec<PreparedAuthority>,
}

impl PreparedFrame {
    pub(super) fn new(material: PreparedFrameMaterial) -> Self {
        let PreparedFrameMaterial {
            authority,
            tenant_scope_id,
            run_id,
            journal_head,
            node_id,
            input_manifest,
            input_manifest_ref,
            config,
            context,
            input,
            authorities,
        } = material;
        Self {
            authority,
            tenant_scope_id,
            run_id,
            journal_head,
            node_id,
            input_manifest,
            input_manifest_ref,
            config,
            context,
            input,
            authorities,
        }
    }

    /// Returns the exact certified node occurrence.
    pub const fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the complete store-authored immutable input manifest.
    pub const fn input_manifest(&self) -> &InputManifest {
        &self.input_manifest
    }

    /// Returns full retained input-manifest authority.
    pub const fn input_manifest_ref(&self) -> &InputManifestRef {
        &self.input_manifest_ref
    }

    /// Returns the exact configured frame value.
    pub const fn config(&self) -> &PreparedValue {
        &self.config
    }

    /// Returns the optional exact predecessor-visible context value.
    pub const fn context(&self) -> Option<&PreparedValue> {
        self.context.as_ref()
    }

    /// Returns the complete store-assembled input tree.
    pub const fn input(&self) -> &PreparedValue {
        &self.input
    }

    pub(super) fn into_parts_for(
        self,
        authority: &StoreAuthorityContext,
        view: &super::VerifiedRunView,
    ) -> Result<PreparedFrameParts> {
        if !self.authority.is_same_instance(authority)
            || self.tenant_scope_id != *view.tenant_scope_id()
            || self.run_id != *view.run_id()
            || self.journal_head != *view.journal_head()
        {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "prepared_frame",
                message: "callback frame does not belong to the exact store, run, and head",
            });
        }
        Ok(PreparedFrameParts {
            node_id: self.node_id,
            input_manifest_ref: self.input_manifest_ref,
            authorities: self.authorities,
        })
    }
}

pub(super) struct PreparedFrameParts {
    pub(super) node_id: NodeId,
    pub(super) input_manifest_ref: InputManifestRef,
    pub(super) authorities: Vec<PreparedAuthority>,
}

/// One exact producer-free callback output slot.
pub struct ProducedOutputSlot {
    output_ordinal: u32,
    field_path: FieldPath,
    root: ProducedObjectRoot,
}

impl ProducedOutputSlot {
    /// Constructs one producer-free output slot proposal.
    pub const fn new(output_ordinal: u32, field_path: FieldPath, root: ProducedObjectRoot) -> Self {
        Self {
            output_ordinal,
            field_path,
            root,
        }
    }

    /// Returns the certified output ordinal.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the certified output path.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns exact producer-free retained material.
    pub const fn root(&self) -> &ProducedObjectRoot {
        &self.root
    }
}

/// Producer-free callback settlement material.
pub enum SettlementMaterial {
    /// Successful output and ordered proposed facts.
    Succeeded {
        /// Exact output slots in certified ordinal order.
        output_roots: Vec<ProducedOutputSlot>,
        /// Facts in callback emission order.
        fact_roots: Vec<FactProposal>,
    },
    /// One typed callback failure.
    Failed {
        /// Complete typed failure root.
        typed_failure_root: Box<ProducedObjectRoot>,
    },
}

/// Closed runtime material accepted by store-owned transition preparation.
pub enum TransitionMaterial {
    /// Pure callback settlement.
    PureSettled {
        /// Exact store-prepared callback frame.
        prepared_frame: Box<PreparedFrame>,
        /// Producer-free callback result.
        settlement: SettlementMaterial,
        /// Complete producer-free recursive object closure returned by the callback.
        object_graph: ObjectGraphProposal,
    },
    /// Read callback settlement consuming one immutable observation.
    ReadSettled {
        /// Exact store-prepared callback frame.
        prepared_frame: Box<PreparedFrame>,
        /// Immutable authored request retained by the authorization.
        immutable_request_ref: ValueRef,
        /// Exact committed observation consumed by the callback.
        consumed_observation_ref: ObservationRef,
        /// Producer-free callback result.
        settlement: SettlementMaterial,
        /// Complete producer-free recursive object closure returned by the callback.
        object_graph: ObjectGraphProposal,
    },
    /// Durable effect request committed before executor entry.
    EffectRequested {
        /// Exact store-prepared callback frame.
        prepared_frame: Box<PreparedFrame>,
        /// Producer-free semantic request.
        semantic_request_root: Box<ProducedObjectRoot>,
        /// Deterministic effect identity.
        effect_key: EffectKey,
        /// Immutable semantic request digest.
        request_digest: RequestDigest,
        /// Certified executor binding.
        executor_binding_ref: CapabilityBindingRef,
        /// Complete producer-free recursive object closure returned by request authoring.
        object_graph: ObjectGraphProposal,
    },
    /// Effect settlement consuming complete terminal evidence.
    EffectSettled {
        /// Exact previously committed request transition.
        request_transition_ref: TransitionRef,
        /// Exact terminal observation consumed by settlement.
        consumed_terminal_observation_ref: ObservationRef,
        /// Producer-free callback result.
        settlement: SettlementMaterial,
        /// Complete producer-free recursive object closure returned by the callback.
        object_graph: ObjectGraphProposal,
    },
    /// Deterministic dependency skip whose exact terminal blockers are store-derived.
    DependencySkipped {
        /// Exact certified unstarted occurrence.
        node_id: NodeId,
    },
}

/// Reviewed producer-free metadata for one typed read failure.
///
/// Provider text, response bodies, paths, credentials, and arbitrary diagnostic maps have no
/// representation here. The store validates this tuple against the exact admitted read contract
/// before retaining the typed diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeFailureMetadata {
    safe_failure_contract_ref: ContentRef,
    stable_code: StableId,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
    coarse_size_class: Option<CoarseSizeClass>,
}

impl SafeFailureMetadata {
    /// Constructs one closed reviewed classification.
    pub const fn new(
        safe_failure_contract_ref: ContentRef,
        stable_code: StableId,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
    ) -> Self {
        Self {
            safe_failure_contract_ref,
            stable_code,
            failure_class,
            boundary_stage,
            coarse_size_class,
        }
    }

    /// Returns the exact admitted safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.safe_failure_contract_ref
    }

    /// Returns the stable code selected by that contract.
    pub const fn stable_code(&self) -> &StableId {
        &self.stable_code
    }

    /// Returns the closed universal failure class.
    pub const fn failure_class(&self) -> FailureClass {
        self.failure_class
    }

    /// Returns the reviewed external-boundary stage.
    pub const fn boundary_stage(&self) -> BoundaryStage {
        self.boundary_stage
    }

    /// Returns the optional reviewed coarse source-envelope size.
    pub const fn coarse_size_class(&self) -> Option<CoarseSizeClass> {
        self.coarse_size_class
    }
}

/// Closed producer-free material for one immutable read observation.
pub enum ReadObservationMaterial {
    /// The read returned one typed retained value.
    Returned {
        /// Complete producer-free returned root.
        returned_root: ProducedObjectRoot,
    },
    /// Boundary entry was proven not to have occurred.
    DidNotEnter {
        /// Optional producer-free typed diagnostic.
        diagnostic_root: Option<ProducedObjectRoot>,
        /// Reviewed redaction-safe classification.
        metadata: SafeFailureMetadata,
    },
    /// Boundary entry or terminal outcome remains indeterminate.
    Indeterminate {
        /// Optional producer-free typed diagnostic.
        diagnostic_root: Option<ProducedObjectRoot>,
        /// Reviewed redaction-safe classification.
        metadata: SafeFailureMetadata,
    },
    /// A surviving operational or integrity failure that is audit-only.
    NonDomainFailure {
        /// Closed failure with conservative entry status and fixed disposition.
        failure: mfm_journal::NonDomainFailure,
    },
}

/// Closed authorization material accepted by existing-run append preparation.
pub enum AuthorizationMaterial {
    /// Authorize one immutable read request from an exact store-prepared frame.
    Read {
        /// Exact frame against which the request was authored.
        prepared_frame: Box<PreparedFrame>,
        /// Complete producer-free immutable request.
        immutable_request_root: Box<ProducedObjectRoot>,
        /// Exact admitted per-call routing generation.
        routing_generation_ref: ContentRef,
    },
    /// Authorize driving one exact already-committed effect request.
    EnsureEffect {
        /// Exact effect-request transition.
        request_transition_ref: TransitionRef,
    },
}

/// Closed observation material accepted by existing-run append preparation.
pub enum ObservationMaterial {
    /// Observe one immutable read authorization.
    Read {
        /// Exact committed authorization.
        authorization_ref: AuthorizationRef,
        /// Closed producer-free read result.
        outcome: Box<ReadObservationMaterial>,
    },
    /// Observe one closed surviving executor outcome.
    EnsureEffect {
        /// Exact committed ensure authorization.
        authorization_ref: AuthorizationRef,
        /// Complete producer-free returned or safe-failure outcome.
        outcome: Box<EffectExecutorOutcome>,
    },
    /// Retain the exact response of one completed authoritative fact scan.
    FactSelection {
        /// Immutable completed scan sealed by this store.
        sealed: Box<super::SealedFactSelectionObservation>,
    },
    /// Observe a returned fact-store failure without rescanning or domain evidence.
    FactSelectionFailure {
        /// Exact committed fact-selection authorization.
        authorization_ref: AuthorizationRef,
        /// Closed fact-layer non-domain failure.
        failure: mfm_journal::NonDomainFailure,
    },
}

/// The exhaustive producer-free material accepted for one existing-run append.
pub enum ExistingRunAppendMaterial {
    /// One state transition and any inseparable closure.
    Transition(Box<TransitionMaterial>),
    /// One external-access authorization.
    Authorization(Box<AuthorizationMaterial>),
    /// One linked external-access observation.
    Observation(Box<ObservationMaterial>),
}
