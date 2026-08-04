//! Pure expansion and certification for structured programs.
//!
//! The process registry in this module contains only qualified, deterministic
//! expansion definitions and secret-free component objects.  No callback in
//! this module receives store, runtime, provider, signer, filesystem, clock,
//! or environment authority.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use futures_util::FutureExt;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    AccessFaultCode, BoundedComponentInvoker, CapabilityContractFault, ComponentFuture,
    EffectAdapterCompletion, EffectCapabilityImplementation, EffectRefreshMode,
    ReadAdapterCompletion, ReadCapabilityImplementation,
};
use mfm_facts::{FactSelectionReadFailure, FactSelectionReadResponse, FactSelectionRequest};
use mfm_ids::{
    AccessAttemptId, ContentDigest, ContentRef, DigestAlgorithm, FragmentBoundaryId, OccurrenceId,
    RequestDigest, RunId, SemanticCallId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    AccessKind, ExternalAccessAuthorized, HistoryObject, LexicalValueRef,
    PriorRunFactScannerBindingCertificate, PriorRunFactSelectionResponse, RecordRef, SemanticHead,
    TypedValueRef,
};
use mfm_program::structured::{
    closed_sum_contract, runtime_effect_capability_contract, runtime_read_capability_contract,
    state_contract, CapabilityExpansion, ClosedSum, CommittedObservation, PolicyExpansionRecipe,
    PriorRunFactSelectionCapability, RuntimeEffectAdapter, RuntimeEffectCapability,
    RuntimeReadAdapter, RuntimeReadCapability, RuntimeResourceAuthority, RuntimeSigner, State,
    StateSettlement, StructuredStateCallbacks,
};
use mfm_spec::structured::{
    access_fault_contract_canonical_json, access_fault_contract_ref,
    capability_expansion_requirement_ref, expansion_support_semantic_call_id,
    failure_handler_semantic_call_id, fan_out_join_contract_canonical_json,
    fan_out_join_contract_ref, lane_outcome_contract_canonical_json, lane_outcome_contract_ref,
    never_failure_contract_canonical_json, never_failure_contract_ref, policy_proceed_program_ref,
    prior_run_fact_scanner_adapter_contract, prior_run_fact_selection_capability_contract,
    retained_value_contract_ref, structured_value_contract, AuthoredBlock, AuthoredDeclaration,
    AuthoredFailureDirective, AuthoredFanOut, AuthoredMatch, AuthoredOperationCall,
    AuthoredStateCall, AuthoredStructuredProgram, BlockTail, CertifiedComponentObject,
    CertifiedFailureBoundary, CertifiedProgramComponents, CertifiedProgramDocument,
    CertifiedProgramRoot, CertifiedStructuralBounds, ClosedSumContract, ComponentObjectReference,
    ExpandedBlock, ExpandedDeclaration, ExpandedFanOut, ExpandedFanOutLane, ExpandedFragment,
    ExpandedMatch, ExpandedMatchArm, ExpandedStateBinding, ExpandedStructuredProgram,
    ExpansionBoundaryId, ExpansionPolicyContract, ExpansionStage, ExpansionTraceEntry,
    FailureMapperRegistration, FailureMappingLink, FailurePlan, FailurePlanIdentity,
    FailureScopeBinding, FragmentInputBinding, HandlerContinuation, LexicalProducer, LexicalSlot,
    NoFailureBoundary, PolicyCoverageEntry, ProposedStateValue, ResultRole,
    SecretFreeExecutableIdentity, SecretFreeImplementationDescriptor,
    SecretFreeImplementationManifest, SecretFreeImplementationManifestEntry,
    SecretFreeQualificationArtifact, SemanticCallPath, SemanticPathSegment,
    StateCapabilityAdapterSignerResourceManifest, StructuralPath, StructuralPathSegment,
    StructuredCapabilityProtocolContract, StructuredComponentKind,
    StructuredComponentManifestEntry, StructuredEffectRefreshContract, StructuredExecutionKind,
    StructuredExpansionProfile, StructuredExpansionProof, StructuredFactDescriptor,
    StructuredFailureContract, StructuredLiveComponentContract, StructuredPolicyCoverageProof,
    StructuredPublicContractRefs, StructuredSafeFailureDispositionContract,
    StructuredStateContract, StructuredStateExecutionContract, MAX_CERTIFIED_COMPONENT_OBJECTS,
    MAX_FAN_OUT_DEPTH, MAX_STRUCTURED_DECLARATIONS, MAX_STRUCTURED_LANES,
    MAX_STRUCTURED_OCCURRENCES,
};
use mfm_spec::{exact_content_ref, CanonicalJsonValue};
use mfm_values::{
    component_object_evidence_contract_canonical, component_object_evidence_contract_ref, MfmValue,
    RetainedValueContract, SchemaIdentity,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{CertifyError, Result};

const AUTHORED_OBJECT_TYPE: &str = "structured.authored_program";
const EXPANDED_OBJECT_TYPE: &str = "structured.expanded_program";
const PROFILE_OBJECT_TYPE: &str = "structured.expansion_profile";
const PROOF_OBJECT_TYPE: &str = "structured.expansion_proof";
const COVERAGE_PROOF_OBJECT_TYPE: &str = "structured.policy_coverage_proof";
const COMPONENT_MANIFEST_OBJECT_TYPE: &str = "structured.component_manifest";
const IMPLEMENTATION_MANIFEST_OBJECT_TYPE: &str = "structured.secret_free_implementation_manifest";
const POLICY_OBJECT_TYPE: &str = "structured.admission_policy";
const DATA_CONTRACT_OBJECT_TYPE: &str = "structured.data_contract";
const FACT_DESCRIPTOR_OBJECT_TYPE: &str = "structured.fact_descriptor";
const CLOSED_SUM_CONTRACT_OBJECT_TYPE: &str = "structured.closed_sum_contract";
const CAPABILITY_REQUIREMENT_OBJECT_TYPE: &str = "structured.capability_requirement";
const EXPANSION_POLICY_CONTRACT_OBJECT_TYPE: &str = "structured.expansion_policy_contract";
const ENTRY_POINT_CONTRACT_OBJECT_TYPE: &str = "structured.entry_point_contract";
const CERTIFIED_PROGRAM_CONTRACT_OBJECT_TYPE: &str = "structured.certified_program_contract";
const POLICY_COVERAGE_CONTRACT_OBJECT_TYPE: &str = "structured.policy_coverage_contract";
const PREDICATE_SET_OBJECT_TYPE: &str = "structured.certification_predicate_set";
const LANE_OUTCOME_CONTRACT_OBJECT_TYPE: &str = "structured.lane_outcome_contract";
const FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE: &str = "structured.fan_out_join_contract";
const STATE_CONTRACT_OBJECT_TYPE: &str = "structured.state_contract";
const CAPABILITY_CONTRACT_OBJECT_TYPE: &str = "structured.capability_contract";
const ADAPTER_CONTRACT_OBJECT_TYPE: &str = "structured.adapter_contract";
const SIGNER_CONTRACT_OBJECT_TYPE: &str = "structured.signer_contract";
const RESOURCE_CONTRACT_OBJECT_TYPE: &str = "structured.resource_contract";
const IMPLEMENTATION_CONTRACT_OBJECT_TYPE: &str = "structured.implementation_contract";
const EXECUTABLE_IDENTITY_OBJECT_TYPE: &str = "structured.executable_identity";
const QUALIFICATION_ARTIFACT_OBJECT_TYPE: &str = "structured.qualification_artifact";
const POLICY_RECIPE_OBJECT_TYPE: &str = "structured.policy_recipe";
const KERNEL_BASELINE_EXECUTABLE_ID: &str = "mfm.kernel/prior-run-fact-scanner";
const KERNEL_BASELINE_QUALIFICATION_ID: &str = "mfm.kernel/prior-run-fact-scanner-qualification";
const KERNEL_FACT_CAPABILITY_IMPLEMENTATION_ID: &str =
    "mfm.kernel/prior-run-fact-selection-capability-implementation";
const KERNEL_FACT_ADAPTER_IMPLEMENTATION_ID: &str =
    "mfm.kernel/prior-run-fact-scanner-adapter-implementation";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredComponentObject {
    object: CertifiedComponentObject,
    outbound_references: Vec<ComponentObjectReference>,
}

/// Closed completion produced only by the store-owned prior-run fact scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorRunFactScanCompletion {
    /// Complete typed response through the authorization frontier.
    Returned(FactSelectionReadResponse),
    /// Reviewed definite bounded-read failure.
    SafeFailure(FactSelectionReadFailure),
    /// Stable redaction-safe integrity fault.
    IntegrityFault(StableId),
}

type FactScanFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = PriorRunFactScanCompletion> + Send + 'static>,
>;

/// Non-cloneable proof that this process directly observed one newly committed
/// external-access authorization.
///
/// Production construction is limited to the store adapter via
/// [`NewlyAppendedAuthorization::from_store_mint`].
pub struct NewlyAppendedAuthorization {
    authorization_ref: RecordRef,
    authorization: ExternalAccessAuthorized,
    fact_scan: Option<Box<dyn FnOnce(FactSelectionRequest) -> FactScanFuture + Send>>,
}

impl std::fmt::Debug for NewlyAppendedAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewlyAppendedAuthorization")
            .field("authorization_ref", &self.authorization_ref)
            .field("access_attempt_id", &self.authorization.access_attempt_id)
            .finish_non_exhaustive()
    }
}

impl NewlyAppendedAuthorization {
    /// Store-adapter mint after a newly committed authorization append.
    #[doc(hidden)]
    pub fn from_store_mint(
        authorization_ref: RecordRef,
        authorization: ExternalAccessAuthorized,
        fact_scan: Option<Box<dyn FnOnce(FactSelectionRequest) -> FactScanFuture + Send>>,
    ) -> Self {
        Self {
            authorization_ref,
            authorization,
            fact_scan,
        }
    }

    /// Returns the exact assigned authorization record reference.
    pub const fn authorization_ref(&self) -> &RecordRef {
        &self.authorization_ref
    }

    /// Returns the kernel-derived access-attempt identity.
    pub const fn access_attempt_id(&self) -> &AccessAttemptId {
        &self.authorization.access_attempt_id
    }

    /// Returns the complete immutable committed authorization.
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        &self.authorization
    }

    /// Consumes the store-minted authority for the exact committed prior-run fact Read.
    pub fn invoke_prior_run_fact_scan(
        self,
        request: FactSelectionRequest,
    ) -> Option<FactScanFuture> {
        self.fact_scan.map(|scan| scan(request))
    }
}

/// Exact qualified entry-point policy selected by process assembly.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QualifiedStructuredEntryPointPolicy {
    /// Stable operation identity selected by this entry point.
    operation_id: StableId,
    /// Exact public entry-point contract object.
    entry_point_contract_ref: ContentRef,
    /// Exact admission-policy object.
    admission_policy_ref: ContentRef,
    /// Exact certified-program contract.
    certified_program_contract_ref: ContentRef,
    /// Exact certification predicate set.
    certification_predicate_set_ref: ContentRef,
    /// Required pure expansion profile.
    expansion_profile_ref: ContentRef,
    /// Exact policy-coverage obligation contract.
    policy_coverage_contract_ref: ContentRef,
    /// Public admission-input contracts in declared root order.
    public_input_contract_refs: Vec<ContentRef>,
    /// Exact public successful-output contract.
    public_output_contract_ref: ContentRef,
    /// Exact public failure contract, including the kernel `Never` sentinel.
    public_failure_contract_ref: ContentRef,
}

#[derive(Serialize)]
struct QualifiedPolicyPreimage<'a> {
    operation_id: &'a StableId,
    entry_point_contract_ref: &'a ContentRef,
    certified_program_contract_ref: &'a ContentRef,
    certification_predicate_set_ref: &'a ContentRef,
    expansion_profile_ref: &'a ContentRef,
    policy_coverage_contract_ref: &'a ContentRef,
    public_input_contract_refs: &'a [ContentRef],
    public_output_contract_ref: &'a ContentRef,
    public_failure_contract_ref: &'a ContentRef,
}

impl QualifiedStructuredEntryPointPolicy {
    /// Constructs one qualified policy and derives its exact canonical identity.
    #[allow(clippy::too_many_arguments)]
    fn new(
        operation_id: StableId,
        entry_point_contract_ref: ContentRef,
        certified_program_contract_ref: ContentRef,
        certification_predicate_set_ref: ContentRef,
        expansion_profile_ref: ContentRef,
        policy_coverage_contract_ref: ContentRef,
        public_input_contract_refs: Vec<ContentRef>,
        public_output_contract_ref: ContentRef,
        public_failure_contract_ref: ContentRef,
    ) -> Result<Self> {
        let admission_policy_ref = typed_content_ref(
            "mfm.qualified-structured-entry-point-policy",
            &QualifiedPolicyPreimage {
                operation_id: &operation_id,
                entry_point_contract_ref: &entry_point_contract_ref,
                certified_program_contract_ref: &certified_program_contract_ref,
                certification_predicate_set_ref: &certification_predicate_set_ref,
                expansion_profile_ref: &expansion_profile_ref,
                policy_coverage_contract_ref: &policy_coverage_contract_ref,
                public_input_contract_refs: &public_input_contract_refs,
                public_output_contract_ref: &public_output_contract_ref,
                public_failure_contract_ref: &public_failure_contract_ref,
            },
        )?;
        Ok(Self {
            operation_id,
            entry_point_contract_ref,
            admission_policy_ref,
            certified_program_contract_ref,
            certification_predicate_set_ref,
            expansion_profile_ref,
            policy_coverage_contract_ref,
            public_input_contract_refs,
            public_output_contract_ref,
            public_failure_contract_ref,
        })
    }

    fn preimage(&self) -> QualifiedPolicyPreimage<'_> {
        QualifiedPolicyPreimage {
            operation_id: &self.operation_id,
            entry_point_contract_ref: &self.entry_point_contract_ref,
            certified_program_contract_ref: &self.certified_program_contract_ref,
            certification_predicate_set_ref: &self.certification_predicate_set_ref,
            expansion_profile_ref: &self.expansion_profile_ref,
            policy_coverage_contract_ref: &self.policy_coverage_contract_ref,
            public_input_contract_refs: &self.public_input_contract_refs,
            public_output_contract_ref: &self.public_output_contract_ref,
            public_failure_contract_ref: &self.public_failure_contract_ref,
        }
    }

    fn derived_admission_policy_ref(&self) -> Result<ContentRef> {
        typed_content_ref(
            "mfm.qualified-structured-entry-point-policy",
            &self.preimage(),
        )
    }
}

/// Non-cloneable process authority for one fully certified program.
///
/// ```compile_fail
/// use mfm_certify::structured::CertifiedProgram;
///
/// fn duplicate(program: CertifiedProgram) {
///     let _second = program.clone();
/// }
/// ```
pub struct CertifiedProgram {
    authored: AuthoredStructuredProgram,
    expanded: ExpandedStructuredProgram,
    profile: StructuredExpansionProfile,
    proof: StructuredExpansionProof,
    coverage: StructuredPolicyCoverageProof,
    document: CertifiedProgramDocument,
    value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
}

impl std::fmt::Debug for CertifiedProgram {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CertifiedProgram")
            .field("operation_id", &self.expanded.operation_id)
            .field("program_ref", &self.reference().ok())
            .finish()
    }
}

impl CertifiedProgram {
    /// Returns the exact canonical authored program.
    pub const fn authored(&self) -> &AuthoredStructuredProgram {
        &self.authored
    }

    /// Returns the exact fully expanded program.
    pub const fn expanded(&self) -> &ExpandedStructuredProgram {
        &self.expanded
    }

    /// Returns the exact trusted expansion profile.
    pub const fn expansion_profile(&self) -> &StructuredExpansionProfile {
        &self.profile
    }

    /// Returns the exact deterministic expansion proof.
    pub const fn expansion_proof(&self) -> &StructuredExpansionProof {
        &self.proof
    }

    /// Returns the exact independently recomputed policy-coverage proof.
    pub const fn policy_coverage_proof(&self) -> &StructuredPolicyCoverageProof {
        &self.coverage
    }

    /// Returns the canonical persisted certification document.
    pub const fn document(&self) -> &CertifiedProgramDocument {
        &self.document
    }

    /// Returns the exact callback-free schema identities for every retained
    /// value contract reachable from this certified program.
    pub const fn value_schemas(&self) -> &BTreeMap<ContentRef, SchemaIdentity> {
        &self.value_schemas
    }

    /// Returns the sole canonical certified-program reference.
    pub fn reference(&self) -> Result<ContentRef> {
        self.document
            .content_ref()
            .map_err(|error| CertifyError::Certification(error.to_string()))
    }

    /// Consumes the authority into its persisted document.
    pub fn into_document(self) -> CertifiedProgramDocument {
        self.document
    }

    /// Consumes certification into the immutable callback-free data required
    /// by persisted-history verification.
    pub fn into_verification_parts(
        self,
    ) -> (
        CertifiedProgramDocument,
        ExpandedStructuredProgram,
        BTreeMap<ContentRef, SchemaIdentity>,
    ) {
        (self.document, self.expanded, self.value_schemas)
    }
}

/// Fully expanded boundary plus all deterministic proof trace owned by it.
#[must_use = "an expanded boundary must be returned to certification or wrapped"]
struct ExpandedBoundary {
    fragment: ExpandedFragment,
}

impl ExpandedBoundary {
    fn from_leaf(fragment: ExpandedFragment) -> Self {
        Self { fragment }
    }

    /// Borrows the external fragment boundary.
    pub const fn fragment(&self) -> &ExpandedFragment {
        &self.fragment
    }

    fn into_fragment(self) -> ExpandedFragment {
        self.fragment
    }
}

fn relocate_protected_boundary(
    boundary: ExpandedBoundary,
    new_declaration_path: &StructuralPath,
    caller_sources: Vec<LexicalSlot>,
) -> Result<ExpandedBoundary> {
    let fragment = boundary.into_fragment();
    if fragment.input_bindings.len() != caller_sources.len() {
        return Err(CertifyError::Certification(
            "policy proceed input count differs from the protected boundary".to_owned(),
        ));
    }
    let old_declaration_path = fragment_declaration_path(&fragment.path)?;
    let trailing_segments =
        fragment.path.segments()[old_declaration_path.segments().len()..].to_vec();
    let mut new_segments = new_declaration_path.segments().to_vec();
    new_segments.extend(trailing_segments);
    let new_fragment_path = StructuralPath::new(new_segments)?;
    let new_boundary_id = new_fragment_path.fragment_boundary_id()?;
    let mut replacements = BTreeMap::new();
    for (binding, source) in fragment.input_bindings.iter().zip(&caller_sources) {
        if binding.child_contract_ref != source.contract_ref {
            return Err(CertifyError::Certification(
                "policy proceed input contract differs from the protected boundary".to_owned(),
            ));
        }
        let old_input = LexicalSlot {
            lexical_path: fragment.path.clone(),
            contract_ref: binding.child_contract_ref.clone(),
            producer: LexicalProducer::FragmentInput {
                boundary_id: fragment.boundary_id.clone(),
                child_root_id: binding.child_root_id.clone(),
                source: Box::new(binding.caller_slot.clone()),
            },
        };
        let new_input = LexicalSlot {
            lexical_path: new_fragment_path.clone(),
            contract_ref: binding.child_contract_ref.clone(),
            producer: LexicalProducer::FragmentInput {
                boundary_id: new_boundary_id.clone(),
                child_root_id: binding.child_root_id.clone(),
                source: Box::new(source.clone()),
            },
        };
        replacements.insert(old_input, new_input);
    }
    let rewriter = ExpandedBoundaryRewriter {
        old_prefix: old_declaration_path,
        new_prefix: new_declaration_path.clone(),
        replacements,
    };
    let mut relocated = rewriter.rewrite_fragment(fragment)?;
    for (binding, source) in relocated.input_bindings.iter_mut().zip(caller_sources) {
        binding.caller_slot = source;
    }
    if relocated.path != new_fragment_path || relocated.boundary_id != new_boundary_id {
        return Err(CertifyError::Certification(
            "policy proceed relocation changed its normalized boundary identity".to_owned(),
        ));
    }
    Ok(ExpandedBoundary {
        fragment: relocated,
    })
}

fn fragment_declaration_path(path: &StructuralPath) -> Result<StructuralPath> {
    let segments = path.segments();
    let trailing_fragments = segments
        .iter()
        .rev()
        .take_while(|segment| matches!(segment, StructuralPathSegment::Fragment { .. }))
        .count();
    let declaration_len = segments.len().saturating_sub(trailing_fragments);
    if trailing_fragments == 0
        || !matches!(
            segments.get(declaration_len.saturating_sub(1)),
            Some(StructuralPathSegment::Declaration { .. })
        )
    {
        return Err(CertifyError::Planning(
            "protected policy fragment has no exact declaration prefix".to_owned(),
        ));
    }
    StructuralPath::new(segments[..declaration_len].to_vec()).map_err(Into::into)
}

struct ExpandedBoundaryRewriter {
    old_prefix: StructuralPath,
    new_prefix: StructuralPath,
    replacements: BTreeMap<LexicalSlot, LexicalSlot>,
}

impl ExpandedBoundaryRewriter {
    fn rewrite_path(&self, path: &StructuralPath) -> Result<StructuralPath> {
        let old = self.old_prefix.segments();
        let actual = path.segments();
        if actual.len() < old.len() || actual[..old.len()] != *old {
            return Ok(path.clone());
        }
        let mut segments = self.new_prefix.segments().to_vec();
        segments.extend_from_slice(&actual[old.len()..]);
        StructuralPath::new(segments).map_err(Into::into)
    }

    fn rewrite_slot(&self, slot: &LexicalSlot) -> Result<LexicalSlot> {
        if let Some(replacement) = self.replacements.get(slot) {
            return Ok(replacement.clone());
        }
        let lexical_path = self.rewrite_path(&slot.lexical_path)?;
        let producer = match &slot.producer {
            LexicalProducer::AdmissionRoot { root_id } => LexicalProducer::AdmissionRoot {
                root_id: root_id.clone(),
            },
            LexicalProducer::AuthoredCallOutput {
                semantic_call_id,
                role,
            } => LexicalProducer::AuthoredCallOutput {
                semantic_call_id: semantic_call_id.clone(),
                role: *role,
            },
            LexicalProducer::StateOutput { role, .. } => LexicalProducer::StateOutput {
                occurrence_id: lexical_path.occurrence_id()?,
                role: *role,
            },
            LexicalProducer::ArmValue {
                selected_arm_path,
                source,
            } => LexicalProducer::ArmValue {
                selected_arm_path: self.rewrite_path(selected_arm_path)?,
                source: Box::new(self.rewrite_slot(source)?),
            },
            LexicalProducer::MatchMerge {
                match_path,
                declaration_ordered_arm_slots,
            } => LexicalProducer::MatchMerge {
                match_path: self.rewrite_path(match_path)?,
                declaration_ordered_arm_slots: declaration_ordered_arm_slots
                    .iter()
                    .map(|slot| self.rewrite_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
            LexicalProducer::ScopeFailureMerge {
                scope_id,
                declaration_ordered_failure_slots,
            } => LexicalProducer::ScopeFailureMerge {
                scope_id: scope_id.clone(),
                declaration_ordered_failure_slots: declaration_ordered_failure_slots
                    .iter()
                    .map(|slot| self.rewrite_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
            LexicalProducer::VariantPayload {
                selector,
                canonical_tag,
                payload_path,
            } => LexicalProducer::VariantPayload {
                selector: Box::new(self.rewrite_slot(selector)?),
                canonical_tag: canonical_tag.clone(),
                payload_path: payload_path.clone(),
            },
            LexicalProducer::FragmentInput {
                child_root_id,
                source,
                ..
            } => LexicalProducer::FragmentInput {
                boundary_id: lexical_path.fragment_boundary_id()?,
                child_root_id: child_root_id.clone(),
                source: Box::new(self.rewrite_slot(source)?),
            },
            LexicalProducer::FragmentBoundary { role, source, .. } => {
                LexicalProducer::FragmentBoundary {
                    boundary_id: lexical_path.fragment_boundary_id()?,
                    role: *role,
                    source: Box::new(self.rewrite_slot(source)?),
                }
            }
            LexicalProducer::LaneOutcome {
                lane_path,
                success_slot,
                failure_slot,
            } => LexicalProducer::LaneOutcome {
                lane_path: self.rewrite_path(lane_path)?,
                success_slot: success_slot
                    .as_deref()
                    .map(|slot| self.rewrite_slot(slot).map(Box::new))
                    .transpose()?,
                failure_slot: failure_slot
                    .as_deref()
                    .map(|slot| self.rewrite_slot(slot).map(Box::new))
                    .transpose()?,
            },
            LexicalProducer::FanOutJoin {
                group_path,
                declaration_ordered_lane_slots,
            } => LexicalProducer::FanOutJoin {
                group_path: self.rewrite_path(group_path)?,
                declaration_ordered_lane_slots: declaration_ordered_lane_slots
                    .iter()
                    .map(|slot| self.rewrite_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
        };
        Ok(LexicalSlot {
            lexical_path,
            contract_ref: slot.contract_ref.clone(),
            producer,
        })
    }

    fn rewrite_tail(&self, tail: BlockTail) -> Result<BlockTail> {
        match tail {
            BlockTail::Normal(slot) => Ok(BlockTail::Normal(self.rewrite_slot(&slot)?)),
            BlockTail::ScopeFailure(slot) => Ok(BlockTail::ScopeFailure(self.rewrite_slot(&slot)?)),
        }
    }

    fn rewrite_block(&self, block: ExpandedBlock) -> Result<ExpandedBlock> {
        Ok(ExpandedBlock {
            path: self.rewrite_path(&block.path)?,
            failure_scope: block.failure_scope,
            declarations: block
                .declarations
                .into_iter()
                .map(|declaration| self.rewrite_declaration(declaration))
                .collect::<Result<Vec<_>>>()?,
            failure_exits: block
                .failure_exits
                .iter()
                .map(|slot| self.rewrite_slot(slot))
                .collect::<Result<Vec<_>>>()?,
            tail: self.rewrite_tail(block.tail)?,
        })
    }

    fn rewrite_declaration(&self, declaration: ExpandedDeclaration) -> Result<ExpandedDeclaration> {
        match declaration {
            ExpandedDeclaration::State(state) => Ok(ExpandedDeclaration::State(Box::new(
                self.rewrite_state(*state)?,
            ))),
            ExpandedDeclaration::Match(binding) => {
                let binding = *binding;
                Ok(ExpandedDeclaration::Match(Box::new(ExpandedMatch {
                    label: binding.label,
                    path: self.rewrite_path(&binding.path)?,
                    selector: self.rewrite_slot(&binding.selector)?,
                    selector_contract: binding.selector_contract,
                    arms: binding
                        .arms
                        .into_iter()
                        .map(|arm| {
                            Ok(ExpandedMatchArm {
                                canonical_tag: arm.canonical_tag,
                                label: arm.label,
                                path: self.rewrite_path(&arm.path)?,
                                body: self.rewrite_block(arm.body)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                    output_slot: self.rewrite_slot(&binding.output_slot)?,
                })))
            }
            ExpandedDeclaration::FanOut(group) => {
                let group = *group;
                Ok(ExpandedDeclaration::FanOut(Box::new(ExpandedFanOut {
                    label: group.label,
                    path: self.rewrite_path(&group.path)?,
                    lane_output_contract_ref: group.lane_output_contract_ref,
                    lane_failure_contract: group.lane_failure_contract,
                    lanes: group
                        .lanes
                        .into_iter()
                        .map(|lane| {
                            Ok(ExpandedFanOutLane {
                                key: lane.key,
                                declaration_ordinal: lane.declaration_ordinal,
                                path: self.rewrite_path(&lane.path)?,
                                body: self.rewrite_block(lane.body)?,
                                outcome_slot: self.rewrite_slot(&lane.outcome_slot)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                    output_slot: self.rewrite_slot(&group.output_slot)?,
                })))
            }
            ExpandedDeclaration::Fragment(fragment) => Ok(ExpandedDeclaration::Fragment(Box::new(
                self.rewrite_fragment(*fragment)?,
            ))),
        }
    }

    fn rewrite_fragment(&self, fragment: ExpandedFragment) -> Result<ExpandedFragment> {
        let path = self.rewrite_path(&fragment.path)?;
        Ok(ExpandedFragment {
            semantic_call_id: fragment.semantic_call_id.clone(),
            label: fragment.label,
            boundary_id: path.fragment_boundary_id()?,
            path,
            input_bindings: fragment
                .input_bindings
                .into_iter()
                .map(|binding| {
                    Ok(mfm_spec::structured::FragmentInputBinding {
                        child_root_id: binding.child_root_id,
                        child_contract_ref: binding.child_contract_ref,
                        caller_slot: self.rewrite_slot(&binding.caller_slot)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            body: self.rewrite_block(fragment.body)?,
            success_slot: self.rewrite_slot(&fragment.success_slot)?,
            failure_boundary: self
                .rewrite_boundary(fragment.failure_boundary, &fragment.semantic_call_id)?,
        })
    }

    fn rewrite_state(&self, state: ExpandedStateBinding) -> Result<ExpandedStateBinding> {
        let occurrence_path = self.rewrite_path(&state.occurrence_path)?;
        Ok(ExpandedStateBinding {
            semantic_call_id: state.semantic_call_id.clone(),
            occurrence_id: occurrence_path.occurrence_id()?,
            occurrence_path,
            label: state.label,
            contract: state.contract,
            inputs: state
                .inputs
                .iter()
                .map(|slot| self.rewrite_slot(slot))
                .collect::<Result<Vec<_>>>()?,
            output_slot: self.rewrite_slot(&state.output_slot)?,
            failure_boundary: self
                .rewrite_boundary(state.failure_boundary, &state.semantic_call_id)?,
        })
    }

    fn rewrite_boundary(
        &self,
        boundary: CertifiedFailureBoundary,
        semantic_call_id: &SemanticCallId,
    ) -> Result<CertifiedFailureBoundary> {
        match boundary {
            CertifiedFailureBoundary::NoFailure(marker) => {
                Ok(CertifiedFailureBoundary::NoFailure(marker))
            }
            CertifiedFailureBoundary::Typed {
                failure_contract,
                source_slot,
                plan,
            } => {
                let source_slot = self.rewrite_slot(&source_slot)?;
                Ok(CertifiedFailureBoundary::Typed {
                    failure_contract,
                    source_slot: source_slot.clone(),
                    plan: Box::new(self.rewrite_plan(*plan, semantic_call_id, &source_slot)?),
                })
            }
        }
    }

    fn rewrite_plan(
        &self,
        plan: FailurePlan,
        semantic_call_id: &SemanticCallId,
        boundary_source: &LexicalSlot,
    ) -> Result<FailurePlan> {
        match plan {
            FailurePlan::Handled {
                plan_path,
                before_handler,
                handler,
                continuation,
                ..
            } => {
                let plan_path = self.rewrite_path(&plan_path)?;
                let plan_id = FailurePlanIdentity {
                    source_semantic_call_id: semantic_call_id.clone(),
                    source_slot: boundary_source.clone(),
                    plan_path: plan_path.clone(),
                }
                .derive()?;
                let continuation = match *continuation {
                    HandlerContinuation::DefaultPropagation {
                        handler_output_slot,
                        route_contract,
                        payload_slot,
                        failure_tail,
                    } => HandlerContinuation::DefaultPropagation {
                        handler_output_slot: self.rewrite_slot(&handler_output_slot)?,
                        route_contract,
                        payload_slot: Box::new(self.rewrite_slot(&payload_slot)?),
                        failure_tail: Box::new(self.rewrite_slot(&failure_tail)?),
                    },
                    HandlerContinuation::CustomRecovery {
                        handler_output_slot,
                        route_contract,
                        arms,
                    } => HandlerContinuation::CustomRecovery {
                        handler_output_slot: self.rewrite_slot(&handler_output_slot)?,
                        route_contract,
                        arms: arms
                            .into_iter()
                            .map(|arm| {
                                Ok(ExpandedMatchArm {
                                    canonical_tag: arm.canonical_tag,
                                    label: arm.label,
                                    path: self.rewrite_path(&arm.path)?,
                                    body: self.rewrite_block(arm.body)?,
                                })
                            })
                            .collect::<Result<Vec<_>>>()?,
                    },
                };
                Ok(FailurePlan::Handled {
                    plan_id,
                    plan_path,
                    source_slot: boundary_source.clone(),
                    before_handler: Box::new(self.rewrite_block(*before_handler)?),
                    handler: Box::new(self.rewrite_state(*handler)?),
                    continuation: Box::new(continuation),
                })
            }
            FailurePlan::Propagate {
                plan_path,
                before_boundary,
                mapping_chain,
                boundary_slot,
                ..
            } => {
                let plan_path = self.rewrite_path(&plan_path)?;
                let plan_id = FailurePlanIdentity {
                    source_semantic_call_id: semantic_call_id.clone(),
                    source_slot: boundary_source.clone(),
                    plan_path: plan_path.clone(),
                }
                .derive()?;
                let boundary_slot = self.rewrite_slot(&boundary_slot)?;
                let mapping_chain = mapping_chain
                    .into_iter()
                    .map(|link| {
                        Ok(FailureMappingLink {
                            plan_id: plan_id.clone(),
                            mapper: Box::new(self.rewrite_state(*link.mapper)?),
                            input_slot: self.rewrite_slot(&link.input_slot)?,
                            output_slot: self.rewrite_slot(&link.output_slot)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(FailurePlan::Propagate {
                    plan_id,
                    plan_path,
                    source_slot: boundary_source.clone(),
                    before_boundary: Box::new(self.rewrite_block(*before_boundary)?),
                    mapping_chain,
                    boundary_id: boundary_slot.lexical_path.fragment_boundary_id()?,
                    boundary_slot: Box::new(boundary_slot),
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredState {
    contract: StructuredStateContract,
}

type ProcessComponentKey = (StructuredComponentKind, ContentRef, ContentRef);

/// Exact registry-issued identity of one qualified live process component.
///
/// Fields remain private so callers can inspect attribution but cannot author a
/// replacement identity for Runtime dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedComponentIdentity {
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    implementation_contract_ref: ContentRef,
}

impl QualifiedComponentIdentity {
    /// Returns the qualified semantic component kind.
    pub const fn component_kind(&self) -> StructuredComponentKind {
        self.component_kind
    }

    /// Returns the exact semantic contract selected by certification.
    pub const fn semantic_contract_ref(&self) -> &ContentRef {
        &self.semantic_contract_ref
    }

    /// Returns the exact qualified implementation contract.
    pub const fn implementation_contract_ref(&self) -> &ContentRef {
        &self.implementation_contract_ref
    }
}

/// Closed redaction-safe fault kind emitted by one qualified process component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedProcessFaultCode {
    /// Registered callback panicked or otherwise failed to return normally.
    Callback,
    /// Typed process input or output failed exact canonical encoding.
    Codec,
    /// The registered callback kind or live component contract did not match.
    Contract,
    /// A callback returned a semantically invalid state proposal.
    Candidate,
}

/// Exact attributed fault from one registry-qualified process component.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("qualified process component failed")]
pub struct QualifiedProcessFault {
    code: QualifiedProcessFaultCode,
    component: Box<QualifiedComponentIdentity>,
}

impl QualifiedProcessFault {
    fn new(code: QualifiedProcessFaultCode, component: QualifiedComponentIdentity) -> Self {
        Self {
            code,
            component: Box::new(component),
        }
    }

    /// Returns the closed redaction-safe fault classification.
    pub const fn code(&self) -> QualifiedProcessFaultCode {
        self.code
    }

    /// Returns the exact process component that produced the fault.
    pub fn component(&self) -> &QualifiedComponentIdentity {
        self.component.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeProcessFaultCode {
    Callback,
    Codec,
    Contract,
    Candidate,
}

type RuntimeProcessResult<T> = std::result::Result<T, RuntimeProcessFaultCode>;

impl From<RuntimeProcessFaultCode> for QualifiedProcessFaultCode {
    fn from(code: RuntimeProcessFaultCode) -> Self {
        match code {
            RuntimeProcessFaultCode::Callback => Self::Callback,
            RuntimeProcessFaultCode::Codec => Self::Codec,
            RuntimeProcessFaultCode::Contract => Self::Contract,
            RuntimeProcessFaultCode::Candidate => Self::Candidate,
        }
    }
}

/// Exact producer-free state result returned through qualified process authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedStateProposal {
    origin: QualifiedComponentIdentity,
    value: QualifiedStateProposalValue,
}

impl QualifiedStateProposal {
    /// Returns the exact qualified callback that authored this proposal.
    pub const fn origin(&self) -> &QualifiedComponentIdentity {
        &self.origin
    }

    /// Consumes the proposal into its exact origin and producer-free value.
    pub fn into_parts(self) -> (QualifiedComponentIdentity, QualifiedStateProposalValue) {
        (self.origin, self.value)
    }
}

/// Producer-free successful or typed-failure value of a qualified proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedStateProposalValue {
    /// Successful typed value with exact callback emission order.
    Success {
        /// Canonical typed state output.
        value: CanonicalJsonValue,
        /// Exact proposed durable facts.
        facts: mfm_facts::FactSet,
    },
    /// Typed domain failure value.
    Failure(CanonicalJsonValue),
}

/// Closed result of invoking one qualified observation settlement callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedStateSettlement {
    /// Exact producer-free semantic proposal.
    Proposed(QualifiedStateProposal),
    /// The callback rejected committed observation evidence.
    InvalidEvidence(QualifiedComponentIdentity),
}

/// Closed normal completion returned by one qualified Read or Effect adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedAccessCompletion {
    /// Exact schema-valid returned value.
    Returned(CanonicalJsonValue),
    /// Exact reviewed definite safe-failure value.
    SafeFailure(CanonicalJsonValue),
    /// Effect-only proof that the protected entry did not occur, together
    /// with the public lineage head derived by that same bound target.
    SupersededBeforeEntry {
        /// Exact typed supersession evidence.
        evidence: CanonicalJsonValue,
        /// Secret-free public head for the bound target's stable lineage.
        public_lineage_head: Box<HistoryObject>,
    },
    /// Effect-only possible-entry ambiguity.
    EntryUnknown(StableId),
    /// Reviewed redaction-safe integrity fault.
    IntegrityFault(StableId),
}

/// Runtime-owned public inputs used to select one current physical target.
#[derive(Debug, Clone, Copy)]
pub struct AccessTargetSelection<'a> {
    /// Target run.
    pub run_id: &'a RunId,
    /// Exact executable occurrence.
    pub occurrence_id: &'a OccurrenceId,
    /// Exact normalized occurrence path reference.
    pub occurrence_path_ref: &'a ContentRef,
    /// Stable semantic call selected by certification.
    pub semantic_call_id: &'a SemanticCallId,
    /// Semantic head against which this access is authorized.
    pub semantic_head: &'a SemanticHead,
    /// Fold-derived attempt ordinal expected for this access.
    pub attempt_ordinal: u64,
    /// Exact current producer-bound input for the access state.
    pub state_input_ref: &'a LexicalValueRef,
    /// Qualified store lineage containing the run and tenant fact frontier.
    pub store_scope_id: &'a StoreScopeId,
    /// Qualified writer epoch containing the run and tenant fact frontier.
    pub store_epoch: StoreEpoch,
    /// Exact admitted tenant.
    pub tenant_scope_id: &'a TenantScopeId,
    /// Exact admitted prior-run source allowlist.
    pub admitted_prior_run_source_manifest_ref: &'a ContentRef,
    /// Immutable routing policy admitted for this run.
    pub admitted_routing_policy_ref: &'a ContentRef,
    /// Exact admitted stable Resource lineage for this access, when any.
    pub stable_resource_lineage_contract_ref: Option<&'a ContentRef>,
    /// Folded minimum non-rollback public head for a refreshed Effect.
    pub minimum_lineage_head_ref: Option<&'a ContentRef>,
}

/// Immutable context presented to one registered target-binding source.
///
/// This value contains only public identities. It carries no writer,
/// authorization, invoker, credential, or target-session authority.
#[derive(Debug, Clone, Copy)]
pub struct PhysicalBindingSelection<'a> {
    /// Target run.
    pub run_id: &'a RunId,
    /// Exact executable occurrence.
    pub occurrence_id: &'a OccurrenceId,
    /// Exact normalized occurrence path reference.
    pub occurrence_path_ref: &'a ContentRef,
    /// Stable semantic call selected by certification.
    pub semantic_call_id: &'a SemanticCallId,
    /// Semantic head against which this access is authorized.
    pub semantic_head: &'a SemanticHead,
    /// Fold-derived attempt ordinal expected for this access.
    pub attempt_ordinal: u64,
    /// Exact current producer-bound input for the access state.
    pub state_input_ref: &'a LexicalValueRef,
    /// Qualified store lineage containing the run and tenant fact frontier.
    pub store_scope_id: &'a StoreScopeId,
    /// Qualified writer epoch containing the run and tenant fact frontier.
    pub store_epoch: StoreEpoch,
    /// Exact admitted tenant.
    pub tenant_scope_id: &'a TenantScopeId,
    /// Exact admitted prior-run source allowlist.
    pub admitted_prior_run_source_manifest_ref: &'a ContentRef,
    /// Exact semantic capability contract.
    pub capability_contract_ref: &'a ContentRef,
    /// Exact process-qualified capability implementation.
    pub capability_implementation_ref: &'a ContentRef,
    /// Exact semantic adapter contract.
    pub adapter_contract_ref: &'a ContentRef,
    /// Exact process-qualified adapter implementation.
    pub adapter_implementation_ref: &'a ContentRef,
    /// Immutable routing policy admitted for this run.
    pub admitted_routing_policy_ref: &'a ContentRef,
    /// Exact admitted stable Resource lineage for this access, when any.
    pub stable_resource_lineage_contract_ref: Option<&'a ContentRef>,
    /// Folded minimum non-rollback public head for a refreshed Effect.
    pub minimum_lineage_head_ref: Option<&'a ContentRef>,
}

/// One concrete Read target owning its public certificate and private live
/// invoker, session, and credential handle.
pub trait RuntimeReadPhysicalBinding<C>: RuntimeReadAdapter<C> + Send + Sync + 'static
where
    C: RuntimeReadCapability,
{
    /// Returns the immutable secret-free certificate for this exact target.
    fn public_certificate(&self) -> &HistoryObject;
}

/// Process-assembly source of current concrete Read targets.
pub trait RuntimeReadPhysicalBindingSource<C>: Send + Sync + 'static
where
    C: RuntimeReadCapability,
{
    /// Exact target-bound adapter type returned by this source.
    type Binding: RuntimeReadPhysicalBinding<C>;

    /// Selects one current target-bound Read binding before authorization.
    /// This is a process-local qualification lookup and must not enter the
    /// provider, resource, signer, or other protected target.
    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>>;
}

/// One concrete Effect target owning its public certificate and private live
/// invoker, session, and credential handle.
pub trait RuntimeEffectPhysicalBinding<C>: RuntimeEffectAdapter<C> + Send + Sync + 'static
where
    C: RuntimeEffectCapability,
{
    /// Returns the immutable secret-free certificate for this exact target.
    fn public_certificate(&self) -> &HistoryObject;

    /// Derives the public lineage head from supersession evidence through this
    /// same bound target. Ordinary non-refreshable Effects return `None`.
    fn supersession_head<'a>(
        &'a self,
        evidence: &'a <C::Refresh as EffectRefreshMode>::Evidence,
    ) -> ComponentFuture<'a, Option<HistoryObject>>;

    /// Performs one invocation with the exact committed authorization origin.
    ///
    /// Bindings that do not consume origin evidence inherit the ordinary
    /// invocation. Origin-sensitive adapters override this method.
    fn invoke_authorized<'a>(
        &'a self,
        request: &'a C::Request,
        _authorization_ref: &'a RecordRef,
        _authorization: &'a ExternalAccessAuthorized,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<C>> {
        self.invoke(request)
    }
}

/// Process-assembly source of current concrete Effect targets.
pub trait RuntimeEffectPhysicalBindingSource<C>: Send + Sync + 'static
where
    C: RuntimeEffectCapability,
{
    /// Exact target-bound adapter type returned by this source.
    type Binding: RuntimeEffectPhysicalBinding<C>;

    /// Selects one current target-bound Effect binding before authorization.
    /// This is a process-local qualification lookup and must not enter the
    /// provider, resource, signer, or other protected target.
    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>>;
}

mod physical_binding_kind {
    pub trait Sealed {}
}

/// Sealed kind marker for one opaque qualified physical binding.
#[doc(hidden)]
pub trait PhysicalBindingKind: physical_binding_kind::Sealed + Send + Sync + 'static {
    /// Whether this kind is an Effect rather than a Read.
    const EFFECT: bool;
}

/// Kind marker for a qualified Read physical binding.
pub enum ReadPhysicalBindingKind {}

/// Kind marker for a qualified Effect physical binding.
pub enum EffectPhysicalBindingKind {}

impl physical_binding_kind::Sealed for ReadPhysicalBindingKind {}
impl physical_binding_kind::Sealed for EffectPhysicalBindingKind {}

impl PhysicalBindingKind for ReadPhysicalBindingKind {
    const EFFECT: bool = false;
}

impl PhysicalBindingKind for EffectPhysicalBindingKind {
    const EFFECT: bool = true;
}

/// Opaque non-cloneable coupling of one frozen request, public certificate,
/// and target-specific private invocation handle.
///
/// Construction and field access remain inside qualified process assembly.
/// Store and replay receive only [`Self::public_certificate`].
pub struct QualifiedPhysicalBinding<K> {
    core: QualifiedPhysicalBindingCore,
    _kind: PhantomData<fn() -> K>,
}

impl<K> std::fmt::Debug for QualifiedPhysicalBinding<K> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QualifiedPhysicalBinding")
            .field("request", &self.core.request)
            .field("public_certificate", &self.core.public_certificate)
            .finish_non_exhaustive()
    }
}

impl<K> QualifiedPhysicalBinding<K> {
    /// Returns the exact frozen canonical request to persist at authorization.
    pub const fn request(&self) -> &CanonicalJsonValue {
        &self.core.request
    }

    /// Returns the immutable public certificate to persist at authorization.
    pub const fn public_certificate(&self) -> &HistoryObject {
        &self.core.public_certificate
    }
}

#[allow(dead_code)]
trait ErasedStateCallbacks: Send + Sync {
    fn kind(&self) -> mfm_spec::structured::StructuredExecutionKind;
    fn semantic_contract_ref(&self) -> Result<ContentRef>;
    fn invoke_pure(&self, input: &CanonicalJsonValue) -> Result<CanonicalJsonValue>;
    fn author_request(&self, input: &CanonicalJsonValue) -> Result<CanonicalJsonValue>;
    fn settle_observation(
        &self,
        input: &CanonicalJsonValue,
        observation: &CanonicalJsonValue,
    ) -> Result<CanonicalJsonValue>;
    fn invoke_pure_runtime(
        &self,
        input: &CanonicalJsonValue,
    ) -> RuntimeProcessResult<QualifiedStateProposalValue>;
    fn author_request_runtime(
        &self,
        input: &CanonicalJsonValue,
    ) -> RuntimeProcessResult<CanonicalJsonValue>;
    fn settle_observation_runtime(
        &self,
        input: &CanonicalJsonValue,
        observation: &CanonicalJsonValue,
    ) -> RuntimeProcessResult<RuntimeStateSettlement>;
}

enum RuntimeStateSettlement {
    Proposed(QualifiedStateProposalValue),
    InvalidEvidence,
}

struct TypedStateCallbacks<S: State> {
    callbacks: StructuredStateCallbacks<S>,
}

impl<S: State> ErasedStateCallbacks for TypedStateCallbacks<S> {
    fn kind(&self) -> mfm_spec::structured::StructuredExecutionKind {
        self.callbacks.kind()
    }

    fn semantic_contract_ref(&self) -> Result<ContentRef> {
        state_contract::<S>()
            .map(|contract| contract.state_contract_ref)
            .map_err(|error| CertifyError::Certification(error.to_string()))
    }

    fn invoke_pure(&self, input: &CanonicalJsonValue) -> Result<CanonicalJsonValue> {
        let input = decode_process_value::<S::Input>(input)?;
        let outcome = catch_unwind(AssertUnwindSafe(|| self.callbacks.invoke_pure(&input)))
            .map_err(|_| {
                CertifyError::Certification("qualified state callback panicked".to_owned())
            })?
            .ok_or_else(|| {
                CertifyError::Certification(
                    "registered state callback variant is not Pure".to_owned(),
                )
            })?;
        encode_process_value(&outcome)
    }

    fn author_request(&self, input: &CanonicalJsonValue) -> Result<CanonicalJsonValue> {
        let input = decode_process_value::<S::Input>(input)?;
        let request = catch_unwind(AssertUnwindSafe(|| self.callbacks.author_request(&input)))
            .map_err(|_| {
                CertifyError::Certification("qualified request callback panicked".to_owned())
            })?
            .ok_or_else(|| {
                CertifyError::Certification(
                    "registered state callback has no access request".to_owned(),
                )
            })?;
        encode_process_value(&request)
    }

    fn settle_observation(
        &self,
        input: &CanonicalJsonValue,
        observation: &CanonicalJsonValue,
    ) -> Result<CanonicalJsonValue> {
        let input = decode_process_value::<S::Input>(input)?;
        let observation =
            decode_process_value::<CommittedObservation<S::Returned, S::SafeFailure>>(observation)?;
        let settlement = catch_unwind(AssertUnwindSafe(|| {
            self.callbacks.settle_observation(&input, &observation)
        }))
        .map_err(|_| {
            CertifyError::Certification("qualified settlement callback panicked".to_owned())
        })?
        .ok_or_else(|| {
            CertifyError::Certification(
                "registered state callback has no observation settlement".to_owned(),
            )
        })?;
        encode_process_value(&settlement)
    }

    fn invoke_pure_runtime(
        &self,
        input: &CanonicalJsonValue,
    ) -> RuntimeProcessResult<QualifiedStateProposalValue> {
        let input = decode_runtime_process_value::<S::Input>(input)?;
        let outcome = catch_unwind(AssertUnwindSafe(|| self.callbacks.invoke_pure(&input)))
            .map_err(|_| RuntimeProcessFaultCode::Callback)?
            .ok_or(RuntimeProcessFaultCode::Contract)?;
        erase_runtime_state_proposal(outcome)
    }

    fn author_request_runtime(
        &self,
        input: &CanonicalJsonValue,
    ) -> RuntimeProcessResult<CanonicalJsonValue> {
        let input = decode_runtime_process_value::<S::Input>(input)?;
        let request = catch_unwind(AssertUnwindSafe(|| self.callbacks.author_request(&input)))
            .map_err(|_| RuntimeProcessFaultCode::Callback)?
            .ok_or(RuntimeProcessFaultCode::Contract)?;
        encode_runtime_process_value(&request)
    }

    fn settle_observation_runtime(
        &self,
        input: &CanonicalJsonValue,
        observation: &CanonicalJsonValue,
    ) -> RuntimeProcessResult<RuntimeStateSettlement> {
        let input = decode_runtime_process_value::<S::Input>(input)?;
        let observation = decode_runtime_process_value::<
            CommittedObservation<S::Returned, S::SafeFailure>,
        >(observation)?;
        let settlement = catch_unwind(AssertUnwindSafe(|| {
            self.callbacks.settle_observation(&input, &observation)
        }))
        .map_err(|_| RuntimeProcessFaultCode::Callback)?
        .ok_or(RuntimeProcessFaultCode::Contract)?;
        match settlement {
            StateSettlement::Proposed(proposal) => {
                erase_runtime_state_proposal(proposal).map(RuntimeStateSettlement::Proposed)
            }
            StateSettlement::InvalidEvidence => Ok(RuntimeStateSettlement::InvalidEvidence),
        }
    }
}

fn erase_runtime_state_proposal<Output: Serialize, Failure: Serialize>(
    proposal: mfm_spec::structured::ProposedStateOutcome<Output, Failure>,
) -> RuntimeProcessResult<QualifiedStateProposalValue> {
    let (value, facts) = proposal.into_parts();
    match value {
        ProposedStateValue::Success(value) => Ok(QualifiedStateProposalValue::Success {
            value: encode_runtime_process_value(&value)?,
            facts,
        }),
        ProposedStateValue::Failure(value) => {
            if !facts.as_slice().is_empty() {
                return Err(RuntimeProcessFaultCode::Candidate);
            }
            Ok(QualifiedStateProposalValue::Failure(
                encode_runtime_process_value(&value)?,
            ))
        }
    }
}

fn encode_runtime_process_value<T: Serialize>(
    value: &T,
) -> RuntimeProcessResult<CanonicalJsonValue> {
    let value = serde_json::to_value(value).map_err(|_| RuntimeProcessFaultCode::Codec)?;
    CanonicalJsonValue::new(value).map_err(|_| RuntimeProcessFaultCode::Codec)
}

fn decode_runtime_process_value<T>(value: &CanonicalJsonValue) -> RuntimeProcessResult<T>
where
    T: DeserializeOwned + Serialize,
{
    let decoded = serde_json::from_value(value.as_json().clone())
        .map_err(|_| RuntimeProcessFaultCode::Codec)?;
    if encode_runtime_process_value(&decoded)? != *value {
        return Err(RuntimeProcessFaultCode::Codec);
    }
    Ok(decoded)
}

#[allow(dead_code)]
trait ErasedReadCapabilityImplementation: Send + Sync {
    fn capability_type_id(&self) -> TypeId;
    fn semantic_contract_ref(&self) -> Result<ContentRef>;
    fn validate_request(&self, request: &CanonicalJsonValue) -> Result<()>;
    fn validate_returned(&self, returned: &CanonicalJsonValue) -> Result<()>;
    fn validate_safe_failure(&self, failure: &CanonicalJsonValue) -> Result<()>;
}

#[allow(dead_code)]
struct TypedReadCapabilityImplementation<C: RuntimeReadCapability> {
    implementation: Arc<dyn ReadCapabilityImplementation<C>>,
}

impl<C: RuntimeReadCapability> ErasedReadCapabilityImplementation
    for TypedReadCapabilityImplementation<C>
{
    fn capability_type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn semantic_contract_ref(&self) -> Result<ContentRef> {
        Ok(runtime_read_capability_contract::<C>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            .content_ref()?)
    }

    fn validate_request(&self, request: &CanonicalJsonValue) -> Result<()> {
        let request = decode_process_value::<C::Request>(request)?;
        self.implementation.validate_request(&request).map_err(|_| {
            CertifyError::Certification(
                "qualified capability rejected the exact typed request".to_owned(),
            )
        })
    }

    fn validate_returned(&self, returned: &CanonicalJsonValue) -> Result<()> {
        let returned = decode_process_value::<C::Returned>(returned)?;
        self.implementation
            .validate_returned(&returned)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Read capability rejected returned evidence".to_owned(),
                )
            })
    }

    fn validate_safe_failure(&self, failure: &CanonicalJsonValue) -> Result<()> {
        let failure = decode_process_value::<C::SafeFailure>(failure)?;
        self.implementation
            .validate_safe_failure(&failure)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Read capability rejected safe-failure evidence".to_owned(),
                )
            })
    }
}

struct KernelPriorRunFactSelectionImplementation {
    fault_code: StableId,
}

impl ReadCapabilityImplementation<PriorRunFactSelectionCapability>
    for KernelPriorRunFactSelectionImplementation
{
    fn validate_request(
        &self,
        request: &mfm_facts::FactSelectionRequest,
    ) -> std::result::Result<(), CapabilityContractFault> {
        let selector = request.selector_contract_ref();
        let expected_selector = mfm_facts::prior_run_fact_selector_contract_ref();
        if request.admitted_source_manifest_ref().is_err()
            || request.scan_bounds().is_err()
            || request.queries().is_err()
            || !matches!((selector, expected_selector), (Ok(left), Ok(right)) if left == right)
        {
            return Err(CapabilityContractFault::new(self.fault_code.clone()));
        }
        Ok(())
    }

    fn validate_returned(
        &self,
        returned: &mfm_facts::FactSelectionReadResponse,
    ) -> std::result::Result<(), CapabilityContractFault> {
        let response: PriorRunFactSelectionResponse =
            serde_json::from_str(returned.canonical_response_json())
                .map_err(|_| CapabilityContractFault::new(self.fault_code.clone()))?;
        let canonical = mfm_journal::structured::canonical_json(&response)
            .map_err(|_| CapabilityContractFault::new(self.fault_code.clone()))?;
        if response.version != PriorRunFactSelectionResponse::VERSION
            || canonical.as_str() != returned.canonical_response_json()
        {
            return Err(CapabilityContractFault::new(self.fault_code.clone()));
        }
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &mfm_facts::FactSelectionReadFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

#[allow(dead_code)]
trait ErasedEffectCapabilityImplementation: Send + Sync {
    fn capability_type_id(&self) -> TypeId;
    fn refresh_mode_type_id(&self) -> TypeId;
    fn semantic_contract_ref(&self) -> Result<ContentRef>;
    fn validate_request(&self, request: &CanonicalJsonValue) -> Result<()>;
    fn validate_returned(&self, returned: &CanonicalJsonValue) -> Result<()>;
    fn validate_safe_failure(&self, failure: &CanonicalJsonValue) -> Result<()>;
    fn validate_superseded_before_entry(&self, evidence: &CanonicalJsonValue) -> Result<()>;
    fn validate_entry_unknown(&self, fault: &AccessFaultCode) -> Result<()>;
    fn validate_integrity_fault(&self, fault: &AccessFaultCode) -> Result<()>;
}

#[allow(dead_code)]
struct TypedEffectCapabilityImplementation<C: RuntimeEffectCapability> {
    implementation: Arc<dyn EffectCapabilityImplementation<C>>,
}

impl<C: RuntimeEffectCapability> ErasedEffectCapabilityImplementation
    for TypedEffectCapabilityImplementation<C>
{
    fn capability_type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn refresh_mode_type_id(&self) -> TypeId {
        TypeId::of::<C::Refresh>()
    }

    fn semantic_contract_ref(&self) -> Result<ContentRef> {
        Ok(runtime_effect_capability_contract::<C>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            .content_ref()?)
    }

    fn validate_request(&self, request: &CanonicalJsonValue) -> Result<()> {
        let request = decode_process_value::<C::Request>(request)?;
        self.implementation.validate_request(&request).map_err(|_| {
            CertifyError::Certification(
                "qualified Effect capability rejected the exact typed request".to_owned(),
            )
        })
    }

    fn validate_returned(&self, returned: &CanonicalJsonValue) -> Result<()> {
        let returned = decode_process_value::<C::Returned>(returned)?;
        self.implementation
            .validate_returned(&returned)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Effect capability rejected returned evidence".to_owned(),
                )
            })
    }

    fn validate_safe_failure(&self, failure: &CanonicalJsonValue) -> Result<()> {
        let failure = decode_process_value::<C::SafeFailure>(failure)?;
        self.implementation
            .validate_safe_failure(&failure)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Effect capability rejected safe-failure evidence".to_owned(),
                )
            })
    }

    fn validate_superseded_before_entry(&self, evidence: &CanonicalJsonValue) -> Result<()> {
        let evidence =
            decode_process_value::<<C::Refresh as EffectRefreshMode>::Evidence>(evidence)?;
        self.implementation
            .validate_superseded_before_entry(&evidence)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Effect capability rejected supersession evidence".to_owned(),
                )
            })
    }

    fn validate_entry_unknown(&self, fault: &AccessFaultCode) -> Result<()> {
        self.implementation
            .validate_entry_unknown(fault)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Effect capability rejected entry-unknown evidence".to_owned(),
                )
            })
    }

    fn validate_integrity_fault(&self, fault: &AccessFaultCode) -> Result<()> {
        self.implementation
            .validate_integrity_fault(fault)
            .map_err(|_| {
                CertifyError::Certification(
                    "qualified Effect capability rejected integrity evidence".to_owned(),
                )
            })
    }
}

struct QualifiedPhysicalBindingCore {
    request: CanonicalJsonValue,
    public_certificate: HistoryObject,
    expected_authorization: ExpectedAuthorization,
    invocation: Box<dyn ErasedBoundAccessInvocation>,
    process_identity: Arc<()>,
    integrity_fault_code: StableId,
}

struct ExpectedAuthorization {
    run_id: RunId,
    occurrence_id: OccurrenceId,
    occurrence_path_ref: ContentRef,
    semantic_call_id: SemanticCallId,
    state_input_ref: LexicalValueRef,
    semantic_head: SemanticHead,
    attempt_ordinal: u64,
    access_kind: AccessKind,
    capability_contract_ref: ContentRef,
    capability_implementation_ref: ContentRef,
    adapter_contract_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
    request_digest: RequestDigest,
    request: TypedValueRef,
    physical_binding_ref: ContentRef,
    stable_resource_lineage_contract_ref: Option<ContentRef>,
}

impl ExpectedAuthorization {
    fn new(
        selection: PhysicalBindingSelection<'_>,
        access_kind: AccessKind,
        request: &CanonicalJsonValue,
        request_contract_ref: &ContentRef,
        request_schema: &mfm_ids::SchemaId,
        public_certificate: &HistoryObject,
    ) -> Result<Self> {
        let canonical = request
            .canonical_json()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let request = TypedValueRef {
            contract_ref: request_contract_ref.clone(),
            value_ref: ContentRef::new(
                request_schema.clone(),
                mfm_ids::ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(canonical.as_bytes()),
                ),
            )
            .map_err(|error| CertifyError::Certification(error.to_string()))?,
        };
        Ok(Self {
            run_id: selection.run_id.clone(),
            occurrence_id: selection.occurrence_id.clone(),
            occurrence_path_ref: selection.occurrence_path_ref.clone(),
            semantic_call_id: selection.semantic_call_id.clone(),
            state_input_ref: selection.state_input_ref.clone(),
            semantic_head: selection.semantic_head.clone(),
            attempt_ordinal: selection.attempt_ordinal,
            access_kind,
            capability_contract_ref: selection.capability_contract_ref.clone(),
            capability_implementation_ref: selection.capability_implementation_ref.clone(),
            adapter_contract_ref: selection.adapter_contract_ref.clone(),
            adapter_implementation_ref: selection.adapter_implementation_ref.clone(),
            request_digest: RequestDigest::from_digest(sha256_digest_bytes(canonical.as_bytes())),
            request,
            physical_binding_ref: public_certificate.content_ref.clone(),
            stable_resource_lineage_contract_ref: selection
                .stable_resource_lineage_contract_ref
                .cloned(),
        })
    }

    fn matches(
        &self,
        authorization_ref: &RecordRef,
        authorization: &ExternalAccessAuthorized,
    ) -> bool {
        authorization_ref.run_id == self.run_id
            && authorization.occurrence_id == self.occurrence_id
            && authorization.occurrence_path_ref == self.occurrence_path_ref
            && authorization.semantic_call_id == self.semantic_call_id
            && authorization.state_input_ref == self.state_input_ref
            && authorization.semantic_head == self.semantic_head
            && authorization.attempt_ordinal == self.attempt_ordinal
            && authorization.access_kind == self.access_kind
            && authorization.capability_contract_ref == self.capability_contract_ref
            && authorization.capability_implementation_ref == self.capability_implementation_ref
            && authorization.adapter_contract_ref == self.adapter_contract_ref
            && authorization.adapter_implementation_ref == self.adapter_implementation_ref
            && authorization.request_digest == self.request_digest
            && authorization.request == self.request
            && authorization.physical_binding_ref == self.physical_binding_ref
            && authorization.stable_resource_lineage_contract_ref
                == self.stable_resource_lineage_contract_ref
            && mfm_journal::structured::derive_access_attempt_id(&ExpectedAccessAttemptPreimage {
                run_id: &self.run_id,
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
                stable_resource_lineage_contract_ref: &authorization
                    .stable_resource_lineage_contract_ref,
            })
            .ok()
            .is_some_and(|expected| expected == authorization.access_attempt_id)
    }
}

#[derive(Serialize)]
struct ExpectedAccessAttemptPreimage<'a> {
    run_id: &'a RunId,
    occurrence_id: &'a OccurrenceId,
    occurrence_path_ref: &'a ContentRef,
    semantic_call_id: &'a SemanticCallId,
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

trait ErasedBoundAccessInvocation: Send {
    fn invoke(
        self: Box<Self>,
        authorization: Option<NewlyAppendedAuthorization>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'static, QualifiedAccessCompletion>;
}

struct BoundReadInvocation<C, B>
where
    C: RuntimeReadCapability,
    B: RuntimeReadPhysicalBinding<C>,
{
    request: C::Request,
    binding: Arc<B>,
    implementation: Arc<dyn ErasedReadCapabilityImplementation>,
}

impl<C, B> ErasedBoundAccessInvocation for BoundReadInvocation<C, B>
where
    C: RuntimeReadCapability,
    B: RuntimeReadPhysicalBinding<C>,
{
    fn invoke(
        self: Box<Self>,
        authorization: Option<NewlyAppendedAuthorization>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'static, QualifiedAccessCompletion> {
        let authorization = authorization.map(|authorization| {
            (
                authorization.authorization_ref().clone(),
                authorization.authorization().clone(),
            )
        });
        Box::pin(async move {
            let invocation = AssertUnwindSafe(async {
                drop(authorization);
                match self.binding.invoke(&self.request).await {
                    ReadAdapterCompletion::Returned(value) => {
                        let value = encode_process_value(&value)?;
                        self.implementation.validate_returned(&value)?;
                        Ok::<QualifiedAccessCompletion, CertifyError>(
                            QualifiedAccessCompletion::Returned(value),
                        )
                    }
                    ReadAdapterCompletion::SafeFailure(value) => {
                        let value = encode_process_value(&value)?;
                        self.implementation.validate_safe_failure(&value)?;
                        Ok(QualifiedAccessCompletion::SafeFailure(value))
                    }
                    ReadAdapterCompletion::IntegrityFault(fault) => Ok(
                        QualifiedAccessCompletion::IntegrityFault(fault.code().clone()),
                    ),
                }
            })
            .catch_unwind()
            .await;
            match invocation {
                Ok(Ok(completion)) => completion,
                Ok(Err(_)) | Err(_) => {
                    QualifiedAccessCompletion::IntegrityFault(integrity_fault_code)
                }
            }
        })
    }
}

struct BoundEffectInvocation<C, B>
where
    C: RuntimeEffectCapability,
    B: RuntimeEffectPhysicalBinding<C>,
{
    request: C::Request,
    binding: Arc<B>,
    implementation: Arc<dyn ErasedEffectCapabilityImplementation>,
}

impl<C, B> ErasedBoundAccessInvocation for BoundEffectInvocation<C, B>
where
    C: RuntimeEffectCapability,
    B: RuntimeEffectPhysicalBinding<C>,
{
    fn invoke(
        self: Box<Self>,
        authorization: Option<NewlyAppendedAuthorization>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'static, QualifiedAccessCompletion> {
        let authorization = authorization.map(|authorization| {
            (
                authorization.authorization_ref().clone(),
                authorization.authorization().clone(),
            )
        });
        Box::pin(async move {
            let invocation = AssertUnwindSafe(async {
                let completion = match authorization.as_ref() {
                    Some((authorization_ref, authorization)) => {
                        self.binding
                            .invoke_authorized(&self.request, authorization_ref, authorization)
                            .await
                    }
                    None => self.binding.invoke(&self.request).await,
                };
                match completion {
                    EffectAdapterCompletion::Returned(value) => {
                        let value = encode_process_value(&value)?;
                        self.implementation.validate_returned(&value)?;
                        Ok::<QualifiedAccessCompletion, CertifyError>(
                            QualifiedAccessCompletion::Returned(value),
                        )
                    }
                    EffectAdapterCompletion::SafeFailure(value) => {
                        let value = encode_process_value(&value)?;
                        self.implementation.validate_safe_failure(&value)?;
                        Ok(QualifiedAccessCompletion::SafeFailure(value))
                    }
                    EffectAdapterCompletion::SupersededBeforeEntry(evidence) => {
                        let public_lineage_head = self
                            .binding
                            .supersession_head(&evidence)
                            .await
                            .ok_or_else(|| {
                            CertifyError::Certification(
                                "bound Effect omitted its supersession lineage head".to_owned(),
                            )
                        })?;
                        public_lineage_head.validate().map_err(|_| {
                            CertifyError::Certification(
                                "bound Effect returned an invalid public lineage head".to_owned(),
                            )
                        })?;
                        let evidence = encode_process_value(&evidence)?;
                        self.implementation
                            .validate_superseded_before_entry(&evidence)?;
                        Ok(QualifiedAccessCompletion::SupersededBeforeEntry {
                            evidence,
                            public_lineage_head: Box::new(public_lineage_head),
                        })
                    }
                    EffectAdapterCompletion::EntryUnknown(fault) => {
                        self.implementation.validate_entry_unknown(&fault)?;
                        Ok(QualifiedAccessCompletion::EntryUnknown(
                            fault.code().clone(),
                        ))
                    }
                    EffectAdapterCompletion::IntegrityFault(fault) => {
                        self.implementation.validate_integrity_fault(&fault)?;
                        Ok(QualifiedAccessCompletion::IntegrityFault(
                            fault.code().clone(),
                        ))
                    }
                }
            })
            .catch_unwind()
            .await;
            match invocation {
                Ok(Ok(completion)) => completion,
                Ok(Err(_)) | Err(_) => {
                    QualifiedAccessCompletion::IntegrityFault(integrity_fault_code)
                }
            }
        })
    }
}

struct PriorRunFactScanInvocation {
    request: mfm_facts::FactSelectionRequest,
    implementation: Arc<dyn ErasedReadCapabilityImplementation>,
}

impl ErasedBoundAccessInvocation for PriorRunFactScanInvocation {
    fn invoke(
        self: Box<Self>,
        authorization: Option<NewlyAppendedAuthorization>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'static, QualifiedAccessCompletion> {
        Box::pin(async move {
            let Some(authorization) = authorization else {
                return QualifiedAccessCompletion::IntegrityFault(integrity_fault_code);
            };
            let Some(scan) = authorization.invoke_prior_run_fact_scan(self.request) else {
                return QualifiedAccessCompletion::IntegrityFault(integrity_fault_code);
            };
            let invocation = AssertUnwindSafe(async {
                match scan.await {
                    PriorRunFactScanCompletion::Returned(value) => {
                        let value = encode_process_value(&value)?;
                        self.implementation.validate_returned(&value)?;
                        Ok::<QualifiedAccessCompletion, CertifyError>(
                            QualifiedAccessCompletion::Returned(value),
                        )
                    }
                    PriorRunFactScanCompletion::SafeFailure(value) => {
                        let value = encode_process_value(&value)?;
                        self.implementation.validate_safe_failure(&value)?;
                        Ok(QualifiedAccessCompletion::SafeFailure(value))
                    }
                    PriorRunFactScanCompletion::IntegrityFault(code) => {
                        Ok(QualifiedAccessCompletion::IntegrityFault(code))
                    }
                }
            })
            .catch_unwind()
            .await;
            match invocation {
                Ok(Ok(completion)) => completion,
                Ok(Err(_)) | Err(_) => {
                    QualifiedAccessCompletion::IntegrityFault(integrity_fault_code)
                }
            }
        })
    }
}

trait ErasedReadPhysicalBindingSource: Send + Sync {
    fn capability_type_id(&self) -> TypeId;
    fn semantic_contract_ref(&self) -> Result<ContentRef>;
    fn resolve<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: CanonicalJsonValue,
        implementation: Arc<dyn ErasedReadCapabilityImplementation>,
        process_identity: Arc<()>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'a, Result<Option<QualifiedPhysicalBindingCore>>>;
}

struct TypedReadPhysicalBindingSource<C, S>
where
    C: RuntimeReadCapability,
    S: RuntimeReadPhysicalBindingSource<C>,
{
    source: Arc<S>,
    _contract: PhantomData<fn() -> C>,
}

impl<C, S> ErasedReadPhysicalBindingSource for TypedReadPhysicalBindingSource<C, S>
where
    C: RuntimeReadCapability,
    S: RuntimeReadPhysicalBindingSource<C>,
{
    fn capability_type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn semantic_contract_ref(&self) -> Result<ContentRef> {
        Ok(S::Binding::contract()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            .content_ref()?)
    }

    fn resolve<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: CanonicalJsonValue,
        implementation: Arc<dyn ErasedReadCapabilityImplementation>,
        process_identity: Arc<()>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'a, Result<Option<QualifiedPhysicalBindingCore>>> {
        Box::pin(async move {
            let typed_request = decode_process_value::<C::Request>(&request)?;
            let Some(binding) = self.source.current_binding(selection, &typed_request).await else {
                return Ok(None);
            };
            binding.public_certificate().validate().map_err(|_| {
                CertifyError::Certification(
                    "Read physical binding returned an invalid public certificate".to_owned(),
                )
            })?;
            let expected_authorization = ExpectedAuthorization::new(
                selection,
                AccessKind::Read,
                &request,
                &mfm_spec::structured::structured_value_contract_ref::<C::Request>()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?,
                &structured_value_contract::<C::Request>()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?
                    .schema_id(),
                binding.public_certificate(),
            )?;
            Ok(Some(QualifiedPhysicalBindingCore {
                request,
                public_certificate: binding.public_certificate().clone(),
                expected_authorization,
                invocation: Box::new(BoundReadInvocation::<C, S::Binding> {
                    request: typed_request,
                    binding,
                    implementation,
                }),
                process_identity,
                integrity_fault_code,
            }))
        })
    }
}

trait ErasedEffectPhysicalBindingSource: Send + Sync {
    fn capability_type_id(&self) -> TypeId;
    fn refresh_mode_type_id(&self) -> TypeId;
    fn semantic_contract_ref(&self) -> Result<ContentRef>;
    fn resolve<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: CanonicalJsonValue,
        implementation: Arc<dyn ErasedEffectCapabilityImplementation>,
        process_identity: Arc<()>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'a, Result<Option<QualifiedPhysicalBindingCore>>>;
}

struct TypedEffectPhysicalBindingSource<C, S>
where
    C: RuntimeEffectCapability,
    S: RuntimeEffectPhysicalBindingSource<C>,
{
    source: Arc<S>,
    _contract: PhantomData<fn() -> C>,
}

impl<C, S> ErasedEffectPhysicalBindingSource for TypedEffectPhysicalBindingSource<C, S>
where
    C: RuntimeEffectCapability,
    S: RuntimeEffectPhysicalBindingSource<C>,
{
    fn capability_type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn refresh_mode_type_id(&self) -> TypeId {
        TypeId::of::<C::Refresh>()
    }

    fn semantic_contract_ref(&self) -> Result<ContentRef> {
        Ok(S::Binding::contract()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            .content_ref()?)
    }

    fn resolve<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        request: CanonicalJsonValue,
        implementation: Arc<dyn ErasedEffectCapabilityImplementation>,
        process_identity: Arc<()>,
        integrity_fault_code: StableId,
    ) -> ComponentFuture<'a, Result<Option<QualifiedPhysicalBindingCore>>> {
        Box::pin(async move {
            let typed_request = decode_process_value::<C::Request>(&request)?;
            let Some(binding) = self.source.current_binding(selection, &typed_request).await else {
                return Ok(None);
            };
            binding.public_certificate().validate().map_err(|_| {
                CertifyError::Certification(
                    "Effect physical binding returned an invalid public certificate".to_owned(),
                )
            })?;
            let expected_authorization = ExpectedAuthorization::new(
                selection,
                AccessKind::Effect,
                &request,
                &mfm_spec::structured::structured_value_contract_ref::<C::Request>()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?,
                &structured_value_contract::<C::Request>()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?
                    .schema_id(),
                binding.public_certificate(),
            )?;
            Ok(Some(QualifiedPhysicalBindingCore {
                request,
                public_certificate: binding.public_certificate().clone(),
                expected_authorization,
                invocation: Box::new(BoundEffectInvocation::<C, S::Binding> {
                    request: typed_request,
                    binding,
                    implementation,
                }),
                process_identity,
                integrity_fault_code,
            }))
        })
    }
}

#[allow(dead_code)]
trait ErasedBoundedInvoker: Send + Sync {
    fn contract_type_id(&self) -> TypeId;
    fn invoke<'a>(
        &'a self,
        request: &'a (dyn Any + Send + Sync),
    ) -> ComponentFuture<'a, Result<Box<dyn Any + Send + Sync>>>;
}

#[allow(dead_code)]
struct TypedBoundedInvoker<C, I> {
    invoker: Arc<I>,
    _contract: PhantomData<fn() -> C>,
}

impl<C, I> ErasedBoundedInvoker for TypedBoundedInvoker<C, I>
where
    C: mfm_capabilities::BoundedComponentContract,
    I: BoundedComponentInvoker<C>,
{
    fn contract_type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn invoke<'a>(
        &'a self,
        request: &'a (dyn Any + Send + Sync),
    ) -> ComponentFuture<'a, Result<Box<dyn Any + Send + Sync>>> {
        let request = request.downcast_ref::<C::Request>().ok_or_else(|| {
            CertifyError::Certification("bounded component request type mismatch".to_owned())
        });
        Box::pin(async move {
            let completion = self.invoker.invoke(request?).await;
            Ok(Box::new(completion) as Box<dyn Any + Send + Sync>)
        })
    }
}

#[derive(Clone)]
#[allow(dead_code)]
enum ProcessHandle {
    State(Arc<dyn ErasedStateCallbacks>),
    ReadCapability(Arc<dyn ErasedReadCapabilityImplementation>),
    EffectCapability(Arc<dyn ErasedEffectCapabilityImplementation>),
    ReadPhysicalBindingSource(Arc<dyn ErasedReadPhysicalBindingSource>),
    PriorRunFactScannerBindingSource,
    EffectPhysicalBindingSource(Arc<dyn ErasedEffectPhysicalBindingSource>),
    Signer(Arc<dyn ErasedBoundedInvoker>),
    Resource(Arc<dyn ErasedBoundedInvoker>),
}

#[derive(Clone)]
struct RegisteredProcessComponent {
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    implementation_contract_ref: ContentRef,
    handle: ProcessHandle,
}

impl std::fmt::Debug for RegisteredProcessComponent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredProcessComponent")
            .field("component_kind", &self.component_kind)
            .field("semantic_contract_ref", &self.semantic_contract_ref)
            .field(
                "implementation_contract_ref",
                &self.implementation_contract_ref,
            )
            .finish_non_exhaustive()
    }
}

impl RegisteredProcessComponent {
    fn identity(&self) -> QualifiedComponentIdentity {
        QualifiedComponentIdentity {
            component_kind: self.component_kind,
            semantic_contract_ref: self.semantic_contract_ref.clone(),
            implementation_contract_ref: self.implementation_contract_ref.clone(),
        }
    }
}

#[allow(dead_code)]
fn encode_process_value<T: Serialize>(value: &T) -> Result<CanonicalJsonValue> {
    let value = serde_json::to_value(value)
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    CanonicalJsonValue::new(value).map_err(Into::into)
}

#[allow(dead_code)]
fn decode_process_value<T>(value: &CanonicalJsonValue) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    let decoded = serde_json::from_value(value.as_json().clone())
        .map_err(|_| CertifyError::Certification("typed process value decode failed".to_owned()))?;
    if encode_process_value(&decoded)? != *value {
        return Err(CertifyError::Certification(
            "typed process value did not round-trip exact canonical JSON".to_owned(),
        ));
    }
    Ok(decoded)
}

/// Process-private expansion and certification registry data.
#[derive(Clone, Default)]
struct StructuredCertificationRegistry {
    children: BTreeMap<ContentRef, AuthoredStructuredProgram>,
    states: BTreeMap<ContentRef, RegisteredState>,
    live_components:
        BTreeMap<(StructuredComponentKind, ContentRef), StructuredLiveComponentContract>,
    component_implementations: BTreeMap<(StructuredComponentKind, ContentRef), ContentRef>,
    closed_sums: BTreeMap<ContentRef, ClosedSumContract>,
    fact_descriptors: BTreeMap<ContentRef, StructuredFactDescriptor>,
    value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
    capability_recipes: BTreeMap<ContentRef, AuthoredStructuredProgram>,
    policy_recipes: BTreeMap<ContentRef, PolicyExpansionRecipe>,
    component_objects: BTreeMap<ContentRef, RegisteredComponentObject>,
}

impl std::fmt::Debug for StructuredCertificationRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuredCertificationRegistry")
            .field("children", &self.children.len())
            .field("states", &self.states.len())
            .field("live_components", &self.live_components.len())
            .field("fact_descriptors", &self.fact_descriptors.len())
            .field("value_schemas", &self.value_schemas.len())
            .field("capability_recipes", &self.capability_recipes.len())
            .field("policy_recipes", &self.policy_recipes.len())
            .field("component_objects", &self.component_objects.len())
            .finish()
    }
}

impl StructuredCertificationRegistry {
    fn state_contract(&self, contract_ref: &ContentRef) -> Option<&StructuredStateContract> {
        self.states.get(contract_ref).map(|state| &state.contract)
    }

    /// Registers the exact canonical qualified entry-point policy object.
    fn register_entry_point_policy(
        &mut self,
        policy: &QualifiedStructuredEntryPointPolicy,
    ) -> Result<()> {
        let mut outbound_references = vec![
            ComponentObjectReference {
                object_type: stable_id(ENTRY_POINT_CONTRACT_OBJECT_TYPE)?,
                content_ref: policy.entry_point_contract_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(CERTIFIED_PROGRAM_CONTRACT_OBJECT_TYPE)?,
                content_ref: policy.certified_program_contract_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(PREDICATE_SET_OBJECT_TYPE)?,
                content_ref: policy.certification_predicate_set_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(PROFILE_OBJECT_TYPE)?,
                content_ref: policy.expansion_profile_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(POLICY_COVERAGE_CONTRACT_OBJECT_TYPE)?,
                content_ref: policy.policy_coverage_contract_ref.clone(),
            },
        ];
        for content_ref in &policy.public_input_contract_refs {
            outbound_references.push(contract_reference(content_ref)?);
        }
        outbound_references.push(contract_reference(&policy.public_output_contract_ref)?);
        outbound_references.push(contract_reference(&policy.public_failure_contract_ref)?);
        let object = component_object(
            POLICY_OBJECT_TYPE,
            policy.admission_policy_ref.clone(),
            &policy.preimage(),
            outbound_references,
        )?;
        self.register_component_object(object)
    }

    /// Adds one exact authored child program.
    fn register_child(&mut self, child: AuthoredStructuredProgram) -> Result<()> {
        let reference = child
            .content_ref()
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        insert_exact(&mut self.children, reference, child, "child program")
    }

    /// Adds one exact qualified state contract and manifest entry.
    fn register_state(&mut self, contract: StructuredStateContract) -> Result<()> {
        contract
            .validate()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let key = contract.state_contract_ref.clone();
        let contract_value: serde_json::Value = serde_json::from_slice(
            contract
                .canonical_contract_json()
                .map_err(|error| CertifyError::Certification(error.to_string()))?
                .as_bytes(),
        )
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let mut outbound_references = Vec::new();
        if let Some(capability_contract_ref) = contract.execution.capability_contract_ref() {
            outbound_references.push(ComponentObjectReference {
                object_type: semantic_component_object_type(StructuredComponentKind::Capability)?,
                content_ref: capability_contract_ref.clone(),
            });
        }
        outbound_references.extend([
            contract_reference(&contract.input_contract_ref)?,
            contract_reference(&contract.output_contract_ref)?,
        ]);
        for slot in &contract.fact_slots {
            let descriptor = self
                .fact_descriptors
                .get(slot.fact_descriptor_ref())
                .ok_or_else(|| {
                    CertifyError::Certification(
                        "state fact descriptor is not process-qualified".to_owned(),
                    )
                })?;
            let subject_contract_ref = retained_value_contract_ref(slot.subject_contract())?;
            let response_contract_ref = retained_value_contract_ref(slot.response_contract())?;
            if descriptor.subject_contract_ref != subject_contract_ref
                || descriptor.response_contract_ref != response_contract_ref
            {
                return Err(CertifyError::Certification(
                    "state fact slot differs from its qualified descriptor".to_owned(),
                ));
            }
            outbound_references.extend([
                ComponentObjectReference {
                    object_type: stable_id(FACT_DESCRIPTOR_OBJECT_TYPE)?,
                    content_ref: descriptor.descriptor_ref.clone(),
                },
                contract_reference(&subject_contract_ref)?,
                contract_reference(&response_contract_ref)?,
            ]);
        }
        collect_failure_contract_reference(&contract.failure_contract, &mut outbound_references)?;
        if let Some(requirement_ref) = &contract.capability_requirement_ref {
            outbound_references.push(contract_reference(requirement_ref)?);
        }
        let contract_object = RegisteredComponentObject {
            object: CertifiedComponentObject {
                object_type: semantic_component_object_type(StructuredComponentKind::State)?,
                content_ref: key.clone(),
                value: CanonicalJsonValue::new(contract_value)
                    .map_err(|error| CertifyError::Certification(error.to_string()))?,
            },
            outbound_references,
        };
        validate_component_object(&contract_object.object)?;
        let registered = RegisteredState { contract };
        if matches!(self.states.get(&key), Some(existing) if existing != &registered) {
            return Err(CertifyError::Certification(
                "conflicting state contract registration".to_owned(),
            ));
        }
        if matches!(
            self.component_objects.get(&key),
            Some(existing) if existing != &contract_object
        ) {
            return Err(CertifyError::Certification(
                "conflicting certified component object registration".to_owned(),
            ));
        }
        insert_exact(&mut self.states, key.clone(), registered, "state contract")?;
        self.register_component_object(contract_object)?;
        Ok(())
    }

    fn register_fact_descriptor(
        &mut self,
        descriptor: StructuredFactDescriptor,
    ) -> Result<ContentRef> {
        descriptor
            .validate()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let descriptor_ref = descriptor.descriptor_ref.clone();
        let canonical = descriptor
            .canonical_json()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let value = CanonicalJsonValue::from_canonical_json(canonical.as_bytes())?;
        let object = RegisteredComponentObject {
            object: CertifiedComponentObject {
                object_type: stable_id(FACT_DESCRIPTOR_OBJECT_TYPE)?,
                content_ref: descriptor_ref.clone(),
                value,
            },
            outbound_references: vec![
                contract_reference(&descriptor.subject_contract_ref)?,
                contract_reference(&descriptor.response_contract_ref)?,
            ],
        };
        validate_component_object(&object.object)?;
        if matches!(
            self.fact_descriptors.get(&descriptor_ref),
            Some(existing) if existing != &descriptor
        ) {
            return Err(CertifyError::Certification(
                "conflicting fact descriptor registration".to_owned(),
            ));
        }
        insert_exact(
            &mut self.fact_descriptors,
            descriptor_ref.clone(),
            descriptor,
            "fact descriptor",
        )?;
        self.register_component_object(object)?;
        Ok(descriptor_ref)
    }

    fn register_live_component(
        &mut self,
        contract: StructuredLiveComponentContract,
    ) -> Result<ContentRef> {
        contract
            .validate()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let component_kind = contract.component_kind;
        let contract_ref = contract
            .content_ref()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let mut outbound_references = Vec::new();
        if let Some(protocol) = &contract.capability_protocol {
            outbound_references.extend([
                contract_reference(protocol.request_contract_ref())?,
                contract_reference(protocol.returned_contract_ref())?,
                contract_reference(protocol.safe_failure_contract_ref())?,
                contract_reference(protocol.access_fault_contract_ref())?,
            ]);
            if let StructuredCapabilityProtocolContract::Effect {
                refresh_contract:
                    StructuredEffectRefreshContract::Refreshable {
                        refresh_evidence_contract_ref,
                        resource_lineage_contract_ref,
                    },
                ..
            } = protocol
            {
                outbound_references.extend([
                    contract_reference(refresh_evidence_contract_ref)?,
                    ComponentObjectReference {
                        object_type: semantic_component_object_type(
                            StructuredComponentKind::Resource,
                        )?,
                        content_ref: resource_lineage_contract_ref.as_ref().clone(),
                    },
                ]);
            }
        }
        outbound_references.extend(
            contract
                .dependencies
                .iter()
                .map(|dependency| {
                    Ok(ComponentObjectReference {
                        object_type: semantic_component_object_type(dependency.component_kind)?,
                        content_ref: dependency.contract_ref.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        );
        self.register_component_object(component_object(
            semantic_component_object_type(component_kind)?.as_str(),
            contract_ref.clone(),
            &contract,
            outbound_references,
        )?)?;
        insert_exact(
            &mut self.live_components,
            (component_kind, contract_ref.clone()),
            contract,
            "live semantic component",
        )?;
        Ok(contract_ref)
    }

    fn register_closed_sum(&mut self, contract: ClosedSumContract) -> Result<()> {
        contract
            .validate()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        #[derive(Serialize)]
        struct ClosedSumObject<'a> {
            selector_contract_ref: &'a ContentRef,
            variants: &'a [mfm_spec::structured::ClosedSumVariant],
        }
        let mut outbound_references = vec![contract_reference(&contract.selector_contract_ref)?];
        for variant in &contract.variants {
            for payload in &variant.payloads {
                outbound_references.push(contract_reference(&payload.contract_ref)?);
            }
        }
        let object = component_object(
            CLOSED_SUM_CONTRACT_OBJECT_TYPE,
            contract.closed_sum_contract_ref.clone(),
            &ClosedSumObject {
                selector_contract_ref: &contract.selector_contract_ref,
                variants: &contract.variants,
            },
            outbound_references,
        )?;
        insert_exact(
            &mut self.closed_sums,
            contract.selector_contract_ref.clone(),
            contract,
            "closed-sum selector contract",
        )?;
        self.register_component_object(object)
    }

    fn register_capability_recipe(
        &mut self,
        requirement_ref: ContentRef,
        recipe: AuthoredStructuredProgram,
    ) -> Result<()> {
        if self.capability_recipes.contains_key(&requirement_ref) {
            return Err(CertifyError::Certification(
                "duplicate capability expansion recipe".to_owned(),
            ));
        }
        let recipe_ref = recipe
            .content_ref()
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        if requirement_ref != capability_expansion_requirement_ref(&recipe_ref)? {
            return Err(CertifyError::Certification(
                "capability requirement does not bind the exact authored recipe".to_owned(),
            ));
        }
        self.register_component_object(component_object(
            AUTHORED_OBJECT_TYPE,
            recipe_ref.clone(),
            &recipe,
            authored_component_references(&recipe)?,
        )?)?;
        self.register_component_object(component_object(
            CAPABILITY_REQUIREMENT_OBJECT_TYPE,
            requirement_ref.clone(),
            &recipe_ref,
            vec![ComponentObjectReference {
                object_type: stable_id(AUTHORED_OBJECT_TYPE)?,
                content_ref: recipe_ref.clone(),
            }],
        )?)?;
        self.capability_recipes.insert(requirement_ref, recipe);
        Ok(())
    }

    fn register_policy_recipe(&mut self, recipe: PolicyExpansionRecipe) -> Result<()> {
        let recipe_ref = recipe
            .content_ref()
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        if self.policy_recipes.contains_key(&recipe_ref) {
            return Err(CertifyError::Certification(
                "duplicate policy expansion recipe".to_owned(),
            ));
        }
        self.register_component_object(component_object(
            POLICY_RECIPE_OBJECT_TYPE,
            recipe_ref.clone(),
            &recipe,
            policy_recipe_component_references(&recipe)?,
        )?)?;
        self.policy_recipes.insert(recipe_ref, recipe);
        Ok(())
    }

    /// Adds one exact secret-free component object.
    fn register_component_object(&mut self, object: RegisteredComponentObject) -> Result<()> {
        validate_component_object(&object.object)?;
        insert_exact(
            &mut self.component_objects,
            object.object.content_ref.clone(),
            object,
            "certified component object",
        )
    }
}

fn require_registered_object_type(
    registry: &StructuredCertificationRegistry,
    content_ref: &ContentRef,
    object_type: &str,
    label: &str,
) -> Result<()> {
    let object = registry
        .component_objects
        .get(content_ref)
        .ok_or_else(|| CertifyError::Certification(format!("{label} is not process-qualified")))?;
    if object.object.object_type != stable_id(object_type)?
        || &object.object.content_ref != content_ref
    {
        return Err(CertifyError::Certification(format!(
            "{label} has the wrong registered object type"
        )));
    }
    Ok(())
}

fn register_implementation_binding(
    registry: &mut StructuredCertificationRegistry,
    process_components: &mut BTreeMap<ProcessComponentKey, RegisteredProcessComponent>,
    descriptor: SecretFreeImplementationDescriptor,
    expected_kind: StructuredComponentKind,
    expected_semantic_contract_ref: &ContentRef,
    handle: ProcessHandle,
) -> Result<ContentRef> {
    if descriptor.component_kind != expected_kind
        || &descriptor.semantic_contract_ref != expected_semantic_contract_ref
    {
        return Err(CertifyError::Certification(
            "implementation descriptor kind or semantic contract mismatch".to_owned(),
        ));
    }
    require_registered_object_type(
        registry,
        expected_semantic_contract_ref,
        semantic_component_object_type(expected_kind)?.as_str(),
        "semantic implementation contract",
    )?;
    require_registered_object_type(
        registry,
        &descriptor.executable_identity_ref,
        EXECUTABLE_IDENTITY_OBJECT_TYPE,
        "executable identity",
    )?;
    require_registered_object_type(
        registry,
        &descriptor.qualification_artifact_ref,
        QUALIFICATION_ARTIFACT_OBJECT_TYPE,
        "qualification artifact",
    )?;

    let implementation_contract_ref = descriptor
        .content_ref()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    registry.register_component_object(component_object(
        IMPLEMENTATION_CONTRACT_OBJECT_TYPE,
        implementation_contract_ref.clone(),
        &descriptor,
        vec![
            ComponentObjectReference {
                object_type: semantic_component_object_type(expected_kind)?,
                content_ref: expected_semantic_contract_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(EXECUTABLE_IDENTITY_OBJECT_TYPE)?,
                content_ref: descriptor.executable_identity_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(QUALIFICATION_ARTIFACT_OBJECT_TYPE)?,
                content_ref: descriptor.qualification_artifact_ref.clone(),
            },
        ],
    )?)?;
    insert_exact(
        &mut registry.component_implementations,
        (expected_kind, expected_semantic_contract_ref.clone()),
        implementation_contract_ref.clone(),
        "component implementation binding",
    )?;
    let key = (
        expected_kind,
        expected_semantic_contract_ref.clone(),
        implementation_contract_ref.clone(),
    );
    if process_components.contains_key(&key) {
        return Err(CertifyError::Certification(
            "duplicate process implementation binding".to_owned(),
        ));
    }
    process_components.insert(
        key,
        RegisteredProcessComponent {
            component_kind: expected_kind,
            semantic_contract_ref: expected_semantic_contract_ref.clone(),
            implementation_contract_ref: implementation_contract_ref.clone(),
            handle,
        },
    );
    Ok(implementation_contract_ref)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntryPointDefinition {
    entry_point_id: StableId,
    entry_point_contract_ref: ContentRef,
    coverage_template: AuthoredStructuredProgram,
    profile: StructuredExpansionProfile,
    support_envelope: Option<EntryPointSupportEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntryPointSupportEnvelope {
    semantic_components: BTreeSet<(StructuredComponentKind, ContentRef)>,
    implementations: BTreeSet<(StructuredComponentKind, ContentRef, ContentRef)>,
}

/// Mutable process-assembly surface that is consumed into one immutable
/// qualified structured-program registry.
#[derive(Debug)]
pub struct ProgramRegistryBuilder {
    registry: StructuredCertificationRegistry,
    entry_points: BTreeMap<StableId, EntryPointDefinition>,
    process_components: BTreeMap<ProcessComponentKey, RegisteredProcessComponent>,
    kernel_baseline_process_components: BTreeSet<ProcessComponentKey>,
    initialization_error: Option<CertifyError>,
}

impl Default for ProgramRegistryBuilder {
    fn default() -> Self {
        let mut builder = Self {
            registry: StructuredCertificationRegistry::default(),
            entry_points: BTreeMap::new(),
            process_components: BTreeMap::new(),
            kernel_baseline_process_components: BTreeSet::new(),
            initialization_error: None,
        };
        if let Err(error) = builder.install_kernel_process_baseline() {
            builder.initialization_error = Some(error);
        }
        builder
    }
}

impl ProgramRegistryBuilder {
    /// Starts an empty qualification assembly.
    pub fn new() -> Self {
        Self::default()
    }

    fn install_kernel_process_baseline(&mut self) -> Result<()> {
        self.register_value::<FactSelectionRequest>()?;
        self.register_value::<FactSelectionReadResponse>()?;
        self.register_value::<FactSelectionReadFailure>()?;
        let executable_identity_ref =
            self.register_executable_identity(SecretFreeExecutableIdentity {
                executable_id: stable_id(KERNEL_BASELINE_EXECUTABLE_ID)?,
            })?;
        let qualification_artifact_ref =
            self.register_qualification_artifact(SecretFreeQualificationArtifact {
                qualification_id: stable_id(KERNEL_BASELINE_QUALIFICATION_ID)?,
            })?;
        let capability_contract_ref =
            prior_run_fact_selection_capability_contract()?.content_ref()?;
        let adapter_contract_ref = prior_run_fact_scanner_adapter_contract()?.content_ref()?;
        let (capability_implementation_ref, adapter_implementation_ref) = self
            .install_prior_run_fact_selection(
                SecretFreeImplementationDescriptor {
                    component_kind: StructuredComponentKind::Capability,
                    semantic_contract_ref: capability_contract_ref.clone(),
                    implementation_id: stable_id(KERNEL_FACT_CAPABILITY_IMPLEMENTATION_ID)?,
                    executable_identity_ref: executable_identity_ref.clone(),
                    qualification_artifact_ref: qualification_artifact_ref.clone(),
                },
                SecretFreeImplementationDescriptor {
                    component_kind: StructuredComponentKind::Adapter,
                    semantic_contract_ref: adapter_contract_ref.clone(),
                    implementation_id: stable_id(KERNEL_FACT_ADAPTER_IMPLEMENTATION_ID)?,
                    executable_identity_ref,
                    qualification_artifact_ref,
                },
            )?;
        self.kernel_baseline_process_components.extend([
            (
                StructuredComponentKind::Capability,
                capability_contract_ref,
                capability_implementation_ref,
            ),
            (
                StructuredComponentKind::Adapter,
                adapter_contract_ref,
                adapter_implementation_ref,
            ),
        ]);
        Ok(())
    }

    /// Registers the exact type-derived structured retained-value contract and
    /// its frozen component-evidence dependency.
    pub fn register_value<T: MfmValue>(&mut self) -> Result<ContentRef> {
        self.register_structured_value::<T>()
    }

    /// Registers one recursive structured-value definition (retained leaf or
    /// non-empty fan-out join) through the single qualification encoder.
    pub fn register_structured_value<T: mfm_program::structured::StructuredValue>(
        &mut self,
    ) -> Result<ContentRef> {
        let definition = T::structured_value_definition()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        self.register_structured_value_definition(&definition)
    }

    /// Registers one recursive structured-value definition through the single
    /// qualification encoder used by retained values and fan-out joins.
    pub fn register_structured_value_definition(
        &mut self,
        definition: &mfm_spec::structured::StructuredValueDefinition,
    ) -> Result<ContentRef> {
        match definition {
            mfm_spec::structured::StructuredValueDefinition::Retained { payload } => {
                let contract = &payload.contract;
                let schema = &payload.schema;
                let evidence_ref = component_object_evidence_contract_ref()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?;
                let evidence_value = CanonicalJsonValue::from_canonical_json(
                    component_object_evidence_contract_canonical()
                        .map_err(|error| CertifyError::Certification(error.to_string()))?
                        .as_bytes(),
                )?;
                self.registry
                    .register_component_object(RegisteredComponentObject {
                        object: CertifiedComponentObject {
                            object_type: stable_id(DATA_CONTRACT_OBJECT_TYPE)?,
                            content_ref: evidence_ref.clone(),
                            value: evidence_value,
                        },
                        outbound_references: Vec::new(),
                    })?;
                let contract_ref = retained_value_contract_ref(contract)?;
                if schema
                    .schema_id()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?
                    != *contract.schema_id()
                    || schema.semantic_type_id.as_ref() != Some(contract.semantic_type_id())
                {
                    return Err(CertifyError::Certification(
                        "registered value schema differs from its retained contract".to_owned(),
                    ));
                }
                let value = CanonicalJsonValue::from_canonical_json(
                    contract
                        .canonical_json()
                        .map_err(|error| CertifyError::Certification(error.to_string()))?
                        .as_bytes(),
                )?;
                self.registry
                    .register_component_object(RegisteredComponentObject {
                        object: CertifiedComponentObject {
                            object_type: stable_id(DATA_CONTRACT_OBJECT_TYPE)?,
                            content_ref: contract_ref.clone(),
                            value,
                        },
                        outbound_references: vec![ComponentObjectReference {
                            object_type: stable_id(DATA_CONTRACT_OBJECT_TYPE)?,
                            content_ref: evidence_ref,
                        }],
                    })?;
                insert_exact(
                    &mut self.registry.value_schemas,
                    contract_ref.clone(),
                    schema.clone(),
                    "retained value schema identity",
                )?;
                Ok(contract_ref)
            }
            mfm_spec::structured::StructuredValueDefinition::NonEmptyFanOutJoin {
                lane_success,
                lane_failure,
            } => {
                let success_ref = self.register_structured_value_definition(lane_success)?;
                if let StructuredFailureContract::Typed { contract_ref, .. } = lane_failure {
                    if !self.registry.component_objects.contains_key(contract_ref)
                        && !self.registry.value_schemas.contains_key(contract_ref)
                    {
                        return Err(CertifyError::Certification(
                            "fan-out join failure contract is not qualified".to_owned(),
                        ));
                    }
                }
                let lane_content_ref = lane_outcome_contract_ref(&success_ref, lane_failure)?;
                let mut lane_references = vec![contract_reference(&success_ref)?];
                collect_failure_contract_reference(lane_failure, &mut lane_references)?;
                self.registry
                    .register_component_object(RegisteredComponentObject {
                        object: CertifiedComponentObject {
                            object_type: stable_id(LANE_OUTCOME_CONTRACT_OBJECT_TYPE)?,
                            content_ref: lane_content_ref.clone(),
                            value: CanonicalJsonValue::from_canonical_json(
                                lane_outcome_contract_canonical_json(&success_ref, lane_failure)?
                                    .as_bytes(),
                            )?,
                        },
                        outbound_references: lane_references,
                    })?;
                let join_content_ref = fan_out_join_contract_ref(&success_ref, lane_failure)?;
                self.registry
                    .register_component_object(RegisteredComponentObject {
                        object: CertifiedComponentObject {
                            object_type: stable_id(FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE)?,
                            content_ref: join_content_ref.clone(),
                            value: CanonicalJsonValue::from_canonical_json(
                                fan_out_join_contract_canonical_json(&success_ref, lane_failure)?
                                    .as_bytes(),
                            )?,
                        },
                        outbound_references: vec![ComponentObjectReference {
                            object_type: stable_id(LANE_OUTCOME_CONTRACT_OBJECT_TYPE)?,
                            content_ref: lane_content_ref,
                        }],
                    })?;
                Ok(join_content_ref)
            }
        }
    }

    /// Registers one exact domain fact descriptor and its typed contract edges.
    pub fn register_fact_descriptor(
        &mut self,
        descriptor: StructuredFactDescriptor,
    ) -> Result<ContentRef> {
        self.registry.register_fact_descriptor(descriptor)
    }

    /// Registers one type-derived retained value together with its exact
    /// process-qualified closed tag and payload table.
    pub fn register_closed_sum<T: ClosedSum>(&mut self) -> Result<ContentRef> {
        let selector_contract_ref = self.register_value::<T>()?;
        let contract = closed_sum_contract::<T>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if contract.selector_contract_ref != selector_contract_ref {
            return Err(CertifyError::Certification(
                "closed-sum selector differs from its registered value contract".to_owned(),
            ));
        }
        let contract_ref = contract.closed_sum_contract_ref.clone();
        self.registry.register_closed_sum(contract)?;
        Ok(contract_ref)
    }

    /// Registers one exact authored child program for pure call-site
    /// substitution.
    pub fn register_child(&mut self, child: AuthoredStructuredProgram) -> Result<()> {
        self.registry.register_child(child)
    }

    /// Registers the exact secret-free executable identity referenced by
    /// implementation descriptors.
    pub fn register_executable_identity(
        &mut self,
        identity: SecretFreeExecutableIdentity,
    ) -> Result<ContentRef> {
        let content_ref = identity
            .content_ref()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        self.registry.register_component_object(component_object(
            EXECUTABLE_IDENTITY_OBJECT_TYPE,
            content_ref.clone(),
            &identity,
            Vec::new(),
        )?)?;
        Ok(content_ref)
    }

    /// Registers the exact secret-free qualification artifact referenced by
    /// implementation descriptors.
    pub fn register_qualification_artifact(
        &mut self,
        artifact: SecretFreeQualificationArtifact,
    ) -> Result<ContentRef> {
        let content_ref = artifact
            .content_ref()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        self.registry.register_component_object(component_object(
            QUALIFICATION_ARTIFACT_OBJECT_TYPE,
            content_ref.clone(),
            &artifact,
            Vec::new(),
        )?)?;
        Ok(content_ref)
    }

    /// Registers a semantic state contract together with its process-qualified,
    /// secret-free implementation descriptor and real typed callbacks.
    pub fn register_state<S: State>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        callbacks: StructuredStateCallbacks<S>,
    ) -> Result<ContentRef> {
        let contract = state_contract::<S>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if contract.capability_requirement_ref.is_some() {
            return Err(CertifyError::Certification(
                "a capability-lowered semantic state has no direct implementation binding"
                    .to_owned(),
            ));
        }
        if callbacks.kind() != contract.execution.kind() {
            return Err(CertifyError::Certification(
                "state callback variant differs from its semantic execution kind".to_owned(),
            ));
        }
        let callback_access_types = (
            TypeId::of::<S::Request>(),
            TypeId::of::<S::Returned>(),
            TypeId::of::<S::SafeFailure>(),
        );
        match <S::Execution as mfm_program::structured::Execution>::access_type_ids() {
            Some(execution_access_types) if execution_access_types != callback_access_types => {
                return Err(CertifyError::Certification(
                    "state callback access ABI differs from its capability contract".to_owned(),
                ));
            }
            Some(_) => {}
            None if callbacks.kind() != mfm_spec::structured::StructuredExecutionKind::Pure => {
                return Err(CertifyError::Certification(
                    "a Pure state cannot register access callbacks".to_owned(),
                ));
            }
            None => {}
        }

        // Safe-failure totality is type-enforced by the disposition's proposal type on
        // settle_safe_failure; qualification does not accept or require a sample corpus.
        match (
            &contract.execution,
            contract.safe_failure_disposition,
            callbacks.kind(),
        ) {
            (
                StructuredStateExecutionContract::Pure,
                StructuredSafeFailureDispositionContract::NotApplicable {},
                mfm_spec::structured::StructuredExecutionKind::Pure,
            ) => {}
            (
                StructuredStateExecutionContract::Read { .. }
                | StructuredStateExecutionContract::Effect { .. },
                StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
                | StructuredSafeFailureDispositionContract::MaySettleTypedFailure {},
                mfm_spec::structured::StructuredExecutionKind::Read
                | mfm_spec::structured::StructuredExecutionKind::Effect,
            ) if callbacks.kind() == contract.execution.kind() => {}
            _ => {
                return Err(CertifyError::Certification(
                    "returned and safe-failure callback contracts differ from the state disposition"
                        .to_owned(),
                ));
            }
        }

        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_state(contract.clone())?;
        let implementation_contract_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            descriptor,
            StructuredComponentKind::State,
            &contract.state_contract_ref,
            ProcessHandle::State(Arc::new(TypedStateCallbacks { callbacks })),
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok(implementation_contract_ref)
    }

    /// Registers one type-derived abstract semantic state whose exact
    /// capability recipe must replace it before certification.
    pub fn register_capability_state<S: State>(&mut self) -> Result<ContentRef> {
        let contract = state_contract::<S>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if contract.capability_requirement_ref.is_none() {
            return Err(CertifyError::Certification(
                "an abstract semantic state must declare an exact capability requirement"
                    .to_owned(),
            ));
        }
        let contract_ref = contract.state_contract_ref.clone();
        self.registry.register_state(contract)?;
        Ok(contract_ref)
    }

    /// Registers one exact typed Read capability implementation.
    pub fn register_read_capability<C, I>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        implementation: Arc<I>,
    ) -> Result<ContentRef>
    where
        C: RuntimeReadCapability,
        I: ReadCapabilityImplementation<C>,
    {
        let contract = runtime_read_capability_contract::<C>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let contract_ref = contract.content_ref()?;
        let reserved_capability_ref =
            prior_run_fact_selection_capability_contract()?.content_ref()?;
        let reserved_adapter_ref = prior_run_fact_scanner_adapter_contract()?.content_ref()?;
        if TypeId::of::<C>() == TypeId::of::<PriorRunFactSelectionCapability>()
            || contract_ref == reserved_capability_ref
            || contract
                .dependencies
                .iter()
                .any(|dependency| dependency.contract_ref == reserved_adapter_ref)
        {
            return Err(CertifyError::Certification(
                "the reserved prior-run fact capability must use its sealed registration"
                    .to_owned(),
            ));
        }
        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_live_component(contract)?;
        let implementation: Arc<dyn ReadCapabilityImplementation<C>> = implementation;
        let implementation_contract_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            descriptor,
            StructuredComponentKind::Capability,
            &contract_ref,
            ProcessHandle::ReadCapability(Arc::new(TypedReadCapabilityImplementation {
                implementation,
            })),
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok(implementation_contract_ref)
    }

    fn install_prior_run_fact_selection(
        &mut self,
        capability_descriptor: SecretFreeImplementationDescriptor,
        adapter_descriptor: SecretFreeImplementationDescriptor,
    ) -> Result<(ContentRef, ContentRef)> {
        let capability_contract =
            runtime_read_capability_contract::<PriorRunFactSelectionCapability>()
                .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let capability_contract_ref = capability_contract.content_ref()?;
        let adapter_contract = prior_run_fact_scanner_adapter_contract()?;
        let adapter_contract_ref = adapter_contract.content_ref()?;
        if capability_contract
            .dependencies
            .first()
            .map(|entry| &entry.contract_ref)
            != Some(&adapter_contract_ref)
        {
            return Err(CertifyError::Certification(
                "prior-run fact capability does not bind its sealed scanner adapter".to_owned(),
            ));
        }
        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_live_component(capability_contract)?;
        registry.register_live_component(adapter_contract)?;
        let implementation: Arc<dyn ReadCapabilityImplementation<PriorRunFactSelectionCapability>> =
            Arc::new(KernelPriorRunFactSelectionImplementation {
                fault_code: stable_id("prior-run-fact-selection-contract-fault")?,
            });
        let capability_implementation_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            capability_descriptor,
            StructuredComponentKind::Capability,
            &capability_contract_ref,
            ProcessHandle::ReadCapability(Arc::new(TypedReadCapabilityImplementation {
                implementation,
            })),
        )?;
        let adapter_implementation_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            adapter_descriptor,
            StructuredComponentKind::Adapter,
            &adapter_contract_ref,
            ProcessHandle::PriorRunFactScannerBindingSource,
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok((capability_implementation_ref, adapter_implementation_ref))
    }

    /// Registers one exact typed Effect capability implementation.
    pub fn register_effect_capability<C, I>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        implementation: Arc<I>,
    ) -> Result<ContentRef>
    where
        C: RuntimeEffectCapability,
        I: EffectCapabilityImplementation<C>,
    {
        let contract = runtime_effect_capability_contract::<C>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let contract_ref = contract.content_ref()?;
        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_live_component(contract)?;
        let implementation: Arc<dyn EffectCapabilityImplementation<C>> = implementation;
        let implementation_contract_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            descriptor,
            StructuredComponentKind::Capability,
            &contract_ref,
            ProcessHandle::EffectCapability(Arc::new(TypedEffectCapabilityImplementation {
                implementation,
            })),
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok(implementation_contract_ref)
    }

    /// Registers one exact typed source of target-bound Read adapters.
    pub fn register_read_adapter<C, S>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        source: Arc<S>,
    ) -> Result<ContentRef>
    where
        C: RuntimeReadCapability,
        S: RuntimeReadPhysicalBindingSource<C>,
    {
        let contract = S::Binding::contract()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if contract.component_kind != StructuredComponentKind::Adapter {
            return Err(CertifyError::Certification(
                "runtime adapter returned a non-adapter semantic contract".to_owned(),
            ));
        }
        let contract_ref = contract.content_ref()?;
        let capability_contract = runtime_read_capability_contract::<C>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let capability_contract_ref = capability_contract.content_ref()?;
        let reserved_capability_ref =
            prior_run_fact_selection_capability_contract()?.content_ref()?;
        let reserved_adapter_ref = prior_run_fact_scanner_adapter_contract()?.content_ref()?;
        if TypeId::of::<C>() == TypeId::of::<PriorRunFactSelectionCapability>()
            || capability_contract_ref == reserved_capability_ref
            || contract_ref == reserved_adapter_ref
        {
            return Err(CertifyError::Certification(
                "the reserved prior-run fact adapter must use its sealed registration".to_owned(),
            ));
        }
        if capability_contract
            .dependencies
            .first()
            .map(|entry| &entry.contract_ref)
            != Some(&contract_ref)
        {
            return Err(CertifyError::Certification(
                "runtime capability does not bind the registered adapter contract".to_owned(),
            ));
        }
        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_live_component(contract)?;
        let implementation_contract_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            descriptor,
            StructuredComponentKind::Adapter,
            &contract_ref,
            ProcessHandle::ReadPhysicalBindingSource(Arc::new(TypedReadPhysicalBindingSource::<
                C,
                S,
            > {
                source,
                _contract: PhantomData,
            })),
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok(implementation_contract_ref)
    }

    /// Registers one exact typed source of target-bound Effect adapters.
    pub fn register_effect_adapter<C, S>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        source: Arc<S>,
    ) -> Result<ContentRef>
    where
        C: RuntimeEffectCapability,
        S: RuntimeEffectPhysicalBindingSource<C>,
    {
        let contract = S::Binding::contract()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if contract.component_kind != StructuredComponentKind::Adapter {
            return Err(CertifyError::Certification(
                "runtime Effect adapter returned a non-adapter semantic contract".to_owned(),
            ));
        }
        let contract_ref = contract.content_ref()?;
        let capability_contract = runtime_effect_capability_contract::<C>()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if capability_contract
            .dependencies
            .first()
            .map(|entry| &entry.contract_ref)
            != Some(&contract_ref)
        {
            return Err(CertifyError::Certification(
                "runtime Effect capability does not bind the registered adapter contract"
                    .to_owned(),
            ));
        }
        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_live_component(contract)?;
        let implementation_contract_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            descriptor,
            StructuredComponentKind::Adapter,
            &contract_ref,
            ProcessHandle::EffectPhysicalBindingSource(Arc::new(
                TypedEffectPhysicalBindingSource::<C, S> {
                    source,
                    _contract: PhantomData,
                },
            )),
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok(implementation_contract_ref)
    }

    /// Registers one exact typed bounded signer invoker.
    pub fn register_signer<S, I>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        invoker: Arc<I>,
    ) -> Result<ContentRef>
    where
        S: RuntimeSigner,
        I: BoundedComponentInvoker<S>,
    {
        self.register_bounded_component::<S, I>(
            descriptor,
            invoker,
            StructuredComponentKind::Signer,
            S::contract().map_err(|error| CertifyError::Certification(error.to_string()))?,
        )
    }

    /// Registers one exact typed bounded resource-authority invoker.
    pub fn register_resource_authority<R, I>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        invoker: Arc<I>,
    ) -> Result<ContentRef>
    where
        R: RuntimeResourceAuthority,
        I: BoundedComponentInvoker<R>,
    {
        self.register_bounded_component::<R, I>(
            descriptor,
            invoker,
            StructuredComponentKind::Resource,
            R::contract().map_err(|error| CertifyError::Certification(error.to_string()))?,
        )
    }

    fn register_bounded_component<C, I>(
        &mut self,
        descriptor: SecretFreeImplementationDescriptor,
        invoker: Arc<I>,
        expected_kind: StructuredComponentKind,
        contract: StructuredLiveComponentContract,
    ) -> Result<ContentRef>
    where
        C: mfm_capabilities::BoundedComponentContract,
        I: BoundedComponentInvoker<C>,
    {
        if contract.component_kind != expected_kind {
            return Err(CertifyError::Certification(
                "bounded component returned the wrong semantic contract kind".to_owned(),
            ));
        }
        let contract_ref = contract.content_ref()?;
        let mut registry = self.registry.clone();
        let mut process_components = self.process_components.clone();
        registry.register_live_component(contract)?;
        let handle = Arc::new(TypedBoundedInvoker::<C, I> {
            invoker,
            _contract: PhantomData,
        });
        let process_handle = match expected_kind {
            StructuredComponentKind::Signer => ProcessHandle::Signer(handle),
            StructuredComponentKind::Resource => ProcessHandle::Resource(handle),
            _ => {
                return Err(CertifyError::Certification(
                    "bounded process component kind is not signer or resource".to_owned(),
                ));
            }
        };
        let implementation_contract_ref = register_implementation_binding(
            &mut registry,
            &mut process_components,
            descriptor,
            expected_kind,
            &contract_ref,
            process_handle,
        )?;
        self.registry = registry;
        self.process_components = process_components;
        Ok(implementation_contract_ref)
    }

    /// Registers one type-selected callback-free capability expansion recipe.
    pub fn register_capability_expansion<Expansion>(&mut self) -> Result<ContentRef>
    where
        Expansion: CapabilityExpansion,
    {
        let recipe =
            Expansion::recipe().map_err(|error| CertifyError::Certification(error.to_string()))?;
        let recipe_ref = recipe
            .content_ref()
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        let requirement_ref = capability_expansion_requirement_ref(&recipe_ref)?;
        self.registry
            .register_capability_recipe(requirement_ref.clone(), recipe)?;
        Ok(requirement_ref)
    }

    /// Registers one exact callback-free policy expansion recipe.
    pub fn register_policy_recipe(&mut self, recipe: PolicyExpansionRecipe) -> Result<()> {
        self.registry.register_policy_recipe(recipe)
    }

    /// Registers one immutable entry point. The authored program, required
    /// profile, public contract, and operation identity are selected here and
    /// cannot be replaced by a certification caller.
    pub fn register_entry_point(
        &mut self,
        entry_point_id: StableId,
        authored: AuthoredStructuredProgram,
        profile: StructuredExpansionProfile,
    ) -> Result<()> {
        if entry_point_id != authored.operation_id {
            return Err(CertifyError::Certification(
                "entry-point identity differs from the authored operation".to_owned(),
            ));
        }
        #[derive(Serialize)]
        struct EntryPointContract<'a> {
            entry_point_id: &'a StableId,
            input_contract_refs: Vec<&'a ContentRef>,
            output_contract_ref: &'a ContentRef,
            failure_contract_ref: ContentRef,
        }
        let entry_contract = EntryPointContract {
            entry_point_id: &entry_point_id,
            input_contract_refs: authored
                .input_roots
                .iter()
                .map(|root| &root.contract_ref)
                .collect(),
            output_contract_ref: &authored.output_contract_ref,
            failure_contract_ref: authored.failure_contract.contract_ref()?,
        };
        let entry_point_contract_ref =
            typed_content_ref("mfm.structured-entry-point-contract", &entry_contract)?;
        let mut outbound_references = entry_contract
            .input_contract_refs
            .iter()
            .map(|content_ref| contract_reference(content_ref))
            .collect::<Result<Vec<_>>>()?;
        outbound_references.extend([
            contract_reference(entry_contract.output_contract_ref)?,
            contract_reference(&entry_contract.failure_contract_ref)?,
        ]);
        self.registry.register_component_object(component_object(
            ENTRY_POINT_CONTRACT_OBJECT_TYPE,
            entry_point_contract_ref.clone(),
            &entry_contract,
            outbound_references,
        )?)?;
        let definition = EntryPointDefinition {
            entry_point_id: entry_point_id.clone(),
            entry_point_contract_ref,
            coverage_template: authored,
            profile,
            support_envelope: None,
        };
        insert_exact(
            &mut self.entry_points,
            entry_point_id,
            definition,
            "qualified entry point",
        )
    }

    /// Consumes all mutable assembly authority and validates the exact caller-declared
    /// entry-point set before returning the immutable lookup registry.
    pub fn build(
        mut self,
        expected_entry_point_ids: &[StableId],
    ) -> Result<QualifiedProgramRegistry> {
        if let Some(error) = self.initialization_error.take() {
            return Err(error);
        }
        let expected_entry_points = expected_entry_point_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if expected_entry_points.len() != expected_entry_point_ids.len() {
            return Err(CertifyError::Certification(
                "expected qualified entry-point identities contain duplicates".to_owned(),
            ));
        }
        if expected_entry_points != self.entry_points.keys().cloned().collect::<BTreeSet<_>>() {
            return Err(CertifyError::Certification(
                "qualified entry-point set differs from production expectation".to_owned(),
            ));
        }
        if self.entry_points.is_empty() {
            return Err(CertifyError::Certification(
                "qualified registry has no entry points".to_owned(),
            ));
        }
        let mut required_process_components = BTreeSet::new();
        for entry in self.entry_points.values_mut() {
            let certified = prepare_entry(entry, entry.coverage_template.clone(), &self.registry)?;
            let (components, implementations) =
                component_manifests(certified.expanded(), &self.registry)?;
            let semantic_components = components
                .entries
                .into_iter()
                .map(|component| (component.component_kind, component.semantic_contract_ref))
                .collect();
            let implementation_envelope = implementations
                .entries
                .iter()
                .map(|implementation| {
                    (
                        implementation.component_kind,
                        implementation.semantic_contract_ref.clone(),
                        implementation.implementation_contract_ref.clone(),
                    )
                })
                .collect();
            entry.support_envelope = Some(EntryPointSupportEnvelope {
                semantic_components,
                implementations: implementation_envelope,
            });
            for implementation in implementations.entries {
                required_process_components.insert((
                    implementation.component_kind,
                    implementation.semantic_contract_ref,
                    implementation.implementation_contract_ref,
                ));
            }
        }
        required_process_components.extend(self.kernel_baseline_process_components);
        let registered_process_components = self
            .process_components
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if required_process_components != registered_process_components {
            return Err(CertifyError::Certification(
                "qualified process bindings are missing, unused, or implementation-substituted"
                    .to_owned(),
            ));
        }
        validate_process_component_graph(
            &self.registry,
            &self.process_components,
            &required_process_components,
        )?;
        let qualified = QualifiedProgramRegistry {
            registry: Arc::new(self.registry),
            entry_points: self.entry_points,
            process_components: self.process_components,
        };
        Ok(qualified)
    }
}

fn validate_process_component_graph(
    registry: &StructuredCertificationRegistry,
    process_components: &BTreeMap<ProcessComponentKey, RegisteredProcessComponent>,
    required: &BTreeSet<ProcessComponentKey>,
) -> Result<()> {
    let reserved_fact_capability_ref =
        prior_run_fact_selection_capability_contract()?.content_ref()?;
    let reserved_fact_adapter_ref = prior_run_fact_scanner_adapter_contract()?.content_ref()?;
    for key @ (component_kind, semantic_contract_ref, implementation_contract_ref) in required {
        let component = process_components.get(key).ok_or_else(|| {
            CertifyError::Certification("required process component is missing".to_owned())
        })?;
        if component.component_kind != *component_kind
            || component.semantic_contract_ref != *semantic_contract_ref
            || component.implementation_contract_ref != *implementation_contract_ref
        {
            return Err(CertifyError::Certification(
                "process component key differs from its retained binding".to_owned(),
            ));
        }
        let expected_implementation = registry
            .component_implementations
            .get(&(*component_kind, semantic_contract_ref.clone()));
        if expected_implementation != Some(implementation_contract_ref) {
            return Err(CertifyError::Certification(
                "process component differs from the secret-free implementation manifest".to_owned(),
            ));
        }
        // Live semantic contracts must re-hash to the exact map key so hostile
        // in-place protocol/evidence field substitution cannot remain qualified.
        if matches!(
            component_kind,
            StructuredComponentKind::Capability
                | StructuredComponentKind::Adapter
                | StructuredComponentKind::Signer
                | StructuredComponentKind::Resource
        ) {
            let live = registry
                .live_components
                .get(&(*component_kind, semantic_contract_ref.clone()))
                .ok_or_else(|| {
                    CertifyError::Certification(
                        "process live component has no exact semantic contract".to_owned(),
                    )
                })?;
            if live.component_kind != *component_kind
                || live.content_ref()? != *semantic_contract_ref
            {
                return Err(CertifyError::Certification(
                    "live semantic component kind or contract identity mismatch".to_owned(),
                ));
            }
        }
        let handle_kind_matches = matches!(
            (component_kind, &component.handle),
            (StructuredComponentKind::State, ProcessHandle::State(_))
                | (
                    StructuredComponentKind::Capability,
                    ProcessHandle::ReadCapability(_) | ProcessHandle::EffectCapability(_)
                )
                | (
                    StructuredComponentKind::Adapter,
                    ProcessHandle::ReadPhysicalBindingSource(_)
                        | ProcessHandle::EffectPhysicalBindingSource(_)
                        | ProcessHandle::PriorRunFactScannerBindingSource
                )
                | (StructuredComponentKind::Signer, ProcessHandle::Signer(_))
                | (
                    StructuredComponentKind::Resource,
                    ProcessHandle::Resource(_)
                )
        );
        if !handle_kind_matches {
            return Err(CertifyError::Certification(
                "process handle variant differs from its component kind".to_owned(),
            ));
        }
        if *component_kind == StructuredComponentKind::State {
            let state = registry.states.get(semantic_contract_ref).ok_or_else(|| {
                CertifyError::Certification(
                    "process state has no exact semantic state contract".to_owned(),
                )
            })?;
            if state.contract.capability_requirement_ref.is_some() {
                return Err(CertifyError::Certification(
                    "an abstract capability state retained a process implementation".to_owned(),
                ));
            }
            let ProcessHandle::State(callbacks) = &component.handle else {
                return Err(CertifyError::Certification(
                    "process state handle differs from its component kind".to_owned(),
                ));
            };
            if callbacks.kind() != state.contract.execution.kind()
                || callbacks.semantic_contract_ref()? != *semantic_contract_ref
            {
                return Err(CertifyError::Certification(
                    "state callback kind or typed contract differs from the qualified semantic contract"
                        .to_owned(),
                ));
            }
            if let Some(capability_contract_ref) =
                state.contract.execution.capability_contract_ref()
            {
                let capability = registry
                    .live_components
                    .get(&(
                        StructuredComponentKind::Capability,
                        capability_contract_ref.clone(),
                    ))
                    .ok_or_else(|| {
                        CertifyError::Certification(
                            "state access capability contract is missing".to_owned(),
                        )
                    })?;
                let protocol_kind_matches = matches!(
                    (&state.contract.execution, &capability.capability_protocol),
                    (
                        StructuredStateExecutionContract::Read { .. },
                        Some(StructuredCapabilityProtocolContract::Read { .. })
                    ) | (
                        StructuredStateExecutionContract::Effect { .. },
                        Some(StructuredCapabilityProtocolContract::Effect { .. })
                    )
                );
                if !protocol_kind_matches {
                    return Err(CertifyError::Certification(
                        "state execution kind differs from its capability protocol".to_owned(),
                    ));
                }
                qualify_live_state_settlement_contracts(state, callbacks.as_ref())?;
            } else if state.contract.safe_failure_disposition
                != (StructuredSafeFailureDispositionContract::NotApplicable {})
                || callbacks.kind() != StructuredExecutionKind::Pure
            {
                return Err(CertifyError::Certification(
                    "Pure state retained a live settlement qualification".to_owned(),
                ));
            }
        }
    }

    for key @ (component_kind, semantic_contract_ref, _) in required {
        if *component_kind != StructuredComponentKind::Capability {
            continue;
        }
        let capability = registry
            .live_components
            .get(&(*component_kind, semantic_contract_ref.clone()))
            .ok_or_else(|| {
                CertifyError::Certification(
                    "qualified capability semantic contract is missing".to_owned(),
                )
            })?;
        let adapter_ref = &capability
            .dependencies
            .first()
            .filter(|dependency| dependency.component_kind == StructuredComponentKind::Adapter)
            .ok_or_else(|| {
                CertifyError::Certification(
                    "qualified capability has no exact adapter dependency".to_owned(),
                )
            })?
            .contract_ref;
        let adapter_implementation_ref = registry
            .component_implementations
            .get(&(StructuredComponentKind::Adapter, adapter_ref.clone()))
            .ok_or_else(|| {
                CertifyError::Certification(
                    "qualified capability adapter implementation is missing".to_owned(),
                )
            })?;
        let adapter_key = (
            StructuredComponentKind::Adapter,
            adapter_ref.clone(),
            adapter_implementation_ref.clone(),
        );
        let capability_process = process_components.get(key).ok_or_else(|| {
            CertifyError::Certification("qualified capability process is missing".to_owned())
        })?;
        let adapter_process = process_components.get(&adapter_key).ok_or_else(|| {
            CertifyError::Certification("qualified adapter process is missing".to_owned())
        })?;
        match (
            capability.capability_protocol.as_ref(),
            &capability_process.handle,
            &adapter_process.handle,
        ) {
            (
                Some(StructuredCapabilityProtocolContract::Read { .. }),
                ProcessHandle::ReadCapability(capability_implementation),
                ProcessHandle::ReadPhysicalBindingSource(binding_source),
            ) if capability_implementation.capability_type_id()
                == binding_source.capability_type_id()
                && capability_implementation.semantic_contract_ref()? == *semantic_contract_ref
                && binding_source.semantic_contract_ref()? == *adapter_ref
                && *semantic_contract_ref != reserved_fact_capability_ref
                && *adapter_ref != reserved_fact_adapter_ref => {}
            (
                Some(StructuredCapabilityProtocolContract::Read { .. }),
                ProcessHandle::ReadCapability(capability_implementation),
                ProcessHandle::PriorRunFactScannerBindingSource,
            ) if capability_implementation.capability_type_id()
                == TypeId::of::<PriorRunFactSelectionCapability>()
                && capability_implementation.semantic_contract_ref()? == *semantic_contract_ref
                && *semantic_contract_ref == reserved_fact_capability_ref
                && *adapter_ref == reserved_fact_adapter_ref => {}
            (
                Some(StructuredCapabilityProtocolContract::Effect {
                    refresh_contract, ..
                }),
                ProcessHandle::EffectCapability(capability_implementation),
                ProcessHandle::EffectPhysicalBindingSource(binding_source),
            ) if capability_implementation.capability_type_id()
                == binding_source.capability_type_id()
                && capability_implementation.refresh_mode_type_id()
                    == binding_source.refresh_mode_type_id()
                && capability_implementation.semantic_contract_ref()? == *semantic_contract_ref
                && binding_source.semantic_contract_ref()? == *adapter_ref =>
            {
                if let StructuredEffectRefreshContract::Refreshable {
                    resource_lineage_contract_ref,
                    ..
                } = refresh_contract
                {
                    let adapter = registry
                        .live_components
                        .get(&(StructuredComponentKind::Adapter, adapter_ref.clone()))
                        .ok_or_else(|| {
                            CertifyError::Certification(
                                "refreshable Effect adapter contract is missing".to_owned(),
                            )
                        })?;
                    let matching_resources = adapter
                        .dependencies
                        .iter()
                        .filter(|dependency| {
                            dependency.component_kind == StructuredComponentKind::Resource
                                && &dependency.contract_ref
                                    == resource_lineage_contract_ref.as_ref()
                        })
                        .count();
                    let resource_dependencies = adapter
                        .dependencies
                        .iter()
                        .filter(|dependency| {
                            dependency.component_kind == StructuredComponentKind::Resource
                        })
                        .count();
                    if matching_resources != 1 || resource_dependencies != 1 {
                        return Err(CertifyError::Certification(
                            "refreshable Effect lineage differs from its adapter Resource"
                                .to_owned(),
                        ));
                    }
                }
            }
            _ => {
                return Err(CertifyError::Certification(
                    "capability and adapter access kinds or typed ABIs differ".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// Verifies live returned/safe-failure settlement contracts against the
/// disposition declared on the semantic state. Totality of safe-failure
/// settlement is owned by the disposition's proposal type, not a sample corpus.
fn qualify_live_state_settlement_contracts(
    state: &RegisteredState,
    callbacks: &dyn ErasedStateCallbacks,
) -> Result<()> {
    match (
        &state.contract.execution,
        state.contract.safe_failure_disposition,
        callbacks.kind(),
    ) {
        (
            StructuredStateExecutionContract::Read { .. },
            StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
            | StructuredSafeFailureDispositionContract::MaySettleTypedFailure {},
            StructuredExecutionKind::Read,
        )
        | (
            StructuredStateExecutionContract::Effect { .. },
            StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
            | StructuredSafeFailureDispositionContract::MaySettleTypedFailure {},
            StructuredExecutionKind::Effect,
        ) => Ok(()),
        _ => Err(CertifyError::Certification(
            "returned and safe-failure callback contracts differ from the state disposition"
                .to_owned(),
        )),
    }
}

/// Immutable process-qualified structured-program registry.
#[derive(Debug)]
pub struct QualifiedProgramRegistry {
    registry: Arc<StructuredCertificationRegistry>,
    entry_points: BTreeMap<StableId, EntryPointDefinition>,
    #[allow(dead_code)]
    process_components: BTreeMap<ProcessComponentKey, RegisteredProcessComponent>,
}

/// Cloneable pure admission-time certifier over fixed qualified entry definitions.
///
/// This snapshot owns no callback, adapter, signer, resource, store, or writer
/// authority. It may only deterministically certify an authored candidate
/// within an entry's frozen signature and support envelope.
#[derive(Debug, Clone)]
pub struct AdmissionCertificationRegistry {
    registry: Arc<StructuredCertificationRegistry>,
    entry_points: BTreeMap<StableId, EntryPointDefinition>,
}

impl AdmissionCertificationRegistry {
    /// Certifies one exact candidate under the fixed qualified entry definition.
    pub fn certify(
        &self,
        entry_point_id: &StableId,
        authored: AuthoredStructuredProgram,
    ) -> Result<CertifiedProgram> {
        let entry = self.entry_points.get(entry_point_id).ok_or_else(|| {
            CertifyError::Certification("entry point is not process-qualified".to_owned())
        })?;
        prepare_entry(entry, authored, &self.registry)
    }
}

impl QualifiedProgramRegistry {
    /// Resolves a private certification handle by exact qualified entry-point
    /// identity.
    pub fn certifier(&self, entry_point_id: &StableId) -> Result<EntryPointCertifier<'_>> {
        let entry = self.entry_points.get(entry_point_id).ok_or_else(|| {
            CertifyError::Certification("entry point is not process-qualified".to_owned())
        })?;
        Ok(EntryPointCertifier {
            entry,
            registry: &self.registry,
        })
    }

    /// Resolves a purpose-limited persisted-document verifier by exact
    /// qualified entry-point identity.
    pub fn admission_verifier(&self, entry_point_id: &StableId) -> Result<AdmissionVerifier<'_>> {
        let entry = self.entry_points.get(entry_point_id).ok_or_else(|| {
            CertifyError::Verification("entry point is not process-qualified".to_owned())
        })?;
        Ok(AdmissionVerifier {
            entry,
            registry: &self.registry,
        })
    }

    /// Returns a cloneable callback-free snapshot for persisted admission
    /// verification by the RunHistory store and replay.
    pub fn admission_verification_registry(&self) -> AdmissionVerificationRegistry {
        AdmissionVerificationRegistry {
            registry: Arc::clone(&self.registry),
            entry_points: self.entry_points.clone(),
        }
    }

    /// Returns a cloneable pure snapshot for dynamic admission certification.
    pub fn admission_certification_registry(&self) -> AdmissionCertificationRegistry {
        AdmissionCertificationRegistry {
            registry: Arc::clone(&self.registry),
            entry_points: self.entry_points.clone(),
        }
    }

    /// Consumes qualified assembly into its callback-free admission snapshot
    /// and the sole non-cloneable live process authority for Runtime.
    ///
    /// Production consumers must pass the complete registry into store assembly
    /// rather than splitting and reassembling halves independently.
    #[doc(hidden)]
    pub fn into_runtime_parts(self) -> (AdmissionVerificationRegistry, RuntimeProcessRegistry) {
        let registry = self.registry;
        let admission = AdmissionVerificationRegistry {
            registry: Arc::clone(&registry),
            entry_points: self.entry_points,
        };
        let runtime = RuntimeProcessRegistry {
            registry,
            process_components: self.process_components,
            process_identity: Arc::new(()),
        };
        (admission, runtime)
    }
}

/// Sole non-cloneable process authority over qualified state callbacks and
/// target-bound live adapter sources.
///
/// Production assembly moves this value into Runtime. It contains no store,
/// writer, or admission mutation authority. Its private target handles can be
/// entered only by consuming an opaque binding after Runtime has obtained one
/// newly committed affine authorization.
pub struct RuntimeProcessRegistry {
    registry: Arc<StructuredCertificationRegistry>,
    process_components: BTreeMap<ProcessComponentKey, RegisteredProcessComponent>,
    process_identity: Arc<()>,
}

impl std::fmt::Debug for RuntimeProcessRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeProcessRegistry")
            .field("process_components", &self.process_components.len())
            .finish_non_exhaustive()
    }
}

impl RuntimeProcessRegistry {
    /// Resolves one exact registry-issued component identity.
    ///
    /// A missing result means the verified candidate selected a component that
    /// is absent from the qualified process closure.
    pub fn component_identity(
        &self,
        kind: StructuredComponentKind,
        semantic_contract_ref: &ContentRef,
    ) -> Option<QualifiedComponentIdentity> {
        self.process_component(kind, semantic_contract_ref)
            .ok()
            .map(RegisteredProcessComponent::identity)
    }

    /// Invokes one exact qualified Pure state callback.
    pub fn invoke_pure(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
    ) -> std::result::Result<QualifiedStateProposal, QualifiedProcessFault> {
        let callbacks = self.state_callbacks(state)?;
        if callbacks.kind() != StructuredExecutionKind::Pure {
            return Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Contract,
                state.clone(),
            ));
        }
        callbacks
            .invoke_pure_runtime(input)
            .map(|value| QualifiedStateProposal {
                origin: state.clone(),
                value,
            })
            .map_err(|code| QualifiedProcessFault::new(code.into(), state.clone()))
    }

    /// Authors one exact typed request for a qualified Read or Effect state.
    pub fn author_request(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
    ) -> std::result::Result<CanonicalJsonValue, QualifiedProcessFault> {
        let callbacks = self.state_callbacks(state)?;
        if callbacks.kind() == StructuredExecutionKind::Pure {
            return Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Contract,
                state.clone(),
            ));
        }
        callbacks
            .author_request_runtime(input)
            .map_err(|code| QualifiedProcessFault::new(code.into(), state.clone()))
    }

    /// Settles one exact already-committed normal observation.
    pub fn settle_observation(
        &self,
        state: &QualifiedComponentIdentity,
        input: &CanonicalJsonValue,
        observation: &CanonicalJsonValue,
    ) -> std::result::Result<QualifiedStateSettlement, QualifiedProcessFault> {
        let settlement = self
            .state_callbacks(state)?
            .settle_observation_runtime(input, observation)
            .map_err(|code| QualifiedProcessFault::new(code.into(), state.clone()))?;
        Ok(match settlement {
            RuntimeStateSettlement::Proposed(value) => {
                QualifiedStateSettlement::Proposed(QualifiedStateProposal {
                    origin: state.clone(),
                    value,
                })
            }
            RuntimeStateSettlement::InvalidEvidence => {
                QualifiedStateSettlement::InvalidEvidence(state.clone())
            }
        })
    }

    /// Returns the exact qualified adapter selected by a capability dependency.
    pub fn access_adapter_identity(
        &self,
        capability: &QualifiedComponentIdentity,
    ) -> std::result::Result<QualifiedComponentIdentity, QualifiedProcessFault> {
        if capability.component_kind != StructuredComponentKind::Capability
            || self.process_component_by_identity(capability).is_err()
        {
            return Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Contract,
                capability.clone(),
            ));
        }
        let adapter_ref = self
            .registry
            .live_components
            .get(&(
                StructuredComponentKind::Capability,
                capability.semantic_contract_ref.clone(),
            ))
            .and_then(|component| component.dependencies.first())
            .filter(|dependency| dependency.component_kind == StructuredComponentKind::Adapter)
            .map(|dependency| &dependency.contract_ref)
            .ok_or_else(|| {
                QualifiedProcessFault::new(QualifiedProcessFaultCode::Contract, capability.clone())
            })?;
        self.component_identity(StructuredComponentKind::Adapter, adapter_ref)
            .ok_or_else(|| {
                QualifiedProcessFault::new(QualifiedProcessFaultCode::Contract, capability.clone())
            })
    }

    /// Selects and preflights one exact target-bound physical binding.
    ///
    /// The result owns the frozen request, public certificate, and private
    /// target handle. Returning `None` means no current qualified target was
    /// available before authorization.
    pub async fn prepare_access<K: PhysicalBindingKind>(
        &self,
        capability_identity: &QualifiedComponentIdentity,
        target: AccessTargetSelection<'_>,
        request: CanonicalJsonValue,
    ) -> std::result::Result<Option<QualifiedPhysicalBinding<K>>, QualifiedProcessFault> {
        let adapter_identity = self.access_adapter_identity(capability_identity)?;
        match AssertUnwindSafe(self.prepare_access_inner::<K>(
            capability_identity,
            &adapter_identity,
            target,
            request,
        ))
        .catch_unwind()
        .await
        {
            Ok(Ok(binding)) => Ok(binding),
            Ok(Err(_)) => Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Contract,
                adapter_identity,
            )),
            Err(_) => Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Callback,
                adapter_identity,
            )),
        }
    }

    async fn prepare_access_inner<K: PhysicalBindingKind>(
        &self,
        capability_identity: &QualifiedComponentIdentity,
        adapter_identity: &QualifiedComponentIdentity,
        target: AccessTargetSelection<'_>,
        request: CanonicalJsonValue,
    ) -> Result<Option<QualifiedPhysicalBinding<K>>> {
        let capability_contract_ref = &capability_identity.semantic_contract_ref;
        let capability = self
            .registry
            .live_components
            .get(&(
                StructuredComponentKind::Capability,
                capability_contract_ref.clone(),
            ))
            .ok_or_else(|| {
                CertifyError::Certification(
                    "Runtime capability is not process-qualified".to_owned(),
                )
            })?;
        let adapter_ref = &capability
            .dependencies
            .first()
            .filter(|dependency| dependency.component_kind == StructuredComponentKind::Adapter)
            .ok_or_else(|| {
                CertifyError::Certification(
                    "Runtime capability has no exact adapter dependency".to_owned(),
                )
            })?
            .contract_ref;
        let capability_process = self.process_component_by_identity(capability_identity)?;
        let adapter_process = self.process_component_by_identity(adapter_identity)?;
        if adapter_process.semantic_contract_ref != *adapter_ref {
            return Err(CertifyError::Certification(
                "Runtime capability selected a different qualified adapter".to_owned(),
            ));
        }
        let reserved_fact_capability_ref =
            prior_run_fact_selection_capability_contract()?.content_ref()?;
        let reserved_fact_adapter_ref = prior_run_fact_scanner_adapter_contract()?.content_ref()?;
        let selection = PhysicalBindingSelection {
            run_id: target.run_id,
            occurrence_id: target.occurrence_id,
            occurrence_path_ref: target.occurrence_path_ref,
            semantic_call_id: target.semantic_call_id,
            semantic_head: target.semantic_head,
            attempt_ordinal: target.attempt_ordinal,
            state_input_ref: target.state_input_ref,
            store_scope_id: target.store_scope_id,
            store_epoch: target.store_epoch,
            tenant_scope_id: target.tenant_scope_id,
            admitted_prior_run_source_manifest_ref: target.admitted_prior_run_source_manifest_ref,
            capability_contract_ref,
            capability_implementation_ref: &capability_process.implementation_contract_ref,
            adapter_contract_ref: adapter_ref,
            adapter_implementation_ref: &adapter_process.implementation_contract_ref,
            admitted_routing_policy_ref: target.admitted_routing_policy_ref,
            stable_resource_lineage_contract_ref: target.stable_resource_lineage_contract_ref,
            minimum_lineage_head_ref: target.minimum_lineage_head_ref,
        };
        let integrity_fault_code = stable_id("structured-runtime-access-contract-fault")?;
        let core = match (
            capability.capability_protocol.as_ref(),
            &capability_process.handle,
            &adapter_process.handle,
        ) {
            (
                Some(StructuredCapabilityProtocolContract::Read { .. }),
                ProcessHandle::ReadCapability(implementation),
                ProcessHandle::PriorRunFactScannerBindingSource,
            ) if !K::EFFECT
                && implementation.capability_type_id()
                    == TypeId::of::<PriorRunFactSelectionCapability>()
                && *capability_contract_ref == reserved_fact_capability_ref
                && *adapter_ref == reserved_fact_adapter_ref =>
            {
                implementation.validate_request(&request)?;
                let typed_request =
                    decode_process_value::<mfm_facts::FactSelectionRequest>(&request)?;
                if typed_request.admitted_source_manifest_ref().map_err(|_| {
                    CertifyError::Certification(
                        "prior-run fact request source cannot be decoded".to_owned(),
                    )
                })? != *selection.admitted_prior_run_source_manifest_ref
                    || typed_request.selector_contract_ref().map_err(|_| {
                        CertifyError::Certification(
                            "prior-run fact request selector cannot be decoded".to_owned(),
                        )
                    })? != mfm_facts::prior_run_fact_selector_contract_ref()
                        .map_err(|error| CertifyError::Certification(error.to_string()))?
                    || selection.stable_resource_lineage_contract_ref.is_some()
                {
                    return Err(CertifyError::Certification(
                        "prior-run fact request exceeds its admitted scanner target".to_owned(),
                    ));
                }
                let public_certificate = PriorRunFactScannerBindingCertificate::new(
                    selection.store_scope_id.clone(),
                    selection.store_epoch,
                    selection.tenant_scope_id.clone(),
                    selection.admitted_prior_run_source_manifest_ref.clone(),
                    mfm_facts::prior_run_fact_selector_contract_ref()
                        .map_err(|error| CertifyError::Certification(error.to_string()))?,
                    selection.capability_contract_ref.clone(),
                    selection.capability_implementation_ref.clone(),
                    selection.adapter_contract_ref.clone(),
                    selection.adapter_implementation_ref.clone(),
                )
                .to_history_object()
                .map_err(|error| CertifyError::Certification(error.to_string()))?;
                let expected_authorization = ExpectedAuthorization::new(
                    selection,
                    AccessKind::Read,
                    &request,
                    &mfm_spec::structured::structured_value_contract_ref::<
                        mfm_facts::FactSelectionRequest,
                    >()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?,
                    &structured_value_contract::<mfm_facts::FactSelectionRequest>()
                        .map_err(|error| CertifyError::Certification(error.to_string()))?
                        .schema_id(),
                    &public_certificate,
                )?;
                Some(QualifiedPhysicalBindingCore {
                    request,
                    public_certificate,
                    expected_authorization,
                    invocation: Box::new(PriorRunFactScanInvocation {
                        request: typed_request,
                        implementation: Arc::clone(implementation),
                    }),
                    process_identity: Arc::clone(&self.process_identity),
                    integrity_fault_code,
                })
            }
            (
                Some(StructuredCapabilityProtocolContract::Read { .. }),
                ProcessHandle::ReadCapability(implementation),
                ProcessHandle::ReadPhysicalBindingSource(source),
            ) if !K::EFFECT
                && *capability_contract_ref != reserved_fact_capability_ref
                && *adapter_ref != reserved_fact_adapter_ref =>
            {
                implementation.validate_request(&request)?;
                source
                    .resolve(
                        selection,
                        request,
                        Arc::clone(implementation),
                        Arc::clone(&self.process_identity),
                        integrity_fault_code,
                    )
                    .await?
            }
            (
                Some(StructuredCapabilityProtocolContract::Effect { .. }),
                ProcessHandle::EffectCapability(implementation),
                ProcessHandle::EffectPhysicalBindingSource(source),
            ) if K::EFFECT => {
                implementation.validate_request(&request)?;
                source
                    .resolve(
                        selection,
                        request,
                        Arc::clone(implementation),
                        Arc::clone(&self.process_identity),
                        integrity_fault_code,
                    )
                    .await?
            }
            _ => {
                return Err(CertifyError::Certification(
                    "Runtime capability, binding source, or requested access kind differs"
                        .to_owned(),
                ));
            }
        };
        Ok(core.map(|core| QualifiedPhysicalBinding {
            core,
            _kind: PhantomData,
        }))
    }

    /// Consumes one binding plus the store's non-cloneable proof of its exact
    /// newly committed authorization, then enters only the retained
    /// target-specific handle. No capability, request, certificate, or
    /// alternate invoker can be supplied at this stage.
    pub async fn invoke_qualified_physical_binding<K>(
        &self,
        binding: QualifiedPhysicalBinding<K>,
        authorization: NewlyAppendedAuthorization,
    ) -> QualifiedAccessCompletion {
        let QualifiedPhysicalBinding { core, .. } = binding;
        if !Arc::ptr_eq(&self.process_identity, &core.process_identity)
            || !core.expected_authorization.matches(
                authorization.authorization_ref(),
                authorization.authorization(),
            )
        {
            return QualifiedAccessCompletion::IntegrityFault(core.integrity_fault_code);
        }
        core.invocation
            .invoke(Some(authorization), core.integrity_fault_code)
            .await
    }

    #[cfg(test)]
    async fn invoke_qualified_physical_binding_for_test<K>(
        &self,
        binding: QualifiedPhysicalBinding<K>,
    ) -> QualifiedAccessCompletion {
        let QualifiedPhysicalBinding { core, .. } = binding;
        if !Arc::ptr_eq(&self.process_identity, &core.process_identity) {
            return QualifiedAccessCompletion::IntegrityFault(core.integrity_fault_code);
        }
        core.invocation
            .invoke(None, core.integrity_fault_code)
            .await
    }

    fn state_callbacks(
        &self,
        identity: &QualifiedComponentIdentity,
    ) -> std::result::Result<&dyn ErasedStateCallbacks, QualifiedProcessFault> {
        if identity.component_kind != StructuredComponentKind::State {
            return Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Contract,
                identity.clone(),
            ));
        }
        let component = self.process_component_by_identity(identity).map_err(|_| {
            QualifiedProcessFault::new(QualifiedProcessFaultCode::Contract, identity.clone())
        })?;
        let ProcessHandle::State(callbacks) = &component.handle else {
            return Err(QualifiedProcessFault::new(
                QualifiedProcessFaultCode::Contract,
                identity.clone(),
            ));
        };
        Ok(callbacks.as_ref())
    }

    fn process_component_by_identity(
        &self,
        identity: &QualifiedComponentIdentity,
    ) -> Result<&RegisteredProcessComponent> {
        let component = self.process_components.get(&(
            identity.component_kind,
            identity.semantic_contract_ref.clone(),
            identity.implementation_contract_ref.clone(),
        ));
        match component {
            Some(component)
                if self.registry.component_implementations.get(&(
                    identity.component_kind,
                    identity.semantic_contract_ref.clone(),
                )) == Some(&identity.implementation_contract_ref) =>
            {
                Ok(component)
            }
            _ => Err(CertifyError::Certification(
                "Runtime process identity is not registered".to_owned(),
            )),
        }
    }

    fn process_component(
        &self,
        kind: StructuredComponentKind,
        contract_ref: &ContentRef,
    ) -> Result<&RegisteredProcessComponent> {
        let implementation_ref = self
            .registry
            .component_implementations
            .get(&(kind, contract_ref.clone()))
            .ok_or_else(|| {
                CertifyError::Certification(
                    "Runtime semantic component has no implementation binding".to_owned(),
                )
            })?;
        self.process_components
            .get(&(kind, contract_ref.clone(), implementation_ref.clone()))
            .ok_or_else(|| {
                CertifyError::Certification(
                    "Runtime implementation binding has no process handle".to_owned(),
                )
            })
    }
}

/// Process-qualified callback-free registry snapshot used only to reverify
/// persisted certification documents.
///
/// The snapshot contains no state callback, capability implementation,
/// adapter invoker, signer, resource, store, or writer handle.
#[derive(Debug, Clone)]
pub struct AdmissionVerificationRegistry {
    registry: Arc<StructuredCertificationRegistry>,
    entry_points: BTreeMap<StableId, EntryPointDefinition>,
}

impl AdmissionVerificationRegistry {
    /// Recomputes the exact qualified document from one persisted authority root
    /// and its content-addressed authored-program object.
    pub fn verify_root(
        &self,
        entry_point_id: &StableId,
        root: &mfm_spec::structured::CertifiedProgramRoot,
        authored: &CanonicalJsonValue,
    ) -> Result<CertifiedProgram> {
        let entry = self.entry_points.get(entry_point_id).ok_or_else(|| {
            CertifyError::Verification("entry point is not process-qualified".to_owned())
        })?;
        AdmissionVerifier {
            entry,
            registry: &self.registry,
        }
        .verify_root(root, authored)
    }

    /// Re-runs the exact qualified entry's pure certification predicates and
    /// returns the verified persisted program authority.
    pub fn verify(
        &self,
        entry_point_id: &StableId,
        document: &CertifiedProgramDocument,
    ) -> Result<CertifiedProgram> {
        let entry = self.entry_points.get(entry_point_id).ok_or_else(|| {
            CertifyError::Verification("entry point is not process-qualified".to_owned())
        })?;
        AdmissionVerifier {
            entry,
            registry: &self.registry,
        }
        .verify(document)
    }
}

/// Opaque borrowed authority that certifies only its registry-selected entry.
pub struct EntryPointCertifier<'registry> {
    entry: &'registry EntryPointDefinition,
    registry: &'registry StructuredCertificationRegistry,
}

impl EntryPointCertifier<'_> {
    /// Expands, proves, and certifies one deterministic candidate under the
    /// immutable registered entry signature, profile, policy, and support envelope.
    pub fn certify(&self, authored: AuthoredStructuredProgram) -> Result<CertifiedProgram> {
        prepare_entry(self.entry, authored, self.registry)
    }
}

/// Purpose-limited verifier for persisted certification data. It owns no
/// registry mutation, implementation invocation, IO, history, or writer
/// authority.
pub struct AdmissionVerifier<'registry> {
    entry: &'registry EntryPointDefinition,
    registry: &'registry StructuredCertificationRegistry,
}

impl AdmissionVerifier<'_> {
    /// Recomputes the full qualified closure and requires one exact persisted root.
    pub fn verify_root(
        &self,
        root: &mfm_spec::structured::CertifiedProgramRoot,
        authored: &CanonicalJsonValue,
    ) -> Result<CertifiedProgram> {
        let authored: AuthoredStructuredProgram =
            serde_json::from_value(authored.as_json().clone()).map_err(|_| {
                CertifyError::Verification(
                    "persisted authored program cannot be strictly decoded".to_owned(),
                )
            })?;
        let certified = prepare_entry(self.entry, authored, self.registry)?;
        if &certified.document().root != root {
            return Err(CertifyError::Verification(
                "persisted certification root differs from exact qualified recomputation"
                    .to_owned(),
            ));
        }
        Ok(certified)
    }

    /// Re-runs the fixed kernel predicate set and exact declarative expansion,
    /// then requires byte-for-byte equality with the persisted document.
    pub fn verify(&self, document: &CertifiedProgramDocument) -> Result<CertifiedProgram> {
        let authored = document
            .component_closure
            .iter()
            .find(|object| {
                object.object_type.as_str() == AUTHORED_OBJECT_TYPE
                    && object.content_ref == document.root.components.authored_program_ref
            })
            .ok_or_else(|| {
                CertifyError::Verification(
                    "persisted certified closure has no exact authored program".to_owned(),
                )
            })?;
        let certified = self.verify_root(&document.root, &authored.value)?;
        if certified.document() != document {
            return Err(CertifyError::Verification(
                "persisted certification differs from exact qualified recomputation".to_owned(),
            ));
        }
        Ok(certified)
    }
}

/// Returns the canonical object reference for an exact semantic component manifest.
pub fn structured_component_manifest_ref(
    manifest: &StateCapabilityAdapterSignerResourceManifest,
) -> Result<ContentRef> {
    typed_content_ref("mfm.structured-component-manifest", manifest)
}

/// Returns the canonical object reference for an exact secret-free implementation manifest.
pub fn secret_free_implementation_manifest_ref(
    manifest: &SecretFreeImplementationManifest,
) -> Result<ContentRef> {
    typed_content_ref(
        "mfm.structured-secret-free-implementation-manifest",
        manifest,
    )
}

struct CoreCertification {
    authored: AuthoredStructuredProgram,
    expanded: ExpandedStructuredProgram,
    profile: StructuredExpansionProfile,
    proof: StructuredExpansionProof,
    coverage: StructuredPolicyCoverageProof,
    component_manifest: StateCapabilityAdapterSignerResourceManifest,
    implementation_manifest: SecretFreeImplementationManifest,
}

fn expand_and_prove(
    authored: AuthoredStructuredProgram,
    profile: StructuredExpansionProfile,
    registry: &StructuredCertificationRegistry,
) -> Result<CoreCertification> {
    validate_profile(&profile, registry)?;
    validate_authored_program(&authored, registry)?;

    let authored_ref = authored
        .content_ref()
        .map_err(|error| CertifyError::Planning(error.to_string()))?;
    let mut expansion = ExpansionContext::new(registry, &profile);
    let bindings = initial_bindings(&authored.input_roots)?;
    let expanded_root = expansion.expand_block(
        &authored.root,
        bindings,
        Vec::new(),
        authored.root.path.clone(),
        Vec::new(),
    )?;
    let expanded = ExpandedStructuredProgram {
        operation_id: authored.operation_id.clone(),
        input_roots: authored.input_roots.clone(),
        output_contract_ref: authored.output_contract_ref.clone(),
        failure_contract: authored.failure_contract.clone(),
        root: expanded_root,
    };
    let eligible_boundaries = expansion.eligible_boundaries.clone();
    validate_expanded_program(&expanded, &profile, registry)?;
    let expanded_ref = expanded
        .content_ref()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    expansion.trace.sort_by_key(|entry| entry.stage);
    let proof = StructuredExpansionProof {
        authored_program_ref: authored_ref.clone(),
        expanded_program_ref: expanded_ref.clone(),
        expansion_profile_ref: profile.content_ref()?,
        substitution_trace: expansion.trace,
    };
    let coverage = StructuredPolicyCoverageProof {
        authored_program_ref: authored_ref.clone(),
        expanded_program_ref: expanded_ref,
        expansion_profile_ref: profile.content_ref()?,
        entries: expansion.coverage,
    };
    if proof
        .substitution_trace
        .windows(2)
        .any(|entries| entries[0].stage > entries[1].stage)
    {
        return Err(CertifyError::Certification(
            "expansion trace is not in frozen pipeline phase order".to_owned(),
        ));
    }
    validate_policy_coverage(&eligible_boundaries, &profile, &coverage)?;

    let (component_manifest, implementation_manifest) = component_manifests(&expanded, registry)?;

    Ok(CoreCertification {
        authored,
        expanded,
        profile,
        proof,
        coverage,
        component_manifest,
        implementation_manifest,
    })
}

fn prepare_entry(
    entry: &EntryPointDefinition,
    authored: AuthoredStructuredProgram,
    registry: &StructuredCertificationRegistry,
) -> Result<CertifiedProgram> {
    let expected_inputs = entry
        .coverage_template
        .input_roots
        .iter()
        .map(|root| &root.contract_ref)
        .collect::<Vec<_>>();
    let actual_inputs = authored
        .input_roots
        .iter()
        .map(|root| &root.contract_ref)
        .collect::<Vec<_>>();
    if authored.operation_id != entry.entry_point_id
        || actual_inputs != expected_inputs
        || authored.output_contract_ref != entry.coverage_template.output_contract_ref
        || authored.failure_contract != entry.coverage_template.failure_contract
    {
        return Err(CertifyError::Certification(
            "candidate program differs from the qualified entry signature".to_owned(),
        ));
    }
    let mut working_registry = registry.clone();
    let (certified_program_contract_ref, predicate_set_ref, coverage_proof_ref) =
        register_kernel_qualification_objects(&mut working_registry)?;
    register_profile_policy_objects(&entry.profile, &mut working_registry)?;
    let core = expand_and_prove(authored, entry.profile.clone(), &working_registry)?;
    if let Some(envelope) = &entry.support_envelope {
        let semantic_components = core
            .component_manifest
            .entries
            .iter()
            .map(|component| {
                (
                    component.component_kind,
                    component.semantic_contract_ref.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        let implementations = core
            .implementation_manifest
            .entries
            .iter()
            .map(|implementation| {
                (
                    implementation.component_kind,
                    implementation.semantic_contract_ref.clone(),
                    implementation.implementation_contract_ref.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        if !semantic_components.is_subset(&envelope.semantic_components)
            || !implementations.is_subset(&envelope.implementations)
        {
            return Err(CertifyError::Certification(
                "candidate program exceeds the qualified entry support envelope".to_owned(),
            ));
        }
    }
    let policy = QualifiedStructuredEntryPointPolicy::new(
        entry.entry_point_id.clone(),
        entry.entry_point_contract_ref.clone(),
        certified_program_contract_ref,
        predicate_set_ref,
        core.profile.content_ref()?,
        coverage_proof_ref,
        core.authored
            .input_roots
            .iter()
            .map(|root| root.contract_ref.clone())
            .collect(),
        core.authored.output_contract_ref.clone(),
        core.authored.failure_contract.contract_ref()?,
    )?;
    validate_policy_root(&core.authored, &policy, &core.profile)?;
    working_registry.register_entry_point_policy(&policy)?;
    let document = build_certified_document(
        &core.authored,
        &core.expanded,
        &core.profile,
        &core.proof,
        &core.coverage,
        &core.component_manifest,
        &core.implementation_manifest,
        &policy,
        &working_registry,
    )?;
    verify_certified_document(&document, &policy, &working_registry)?;
    let value_schemas = qualified_value_schemas(&document, &working_registry)?;
    Ok(CertifiedProgram {
        authored: core.authored,
        expanded: core.expanded,
        profile: core.profile,
        proof: core.proof,
        coverage: core.coverage,
        document,
        value_schemas,
    })
}

fn qualified_value_schemas(
    document: &CertifiedProgramDocument,
    registry: &StructuredCertificationRegistry,
) -> Result<BTreeMap<ContentRef, SchemaIdentity>> {
    let mut schemas = BTreeMap::new();
    for component in &document.component_closure {
        if component.object_type != stable_id(DATA_CONTRACT_OBJECT_TYPE)? {
            continue;
        }
        let canonical = component
            .value
            .canonical_json()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        let Ok(contract) = RetainedValueContract::strict_decode(canonical.as_bytes()) else {
            continue;
        };
        let contract_ref = retained_value_contract_ref(&contract)?;
        if contract_ref != component.content_ref {
            return Err(CertifyError::Certification(
                "retained value contract reference differs from its exact bytes".to_owned(),
            ));
        }
        let schema = registry.value_schemas.get(&contract_ref).ok_or_else(|| {
            CertifyError::Certification(
                "certified retained value contract has no qualified schema identity".to_owned(),
            )
        })?;
        if schema
            .schema_id()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            != *contract.schema_id()
            || schema.semantic_type_id.as_ref() != Some(contract.semantic_type_id())
        {
            return Err(CertifyError::Certification(
                "qualified schema identity differs from retained value contract".to_owned(),
            ));
        }
        insert_exact(
            &mut schemas,
            contract_ref,
            schema.clone(),
            "certified retained value schema identity",
        )?;
    }
    Ok(schemas)
}

fn register_kernel_qualification_objects(
    registry: &mut StructuredCertificationRegistry,
) -> Result<(ContentRef, ContentRef, ContentRef)> {
    let certified_marker = "mfm.certified-program-contract.v1";
    let coverage_marker = "mfm.policy-coverage-proof-contract.v1";
    let predicates = [
        "authored-structure-v1",
        "declarative-expansion-v1",
        "expanded-structure-v1",
        "exact-policy-coverage-v1",
        "structural-first-use-manifest-v1",
        "canonical-component-closure-v1",
    ];
    let certified_ref = typed_content_ref("mfm.structured-kernel-contract", &certified_marker)?;
    let coverage_ref = typed_content_ref("mfm.structured-kernel-contract", &coverage_marker)?;
    let predicate_ref =
        typed_content_ref("mfm.structured-certification-predicate-set", &predicates)?;
    registry.register_component_object(component_object(
        CERTIFIED_PROGRAM_CONTRACT_OBJECT_TYPE,
        certified_ref.clone(),
        &certified_marker,
        Vec::new(),
    )?)?;
    registry.register_component_object(component_object(
        POLICY_COVERAGE_CONTRACT_OBJECT_TYPE,
        coverage_ref.clone(),
        &coverage_marker,
        Vec::new(),
    )?)?;
    registry.register_component_object(component_object(
        PREDICATE_SET_OBJECT_TYPE,
        predicate_ref.clone(),
        &predicates,
        Vec::new(),
    )?)?;
    Ok((certified_ref, predicate_ref, coverage_ref))
}

fn register_profile_policy_objects(
    profile: &StructuredExpansionProfile,
    registry: &mut StructuredCertificationRegistry,
) -> Result<()> {
    for policy in &profile.policies {
        let value =
            CanonicalJsonValue::from_canonical_json(policy.canonical_contract_json()?.as_bytes())?;
        let mut outbound_references = Vec::with_capacity(policy.boundary_recipes.len() * 2);
        for binding in &policy.boundary_recipes {
            outbound_references.push(ComponentObjectReference {
                object_type: stable_id(STATE_CONTRACT_OBJECT_TYPE)?,
                content_ref: binding.boundary_contract_ref.clone(),
            });
            outbound_references.push(ComponentObjectReference {
                object_type: stable_id(POLICY_RECIPE_OBJECT_TYPE)?,
                content_ref: binding.recipe_ref.clone(),
            });
        }
        registry.register_component_object(RegisteredComponentObject {
            object: CertifiedComponentObject {
                object_type: stable_id(EXPANSION_POLICY_CONTRACT_OBJECT_TYPE)?,
                content_ref: policy.policy_ref.clone(),
                value,
            },
            outbound_references,
        })?;
    }
    Ok(())
}

struct ExpansionContext<'a> {
    registry: &'a StructuredCertificationRegistry,
    profile: &'a StructuredExpansionProfile,
    trace: Vec<ExpansionTraceEntry>,
    coverage: Vec<PolicyCoverageEntry>,
    child_stack: Vec<ContentRef>,
    eligible_boundaries: Vec<(SemanticCallId, ContentRef)>,
    eligible_boundary_ids: BTreeSet<SemanticCallId>,
    support_expansion: Option<SupportExpansionIdentity>,
    active_policy_proceed: Option<ActivePolicyProceed>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SupportExpansionKind {
    Capability,
    Policy,
}

#[derive(Clone)]
struct SupportExpansionIdentity {
    kind: SupportExpansionKind,
    protected_semantic_call_id: SemanticCallId,
    expansion_ref: ContentRef,
}

#[derive(Clone)]
struct AuthoredCallOrigin {
    semantic_call_id: SemanticCallId,
    occurrence_path: StructuralPath,
}

impl AuthoredCallOrigin {
    fn state(call: &AuthoredStateCall) -> Self {
        Self {
            semantic_call_id: call.semantic_call_id.clone(),
            occurrence_path: call.output_slot.lexical_path.clone(),
        }
    }

    fn operation(call: &AuthoredOperationCall) -> Self {
        Self {
            semantic_call_id: call.semantic_call_id.clone(),
            occurrence_path: call.output_slot.lexical_path.clone(),
        }
    }
}

struct ActivePolicyProceed {
    boundary: Option<ExpandedBoundary>,
    failure_post: Option<AuthoredStructuredProgram>,
    policy_ref: ContentRef,
    recipe_root_path: StructuralPath,
}

impl<'a> ExpansionContext<'a> {
    fn new(
        registry: &'a StructuredCertificationRegistry,
        profile: &'a StructuredExpansionProfile,
    ) -> Self {
        Self {
            registry,
            profile,
            trace: Vec::new(),
            coverage: Vec::new(),
            child_stack: Vec::new(),
            eligible_boundaries: Vec::new(),
            eligible_boundary_ids: BTreeSet::new(),
            support_expansion: None,
            active_policy_proceed: None,
        }
    }

    fn expand_block(
        &mut self,
        authored: &AuthoredBlock,
        mut bindings: SlotBindings,
        inherited_mappers: Vec<FailureMapperRegistration>,
        instantiated_path: StructuralPath,
        semantic_prefix: Vec<mfm_spec::structured::SemanticPathSegment>,
    ) -> Result<ExpandedBlock> {
        let visible_mappers = match &authored.failure_scope {
            FailureScopeBinding::Owns { scope } => scope.default_mappers.clone(),
            FailureScopeBinding::Inherits { .. } => inherited_mappers,
        };
        let mut declarations = Vec::with_capacity(authored.declarations.len());
        let mut failure_exits = Vec::new();
        for (ordinal, declaration) in authored.declarations.iter().enumerate() {
            let ordinal = u32::try_from(ordinal).map_err(|_| {
                CertifyError::Planning("declaration ordinal exceeds u32".to_owned())
            })?;
            match declaration {
                AuthoredDeclaration::State(state) => {
                    let authored_origin = AuthoredCallOrigin::state(state);
                    let (state, occurrence_path) = instantiate_state_call(
                        state,
                        &instantiated_path,
                        &semantic_prefix,
                        ordinal,
                        self.support_expansion.as_ref(),
                    )?;
                    let resolved = self.expand_state(
                        &state,
                        &authored_origin,
                        &occurrence_path,
                        &bindings,
                        &visible_mappers,
                        authored.failure_scope.scope_id(),
                        authored.failure_scope.failure_contract(),
                    )?;
                    bindings.insert(
                        declaration_output_slot(declaration),
                        resolved.output_slot().clone(),
                    )?;
                    failure_exits.extend(resolved.failure_exits.iter().cloned());
                    declarations.push(resolved.declaration);
                }
                AuthoredDeclaration::OperationCall(call) => {
                    if call.child_program_ref == policy_proceed_program_ref()? {
                        let (call, occurrence_path) = instantiate_operation_call(
                            call,
                            &instantiated_path,
                            &semantic_prefix,
                            ordinal,
                            self.support_expansion.as_ref(),
                        )?;
                        let fragment =
                            self.expand_policy_proceed(&call, &occurrence_path, &bindings)?;
                        bindings.insert(
                            declaration_output_slot(declaration),
                            fragment.success_slot.clone(),
                        )?;
                        failure_exits
                            .extend(fragment_failure_scope_exits(&fragment.failure_boundary));
                        declarations.push(ExpandedDeclaration::Fragment(Box::new(fragment)));
                    } else if self
                        .support_expansion
                        .as_ref()
                        .is_some_and(|support| support.kind == SupportExpansionKind::Policy)
                    {
                        return Err(CertifyError::Certification(
                            "expansion support cannot contain child operations".to_owned(),
                        ));
                    } else {
                        let authored_origin = AuthoredCallOrigin::operation(call);
                        let (call, occurrence_path) = instantiate_operation_call(
                            call,
                            &instantiated_path,
                            &semantic_prefix,
                            ordinal,
                            self.support_expansion.as_ref(),
                        )?;
                        let fragment = self.expand_child(
                            &call,
                            &authored_origin,
                            &occurrence_path,
                            &bindings,
                            &visible_mappers,
                            authored.failure_scope.scope_id(),
                            authored.failure_scope.failure_contract(),
                        )?;
                        bindings.insert(
                            declaration_output_slot(declaration),
                            fragment.success_slot.clone(),
                        )?;
                        failure_exits
                            .extend(fragment_failure_scope_exits(&fragment.failure_boundary));
                        declarations.push(ExpandedDeclaration::Fragment(Box::new(fragment)));
                    }
                }
                AuthoredDeclaration::Match(binding) => {
                    let path = declaration_path(&instantiated_path, &binding.label, ordinal)?;
                    let nested_semantic =
                        declaration_semantic_prefix(&semantic_prefix, &binding.label)?;
                    let expanded = self.expand_match(
                        binding,
                        path,
                        nested_semantic,
                        &bindings,
                        &visible_mappers,
                    )?;
                    bindings.insert(&binding.output_slot, expanded.output_slot.clone())?;
                    for arm in &expanded.arms {
                        failure_exits.extend(arm.body.failure_exits.iter().cloned());
                    }
                    declarations.push(ExpandedDeclaration::Match(Box::new(expanded)));
                }
                AuthoredDeclaration::FanOut(group) => {
                    let path = declaration_path(&instantiated_path, &group.label, ordinal)?;
                    let nested_semantic =
                        declaration_semantic_prefix(&semantic_prefix, &group.label)?;
                    let expanded = self.expand_fan_out(group, path, nested_semantic, &bindings)?;
                    bindings.insert(&group.output_slot, expanded.output_slot.clone())?;
                    declarations.push(ExpandedDeclaration::FanOut(Box::new(expanded)));
                }
            }
        }
        let tail = resolve_tail(&authored.tail, &bindings)?;
        if let BlockTail::ScopeFailure(slot) = &tail {
            failure_exits.push(slot.clone());
        }
        dedup_slots(&mut failure_exits)?;
        let scope_contract = authored.failure_scope.failure_contract();
        if matches!(scope_contract, StructuredFailureContract::Never) && !failure_exits.is_empty() {
            return Err(CertifyError::Certification(
                "Never block exposes a typed failure exit".to_owned(),
            ));
        }
        if let StructuredFailureContract::Typed { contract_ref, .. } = scope_contract {
            if failure_exits
                .iter()
                .any(|slot| &slot.contract_ref != contract_ref)
            {
                return Err(CertifyError::Certification(
                    "block failure exit differs from its lexical scope contract".to_owned(),
                ));
            }
        }
        Ok(ExpandedBlock {
            path: instantiated_path,
            failure_scope: authored.failure_scope.clone(),
            declarations,
            failure_exits,
            tail,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_state(
        &mut self,
        state: &AuthoredStateCall,
        authored_origin: &AuthoredCallOrigin,
        occurrence_path: &StructuralPath,
        bindings: &SlotBindings,
        visible_mappers: &[FailureMapperRegistration],
        scope_id: &StableId,
        scope_failure: &StructuredFailureContract,
    ) -> Result<ExpandedStateResult> {
        if self.support_expansion.is_none() {
            if !self
                .eligible_boundary_ids
                .insert(state.semantic_call_id.clone())
            {
                return Err(CertifyError::Certification(
                    "duplicate instantiated semantic call identity".to_owned(),
                ));
            }
            self.eligible_boundaries.push((
                state.semantic_call_id.clone(),
                state.contract.state_contract_ref.clone(),
            ));
        }
        let registered = self
            .registry
            .state_contract(&state.contract.state_contract_ref)
            .ok_or_else(|| {
                CertifyError::Certification("state contract is not registered".to_owned())
            })?;
        if registered != &state.contract {
            return Err(CertifyError::Certification(
                "authored state contract differs from the qualified registry".to_owned(),
            ));
        }
        let inputs = state
            .inputs
            .iter()
            .map(|slot| bindings.resolve(slot))
            .collect::<Result<Vec<_>>>()?;
        let occurrence_path = occurrence_path.clone();
        let has_policy = self.support_expansion.is_none()
            && self.profile.policies.iter().any(|policy| {
                policy
                    .recipe_for(&state.contract.state_contract_ref)
                    .is_some()
            });

        if let Some(requirement_ref) = &state.contract.capability_requirement_ref {
            if self.support_expansion.is_some() {
                return Err(CertifyError::Certification(
                    "injected support state requests recursive capability expansion".to_owned(),
                ));
            }
            let recipe = self
                .registry
                .capability_recipes
                .get(requirement_ref)
                .cloned()
                .ok_or_else(|| {
                    CertifyError::Planning(
                        "semantic capability requirement has no qualified recipe".to_owned(),
                    )
                })?;
            let boundary = self.expand_capability_recipe(
                state,
                &occurrence_path,
                inputs,
                requirement_ref,
                &recipe,
            )?;
            validate_external_boundary(state, boundary.fragment())?;
            let boundary = self.apply_policies(state, boundary)?;
            let mut fragment = boundary.into_fragment();
            validate_external_boundary(state, &fragment)?;
            complete_external_failure_boundary(
                self,
                state,
                authored_origin,
                &mut fragment,
                bindings,
                visible_mappers,
                scope_id,
                scope_failure,
            )?;
            self.record_external_trace(state, &fragment, Some(requirement_ref))?;
            self.record_failure_completion(state, &fragment.boundary_id)?;
            return Ok(ExpandedStateResult {
                declaration: ExpandedDeclaration::Fragment(Box::new(fragment.clone())),
                output_slot: fragment.success_slot,
                failure_exits: fragment_failure_scope_exits(&fragment.failure_boundary),
            });
        }

        if has_policy {
            let boundary = state_binding_expansion_boundary(state, occurrence_path, inputs)?;
            let boundary = self.apply_policies(state, boundary)?;
            let mut fragment = boundary.into_fragment();
            validate_external_boundary(state, &fragment)?;
            complete_external_failure_boundary(
                self,
                state,
                authored_origin,
                &mut fragment,
                bindings,
                visible_mappers,
                scope_id,
                scope_failure,
            )?;
            self.record_external_trace(state, &fragment, None)?;
            self.record_failure_completion(state, &fragment.boundary_id)?;
            let failure_exits = fragment_failure_scope_exits(&fragment.failure_boundary);
            return Ok(ExpandedStateResult {
                output_slot: fragment.success_slot.clone(),
                declaration: ExpandedDeclaration::Fragment(Box::new(fragment)),
                failure_exits,
            });
        }

        let occurrence_id = occurrence_path
            .occurrence_id()
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        let output_slot = state_output_slot(
            &occurrence_path,
            &occurrence_id,
            &state.contract.output_contract_ref,
            ResultRole::SuccessOutput,
        );
        let failure_boundary = build_failure_boundary(
            self,
            state,
            authored_origin,
            &occurrence_id,
            &occurrence_path,
            bindings,
            visible_mappers,
            scope_id,
            scope_failure,
        )?;
        let binding = ExpandedStateBinding {
            semantic_call_id: state.semantic_call_id.clone(),
            occurrence_id,
            occurrence_path,
            label: state.label.clone(),
            contract: state.contract.clone(),
            inputs,
            output_slot: output_slot.clone(),
            failure_boundary,
        };

        if matches!(
            &binding.failure_boundary,
            CertifiedFailureBoundary::Typed { .. }
        ) {
            self.trace.push(ExpansionTraceEntry {
                stage: ExpansionStage::FailureCompletion,
                semantic_call_id: state.semantic_call_id.clone(),
                expansion_ref: state.contract.failure_contract.contract_ref()?,
                boundary_id: ExpansionBoundaryId::State(binding.occurrence_id.clone()),
            });
        }
        let failure_exits = failure_boundary_scope_exits(&binding.failure_boundary);
        Ok(ExpandedStateResult {
            declaration: ExpandedDeclaration::State(Box::new(binding)),
            output_slot,
            failure_exits,
        })
    }

    fn expand_capability_recipe(
        &mut self,
        state: &AuthoredStateCall,
        occurrence_path: &StructuralPath,
        inputs: Vec<LexicalSlot>,
        requirement_ref: &ContentRef,
        recipe: &AuthoredStructuredProgram,
    ) -> Result<ExpandedBoundary> {
        validate_authored_program(recipe, self.registry)?;
        if recipe.input_roots.len() != inputs.len()
            || recipe.output_contract_ref != state.contract.output_contract_ref
            || recipe.failure_contract != state.contract.failure_contract
        {
            return Err(CertifyError::Certification(
                "capability recipe changes its semantic call boundary".to_owned(),
            ));
        }
        let fragment_path = occurrence_path.child(StructuralPathSegment::Fragment {
            label: state.label.clone(),
            expansion_ref: requirement_ref.clone(),
        })?;
        let boundary_id = fragment_path.fragment_boundary_id()?;
        let mut recipe_bindings = SlotBindings::default();
        let mut input_bindings = Vec::with_capacity(inputs.len());
        for (root, source) in recipe.input_roots.iter().zip(inputs) {
            let LexicalProducer::AdmissionRoot { root_id } = &root.producer else {
                return Err(CertifyError::Certification(
                    "capability recipe input is not an admission root".to_owned(),
                ));
            };
            if root.contract_ref != source.contract_ref {
                return Err(CertifyError::Certification(
                    "capability recipe input mapping is not an exact ordered bijection".to_owned(),
                ));
            }
            let rebound = LexicalSlot {
                lexical_path: fragment_path.clone(),
                contract_ref: root.contract_ref.clone(),
                producer: LexicalProducer::FragmentInput {
                    boundary_id: boundary_id.clone(),
                    child_root_id: root_id.clone(),
                    source: Box::new(source.clone()),
                },
            };
            recipe_bindings.insert(root, rebound)?;
            input_bindings.push(FragmentInputBinding {
                child_root_id: root_id.clone(),
                child_contract_ref: root.contract_ref.clone(),
                caller_slot: source,
            });
        }
        let previous_support = self.support_expansion.replace(SupportExpansionIdentity {
            kind: SupportExpansionKind::Capability,
            protected_semantic_call_id: state.semantic_call_id.clone(),
            expansion_ref: requirement_ref.clone(),
        });
        let expanded_body = self.expand_block(
            &recipe.root,
            recipe_bindings,
            Vec::new(),
            fragment_path.clone(),
            state.semantic_path.segments().to_vec(),
        );
        self.support_expansion = previous_support;
        let mut body = expanded_body?;
        let body_success = match &body.tail {
            BlockTail::Normal(slot) if slot.contract_ref == recipe.output_contract_ref => {
                slot.clone()
            }
            _ => {
                return Err(CertifyError::Certification(
                    "capability recipe has no exact normal boundary source".to_owned(),
                ));
            }
        };
        let success_slot = LexicalSlot {
            lexical_path: fragment_path.clone(),
            contract_ref: recipe.output_contract_ref.clone(),
            producer: LexicalProducer::FragmentBoundary {
                boundary_id: boundary_id.clone(),
                role: ResultRole::SuccessOutput,
                source: Box::new(body_success),
            },
        };
        let failure_boundary = provisional_fragment_failure_boundary(
            &state.semantic_call_id,
            &fragment_path,
            &boundary_id,
            &recipe.failure_contract,
            &mut body,
        )?;
        Ok(ExpandedBoundary::from_leaf(ExpandedFragment {
            semantic_call_id: state.semantic_call_id.clone(),
            label: state.label.clone(),
            path: fragment_path,
            boundary_id,
            input_bindings,
            body,
            success_slot,
            failure_boundary,
        }))
    }

    fn expand_policy_recipe(
        &mut self,
        state: &AuthoredStateCall,
        boundary: ExpandedBoundary,
        policy_ref: &ContentRef,
        recipe: &PolicyExpansionRecipe,
    ) -> Result<ExpandedBoundary> {
        let program = recipe.program();
        let protected_inputs = boundary.fragment().input_bindings.clone();
        if program.input_roots.len() != protected_inputs.len()
            || program.output_contract_ref != state.contract.output_contract_ref
            || program.failure_contract != state.contract.failure_contract
        {
            return Err(CertifyError::Certification(
                "policy recipe changes its protected semantic boundary".to_owned(),
            ));
        }
        let declaration_path = fragment_declaration_path(&boundary.fragment().path)?;
        let outer_path = declaration_path.child(StructuralPathSegment::Fragment {
            label: state.label.clone(),
            expansion_ref: policy_ref.clone(),
        })?;
        let outer_boundary_id = outer_path.fragment_boundary_id()?;
        let mut root_bindings = SlotBindings::default();
        let mut outer_input_bindings = Vec::with_capacity(protected_inputs.len());
        for (root, protected_input) in program.input_roots.iter().zip(protected_inputs) {
            let LexicalProducer::AdmissionRoot { root_id } = &root.producer else {
                return Err(CertifyError::Certification(
                    "policy recipe input is not an admission root".to_owned(),
                ));
            };
            if root.contract_ref != protected_input.child_contract_ref {
                return Err(CertifyError::Certification(
                    "policy recipe input mapping is not an exact ordered bijection".to_owned(),
                ));
            }
            let outer_input = LexicalSlot {
                lexical_path: outer_path.clone(),
                contract_ref: root.contract_ref.clone(),
                producer: LexicalProducer::FragmentInput {
                    boundary_id: outer_boundary_id.clone(),
                    child_root_id: root_id.clone(),
                    source: Box::new(protected_input.caller_slot.clone()),
                },
            };
            root_bindings.insert(root, outer_input)?;
            outer_input_bindings.push(FragmentInputBinding {
                child_root_id: root_id.clone(),
                child_contract_ref: root.contract_ref.clone(),
                caller_slot: protected_input.caller_slot,
            });
        }

        if self.active_policy_proceed.is_some() {
            return Err(CertifyError::Certification(
                "policy recipe expansion attempted recursive proceed substitution".to_owned(),
            ));
        }
        self.active_policy_proceed = Some(ActivePolicyProceed {
            boundary: Some(boundary),
            failure_post: recipe.failure_post().cloned(),
            policy_ref: policy_ref.clone(),
            recipe_root_path: outer_path.clone(),
        });
        let previous_support = self.support_expansion.replace(SupportExpansionIdentity {
            kind: SupportExpansionKind::Policy,
            protected_semantic_call_id: state.semantic_call_id.clone(),
            expansion_ref: policy_ref.clone(),
        });
        let expanded_body = self.expand_block(
            &program.root,
            root_bindings,
            Vec::new(),
            outer_path.clone(),
            state.semantic_path.segments().to_vec(),
        );
        self.support_expansion = previous_support;
        let mut body = match expanded_body {
            Ok(body) => body,
            Err(error) => {
                self.active_policy_proceed = None;
                return Err(error);
            }
        };
        let active_proceed = self.active_policy_proceed.take().ok_or_else(|| {
            CertifyError::Certification(
                "policy recipe lost its affine proceed substitution state".to_owned(),
            )
        })?;
        let remaining_proceed = active_proceed.boundary;
        if remaining_proceed.is_some() {
            return Err(CertifyError::Certification(
                "policy recipe did not consume its affine proceed boundary".to_owned(),
            ));
        }
        let body_success = match &body.tail {
            BlockTail::Normal(slot) if slot.contract_ref == program.output_contract_ref => {
                slot.clone()
            }
            _ => {
                return Err(CertifyError::Certification(
                    "policy recipe has no exact normal boundary source".to_owned(),
                ));
            }
        };
        let success_slot = LexicalSlot {
            lexical_path: outer_path.clone(),
            contract_ref: program.output_contract_ref.clone(),
            producer: LexicalProducer::FragmentBoundary {
                boundary_id: outer_boundary_id.clone(),
                role: ResultRole::SuccessOutput,
                source: Box::new(body_success),
            },
        };
        let failure_boundary = provisional_fragment_failure_boundary(
            &state.semantic_call_id,
            &outer_path,
            &outer_boundary_id,
            &program.failure_contract,
            &mut body,
        )?;
        Ok(ExpandedBoundary {
            fragment: ExpandedFragment {
                semantic_call_id: state.semantic_call_id.clone(),
                label: state.label.clone(),
                path: outer_path,
                boundary_id: outer_boundary_id,
                input_bindings: outer_input_bindings,
                body,
                success_slot,
                failure_boundary,
            },
        })
    }

    fn expand_policy_proceed(
        &mut self,
        call: &AuthoredOperationCall,
        occurrence_path: &StructuralPath,
        bindings: &SlotBindings,
    ) -> Result<ExpandedFragment> {
        let mut active = self.active_policy_proceed.take().ok_or_else(|| {
            CertifyError::Certification(
                "reserved policy proceed appeared outside a qualified recipe".to_owned(),
            )
        })?;
        let recipe_root_segments = active.recipe_root_path.segments();
        let occurrence_segments = occurrence_path.segments();
        if !occurrence_segments.starts_with(recipe_root_segments) {
            return Err(CertifyError::Certification(
                "policy proceed escaped its qualified recipe root".to_owned(),
            ));
        }
        if occurrence_segments[recipe_root_segments.len()..]
            .iter()
            .any(|segment| matches!(segment, StructuralPathSegment::FanOutLane { .. }))
        {
            return Err(CertifyError::Certification(
                "policy proceed cannot be captured inside FanOut".to_owned(),
            ));
        }
        let boundary = active.boundary.take().ok_or_else(|| {
            CertifyError::Certification(
                "policy recipe cloned or reused its affine proceed boundary".to_owned(),
            )
        })?;
        if call.output_contract_ref != boundary.fragment().success_slot.contract_ref
            || call.failure_contract
                != fragment_failure_contract(&boundary.fragment().failure_boundary)?
            || call.input_bindings.len() != boundary.fragment().input_bindings.len()
        {
            return Err(CertifyError::Certification(
                "policy proceed placeholder changes the protected boundary".to_owned(),
            ));
        }
        let mut sources = Vec::with_capacity(call.input_bindings.len());
        for (call_input, protected_input) in call
            .input_bindings
            .iter()
            .zip(&boundary.fragment().input_bindings)
        {
            let source = bindings.resolve(&call_input.caller_slot)?;
            if source.contract_ref != protected_input.child_contract_ref
                || call_input.child_contract_ref != protected_input.child_contract_ref
            {
                return Err(CertifyError::Certification(
                    "policy proceed inputs are not the exact ordered boundary bijection".to_owned(),
                ));
            }
            sources.push(source);
        }
        let mut relocated = relocate_protected_boundary(boundary, occurrence_path, sources)?;
        relocated.fragment.label = call.label.clone();
        let failure_post = active.failure_post.clone();
        let policy_ref = active.policy_ref.clone();
        self.active_policy_proceed = Some(active);
        if let Some(failure_post) = failure_post {
            self.attach_policy_failure_post(&mut relocated.fragment, &policy_ref, &failure_post)?;
        }
        Ok(relocated.fragment)
    }

    fn attach_policy_failure_post(
        &mut self,
        protected: &mut ExpandedFragment,
        _policy_ref: &ContentRef,
        program: &AuthoredStructuredProgram,
    ) -> Result<()> {
        let CertifiedFailureBoundary::Typed {
            failure_contract,
            source_slot,
            plan,
        } = &protected.failure_boundary
        else {
            return Err(CertifyError::Certification(
                "a failure-post recipe cannot wrap a Never boundary".to_owned(),
            ));
        };
        let FailurePlan::Propagate {
            plan_id,
            plan_path,
            before_boundary,
            ..
        } = plan.as_ref()
        else {
            return Err(CertifyError::Certification(
                "a failure-post recipe does not own the protected propagation plan".to_owned(),
            ));
        };
        let mapping_count = Self::failure_post_mapping_suffix(program)?;
        if !before_boundary.declarations.is_empty()
            || program.input_roots.len() != 1
            || program.input_roots[0].contract_ref != source_slot.contract_ref
            || program.output_contract_ref != source_slot.contract_ref
            || !matches!(&program.failure_contract, StructuredFailureContract::Never)
                && &program.failure_contract != failure_contract.as_ref()
            || !matches!(
                &program.input_roots[0].producer,
                LexicalProducer::AdmissionRoot { .. }
            )
        {
            return Err(CertifyError::Certification(
                "failure-post recipe changes the exact protected failure boundary".to_owned(),
            ));
        }
        let protected_source = source_slot.clone();
        let plan_path = plan_path.clone();
        let plan_id = plan_id.clone();
        let label = stable_id("failure-post")?;
        let mut root_bindings = SlotBindings::default();
        root_bindings.insert(&program.input_roots[0], protected_source.clone())?;
        let local_path = SemanticCallPath::new(vec![SemanticPathSegment {
            label: label.clone(),
            discriminator: Some(label.clone()),
        }])?;
        let mut body = self.expand_block(
            &program.root,
            root_bindings,
            Vec::new(),
            plan_path.clone(),
            local_path.segments().to_vec(),
        )?;
        if body.declarations.len() < mapping_count {
            return Err(CertifyError::Certification(
                "failure-post mapping suffix exceeds its expanded declarations".to_owned(),
            ));
        }
        let mapper_declarations = body
            .declarations
            .split_off(body.declarations.len() - mapping_count);
        let mut mapped_target = protected_source.clone();
        let mut mapping_chain = Vec::with_capacity(mapping_count);
        for declaration in mapper_declarations {
            let ExpandedDeclaration::State(mapper) = declaration else {
                return Err(CertifyError::Certification(
                    "failure-post mapping suffix contains non-state structure".to_owned(),
                ));
            };
            if mapper.inputs.as_slice() != [mapped_target.clone()]
                || !matches!(
                    mapper.contract.execution,
                    StructuredStateExecutionContract::Pure
                )
                || !matches!(
                    mapper.contract.failure_contract,
                    StructuredFailureContract::Never
                )
                || !matches!(
                    mapper.failure_boundary,
                    CertifiedFailureBoundary::NoFailure(_)
                )
            {
                return Err(CertifyError::Certification(
                    "failure-post mapper is not one exact affine Pure + Never state".to_owned(),
                ));
            }
            mapped_target = mapper.output_slot.clone();
            mapping_chain.push(FailureMappingLink {
                plan_id: plan_id.clone(),
                input_slot: mapper.inputs[0].clone(),
                output_slot: mapper.output_slot.clone(),
                mapper,
            });
        }
        if !matches!(&body.tail, BlockTail::Normal(slot) if slot == &mapped_target)
            || mapped_target.contract_ref != protected_source.contract_ref
        {
            return Err(CertifyError::Certification(
                "failure-post mapped tail differs from the protected failure contract".to_owned(),
            ));
        }
        let CertifiedFailureBoundary::Typed { plan, .. } = &mut protected.failure_boundary else {
            return Err(CertifyError::Certification(
                "failure-post lost its protected typed boundary".to_owned(),
            ));
        };
        let FailurePlan::Propagate {
            before_boundary,
            mapping_chain: protected_mapping_chain,
            boundary_id,
            boundary_slot,
            ..
        } = plan.as_mut()
        else {
            return Err(CertifyError::Certification(
                "failure-post lost its protected propagation plan".to_owned(),
            ));
        };
        if !protected_mapping_chain.is_empty() {
            return Err(CertifyError::Certification(
                "more than one failure-post recipe targeted one propagation plan".to_owned(),
            ));
        }
        let LexicalProducer::FragmentBoundary {
            boundary_id: producer_boundary_id,
            role: ResultRole::TypedFailure,
            source: boundary_source,
        } = &mut boundary_slot.producer
        else {
            return Err(CertifyError::Certification(
                "protected propagation target is not a typed fragment boundary".to_owned(),
            ));
        };
        if producer_boundary_id != boundary_id {
            return Err(CertifyError::Certification(
                "protected propagation target changed fragment identity".to_owned(),
            ));
        }
        **boundary_source = mapped_target;
        **before_boundary = body;
        *protected_mapping_chain = mapping_chain;
        Ok(())
    }

    fn apply_policies(
        &mut self,
        state: &AuthoredStateCall,
        mut boundary: ExpandedBoundary,
    ) -> Result<ExpandedBoundary> {
        let eligible: Vec<(u32, ContentRef, ContentRef)> = self
            .profile
            .policies
            .iter()
            .enumerate()
            .filter_map(|(ordinal, policy)| {
                policy
                    .recipe_for(&state.contract.state_contract_ref)
                    .map(|recipe_ref| {
                        (
                            ordinal as u32,
                            policy.policy_ref.clone(),
                            recipe_ref.clone(),
                        )
                    })
            })
            .collect();
        for (_, policy_ref, recipe_ref) in eligible.iter().rev() {
            let recipe = self
                .registry
                .policy_recipes
                .get(recipe_ref)
                .cloned()
                .ok_or_else(|| {
                    CertifyError::Planning(
                        "qualified declarative policy recipe is absent".to_owned(),
                    )
                })?;
            boundary = self.expand_policy_recipe(state, boundary, policy_ref, &recipe)?;
            validate_external_boundary(state, boundary.fragment())?;
        }
        for (ordinal, policy_ref, _) in eligible {
            self.coverage.push(PolicyCoverageEntry {
                semantic_call_id: state.semantic_call_id.clone(),
                profile_ordinal: ordinal,
                policy_ref,
            });
        }
        Ok(boundary)
    }

    fn failure_post_mapping_suffix(program: &AuthoredStructuredProgram) -> Result<usize> {
        let source = program.input_roots.first().ok_or_else(|| {
            CertifyError::Certification("failure-post recipe has no protected input".to_owned())
        })?;
        let mut current: &LexicalSlot = match &program.root.tail {
            BlockTail::Normal(slot) => slot,
            BlockTail::ScopeFailure(_) => {
                return Err(CertifyError::Certification(
                    "failure-post recipe must end on its normal protected-failure channel"
                        .to_owned(),
                ));
            }
        };
        if current == source {
            return Ok(0);
        }

        let mut next_index = program.root.declarations.len();
        let mut mapping_count = 0usize;
        loop {
            next_index = next_index.checked_sub(1).ok_or_else(|| {
                CertifyError::Certification(
                    "failure-post mapped tail is not a contiguous state suffix".to_owned(),
                )
            })?;
            let AuthoredDeclaration::State(mapper) = &program.root.declarations[next_index] else {
                return Err(CertifyError::Certification(
                    "failure-post mapper suffix contains nested or non-state structure".to_owned(),
                ));
            };
            if &mapper.output_slot != current || mapper.inputs.len() != 1 {
                return Err(CertifyError::Certification(
                    "failure-post mapper suffix is not directly adjacent".to_owned(),
                ));
            }
            validate_handler_contract(&mapper.contract, &mapper.inputs[0].contract_ref)?;
            if matches!(
                &program.root.failure_scope,
                FailureScopeBinding::Owns { scope }
                    if scope.default_mappers.iter().any(|registration| {
                        registration.mapper_state_contract_ref
                            == mapper.contract.state_contract_ref
                    })
            ) {
                return Err(CertifyError::Certification(
                    "a lexical default handler cannot become a propagation mapper".to_owned(),
                ));
            }
            if !matches!(
                mapper.failure_directive,
                AuthoredFailureDirective::NoFailure
            ) {
                return Err(CertifyError::Certification(
                    "failure-post mapper is not explicitly Pure + Never".to_owned(),
                ));
            }
            mapping_count += 1;
            current = &mapper.inputs[0];
            if current == source {
                return Ok(mapping_count);
            }
        }
    }

    fn record_external_trace(
        &mut self,
        state: &AuthoredStateCall,
        fragment: &ExpandedFragment,
        capability_ref: Option<&ContentRef>,
    ) -> Result<()> {
        if let Some(capability_ref) = capability_ref {
            self.trace.push(ExpansionTraceEntry {
                stage: ExpansionStage::CapabilityLowering,
                semantic_call_id: state.semantic_call_id.clone(),
                expansion_ref: capability_ref.clone(),
                boundary_id: ExpansionBoundaryId::Fragment(find_expansion_boundary(
                    fragment,
                    &state.semantic_call_id,
                    capability_ref,
                )?),
            });
        }
        let eligible: Vec<&ExpansionPolicyContract> = self
            .profile
            .policies
            .iter()
            .filter(|policy| {
                policy
                    .recipe_for(&state.contract.state_contract_ref)
                    .is_some()
            })
            .collect();
        for policy in eligible {
            self.trace.push(ExpansionTraceEntry {
                stage: ExpansionStage::PolicyWrapping,
                semantic_call_id: state.semantic_call_id.clone(),
                expansion_ref: policy.policy_ref.clone(),
                boundary_id: ExpansionBoundaryId::Fragment(find_expansion_boundary(
                    fragment,
                    &state.semantic_call_id,
                    &policy.policy_ref,
                )?),
            });
        }
        Ok(())
    }

    fn record_failure_completion(
        &mut self,
        state: &AuthoredStateCall,
        boundary_id: &FragmentBoundaryId,
    ) -> Result<()> {
        if matches!(
            state.contract.failure_contract,
            StructuredFailureContract::Typed { .. }
        ) {
            self.trace.push(ExpansionTraceEntry {
                stage: ExpansionStage::FailureCompletion,
                semantic_call_id: state.semantic_call_id.clone(),
                expansion_ref: state.contract.failure_contract.contract_ref()?,
                boundary_id: ExpansionBoundaryId::Fragment(boundary_id.clone()),
            });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_child(
        &mut self,
        call: &AuthoredOperationCall,
        authored_origin: &AuthoredCallOrigin,
        occurrence_path: &StructuralPath,
        caller_bindings: &SlotBindings,
        visible_mappers: &[FailureMapperRegistration],
        scope_id: &StableId,
        scope_failure: &StructuredFailureContract,
    ) -> Result<ExpandedFragment> {
        if self.child_stack.contains(&call.child_program_ref) {
            return Err(CertifyError::Planning(
                "child-operation substitution cycle".to_owned(),
            ));
        }
        let child = self
            .registry
            .children
            .get(&call.child_program_ref)
            .ok_or_else(|| CertifyError::Planning("child program is not registered".to_owned()))?
            .clone();
        validate_authored_program(&child, self.registry)?;
        if child.output_contract_ref != call.output_contract_ref
            || child.failure_contract != call.failure_contract
            || child.input_roots.len() != call.input_bindings.len()
        {
            return Err(CertifyError::Planning(
                "child call boundary differs from its registered program".to_owned(),
            ));
        }

        let fragment_path = occurrence_path.child(StructuralPathSegment::Fragment {
            label: call.label.clone(),
            expansion_ref: call.child_program_ref.clone(),
        })?;
        let boundary_id = fragment_path
            .fragment_boundary_id()
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        let mut child_bindings = SlotBindings::default();
        let mut resolved_input_bindings = Vec::with_capacity(call.input_bindings.len());
        for (child_root, input) in child.input_roots.iter().zip(call.input_bindings.iter()) {
            if child_root.contract_ref != input.child_contract_ref
                || !matches!(
                    &child_root.producer,
                    LexicalProducer::AdmissionRoot { root_id }
                        if root_id == &input.child_root_id
                )
            {
                return Err(CertifyError::Planning(
                    "child input-root substitution is not exact".to_owned(),
                ));
            }
            let source = caller_bindings.resolve(&input.caller_slot)?;
            if source.contract_ref != input.child_contract_ref {
                return Err(CertifyError::Planning(
                    "child caller source contract differs".to_owned(),
                ));
            }
            let rebound = LexicalSlot {
                lexical_path: fragment_path.clone(),
                contract_ref: input.child_contract_ref.clone(),
                producer: LexicalProducer::FragmentInput {
                    boundary_id: boundary_id.clone(),
                    child_root_id: input.child_root_id.clone(),
                    source: Box::new(source.clone()),
                },
            };
            child_bindings.insert(child_root, rebound)?;
            resolved_input_bindings.push(mfm_spec::structured::FragmentInputBinding {
                child_root_id: input.child_root_id.clone(),
                child_contract_ref: input.child_contract_ref.clone(),
                caller_slot: source,
            });
        }
        self.trace.push(ExpansionTraceEntry {
            stage: ExpansionStage::ChildSubstitution,
            semantic_call_id: call.semantic_call_id.clone(),
            expansion_ref: call.child_program_ref.clone(),
            boundary_id: ExpansionBoundaryId::Fragment(boundary_id.clone()),
        });
        self.child_stack.push(call.child_program_ref.clone());
        let body = self.expand_block(
            &child.root,
            child_bindings,
            Vec::new(),
            fragment_path.clone(),
            call.semantic_path.segments().to_vec(),
        )?;
        self.child_stack.pop();

        let body_success = match &body.tail {
            BlockTail::Normal(slot) => slot.clone(),
            BlockTail::ScopeFailure(_) => {
                return Err(CertifyError::Planning(
                    "child root has no normal boundary source".to_owned(),
                ));
            }
        };
        if body_success.contract_ref != call.output_contract_ref {
            return Err(CertifyError::Planning(
                "child normal tail contract differs from call boundary".to_owned(),
            ));
        }
        let success_slot = LexicalSlot {
            lexical_path: fragment_path.clone(),
            contract_ref: call.output_contract_ref.clone(),
            producer: LexicalProducer::FragmentBoundary {
                boundary_id: boundary_id.clone(),
                role: ResultRole::SuccessOutput,
                source: Box::new(body_success),
            },
        };
        let failure_boundary = build_fragment_failure_boundary(
            self,
            call,
            authored_origin,
            &boundary_id,
            &fragment_path,
            &body,
            caller_bindings,
            visible_mappers,
            scope_id,
            scope_failure,
        )?;
        if matches!(
            call.failure_contract,
            StructuredFailureContract::Typed { .. }
        ) {
            self.trace.push(ExpansionTraceEntry {
                stage: ExpansionStage::FailureCompletion,
                semantic_call_id: call.semantic_call_id.clone(),
                expansion_ref: call.failure_contract.contract_ref()?,
                boundary_id: ExpansionBoundaryId::Fragment(boundary_id.clone()),
            });
        }
        Ok(ExpandedFragment {
            semantic_call_id: call.semantic_call_id.clone(),
            label: call.label.clone(),
            path: fragment_path,
            boundary_id,
            input_bindings: resolved_input_bindings,
            body,
            success_slot,
            failure_boundary,
        })
    }

    fn expand_match(
        &mut self,
        binding: &AuthoredMatch,
        path: StructuralPath,
        semantic_prefix: Vec<mfm_spec::structured::SemanticPathSegment>,
        outer_bindings: &SlotBindings,
        visible_mappers: &[FailureMapperRegistration],
    ) -> Result<ExpandedMatch> {
        let selector = outer_bindings.resolve(&binding.selector)?;
        if selector.contract_ref != binding.selector_contract.selector_contract_ref {
            return Err(CertifyError::Planning(
                "Match selector contract substitution".to_owned(),
            ));
        }
        validate_registered_closed_sum(&binding.selector_contract, self.registry)?;
        let expected: Vec<&str> = binding
            .selector_contract
            .variants
            .iter()
            .map(|variant| variant.canonical_tag.as_str())
            .collect();
        let actual: Vec<&str> = binding
            .arms
            .iter()
            .map(|arm| arm.canonical_tag.as_str())
            .collect();
        if expected != actual {
            return Err(CertifyError::Planning(
                "Match arms are not exhaustive in canonical tag order".to_owned(),
            ));
        }
        let mut arms = Vec::with_capacity(binding.arms.len());
        let mut arm_slots = Vec::new();
        for (arm, variant) in binding.arms.iter().zip(&binding.selector_contract.variants) {
            let arm_path = path.child(StructuralPathSegment::MatchArm {
                label: arm.label.clone(),
                tag: arm.canonical_tag.clone(),
            })?;
            let mut arm_semantic = semantic_prefix.clone();
            arm_semantic.push(mfm_spec::structured::SemanticPathSegment {
                label: arm.label.clone(),
                discriminator: Some(arm.label.clone()),
            });
            let mut arm_bindings = outer_bindings.clone();
            bind_variant_payloads(
                &mut arm_bindings,
                &binding.selector,
                &selector,
                &arm.body.path,
                &arm_path,
                &arm.canonical_tag,
                variant,
            )?;
            let body = self.expand_block(
                &arm.body,
                arm_bindings,
                visible_mappers.to_vec(),
                arm_path.clone(),
                arm_semantic,
            )?;
            if let BlockTail::Normal(slot) = &body.tail {
                if slot.contract_ref != binding.output_slot.contract_ref {
                    return Err(CertifyError::Planning(
                        "Match arm result contract substitution".to_owned(),
                    ));
                }
                arm_slots.push(LexicalSlot {
                    lexical_path: arm_path.clone(),
                    contract_ref: slot.contract_ref.clone(),
                    producer: LexicalProducer::ArmValue {
                        selected_arm_path: arm_path.clone(),
                        source: Box::new(slot.clone()),
                    },
                });
            }
            arms.push(ExpandedMatchArm {
                canonical_tag: arm.canonical_tag.clone(),
                label: arm.label.clone(),
                path: arm_path,
                body,
            });
        }
        let output_slot = LexicalSlot {
            lexical_path: path.clone(),
            contract_ref: binding.output_slot.contract_ref.clone(),
            producer: LexicalProducer::MatchMerge {
                match_path: path.clone(),
                declaration_ordered_arm_slots: arm_slots,
            },
        };
        Ok(ExpandedMatch {
            label: binding.label.clone(),
            path,
            selector,
            selector_contract: binding.selector_contract.clone(),
            arms,
            output_slot,
        })
    }

    fn expand_fan_out(
        &mut self,
        group: &AuthoredFanOut,
        path: StructuralPath,
        semantic_prefix: Vec<mfm_spec::structured::SemanticPathSegment>,
        outer_bindings: &SlotBindings,
    ) -> Result<ExpandedFanOut> {
        if group.lanes.is_empty() {
            return Err(CertifyError::Planning(
                "FanOut must be non-empty".to_owned(),
            ));
        }
        let expected_join_contract_ref = fan_out_join_contract_ref(
            &group.lane_output_contract_ref,
            &group.lane_failure_contract,
        )?;
        if group.output_slot.contract_ref != expected_join_contract_ref {
            return Err(CertifyError::Certification(
                "fan-out join contract is not kernel-derived from its lane algebra".to_owned(),
            ));
        }
        let mut lanes = Vec::with_capacity(group.lanes.len());
        let mut lane_slots = Vec::with_capacity(group.lanes.len());
        for (ordinal, lane) in group.lanes.iter().enumerate() {
            if usize::try_from(lane.declaration_ordinal).ok() != Some(ordinal) {
                return Err(CertifyError::Planning(
                    "fan-out lane ordinal is not dense declaration order".to_owned(),
                ));
            }
            let lane_path = path.child(StructuralPathSegment::FanOutLane {
                key: lane.key.clone(),
                ordinal: lane.declaration_ordinal,
            })?;
            let mut lane_semantic = semantic_prefix.clone();
            lane_semantic.push(mfm_spec::structured::SemanticPathSegment {
                label: lane.key.clone(),
                discriminator: Some(lane.key.clone()),
            });
            let body = self.expand_block(
                &lane.body,
                outer_bindings.clone(),
                Vec::new(),
                lane_path.clone(),
                lane_semantic,
            )?;
            reject_effects(&body)?;
            let success_slot = match &body.tail {
                BlockTail::Normal(slot) => {
                    if slot.contract_ref != group.lane_output_contract_ref {
                        return Err(CertifyError::Planning(
                            "fan-out lane output contract substitution".to_owned(),
                        ));
                    }
                    Some(Box::new(slot.clone()))
                }
                BlockTail::ScopeFailure(slot) => {
                    if slot.contract_ref != group.lane_failure_contract.contract_ref()? {
                        return Err(CertifyError::Planning(
                            "fan-out lane failure contract substitution".to_owned(),
                        ));
                    }
                    None
                }
            };
            let failure_slot = match &group.lane_failure_contract {
                StructuredFailureContract::Never => {
                    if !body.failure_exits.is_empty() {
                        return Err(CertifyError::Certification(
                            "Never fan-out lane exposes a typed failure exit".to_owned(),
                        ));
                    }
                    None
                }
                StructuredFailureContract::Typed { contract_ref, .. } => {
                    if body
                        .failure_exits
                        .iter()
                        .any(|slot| &slot.contract_ref != contract_ref)
                    {
                        return Err(CertifyError::Certification(
                            "fan-out lane failure exit contract substitution".to_owned(),
                        ));
                    }
                    (!body.failure_exits.is_empty()).then(|| {
                        Box::new(LexicalSlot {
                            lexical_path: body.path.clone(),
                            contract_ref: contract_ref.clone(),
                            producer: LexicalProducer::ScopeFailureMerge {
                                scope_id: body.failure_scope.scope_id().clone(),
                                declaration_ordered_failure_slots: body.failure_exits.clone(),
                            },
                        })
                    })
                }
            };
            let outcome_slot = LexicalSlot {
                lexical_path: body.path.clone(),
                contract_ref: lane_outcome_contract_ref(
                    &group.lane_output_contract_ref,
                    &group.lane_failure_contract,
                )?,
                producer: LexicalProducer::LaneOutcome {
                    lane_path: body.path.clone(),
                    success_slot,
                    failure_slot,
                },
            };
            lane_slots.push(outcome_slot.clone());
            lanes.push(ExpandedFanOutLane {
                key: lane.key.clone(),
                declaration_ordinal: lane.declaration_ordinal,
                path: lane_path,
                body,
                outcome_slot,
            });
        }
        let output_slot = LexicalSlot {
            lexical_path: path.clone(),
            contract_ref: expected_join_contract_ref,
            producer: LexicalProducer::FanOutJoin {
                group_path: path.clone(),
                declaration_ordered_lane_slots: lane_slots,
            },
        };
        Ok(ExpandedFanOut {
            label: group.label.clone(),
            path,
            lane_output_contract_ref: group.lane_output_contract_ref.clone(),
            lane_failure_contract: group.lane_failure_contract.clone(),
            lanes,
            output_slot,
        })
    }
}

struct ExpandedStateResult {
    declaration: ExpandedDeclaration,
    output_slot: LexicalSlot,
    failure_exits: Vec<LexicalSlot>,
}

impl ExpandedStateResult {
    fn output_slot(&self) -> &LexicalSlot {
        &self.output_slot
    }
}

#[derive(Clone, Default)]
struct SlotBindings(BTreeMap<LexicalSlot, LexicalSlot>);

impl SlotBindings {
    fn insert(&mut self, authored: &LexicalSlot, expanded: LexicalSlot) -> Result<()> {
        if self.0.insert(authored.clone(), expanded).is_some() {
            return Err(CertifyError::Planning(
                "duplicate lexical slot binding".to_owned(),
            ));
        }
        Ok(())
    }

    fn resolve(&self, slot: &LexicalSlot) -> Result<LexicalSlot> {
        if let Some(resolved) = self.0.get(slot) {
            return Ok(resolved.clone());
        }
        if let LexicalProducer::VariantPayload {
            selector,
            canonical_tag,
            payload_path,
        } = &slot.producer
        {
            return Ok(LexicalSlot {
                lexical_path: slot.lexical_path.clone(),
                contract_ref: slot.contract_ref.clone(),
                producer: LexicalProducer::VariantPayload {
                    selector: Box::new(self.resolve(selector)?),
                    canonical_tag: canonical_tag.clone(),
                    payload_path: payload_path.clone(),
                },
            });
        }
        self.0
            .get(slot)
            .cloned()
            .ok_or_else(|| CertifyError::Planning("non-dominating lexical slot".to_owned()))
    }
}

#[allow(clippy::too_many_arguments)]
fn bind_variant_payloads(
    bindings: &mut SlotBindings,
    authored_selector: &LexicalSlot,
    expanded_selector: &LexicalSlot,
    authored_arm_path: &StructuralPath,
    expanded_arm_path: &StructuralPath,
    canonical_tag: &str,
    variant: &mfm_spec::structured::ClosedSumVariant,
) -> Result<()> {
    for payload in &variant.payloads {
        let authored = LexicalSlot {
            lexical_path: authored_arm_path.clone(),
            contract_ref: payload.contract_ref.clone(),
            producer: LexicalProducer::VariantPayload {
                selector: Box::new(authored_selector.clone()),
                canonical_tag: canonical_tag.to_owned(),
                payload_path: payload.payload_path.clone(),
            },
        };
        let expanded = LexicalSlot {
            lexical_path: expanded_arm_path.clone(),
            contract_ref: payload.contract_ref.clone(),
            producer: LexicalProducer::VariantPayload {
                selector: Box::new(expanded_selector.clone()),
                canonical_tag: canonical_tag.to_owned(),
                payload_path: payload.payload_path.clone(),
            },
        };
        bindings.insert(&authored, expanded)?;
    }
    Ok(())
}

fn initial_bindings(input_roots: &[LexicalSlot]) -> Result<SlotBindings> {
    let mut bindings = SlotBindings::default();
    let mut roots = BTreeSet::new();
    for root in input_roots {
        let LexicalProducer::AdmissionRoot { root_id } = &root.producer else {
            return Err(CertifyError::Planning(
                "program input root is not admission authority".to_owned(),
            ));
        };
        if !roots.insert(root_id.clone()) {
            return Err(CertifyError::Planning(
                "duplicate admission input root".to_owned(),
            ));
        }
        bindings.insert(root, root.clone())?;
    }
    Ok(bindings)
}

fn declaration_path(
    block_path: &StructuralPath,
    label: &StableId,
    ordinal: u32,
) -> Result<StructuralPath> {
    block_path
        .child(StructuralPathSegment::Declaration {
            label: label.clone(),
            ordinal,
        })
        .map_err(|error| CertifyError::Planning(error.to_string()))
}

fn declaration_semantic_prefix(
    semantic_prefix: &[mfm_spec::structured::SemanticPathSegment],
    label: &StableId,
) -> Result<Vec<mfm_spec::structured::SemanticPathSegment>> {
    let mut semantic = semantic_prefix.to_vec();
    semantic.push(mfm_spec::structured::SemanticPathSegment {
        label: label.clone(),
        discriminator: None,
    });
    mfm_spec::structured::SemanticCallPath::new(semantic.clone())
        .map_err(|error| CertifyError::Planning(error.to_string()))?;
    Ok(semantic)
}

fn find_expansion_boundary(
    root: &ExpandedFragment,
    semantic_call_id: &SemanticCallId,
    expansion_ref: &ContentRef,
) -> Result<FragmentBoundaryId> {
    fn collect(
        fragment: &ExpandedFragment,
        semantic_call_id: &SemanticCallId,
        expansion_ref: &ContentRef,
        matches: &mut Vec<FragmentBoundaryId>,
    ) {
        if &fragment.semantic_call_id == semantic_call_id
            && matches!(
                fragment.path.segments().last(),
                Some(StructuralPathSegment::Fragment {
                    expansion_ref: actual,
                    ..
                }) if actual == expansion_ref
            )
        {
            matches.push(fragment.boundary_id.clone());
        }
        collect_expansion_boundaries_from_block(
            &fragment.body,
            semantic_call_id,
            expansion_ref,
            matches,
        );
    }

    fn collect_expansion_boundaries_from_block(
        block: &ExpandedBlock,
        semantic_call_id: &SemanticCallId,
        expansion_ref: &ContentRef,
        matches: &mut Vec<FragmentBoundaryId>,
    ) {
        for declaration in &block.declarations {
            match declaration {
                ExpandedDeclaration::State(_) => {}
                ExpandedDeclaration::Match(binding) => {
                    for arm in &binding.arms {
                        collect_expansion_boundaries_from_block(
                            &arm.body,
                            semantic_call_id,
                            expansion_ref,
                            matches,
                        );
                    }
                }
                ExpandedDeclaration::FanOut(group) => {
                    for lane in &group.lanes {
                        collect_expansion_boundaries_from_block(
                            &lane.body,
                            semantic_call_id,
                            expansion_ref,
                            matches,
                        );
                    }
                }
                ExpandedDeclaration::Fragment(fragment) => {
                    collect(fragment, semantic_call_id, expansion_ref, matches);
                }
            }
        }
    }

    let mut matches = Vec::new();
    collect(root, semantic_call_id, expansion_ref, &mut matches);
    match matches.as_slice() {
        [boundary_id] => Ok(boundary_id.clone()),
        _ => Err(CertifyError::Certification(
            "expansion proof does not identify one exact normalized boundary".to_owned(),
        )),
    }
}

fn instantiate_state_call(
    authored: &AuthoredStateCall,
    block_path: &StructuralPath,
    semantic_prefix: &[mfm_spec::structured::SemanticPathSegment],
    ordinal: u32,
    support: Option<&SupportExpansionIdentity>,
) -> Result<(AuthoredStateCall, StructuralPath)> {
    let occurrence_path = declaration_path(block_path, &authored.label, ordinal)?;
    let semantic_path = mfm_spec::structured::SemanticCallPath::new(declaration_semantic_prefix(
        semantic_prefix,
        &authored.label,
    )?)
    .map_err(|error| CertifyError::Planning(error.to_string()))?;
    let semantic_call_id = match support {
        Some(support) => expansion_support_semantic_call_id(
            &support.protected_semantic_call_id,
            &support.expansion_ref,
            &semantic_path,
        )?,
        None => semantic_path
            .identity()
            .map_err(|error| CertifyError::Planning(error.to_string()))?,
    };
    let mut call = authored.clone();
    call.semantic_path = semantic_path;
    call.semantic_call_id = semantic_call_id.clone();
    call.output_slot = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: call.contract.output_contract_ref.clone(),
        producer: LexicalProducer::AuthoredCallOutput {
            semantic_call_id,
            role: ResultRole::SuccessOutput,
        },
    };
    Ok((call, occurrence_path))
}

fn instantiate_operation_call(
    authored: &AuthoredOperationCall,
    block_path: &StructuralPath,
    semantic_prefix: &[mfm_spec::structured::SemanticPathSegment],
    ordinal: u32,
    support: Option<&SupportExpansionIdentity>,
) -> Result<(AuthoredOperationCall, StructuralPath)> {
    let occurrence_path = declaration_path(block_path, &authored.label, ordinal)?;
    let semantic_path = mfm_spec::structured::SemanticCallPath::new(declaration_semantic_prefix(
        semantic_prefix,
        &authored.label,
    )?)
    .map_err(|error| CertifyError::Planning(error.to_string()))?;
    let semantic_call_id = match support {
        Some(support) => expansion_support_semantic_call_id(
            &support.protected_semantic_call_id,
            &support.expansion_ref,
            &semantic_path,
        )?,
        None => semantic_path
            .identity()
            .map_err(|error| CertifyError::Planning(error.to_string()))?,
    };
    let mut call = authored.clone();
    call.semantic_path = semantic_path;
    call.semantic_call_id = semantic_call_id.clone();
    call.output_slot = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: call.output_contract_ref.clone(),
        producer: LexicalProducer::AuthoredCallOutput {
            semantic_call_id,
            role: ResultRole::SuccessOutput,
        },
    };
    Ok((call, occurrence_path))
}

fn declaration_output_slot(declaration: &AuthoredDeclaration) -> &LexicalSlot {
    match declaration {
        AuthoredDeclaration::State(state) => &state.output_slot,
        AuthoredDeclaration::OperationCall(call) => &call.output_slot,
        AuthoredDeclaration::Match(binding) => &binding.output_slot,
        AuthoredDeclaration::FanOut(group) => &group.output_slot,
    }
}

fn resolve_tail(tail: &BlockTail, bindings: &SlotBindings) -> Result<BlockTail> {
    match tail {
        BlockTail::Normal(slot) => Ok(BlockTail::Normal(bindings.resolve(slot)?)),
        BlockTail::ScopeFailure(slot) => Ok(BlockTail::ScopeFailure(bindings.resolve(slot)?)),
    }
}

fn failure_boundary_scope_exits(boundary: &CertifiedFailureBoundary) -> Vec<LexicalSlot> {
    let CertifiedFailureBoundary::Typed { plan, .. } = boundary else {
        return Vec::new();
    };
    match plan.as_ref() {
        FailurePlan::Handled {
            before_handler,
            continuation,
            ..
        } => {
            let mut exits = before_handler.failure_exits.clone();
            match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation { failure_tail, .. } => {
                    exits.push(failure_tail.as_ref().clone());
                }
                HandlerContinuation::CustomRecovery { arms, .. } => {
                    exits.extend(
                        arms.iter()
                            .flat_map(|arm| arm.body.failure_exits.iter().cloned()),
                    );
                }
            }
            exits
        }
        FailurePlan::Propagate {
            before_boundary,
            boundary_slot,
            ..
        } => {
            let mut exits = before_boundary.failure_exits.clone();
            exits.push(boundary_slot.as_ref().clone());
            exits
        }
    }
}

fn fragment_failure_scope_exits(boundary: &CertifiedFailureBoundary) -> Vec<LexicalSlot> {
    match boundary {
        CertifiedFailureBoundary::Typed {
            source_slot, plan, ..
        } if matches!(plan.as_ref(), FailurePlan::Propagate { .. }) => {
            let mut exits = match plan.as_ref() {
                FailurePlan::Propagate {
                    before_boundary, ..
                } => before_boundary.failure_exits.clone(),
                FailurePlan::Handled { .. } => Vec::new(),
            };
            let outward = match plan.as_ref() {
                FailurePlan::Propagate {
                    mapping_chain,
                    boundary_slot,
                    ..
                } if !mapping_chain.is_empty() => {
                    local_mapped_fragment_failure_slot(source_slot, mapping_chain)
                        .unwrap_or_else(|| boundary_slot.as_ref().clone())
                }
                FailurePlan::Propagate { .. } | FailurePlan::Handled { .. } => source_slot.clone(),
            };
            exits.push(outward);
            exits
        }
        _ => failure_boundary_scope_exits(boundary),
    }
}

fn pre_handler_block(
    plan_path: &StructuralPath,
    scope_id: &StableId,
    scope_failure: &StructuredFailureContract,
    declarations: Vec<ExpandedDeclaration>,
    protected_source: LexicalSlot,
) -> Result<ExpandedBlock> {
    let mut failure_exits = Vec::new();
    for declaration in &declarations {
        collect_declaration_failure_exits(declaration, &mut failure_exits);
    }
    dedup_slots(&mut failure_exits)?;
    Ok(ExpandedBlock {
        path: plan_path.clone(),
        failure_scope: FailureScopeBinding::Inherits {
            scope_id: scope_id.clone(),
            failure_contract: scope_failure.clone(),
        },
        declarations,
        failure_exits,
        tail: BlockTail::Normal(protected_source),
    })
}

fn collect_declaration_failure_exits(
    declaration: &ExpandedDeclaration,
    exits: &mut Vec<LexicalSlot>,
) {
    match declaration {
        ExpandedDeclaration::State(state) => {
            exits.extend(failure_boundary_scope_exits(&state.failure_boundary));
        }
        ExpandedDeclaration::Match(binding) => {
            for arm in &binding.arms {
                exits.extend(arm.body.failure_exits.iter().cloned());
            }
        }
        ExpandedDeclaration::FanOut(_) => {}
        ExpandedDeclaration::Fragment(fragment) => {
            exits.extend(fragment_failure_scope_exits(&fragment.failure_boundary));
        }
    }
}

fn normal_tail_slot(block: &ExpandedBlock) -> Result<LexicalSlot> {
    match &block.tail {
        BlockTail::Normal(slot) => Ok(slot.clone()),
        BlockTail::ScopeFailure(_) => Err(CertifyError::Certification(
            "pre-handler block has a direct scope-failure tail".to_owned(),
        )),
    }
}

fn dedup_slots(slots: &mut Vec<LexicalSlot>) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut retained = Vec::with_capacity(slots.len());
    for slot in slots.drain(..) {
        if seen.insert(slot.clone()) {
            retained.push(slot);
        }
    }
    *slots = retained;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_failure_boundary(
    context: &mut ExpansionContext<'_>,
    state: &AuthoredStateCall,
    authored_origin: &AuthoredCallOrigin,
    occurrence_id: &OccurrenceId,
    occurrence_path: &StructuralPath,
    bindings: &SlotBindings,
    visible_mappers: &[FailureMapperRegistration],
    scope_id: &StableId,
    scope_failure: &StructuredFailureContract,
) -> Result<CertifiedFailureBoundary> {
    match (&state.contract.failure_contract, &state.failure_directive) {
        (StructuredFailureContract::Never, AuthoredFailureDirective::NoFailure) => {
            Ok(CertifiedFailureBoundary::NoFailure(NoFailureBoundary {
                never_contract_ref: never_failure_contract_ref()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?,
            }))
        }
        (StructuredFailureContract::Never, _) => Err(CertifyError::Certification(
            "Never state cannot own a failure plan".to_owned(),
        )),
        (StructuredFailureContract::Typed { contract_ref, .. }, directive) => {
            let source_slot = LexicalSlot {
                lexical_path: occurrence_path.clone(),
                contract_ref: contract_ref.clone(),
                producer: LexicalProducer::StateOutput {
                    occurrence_id: occurrence_id.clone(),
                    role: ResultRole::TypedFailure,
                },
            };
            let plan_path = occurrence_path.child(StructuralPathSegment::FailurePlan {
                label: stable_id("handler")?,
            })?;
            let identity = FailurePlanIdentity {
                source_semantic_call_id: state.semantic_call_id.clone(),
                source_slot: source_slot.clone(),
                plan_path: plan_path.clone(),
            };
            let plan_id = identity
                .derive()
                .map_err(|error| CertifyError::Certification(error.to_string()))?;
            let before_handler = pre_handler_block(
                &plan_path,
                scope_id,
                scope_failure,
                Vec::new(),
                source_slot.clone(),
            )?;
            let plan = match directive {
                AuthoredFailureDirective::NoFailure => {
                    return Err(CertifyError::Certification(
                        "typed state has no failure plan".to_owned(),
                    ));
                }
                AuthoredFailureDirective::Default => build_default_plan(
                    &state.semantic_call_id,
                    plan_id,
                    source_slot.clone(),
                    &plan_path,
                    before_handler.clone(),
                    visible_mappers,
                    scope_failure,
                    context.registry,
                )?,
                AuthoredFailureDirective::Custom {
                    handler_state_contract_ref,
                    route_contract,
                    arms,
                } => build_custom_plan(
                    context,
                    &state.semantic_call_id,
                    &state.semantic_path,
                    authored_origin,
                    plan_id,
                    source_slot.clone(),
                    &plan_path,
                    before_handler,
                    &state.contract.output_contract_ref,
                    handler_state_contract_ref,
                    route_contract,
                    arms,
                    bindings,
                    visible_mappers,
                    scope_failure,
                )?,
            };
            Ok(CertifiedFailureBoundary::Typed {
                failure_contract: Box::new(state.contract.failure_contract.clone()),
                source_slot,
                plan: Box::new(plan),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_fragment_failure_boundary(
    context: &mut ExpansionContext<'_>,
    call: &AuthoredOperationCall,
    authored_origin: &AuthoredCallOrigin,
    boundary_id: &FragmentBoundaryId,
    fragment_path: &StructuralPath,
    body: &ExpandedBlock,
    bindings: &SlotBindings,
    visible_mappers: &[FailureMapperRegistration],
    scope_id: &StableId,
    scope_failure: &StructuredFailureContract,
) -> Result<CertifiedFailureBoundary> {
    match (&call.failure_contract, &call.failure_directive) {
        (StructuredFailureContract::Never, AuthoredFailureDirective::NoFailure) => {
            if !body.failure_exits.is_empty() {
                return Err(CertifyError::Certification(
                    "Never child boundary exposes a typed failure exit".to_owned(),
                ));
            }
            Ok(CertifiedFailureBoundary::NoFailure(NoFailureBoundary {
                never_contract_ref: never_failure_contract_ref()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?,
            }))
        }
        (StructuredFailureContract::Never, _) => Err(CertifyError::Certification(
            "Never child boundary cannot own a failure plan".to_owned(),
        )),
        (StructuredFailureContract::Typed { contract_ref, .. }, directive) => {
            if body.failure_exits.is_empty()
                || body
                    .failure_exits
                    .iter()
                    .any(|slot| &slot.contract_ref != contract_ref)
            {
                return Err(CertifyError::Certification(
                    "typed child boundary has no exact failure-tail source".to_owned(),
                ));
            }
            let body_failure = LexicalSlot {
                lexical_path: body.path.clone(),
                contract_ref: contract_ref.clone(),
                producer: LexicalProducer::ScopeFailureMerge {
                    scope_id: body.failure_scope.scope_id().clone(),
                    declaration_ordered_failure_slots: body.failure_exits.clone(),
                },
            };
            let source_slot = LexicalSlot {
                lexical_path: fragment_path.clone(),
                contract_ref: contract_ref.clone(),
                producer: LexicalProducer::FragmentBoundary {
                    boundary_id: boundary_id.clone(),
                    role: ResultRole::TypedFailure,
                    source: Box::new(body_failure),
                },
            };
            let plan_path = fragment_path.child(StructuralPathSegment::FailurePlan {
                label: stable_id("handler")?,
            })?;
            let identity = FailurePlanIdentity {
                source_semantic_call_id: call.semantic_call_id.clone(),
                source_slot: source_slot.clone(),
                plan_path: plan_path.clone(),
            };
            let plan_id = identity
                .derive()
                .map_err(|error| CertifyError::Certification(error.to_string()))?;
            let before_handler = pre_handler_block(
                &plan_path,
                scope_id,
                scope_failure,
                Vec::new(),
                source_slot.clone(),
            )?;
            let plan = match directive {
                AuthoredFailureDirective::NoFailure => {
                    return Err(CertifyError::Certification(
                        "typed child boundary has no failure plan".to_owned(),
                    ));
                }
                AuthoredFailureDirective::Default => build_default_plan(
                    &call.semantic_call_id,
                    plan_id,
                    source_slot.clone(),
                    &plan_path,
                    before_handler.clone(),
                    visible_mappers,
                    scope_failure,
                    context.registry,
                )?,
                AuthoredFailureDirective::Custom {
                    handler_state_contract_ref,
                    route_contract,
                    arms,
                } => build_custom_plan(
                    context,
                    &call.semantic_call_id,
                    &call.semantic_path,
                    authored_origin,
                    plan_id,
                    source_slot.clone(),
                    &plan_path,
                    before_handler,
                    &call.output_contract_ref,
                    handler_state_contract_ref,
                    route_contract,
                    arms,
                    bindings,
                    visible_mappers,
                    scope_failure,
                )?,
            };
            Ok(CertifiedFailureBoundary::Typed {
                failure_contract: Box::new(call.failure_contract.clone()),
                source_slot,
                plan: Box::new(plan),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_default_plan(
    semantic_call_id: &SemanticCallId,
    plan_id: mfm_ids::FailurePlanId,
    source_slot: LexicalSlot,
    plan_path: &StructuralPath,
    before_handler: ExpandedBlock,
    visible_mappers: &[FailureMapperRegistration],
    scope_failure: &StructuredFailureContract,
    registry: &StructuredCertificationRegistry,
) -> Result<FailurePlan> {
    let source_ref = source_slot.contract_ref.clone();
    let matches: Vec<&FailureMapperRegistration> = visible_mappers
        .iter()
        .filter(|mapper| mapper.source_failure_contract_ref == source_ref)
        .collect();
    if matches.len() != 1 {
        return Err(CertifyError::Certification(
            "default failure plan has zero or multiple exact mappers".to_owned(),
        ));
    }
    let mapper = matches[0];
    let contract = registry
        .state_contract(&mapper.mapper_state_contract_ref)
        .ok_or_else(|| {
            CertifyError::Certification("default mapper state is not registered".to_owned())
        })?;
    validate_handler_contract(contract, &source_ref)?;
    validate_registered_closed_sum(&mapper.route_contract, registry)?;
    if mapper.route_contract.selector_contract_ref != contract.output_contract_ref
        || mapper.route_contract.variants.len() != 1
        || mapper.route_contract.variants[0].canonical_tag != "propagate"
        || mapper.route_contract.variants[0].payloads.len() != 1
        || mapper.route_contract.variants[0].payloads[0].contract_ref
            != scope_failure.contract_ref()?
    {
        return Err(CertifyError::Certification(
            "default mapper route does not exactly propagate scope failure".to_owned(),
        ));
    }
    let handler = handler_binding(
        semantic_call_id,
        plan_path,
        contract,
        normal_tail_slot(&before_handler)?,
        u32::try_from(before_handler.declarations.len()).map_err(|_| {
            CertifyError::Certification("pre-handler declaration count exceeds u32".to_owned())
        })?,
    )?;
    let payload = &mapper.route_contract.variants[0].payloads[0];
    let payload_slot = LexicalSlot {
        lexical_path: plan_path.clone(),
        contract_ref: payload.contract_ref.clone(),
        producer: LexicalProducer::VariantPayload {
            selector: Box::new(handler.output_slot.clone()),
            canonical_tag: "propagate".to_owned(),
            payload_path: payload.payload_path.clone(),
        },
    };
    Ok(FailurePlan::Handled {
        plan_id,
        plan_path: plan_path.clone(),
        source_slot,
        before_handler: Box::new(before_handler),
        handler: Box::new(handler.clone()),
        continuation: Box::new(HandlerContinuation::DefaultPropagation {
            handler_output_slot: handler.output_slot,
            route_contract: mapper.route_contract.clone(),
            payload_slot: Box::new(payload_slot.clone()),
            failure_tail: Box::new(payload_slot),
        }),
    })
}

#[allow(clippy::too_many_arguments)]
fn build_custom_plan(
    context: &mut ExpansionContext<'_>,
    semantic_call_id: &SemanticCallId,
    semantic_path: &mfm_spec::structured::SemanticCallPath,
    authored_origin: &AuthoredCallOrigin,
    plan_id: mfm_ids::FailurePlanId,
    source_slot: LexicalSlot,
    plan_path: &StructuralPath,
    before_handler: ExpandedBlock,
    recovered_output_contract_ref: &ContentRef,
    handler_state_contract_ref: &ContentRef,
    route_contract: &ClosedSumContract,
    arms: &[mfm_spec::structured::AuthoredMatchArm],
    outer_bindings: &SlotBindings,
    visible_mappers: &[FailureMapperRegistration],
    scope_failure: &StructuredFailureContract,
) -> Result<FailurePlan> {
    let contract = context
        .registry
        .state_contract(handler_state_contract_ref)
        .cloned()
        .ok_or_else(|| {
            CertifyError::Certification("custom handler state is not registered".to_owned())
        })?;
    validate_handler_contract(&contract, &source_slot.contract_ref)?;
    validate_registered_closed_sum(route_contract, context.registry)?;
    if route_contract.selector_contract_ref != contract.output_contract_ref {
        return Err(CertifyError::Certification(
            "custom handler route/output contract mismatch".to_owned(),
        ));
    }
    let expected: Vec<&str> = route_contract
        .variants
        .iter()
        .map(|variant| variant.canonical_tag.as_str())
        .collect();
    let actual: Vec<&str> = arms.iter().map(|arm| arm.canonical_tag.as_str()).collect();
    if expected != actual {
        return Err(CertifyError::Certification(
            "custom handler continuation is not exhaustive".to_owned(),
        ));
    }
    let handler = handler_binding(
        semantic_call_id,
        plan_path,
        &contract,
        normal_tail_slot(&before_handler)?,
        u32::try_from(before_handler.declarations.len()).map_err(|_| {
            CertifyError::Certification("pre-handler declaration count exceeds u32".to_owned())
        })?,
    )?;
    let authored_plan_path =
        authored_origin
            .occurrence_path
            .child(StructuralPathSegment::FailurePlan {
                label: stable_id("handler")?,
            })?;
    let authored_handler_path = authored_plan_path.child(StructuralPathSegment::Declaration {
        label: stable_id("handler")?,
        ordinal: 0,
    })?;
    let authored_handler_slot = LexicalSlot {
        lexical_path: authored_handler_path,
        contract_ref: handler.output_slot.contract_ref.clone(),
        producer: LexicalProducer::AuthoredCallOutput {
            semantic_call_id: failure_handler_semantic_call_id(&authored_origin.semantic_call_id)?,
            role: ResultRole::SuccessOutput,
        },
    };
    let mut expanded_arms = Vec::with_capacity(arms.len());
    for (arm, variant) in arms.iter().zip(&route_contract.variants) {
        if arm.body.failure_scope.failure_contract() != scope_failure {
            return Err(CertifyError::Certification(
                "custom recovery arm changed its inherited failure scope".to_owned(),
            ));
        }
        let mut bindings = outer_bindings.clone();
        bindings.insert(&authored_handler_slot, handler.output_slot.clone())?;
        let arm_path = plan_path.child(StructuralPathSegment::MatchArm {
            label: arm.label.clone(),
            tag: arm.canonical_tag.clone(),
        })?;
        let mut arm_semantic = semantic_path.segments().to_vec();
        arm_semantic.push(mfm_spec::structured::SemanticPathSegment {
            label: arm.label.clone(),
            discriminator: Some(arm.label.clone()),
        });
        bind_variant_payloads(
            &mut bindings,
            &authored_handler_slot,
            &handler.output_slot,
            &arm.body.path,
            &arm_path,
            &arm.canonical_tag,
            variant,
        )?;
        let body = context.expand_block(
            &arm.body,
            bindings,
            visible_mappers.to_vec(),
            arm_path.clone(),
            arm_semantic,
        )?;
        if let BlockTail::Normal(slot) = &body.tail {
            if &slot.contract_ref != recovered_output_contract_ref {
                return Err(CertifyError::Certification(
                    "custom recovery normal route changed the protected output contract".to_owned(),
                ));
            }
        }
        expanded_arms.push(ExpandedMatchArm {
            canonical_tag: arm.canonical_tag.clone(),
            label: arm.label.clone(),
            path: arm_path,
            body,
        });
    }
    Ok(FailurePlan::Handled {
        plan_id,
        plan_path: plan_path.clone(),
        source_slot,
        before_handler: Box::new(before_handler),
        handler: Box::new(handler.clone()),
        continuation: Box::new(HandlerContinuation::CustomRecovery {
            handler_output_slot: handler.output_slot,
            route_contract: route_contract.clone(),
            arms: expanded_arms,
        }),
    })
}

fn handler_binding(
    protected_semantic_call_id: &SemanticCallId,
    plan_path: &StructuralPath,
    contract: &StructuredStateContract,
    source_slot: LexicalSlot,
    declaration_ordinal: u32,
) -> Result<ExpandedStateBinding> {
    let label = stable_id("handler")?;
    let occurrence_path = plan_path.child(StructuralPathSegment::Declaration {
        label: label.clone(),
        ordinal: declaration_ordinal,
    })?;
    let occurrence_id = occurrence_path
        .occurrence_id()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let output_slot = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: contract.output_contract_ref.clone(),
        producer: LexicalProducer::StateOutput {
            occurrence_id: occurrence_id.clone(),
            role: ResultRole::SuccessOutput,
        },
    };
    Ok(ExpandedStateBinding {
        semantic_call_id: failure_handler_semantic_call_id(protected_semantic_call_id)?,
        occurrence_id,
        occurrence_path,
        label,
        contract: contract.clone(),
        inputs: vec![source_slot],
        output_slot,
        failure_boundary: CertifiedFailureBoundary::NoFailure(NoFailureBoundary {
            never_contract_ref: never_failure_contract_ref()
                .map_err(|error| CertifyError::Certification(error.to_string()))?,
        }),
    })
}

fn validate_handler_contract(
    contract: &StructuredStateContract,
    source_contract_ref: &ContentRef,
) -> Result<()> {
    if !matches!(contract.execution, StructuredStateExecutionContract::Pure)
        || !matches!(contract.failure_contract, StructuredFailureContract::Never)
        || &contract.input_contract_ref != source_contract_ref
    {
        return Err(CertifyError::Certification(
            "failure handler is not exact Pure + Never".to_owned(),
        ));
    }
    Ok(())
}

fn validate_registered_closed_sum(
    contract: &ClosedSumContract,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    contract
        .validate()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    if registry.closed_sums.get(&contract.selector_contract_ref) != Some(contract) {
        return Err(CertifyError::Certification(
            "closed-sum tag and payload table is not process-qualified".to_owned(),
        ));
    }
    Ok(())
}

fn state_binding_expansion_boundary(
    authored: &AuthoredStateCall,
    authored_occurrence_path: StructuralPath,
    inputs: Vec<LexicalSlot>,
) -> Result<ExpandedBoundary> {
    let path = authored_occurrence_path.child(StructuralPathSegment::Fragment {
        label: authored.label.clone(),
        expansion_ref: authored.contract.state_contract_ref.clone(),
    })?;
    let boundary_id = path
        .fragment_boundary_id()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let occurrence_path = path.child(StructuralPathSegment::Declaration {
        label: authored.label.clone(),
        ordinal: 0,
    })?;
    let occurrence_id = occurrence_path
        .occurrence_id()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let mut input_bindings = Vec::with_capacity(inputs.len());
    let mut rebound_inputs = Vec::with_capacity(inputs.len());
    for (ordinal, source) in inputs.into_iter().enumerate() {
        let root_id = stable_id(&format!("input/{ordinal}"))?;
        rebound_inputs.push(LexicalSlot {
            lexical_path: path.clone(),
            contract_ref: source.contract_ref.clone(),
            producer: LexicalProducer::FragmentInput {
                boundary_id: boundary_id.clone(),
                child_root_id: root_id.clone(),
                source: Box::new(source.clone()),
            },
        });
        input_bindings.push(mfm_spec::structured::FragmentInputBinding {
            child_root_id: root_id,
            child_contract_ref: source.contract_ref.clone(),
            caller_slot: source,
        });
    }
    let state_success = state_output_slot(
        &occurrence_path,
        &occurrence_id,
        &authored.contract.output_contract_ref,
        ResultRole::SuccessOutput,
    );
    let success_slot = LexicalSlot {
        lexical_path: path.clone(),
        contract_ref: authored.contract.output_contract_ref.clone(),
        producer: LexicalProducer::FragmentBoundary {
            boundary_id: boundary_id.clone(),
            role: ResultRole::SuccessOutput,
            source: Box::new(state_success.clone()),
        },
    };
    let (state_failure_boundary, fragment_failure_boundary) =
        match &authored.contract.failure_contract {
            StructuredFailureContract::Never => {
                let marker = NoFailureBoundary {
                    never_contract_ref: never_failure_contract_ref()?,
                };
                (
                    CertifiedFailureBoundary::NoFailure(marker.clone()),
                    CertifiedFailureBoundary::NoFailure(marker),
                )
            }
            StructuredFailureContract::Typed { contract_ref, .. } => {
                let state_source = state_output_slot(
                    &occurrence_path,
                    &occurrence_id,
                    contract_ref,
                    ResultRole::TypedFailure,
                );
                let boundary_source = LexicalSlot {
                    lexical_path: path.clone(),
                    contract_ref: contract_ref.clone(),
                    producer: LexicalProducer::FragmentBoundary {
                        boundary_id: boundary_id.clone(),
                        role: ResultRole::TypedFailure,
                        source: Box::new(state_source.clone()),
                    },
                };
                let state_plan = propagation_plan(
                    &authored.semantic_call_id,
                    state_source.clone(),
                    &occurrence_path,
                    boundary_id.clone(),
                    boundary_source.clone(),
                )?;
                let placeholder_plan = propagation_plan(
                    &authored.semantic_call_id,
                    boundary_source.clone(),
                    &path,
                    boundary_id.clone(),
                    boundary_source.clone(),
                )?;
                (
                    CertifiedFailureBoundary::Typed {
                        failure_contract: Box::new(authored.contract.failure_contract.clone()),
                        source_slot: state_source,
                        plan: Box::new(state_plan),
                    },
                    CertifiedFailureBoundary::Typed {
                        failure_contract: Box::new(authored.contract.failure_contract.clone()),
                        source_slot: boundary_source,
                        plan: Box::new(placeholder_plan),
                    },
                )
            }
        };
    let binding = ExpandedStateBinding {
        semantic_call_id: authored.semantic_call_id.clone(),
        occurrence_id,
        occurrence_path,
        label: authored.label.clone(),
        contract: authored.contract.clone(),
        inputs: rebound_inputs,
        output_slot: state_success.clone(),
        failure_boundary: state_failure_boundary,
    };
    let body = ExpandedBlock {
        path: path.clone(),
        failure_scope: FailureScopeBinding::Inherits {
            scope_id: stable_id("policy-protected")?,
            failure_contract: authored.contract.failure_contract.clone(),
        },
        declarations: vec![ExpandedDeclaration::State(Box::new(binding.clone()))],
        failure_exits: failure_boundary_scope_exits(&binding.failure_boundary),
        tail: BlockTail::Normal(binding.output_slot.clone()),
    };
    Ok(ExpandedBoundary::from_leaf(ExpandedFragment {
        semantic_call_id: authored.semantic_call_id.clone(),
        label: authored.label.clone(),
        path,
        boundary_id,
        input_bindings,
        body,
        success_slot,
        failure_boundary: fragment_failure_boundary,
    }))
}

fn provisional_fragment_failure_boundary(
    semantic_call_id: &SemanticCallId,
    fragment_path: &StructuralPath,
    boundary_id: &FragmentBoundaryId,
    failure_contract: &StructuredFailureContract,
    body: &mut ExpandedBlock,
) -> Result<CertifiedFailureBoundary> {
    match failure_contract {
        StructuredFailureContract::Never => {
            if !body.failure_exits.is_empty() {
                return Err(CertifyError::Certification(
                    "infallible expansion recipe exposes a typed failure exit".to_owned(),
                ));
            }
            Ok(CertifiedFailureBoundary::NoFailure(NoFailureBoundary {
                never_contract_ref: never_failure_contract_ref()?,
            }))
        }
        StructuredFailureContract::Typed { contract_ref, .. } => {
            if body.failure_exits.is_empty()
                || body
                    .failure_exits
                    .iter()
                    .any(|slot| &slot.contract_ref != contract_ref)
            {
                return Err(CertifyError::Certification(
                    "typed expansion recipe lacks an exact failure source".to_owned(),
                ));
            }
            let body_failure = LexicalSlot {
                lexical_path: body.path.clone(),
                contract_ref: contract_ref.clone(),
                producer: LexicalProducer::ScopeFailureMerge {
                    scope_id: body.failure_scope.scope_id().clone(),
                    declaration_ordered_failure_slots: body.failure_exits.clone(),
                },
            };
            let boundary_source = LexicalSlot {
                lexical_path: fragment_path.clone(),
                contract_ref: contract_ref.clone(),
                producer: LexicalProducer::FragmentBoundary {
                    boundary_id: boundary_id.clone(),
                    role: ResultRole::TypedFailure,
                    source: Box::new(body_failure),
                },
            };
            link_direct_failure_exits(body, boundary_id, &boundary_source)?;
            let placeholder = propagation_plan(
                semantic_call_id,
                boundary_source.clone(),
                fragment_path,
                boundary_id.clone(),
                boundary_source.clone(),
            )?;
            Ok(CertifiedFailureBoundary::Typed {
                failure_contract: Box::new(failure_contract.clone()),
                source_slot: boundary_source,
                plan: Box::new(placeholder),
            })
        }
    }
}

fn link_direct_failure_exits(
    block: &mut ExpandedBlock,
    boundary_id: &FragmentBoundaryId,
    boundary_slot: &LexicalSlot,
) -> Result<()> {
    let exits = block.failure_exits.iter().cloned().collect::<BTreeSet<_>>();
    link_declaration_failure_exits(&mut block.declarations, &exits, boundary_id, boundary_slot)
}

fn link_declaration_failure_exits(
    declarations: &mut [ExpandedDeclaration],
    exits: &BTreeSet<LexicalSlot>,
    boundary_id: &FragmentBoundaryId,
    boundary_slot: &LexicalSlot,
) -> Result<()> {
    for declaration in declarations {
        match declaration {
            ExpandedDeclaration::State(state) => {
                link_failure_boundary_exit(
                    &mut state.failure_boundary,
                    exits,
                    boundary_id,
                    boundary_slot,
                )?;
            }
            ExpandedDeclaration::Match(binding) => {
                for arm in &mut binding.arms {
                    link_declaration_failure_exits(
                        &mut arm.body.declarations,
                        exits,
                        boundary_id,
                        boundary_slot,
                    )?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                for lane in &mut group.lanes {
                    link_declaration_failure_exits(
                        &mut lane.body.declarations,
                        exits,
                        boundary_id,
                        boundary_slot,
                    )?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                link_failure_boundary_exit(
                    &mut fragment.failure_boundary,
                    exits,
                    boundary_id,
                    boundary_slot,
                )?;
                link_declaration_failure_exits(
                    &mut fragment.body.declarations,
                    exits,
                    boundary_id,
                    boundary_slot,
                )?;
            }
        }
    }
    Ok(())
}

fn link_failure_boundary_exit(
    boundary: &mut CertifiedFailureBoundary,
    exits: &BTreeSet<LexicalSlot>,
    enclosing_boundary_id: &FragmentBoundaryId,
    enclosing_boundary_slot: &LexicalSlot,
) -> Result<()> {
    let CertifiedFailureBoundary::Typed {
        source_slot, plan, ..
    } = boundary
    else {
        return Ok(());
    };
    match plan.as_mut() {
        FailurePlan::Propagate {
            before_boundary, ..
        } => link_declaration_failure_exits(
            &mut before_boundary.declarations,
            exits,
            enclosing_boundary_id,
            enclosing_boundary_slot,
        )?,
        FailurePlan::Handled {
            before_handler,
            continuation,
            ..
        } => {
            link_declaration_failure_exits(
                &mut before_handler.declarations,
                exits,
                enclosing_boundary_id,
                enclosing_boundary_slot,
            )?;
            if let HandlerContinuation::CustomRecovery { arms, .. } = continuation.as_mut() {
                for arm in arms {
                    link_declaration_failure_exits(
                        &mut arm.body.declarations,
                        exits,
                        enclosing_boundary_id,
                        enclosing_boundary_slot,
                    )?;
                }
            }
        }
    }
    let current_exit = match plan.as_ref() {
        FailurePlan::Propagate {
            mapping_chain,
            boundary_slot,
            ..
        } if !mapping_chain.is_empty() => {
            local_mapped_fragment_failure_slot(source_slot, mapping_chain)
                .unwrap_or_else(|| boundary_slot.as_ref().clone())
        }
        FailurePlan::Propagate { .. } | FailurePlan::Handled { .. } => source_slot.clone(),
    };
    if !exits.contains(&current_exit) {
        return Ok(());
    }
    let FailurePlan::Propagate {
        boundary_id,
        boundary_slot,
        ..
    } = plan.as_mut()
    else {
        return Ok(());
    };
    *boundary_id = enclosing_boundary_id.clone();
    **boundary_slot = enclosing_boundary_slot.clone();
    Ok(())
}

fn local_mapped_fragment_failure_slot(
    source_slot: &LexicalSlot,
    mapping_chain: &[FailureMappingLink],
) -> Option<LexicalSlot> {
    let mapped_target = &mapping_chain.last()?.output_slot;
    let LexicalProducer::FragmentBoundary {
        boundary_id,
        role: ResultRole::TypedFailure,
        ..
    } = &source_slot.producer
    else {
        return None;
    };
    Some(LexicalSlot {
        lexical_path: source_slot.lexical_path.clone(),
        contract_ref: mapped_target.contract_ref.clone(),
        producer: LexicalProducer::FragmentBoundary {
            boundary_id: boundary_id.clone(),
            role: ResultRole::TypedFailure,
            source: Box::new(mapped_target.clone()),
        },
    })
}

fn state_output_slot(
    occurrence_path: &StructuralPath,
    occurrence_id: &OccurrenceId,
    contract_ref: &ContentRef,
    role: ResultRole,
) -> LexicalSlot {
    LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::StateOutput {
            occurrence_id: occurrence_id.clone(),
            role,
        },
    }
}

fn propagation_plan(
    semantic_call_id: &SemanticCallId,
    source_slot: LexicalSlot,
    source_path: &StructuralPath,
    boundary_id: FragmentBoundaryId,
    boundary_slot: LexicalSlot,
) -> Result<FailurePlan> {
    if source_slot.contract_ref != boundary_slot.contract_ref {
        return Err(CertifyError::Certification(
            "zero-link propagation changes the exact failure contract".to_owned(),
        ));
    }
    let plan_path = source_path.child(StructuralPathSegment::FailurePlan {
        label: stable_id("propagate")?,
    })?;
    let plan_id = FailurePlanIdentity {
        source_semantic_call_id: semantic_call_id.clone(),
        source_slot: source_slot.clone(),
        plan_path: plan_path.clone(),
    }
    .derive()
    .map_err(|error| CertifyError::Certification(error.to_string()))?;
    Ok(FailurePlan::Propagate {
        plan_id,
        before_boundary: Box::new(ExpandedBlock {
            path: plan_path.clone(),
            failure_scope: FailureScopeBinding::Inherits {
                scope_id: stable_id("fragment-propagation")?,
                failure_contract: StructuredFailureContract::Never,
            },
            declarations: Vec::new(),
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(source_slot.clone()),
        }),
        plan_path,
        source_slot,
        mapping_chain: Vec::new(),
        boundary_id,
        boundary_slot: Box::new(boundary_slot),
    })
}

#[allow(clippy::too_many_arguments)]
fn complete_external_failure_boundary(
    context: &mut ExpansionContext<'_>,
    authored: &AuthoredStateCall,
    authored_origin: &AuthoredCallOrigin,
    fragment: &mut ExpandedFragment,
    bindings: &SlotBindings,
    visible_mappers: &[FailureMapperRegistration],
    scope_id: &StableId,
    scope_failure: &StructuredFailureContract,
) -> Result<()> {
    match (
        &authored.contract.failure_contract,
        &fragment.failure_boundary,
    ) {
        (StructuredFailureContract::Never, CertifiedFailureBoundary::NoFailure(marker)) => {
            if marker.never_contract_ref != never_failure_contract_ref()? {
                return Err(CertifyError::Certification(
                    "capability or policy forged its Never marker".to_owned(),
                ));
            }
            return Ok(());
        }
        (StructuredFailureContract::Never, _) => {
            return Err(CertifyError::Certification(
                "capability or policy widened a Never boundary".to_owned(),
            ));
        }
        (StructuredFailureContract::Typed { .. }, CertifiedFailureBoundary::NoFailure(_)) => {
            return Err(CertifyError::Certification(
                "capability or policy erased a typed failure boundary".to_owned(),
            ));
        }
        (StructuredFailureContract::Typed { .. }, CertifiedFailureBoundary::Typed { .. }) => {}
    }
    let CertifiedFailureBoundary::Typed {
        failure_contract,
        source_slot,
        ..
    } = &fragment.failure_boundary
    else {
        return Err(CertifyError::Certification(
            "expanded boundary failure classification changed during completion".to_owned(),
        ));
    };
    if failure_contract.as_ref() != &authored.contract.failure_contract
        || source_slot.contract_ref != authored.contract.failure_contract.contract_ref()?
        || !matches!(
            &source_slot.producer,
            LexicalProducer::FragmentBoundary {
                boundary_id,
                role: ResultRole::TypedFailure,
                ..
            } if boundary_id == &fragment.boundary_id
        )
    {
        return Err(CertifyError::Certification(
            "capability or policy returned a forged typed boundary source".to_owned(),
        ));
    }
    let source_slot = source_slot.clone();
    let plan_path = fragment.path.child(StructuralPathSegment::FailurePlan {
        label: stable_id("handler")?,
    })?;
    let plan_id = FailurePlanIdentity {
        source_semantic_call_id: authored.semantic_call_id.clone(),
        source_slot: source_slot.clone(),
        plan_path: plan_path.clone(),
    }
    .derive()
    .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let before_handler = pre_handler_block(
        &plan_path,
        scope_id,
        scope_failure,
        Vec::new(),
        source_slot.clone(),
    )?;
    let plan = match &authored.failure_directive {
        AuthoredFailureDirective::NoFailure => {
            return Err(CertifyError::Certification(
                "typed expanded boundary has no call-site failure plan".to_owned(),
            ));
        }
        AuthoredFailureDirective::Default => build_default_plan(
            &authored.semantic_call_id,
            plan_id,
            source_slot.clone(),
            &plan_path,
            before_handler.clone(),
            visible_mappers,
            scope_failure,
            context.registry,
        )?,
        AuthoredFailureDirective::Custom {
            handler_state_contract_ref,
            route_contract,
            arms,
        } => build_custom_plan(
            context,
            &authored.semantic_call_id,
            &authored.semantic_path,
            authored_origin,
            plan_id,
            source_slot.clone(),
            &plan_path,
            before_handler,
            &authored.contract.output_contract_ref,
            handler_state_contract_ref,
            route_contract,
            arms,
            bindings,
            visible_mappers,
            scope_failure,
        )?,
    };
    fragment.failure_boundary = CertifiedFailureBoundary::Typed {
        failure_contract: Box::new(authored.contract.failure_contract.clone()),
        source_slot,
        plan: Box::new(plan),
    };
    Ok(())
}

fn validate_external_boundary(
    authored: &AuthoredStateCall,
    fragment: &ExpandedFragment,
) -> Result<()> {
    let expected_boundary_id = fragment
        .path
        .fragment_boundary_id()
        .map_err(|error| CertifyError::Planning(error.to_string()))?;
    let success_source = match &fragment.success_slot.producer {
        LexicalProducer::FragmentBoundary {
            boundary_id,
            role: ResultRole::SuccessOutput,
            source,
        } if boundary_id == &fragment.boundary_id => source,
        _ => {
            return Err(CertifyError::Planning(
                "capability or policy forged its success boundary producer".to_owned(),
            ));
        }
    };
    if fragment.semantic_call_id != authored.semantic_call_id
        || fragment.boundary_id != expected_boundary_id
        || fragment.body.path != fragment.path
        || fragment.success_slot.contract_ref != authored.contract.output_contract_ref
        || success_source.contract_ref != authored.contract.output_contract_ref
        || !matches!(&fragment.body.tail, BlockTail::Normal(slot) if slot == success_source.as_ref())
        || fragment_failure_contract(&fragment.failure_boundary)?
            != authored.contract.failure_contract
    {
        return Err(CertifyError::Planning(
            "capability or policy expansion widened the protected boundary".to_owned(),
        ));
    }
    Ok(())
}

fn fragment_failure_contract(
    boundary: &CertifiedFailureBoundary,
) -> Result<StructuredFailureContract> {
    match boundary {
        CertifiedFailureBoundary::NoFailure(marker) => {
            if marker.never_contract_ref != never_failure_contract_ref()? {
                return Err(CertifyError::Certification(
                    "forged Never boundary marker".to_owned(),
                ));
            }
            Ok(StructuredFailureContract::Never)
        }
        CertifiedFailureBoundary::Typed {
            failure_contract, ..
        } => Ok(failure_contract.as_ref().clone()),
    }
}

fn reject_effects(block: &ExpandedBlock) -> Result<()> {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state)
                if matches!(
                    state.contract.execution,
                    StructuredStateExecutionContract::Effect { .. }
                ) =>
            {
                return Err(CertifyError::Certification(
                    "Effect is forbidden transitively inside FanOut".to_owned(),
                ));
            }
            ExpandedDeclaration::State(_) => {}
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    reject_effects(&arm.body)?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    reject_effects(&lane.body)?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => reject_effects(&fragment.body)?,
        }
    }
    Ok(())
}

fn validate_profile(
    profile: &StructuredExpansionProfile,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    if profile.max_occurrences == 0
        || profile.max_occurrences as usize > MAX_STRUCTURED_OCCURRENCES
        || profile.max_declarations == 0
        || profile.max_declarations as usize > MAX_STRUCTURED_DECLARATIONS
        || profile.max_lanes == 0
        || profile.max_lanes as usize > MAX_STRUCTURED_LANES
        || profile.max_fan_out_depth == 0
        || profile.max_fan_out_depth > MAX_FAN_OUT_DEPTH
        || profile.max_branch_depth == 0
    {
        return Err(CertifyError::Certification(
            "structured expansion profile exceeds kernel bounds".to_owned(),
        ));
    }
    let mut policies = BTreeSet::new();
    for policy in &profile.policies {
        policy
            .validate()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if !policies.insert(policy.policy_ref.clone()) {
            return Err(CertifyError::Certification(
                "duplicate expansion policy".to_owned(),
            ));
        }
        let mut eligible = BTreeSet::new();
        if policy
            .boundary_recipes
            .iter()
            .any(|binding| !eligible.insert(binding.boundary_contract_ref.clone()))
        {
            return Err(CertifyError::Certification(
                "duplicate eligible policy boundary".to_owned(),
            ));
        }
        for binding in &policy.boundary_recipes {
            if !registry.policy_recipes.contains_key(&binding.recipe_ref) {
                return Err(CertifyError::Certification(
                    "expansion profile policy has no exact qualified recipe".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_policy_root(
    authored: &AuthoredStructuredProgram,
    policy: &QualifiedStructuredEntryPointPolicy,
    profile: &StructuredExpansionProfile,
) -> Result<()> {
    let input_contracts: Vec<ContentRef> = authored
        .input_roots
        .iter()
        .map(|root| root.contract_ref.clone())
        .collect();
    if policy.derived_admission_policy_ref()? != policy.admission_policy_ref
        || authored.operation_id != policy.operation_id
        || profile.content_ref()? != policy.expansion_profile_ref
        || input_contracts != policy.public_input_contract_refs
        || authored.output_contract_ref != policy.public_output_contract_ref
        || authored.failure_contract.contract_ref()? != policy.public_failure_contract_ref
    {
        return Err(CertifyError::Certification(
            "qualified entry-point policy mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn validate_authored_program(
    authored: &AuthoredStructuredProgram,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    authored
        .failure_contract
        .validate()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let root_path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: authored.operation_id.clone(),
    }])?;
    let mut roots = BTreeSet::new();
    if authored.root.path != root_path
        || !matches!(
            authored.root.failure_scope,
            FailureScopeBinding::Owns { .. }
        )
        || authored.root.failure_scope.failure_contract() != &authored.failure_contract
        || authored.input_roots.iter().any(|root| {
            root.lexical_path != root_path
                || !matches!(
                    &root.producer,
                    LexicalProducer::AdmissionRoot { root_id }
                        if roots.insert(root_id.clone())
                )
        })
    {
        return Err(CertifyError::Certification(
            "authored root path, input roots, or failure scope is not exact".to_owned(),
        ));
    }
    validate_authored_block(&authored.root, registry, 0, &root_path, &[], None)
}

fn validate_authored_block(
    block: &AuthoredBlock,
    registry: &StructuredCertificationRegistry,
    depth: usize,
    expected_path: &StructuralPath,
    semantic_prefix: &[mfm_spec::structured::SemanticPathSegment],
    inherited_scope: Option<&FailureScopeBinding>,
) -> Result<()> {
    if depth > 128 {
        return Err(CertifyError::Certification(
            "authored structural depth exceeded".to_owned(),
        ));
    }
    if &block.path != expected_path {
        return Err(CertifyError::Certification(
            "authored block structural path mismatch".to_owned(),
        ));
    }
    if let Some(parent) = inherited_scope {
        if block.failure_scope.scope_id() != parent.scope_id()
            || block.failure_scope.failure_contract() != parent.failure_contract()
            || !matches!(block.failure_scope, FailureScopeBinding::Inherits { .. })
        {
            return Err(CertifyError::Certification(
                "authored inherited failure scope substitution".to_owned(),
            ));
        }
    }
    if let FailureScopeBinding::Owns { scope } = &block.failure_scope {
        scope
            .failure_contract
            .validate()
            .map_err(|error| CertifyError::Certification(error.to_string()))?;
        if matches!(scope.failure_contract, StructuredFailureContract::Never)
            && !scope.default_mappers.is_empty()
        {
            return Err(CertifyError::Certification(
                "Never scope cannot own a default failure map".to_owned(),
            ));
        }
        let mut sources = BTreeSet::new();
        if scope
            .default_mappers
            .iter()
            .any(|mapper| !sources.insert(mapper.source_failure_contract_ref.clone()))
        {
            return Err(CertifyError::Certification(
                "duplicate exact failure mapper".to_owned(),
            ));
        }
        for mapper in &scope.default_mappers {
            let contract = registry
                .state_contract(&mapper.mapper_state_contract_ref)
                .ok_or_else(|| {
                    CertifyError::Certification(
                        "authored failure mapper is not qualified".to_owned(),
                    )
                })?;
            validate_handler_contract(contract, &mapper.source_failure_contract_ref)?;
            validate_registered_closed_sum(&mapper.route_contract, registry)?;
        }
    }
    let mut labels = BTreeSet::new();
    for (ordinal, declaration) in block.declarations.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| {
            CertifyError::Certification("authored declaration count exceeds u32".to_owned())
        })?;
        let label = authored_declaration_label(declaration);
        if !labels.insert(label.clone()) {
            return Err(CertifyError::Certification(
                "duplicate authored declaration label".to_owned(),
            ));
        }
        let path = declaration_path(expected_path, label, ordinal)?;
        match declaration {
            AuthoredDeclaration::State(state) => {
                let expected_semantic = mfm_spec::structured::SemanticCallPath::new(
                    declaration_semantic_prefix(semantic_prefix, &state.label)?,
                )?;
                if state.semantic_path != expected_semantic
                    || state.semantic_path.identity()? != state.semantic_call_id
                    || state.inputs.len() != 1
                    || state.inputs[0].contract_ref != state.contract.input_contract_ref
                    || !authored_call_output_is_exact(
                        &state.output_slot,
                        &path,
                        &state.semantic_call_id,
                        &state.contract.output_contract_ref,
                    )
                {
                    return Err(CertifyError::Certification(
                        "authored state identity, input contract, or output producer mismatch"
                            .to_owned(),
                    ));
                }
                if registry.state_contract(&state.contract.state_contract_ref)
                    != Some(&state.contract)
                {
                    return Err(CertifyError::Certification(
                        "authored state is not qualified".to_owned(),
                    ));
                }
                validate_authored_failure_directive(
                    &state.contract.failure_contract,
                    &state.failure_directive,
                    &state.semantic_path,
                    &path,
                    &state.contract.output_contract_ref,
                    &block.failure_scope,
                    registry,
                    depth,
                )?;
            }
            AuthoredDeclaration::OperationCall(call) => {
                let expected_semantic = mfm_spec::structured::SemanticCallPath::new(
                    declaration_semantic_prefix(semantic_prefix, &call.label)?,
                )?;
                if call.semantic_path != expected_semantic
                    || call.semantic_path.identity()? != call.semantic_call_id
                    || !authored_call_output_is_exact(
                        &call.output_slot,
                        &path,
                        &call.semantic_call_id,
                        &call.output_contract_ref,
                    )
                    || !registry.children.contains_key(&call.child_program_ref)
                {
                    return Err(CertifyError::Certification(
                        "authored child call is not exact and qualified".to_owned(),
                    ));
                }
                validate_authored_failure_directive(
                    &call.failure_contract,
                    &call.failure_directive,
                    &call.semantic_path,
                    &path,
                    &call.output_contract_ref,
                    &block.failure_scope,
                    registry,
                    depth,
                )?;
            }
            AuthoredDeclaration::Match(binding) => {
                validate_registered_closed_sum(&binding.selector_contract, registry)?;
                let expected_tags: Vec<&str> = binding
                    .selector_contract
                    .variants
                    .iter()
                    .map(|variant| variant.canonical_tag.as_str())
                    .collect();
                let actual_tags: Vec<&str> = binding
                    .arms
                    .iter()
                    .map(|arm| arm.canonical_tag.as_str())
                    .collect();
                let mut arm_labels = BTreeSet::new();
                for arm in &binding.arms {
                    if !arm_labels.insert(arm.label.clone()) {
                        return Err(CertifyError::Certification(
                            "authored Match has duplicate stable arm label".to_owned(),
                        ));
                    }
                }
                let normal_slots: Vec<LexicalSlot> = binding
                    .arms
                    .iter()
                    .filter_map(|arm| match &arm.body.tail {
                        BlockTail::Normal(slot) => Some(slot.clone()),
                        BlockTail::ScopeFailure(_) => None,
                    })
                    .collect();
                if expected_tags != actual_tags
                    || binding.output_slot.lexical_path != path
                    || !matches!(
                        &binding.output_slot.producer,
                        LexicalProducer::MatchMerge {
                            match_path,
                            declaration_ordered_arm_slots,
                        } if match_path == &path
                            && declaration_ordered_arm_slots == &normal_slots
                    )
                {
                    return Err(CertifyError::Certification(
                        "authored Match path or exhaustive tag order mismatch".to_owned(),
                    ));
                }
                let match_semantic = declaration_semantic_prefix(semantic_prefix, &binding.label)?;
                for arm in &binding.arms {
                    let arm_path = path.child(StructuralPathSegment::MatchArm {
                        label: arm.label.clone(),
                        tag: arm.canonical_tag.clone(),
                    })?;
                    let mut arm_semantic = match_semantic.clone();
                    arm_semantic.push(mfm_spec::structured::SemanticPathSegment {
                        label: arm.label.clone(),
                        discriminator: Some(arm.label.clone()),
                    });
                    validate_authored_block(
                        &arm.body,
                        registry,
                        depth + 1,
                        &arm_path,
                        &arm_semantic,
                        Some(&block.failure_scope),
                    )?;
                }
            }
            AuthoredDeclaration::FanOut(group) => {
                if group.lanes.is_empty() {
                    return Err(CertifyError::Certification(
                        "authored FanOut is empty".to_owned(),
                    ));
                }
                let mut lane_keys = BTreeSet::new();
                let lane_tail_slots: Vec<LexicalSlot> = group
                    .lanes
                    .iter()
                    .map(|lane| match &lane.body.tail {
                        BlockTail::Normal(slot) | BlockTail::ScopeFailure(slot) => slot.clone(),
                    })
                    .collect();
                let expected_join_contract_ref = fan_out_join_contract_ref(
                    &group.lane_output_contract_ref,
                    &group.lane_failure_contract,
                )?;
                if group.output_slot.contract_ref != expected_join_contract_ref
                    || group.output_slot.lexical_path != path
                    || group
                        .lanes
                        .iter()
                        .any(|lane| !lane_keys.insert(lane.key.clone()))
                    || !matches!(
                        &group.output_slot.producer,
                        LexicalProducer::FanOutJoin {
                            group_path,
                            declaration_ordered_lane_slots,
                        } if group_path == &path
                            && declaration_ordered_lane_slots == &lane_tail_slots
                    )
                {
                    return Err(CertifyError::Certification(
                        "authored fan-out join, path, or lane keys are not exact".to_owned(),
                    ));
                }
                let fan_semantic = declaration_semantic_prefix(semantic_prefix, &group.label)?;
                for (lane_ordinal, lane) in group.lanes.iter().enumerate() {
                    if usize::try_from(lane.declaration_ordinal).ok() != Some(lane_ordinal)
                        || lane.body.failure_scope.failure_contract()
                            != &group.lane_failure_contract
                        || !matches!(lane.body.failure_scope, FailureScopeBinding::Owns { .. })
                    {
                        return Err(CertifyError::Certification(
                            "authored fan-out lane scope or ordinal mismatch".to_owned(),
                        ));
                    }
                    let lane_path = path.child(StructuralPathSegment::FanOutLane {
                        key: lane.key.clone(),
                        ordinal: lane.declaration_ordinal,
                    })?;
                    let mut lane_semantic = fan_semantic.clone();
                    lane_semantic.push(mfm_spec::structured::SemanticPathSegment {
                        label: lane.key.clone(),
                        discriminator: Some(lane.key.clone()),
                    });
                    validate_authored_block(
                        &lane.body,
                        registry,
                        depth + 1,
                        &lane_path,
                        &lane_semantic,
                        None,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn authored_declaration_label(declaration: &AuthoredDeclaration) -> &StableId {
    match declaration {
        AuthoredDeclaration::State(state) => &state.label,
        AuthoredDeclaration::OperationCall(call) => &call.label,
        AuthoredDeclaration::Match(binding) => &binding.label,
        AuthoredDeclaration::FanOut(group) => &group.label,
    }
}

fn authored_call_output_is_exact(
    slot: &LexicalSlot,
    path: &StructuralPath,
    semantic_call_id: &SemanticCallId,
    contract_ref: &ContentRef,
) -> bool {
    slot.lexical_path == *path
        && slot.contract_ref == *contract_ref
        && matches!(
            &slot.producer,
            LexicalProducer::AuthoredCallOutput {
                semantic_call_id: producer,
                role: ResultRole::SuccessOutput,
            } if producer == semantic_call_id
        )
}

#[allow(clippy::too_many_arguments)]
fn validate_authored_failure_directive(
    failure_contract: &StructuredFailureContract,
    directive: &AuthoredFailureDirective,
    semantic_path: &mfm_spec::structured::SemanticCallPath,
    occurrence_path: &StructuralPath,
    recovered_output_contract_ref: &ContentRef,
    failure_scope: &FailureScopeBinding,
    registry: &StructuredCertificationRegistry,
    depth: usize,
) -> Result<()> {
    match (failure_contract, directive) {
        (StructuredFailureContract::Never, AuthoredFailureDirective::NoFailure) => Ok(()),
        (StructuredFailureContract::Never, _) => Err(CertifyError::Certification(
            "Never authored call carries a failure handler".to_owned(),
        )),
        (StructuredFailureContract::Typed { .. }, AuthoredFailureDirective::NoFailure) => Err(
            CertifyError::Certification("typed authored call omits its failure handler".to_owned()),
        ),
        (StructuredFailureContract::Typed { .. }, AuthoredFailureDirective::Default) => Ok(()),
        (
            StructuredFailureContract::Typed { contract_ref, .. },
            AuthoredFailureDirective::Custom {
                handler_state_contract_ref,
                route_contract,
                arms,
            },
        ) => {
            let handler = registry
                .state_contract(handler_state_contract_ref)
                .ok_or_else(|| {
                    CertifyError::Certification(
                        "custom failure handler is not qualified".to_owned(),
                    )
                })?;
            validate_handler_contract(handler, contract_ref)?;
            validate_registered_closed_sum(route_contract, registry)?;
            if route_contract.selector_contract_ref != handler.output_contract_ref
                || !route_contract
                    .variants
                    .iter()
                    .map(|variant| variant.canonical_tag.as_str())
                    .eq(arms.iter().map(|arm| arm.canonical_tag.as_str()))
            {
                return Err(CertifyError::Certification(
                    "custom failure route is not exact and exhaustive".to_owned(),
                ));
            }
            let mut arm_labels = BTreeSet::new();
            for arm in arms {
                if !arm_labels.insert(arm.label.clone()) {
                    return Err(CertifyError::Certification(
                        "custom recovery has duplicate stable arm label".to_owned(),
                    ));
                }
            }
            let plan_path = occurrence_path.child(StructuralPathSegment::FailurePlan {
                label: stable_id("handler")?,
            })?;
            for arm in arms {
                let arm_path = plan_path.child(StructuralPathSegment::MatchArm {
                    label: arm.label.clone(),
                    tag: arm.canonical_tag.clone(),
                })?;
                let mut arm_semantic = semantic_path.segments().to_vec();
                arm_semantic.push(mfm_spec::structured::SemanticPathSegment {
                    label: arm.label.clone(),
                    discriminator: Some(arm.label.clone()),
                });
                validate_authored_block(
                    &arm.body,
                    registry,
                    depth + 1,
                    &arm_path,
                    &arm_semantic,
                    Some(failure_scope),
                )?;
                if let BlockTail::Normal(slot) = &arm.body.tail {
                    if &slot.contract_ref != recovered_output_contract_ref {
                        return Err(CertifyError::Certification(
                            "custom recovery arm changes the protected output contract".to_owned(),
                        ));
                    }
                }
            }
            Ok(())
        }
    }
}

fn validate_expanded_program(
    expanded: &ExpandedStructuredProgram,
    profile: &StructuredExpansionProfile,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    let mut metrics = ProgramMetrics::default();
    validate_expanded_block(
        &expanded.root,
        registry,
        &mut metrics,
        0,
        profile.max_fan_out_depth,
        false,
    )?;
    if metrics.occurrences > profile.max_occurrences as usize
        || metrics.declarations > profile.max_declarations as usize
        || metrics.lanes > profile.max_lanes as usize
        || metrics.branch_depth > profile.max_branch_depth as usize
    {
        return Err(CertifyError::Certification(
            "expanded program exceeds its qualified structural bounds".to_owned(),
        ));
    }
    match &expanded.root.tail {
        BlockTail::Normal(slot) if slot.contract_ref == expanded.output_contract_ref => {}
        BlockTail::ScopeFailure(slot)
            if slot.contract_ref == expanded.failure_contract.contract_ref()? => {}
        _ => {
            return Err(CertifyError::Certification(
                "expanded root does not have one exact typed outcome".to_owned(),
            ));
        }
    }
    validate_eventual_failure_handlers(expanded)?;
    validate_expanded_lexical_bindings(expanded)?;
    Ok(())
}

#[derive(Clone, Default)]
struct VisibleSlots(BTreeSet<LexicalSlot>);

impl VisibleSlots {
    fn insert(&mut self, slot: &LexicalSlot) -> Result<()> {
        self.0.insert(slot.clone());
        Ok(())
    }

    fn contains(&self, slot: &LexicalSlot) -> Result<bool> {
        Ok(self.0.contains(slot))
    }
}

fn validate_expanded_lexical_bindings(expanded: &ExpandedStructuredProgram) -> Result<()> {
    let root_path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: expanded.operation_id.clone(),
    }])?;
    if expanded.root.path != root_path {
        return Err(CertifyError::Certification(
            "expanded root structural path mismatch".to_owned(),
        ));
    }
    let mut visible = VisibleSlots::default();
    let mut roots = BTreeSet::new();
    for root in &expanded.input_roots {
        if root.lexical_path != root_path
            || !matches!(
                &root.producer,
                LexicalProducer::AdmissionRoot { root_id }
                    if roots.insert(root_id.clone())
            )
        {
            return Err(CertifyError::Certification(
                "expanded admission root is forged or duplicated".to_owned(),
            ));
        }
        visible.insert(root)?;
    }
    validate_block_lexical(&expanded.root, &mut visible)
}

fn validate_block_lexical(block: &ExpandedBlock, visible: &mut VisibleSlots) -> Result<()> {
    let mut expected_failure_exits = Vec::new();
    let mut labels = BTreeSet::new();
    for (ordinal, declaration) in block.declarations.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| {
            CertifyError::Certification("expanded declaration count exceeds u32".to_owned())
        })?;
        let label = expanded_declaration_label(declaration);
        if !labels.insert(label.clone()) {
            return Err(CertifyError::Certification(
                "expanded block has duplicate stable declaration labels".to_owned(),
            ));
        }
        let expected_path = declaration_path(&block.path, label, ordinal)?;
        match declaration {
            ExpandedDeclaration::State(state) => {
                if state.occurrence_path != expected_path
                    || !all_slots_visible(&state.inputs, visible)?
                {
                    return Err(CertifyError::Certification(
                        "expanded state uses a non-dominating input or wrong declaration path"
                            .to_owned(),
                    ));
                }
                validate_boundary_lexical(
                    &state.failure_boundary,
                    visible,
                    &state.semantic_call_id,
                )?;
                expected_failure_exits
                    .extend(failure_boundary_scope_exits(&state.failure_boundary));
                visible.insert(&state.output_slot)?;
            }
            ExpandedDeclaration::Match(binding) => {
                if binding.path != expected_path || !visible.contains(&binding.selector)? {
                    return Err(CertifyError::Certification(
                        "expanded Match selector or structural path is not dominating".to_owned(),
                    ));
                }
                let mut arm_labels = BTreeSet::new();
                let mut arm_slots = Vec::new();
                for (arm, variant) in binding.arms.iter().zip(&binding.selector_contract.variants) {
                    if !arm_labels.insert(arm.label.clone()) {
                        return Err(CertifyError::Certification(
                            "expanded Match has duplicate stable arm label".to_owned(),
                        ));
                    }
                    let arm_path = binding.path.child(StructuralPathSegment::MatchArm {
                        label: arm.label.clone(),
                        tag: arm.canonical_tag.clone(),
                    })?;
                    if arm.path != arm_path || arm.body.path != arm_path {
                        return Err(CertifyError::Certification(
                            "expanded Match arm path substitution".to_owned(),
                        ));
                    }
                    let mut arm_visible = visible.clone();
                    for payload in &variant.payloads {
                        arm_visible.insert(&LexicalSlot {
                            lexical_path: arm_path.clone(),
                            contract_ref: payload.contract_ref.clone(),
                            producer: LexicalProducer::VariantPayload {
                                selector: Box::new(binding.selector.clone()),
                                canonical_tag: arm.canonical_tag.clone(),
                                payload_path: payload.payload_path.clone(),
                            },
                        })?;
                    }
                    validate_block_lexical(&arm.body, &mut arm_visible)?;
                    expected_failure_exits.extend(arm.body.failure_exits.iter().cloned());
                    if let BlockTail::Normal(slot) = &arm.body.tail {
                        arm_slots.push(LexicalSlot {
                            lexical_path: arm_path.clone(),
                            contract_ref: slot.contract_ref.clone(),
                            producer: LexicalProducer::ArmValue {
                                selected_arm_path: arm_path,
                                source: Box::new(slot.clone()),
                            },
                        });
                    }
                }
                let expected_output = LexicalSlot {
                    lexical_path: binding.path.clone(),
                    contract_ref: binding.output_slot.contract_ref.clone(),
                    producer: LexicalProducer::MatchMerge {
                        match_path: binding.path.clone(),
                        declaration_ordered_arm_slots: arm_slots,
                    },
                };
                if binding.output_slot != expected_output {
                    return Err(CertifyError::Certification(
                        "expanded Match merge recipe substitution".to_owned(),
                    ));
                }
                visible.insert(&binding.output_slot)?;
            }
            ExpandedDeclaration::FanOut(group) => {
                if group.path != expected_path {
                    return Err(CertifyError::Certification(
                        "expanded fan-out group path substitution".to_owned(),
                    ));
                }
                let mut outcome_slots = Vec::with_capacity(group.lanes.len());
                let mut lane_keys = BTreeSet::new();
                for lane in &group.lanes {
                    let lane_path = group.path.child(StructuralPathSegment::FanOutLane {
                        key: lane.key.clone(),
                        ordinal: lane.declaration_ordinal,
                    })?;
                    if !lane_keys.insert(lane.key.clone())
                        || lane.path != lane_path
                        || lane.body.path != lane_path
                    {
                        return Err(CertifyError::Certification(
                            "expanded fan-out lane key or path substitution".to_owned(),
                        ));
                    }
                    let mut lane_visible = visible.clone();
                    validate_block_lexical(&lane.body, &mut lane_visible)?;
                    let success_slot = match &lane.body.tail {
                        BlockTail::Normal(slot) => Some(Box::new(slot.clone())),
                        BlockTail::ScopeFailure(_) => None,
                    };
                    let failure_slot = match &group.lane_failure_contract {
                        StructuredFailureContract::Never => None,
                        StructuredFailureContract::Typed { contract_ref, .. }
                            if !lane.body.failure_exits.is_empty() =>
                        {
                            Some(Box::new(LexicalSlot {
                                lexical_path: lane.body.path.clone(),
                                contract_ref: contract_ref.clone(),
                                producer: LexicalProducer::ScopeFailureMerge {
                                    scope_id: lane.body.failure_scope.scope_id().clone(),
                                    declaration_ordered_failure_slots: lane
                                        .body
                                        .failure_exits
                                        .clone(),
                                },
                            }))
                        }
                        StructuredFailureContract::Typed { .. } => None,
                    };
                    let expected_outcome = LexicalSlot {
                        lexical_path: lane_path.clone(),
                        contract_ref: lane_outcome_contract_ref(
                            &group.lane_output_contract_ref,
                            &group.lane_failure_contract,
                        )?,
                        producer: LexicalProducer::LaneOutcome {
                            lane_path,
                            success_slot,
                            failure_slot,
                        },
                    };
                    if lane.outcome_slot != expected_outcome {
                        return Err(CertifyError::Certification(
                            "expanded nominal lane-outcome recipe substitution".to_owned(),
                        ));
                    }
                    outcome_slots.push(expected_outcome);
                }
                let expected_output = LexicalSlot {
                    lexical_path: group.path.clone(),
                    contract_ref: fan_out_join_contract_ref(
                        &group.lane_output_contract_ref,
                        &group.lane_failure_contract,
                    )?,
                    producer: LexicalProducer::FanOutJoin {
                        group_path: group.path.clone(),
                        declaration_ordered_lane_slots: outcome_slots,
                    },
                };
                if group.output_slot != expected_output {
                    return Err(CertifyError::Certification(
                        "expanded fan-out join recipe substitution".to_owned(),
                    ));
                }
                visible.insert(&group.output_slot)?;
            }
            ExpandedDeclaration::Fragment(fragment) => {
                if !fragment_path_extends_declaration(&fragment.path, &expected_path)
                    || fragment.path.fragment_boundary_id()? != fragment.boundary_id
                    || fragment.body.path != fragment.path
                {
                    return Err(CertifyError::Certification(
                        "expanded fragment path or boundary identity substitution".to_owned(),
                    ));
                }
                let mut inner_visible = VisibleSlots::default();
                let mut root_ids = BTreeSet::new();
                for input in &fragment.input_bindings {
                    if !root_ids.insert(input.child_root_id.clone()) {
                        return Err(CertifyError::Certification(
                            "expanded fragment input binding repeats a root identity".to_owned(),
                        ));
                    }
                    if input.caller_slot.contract_ref != input.child_contract_ref {
                        return Err(CertifyError::Certification(
                            "expanded fragment input binding changes its root contract".to_owned(),
                        ));
                    }
                    if !visible.contains(&input.caller_slot)? {
                        return Err(CertifyError::Certification(
                            "expanded fragment input binding is not a dominating caller slot"
                                .to_owned(),
                        ));
                    }
                    inner_visible.insert(&LexicalSlot {
                        lexical_path: fragment.path.clone(),
                        contract_ref: input.child_contract_ref.clone(),
                        producer: LexicalProducer::FragmentInput {
                            boundary_id: fragment.boundary_id.clone(),
                            child_root_id: input.child_root_id.clone(),
                            source: Box::new(input.caller_slot.clone()),
                        },
                    })?;
                }
                validate_block_lexical(&fragment.body, &mut inner_visible)?;
                let body_success = match &fragment.body.tail {
                    BlockTail::Normal(slot) => slot,
                    BlockTail::ScopeFailure(_) => {
                        return Err(CertifyError::Certification(
                            "expanded fragment has no normal success source".to_owned(),
                        ));
                    }
                };
                let expected_success = LexicalSlot {
                    lexical_path: fragment.path.clone(),
                    contract_ref: body_success.contract_ref.clone(),
                    producer: LexicalProducer::FragmentBoundary {
                        boundary_id: fragment.boundary_id.clone(),
                        role: ResultRole::SuccessOutput,
                        source: Box::new(body_success.clone()),
                    },
                };
                if fragment.success_slot != expected_success {
                    return Err(CertifyError::Certification(
                        "expanded fragment success source substitution".to_owned(),
                    ));
                }
                validate_fragment_failure_source(fragment)?;
                validate_boundary_lexical(
                    &fragment.failure_boundary,
                    visible,
                    &fragment.semantic_call_id,
                )?;
                expected_failure_exits
                    .extend(fragment_failure_scope_exits(&fragment.failure_boundary));
                visible.insert(&fragment.success_slot)?;
            }
        }
    }
    let tail = match &block.tail {
        BlockTail::Normal(slot) | BlockTail::ScopeFailure(slot) => slot,
    };
    if !visible.contains(tail)? {
        return Err(CertifyError::Certification(
            "expanded block tail is not an exact dominating lexical slot".to_owned(),
        ));
    }
    if let BlockTail::ScopeFailure(slot) = &block.tail {
        expected_failure_exits.push(slot.clone());
    }
    dedup_slots(&mut expected_failure_exits)?;
    let scope_failure_contract_ref = block.failure_scope.failure_contract().contract_ref()?;
    if expected_failure_exits
        .iter()
        .any(|slot| slot.contract_ref != scope_failure_contract_ref)
        || (matches!(
            block.failure_scope.failure_contract(),
            StructuredFailureContract::Never
        ) && !expected_failure_exits.is_empty())
    {
        return Err(CertifyError::Certification(
            "expanded failure exit differs from its exact enclosing scope contract".to_owned(),
        ));
    }
    if expected_failure_exits != block.failure_exits {
        return Err(CertifyError::Certification(
            "expanded block failure-exit closure mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn expanded_declaration_label(declaration: &ExpandedDeclaration) -> &StableId {
    match declaration {
        ExpandedDeclaration::State(state) => &state.label,
        ExpandedDeclaration::Match(binding) => &binding.label,
        ExpandedDeclaration::FanOut(group) => &group.label,
        ExpandedDeclaration::Fragment(fragment) => &fragment.label,
    }
}

fn fragment_path_extends_declaration(
    fragment_path: &StructuralPath,
    declaration_path: &StructuralPath,
) -> bool {
    let fragment = fragment_path.segments();
    let declaration = declaration_path.segments();
    declaration.len() < fragment.len()
        && declaration
            .iter()
            .zip(fragment)
            .all(|(expected, actual)| expected == actual)
        && fragment[declaration.len()..]
            .iter()
            .all(|segment| matches!(segment, StructuralPathSegment::Fragment { .. }))
}

fn validate_fragment_failure_source(fragment: &ExpandedFragment) -> Result<()> {
    match &fragment.failure_boundary {
        CertifiedFailureBoundary::NoFailure(_) => {
            if !fragment.body.failure_exits.is_empty() {
                return Err(CertifyError::Certification(
                    "Never fragment body exposes a typed failure exit".to_owned(),
                ));
            }
        }
        CertifiedFailureBoundary::Typed {
            failure_contract,
            source_slot,
            ..
        } => {
            let expected_contract = failure_contract.contract_ref()?;
            if source_slot.lexical_path != fragment.path
                || source_slot.contract_ref != expected_contract
            {
                return Err(CertifyError::Certification(
                    "typed fragment failure boundary path or contract substitution".to_owned(),
                ));
            }
            let inner = match &source_slot.producer {
                LexicalProducer::FragmentBoundary {
                    boundary_id,
                    role: ResultRole::TypedFailure,
                    source,
                } if boundary_id == &fragment.boundary_id => source.as_ref(),
                _ => {
                    return Err(CertifyError::Certification(
                        "typed fragment failure source is not its exact boundary producer"
                            .to_owned(),
                    ));
                }
            };
            let is_direct = fragment
                .body
                .failure_exits
                .iter()
                .any(|slot| slot == source_slot);
            let is_merge = matches!(
                &inner.producer,
                LexicalProducer::ScopeFailureMerge {
                    scope_id,
                    declaration_ordered_failure_slots,
                } if scope_id == fragment.body.failure_scope.scope_id()
                    && inner.lexical_path == fragment.body.path
                    && inner.contract_ref == expected_contract
                    && declaration_ordered_failure_slots == &fragment.body.failure_exits
            );
            if !is_direct && !is_merge {
                return Err(CertifyError::Certification(
                    "typed fragment failure source is not its exact body failure channel"
                        .to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_boundary_lexical(
    boundary: &CertifiedFailureBoundary,
    outer_visible: &VisibleSlots,
    source_semantic_call_id: &SemanticCallId,
) -> Result<()> {
    let CertifiedFailureBoundary::Typed {
        source_slot, plan, ..
    } = boundary
    else {
        return Ok(());
    };
    match plan.as_ref() {
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            ..
        } => {
            let mut plan_visible = outer_visible.clone();
            plan_visible.insert(source_slot)?;
            let mut prefix = before_boundary.as_ref().clone();
            prefix.tail = BlockTail::Normal(source_slot.clone());
            validate_block_lexical(&prefix, &mut plan_visible)?;

            let mut labels = prefix
                .declarations
                .iter()
                .map(|declaration| expanded_declaration_label(declaration).clone())
                .collect::<BTreeSet<_>>();
            for (offset, link) in mapping_chain.iter().enumerate() {
                let ordinal = prefix
                    .declarations
                    .len()
                    .checked_add(offset)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        CertifyError::Certification(
                            "failure mapping declaration count exceeds u32".to_owned(),
                        )
                    })?;
                let expected_path =
                    declaration_path(&before_boundary.path, &link.mapper.label, ordinal)?;
                if !labels.insert(link.mapper.label.clone())
                    || link.mapper.occurrence_path != expected_path
                    || !all_slots_visible(&link.mapper.inputs, &plan_visible)?
                {
                    return Err(CertifyError::Certification(
                        "failure mapping is duplicated, reordered, or non-dominating".to_owned(),
                    ));
                }
                validate_boundary_lexical(
                    &link.mapper.failure_boundary,
                    &plan_visible,
                    &link.mapper.semantic_call_id,
                )?;
                if !failure_boundary_scope_exits(&link.mapper.failure_boundary).is_empty() {
                    return Err(CertifyError::Certification(
                        "failure mapping state exposes a failure continuation".to_owned(),
                    ));
                }
                plan_visible.insert(&link.mapper.output_slot)?;
            }
            let BlockTail::Normal(mapped_tail) = &before_boundary.tail else {
                return Err(CertifyError::Certification(
                    "failure propagation has a non-normal mapped tail".to_owned(),
                ));
            };
            if !plan_visible.contains(mapped_tail)? {
                return Err(CertifyError::Certification(
                    "failure propagation mapped tail is not produced by its affine chain"
                        .to_owned(),
                ));
            }
            Ok(())
        }
        FailurePlan::Handled {
            before_handler,
            handler,
            continuation,
            ..
        } => {
            let mut plan_visible = outer_visible.clone();
            plan_visible.insert(source_slot)?;
            validate_block_lexical(before_handler, &mut plan_visible)?;
            if handler.semantic_call_id
                != failure_handler_semantic_call_id(source_semantic_call_id)?
                || !all_slots_visible(&handler.inputs, &plan_visible)?
            {
                return Err(CertifyError::Certification(
                    "failure handler input is not the exact pre-handler result".to_owned(),
                ));
            }
            plan_visible.insert(&handler.output_slot)?;
            if let HandlerContinuation::CustomRecovery {
                route_contract,
                arms,
                ..
            } = continuation.as_ref()
            {
                for (arm, variant) in arms.iter().zip(&route_contract.variants) {
                    let mut arm_visible = plan_visible.clone();
                    for payload in &variant.payloads {
                        arm_visible.insert(&LexicalSlot {
                            lexical_path: arm.path.clone(),
                            contract_ref: payload.contract_ref.clone(),
                            producer: LexicalProducer::VariantPayload {
                                selector: Box::new(handler.output_slot.clone()),
                                canonical_tag: arm.canonical_tag.clone(),
                                payload_path: payload.payload_path.clone(),
                            },
                        })?;
                    }
                    validate_block_lexical(&arm.body, &mut arm_visible)?;
                }
            }
            Ok(())
        }
    }
}

fn all_slots_visible(slots: &[LexicalSlot], visible: &VisibleSlots) -> Result<bool> {
    for slot in slots {
        if !visible.contains(slot)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_eventual_failure_handlers(expanded: &ExpandedStructuredProgram) -> Result<()> {
    let mut plans = BTreeMap::new();
    collect_failure_plans(&expanded.root, &mut plans)?;
    for source in plans.keys() {
        let mut current = source.clone();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return Err(CertifyError::Certification(
                    "failure propagation contains a cycle".to_owned(),
                ));
            }
            let plan = plans.get(&current).ok_or_else(|| {
                CertifyError::Certification(
                    "failure propagation has no exact eventual plan".to_owned(),
                )
            })?;
            match plan {
                FailurePlan::Handled { .. } => break,
                FailurePlan::Propagate { boundary_slot, .. } => {
                    current = boundary_slot.as_ref().clone();
                }
            }
        }
    }
    Ok(())
}

fn collect_failure_plans(
    block: &ExpandedBlock,
    plans: &mut BTreeMap<LexicalSlot, FailurePlan>,
) -> Result<()> {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state) => {
                collect_boundary_plan(&state.failure_boundary, plans)?;
            }
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    collect_failure_plans(&arm.body, plans)?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    collect_failure_plans(&lane.body, plans)?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                collect_failure_plans(&fragment.body, plans)?;
                collect_boundary_plan(&fragment.failure_boundary, plans)?;
            }
        }
    }
    Ok(())
}

fn collect_boundary_plan(
    boundary: &CertifiedFailureBoundary,
    plans: &mut BTreeMap<LexicalSlot, FailurePlan>,
) -> Result<()> {
    let CertifiedFailureBoundary::Typed {
        source_slot, plan, ..
    } = boundary
    else {
        return Ok(());
    };
    if plans
        .insert(source_slot.clone(), plan.as_ref().clone())
        .is_some()
    {
        return Err(CertifyError::Certification(
            "typed failure source has duplicate certified plans".to_owned(),
        ));
    }
    match plan.as_ref() {
        FailurePlan::Handled {
            before_handler,
            continuation,
            ..
        } => {
            collect_failure_plans(before_handler, plans)?;
            if let HandlerContinuation::CustomRecovery { arms, .. } = continuation.as_ref() {
                for arm in arms {
                    collect_failure_plans(&arm.body, plans)?;
                }
            }
        }
        FailurePlan::Propagate {
            before_boundary, ..
        } => collect_failure_plans(before_boundary, plans)?,
    }
    Ok(())
}

#[derive(Default)]
struct ProgramMetrics {
    occurrences: usize,
    declarations: usize,
    lanes: usize,
    branch_depth: usize,
    occurrence_ids: BTreeSet<OccurrenceId>,
    semantic_call_ids: BTreeSet<SemanticCallId>,
}

fn record_occurrence(metrics: &mut ProgramMetrics, state: &ExpandedStateBinding) -> Result<()> {
    metrics.occurrences += 1;
    if !metrics.occurrence_ids.insert(state.occurrence_id.clone())
        || !metrics
            .semantic_call_ids
            .insert(state.semantic_call_id.clone())
    {
        return Err(CertifyError::Certification(
            "expanded executable state identity is duplicated".to_owned(),
        ));
    }
    Ok(())
}

fn validate_expanded_block(
    block: &ExpandedBlock,
    registry: &StructuredCertificationRegistry,
    metrics: &mut ProgramMetrics,
    branch_depth: usize,
    remaining_fan_out_depth: u8,
    inside_fan_out: bool,
) -> Result<()> {
    block
        .failure_scope
        .failure_contract()
        .validate()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    if let FailureScopeBinding::Owns { scope } = &block.failure_scope {
        if matches!(scope.failure_contract, StructuredFailureContract::Never)
            && !scope.default_mappers.is_empty()
        {
            return Err(CertifyError::Certification(
                "expanded Never scope cannot own a default failure map".to_owned(),
            ));
        }
        let mut source_contracts = BTreeSet::new();
        for mapper in &scope.default_mappers {
            if !source_contracts.insert(mapper.source_failure_contract_ref.clone()) {
                return Err(CertifyError::Certification(
                    "expanded failure scope repeats an exact default mapper".to_owned(),
                ));
            }
            let contract = registry
                .state_contract(&mapper.mapper_state_contract_ref)
                .ok_or_else(|| {
                    CertifyError::Certification(
                        "expanded default mapper is not process-qualified".to_owned(),
                    )
                })?;
            validate_handler_contract(contract, &mapper.source_failure_contract_ref)?;
            validate_registered_closed_sum(&mapper.route_contract, registry)?;
            if mapper.route_contract.selector_contract_ref != contract.output_contract_ref
                || mapper.route_contract.variants.len() != 1
                || mapper.route_contract.variants[0].canonical_tag != "propagate"
                || mapper.route_contract.variants[0].payloads.len() != 1
                || mapper.route_contract.variants[0].payloads[0].contract_ref
                    != scope.failure_contract.contract_ref()?
            {
                return Err(CertifyError::Certification(
                    "expanded default mapper does not exactly propagate its scope contract"
                        .to_owned(),
                ));
            }
        }
    }
    metrics.branch_depth = metrics.branch_depth.max(branch_depth);
    for declaration in &block.declarations {
        metrics.declarations += 1;
        match declaration {
            ExpandedDeclaration::State(state) => {
                record_occurrence(metrics, state)?;
                validate_state_binding(state, registry, inside_fan_out)?;
                validate_failure_boundary_paths(
                    &state.failure_boundary,
                    registry,
                    metrics,
                    branch_depth,
                    remaining_fan_out_depth,
                    inside_fan_out,
                )?;
            }
            ExpandedDeclaration::Match(binding) => {
                validate_registered_closed_sum(&binding.selector_contract, registry)?;
                let expected: Vec<&str> = binding
                    .selector_contract
                    .variants
                    .iter()
                    .map(|variant| variant.canonical_tag.as_str())
                    .collect();
                let actual: Vec<&str> = binding
                    .arms
                    .iter()
                    .map(|arm| arm.canonical_tag.as_str())
                    .collect();
                if expected != actual
                    || binding.selector.contract_ref
                        != binding.selector_contract.selector_contract_ref
                {
                    return Err(CertifyError::Certification(
                        "expanded Match is not exact and exhaustive".to_owned(),
                    ));
                }
                for arm in &binding.arms {
                    validate_expanded_block(
                        &arm.body,
                        registry,
                        metrics,
                        branch_depth + 1,
                        remaining_fan_out_depth,
                        inside_fan_out,
                    )?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                if remaining_fan_out_depth == 0 || group.lanes.is_empty() {
                    return Err(CertifyError::Certification(
                        "expanded FanOut exceeds depth or is empty".to_owned(),
                    ));
                }
                metrics.lanes += group.lanes.len();
                for (ordinal, lane) in group.lanes.iter().enumerate() {
                    if usize::try_from(lane.declaration_ordinal).ok() != Some(ordinal) {
                        return Err(CertifyError::Certification(
                            "fan-out lane order is not dense".to_owned(),
                        ));
                    }
                    validate_expanded_block(
                        &lane.body,
                        registry,
                        metrics,
                        branch_depth + 1,
                        remaining_fan_out_depth - 1,
                        true,
                    )?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                validate_expanded_block(
                    &fragment.body,
                    registry,
                    metrics,
                    branch_depth + 1,
                    remaining_fan_out_depth,
                    inside_fan_out,
                )?;
                validate_failure_boundary(
                    &fragment.failure_boundary,
                    &fragment.semantic_call_id,
                    registry,
                )?;
                validate_failure_boundary_paths(
                    &fragment.failure_boundary,
                    registry,
                    metrics,
                    branch_depth + 1,
                    remaining_fan_out_depth,
                    inside_fan_out,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_failure_boundary_paths(
    boundary: &CertifiedFailureBoundary,
    registry: &StructuredCertificationRegistry,
    metrics: &mut ProgramMetrics,
    branch_depth: usize,
    remaining_fan_out_depth: u8,
    inside_fan_out: bool,
) -> Result<()> {
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
            validate_expanded_block(
                before_handler,
                registry,
                metrics,
                branch_depth,
                remaining_fan_out_depth,
                inside_fan_out,
            )?;
            record_occurrence(metrics, handler)?;
            metrics.declarations += 1;
            validate_state_binding(handler, registry, inside_fan_out)?;
            if let HandlerContinuation::CustomRecovery { arms, .. } = continuation.as_ref() {
                for arm in arms {
                    validate_expanded_block(
                        &arm.body,
                        registry,
                        metrics,
                        branch_depth + 1,
                        remaining_fan_out_depth,
                        inside_fan_out,
                    )?;
                }
            }
        }
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            ..
        } => {
            validate_expanded_block(
                before_boundary,
                registry,
                metrics,
                branch_depth,
                remaining_fan_out_depth,
                inside_fan_out,
            )?;
            for link in mapping_chain {
                record_occurrence(metrics, &link.mapper)?;
                metrics.declarations += 1;
                validate_state_binding(&link.mapper, registry, inside_fan_out)?;
            }
        }
    }
    Ok(())
}

fn validate_state_binding(
    state: &ExpandedStateBinding,
    registry: &StructuredCertificationRegistry,
    inside_fan_out: bool,
) -> Result<()> {
    let output_is_exact = matches!(
        &state.output_slot.producer,
        LexicalProducer::StateOutput {
            occurrence_id,
            role: ResultRole::SuccessOutput,
        } if occurrence_id == &state.occurrence_id
    ) && state.output_slot.lexical_path == state.occurrence_path
        && state.output_slot.contract_ref == state.contract.output_contract_ref;
    let failure_boundary_is_exact =
        match (&state.contract.failure_contract, &state.failure_boundary) {
            (StructuredFailureContract::Never, CertifiedFailureBoundary::NoFailure(_)) => true,
            (
                StructuredFailureContract::Typed { contract_ref, .. },
                CertifiedFailureBoundary::Typed {
                    failure_contract,
                    source_slot,
                    ..
                },
            ) => {
                failure_contract.as_ref() == &state.contract.failure_contract
                    && source_slot.lexical_path == state.occurrence_path
                    && source_slot.contract_ref == *contract_ref
                    && matches!(
                        &source_slot.producer,
                        LexicalProducer::StateOutput {
                            occurrence_id,
                            role: ResultRole::TypedFailure,
                        } if occurrence_id == &state.occurrence_id
                    )
            }
            _ => false,
        };
    if state.occurrence_path.occurrence_id()? != state.occurrence_id
        || registry.state_contract(&state.contract.state_contract_ref) != Some(&state.contract)
        || state.contract.capability_requirement_ref.is_some()
        || state.inputs.len() != 1
        || state.inputs[0].contract_ref != state.contract.input_contract_ref
        || !output_is_exact
        || !failure_boundary_is_exact
        || (inside_fan_out
            && matches!(
                state.contract.execution,
                StructuredStateExecutionContract::Effect { .. }
            ))
    {
        return Err(CertifyError::Certification(
            "expanded state identity, registry, or fan-out policy mismatch".to_owned(),
        ));
    }
    validate_failure_boundary(&state.failure_boundary, &state.semantic_call_id, registry)
}

fn validate_failure_boundary(
    boundary: &CertifiedFailureBoundary,
    source_semantic_call_id: &SemanticCallId,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    match boundary {
        CertifiedFailureBoundary::NoFailure(marker) => {
            if marker.never_contract_ref != never_failure_contract_ref()? {
                return Err(CertifyError::Certification(
                    "Never marker substitution".to_owned(),
                ));
            }
        }
        CertifiedFailureBoundary::Typed {
            failure_contract,
            source_slot,
            plan,
        } => {
            if source_slot.contract_ref != failure_contract.contract_ref()? {
                return Err(CertifyError::Certification(
                    "failure source contract substitution".to_owned(),
                ));
            }
            validate_failure_plan(plan, source_semantic_call_id, source_slot, registry)?;
        }
    }
    Ok(())
}

fn validate_failure_plan(
    plan: &FailurePlan,
    source_semantic_call_id: &SemanticCallId,
    source_slot: &LexicalSlot,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    match plan {
        FailurePlan::Handled {
            plan_id,
            plan_path,
            source_slot: plan_source,
            before_handler,
            handler,
            continuation,
        } => {
            validate_plan_identity(
                plan_id,
                plan_path,
                source_semantic_call_id,
                source_slot,
                "handler",
            )?;
            let before_target = normal_tail_slot(before_handler)?;
            if plan_source != source_slot
                || before_handler.path != *plan_path
                || before_target != *source_slot
                || handler.inputs.as_slice() != [before_target.clone()]
                || handler.semantic_call_id
                    != failure_handler_semantic_call_id(source_semantic_call_id)?
                || handler.occurrence_path
                    != plan_path.child(StructuralPathSegment::Declaration {
                        label: stable_id("handler")?,
                        ordinal: u32::try_from(before_handler.declarations.len()).map_err(
                            |_| {
                                CertifyError::Certification(
                                    "pre-handler declaration count exceeds u32".to_owned(),
                                )
                            },
                        )?,
                    })?
                || !pre_handler_has_only_normal_control(before_handler)
            {
                return Err(CertifyError::Certification(
                    "handled failure plan source, pre-handler, or handler substitution".to_owned(),
                ));
            }
            validate_handler_contract(&handler.contract, &before_target.contract_ref)?;
            validate_state_binding(handler, registry, false)?;
            let route_contract = match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation { route_contract, .. }
                | HandlerContinuation::CustomRecovery { route_contract, .. } => route_contract,
            };
            validate_registered_closed_sum(route_contract, registry)?;
            match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation {
                    handler_output_slot,
                    route_contract,
                    payload_slot,
                    failure_tail,
                } if before_handler.declarations.is_empty()
                    && handler_output_slot == &handler.output_slot
                    && payload_slot == failure_tail
                    && route_contract.selector_contract_ref == handler.output_slot.contract_ref
                    && route_contract.variants.len() == 1
                    && route_contract.variants[0].canonical_tag == "propagate"
                    && route_contract.variants[0].payloads.len() == 1
                    && route_contract.variants[0].payloads[0].contract_ref
                        == payload_slot.contract_ref
                    && matches!(
                        &payload_slot.producer,
                        LexicalProducer::VariantPayload {
                            selector,
                            canonical_tag,
                            payload_path,
                        } if selector.as_ref() == &handler.output_slot
                            && canonical_tag == "propagate"
                            && payload_path
                                == &route_contract.variants[0].payloads[0].payload_path
                    ) => {}
                HandlerContinuation::CustomRecovery {
                    handler_output_slot,
                    route_contract,
                    arms,
                } if handler_output_slot == &handler.output_slot
                    && route_contract.selector_contract_ref == handler.output_slot.contract_ref
                    && route_contract
                        .variants
                        .iter()
                        .map(|variant| variant.canonical_tag.as_str())
                        .eq(arms.iter().map(|arm| arm.canonical_tag.as_str())) => {}
                _ => {
                    return Err(CertifyError::Certification(
                        "failure handler continuation substitution".to_owned(),
                    ));
                }
            }
        }
        FailurePlan::Propagate {
            plan_id,
            plan_path,
            source_slot: plan_source,
            before_boundary,
            mapping_chain,
            boundary_id,
            boundary_slot,
        } => {
            validate_plan_identity(
                plan_id,
                plan_path,
                source_semantic_call_id,
                source_slot,
                "propagate",
            )?;
            let mapped_target = validate_mapping_chain(
                plan_id,
                plan_path,
                source_slot,
                mapping_chain,
                before_boundary.declarations.len(),
                &before_boundary.failure_scope,
                registry,
            )?;
            if plan_source != source_slot {
                return Err(CertifyError::Certification(
                    "affine propagation substituted its protected source".to_owned(),
                ));
            }
            if before_boundary.path != *plan_path {
                return Err(CertifyError::Certification(
                    "affine propagation changed its plan path".to_owned(),
                ));
            }
            if normal_tail_slot(before_boundary)? != mapped_target {
                return Err(CertifyError::Certification(
                    "affine propagation tail differs from its mapped target".to_owned(),
                ));
            }
            if !pre_handler_has_only_normal_control(before_boundary) {
                return Err(CertifyError::Certification(
                    "affine propagation pre-boundary structure escapes normal control".to_owned(),
                ));
            }
            if boundary_slot.contract_ref != mapped_target.contract_ref {
                return Err(CertifyError::Certification(
                    "affine propagation changes its mapped target contract".to_owned(),
                ));
            }
            let exact_boundary_path = enclosing_fragment_path(source_slot)?;
            if boundary_slot.lexical_path != exact_boundary_path
                || exact_boundary_path.fragment_boundary_id()? != *boundary_id
            {
                return Err(CertifyError::Certification(
                    "affine propagation boundary is not the exact enclosing fragment".to_owned(),
                ));
            }
            if !matches!(
                &boundary_slot.producer,
                LexicalProducer::FragmentBoundary {
                    boundary_id: producer,
                    role: ResultRole::TypedFailure,
                    source,
                } if producer == boundary_id
                    && propagation_source_contains(source, &mapped_target)
            ) {
                return Err(CertifyError::Certification(
                    "affine propagation target is not its exact fragment-boundary rebind"
                        .to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn enclosing_fragment_path(source_slot: &LexicalSlot) -> Result<StructuralPath> {
    let source_segments = source_slot.lexical_path.segments();
    let search_end = match &source_slot.producer {
        LexicalProducer::StateOutput {
            role: ResultRole::TypedFailure,
            ..
        } => source_segments.len(),
        LexicalProducer::FragmentBoundary {
            boundary_id,
            role: ResultRole::TypedFailure,
            ..
        } if matches!(
            source_segments.last(),
            Some(StructuralPathSegment::Fragment { .. })
        ) && source_slot.lexical_path.fragment_boundary_id()? == *boundary_id =>
        {
            source_segments.len().saturating_sub(1)
        }
        _ => {
            return Err(CertifyError::Certification(
                "typed propagation source is not an exact state or fragment failure".to_owned(),
            ));
        }
    };
    let fragment_ordinal = source_segments[..search_end]
        .iter()
        .rposition(|segment| matches!(segment, StructuralPathSegment::Fragment { .. }))
        .ok_or_else(|| {
            CertifyError::Certification(
                "typed propagation source is outside an expanded fragment".to_owned(),
            )
        })?;
    StructuralPath::new(source_segments[..=fragment_ordinal].to_vec())
        .map_err(|error| CertifyError::Certification(error.to_string()))
}

fn propagation_source_contains(source: &LexicalSlot, mapped_target: &LexicalSlot) -> bool {
    if source == mapped_target {
        return true;
    }
    match &source.producer {
        LexicalProducer::FragmentBoundary {
            role: ResultRole::TypedFailure,
            source,
            ..
        } => propagation_source_contains(source, mapped_target),
        LexicalProducer::ScopeFailureMerge {
            declaration_ordered_failure_slots,
            ..
        } => declaration_ordered_failure_slots
            .iter()
            .any(|slot| propagation_source_contains(slot, mapped_target)),
        _ => false,
    }
}

fn validate_plan_identity(
    plan_id: &mfm_ids::FailurePlanId,
    plan_path: &StructuralPath,
    source_semantic_call_id: &SemanticCallId,
    source_slot: &LexicalSlot,
    plan_label: &str,
) -> Result<()> {
    let canonical_plan_path =
        source_slot
            .lexical_path
            .child(StructuralPathSegment::FailurePlan {
                label: stable_id(plan_label)?,
            })?;
    if plan_path != &canonical_plan_path {
        return Err(CertifyError::Certification(
            "failure plan path is not the canonical child of its exact source".to_owned(),
        ));
    }
    let expected = FailurePlanIdentity {
        source_semantic_call_id: source_semantic_call_id.clone(),
        source_slot: source_slot.clone(),
        plan_path: canonical_plan_path,
    }
    .derive()
    .map_err(|error| CertifyError::Certification(error.to_string()))?;
    if &expected != plan_id {
        return Err(CertifyError::Certification(
            "failure plan identity is not derived from its exact source".to_owned(),
        ));
    }
    Ok(())
}

fn validate_mapping_chain(
    plan_id: &mfm_ids::FailurePlanId,
    plan_path: &StructuralPath,
    source_slot: &LexicalSlot,
    links: &[mfm_spec::structured::FailureMappingLink],
    declaration_ordinal_base: usize,
    failure_scope: &FailureScopeBinding,
    registry: &StructuredCertificationRegistry,
) -> Result<LexicalSlot> {
    let mut current = source_slot.clone();
    for (offset, link) in links.iter().enumerate() {
        let ordinal = declaration_ordinal_base
            .checked_add(offset)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                CertifyError::Certification(
                    "failure mapping declaration count exceeds u32".to_owned(),
                )
            })?;
        let expected_path = declaration_path(plan_path, &link.mapper.label, ordinal)?;
        let is_lexical_default_mapper = matches!(
            failure_scope,
            FailureScopeBinding::Owns { scope }
                if scope.default_mappers.iter().any(|registration| {
                    registration.mapper_state_contract_ref
                        == link.mapper.contract.state_contract_ref
                })
        );
        if &link.plan_id != plan_id
            || link.input_slot != current
            || link.mapper.inputs.as_slice() != [current.clone()]
            || link.output_slot != link.mapper.output_slot
            || link.mapper.occurrence_path != expected_path
            || is_lexical_default_mapper
        {
            return Err(CertifyError::Certification(
                "affine failure mapping chain breaks source, plan, or lexical adjacency".to_owned(),
            ));
        }
        validate_handler_contract(&link.mapper.contract, &current.contract_ref)?;
        validate_state_binding(&link.mapper, registry, false)?;
        current = link.output_slot.clone();
    }
    Ok(current)
}

fn pre_handler_has_only_normal_control(block: &ExpandedBlock) -> bool {
    if !matches!(block.tail, BlockTail::Normal(_)) {
        return false;
    }
    block
        .declarations
        .iter()
        .all(|declaration| match declaration {
            ExpandedDeclaration::Match(binding) => binding
                .arms
                .iter()
                .all(|arm| pre_handler_has_only_normal_control(&arm.body)),
            ExpandedDeclaration::State(_)
            | ExpandedDeclaration::FanOut(_)
            | ExpandedDeclaration::Fragment(_) => true,
        })
}

fn validate_policy_coverage(
    semantic_contracts: &[(SemanticCallId, ContentRef)],
    profile: &StructuredExpansionProfile,
    proof: &StructuredPolicyCoverageProof,
) -> Result<()> {
    let mut expected = Vec::new();
    for (semantic_call_id, state_contract_ref) in semantic_contracts {
        for (ordinal, policy) in profile.policies.iter().enumerate() {
            if policy.recipe_for(state_contract_ref).is_some() {
                expected.push(PolicyCoverageEntry {
                    semantic_call_id: semantic_call_id.clone(),
                    profile_ordinal: ordinal as u32,
                    policy_ref: policy.policy_ref.clone(),
                });
            }
        }
    }
    if expected != proof.entries {
        return Err(CertifyError::Certification(
            "policy coverage is missing, duplicate, foreign, or reordered".to_owned(),
        ));
    }
    Ok(())
}

fn component_manifests(
    expanded: &ExpandedStructuredProgram,
    registry: &StructuredCertificationRegistry,
) -> Result<(
    StateCapabilityAdapterSignerResourceManifest,
    SecretFreeImplementationManifest,
)> {
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    collect_component_requirements(&expanded.root, registry, &mut entries, &mut seen)?;
    let implementations = entries
        .iter()
        .map(|semantic_contract_ref| {
            let implementation_contract_ref = registry
                .component_implementations
                .get(&(
                    semantic_contract_ref.component_kind,
                    semantic_contract_ref.semantic_contract_ref.clone(),
                ))
                .cloned()
                .ok_or_else(|| {
                    CertifyError::Certification(
                        "semantic component lacks one qualified implementation binding".to_owned(),
                    )
                })?;
            Ok(SecretFreeImplementationManifestEntry {
                component_kind: semantic_contract_ref.component_kind,
                semantic_contract_ref: semantic_contract_ref.semantic_contract_ref.clone(),
                implementation_contract_ref,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        StateCapabilityAdapterSignerResourceManifest { entries },
        SecretFreeImplementationManifest {
            entries: implementations,
        },
    ))
}

fn collect_component_requirements(
    block: &ExpandedBlock,
    registry: &StructuredCertificationRegistry,
    out: &mut Vec<StructuredComponentManifestEntry>,
    seen: &mut BTreeSet<(StructuredComponentKind, ContentRef)>,
) -> Result<()> {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state) => {
                push_component_requirement(
                    StructuredComponentKind::State,
                    &state.contract.state_contract_ref,
                    out,
                    seen,
                );
                if let Some(capability_contract_ref) =
                    state.contract.execution.capability_contract_ref()
                {
                    collect_live_component(
                        StructuredComponentKind::Capability,
                        capability_contract_ref,
                        registry,
                        out,
                        seen,
                    )?;
                }
                collect_failure_requirements(&state.failure_boundary, registry, out, seen)?;
            }
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    collect_component_requirements(&arm.body, registry, out, seen)?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    collect_component_requirements(&lane.body, registry, out, seen)?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                collect_component_requirements(&fragment.body, registry, out, seen)?;
                collect_failure_requirements(&fragment.failure_boundary, registry, out, seen)?;
            }
        }
    }
    Ok(())
}

fn collect_failure_requirements(
    boundary: &CertifiedFailureBoundary,
    registry: &StructuredCertificationRegistry,
    out: &mut Vec<StructuredComponentManifestEntry>,
    seen: &mut BTreeSet<(StructuredComponentKind, ContentRef)>,
) -> Result<()> {
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
            collect_component_requirements(before_handler, registry, out, seen)?;
            push_component_requirement(
                StructuredComponentKind::State,
                &handler.contract.state_contract_ref,
                out,
                seen,
            );
            if let HandlerContinuation::CustomRecovery { arms, .. } = continuation.as_ref() {
                for arm in arms {
                    collect_component_requirements(&arm.body, registry, out, seen)?;
                }
            }
        }
        FailurePlan::Propagate {
            before_boundary,
            mapping_chain,
            ..
        } => {
            collect_component_requirements(before_boundary, registry, out, seen)?;
            for link in mapping_chain {
                push_component_requirement(
                    StructuredComponentKind::State,
                    &link.mapper.contract.state_contract_ref,
                    out,
                    seen,
                );
            }
        }
    }
    Ok(())
}

fn push_component_requirement(
    component_kind: StructuredComponentKind,
    contract_ref: &ContentRef,
    out: &mut Vec<StructuredComponentManifestEntry>,
    seen: &mut BTreeSet<(StructuredComponentKind, ContentRef)>,
) {
    if seen.insert((component_kind, contract_ref.clone())) {
        out.push(StructuredComponentManifestEntry {
            component_kind,
            semantic_contract_ref: contract_ref.clone(),
        });
    }
}

fn collect_live_component(
    component_kind: StructuredComponentKind,
    contract_ref: &ContentRef,
    registry: &StructuredCertificationRegistry,
    out: &mut Vec<StructuredComponentManifestEntry>,
    seen: &mut BTreeSet<(StructuredComponentKind, ContentRef)>,
) -> Result<()> {
    if !seen.insert((component_kind, contract_ref.clone())) {
        return Ok(());
    }
    let contract = registry
        .live_components
        .get(&(component_kind, contract_ref.clone()))
        .ok_or_else(|| {
            CertifyError::Certification(
                "live semantic component dependency is not process-qualified".to_owned(),
            )
        })?;
    contract
        .validate()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    if contract.component_kind != component_kind || contract.content_ref()? != *contract_ref {
        return Err(CertifyError::Certification(
            "live semantic component kind or contract identity mismatch".to_owned(),
        ));
    }
    out.push(StructuredComponentManifestEntry {
        component_kind,
        semantic_contract_ref: contract_ref.clone(),
    });
    for dependency in &contract.dependencies {
        collect_live_component(
            dependency.component_kind,
            &dependency.contract_ref,
            registry,
            out,
            seen,
        )?;
    }
    Ok(())
}

fn authored_component_references(
    program: &AuthoredStructuredProgram,
) -> Result<Vec<ComponentObjectReference>> {
    let mut references = Vec::new();
    for root in &program.input_roots {
        collect_slot_contract_references(root, &mut references)?;
    }
    push_contract_reference(&mut references, &program.output_contract_ref)?;
    collect_failure_contract_reference(&program.failure_contract, &mut references)?;
    collect_authored_block_references(&program.root, &mut references)?;
    Ok(references)
}

fn policy_recipe_component_references(
    recipe: &PolicyExpansionRecipe,
) -> Result<Vec<ComponentObjectReference>> {
    let proceed_ref = policy_proceed_program_ref()?;
    let authored_type = stable_id(AUTHORED_OBJECT_TYPE)?;
    let mut removed = 0usize;
    let mut references = authored_component_references(recipe.program())?
        .into_iter()
        .filter(|reference| {
            let is_proceed =
                reference.object_type == authored_type && reference.content_ref == proceed_ref;
            removed += usize::from(is_proceed);
            !is_proceed
        })
        .collect::<Vec<_>>();
    if removed != 1 {
        return Err(CertifyError::Certification(
            "policy recipe does not contain exactly one proceed reference".to_owned(),
        ));
    }
    if let Some(failure_post) = recipe.failure_post() {
        references.extend(authored_component_references(failure_post)?);
    }
    Ok(references)
}

fn collect_authored_block_references(
    block: &AuthoredBlock,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    collect_structural_path_references(&block.path, references)?;
    collect_failure_scope_references(&block.failure_scope, references)?;
    for declaration in &block.declarations {
        match declaration {
            AuthoredDeclaration::State(state) => {
                collect_state_contract_references(&state.contract, references)?;
                for input in &state.inputs {
                    collect_slot_contract_references(input, references)?;
                }
                collect_authored_failure_directive_references(
                    &state.failure_directive,
                    references,
                )?;
                collect_slot_contract_references(&state.output_slot, references)?;
            }
            AuthoredDeclaration::OperationCall(call) => {
                push_typed_reference(references, AUTHORED_OBJECT_TYPE, &call.child_program_ref)?;
                for binding in &call.input_bindings {
                    push_contract_reference(references, &binding.child_contract_ref)?;
                    collect_slot_contract_references(&binding.caller_slot, references)?;
                }
                push_contract_reference(references, &call.output_contract_ref)?;
                collect_failure_contract_reference(&call.failure_contract, references)?;
                collect_authored_failure_directive_references(&call.failure_directive, references)?;
                collect_slot_contract_references(&call.output_slot, references)?;
            }
            AuthoredDeclaration::Match(binding) => {
                collect_slot_contract_references(&binding.selector, references)?;
                collect_closed_sum_references(&binding.selector_contract, references)?;
                for arm in &binding.arms {
                    collect_authored_block_references(&arm.body, references)?;
                }
                collect_slot_contract_references(&binding.output_slot, references)?;
            }
            AuthoredDeclaration::FanOut(group) => {
                push_contract_reference(references, &group.lane_output_contract_ref)?;
                collect_failure_contract_reference(&group.lane_failure_contract, references)?;
                for lane in &group.lanes {
                    collect_authored_block_references(&lane.body, references)?;
                }
                collect_slot_contract_references(&group.output_slot, references)?;
            }
        }
    }
    collect_tail_contract_references(&block.tail, references)
}

fn collect_authored_failure_directive_references(
    directive: &AuthoredFailureDirective,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    if let AuthoredFailureDirective::Custom {
        handler_state_contract_ref,
        route_contract,
        arms,
    } = directive
    {
        push_typed_reference(
            references,
            STATE_CONTRACT_OBJECT_TYPE,
            handler_state_contract_ref,
        )?;
        collect_closed_sum_references(route_contract, references)?;
        for arm in arms {
            collect_authored_block_references(&arm.body, references)?;
        }
    }
    Ok(())
}

fn expanded_component_references(
    program: &ExpandedStructuredProgram,
) -> Result<Vec<ComponentObjectReference>> {
    let mut references = Vec::new();
    for root in &program.input_roots {
        collect_slot_contract_references(root, &mut references)?;
    }
    push_contract_reference(&mut references, &program.output_contract_ref)?;
    collect_failure_contract_reference(&program.failure_contract, &mut references)?;
    collect_expanded_block_references(&program.root, &mut references)?;
    Ok(references)
}

fn collect_expanded_block_references(
    block: &ExpandedBlock,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    collect_structural_path_references(&block.path, references)?;
    collect_failure_scope_references(&block.failure_scope, references)?;
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state) => {
                collect_expanded_state_references(state, references)?;
            }
            ExpandedDeclaration::Match(binding) => {
                collect_structural_path_references(&binding.path, references)?;
                collect_slot_contract_references(&binding.selector, references)?;
                collect_closed_sum_references(&binding.selector_contract, references)?;
                for arm in &binding.arms {
                    collect_structural_path_references(&arm.path, references)?;
                    collect_expanded_block_references(&arm.body, references)?;
                }
                collect_slot_contract_references(&binding.output_slot, references)?;
            }
            ExpandedDeclaration::FanOut(group) => {
                collect_structural_path_references(&group.path, references)?;
                push_contract_reference(references, &group.lane_output_contract_ref)?;
                collect_failure_contract_reference(&group.lane_failure_contract, references)?;
                for lane in &group.lanes {
                    collect_structural_path_references(&lane.path, references)?;
                    collect_expanded_block_references(&lane.body, references)?;
                    collect_slot_contract_references(&lane.outcome_slot, references)?;
                }
                collect_slot_contract_references(&group.output_slot, references)?;
            }
            ExpandedDeclaration::Fragment(fragment) => {
                collect_structural_path_references(&fragment.path, references)?;
                for binding in &fragment.input_bindings {
                    push_contract_reference(references, &binding.child_contract_ref)?;
                    collect_slot_contract_references(&binding.caller_slot, references)?;
                }
                collect_expanded_block_references(&fragment.body, references)?;
                collect_slot_contract_references(&fragment.success_slot, references)?;
                collect_failure_boundary_references(&fragment.failure_boundary, references)?;
            }
        }
    }
    for failure_exit in &block.failure_exits {
        collect_slot_contract_references(failure_exit, references)?;
    }
    collect_tail_contract_references(&block.tail, references)
}

fn collect_expanded_state_references(
    state: &ExpandedStateBinding,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    collect_structural_path_references(&state.occurrence_path, references)?;
    collect_state_contract_references(&state.contract, references)?;
    for input in &state.inputs {
        collect_slot_contract_references(input, references)?;
    }
    collect_slot_contract_references(&state.output_slot, references)?;
    collect_failure_boundary_references(&state.failure_boundary, references)
}

fn collect_failure_boundary_references(
    boundary: &CertifiedFailureBoundary,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    match boundary {
        CertifiedFailureBoundary::NoFailure(marker) => {
            push_contract_reference(references, &marker.never_contract_ref)
        }
        CertifiedFailureBoundary::Typed {
            failure_contract,
            source_slot,
            plan,
        } => {
            collect_failure_contract_reference(failure_contract, references)?;
            collect_slot_contract_references(source_slot, references)?;
            collect_failure_plan_references(plan, references)
        }
    }
}

fn collect_failure_plan_references(
    plan: &FailurePlan,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    match plan {
        FailurePlan::Handled {
            plan_path,
            source_slot,
            before_handler,
            handler,
            continuation,
            ..
        } => {
            collect_structural_path_references(plan_path, references)?;
            collect_slot_contract_references(source_slot, references)?;
            collect_expanded_block_references(before_handler, references)?;
            collect_expanded_state_references(handler, references)?;
            match continuation.as_ref() {
                HandlerContinuation::DefaultPropagation {
                    handler_output_slot,
                    route_contract,
                    payload_slot,
                    failure_tail,
                } => {
                    collect_slot_contract_references(handler_output_slot, references)?;
                    collect_closed_sum_references(route_contract, references)?;
                    collect_slot_contract_references(payload_slot, references)?;
                    collect_slot_contract_references(failure_tail, references)?;
                }
                HandlerContinuation::CustomRecovery {
                    handler_output_slot,
                    route_contract,
                    arms,
                } => {
                    collect_slot_contract_references(handler_output_slot, references)?;
                    collect_closed_sum_references(route_contract, references)?;
                    for arm in arms {
                        collect_expanded_block_references(&arm.body, references)?;
                    }
                }
            }
        }
        FailurePlan::Propagate {
            plan_path,
            source_slot,
            before_boundary,
            mapping_chain,
            boundary_slot,
            ..
        } => {
            collect_structural_path_references(plan_path, references)?;
            collect_slot_contract_references(source_slot, references)?;
            collect_expanded_block_references(before_boundary, references)?;
            for link in mapping_chain {
                collect_expanded_state_references(&link.mapper, references)?;
                collect_slot_contract_references(&link.input_slot, references)?;
                collect_slot_contract_references(&link.output_slot, references)?;
            }
            collect_slot_contract_references(boundary_slot, references)?;
        }
    }
    Ok(())
}

fn collect_state_contract_references(
    contract: &StructuredStateContract,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    push_typed_reference(
        references,
        STATE_CONTRACT_OBJECT_TYPE,
        &contract.state_contract_ref,
    )?;
    if let Some(capability_contract_ref) = contract.execution.capability_contract_ref() {
        push_typed_reference(
            references,
            CAPABILITY_CONTRACT_OBJECT_TYPE,
            capability_contract_ref,
        )?;
    }
    push_contract_reference(references, &contract.input_contract_ref)?;
    push_contract_reference(references, &contract.output_contract_ref)?;
    for slot in &contract.fact_slots {
        push_typed_reference(
            references,
            FACT_DESCRIPTOR_OBJECT_TYPE,
            slot.fact_descriptor_ref(),
        )?;
        push_contract_reference(
            references,
            &retained_value_contract_ref(slot.subject_contract())?,
        )?;
        push_contract_reference(
            references,
            &retained_value_contract_ref(slot.response_contract())?,
        )?;
    }
    collect_failure_contract_reference(&contract.failure_contract, references)?;
    if let Some(requirement_ref) = &contract.capability_requirement_ref {
        push_contract_reference(references, requirement_ref)?;
    }
    Ok(())
}

fn collect_failure_scope_references(
    binding: &FailureScopeBinding,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    collect_failure_contract_reference(binding.failure_contract(), references)?;
    if let FailureScopeBinding::Owns { scope } = binding {
        for mapper in &scope.default_mappers {
            push_contract_reference(references, &mapper.source_failure_contract_ref)?;
            push_typed_reference(
                references,
                STATE_CONTRACT_OBJECT_TYPE,
                &mapper.mapper_state_contract_ref,
            )?;
            collect_closed_sum_references(&mapper.route_contract, references)?;
        }
    }
    Ok(())
}

fn collect_failure_contract_reference(
    failure_contract: &StructuredFailureContract,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    if let StructuredFailureContract::Typed {
        contract,
        contract_ref,
    } = failure_contract
    {
        push_contract_reference(references, contract.evidence_contract_ref())?;
        push_contract_reference(references, contract_ref)?;
    }
    Ok(())
}

fn collect_closed_sum_references(
    contract: &ClosedSumContract,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    push_contract_reference(references, &contract.closed_sum_contract_ref)?;
    push_contract_reference(references, &contract.selector_contract_ref)?;
    for variant in &contract.variants {
        for payload in &variant.payloads {
            push_contract_reference(references, &payload.contract_ref)?;
        }
    }
    Ok(())
}

fn collect_tail_contract_references(
    tail: &BlockTail,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    match tail {
        BlockTail::Normal(slot) | BlockTail::ScopeFailure(slot) => {
            collect_slot_contract_references(slot, references)
        }
    }
}

fn collect_slot_contract_references(
    slot: &LexicalSlot,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    collect_structural_path_references(&slot.lexical_path, references)?;
    push_contract_reference(references, &slot.contract_ref)?;
    match &slot.producer {
        LexicalProducer::AdmissionRoot { .. }
        | LexicalProducer::AuthoredCallOutput { .. }
        | LexicalProducer::StateOutput { .. } => {}
        LexicalProducer::ArmValue {
            selected_arm_path,
            source,
        } => {
            collect_structural_path_references(selected_arm_path, references)?;
            collect_slot_contract_references(source, references)?;
        }
        LexicalProducer::FragmentInput { source, .. }
        | LexicalProducer::FragmentBoundary { source, .. } => {
            collect_slot_contract_references(source, references)?;
        }
        LexicalProducer::MatchMerge {
            match_path,
            declaration_ordered_arm_slots,
            ..
        } => {
            collect_structural_path_references(match_path, references)?;
            for source in declaration_ordered_arm_slots {
                collect_slot_contract_references(source, references)?;
            }
        }
        LexicalProducer::ScopeFailureMerge {
            declaration_ordered_failure_slots,
            ..
        } => {
            for source in declaration_ordered_failure_slots {
                collect_slot_contract_references(source, references)?;
            }
        }
        LexicalProducer::VariantPayload { selector, .. } => {
            collect_slot_contract_references(selector, references)?;
        }
        LexicalProducer::LaneOutcome {
            lane_path,
            success_slot,
            failure_slot,
            ..
        } => {
            collect_structural_path_references(lane_path, references)?;
            if let Some(success_slot) = success_slot {
                collect_slot_contract_references(success_slot, references)?;
            }
            if let Some(failure_slot) = failure_slot {
                collect_slot_contract_references(failure_slot, references)?;
            }
        }
        LexicalProducer::FanOutJoin {
            group_path,
            declaration_ordered_lane_slots,
            ..
        } => {
            collect_structural_path_references(group_path, references)?;
            for source in declaration_ordered_lane_slots {
                collect_slot_contract_references(source, references)?;
            }
        }
    }
    Ok(())
}

fn collect_structural_path_references(
    path: &StructuralPath,
    references: &mut Vec<ComponentObjectReference>,
) -> Result<()> {
    for segment in path.segments() {
        if let StructuralPathSegment::Fragment { expansion_ref, .. } = segment {
            let schema_name = expansion_ref.schema_id().canonical_name().ok_or_else(|| {
                CertifyError::Certification(
                    "structured expansion reference has no canonical schema name".to_owned(),
                )
            })?;
            let object_type = match schema_name {
                "mfm.authored-structured-program" => stable_id(AUTHORED_OBJECT_TYPE)?,
                "mfm.structured-state-contract" => stable_id(STATE_CONTRACT_OBJECT_TYPE)?,
                _ => contract_object_type(expansion_ref)?,
            };
            references.push(ComponentObjectReference {
                object_type,
                content_ref: expansion_ref.clone(),
            });
        }
    }
    Ok(())
}

fn profile_component_references(
    profile: &StructuredExpansionProfile,
) -> Result<Vec<ComponentObjectReference>> {
    let mut references = Vec::new();
    for policy in &profile.policies {
        push_typed_reference(
            &mut references,
            EXPANSION_POLICY_CONTRACT_OBJECT_TYPE,
            &policy.policy_ref,
        )?;
        for binding in &policy.boundary_recipes {
            push_typed_reference(
                &mut references,
                STATE_CONTRACT_OBJECT_TYPE,
                &binding.boundary_contract_ref,
            )?;
            push_typed_reference(
                &mut references,
                POLICY_RECIPE_OBJECT_TYPE,
                &binding.recipe_ref,
            )?;
        }
    }
    Ok(references)
}

fn push_contract_reference(
    references: &mut Vec<ComponentObjectReference>,
    content_ref: &ContentRef,
) -> Result<()> {
    references.push(contract_reference(content_ref)?);
    Ok(())
}

fn push_typed_reference(
    references: &mut Vec<ComponentObjectReference>,
    object_type: &str,
    content_ref: &ContentRef,
) -> Result<()> {
    references.push(ComponentObjectReference {
        object_type: stable_id(object_type)?,
        content_ref: content_ref.clone(),
    });
    Ok(())
}

fn kernel_derived_contract_objects(
    expanded: &ExpandedStructuredProgram,
) -> Result<BTreeMap<ContentRef, RegisteredComponentObject>> {
    let mut objects = BTreeMap::new();
    collect_kernel_derived_contract_objects(&expanded.root, &mut objects)?;
    Ok(objects)
}

fn collect_kernel_derived_contract_objects(
    block: &ExpandedBlock,
    objects: &mut BTreeMap<ContentRef, RegisteredComponentObject>,
) -> Result<()> {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state) => {
                collect_kernel_derived_failure_contract_objects(&state.failure_boundary, objects)?;
            }
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    collect_kernel_derived_contract_objects(&arm.body, objects)?;
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                let lane_content_ref = lane_outcome_contract_ref(
                    &group.lane_output_contract_ref,
                    &group.lane_failure_contract,
                )?;
                let value = CanonicalJsonValue::from_canonical_json(
                    lane_outcome_contract_canonical_json(
                        &group.lane_output_contract_ref,
                        &group.lane_failure_contract,
                    )?
                    .as_bytes(),
                )?;
                let mut lane_references =
                    vec![contract_reference(&group.lane_output_contract_ref)?];
                collect_failure_contract_reference(
                    &group.lane_failure_contract,
                    &mut lane_references,
                )?;
                let object = RegisteredComponentObject {
                    object: CertifiedComponentObject {
                        object_type: stable_id(LANE_OUTCOME_CONTRACT_OBJECT_TYPE)?,
                        content_ref: lane_content_ref.clone(),
                        value,
                    },
                    outbound_references: lane_references,
                };
                validate_component_object(&object.object)?;
                insert_exact(
                    objects,
                    lane_content_ref.clone(),
                    object,
                    "kernel-derived lane-outcome contract",
                )?;
                let join_content_ref = fan_out_join_contract_ref(
                    &group.lane_output_contract_ref,
                    &group.lane_failure_contract,
                )?;
                if group.output_slot.contract_ref != join_content_ref {
                    return Err(CertifyError::Certification(
                        "fan-out output does not use its exact nominal join contract".to_owned(),
                    ));
                }
                let join = RegisteredComponentObject {
                    object: CertifiedComponentObject {
                        object_type: stable_id(FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE)?,
                        content_ref: join_content_ref.clone(),
                        value: CanonicalJsonValue::from_canonical_json(
                            fan_out_join_contract_canonical_json(
                                &group.lane_output_contract_ref,
                                &group.lane_failure_contract,
                            )?
                            .as_bytes(),
                        )?,
                    },
                    outbound_references: vec![ComponentObjectReference {
                        object_type: stable_id(LANE_OUTCOME_CONTRACT_OBJECT_TYPE)?,
                        content_ref: lane_content_ref,
                    }],
                };
                validate_component_object(&join.object)?;
                insert_exact(
                    objects,
                    join_content_ref,
                    join,
                    "kernel-derived fan-out join contract",
                )?;
                for lane in &group.lanes {
                    collect_kernel_derived_contract_objects(&lane.body, objects)?;
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                collect_kernel_derived_contract_objects(&fragment.body, objects)?;
                collect_kernel_derived_failure_contract_objects(
                    &fragment.failure_boundary,
                    objects,
                )?;
            }
        }
    }
    Ok(())
}

fn collect_kernel_derived_failure_contract_objects(
    boundary: &CertifiedFailureBoundary,
    objects: &mut BTreeMap<ContentRef, RegisteredComponentObject>,
) -> Result<()> {
    let CertifiedFailureBoundary::Typed { plan, .. } = boundary else {
        return Ok(());
    };
    match plan.as_ref() {
        FailurePlan::Handled {
            before_handler,
            continuation,
            ..
        } => {
            collect_kernel_derived_contract_objects(before_handler, objects)?;
            if let HandlerContinuation::CustomRecovery { arms, .. } = continuation.as_ref() {
                for arm in arms {
                    collect_kernel_derived_contract_objects(&arm.body, objects)?;
                }
            }
        }
        FailurePlan::Propagate {
            before_boundary, ..
        } => collect_kernel_derived_contract_objects(before_boundary, objects)?,
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_certified_document(
    authored: &AuthoredStructuredProgram,
    expanded: &ExpandedStructuredProgram,
    profile: &StructuredExpansionProfile,
    proof: &StructuredExpansionProof,
    coverage: &StructuredPolicyCoverageProof,
    component_manifest: &StateCapabilityAdapterSignerResourceManifest,
    implementation_manifest: &SecretFreeImplementationManifest,
    policy: &QualifiedStructuredEntryPointPolicy,
    registry: &StructuredCertificationRegistry,
) -> Result<CertifiedProgramDocument> {
    let mut objects = registry.component_objects.clone();
    let never_object = kernel_never_component_object()?;
    insert_exact(
        &mut objects,
        never_object.object.content_ref.clone(),
        never_object,
        "kernel Never component",
    )?;
    let access_fault_object = kernel_access_fault_component_object()?;
    insert_exact(
        &mut objects,
        access_fault_object.object.content_ref.clone(),
        access_fault_object,
        "kernel access-fault component",
    )?;
    for object in kernel_derived_contract_objects(expanded)?.into_values() {
        insert_exact(
            &mut objects,
            object.object.content_ref.clone(),
            object,
            "kernel-derived component",
        )?;
    }
    for child in registry.children.values() {
        let child_object = component_object(
            AUTHORED_OBJECT_TYPE,
            child.content_ref()?,
            child,
            authored_component_references(child)?,
        )?;
        insert_exact(
            &mut objects,
            child_object.object.content_ref.clone(),
            child_object,
            "registered child component",
        )?;
    }
    let authored_object = component_object(
        AUTHORED_OBJECT_TYPE,
        authored.content_ref()?,
        authored,
        authored_component_references(authored)?,
    )?;
    let expanded_object = component_object(
        EXPANDED_OBJECT_TYPE,
        expanded.content_ref()?,
        expanded,
        expanded_component_references(expanded)?,
    )?;
    let policy_object_type = stable_id(EXPANSION_POLICY_CONTRACT_OBJECT_TYPE)?;
    let profile_object = component_object(
        PROFILE_OBJECT_TYPE,
        profile.content_ref()?,
        profile,
        profile_component_references(profile)?,
    )?;
    let proof_ref = typed_content_ref("mfm.structured-expansion-proof", proof)?;
    let mut proof_references = vec![
        ComponentObjectReference {
            object_type: stable_id(AUTHORED_OBJECT_TYPE)?,
            content_ref: proof.authored_program_ref.clone(),
        },
        ComponentObjectReference {
            object_type: stable_id(EXPANDED_OBJECT_TYPE)?,
            content_ref: proof.expanded_program_ref.clone(),
        },
        ComponentObjectReference {
            object_type: stable_id(PROFILE_OBJECT_TYPE)?,
            content_ref: proof.expansion_profile_ref.clone(),
        },
    ];
    for entry in &proof.substitution_trace {
        proof_references.push(ComponentObjectReference {
            object_type: match entry.stage {
                ExpansionStage::ChildSubstitution => stable_id(AUTHORED_OBJECT_TYPE)?,
                ExpansionStage::CapabilityLowering => {
                    stable_id(CAPABILITY_REQUIREMENT_OBJECT_TYPE)?
                }
                ExpansionStage::PolicyWrapping => policy_object_type.clone(),
                ExpansionStage::FailureCompletion => contract_object_type(&entry.expansion_ref)?,
            },
            content_ref: entry.expansion_ref.clone(),
        });
    }
    let proof_object = component_object(
        PROOF_OBJECT_TYPE,
        proof_ref.clone(),
        proof,
        proof_references,
    )?;
    let coverage_ref = typed_content_ref("mfm.structured-policy-coverage-proof", coverage)?;
    let mut coverage_references = vec![
        ComponentObjectReference {
            object_type: stable_id(AUTHORED_OBJECT_TYPE)?,
            content_ref: coverage.authored_program_ref.clone(),
        },
        ComponentObjectReference {
            object_type: stable_id(EXPANDED_OBJECT_TYPE)?,
            content_ref: coverage.expanded_program_ref.clone(),
        },
        ComponentObjectReference {
            object_type: stable_id(PROFILE_OBJECT_TYPE)?,
            content_ref: coverage.expansion_profile_ref.clone(),
        },
    ];
    coverage_references.extend(
        coverage
            .entries
            .iter()
            .map(|entry| ComponentObjectReference {
                object_type: policy_object_type.clone(),
                content_ref: entry.policy_ref.clone(),
            }),
    );
    let coverage_object = component_object(
        COVERAGE_PROOF_OBJECT_TYPE,
        coverage_ref.clone(),
        coverage,
        coverage_references,
    )?;
    let component_manifest_ref = structured_component_manifest_ref(component_manifest)?;
    let component_manifest_object = component_object(
        COMPONENT_MANIFEST_OBJECT_TYPE,
        component_manifest_ref.clone(),
        component_manifest,
        component_manifest
            .entries
            .iter()
            .map(|entry| {
                Ok(ComponentObjectReference {
                    object_type: semantic_component_object_type(entry.component_kind)?,
                    content_ref: entry.semantic_contract_ref.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
    )?;
    let implementation_manifest_ref =
        secret_free_implementation_manifest_ref(implementation_manifest)?;
    let mut implementation_manifest_references =
        Vec::with_capacity(implementation_manifest.entries.len() * 2);
    for entry in &implementation_manifest.entries {
        implementation_manifest_references.extend([
            ComponentObjectReference {
                object_type: semantic_component_object_type(entry.component_kind)?,
                content_ref: entry.semantic_contract_ref.clone(),
            },
            ComponentObjectReference {
                object_type: stable_id(IMPLEMENTATION_CONTRACT_OBJECT_TYPE)?,
                content_ref: entry.implementation_contract_ref.clone(),
            },
        ]);
    }
    let implementation_manifest_object = component_object(
        IMPLEMENTATION_MANIFEST_OBJECT_TYPE,
        implementation_manifest_ref.clone(),
        implementation_manifest,
        implementation_manifest_references,
    )?;
    for object in [
        authored_object,
        expanded_object,
        profile_object,
        proof_object,
        coverage_object,
        component_manifest_object,
        implementation_manifest_object,
    ] {
        insert_exact(
            &mut objects,
            object.object.content_ref.clone(),
            object,
            "built component",
        )?;
    }

    let components = CertifiedProgramComponents {
        certified_program_contract_ref: policy.certified_program_contract_ref.clone(),
        entry_point_contract_ref: policy.entry_point_contract_ref.clone(),
        qualified_entry_point_admission_policy_ref: policy.admission_policy_ref.clone(),
        authored_program_ref: proof.authored_program_ref.clone(),
        expanded_program_ref: proof.expanded_program_ref.clone(),
        expansion_profile_ref: profile.content_ref()?,
        expansion_proof_ref: proof_ref,
        policy_coverage_proof_ref: coverage_ref,
        public_input_output_failure_contract_refs: StructuredPublicContractRefs {
            input_contract_refs: policy.public_input_contract_refs.clone(),
            output_contract_ref: policy.public_output_contract_ref.clone(),
            failure_contract_ref: policy.public_failure_contract_ref.clone(),
        },
        certified_structural_bounds: CertifiedStructuralBounds::from(profile),
        state_capability_adapter_signer_resource_manifest_closure_ref: component_manifest_ref,
        secret_free_implementation_manifest_closure_ref: implementation_manifest_ref,
        certification_predicate_set_ref: policy.certification_predicate_set_ref.clone(),
    };
    let closure = canonical_component_closure(&components, &objects)?;
    let digest = component_closure_digest(&components, &closure)?;
    Ok(CertifiedProgramDocument {
        root: CertifiedProgramRoot {
            components,
            canonical_component_closure_digest: digest,
        },
        component_closure: closure,
    })
}

fn kernel_never_component_object() -> Result<RegisteredComponentObject> {
    let never_value: serde_json::Value = serde_json::from_slice(
        never_failure_contract_canonical_json()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            .as_bytes(),
    )
    .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let object = CertifiedComponentObject {
        object_type: stable_id(DATA_CONTRACT_OBJECT_TYPE)?,
        content_ref: never_failure_contract_ref()?,
        value: CanonicalJsonValue::new(never_value)
            .map_err(|error| CertifyError::Certification(error.to_string()))?,
    };
    validate_component_object(&object)?;
    Ok(RegisteredComponentObject {
        object,
        outbound_references: Vec::new(),
    })
}

fn kernel_access_fault_component_object() -> Result<RegisteredComponentObject> {
    let value = CanonicalJsonValue::from_canonical_json(
        access_fault_contract_canonical_json()
            .map_err(|error| CertifyError::Certification(error.to_string()))?
            .as_bytes(),
    )?;
    let object = RegisteredComponentObject {
        object: CertifiedComponentObject {
            object_type: stable_id(DATA_CONTRACT_OBJECT_TYPE)?,
            content_ref: access_fault_contract_ref()?,
            value,
        },
        outbound_references: Vec::new(),
    };
    validate_component_object(&object.object)?;
    Ok(object)
}

/// Callback-free verification of one persisted certification document.
fn verify_certified_document(
    document: &CertifiedProgramDocument,
    policy: &QualifiedStructuredEntryPointPolicy,
    registry: &StructuredCertificationRegistry,
) -> Result<()> {
    let components = &document.root.components;
    let public_contracts = &components.public_input_output_failure_contract_refs;
    if components.certified_program_contract_ref != policy.certified_program_contract_ref
        || components.entry_point_contract_ref != policy.entry_point_contract_ref
        || components.qualified_entry_point_admission_policy_ref != policy.admission_policy_ref
        || components.expansion_profile_ref != policy.expansion_profile_ref
        || components.certification_predicate_set_ref != policy.certification_predicate_set_ref
        || public_contracts.input_contract_refs != policy.public_input_contract_refs
        || public_contracts.output_contract_ref != policy.public_output_contract_ref
        || public_contracts.failure_contract_ref != policy.public_failure_contract_ref
    {
        return Err(CertifyError::Verification(
            "certified document is not bound to the exact qualified policy".to_owned(),
        ));
    }
    let mut objects = BTreeMap::new();
    for object in &document.component_closure {
        validate_component_object(object)
            .map_err(|error| CertifyError::Verification(error.to_string()))?;
        if objects
            .insert(object.content_ref.clone(), object.clone())
            .is_some()
        {
            return Err(CertifyError::Verification(
                "certified component closure repeats an emitted object".to_owned(),
            ));
        }
    }
    let expanded: ExpandedStructuredProgram = decode_component(
        &objects,
        &components.expanded_program_ref,
        EXPANDED_OBJECT_TYPE,
    )?;
    let derived_contracts = kernel_derived_contract_objects(&expanded)?;
    let registered_objects =
        validate_qualified_closure_objects(&objects, registry, &derived_contracts)?;
    let closure = canonical_component_closure(components, &registered_objects)?;
    if closure != document.component_closure
        || component_closure_digest(components, &closure)?
            != document.root.canonical_component_closure_digest
    {
        return Err(CertifyError::Verification(
            "certified component closure mismatch".to_owned(),
        ));
    }
    let authored: AuthoredStructuredProgram = decode_component(
        &objects,
        &components.authored_program_ref,
        AUTHORED_OBJECT_TYPE,
    )?;
    let profile: StructuredExpansionProfile = decode_component(
        &objects,
        &components.expansion_profile_ref,
        PROFILE_OBJECT_TYPE,
    )?;
    let proof: StructuredExpansionProof =
        decode_component(&objects, &components.expansion_proof_ref, PROOF_OBJECT_TYPE)?;
    let coverage: StructuredPolicyCoverageProof = decode_component(
        &objects,
        &components.policy_coverage_proof_ref,
        COVERAGE_PROOF_OBJECT_TYPE,
    )?;
    let component_manifest: StateCapabilityAdapterSignerResourceManifest = decode_component(
        &objects,
        &components.state_capability_adapter_signer_resource_manifest_closure_ref,
        COMPONENT_MANIFEST_OBJECT_TYPE,
    )?;
    let implementation_manifest: SecretFreeImplementationManifest = decode_component(
        &objects,
        &components.secret_free_implementation_manifest_closure_ref,
        IMPLEMENTATION_MANIFEST_OBJECT_TYPE,
    )?;

    let authored_ref = authored.content_ref()?;
    let expanded_ref = expanded.content_ref()?;
    let profile_ref = profile.content_ref()?;
    let proof_ref = typed_content_ref("mfm.structured-expansion-proof", &proof)?;
    let coverage_ref = typed_content_ref("mfm.structured-policy-coverage-proof", &coverage)?;
    let component_manifest_ref = structured_component_manifest_ref(&component_manifest)?;
    let implementation_manifest_ref =
        secret_free_implementation_manifest_ref(&implementation_manifest)?;
    if authored_ref != components.authored_program_ref
        || expanded_ref != components.expanded_program_ref
        || profile_ref != components.expansion_profile_ref
        || proof_ref != components.expansion_proof_ref
        || coverage_ref != components.policy_coverage_proof_ref
        || component_manifest_ref
            != components.state_capability_adapter_signer_resource_manifest_closure_ref
        || implementation_manifest_ref != components.secret_free_implementation_manifest_closure_ref
        || proof.authored_program_ref != authored_ref
        || proof.expanded_program_ref != expanded_ref
        || proof.expansion_profile_ref != profile_ref
        || coverage.authored_program_ref != authored_ref
        || coverage.expanded_program_ref != expanded_ref
        || coverage.expansion_profile_ref != profile_ref
        || components.certified_structural_bounds != CertifiedStructuralBounds::from(&profile)
    {
        return Err(CertifyError::Verification(
            "certified document component references or structural bounds diverge".to_owned(),
        ));
    }
    validate_policy_root(&authored, policy, &profile)
        .map_err(|error| CertifyError::Verification(error.to_string()))?;
    let recomputed = expand_and_prove(authored.clone(), profile.clone(), registry)
        .map_err(|error| CertifyError::Verification(error.to_string()))?;
    if recomputed.authored != authored
        || recomputed.expanded != expanded
        || recomputed.profile != profile
        || recomputed.proof != proof
        || recomputed.coverage != coverage
        || recomputed.component_manifest != component_manifest
        || recomputed.implementation_manifest != implementation_manifest
    {
        return Err(CertifyError::Verification(
            "persisted expansion, proof, coverage, trace, or manifest differs from exact recomputation"
                .to_owned(),
        ));
    }
    Ok(())
}

fn decode_component<T: DeserializeOwned>(
    objects: &BTreeMap<ContentRef, CertifiedComponentObject>,
    content_ref: &ContentRef,
    object_type: &str,
) -> Result<T> {
    let object = objects.get(content_ref).ok_or_else(|| {
        CertifyError::Verification("certified component object is missing".to_owned())
    })?;
    if object.object_type != stable_id(object_type)? || &object.content_ref != content_ref {
        return Err(CertifyError::Verification(
            "certified component object has the wrong registered type".to_owned(),
        ));
    }
    serde_json::from_value(object.value.as_json().clone()).map_err(|error| {
        CertifyError::Verification(format!("certified component object decode failed: {error}"))
    })
}

fn validate_qualified_closure_objects(
    objects: &BTreeMap<ContentRef, CertifiedComponentObject>,
    registry: &StructuredCertificationRegistry,
    derived_contracts: &BTreeMap<ContentRef, RegisteredComponentObject>,
) -> Result<BTreeMap<ContentRef, RegisteredComponentObject>> {
    let never_object = kernel_never_component_object()?;
    let access_fault_object = kernel_access_fault_component_object()?;
    let mut registered_objects = BTreeMap::new();
    for object in objects.values() {
        let expected = match object.object_type.as_str() {
            DATA_CONTRACT_OBJECT_TYPE
            | FACT_DESCRIPTOR_OBJECT_TYPE
            | CLOSED_SUM_CONTRACT_OBJECT_TYPE
            | CAPABILITY_REQUIREMENT_OBJECT_TYPE
            | EXPANSION_POLICY_CONTRACT_OBJECT_TYPE
            | ENTRY_POINT_CONTRACT_OBJECT_TYPE
            | CERTIFIED_PROGRAM_CONTRACT_OBJECT_TYPE
            | POLICY_COVERAGE_CONTRACT_OBJECT_TYPE
            | PREDICATE_SET_OBJECT_TYPE
            | LANE_OUTCOME_CONTRACT_OBJECT_TYPE
            | FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE
            | POLICY_OBJECT_TYPE
            | STATE_CONTRACT_OBJECT_TYPE
            | CAPABILITY_CONTRACT_OBJECT_TYPE
            | ADAPTER_CONTRACT_OBJECT_TYPE
            | SIGNER_CONTRACT_OBJECT_TYPE
            | RESOURCE_CONTRACT_OBJECT_TYPE
            | IMPLEMENTATION_CONTRACT_OBJECT_TYPE
            | EXECUTABLE_IDENTITY_OBJECT_TYPE
            | QUALIFICATION_ARTIFACT_OBJECT_TYPE => registry
                .component_objects
                .get(&object.content_ref)
                .or_else(|| derived_contracts.get(&object.content_ref))
                .or_else(|| {
                    (object.content_ref == never_object.object.content_ref).then_some(&never_object)
                })
                .or_else(|| {
                    (object.content_ref == access_fault_object.object.content_ref)
                        .then_some(&access_fault_object)
                })
                .cloned()
                .ok_or_else(|| {
                    CertifyError::Verification(
                        "component contract is not present in the qualified registry".to_owned(),
                    )
                })?,
            AUTHORED_OBJECT_TYPE => {
                let value: AuthoredStructuredProgram =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                component_object(
                    AUTHORED_OBJECT_TYPE,
                    value.content_ref()?,
                    &value,
                    authored_component_references(&value)?,
                )?
            }
            POLICY_RECIPE_OBJECT_TYPE => {
                let value: PolicyExpansionRecipe =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                let expected = component_object(
                    POLICY_RECIPE_OBJECT_TYPE,
                    value
                        .content_ref()
                        .map_err(|error| CertifyError::Verification(error.to_string()))?,
                    &value,
                    policy_recipe_component_references(&value)?,
                )?;
                if !registry.policy_recipes.contains_key(&object.content_ref) {
                    return Err(CertifyError::Verification(
                        "policy recipe is not exact qualified callback-free data".to_owned(),
                    ));
                }
                expected
            }
            EXPANDED_OBJECT_TYPE => {
                let value: ExpandedStructuredProgram =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                component_object(
                    EXPANDED_OBJECT_TYPE,
                    value.content_ref()?,
                    &value,
                    expanded_component_references(&value)?,
                )?
            }
            PROFILE_OBJECT_TYPE => {
                let value: StructuredExpansionProfile =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                component_object(
                    PROFILE_OBJECT_TYPE,
                    value.content_ref()?,
                    &value,
                    profile_component_references(&value)?,
                )?
            }
            PROOF_OBJECT_TYPE => {
                let value: StructuredExpansionProof =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                let policy_object_type = stable_id(EXPANSION_POLICY_CONTRACT_OBJECT_TYPE)?;
                let mut outbound_references = vec![
                    ComponentObjectReference {
                        object_type: stable_id(AUTHORED_OBJECT_TYPE)?,
                        content_ref: value.authored_program_ref.clone(),
                    },
                    ComponentObjectReference {
                        object_type: stable_id(EXPANDED_OBJECT_TYPE)?,
                        content_ref: value.expanded_program_ref.clone(),
                    },
                    ComponentObjectReference {
                        object_type: stable_id(PROFILE_OBJECT_TYPE)?,
                        content_ref: value.expansion_profile_ref.clone(),
                    },
                ];
                for entry in &value.substitution_trace {
                    outbound_references.push(ComponentObjectReference {
                        object_type: match entry.stage {
                            ExpansionStage::ChildSubstitution => stable_id(AUTHORED_OBJECT_TYPE)?,
                            ExpansionStage::CapabilityLowering => {
                                stable_id(CAPABILITY_REQUIREMENT_OBJECT_TYPE)?
                            }
                            ExpansionStage::PolicyWrapping => policy_object_type.clone(),
                            ExpansionStage::FailureCompletion => {
                                contract_object_type(&entry.expansion_ref)?
                            }
                        },
                        content_ref: entry.expansion_ref.clone(),
                    });
                }
                component_object(
                    PROOF_OBJECT_TYPE,
                    typed_content_ref("mfm.structured-expansion-proof", &value)?,
                    &value,
                    outbound_references,
                )?
            }
            COVERAGE_PROOF_OBJECT_TYPE => {
                let value: StructuredPolicyCoverageProof =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                let contract_object_type = stable_id(EXPANSION_POLICY_CONTRACT_OBJECT_TYPE)?;
                let mut outbound_references = vec![
                    ComponentObjectReference {
                        object_type: stable_id(AUTHORED_OBJECT_TYPE)?,
                        content_ref: value.authored_program_ref.clone(),
                    },
                    ComponentObjectReference {
                        object_type: stable_id(EXPANDED_OBJECT_TYPE)?,
                        content_ref: value.expanded_program_ref.clone(),
                    },
                    ComponentObjectReference {
                        object_type: stable_id(PROFILE_OBJECT_TYPE)?,
                        content_ref: value.expansion_profile_ref.clone(),
                    },
                ];
                outbound_references.extend(value.entries.iter().map(|entry| {
                    ComponentObjectReference {
                        object_type: contract_object_type.clone(),
                        content_ref: entry.policy_ref.clone(),
                    }
                }));
                component_object(
                    COVERAGE_PROOF_OBJECT_TYPE,
                    typed_content_ref("mfm.structured-policy-coverage-proof", &value)?,
                    &value,
                    outbound_references,
                )?
            }
            COMPONENT_MANIFEST_OBJECT_TYPE => {
                let value: StateCapabilityAdapterSignerResourceManifest =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                let outbound_references = value
                    .entries
                    .iter()
                    .map(|entry| {
                        Ok(ComponentObjectReference {
                            object_type: semantic_component_object_type(entry.component_kind)?,
                            content_ref: entry.semantic_contract_ref.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                component_object(
                    COMPONENT_MANIFEST_OBJECT_TYPE,
                    structured_component_manifest_ref(&value)?,
                    &value,
                    outbound_references,
                )?
            }
            IMPLEMENTATION_MANIFEST_OBJECT_TYPE => {
                let value: SecretFreeImplementationManifest =
                    serde_json::from_value(object.value.as_json().clone())
                        .map_err(|error| CertifyError::Verification(error.to_string()))?;
                let mut outbound_references = Vec::with_capacity(value.entries.len() * 2);
                for entry in &value.entries {
                    outbound_references.extend([
                        ComponentObjectReference {
                            object_type: semantic_component_object_type(entry.component_kind)?,
                            content_ref: entry.semantic_contract_ref.clone(),
                        },
                        ComponentObjectReference {
                            object_type: stable_id(IMPLEMENTATION_CONTRACT_OBJECT_TYPE)?,
                            content_ref: entry.implementation_contract_ref.clone(),
                        },
                    ]);
                }
                component_object(
                    IMPLEMENTATION_MANIFEST_OBJECT_TYPE,
                    secret_free_implementation_manifest_ref(&value)?,
                    &value,
                    outbound_references,
                )?
            }
            _ => {
                return Err(CertifyError::Verification(
                    "certified component uses an unregistered object type".to_owned(),
                ));
            }
        };
        if object != &expected.object {
            return Err(CertifyError::Verification(
                "qualified component bytes or schema-derived references differ".to_owned(),
            ));
        }
        insert_exact(
            &mut registered_objects,
            object.content_ref.clone(),
            expected,
            "verified component object",
        )?;
    }
    Ok(registered_objects)
}

fn canonical_component_closure(
    components: &CertifiedProgramComponents,
    objects: &BTreeMap<ContentRef, RegisteredComponentObject>,
) -> Result<Vec<CertifiedComponentObject>> {
    let mut walker = ClosureWalker {
        objects,
        active: BTreeSet::new(),
        completed: BTreeSet::new(),
        content_types: BTreeMap::new(),
        emitted: Vec::new(),
    };
    for (object_type, content_ref) in component_roots(components)? {
        walker.walk(object_type, content_ref)?;
    }
    Ok(walker.emitted)
}

struct ClosureWalker<'a> {
    objects: &'a BTreeMap<ContentRef, RegisteredComponentObject>,
    active: BTreeSet<(StableId, ContentRef)>,
    completed: BTreeSet<(StableId, ContentRef)>,
    content_types: BTreeMap<ContentRef, StableId>,
    emitted: Vec<CertifiedComponentObject>,
}

impl ClosureWalker<'_> {
    fn walk(&mut self, object_type: StableId, content_ref: ContentRef) -> Result<()> {
        if let Some(previous) = self.content_types.get(&content_ref) {
            if previous != &object_type {
                return Err(CertifyError::Certification(
                    "component reference reused under a different object type".to_owned(),
                ));
            }
        } else {
            self.content_types
                .insert(content_ref.clone(), object_type.clone());
        }
        let key = (object_type.clone(), content_ref.clone());
        if self.completed.contains(&key) {
            return Ok(());
        }
        if !self.active.insert(key.clone()) {
            return Err(CertifyError::Certification(
                "certified component closure contains a cycle".to_owned(),
            ));
        }
        if self.emitted.len() >= MAX_CERTIFIED_COMPONENT_OBJECTS {
            return Err(CertifyError::Certification(
                "certified component closure exceeds its bound".to_owned(),
            ));
        }
        let registered = self.objects.get(&content_ref).ok_or_else(|| {
            CertifyError::Certification(format!(
                "certified component object is missing: type {object_type}, ref {content_ref:?}"
            ))
        })?;
        let object = &registered.object;
        validate_component_object(object)?;
        if object.object_type != object_type || object.content_ref != content_ref {
            return Err(CertifyError::Certification(
                "component object type or content reference mismatch".to_owned(),
            ));
        }
        self.emitted.push(object.clone());
        for outbound in &registered.outbound_references {
            self.walk(outbound.object_type.clone(), outbound.content_ref.clone())?;
        }
        self.active.remove(&key);
        self.completed.insert(key);
        Ok(())
    }
}

fn component_roots(components: &CertifiedProgramComponents) -> Result<Vec<(StableId, ContentRef)>> {
    let mut roots = vec![
        (
            stable_id(CERTIFIED_PROGRAM_CONTRACT_OBJECT_TYPE)?,
            components.certified_program_contract_ref.clone(),
        ),
        (
            stable_id(ENTRY_POINT_CONTRACT_OBJECT_TYPE)?,
            components.entry_point_contract_ref.clone(),
        ),
        (
            stable_id(POLICY_OBJECT_TYPE)?,
            components
                .qualified_entry_point_admission_policy_ref
                .clone(),
        ),
        (
            stable_id(AUTHORED_OBJECT_TYPE)?,
            components.authored_program_ref.clone(),
        ),
        (
            stable_id(EXPANDED_OBJECT_TYPE)?,
            components.expanded_program_ref.clone(),
        ),
        (
            stable_id(PROFILE_OBJECT_TYPE)?,
            components.expansion_profile_ref.clone(),
        ),
        (
            stable_id(PROOF_OBJECT_TYPE)?,
            components.expansion_proof_ref.clone(),
        ),
        (
            stable_id(COVERAGE_PROOF_OBJECT_TYPE)?,
            components.policy_coverage_proof_ref.clone(),
        ),
    ];
    for reference in &components
        .public_input_output_failure_contract_refs
        .input_contract_refs
    {
        roots.push((contract_object_type(reference)?, reference.clone()));
    }
    let output_contract_ref = &components
        .public_input_output_failure_contract_refs
        .output_contract_ref;
    roots.push((
        contract_object_type(output_contract_ref)?,
        output_contract_ref.clone(),
    ));
    let failure_contract_ref = &components
        .public_input_output_failure_contract_refs
        .failure_contract_ref;
    roots.push((
        contract_object_type(failure_contract_ref)?,
        failure_contract_ref.clone(),
    ));
    roots.extend([
        (
            stable_id(COMPONENT_MANIFEST_OBJECT_TYPE)?,
            components
                .state_capability_adapter_signer_resource_manifest_closure_ref
                .clone(),
        ),
        (
            stable_id(IMPLEMENTATION_MANIFEST_OBJECT_TYPE)?,
            components
                .secret_free_implementation_manifest_closure_ref
                .clone(),
        ),
        (
            stable_id(PREDICATE_SET_OBJECT_TYPE)?,
            components.certification_predicate_set_ref.clone(),
        ),
    ]);
    Ok(roots)
}

fn component_closure_digest(
    components: &CertifiedProgramComponents,
    closure: &[CertifiedComponentObject],
) -> Result<ContentDigest> {
    let closure_items = closure
        .iter()
        .map(|object| {
            Ok((
                &object.object_type,
                &object.content_ref,
                object
                    .value
                    .canonical_json()
                    .map_err(|error| CertifyError::Certification(error.to_string()))?
                    .to_vec(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let canonical = canonical(&(components, closure_items))?;
    let mut preimage = b"mfm.certified-program-closure.v1\0".to_vec();
    preimage.extend_from_slice(canonical.as_bytes());
    Ok(ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(&preimage),
    ))
}

fn component_object<T: Serialize>(
    object_type: &str,
    content_ref: ContentRef,
    value: &T,
    outbound_references: Vec<ComponentObjectReference>,
) -> Result<RegisteredComponentObject> {
    let value = serde_json::to_value(value)
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let object = CertifiedComponentObject {
        object_type: stable_id(object_type)?,
        content_ref,
        value: CanonicalJsonValue::new(value).map_err(|error| {
            CertifyError::Certification(format!(
                "{object_type} component value is invalid: {error}"
            ))
        })?,
    };
    validate_component_object(&object)?;
    Ok(RegisteredComponentObject {
        object,
        outbound_references,
    })
}

fn validate_component_object(object: &CertifiedComponentObject) -> Result<()> {
    let canonical = object
        .value
        .canonical_json()
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    let expected = exact_content_ref(object.content_ref.schema_id().clone(), &canonical)
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    if expected != object.content_ref {
        return Err(CertifyError::Certification(
            "component object bytes do not match their content reference".to_owned(),
        ));
    }
    Ok(())
}

fn typed_content_ref<T: Serialize>(schema_name: &str, value: &T) -> Result<ContentRef> {
    let canonical = canonical(value)?;
    let schema = mfm_ids::SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.structured-schema.v1:{schema_name}:1").as_bytes()),
    )
    .map_err(|error| CertifyError::Certification(error.to_string()))?;
    exact_content_ref(schema, &canonical)
        .map_err(|error| CertifyError::Certification(error.to_string()))
}

fn canonical<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value)
        .map_err(|error| CertifyError::Certification(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| CertifyError::Certification(error.to_string()))
}

fn stable_id(value: &str) -> Result<StableId> {
    StableId::new(value).map_err(|error| CertifyError::Certification(error.to_string()))
}

fn contract_reference(content_ref: &ContentRef) -> Result<ComponentObjectReference> {
    Ok(ComponentObjectReference {
        object_type: contract_object_type(content_ref)?,
        content_ref: content_ref.clone(),
    })
}

fn contract_object_type(content_ref: &ContentRef) -> Result<StableId> {
    let schema_name = content_ref.schema_id().canonical_name().ok_or_else(|| {
        CertifyError::Certification(
            "structured contract reference has no canonical schema name".to_owned(),
        )
    })?;
    stable_id(match schema_name {
        "mfm.retained-value-contract"
        | "mfm.component-object-evidence-contract"
        | "mfm.kernel.never-failure-contract"
        | "mfm.kernel.access-fault-contract" => DATA_CONTRACT_OBJECT_TYPE,
        "mfm.closed-sum-contract" => CLOSED_SUM_CONTRACT_OBJECT_TYPE,
        "mfm.structured-fact-descriptor" => FACT_DESCRIPTOR_OBJECT_TYPE,
        "mfm.capability-expansion-requirement" => CAPABILITY_REQUIREMENT_OBJECT_TYPE,
        "mfm.structured-expansion-policy" => EXPANSION_POLICY_CONTRACT_OBJECT_TYPE,
        "mfm.structured-entry-point-contract" => ENTRY_POINT_CONTRACT_OBJECT_TYPE,
        "mfm.structured-certification-predicate-set" => PREDICATE_SET_OBJECT_TYPE,
        "mfm.lane-outcome-contract" => LANE_OUTCOME_CONTRACT_OBJECT_TYPE,
        "mfm.fan-out-join-contract" => FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE,
        _ => {
            return Err(CertifyError::Certification(format!(
                "unregistered structured contract schema: {schema_name}"
            )))
        }
    })
}

fn semantic_component_object_type(kind: StructuredComponentKind) -> Result<StableId> {
    stable_id(match kind {
        StructuredComponentKind::State => STATE_CONTRACT_OBJECT_TYPE,
        StructuredComponentKind::Capability => CAPABILITY_CONTRACT_OBJECT_TYPE,
        StructuredComponentKind::Adapter => ADAPTER_CONTRACT_OBJECT_TYPE,
        StructuredComponentKind::Signer => SIGNER_CONTRACT_OBJECT_TYPE,
        StructuredComponentKind::Resource => RESOURCE_CONTRACT_OBJECT_TYPE,
    })
}

fn insert_exact<K, V>(map: &mut BTreeMap<K, V>, key: K, value: V, label: &str) -> Result<()>
where
    K: Ord,
    V: PartialEq,
{
    match map.get(&key) {
        Some(existing) if existing == &value => Ok(()),
        Some(_) => Err(CertifyError::Certification(format!(
            "conflicting {label} registration"
        ))),
        None => {
            map.insert(key, value);
            Ok(())
        }
    }
}

#[cfg(test)]
mod closure_walker_tests {
    use super::*;

    fn reference(content_ref: &ContentRef) -> ComponentObjectReference {
        ComponentObjectReference {
            object_type: stable_id(DATA_CONTRACT_OBJECT_TYPE).expect("contract type"),
            content_ref: content_ref.clone(),
        }
    }

    fn walker(objects: &BTreeMap<ContentRef, RegisteredComponentObject>) -> ClosureWalker<'_> {
        ClosureWalker {
            objects,
            active: BTreeSet::new(),
            completed: BTreeSet::new(),
            content_types: BTreeMap::new(),
            emitted: Vec::new(),
        }
    }

    #[test]
    fn closure_walk_rejects_an_active_stack_cycle() {
        let left_ref = typed_content_ref("mfm.closure-walker-test", &"left").expect("left ref");
        let right_ref = typed_content_ref("mfm.closure-walker-test", &"right").expect("right ref");
        let left = component_object(
            DATA_CONTRACT_OBJECT_TYPE,
            left_ref.clone(),
            &"left",
            vec![reference(&right_ref)],
        )
        .expect("left object");
        let right = component_object(
            DATA_CONTRACT_OBJECT_TYPE,
            right_ref.clone(),
            &"right",
            vec![reference(&left_ref)],
        )
        .expect("right object");
        let objects = BTreeMap::from([(left_ref.clone(), left), (right_ref, right)]);
        let error = walker(&objects)
            .walk(
                stable_id(DATA_CONTRACT_OBJECT_TYPE).expect("contract type"),
                left_ref,
            )
            .expect_err("an active pair cannot recur");
        assert!(error.to_string().contains("contains a cycle"));
    }

    #[test]
    fn closure_walk_deduplicates_a_shared_dag_and_rejects_type_aliasing() {
        let root_ref = typed_content_ref("mfm.closure-walker-test", &"root").expect("root ref");
        let left_ref = typed_content_ref("mfm.closure-walker-test", &"left").expect("left ref");
        let right_ref = typed_content_ref("mfm.closure-walker-test", &"right").expect("right ref");
        let shared_ref =
            typed_content_ref("mfm.closure-walker-test", &"shared").expect("shared ref");
        let objects = BTreeMap::from([
            (
                root_ref.clone(),
                component_object(
                    DATA_CONTRACT_OBJECT_TYPE,
                    root_ref.clone(),
                    &"root",
                    vec![reference(&left_ref), reference(&right_ref)],
                )
                .expect("root object"),
            ),
            (
                left_ref.clone(),
                component_object(
                    DATA_CONTRACT_OBJECT_TYPE,
                    left_ref,
                    &"left",
                    vec![reference(&shared_ref)],
                )
                .expect("left object"),
            ),
            (
                right_ref.clone(),
                component_object(
                    DATA_CONTRACT_OBJECT_TYPE,
                    right_ref,
                    &"right",
                    vec![reference(&shared_ref)],
                )
                .expect("right object"),
            ),
            (
                shared_ref.clone(),
                component_object(DATA_CONTRACT_OBJECT_TYPE, shared_ref, &"shared", Vec::new())
                    .expect("shared object"),
            ),
        ]);
        let mut walker = walker(&objects);
        walker
            .walk(
                stable_id(DATA_CONTRACT_OBJECT_TYPE).expect("contract type"),
                root_ref.clone(),
            )
            .expect("shared DAG walk");
        assert_eq!(
            walker
                .emitted
                .iter()
                .map(|object| object.value.as_json().as_str().expect("string object"))
                .collect::<Vec<_>>(),
            ["root", "left", "shared", "right"]
        );
        let error = walker
            .walk(
                stable_id(STATE_CONTRACT_OBJECT_TYPE).expect("state type"),
                root_ref,
            )
            .expect_err("one content ref cannot carry two object-type tags");
        assert!(error.to_string().contains("different object type"));
    }
}

#[cfg(test)]
#[path = "structured/failure_mapping_tests.rs"]
mod failure_mapping_tests;

#[cfg(test)]
#[path = "structured/process_handle_tests.rs"]
mod process_handle_tests;
