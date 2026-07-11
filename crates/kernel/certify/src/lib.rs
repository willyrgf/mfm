#![warn(missing_docs)]
//! Certification contracts for MFM typed execution specs.
//!
//! This crate lowers typed program drafts into `mfm_spec::v1` specs and validates
//! that a v1 typed execution spec is the only semantic runtime contract.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{CapabilitySet, CapabilitySetDescriptor, CapabilitySetFor, NoCaps};
use mfm_effects::{ApplySideEffect, EffectClass, EffectSpec, ManagedPlatformWrite, Pure};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EffectKind,
    NodeId, OperationInstanceId, RunId, SchemaId, ScopeId, SemanticTypeId, SpecHash, StateKind,
    StateVersion,
};
use mfm_program as program;
use mfm_spec::v1 as spec;
use mfm_spec::SideEffectVerifyPairErrorKind;
use mfm_values::MfmConfig;

#[path = "registry.rs"]
mod registry;
pub use self::registry::{CertificationRegistry, ConfigValidationSource};
use self::registry::{ConfigValidator, ContextValidator};
#[path = "lowering.rs"]
mod lowering;
use self::lowering::{CellInfo, DraftLowerer};
#[path = "validation.rs"]
mod validation;
use self::validation::{
    fact_descriptor_hashes_for_spec, validate_typed_spec, ConfigIndex, DescriptorIndex,
};

/// Result type for typed certification.
pub type Result<T> = std::result::Result<T, CertifyError>;

/// Version string for the v1 persisted typed-spec certificate.
pub const CERTIFICATE_VERSION: &str = "mfm.certified_typed_spec_certificate.v1";
/// Media type for canonical persisted typed-spec certificate JSON.
pub const CERTIFICATE_MEDIA_TYPE: &str =
    "application/vnd.mfm.certified-typed-spec-certificate+json;version=1";
/// Stable identifier for the v1 certification algorithm.
pub const CERTIFIER_ALGORITHM: &str = "mfm-certify.registry-validation.v1";
/// Stable identifier for the v1 registry digest payload.
pub const REGISTRY_DIGEST_ALGORITHM: &str = "mfm-certify.registry-digest.v1";
/// Supported digest-only signing scheme for certified manual resolution decisions.
pub const MANUAL_RESOLUTION_SIGNING_SCHEME: &str = "mfm.manual_resolution.digest_signature.v1";

/// Canonical fact descriptor artifact material trusted by the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDescriptorArtifact {
    descriptor_hash: ContentDigest,
    bytes: Vec<u8>,
}

impl FactDescriptorArtifact {
    /// Returns the canonical descriptor content hash.
    pub fn descriptor_hash(&self) -> &ContentDigest {
        &self.descriptor_hash
    }

    /// Returns the canonical descriptor bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Returns the schema id for canonical persisted v1 typed-spec certificates.
pub fn typed_spec_certificate_schema_id() -> Result<SchemaId> {
    let digest = content_digest_json(serde_json::json!({
        "fields": [
            "certificate_version",
            "media_type",
            "certifier_algorithm",
            "spec_hash",
            "registry_digest",
            "descriptor_identities",
            "schema_role_grants",
            "manual_authorization_verifiers",
            "operator_authority_snapshots",
            "certificate_hash",
        ],
        "media_type": CERTIFICATE_MEDIA_TYPE,
        "name": "mfm.certified_typed_spec_certificate",
        "version": "1",
    }))?;
    SchemaId::new(
        "mfm.certified_typed_spec_certificate",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )
    .map_err(|error| CertifyError::Certificate(error.to_string()))
}

/// Problem taxonomy class rejected by typed certification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProblemClass {
    /// Duplicate, missing, cyclic, or otherwise impossible graph structure.
    InvalidTopology,
    /// Producer/consumer interface metadata does not match the referenced cell.
    InvalidInterfaceWiring,
    /// Effect, capability, descriptor, runner, or side-effect transition evidence is invalid.
    InvalidSemanticTransition,
    /// Canonical data, descriptor, config, dynamic ordering, or persisted shape evidence is invalid.
    InvalidDataShape,
    /// Value lineage, scope, producer, or domain meaning evidence is invalid.
    InvalidDataMeaning,
    /// Public output, render node, or terminal-output evidence is invalid.
    InvalidTerminalShape,
}

impl ProblemClass {
    /// Returns the stable summary key used by typed-kernel contract checks.
    pub const fn summary_key(self) -> &'static str {
        match self {
            Self::InvalidTopology => "invalid_topology_rejected",
            Self::InvalidInterfaceWiring => "invalid_interface_wiring_rejected",
            Self::InvalidSemanticTransition => "invalid_semantic_transition_rejected",
            Self::InvalidDataShape => "invalid_data_shape_rejected",
            Self::InvalidDataMeaning => "invalid_data_meaning_rejected",
            Self::InvalidTerminalShape => "invalid_terminal_shape_rejected",
        }
    }
}

/// Certification failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertifyError {
    /// A typed-core problem class was rejected.
    #[error("{class:?}: {message}")]
    Problem {
        /// Rejected problem class.
        class: ProblemClass,
        /// Stable diagnostic.
        message: String,
    },
    /// Lowering a program draft failed before v1 validation.
    #[error("typed lowering failed: {0}")]
    Lowering(String),
    /// Canonicalization or hashing failed.
    #[error("canonicalization failed: {0}")]
    Canonical(String),
    /// Spec contract validation failed.
    #[error("typed spec failed: {0}")]
    Spec(String),
    /// Persisted spec/certificate verification failed.
    #[error("typed spec certificate failed: {0}")]
    Certificate(String),
}

impl CertifyError {
    /// Returns this error's problem class when it is a taxonomy rejection.
    pub fn problem_class(&self) -> Option<ProblemClass> {
        match self {
            Self::Problem { class, .. } => Some(*class),
            Self::Lowering(_) | Self::Canonical(_) | Self::Spec(_) | Self::Certificate(_) => None,
        }
    }
}

impl From<program::RegistryError> for CertifyError {
    fn from(error: program::RegistryError) -> Self {
        Self::Lowering(error.to_string())
    }
}

/// Defines matching program and certification registry functions from one descriptor inventory.
#[macro_export]
macro_rules! define_program_descriptor_registry {
    (
        state_registry: $state_vis:vis $state_registry:ident,
        operation_registry: $operation_vis:vis $operation_registry:ident,
        certification: $cert_vis:vis $certification:ident,
        states: [$($state:ty),* $(,)?],
        operations: [$($operation:ty),* $(,)?] $(,)?
    ) => {
        $crate::define_program_descriptor_registry! {
            state_registry: $state_vis $state_registry,
            operation_registry: $operation_vis $operation_registry,
            certification: $cert_vis $certification,
            states: [$($state),*],
            operations: [$($operation),*],
            after_registration:,
        }
    };
    (
        state_registry: $state_vis:vis $state_registry:ident,
        operation_registry: $operation_vis:vis $operation_registry:ident,
        certification: $cert_vis:vis $certification:ident,
        states: [$($state:ty),* $(,)?],
        operations: [$($operation:ty),* $(,)?],
        after_registration: $($after:path)?,
    ) => {
        #[doc = "Builds the state registry used for typed authoring and certification."]
        $state_vis fn $state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
            let mut states = mfm_program::StateRegistryBuilder::new();
            $(states.register::<$state>()?;)*
            Ok(states.into_snapshot())
        }

        #[doc = "Builds the operation registry used for typed authoring and certification."]
        $operation_vis fn $operation_registry() -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
            let mut operations = mfm_program::OperationRegistryBuilder::new();
            $(operations.register::<$operation>()?;)*
            Ok(operations.into_snapshot())
        }

        #[doc = "Adds program descriptors to a trusted certification registry."]
        $cert_vis fn $certification(
            registry: &mut $crate::CertificationRegistry,
        ) -> $crate::Result<()> {
            $(registry.register_state::<$state>()?;)*
            $(registry.register_operation::<$operation>()?;)*

            $($after(registry)?;)?
            Ok(())
        }
    };
}

/// Program-lowered typed spec data.
///
/// This is the output of deterministic program lowering. It is still not certification or runtime
/// authority because the registry-backed certificate has not been minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredTypedSpec {
    spec: spec::TypedExecutionSpec,
}

impl LoweredTypedSpec {
    fn new(spec: spec::TypedExecutionSpec) -> Self {
        Self { spec }
    }

    /// Returns the lowered spec data without granting runtime authority.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        &self.spec
    }

    fn into_raw_spec(self) -> spec::TypedExecutionSpec {
        self.spec
    }
}

/// Certifier-validated typed spec body.
///
/// The fields and constructor are private so only registry-backed certification in this crate can
/// mint this authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTypedExecutionSpec {
    envelope: spec::HashedSpecEnvelope,
    graph: CertifiedSpecGraph,
    _seal: ValidatedSpecSeal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ValidatedSpecSeal;

impl ValidatedTypedExecutionSpec {
    fn new(envelope: spec::HashedSpecEnvelope, graph: CertifiedSpecGraph) -> Self {
        Self {
            envelope,
            graph,
            _seal: ValidatedSpecSeal,
        }
    }

    /// Returns the hash-only spec envelope that was validated by the certifier.
    pub fn envelope(&self) -> &spec::HashedSpecEnvelope {
        &self.envelope
    }

    /// Returns the validated hash-defining spec data.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        &self.envelope.spec
    }

    /// Returns the certified graph authority minted for this spec.
    pub fn graph(&self) -> &CertifiedSpecGraph {
        &self.graph
    }

    /// Returns the validated canonical spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.envelope.spec_hash
    }

    fn into_parts(self) -> (spec::HashedSpecEnvelope, CertifiedSpecGraph) {
        (self.envelope, self.graph)
    }
}

/// Registry-backed descriptor authority for a certified typed spec.
///
/// The fields are private so callers can inspect descriptors only after this crate has validated
/// descriptor identities against the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedDescriptorSet {
    states: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
    operations: BTreeMap<DescriptorId, spec::OperationDescriptorIdentity>,
    renderers: BTreeMap<DescriptorId, spec::RendererDescriptorIdentity>,
}

impl CertifiedDescriptorSet {
    fn from_descriptor_index(descriptors: &DescriptorIndex<'_>) -> Self {
        Self {
            states: descriptors
                .states
                .values()
                .map(|descriptor| (descriptor.descriptor_id.clone(), (*descriptor).clone()))
                .collect(),
            operations: descriptors
                .operations
                .values()
                .map(|descriptor| (descriptor.descriptor_id.clone(), (*descriptor).clone()))
                .collect(),
            renderers: descriptors
                .renderers
                .values()
                .map(|descriptor| (descriptor.descriptor_id.clone(), (*descriptor).clone()))
                .collect(),
        }
    }

    /// Returns a certified state descriptor by id.
    pub fn state(&self, id: &DescriptorId) -> Option<&spec::StateDescriptorIdentity> {
        self.states.get(id)
    }

    /// Returns a certified operation descriptor by id.
    pub fn operation(&self, id: &DescriptorId) -> Option<&spec::OperationDescriptorIdentity> {
        self.operations.get(id)
    }

    /// Returns a certified renderer descriptor by id.
    pub fn renderer(&self, id: &DescriptorId) -> Option<&spec::RendererDescriptorIdentity> {
        self.renderers.get(id)
    }

    /// Iterates certified state descriptors in deterministic id order.
    pub fn state_descriptors(
        &self,
    ) -> impl Iterator<Item = (&DescriptorId, &spec::StateDescriptorIdentity)> {
        self.states.iter()
    }
}

/// Certified graph and index authority for one typed execution spec.
///
/// This is minted only after the certifier has validated the persisted DTO shape, descriptor
/// authority, graph topology, lineage references, config references, public outputs, and saga
/// structure. Consumers that need graph facts should depend on this authority instead of scanning
/// raw `mfm_spec::v1` vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSpecGraph {
    scope_ids: BTreeSet<String>,
    descriptors: CertifiedDescriptorSet,
    config_refs: ConfigIndex,
    value_lineages: BTreeMap<String, spec::ValueLineage>,
    cells: BTreeMap<String, spec::CellSpec>,
    forward_nodes: BTreeMap<String, spec::NodeSpec>,
    remediations: BTreeMap<NodeId, spec::NodeSpec>,
}

impl CertifiedSpecGraph {
    fn from_validated_parts(
        scope_ids: BTreeSet<String>,
        descriptors: &DescriptorIndex<'_>,
        config_refs: ConfigIndex,
        value_lineages: BTreeMap<String, spec::ValueLineage>,
        cells: BTreeMap<String, spec::CellSpec>,
        forward_nodes: BTreeMap<String, spec::NodeSpec>,
        remediations: BTreeMap<NodeId, spec::NodeSpec>,
    ) -> Self {
        Self {
            scope_ids,
            descriptors: CertifiedDescriptorSet::from_descriptor_index(descriptors),
            config_refs,
            value_lineages,
            cells,
            forward_nodes,
            remediations,
        }
    }

    /// Returns the certified descriptor authority for this graph.
    pub fn descriptors(&self) -> &CertifiedDescriptorSet {
        &self.descriptors
    }

    /// Returns the number of certified scopes.
    pub fn scope_count(&self) -> usize {
        self.scope_ids.len()
    }

    /// Iterates certified scope ids in deterministic order.
    pub fn scope_ids(&self) -> impl Iterator<Item = &str> {
        self.scope_ids.iter().map(String::as_str)
    }

    /// Returns the number of certified config refs.
    pub fn config_ref_count(&self) -> usize {
        self.config_refs.keys.len()
    }

    /// Returns the number of certified value lineages.
    pub fn value_lineage_count(&self) -> usize {
        self.value_lineages.len()
    }

    /// Returns the number of certified cells.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Iterates certified cells in deterministic id order.
    pub fn cells(&self) -> impl Iterator<Item = (&CellId, &spec::CellSpec)> {
        self.cells.values().map(|cell| (&cell.cell_id, cell))
    }

    /// Returns the certified cell for `cell_id`, when present.
    pub fn cell(&self, cell_id: &CellId) -> Option<&spec::CellSpec> {
        self.cells.get(cell_id.as_str())
    }

    /// Returns the number of certified forward nodes.
    pub fn forward_node_count(&self) -> usize {
        self.forward_nodes.len()
    }

    /// Returns the certified forward node for `node_id`, when present.
    pub fn forward_node(&self, node_id: &NodeId) -> Option<&spec::NodeSpec> {
        self.forward_nodes.get(node_id.as_str())
    }

    /// Iterates certified forward nodes in deterministic id order.
    pub fn forward_nodes(&self) -> impl Iterator<Item = &spec::NodeSpec> {
        self.forward_nodes.values()
    }

    /// Returns the number of certified remediation nodes.
    pub fn remediation_count(&self) -> usize {
        self.remediations.len()
    }

    /// Returns the certified remediation node linked to `forward_node_id`, when present.
    pub fn remediation_for_forward_node(
        &self,
        forward_node_id: &NodeId,
    ) -> Option<&spec::NodeSpec> {
        self.remediations.get(forward_node_id)
    }

    /// Iterates certified remediation nodes keyed by their forward side-effect node id.
    pub fn remediations(&self) -> impl Iterator<Item = (&NodeId, &spec::NodeSpec)> {
        self.remediations.iter()
    }
}

/// Certified framework node role in the static lifecycle tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedFrameworkNodeRole {
    node_id: NodeId,
    output_cell: CellId,
    descriptor_id: DescriptorId,
}

impl CertifiedFrameworkNodeRole {
    fn from_node(node: &spec::NodeSpec) -> Self {
        Self {
            node_id: node.node_id.clone(),
            output_cell: node.output_cell.clone(),
            descriptor_id: node.descriptor_id.clone(),
        }
    }

    /// Returns the certified framework node id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the receipt cell produced by the certified framework node.
    pub fn output_cell(&self) -> &CellId {
        &self.output_cell
    }

    /// Returns the certified framework state descriptor id.
    pub fn descriptor_id(&self) -> &DescriptorId {
        &self.descriptor_id
    }
}

/// Certified static framework lifecycle authority for a typed spec.
///
/// This view is minted only after certifier validation has checked lifecycle node roles, receipt
/// wiring, output contracts, framework config refs, renderer linkage, and lifecycle tail finality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedFrameworkLifecycle {
    render: CertifiedFrameworkNodeRole,
    retention: CertifiedFrameworkNodeRole,
    complete: CertifiedFrameworkNodeRole,
    resolve: CertifiedFrameworkNodeRole,
}

impl CertifiedFrameworkLifecycle {
    fn from_validated_spec(validated: &ValidatedTypedExecutionSpec) -> Result<Self> {
        let descriptors = validated.graph().descriptors();
        let mut render = None;
        let mut retention = None;
        let mut complete = None;
        let mut resolve = None;

        for node in validated.graph().forward_nodes() {
            if descriptors.state(&node.descriptor_id).is_none() {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "framework lifecycle node {} references missing descriptor {}",
                        node.node_id, node.descriptor_id
                    ),
                ));
            }
            match &node.framework {
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
                    set_framework_role(&mut render, node, "public-output render node")?;
                }
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                    set_framework_role(&mut retention, node, "retention lifecycle node")?;
                }
                Some(spec::FrameworkNodeSpec::CompleteRun(_)) => {
                    set_framework_role(&mut complete, node, "complete-run lifecycle node")?;
                }
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) => {
                    set_framework_role(&mut resolve, node, "resolve-saga-terminal lifecycle node")?;
                }
                Some(spec::FrameworkNodeSpec::Bridge(_))
                | Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
                | None => {}
            }
        }

        Ok(Self {
            render: take_framework_role(render, "public-output render node")?,
            retention: take_framework_role(retention, "retention lifecycle node")?,
            complete: take_framework_role(complete, "complete-run lifecycle node")?,
            resolve: take_framework_role(resolve, "resolve-saga-terminal lifecycle node")?,
        })
    }

    /// Returns the certified public-output render receipt producer.
    pub fn render(&self) -> &CertifiedFrameworkNodeRole {
        &self.render
    }

    /// Returns the certified retention manifest receipt producer.
    pub fn retention(&self) -> &CertifiedFrameworkNodeRole {
        &self.retention
    }

    /// Returns the certified completion receipt producer.
    pub fn complete(&self) -> &CertifiedFrameworkNodeRole {
        &self.complete
    }

    /// Returns the certified saga-terminal resolution receipt producer.
    pub fn resolve(&self) -> &CertifiedFrameworkNodeRole {
        &self.resolve
    }
}

fn set_framework_role(
    slot: &mut Option<CertifiedFrameworkNodeRole>,
    node: &spec::NodeSpec,
    label: &'static str,
) -> Result<()> {
    if slot
        .replace(CertifiedFrameworkNodeRole::from_node(node))
        .is_some()
    {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!("expected exactly one {label}"),
        ));
    }
    Ok(())
}

fn take_framework_role(
    role: Option<CertifiedFrameworkNodeRole>,
    label: &'static str,
) -> Result<CertifiedFrameworkNodeRole> {
    role.ok_or_else(|| {
        problem(
            ProblemClass::InvalidTopology,
            format!("missing certified {label}"),
        )
    })
}

/// Input accepted by typed-spec certification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedSpecCertificationInput {
    /// Hostile parsed or constructed spec data.
    Untrusted(spec::UntrustedTypedSpec),
    /// Deterministically lowered program output.
    Lowered(LoweredTypedSpec),
}

impl From<spec::UntrustedTypedSpec> for TypedSpecCertificationInput {
    fn from(spec: spec::UntrustedTypedSpec) -> Self {
        Self::Untrusted(spec)
    }
}

impl From<LoweredTypedSpec> for TypedSpecCertificationInput {
    fn from(spec: LoweredTypedSpec) -> Self {
        Self::Lowered(spec)
    }
}

impl TypedSpecCertificationInput {
    fn into_raw_spec(self) -> spec::TypedExecutionSpec {
        match self {
            Self::Untrusted(spec) => spec.into_raw_spec(),
            Self::Lowered(spec) => spec.into_raw_spec(),
        }
    }
}

/// Certified side-effect resource and remediation contract for one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSideEffectContract {
    node_id: NodeId,
    side_effect: spec::SideEffectContractSpec,
    remediation_forward_node_id: Option<NodeId>,
}

impl CertifiedSideEffectContract {
    /// Mints a side-effect contract from a certified typed spec and node id.
    pub fn for_node(spec: &spec::TypedExecutionSpec, node_id: &NodeId) -> Result<Self> {
        let node = spec
            .nodes
            .iter()
            .chain(spec.remediations.values())
            .find(|node| node.node_id == *node_id)
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("side-effect contract requested for uncertified node {node_id}"),
                )
            })?;
        let side_effect = node.side_effect.clone().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("node {node_id} has no certified side-effect contract"),
            )
        })?;
        let remediation_forward_node_id =
            spec.remediations
                .iter()
                .find_map(|(forward_node_id, remediation)| {
                    (remediation.node_id == *node_id).then_some(forward_node_id.clone())
                });
        Ok(Self {
            node_id: node_id.clone(),
            side_effect,
            remediation_forward_node_id,
        })
    }

    /// Returns the node id this contract certifies.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the certified side-effect resource claim.
    pub fn resource_claim(&self) -> &spec::ResourceClaimSpec {
        &self.side_effect.resource_claim
    }

    /// Returns the certified terminal verification policy.
    pub fn verification(&self) -> &spec::SideEffectVerificationSpec {
        &self.side_effect.verification
    }

    /// Validates whether a ledger purpose is admissible for this node.
    pub fn validate_ledger_purpose(&self, purpose: &events::SideEffectLedgerPurpose) -> Result<()> {
        match (&self.remediation_forward_node_id, purpose) {
            (None, events::SideEffectLedgerPurpose::Forward)
            | (Some(_), events::SideEffectLedgerPurpose::Remediation { .. }) => Ok(()),
            (None, events::SideEffectLedgerPurpose::Remediation { .. }) => Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "forward side-effect node {} emitted remediation ledger purpose",
                    self.node_id
                ),
            )),
            (Some(_), events::SideEffectLedgerPurpose::Forward) => Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} emitted forward ledger purpose",
                    self.node_id
                ),
            )),
        }
    }

    /// Validates exclusive resource-key evidence against the certified resource claim.
    pub fn validate_resource_key(
        &self,
        resource_key: Option<&events::ResourceKeyEvidence>,
    ) -> Result<()> {
        match &self.side_effect.resource_claim {
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            } => {
                let Some(resource_key) = resource_key else {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "exclusive side-effect node {} recorded without resource key evidence",
                            self.node_id
                        ),
                    ));
                };
                if &resource_key.namespace != namespace || &resource_key.key_schema_id != key_schema
                {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "exclusive side-effect node {} recorded resource key evidence outside certified schema",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
            spec::ResourceClaimSpec::ExactTouchedSet { .. }
            | spec::ResourceClaimSpec::ManualOnly => {
                if resource_key.is_some() {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "side-effect node {} recorded exclusive resource key without an exclusive certified resource claim",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Validates exact touched-set evidence against the certified resource claim.
    pub fn validate_touched_set(
        &self,
        touched_set: Option<&events::ResourceTouchedSetEvidence>,
    ) -> Result<()> {
        match &self.side_effect.resource_claim {
            spec::ResourceClaimSpec::ExactTouchedSet {
                namespace,
                evidence_schema,
            } => {
                let Some(touched_set) = touched_set else {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "exact-touched-set side-effect node {} recorded without touched-set evidence",
                            self.node_id
                        ),
                    ));
                };
                if &touched_set.namespace != namespace
                    || &touched_set.evidence_schema_id != evidence_schema
                {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "side-effect node {} recorded touched-set evidence outside certified schema",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
            spec::ResourceClaimSpec::Exclusive { .. } | spec::ResourceClaimSpec::ManualOnly => {
                if touched_set.is_some() {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "side-effect node {} recorded touched-set evidence without an exact-touched-set certified resource claim",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Validates that a resource key remains stable across invocation epochs.
    pub fn validate_epoch_resource_consistency(
        &self,
        previous: Option<&events::ResourceKeyEvidence>,
        current: Option<&events::ResourceKeyEvidence>,
    ) -> Result<()> {
        self.validate_resource_key(current)?;
        if let (Some(previous), Some(current)) = (previous, current) {
            if previous != current {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "exclusive side-effect node {} changed resource key across invocation epochs",
                        self.node_id
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Validates a remediation ledger link against certified forward/remediation relations.
    pub fn validate_remediation_link(&self, link: CertifiedRemediationLink<'_>) -> Result<()> {
        self.validate_ledger_purpose(link.ledger_purpose)?;
        let events::SideEffectLedgerPurpose::Remediation { .. } = link.ledger_purpose else {
            return Ok(());
        };
        let expected_forward_node_id =
            self.remediation_forward_node_id.as_ref().ok_or_else(|| {
                problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "forward side-effect node {} cannot emit remediation ledger purpose",
                        self.node_id
                    ),
                )
            })?;
        let Some(forward_run_id) = link.forward_run_id else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked missing forward ledger",
                    self.node_id
                ),
            ));
        };
        let Some(forward_node_id) = link.forward_node_id else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked missing forward node",
                    self.node_id
                ),
            ));
        };
        let Some(forward_ledger_purpose) = link.forward_ledger_purpose else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked missing forward ledger purpose",
                    self.node_id
                ),
            ));
        };
        if forward_run_id != link.remediation_run_id
            || forward_node_id != expected_forward_node_id
            || !matches!(
                forward_ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
            || !link.forward_terminal
        {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked ledger outside certified terminal forward node {}",
                    self.node_id, expected_forward_node_id
                ),
            ));
        }
        Ok(())
    }
}

/// Projection facts needed to certify a remediation ledger link without depending on a store type.
#[derive(Debug, Clone, Copy)]
pub struct CertifiedRemediationLink<'a> {
    /// Run id of the remediation ledger.
    pub remediation_run_id: &'a RunId,
    /// Ledger purpose recorded by the remediation event or projection.
    pub ledger_purpose: &'a events::SideEffectLedgerPurpose,
    /// Run id of the linked forward ledger, when projected.
    pub forward_run_id: Option<&'a RunId>,
    /// Node id that owns the linked forward ledger, when projected.
    pub forward_node_id: Option<&'a NodeId>,
    /// Ledger purpose of the linked forward ledger, when projected.
    pub forward_ledger_purpose: Option<&'a events::SideEffectLedgerPurpose>,
    /// Whether the linked forward ledger has certified terminal side-effect evidence.
    pub forward_terminal: bool,
}

/// Non-forgeable certified typed spec ready to become runtime authority.
///
/// The fields are private so only this crate's certifier and verifier can mint the authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedTypedSpec {
    validated: ValidatedTypedExecutionSpec,
    certificate: CertifiedSpecCertificate,
    framework_lifecycle: CertifiedFrameworkLifecycle,
}

impl CertifiedTypedSpec {
    /// Returns the validated typed spec authority.
    pub fn validated_spec(&self) -> &ValidatedTypedExecutionSpec {
        &self.validated
    }

    /// Returns the hash-only spec envelope carried by this certified authority.
    pub fn envelope(&self) -> &spec::HashedSpecEnvelope {
        self.validated.envelope()
    }

    /// Returns the canonical spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        self.validated.spec_hash()
    }

    /// Returns the persisted certificate evidence that was verified or emitted.
    pub fn certificate(&self) -> &CertifiedSpecCertificate {
        &self.certificate
    }

    /// Returns the certificate evidence hash.
    pub fn certificate_hash(&self) -> &ContentDigest {
        &self.certificate.certificate_hash
    }

    /// Returns the certified descriptor authority.
    pub fn descriptor_set(&self) -> &CertifiedDescriptorSet {
        self.validated.graph().descriptors()
    }

    /// Returns the certified framework lifecycle authority.
    pub fn framework_lifecycle(&self) -> &CertifiedFrameworkLifecycle {
        &self.framework_lifecycle
    }

    /// Consumes the authority and returns runtime construction parts.
    pub fn into_parts(self) -> CertifiedTypedSpecParts {
        let (envelope, graph) = self.validated.into_parts();
        CertifiedTypedSpecParts {
            envelope,
            graph,
            certificate: self.certificate,
            framework_lifecycle: self.framework_lifecycle,
        }
    }

    /// Builds canonical persisted spec and certificate bytes for storage.
    pub fn to_persisted_parts(&self) -> Result<PersistedSpecCertificateParts> {
        PersistedSpecCertificateParts::from_certified(self)
    }
}

/// Runtime construction parts carried by a certified typed spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedTypedSpecParts {
    /// Hash-only typed spec envelope covered by the certificate.
    pub envelope: spec::HashedSpecEnvelope,
    /// Certified graph and index authority for the typed spec.
    pub graph: CertifiedSpecGraph,
    /// Verified certificate evidence for the typed spec.
    pub certificate: CertifiedSpecCertificate,
    /// Certified static framework lifecycle authority for the typed spec.
    pub framework_lifecycle: CertifiedFrameworkLifecycle,
}

/// Persisted typed-spec certificate.
///
/// This is durable evidence only. Parsed or constructed certificate data is hostile until
/// [`verify_persisted_spec_certificate`] validates it against a registry and returns
/// [`CertifiedTypedSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSpecCertificate {
    /// Hash of the hash-defining certificate evidence.
    pub certificate_hash: ContentDigest,
    /// Hash-defining certificate evidence.
    pub evidence: CertifiedSpecCertificateEvidence,
}

impl CertifiedSpecCertificate {
    /// Builds persisted certificate evidence and computes its deterministic certificate hash.
    ///
    /// This does not mint runtime authority. Call [`verify_persisted_spec_certificate`] to turn
    /// persisted bytes into [`CertifiedTypedSpec`].
    pub fn from_evidence(evidence: CertifiedSpecCertificateEvidence) -> Result<Self> {
        let certificate_hash = evidence.canonical_json()?.content_digest();
        Ok(Self {
            certificate_hash,
            evidence,
        })
    }

    /// Parses persisted certificate JSON as untrusted certificate data.
    pub fn from_json_slice(input: &[u8]) -> Result<Self> {
        let input = std::str::from_utf8(input)
            .map_err(|error| certificate(format!("certificate JSON is not UTF-8: {error}")))?;
        Self::from_json_str(input)
    }

    /// Parses persisted certificate JSON as untrusted certificate data.
    pub fn from_json_str(input: &str) -> Result<Self> {
        let input_canonical = PlainCanonicalJsonBytes::from_json_str(input)
            .map_err(|error| certificate(error.to_string()))?;
        let value: serde_json::Value =
            serde_json::from_str(input).map_err(|error| certificate(error.to_string()))?;
        let parsed = parse_certificate(&value)?;
        parsed.verify_hash()?;
        if parsed.canonical_json()? != input_canonical {
            return Err(certificate(
                "persisted certificate contains unknown or non-normalized fields",
            ));
        }
        Ok(parsed)
    }

    /// Returns canonical JSON bytes for this persisted certificate.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json_bytes(self.json())
    }

    /// Verifies that `certificate_hash` matches the hash-defining evidence.
    pub fn verify_hash(&self) -> Result<()> {
        let actual = self.evidence.canonical_json()?.content_digest();
        if self.certificate_hash != actual {
            return Err(certificate(format!(
                "certificate hash mismatch: expected {}, recomputed {}",
                self.certificate_hash, actual
            )));
        }
        Ok(())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "certificate_hash": self.certificate_hash.as_str(),
            "evidence": self.evidence.json(),
        })
    }
}

/// Hash-defining certificate evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSpecCertificateEvidence {
    /// Persisted certificate contract version.
    pub certificate_version: String,
    /// Persisted certificate media type.
    pub media_type: String,
    /// Certifier algorithm identity.
    pub certifier_algorithm: String,
    /// Spec hash that this certificate covers.
    pub spec_hash: SpecHash,
    /// Digest of the certification registry authority.
    pub registry_digest: ContentDigest,
    /// Descriptor identities and digests covered by certification.
    pub descriptor_identities: Vec<CertifiedDescriptorEvidence>,
    /// Schema role grants resolved during certification.
    pub schema_role_grants: Vec<CertifiedSchemaRoleGrantEvidence>,
    /// Manual authorization verifier identities resolved during certification.
    pub manual_authorization_verifiers: Vec<CertifiedManualAuthorizationVerifierEvidence>,
    /// Operator authority snapshots resolved during certification.
    pub operator_authority_snapshots: Vec<CertifiedOperatorAuthoritySnapshotEvidence>,
}

impl CertifiedSpecCertificateEvidence {
    /// Returns canonical JSON bytes for the hash-defining certificate evidence.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json_bytes(self.json())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "certificate_version": self.certificate_version.as_str(),
            "certifier_algorithm": self.certifier_algorithm.as_str(),
            "descriptor_identities": self
                .descriptor_identities
                .iter()
                .map(CertifiedDescriptorEvidence::json)
                .collect::<Vec<_>>(),
            "manual_authorization_verifiers": self
                .manual_authorization_verifiers
                .iter()
                .map(CertifiedManualAuthorizationVerifierEvidence::json)
                .collect::<Vec<_>>(),
            "media_type": self.media_type.as_str(),
            "operator_authority_snapshots": self
                .operator_authority_snapshots
                .iter()
                .map(CertifiedOperatorAuthoritySnapshotEvidence::json)
                .collect::<Vec<_>>(),
            "registry_digest": self.registry_digest.as_str(),
            "schema_role_grants": self
                .schema_role_grants
                .iter()
                .map(CertifiedSchemaRoleGrantEvidence::json)
                .collect::<Vec<_>>(),
            "spec_hash": self.spec_hash.as_str(),
        })
    }
}

/// Certified schema role in the certification registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CertifiedSchemaRole {
    /// Schema may be used as manual resolution evidence.
    ManualResolutionEvidence,
    /// Schema may be used as manual resolution authorization proof evidence.
    ManualResolutionAuthorization,
}

impl CertifiedSchemaRole {
    /// Returns the stable persisted schema-role string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManualResolutionEvidence => "manual_resolution_evidence",
            Self::ManualResolutionAuthorization => "manual_resolution_authorization",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "manual_resolution_evidence" => Ok(Self::ManualResolutionEvidence),
            "manual_resolution_authorization" => Ok(Self::ManualResolutionAuthorization),
            other => Err(certificate(format!(
                "unknown certified schema role {other:?}"
            ))),
        }
    }
}

/// Schema role grant resolved from the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSchemaRoleGrantEvidence {
    /// Schema id that received the grant.
    pub schema_id: SchemaId,
    /// Certified schema role.
    pub role: CertifiedSchemaRole,
}

impl CertifiedSchemaRoleGrantEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "role": self.role.as_str(),
            "schema_id": self.schema_id.as_str(),
        })
    }
}

/// Manual authorization verifier resolved from the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedManualAuthorizationVerifierEvidence {
    /// Verifier id trusted by the registry.
    pub verifier_id: spec::ManualAuthorizationVerifierId,
}

impl CertifiedManualAuthorizationVerifierEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "verifier_id": self.verifier_id.as_str(),
        })
    }
}

/// Operator authority snapshot resolved from the certification registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedOperatorAuthoritySnapshotEvidence {
    /// Authority id trusted by the registry.
    pub authority_id: spec::OperatorAuthorityId,
    /// Digest of the certified authority snapshot.
    pub authority_digest: ContentDigest,
    /// Operators included in the certified snapshot.
    pub operators: Vec<CertifiedOperatorAuthorityMemberEvidence>,
}

impl CertifiedOperatorAuthoritySnapshotEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "authority_digest": self.authority_digest.as_str(),
            "authority_id": self.authority_id.as_str(),
            "operators": self
                .operators
                .iter()
                .map(CertifiedOperatorAuthorityMemberEvidence::json)
                .collect::<Vec<_>>(),
        })
    }
}

/// Operator member included in certified operator authority snapshot evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedOperatorAuthorityMemberEvidence {
    /// Stable operator id.
    pub operator_id: spec::OperatorId,
    /// Public operator identity.
    pub public_identity: spec::OperatorPublicIdentity,
}

impl CertifiedOperatorAuthorityMemberEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "operator_id": self.operator_id.as_str(),
            "public_identity": self.public_identity.as_str(),
        })
    }
}

/// Descriptor family covered by a certificate.
pub type CertifiedDescriptorFamily = spec::DescriptorFamily;

/// Descriptor identity and digest covered by a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedDescriptorEvidence {
    /// Descriptor family.
    pub descriptor_family: CertifiedDescriptorFamily,
    /// Descriptor identity.
    pub descriptor_id: DescriptorId,
    /// Canonical digest of the descriptor identity payload.
    pub descriptor_digest: ContentDigest,
}

impl CertifiedDescriptorEvidence {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "descriptor_digest": self.descriptor_digest.as_str(),
            "descriptor_family": self.descriptor_family.as_str(),
            "descriptor_id": self.descriptor_id.as_str(),
        })
    }
}

/// Canonical persisted bytes for a certified spec and its certificate.
///
/// This is a storage container only. Parsing it yields [`UntrustedSpecCertificateParts`], not
/// runtime authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedSpecCertificateParts {
    spec_bytes: Vec<u8>,
    certificate_bytes: Vec<u8>,
}

impl PersistedSpecCertificateParts {
    /// Builds persisted canonical bytes from an in-memory certified authority.
    pub fn from_certified(certified: &CertifiedTypedSpec) -> Result<Self> {
        Ok(Self {
            spec_bytes: certified
                .validated_spec()
                .spec()
                .canonical_json()
                .map_err(|error| CertifyError::Spec(error.to_string()))?
                .to_vec(),
            certificate_bytes: certified.certificate.canonical_json()?.to_vec(),
        })
    }

    /// Wraps untrusted persisted bytes for parsing and later verification.
    pub fn from_untrusted_bytes(spec_bytes: Vec<u8>, certificate_bytes: Vec<u8>) -> Self {
        Self {
            spec_bytes,
            certificate_bytes,
        }
    }

    /// Returns the persisted spec bytes.
    pub fn spec_bytes(&self) -> &[u8] {
        &self.spec_bytes
    }

    /// Returns the persisted certificate bytes.
    pub fn certificate_bytes(&self) -> &[u8] {
        &self.certificate_bytes
    }

    /// Parses the persisted bytes as hostile data.
    pub fn parse_untrusted(&self) -> Result<UntrustedSpecCertificateParts> {
        let spec = spec::UntrustedTypedSpec::from_json_slice(&self.spec_bytes)
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let certificate = CertifiedSpecCertificate::from_json_slice(&self.certificate_bytes)?;
        Ok(UntrustedSpecCertificateParts { spec, certificate })
    }
}

/// Parsed persisted spec and certificate data that has not been verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrustedSpecCertificateParts {
    spec: spec::UntrustedTypedSpec,
    certificate: CertifiedSpecCertificate,
}

impl UntrustedSpecCertificateParts {
    /// Returns the parsed typed spec data.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        self.spec.spec()
    }

    /// Returns the parsed certificate data.
    pub fn certificate(&self) -> &CertifiedSpecCertificate {
        &self.certificate
    }
}

fn config_validator_for<C: MfmConfig>() -> Result<ConfigValidator> {
    let schema_id = C::schema_id().map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("config schema descriptor invalid: {error}"),
        )
    })?;
    Ok(ConfigValidator {
        schema_id,
        validate: validate_config_bytes_for::<C>,
    })
}

fn context_validator_for<C: program::StateContext>() -> Result<Option<ContextValidator>> {
    let descriptor = C::descriptor().map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("context schema descriptor invalid: {error}"),
        )
    })?;
    let spec::StateContextDescriptorSpec::Required(requirement) = descriptor else {
        return Ok(None);
    };
    Ok(Some(ContextValidator {
        requirement: *requirement,
        validate: validate_context_spec_for::<C>,
    }))
}

fn validate_context_spec_for<C: program::StateContext>(
    context: &spec::CertifiedContextSpec,
) -> program::Result<()> {
    C::materialize_certified(Some(context)).map(|_| ())
}

fn validate_config_bytes_for<C: MfmConfig>(bytes: &[u8]) -> Result<()> {
    let config: C = serde_json::from_slice(bytes).map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("typed config did not match registered schema: {error}"),
        )
    })?;
    config.validate().map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("typed config failed validation: {error}"),
        )
    })?;
    let encoded = serde_json::to_string(&config).map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("typed config could not be serialized canonically: {error}"),
        )
    })?;
    let expected = PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("typed config canonical encoding was invalid: {error}"),
        )
    })?;
    if expected.as_bytes() != bytes {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "typed config did not match registered canonical encoding",
        ));
    }
    Ok(())
}

fn canonical_config_bytes_for_ref(
    config_ref: &spec::ConfigRef,
    bytes: &[u8],
) -> Result<PlainCanonicalJsonBytes> {
    let raw = std::str::from_utf8(bytes).map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!("typed config {} is not UTF-8: {error}", config_ref.digest),
        )
    })?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(raw).map_err(|error| {
        problem(
            ProblemClass::InvalidDataShape,
            format!(
                "typed config {} is not canonical JSON: {error}",
                config_ref.digest
            ),
        )
    })?;
    if canonical.as_bytes() != bytes {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "typed config {} is not normalized canonical JSON",
                config_ref.digest
            ),
        ));
    }
    if canonical.as_bytes().len() as u64 != config_ref.byte_len {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("typed config {} byte length mismatch", config_ref.digest),
        ));
    }
    let actual = canonical.content_digest();
    if actual != config_ref.digest {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "typed config digest mismatch: expected {}, recomputed {}",
                config_ref.digest, actual
            ),
        ));
    }
    Ok(canonical)
}

/// Lowers and certifies a typed program draft.
pub fn certify_program_draft(draft: &program::TypedProgramDraft) -> Result<CertifiedTypedSpec> {
    let registry = CertificationRegistry::from_program_draft(draft)?;
    let spec = lower_program_draft_with_registry(draft, &registry)?;
    certify_typed_spec(spec, &registry)
}

/// Certifies typed spec input with registry-backed validation.
pub fn certify_typed_spec(
    input: impl Into<TypedSpecCertificationInput>,
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    let validated = validate_typed_spec(input.into().into_raw_spec(), registry)?;
    let certificate = certificate_for_envelope(validated.envelope(), registry)?;
    let framework_lifecycle = CertifiedFrameworkLifecycle::from_validated_spec(&validated)?;
    Ok(CertifiedTypedSpec {
        validated,
        certificate,
        framework_lifecycle,
    })
}

/// Verifies persisted spec and certificate bytes against a certification registry.
///
/// Hash matches alone are insufficient: this parses hostile persisted data, compares spec and
/// certificate evidence, re-runs registry-backed certification, and only then returns the
/// non-forgeable in-memory authority.
pub fn verify_persisted_spec_certificate(
    spec_bytes: &[u8],
    certificate_bytes: &[u8],
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    let persisted_parts = PersistedSpecCertificateParts::from_untrusted_bytes(
        spec_bytes.to_vec(),
        certificate_bytes.to_vec(),
    );
    verify_untrusted_spec_certificate_parts(persisted_parts.parse_untrusted()?, registry)
}

/// Verifies persisted spec and certificate bytes against a trusted registry superset.
///
/// The parsed spec is used only to select descriptor identities from `trusted_registry`; the
/// resulting scoped registry must match the certificate's registry digest and descriptor evidence.
pub fn verify_persisted_spec_certificate_with_trusted_registry(
    spec_bytes: &[u8],
    certificate_bytes: &[u8],
    trusted_registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    let persisted_parts = PersistedSpecCertificateParts::from_untrusted_bytes(
        spec_bytes.to_vec(),
        certificate_bytes.to_vec(),
    );
    let untrusted = persisted_parts.parse_untrusted()?;
    let scoped = trusted_registry.scoped_for_spec(untrusted.spec())?;
    verify_untrusted_spec_certificate_parts(untrusted, &scoped)
}

/// Lowers a typed program draft into a v1 spec without minting certification authority.
pub fn lower_program_draft(draft: &program::TypedProgramDraft) -> Result<LoweredTypedSpec> {
    let registry = CertificationRegistry::from_program_draft(draft)?;
    lower_program_draft_with_registry(draft, &registry)
}

fn verify_untrusted_spec_certificate_parts(
    persisted_parts: UntrustedSpecCertificateParts,
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    let UntrustedSpecCertificateParts {
        spec,
        certificate: expected_certificate,
    } = persisted_parts;
    expected_certificate.verify_hash()?;
    let actual_spec_hash = spec
        .spec()
        .spec_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    if expected_certificate.evidence.spec_hash != actual_spec_hash {
        return Err(certificate(format!(
            "certificate/spec hash mismatch: certificate {}, recomputed {}",
            expected_certificate.evidence.spec_hash, actual_spec_hash
        )));
    }

    let expected_registry_digest = registry.digest()?;
    if expected_certificate.evidence.registry_digest != expected_registry_digest {
        return Err(certificate(format!(
            "registry digest mismatch: certificate {}, current {}",
            expected_certificate.evidence.registry_digest, expected_registry_digest
        )));
    }

    let spec_ref = spec.spec();
    let expected_descriptor_evidence = descriptor_evidence_for_spec(spec_ref)?;
    if expected_certificate.evidence.descriptor_identities != expected_descriptor_evidence {
        return Err(certificate(
            "descriptor identity evidence does not match persisted spec",
        ));
    }
    let expected_schema_role_grants = schema_role_grants_for_spec(spec_ref, registry)?;
    if expected_certificate.evidence.schema_role_grants != expected_schema_role_grants {
        return Err(certificate(
            "schema role grant evidence does not match persisted spec",
        ));
    }
    let expected_verifiers = manual_authorization_verifiers_for_spec(spec_ref, registry)?;
    if expected_certificate.evidence.manual_authorization_verifiers != expected_verifiers {
        return Err(certificate(
            "manual authorization verifier evidence does not match persisted spec",
        ));
    }
    let expected_authorities = operator_authority_snapshots_for_spec(spec_ref, registry)?;
    if expected_certificate.evidence.operator_authority_snapshots != expected_authorities {
        return Err(certificate(
            "operator authority snapshot evidence does not match persisted spec",
        ));
    }
    let certified = certify_typed_spec(spec, registry)?;
    if certified.certificate != expected_certificate {
        return Err(certificate(
            "persisted certificate does not match registry-backed certification",
        ));
    }
    Ok(certified)
}

fn certificate_for_envelope(
    envelope: &spec::HashedSpecEnvelope,
    registry: &CertificationRegistry,
) -> Result<CertifiedSpecCertificate> {
    CertifiedSpecCertificate::from_evidence(CertifiedSpecCertificateEvidence {
        certificate_version: CERTIFICATE_VERSION.to_owned(),
        media_type: CERTIFICATE_MEDIA_TYPE.to_owned(),
        certifier_algorithm: CERTIFIER_ALGORITHM.to_owned(),
        spec_hash: envelope.spec_hash.clone(),
        registry_digest: registry.digest()?,
        descriptor_identities: descriptor_evidence_for_spec(&envelope.spec)?,
        schema_role_grants: schema_role_grants_for_spec(&envelope.spec, registry)?,
        manual_authorization_verifiers: manual_authorization_verifiers_for_spec(
            &envelope.spec,
            registry,
        )?,
        operator_authority_snapshots: operator_authority_snapshots_for_spec(
            &envelope.spec,
            registry,
        )?,
    })
}

fn descriptor_evidence_for_spec(
    spec: &spec::TypedExecutionSpec,
) -> Result<Vec<CertifiedDescriptorEvidence>> {
    let mut evidence = spec
        .descriptor_identities
        .iter()
        .map(|descriptor| {
            let reference = descriptor
                .descriptor_ref()
                .map_err(|error| CertifyError::Spec(error.to_string()))?;
            Ok(CertifiedDescriptorEvidence {
                descriptor_family: reference.family,
                descriptor_id: reference.descriptor_id,
                descriptor_digest: reference.descriptor_digest,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    evidence.sort_by(|left, right| {
        (
            left.descriptor_family,
            left.descriptor_id.as_str(),
            left.descriptor_digest.as_str(),
        )
            .cmp(&(
                right.descriptor_family,
                right.descriptor_id.as_str(),
                right.descriptor_digest.as_str(),
            ))
    });
    Ok(evidence)
}

fn schema_role_grants_for_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<CertifiedSchemaRoleGrantEvidence>> {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for manual in manual_resolution_specs(spec) {
        let role = CertifiedSchemaRole::ManualResolutionEvidence;
        let key = (manual.evidence_schema.as_str().to_owned(), role);
        if !seen.insert(key) {
            continue;
        }
        let Some(roles) = registry.schema_roles.get(manual.evidence_schema.as_str()) else {
            return Err(certificate(format!(
                "manual evidence schema {} is missing from registry",
                manual.evidence_schema
            )));
        };
        if !roles.contains(&role) {
            return Err(certificate(format!(
                "manual evidence schema {} lacks certified role {}",
                manual.evidence_schema,
                role.as_str()
            )));
        }
        evidence.push(CertifiedSchemaRoleGrantEvidence {
            schema_id: manual.evidence_schema.clone(),
            role,
        });
    }
    evidence.sort_by(|left, right| {
        (left.schema_id.as_str(), left.role.as_str())
            .cmp(&(right.schema_id.as_str(), right.role.as_str()))
    });
    Ok(evidence)
}

fn manual_authorization_verifiers_for_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<CertifiedManualAuthorizationVerifierEvidence>> {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for manual in manual_resolution_specs(spec) {
        let verifier_id = &manual.authorization.verifier_id;
        if !seen.insert(verifier_id.as_str().to_owned()) {
            continue;
        }
        if !registry
            .manual_authorization_verifiers
            .contains(verifier_id.as_str())
        {
            return Err(certificate(format!(
                "manual authorization verifier {} is missing from registry",
                verifier_id
            )));
        }
        evidence.push(CertifiedManualAuthorizationVerifierEvidence {
            verifier_id: verifier_id.clone(),
        });
    }
    evidence.sort_by(|left, right| left.verifier_id.as_str().cmp(right.verifier_id.as_str()));
    Ok(evidence)
}

fn operator_authority_snapshots_for_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<CertifiedOperatorAuthoritySnapshotEvidence>> {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for manual in manual_resolution_specs(spec) {
        let authority_id = &manual.authorization.authority.authority_id;
        if !seen.insert(authority_id.as_str().to_owned()) {
            continue;
        }
        let Some(snapshot) = registry
            .operator_authority_snapshots
            .get(authority_id.as_str())
        else {
            return Err(certificate(format!(
                "operator authority snapshot {} is missing from registry",
                authority_id
            )));
        };
        evidence.push(operator_authority_snapshot_evidence(snapshot)?);
    }
    evidence.sort_by(|left, right| left.authority_id.as_str().cmp(right.authority_id.as_str()));
    Ok(evidence)
}

fn operator_authority_snapshot_evidence(
    snapshot: &spec::OperatorAuthoritySnapshotSpec,
) -> Result<CertifiedOperatorAuthoritySnapshotEvidence> {
    Ok(CertifiedOperatorAuthoritySnapshotEvidence {
        authority_id: snapshot.authority_id.clone(),
        authority_digest: operator_authority_snapshot_digest(snapshot)?,
        operators: snapshot
            .operators
            .iter()
            .map(|operator| CertifiedOperatorAuthorityMemberEvidence {
                operator_id: operator.operator_id.clone(),
                public_identity: operator.public_identity.clone(),
            })
            .collect(),
    })
}

fn operator_authority_snapshot_digest(
    snapshot: &spec::OperatorAuthoritySnapshotSpec,
) -> Result<ContentDigest> {
    content_digest_json(operator_authority_snapshot_json(snapshot))
}

fn operator_authority_snapshot_json(
    snapshot: &spec::OperatorAuthoritySnapshotSpec,
) -> serde_json::Value {
    serde_json::json!({
        "authority_id": snapshot.authority_id.as_str(),
        "operators": snapshot
            .operators
            .iter()
            .map(|operator| {
                serde_json::json!({
                    "operator_id": operator.operator_id.as_str(),
                    "public_identity": operator.public_identity.as_str(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn manual_resolution_specs(
    spec: &spec::TypedExecutionSpec,
) -> Vec<&spec::ManualResolutionEvidenceSpec> {
    let mut manuals = Vec::new();
    match &spec.saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manuals.push(manual),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manuals.push(manual.as_ref()),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted { .. } => {}
    }
    manuals
}

fn lower_program_draft_with_registry(
    draft: &program::TypedProgramDraft,
    _registry: &CertificationRegistry,
) -> Result<LoweredTypedSpec> {
    let mut lowerer = DraftLowerer::new(draft)?;
    let lowered = lowerer.lower()?;
    Ok(LoweredTypedSpec::new(lowered))
}

fn problem(class: ProblemClass, message: impl Into<String>) -> CertifyError {
    CertifyError::Problem {
        class,
        message: message.into(),
    }
}

fn lower(message: impl Into<String>) -> CertifyError {
    CertifyError::Lowering(message.into())
}

fn canonical(message: impl Into<String>) -> CertifyError {
    CertifyError::Canonical(message.into())
}

fn certificate(message: impl Into<String>) -> CertifyError {
    CertifyError::Certificate(message.into())
}

fn predecessor_nodes(
    input_cells: &[CellId],
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<Vec<NodeId>> {
    let mut predecessors = BTreeSet::new();
    for cell_id in input_cells {
        let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("predecessor input cell {cell_id} is missing"),
            )
        })?;
        if let spec::CellProducer::Node(node_id) = &cell.producer {
            predecessors.insert(node_id.clone());
        }
    }
    Ok(predecessors.into_iter().collect())
}

fn lower_input_binding(binding: &program::InputBindingSpec) -> Result<spec::InputBindingSpec> {
    Ok(spec::InputBindingSpec {
        input_schema_id: binding.input_schema_id.clone(),
        input_descriptor_id: binding.input_descriptor_id.clone(),
        root: lower_input_node(&binding.root)?,
        digest: binding.digest.clone(),
    })
}

fn lower_saga_policy(policy: &program::SagaPolicy) -> spec::SagaPolicySpec {
    match policy {
        program::SagaPolicy::NoSideEffects => spec::SagaPolicySpec::NoSideEffects,
        program::SagaPolicy::FailWithoutAcdcClaim => spec::SagaPolicySpec::FailWithoutAcdcClaim,
        program::SagaPolicy::ManualResolution { manual } => {
            spec::SagaPolicySpec::ManualResolution {
                manual: lower_manual_resolution_evidence(manual),
            }
        }
        program::SagaPolicy::CompensateCompleted {
            on_remediation_unresolved,
        } => spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: lower_remediation_unresolved(on_remediation_unresolved),
        },
    }
}

fn lower_remediation_unresolved(
    directive: &program::RemediationUnresolved,
) -> spec::RemediationUnresolvedSpec {
    match directive {
        program::RemediationUnresolved::ManualResolution { manual } => {
            spec::RemediationUnresolvedSpec::ManualResolution {
                manual: Box::new(lower_manual_resolution_evidence(manual)),
            }
        }
        program::RemediationUnresolved::FailWithoutAcdcClaim => {
            spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim
        }
    }
}

fn lower_manual_resolution_evidence(
    manual: &program::ManualResolutionPolicyDraft,
) -> spec::ManualResolutionEvidenceSpec {
    manual.to_spec()
}

fn lower_operation_input_binding(
    binding: &program::OperationInputBindingSpec,
) -> Result<spec::InputBindingSpec> {
    Ok(spec::InputBindingSpec {
        input_schema_id: binding.input_schema_id.clone(),
        input_descriptor_id: binding.input_descriptor_id.clone(),
        root: lower_input_node(&binding.root)?,
        digest: binding.digest.clone(),
    })
}

fn lower_input_node(node: &program::InputBindingNode) -> Result<spec::InputBindingNodeSpec> {
    match node.as_ref() {
        program::InputBindingNodeRef::Unit => Ok(spec::InputBindingNodeSpec::Unit),
        program::InputBindingNodeRef::Cell(cell) => Ok(spec::InputBindingNodeSpec::Cell(Box::new(
            spec::InputBindingCellSpec {
                field_path: input_field_path(cell.field_path().as_str())?,
                cell_id: cell.cell_id().clone(),
                semantic_type_id: cell.semantic_type_id().clone(),
                schema_id: cell.schema_id().clone(),
                required_terminal: lower_required_terminal(cell.required_terminal()),
                value_lineage: lineage_ref(cell.value_lineage().digest()),
                context: cell.context().clone(),
            },
        ))),
        program::InputBindingNodeRef::Tuple(elements) => elements
            .iter()
            .map(lower_input_node)
            .collect::<Result<Vec<_>>>()
            .map(spec::InputBindingNodeSpec::Tuple),
        program::InputBindingNodeRef::Struct(fields) => fields
            .iter()
            .map(|field| {
                Ok(spec::NamedInputBindingSpec {
                    field_path: input_field_path(field.field_path.as_str())?,
                    node: lower_input_node(&field.node)?,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(spec::InputBindingNodeSpec::Struct),
        program::InputBindingNodeRef::Vec {
            elements,
            ordering,
            domain_keys,
        } => Ok(spec::InputBindingNodeSpec::Vec {
            elements: elements
                .iter()
                .map(lower_input_node)
                .collect::<Result<Vec<_>>>()?,
            ordering: lower_ordering_evidence(ordering),
            domain_keys: domain_keys.iter().map(lower_domain_key_ref).collect(),
        }),
        program::InputBindingNodeRef::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => Ok(spec::InputBindingNodeSpec::NonEmptyVec {
            elements: elements
                .iter()
                .map(lower_input_node)
                .collect::<Result<Vec<_>>>()?,
            ordering: lower_ordering_evidence(ordering),
            domain_keys: domain_keys.iter().map(lower_domain_key_ref).collect(),
        }),
    }
}

fn single_cell_input_binding(
    source: CellInfo,
    cell_id: CellId,
    field_path: &str,
) -> Result<spec::InputBindingSpec> {
    let node = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: input_field_path(field_path)?,
        cell_id,
        semantic_type_id: source.semantic_type_id.clone(),
        schema_id: source.schema_id.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: source.value_lineage.clone(),
        context: input_context_from_cell_context(&source.context),
    }));
    let digest = content_digest_json(input_node_json(&node))?;
    Ok(spec::InputBindingSpec {
        input_schema_id: source.schema_id,
        input_descriptor_id: descriptor_id_json(serde_json::json!({
            "framework": "single_cell_input",
            "semantic_type_id": source.semantic_type_id.as_str(),
        }))?,
        root: node,
        digest,
    })
}

fn lower_required_terminal(value: program::RequiredTerminal) -> spec::RequiredTerminal {
    match value {
        program::RequiredTerminal::ProducedOnly => spec::RequiredTerminal::ProducedOnly,
        program::RequiredTerminal::MaybeSkipped => spec::RequiredTerminal::MaybeSkipped,
    }
}

fn input_context_from_cell_context(context: &spec::CellContextSpec) -> spec::InputContextSpec {
    match context {
        spec::CellContextSpec::NoContext => spec::InputContextSpec::NoContext,
        spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => spec::InputContextSpec::Required {
            context_ref: context_ref.clone(),
            resource_kind: resource_kind.clone(),
            stage: stage.clone(),
            producer: producer.clone(),
        },
    }
}

fn node_context_from_cell_context(context: &spec::CellContextSpec) -> spec::NodeContextSpec {
    match context {
        spec::CellContextSpec::NoContext => spec::NodeContextSpec::no_context(),
        spec::CellContextSpec::Bound { context_ref, .. } => spec::NodeContextSpec::Required {
            context_ref: context_ref.clone(),
        },
    }
}

fn state_context_descriptor_from_cell_context(
    context: &spec::CellContextSpec,
    contexts: &[spec::CertifiedContextSpec],
) -> Result<spec::StateContextDescriptorSpec> {
    match context {
        spec::CellContextSpec::NoContext => Ok(spec::StateContextDescriptorSpec::no_context()),
        spec::CellContextSpec::Bound { context_ref, .. } => {
            let context = contexts
                .iter()
                .find(|context| context.context_ref == *context_ref)
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "context-bound side-effect verify output references missing context {}",
                            context_ref
                        ),
                    )
                })?;
            Ok(spec::StateContextDescriptorSpec::Required(Box::new(
                spec::StateContextDescriptorRequirementSpec {
                    context_descriptor_id: context.context_descriptor_id.clone(),
                    schema_id: context.schema_id.clone(),
                    semantic_type_id: context.semantic_type_id.clone(),
                    canonicalizer_identity: context.canonicalizer_identity.clone(),
                },
            )))
        }
    }
}

fn render_context_contract_for_public_outputs(
    required_cells: &[spec::PublicOutputCell],
    cells: &[spec::CellSpec],
    contexts: &[spec::CertifiedContextSpec],
) -> Result<(spec::NodeContextSpec, spec::StateContextDescriptorSpec)> {
    let mut context = None::<spec::CellContextSpec>;
    for output in required_cells {
        let cell = cells
            .iter()
            .find(|cell| cell.cell_id == output.cell_id)
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("public output cell {} is missing", output.cell_id),
                )
            })?;
        let spec::CellContextSpec::Bound { context_ref, .. } = &cell.context else {
            continue;
        };
        match &context {
            Some(spec::CellContextSpec::Bound {
                context_ref: existing,
                ..
            }) if existing != context_ref => {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "public output render node cannot consume multiple context refs: {} and {}",
                        existing, context_ref
                    ),
                ));
            }
            Some(_) => {}
            None => context = Some(cell.context.clone()),
        }
    }
    let Some(context) = context else {
        return Ok((
            spec::NodeContextSpec::no_context(),
            spec::StateContextDescriptorSpec::no_context(),
        ));
    };
    Ok((
        node_context_from_cell_context(&context),
        state_context_descriptor_from_cell_context(&context, contexts)?,
    ))
}

fn lower_ordering_evidence(value: &program::OrderingEvidence) -> spec::OrderingEvidence {
    match value {
        program::OrderingEvidence::ExplicitAuthorOrder => {
            spec::OrderingEvidence::ExplicitAuthorOrder
        }
        program::OrderingEvidence::StableDomainKey => spec::OrderingEvidence::StableDomainKey,
    }
}

fn lower_domain_key_ref(value: &program::StableDomainKeyRef) -> spec::StableDomainKeyRef {
    spec::StableDomainKeyRef {
        schema_id: value.schema_id.clone(),
        content_digest: value.content_digest.clone(),
    }
}

fn lower_bridge_kind(value: program::BridgeKind) -> spec::BridgeKind {
    match value {
        program::BridgeKind::ImportFromParent => spec::BridgeKind::ImportFromParent,
        program::BridgeKind::ExportToParent => spec::BridgeKind::ExportToParent,
    }
}

fn bridge_kind_json(value: spec::BridgeKind) -> &'static str {
    match value {
        spec::BridgeKind::ImportFromParent => "import-from-parent",
        spec::BridgeKind::ExportToParent => "export-to-parent",
    }
}

fn lower_bridge_policy(value: program::BridgePolicy) -> spec::BridgePolicy {
    match value {
        program::BridgePolicy::SameRunSameValueV1 => spec::BridgePolicy::SameRunSameValue,
    }
}

fn bridge_policy_json(value: spec::BridgePolicy) -> &'static str {
    match value {
        spec::BridgePolicy::SameRunSameValue => "same-run-same-value-v1",
    }
}

fn state_descriptor_identity_from_program(
    node: &program::StateNodeSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = effect_descriptor_for_kind(&node.effect_kind)?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: node.state_descriptor_id.clone(),
        name: node.state_descriptor_name.clone(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        context: node.context_descriptor.clone(),
        input_context: node.input_context_contract.clone(),
        output_context: node.output_context_contract.clone(),
        config_schema_id: node.config.schema_id.clone(),
        input_schema_id: node.input.input_schema_id.clone(),
        output_schema_id: node.output_schema_id.clone(),
        output_semantic_type_id: node.output_semantic_type_id.clone(),
        effect_kind: node.effect_kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities: node.capability_bindings.clone(),
        emitted_fact_descriptors: node.fact_descriptor_allowlist.clone(),
        runner: runner_kind_name(node.runner).to_owned(),
        side_effect_contract_digest: node.side_effect_contract_digest.clone(),
    })
}

fn state_descriptor_identity_from_descriptor(
    descriptor: &program::StateDescriptorIdentity,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = descriptor.effect();
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        state_kind: descriptor.kind().clone(),
        state_version: descriptor.version().clone(),
        context: descriptor.context().clone(),
        input_context: descriptor.input_context().clone(),
        output_context: descriptor.output_context().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        output_semantic_type_id: descriptor.output_semantic_type_id().clone(),
        effect_kind: effect.kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version.clone(),
        capabilities: descriptor.capabilities().clone(),
        emitted_fact_descriptors: descriptor.emitted_fact_descriptors().to_vec(),
        runner: runner_kind_name(descriptor.runner()).to_owned(),
        side_effect_contract_digest: descriptor.side_effect_contract_digest().cloned(),
    })
}

fn operation_descriptor_identity_from_program(
    frame: &program::OperationLineageFrameSpec,
) -> spec::OperationDescriptorIdentity {
    spec::OperationDescriptorIdentity {
        descriptor_id: frame.operation_descriptor_id.clone(),
        name: frame.operation_name.clone(),
        operation_kind: frame.operation_kind.clone(),
        operation_version: frame.operation_version.clone(),
        config_schema_id: frame.config.schema_id.clone(),
        input_schema_id: frame.input.input_schema_id.clone(),
        output_schema_id: frame.output_schema_id.clone(),
        expansion_abi: frame.expansion_abi.to_owned(),
    }
}

fn operation_descriptor_identity_from_descriptor(
    descriptor: &program::OperationDescriptorIdentity,
) -> spec::OperationDescriptorIdentity {
    spec::OperationDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        operation_kind: descriptor.kind().clone(),
        operation_version: descriptor.version().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        expansion_abi: descriptor.expansion_abi().to_owned(),
    }
}

fn runner_kind_name(runner: program::RunnerKind) -> &'static str {
    match runner {
        program::RunnerKind::Pure => "pure",
        program::RunnerKind::ReadExternal => "read_external",
        program::RunnerKind::ManagedPlatformWrite => "managed_platform_write",
        program::RunnerKind::ApplySideEffect => "apply_side_effect",
    }
}

fn lower_planning_lineage(lineage: &program::OperationLineage) -> spec::PlanningLineage {
    spec::PlanningLineage {
        active_operation_instances: lineage.active_instances.clone(),
        completed_operation_frames: lineage.completed_frames.clone(),
        lineage_digest: lineage.digest.clone(),
    }
}

fn empty_planning_lineage() -> Result<spec::PlanningLineage> {
    planning_lineage_from_parts(Vec::new(), Vec::new())
}

fn final_planning_lineage(
    frames: &[program::OperationLineageFrameSpec],
) -> Result<spec::PlanningLineage> {
    planning_lineage_from_parts(
        Vec::new(),
        frames
            .iter()
            .map(|frame| frame.lineage_digest.clone())
            .collect(),
    )
}

fn planning_lineage_from_parts(
    active_operation_instances: Vec<OperationInstanceId>,
    completed_operation_frames: Vec<ContentDigest>,
) -> Result<spec::PlanningLineage> {
    let lineage_digest =
        planning_lineage_digest(&active_operation_instances, &completed_operation_frames)?;
    Ok(spec::PlanningLineage {
        active_operation_instances,
        completed_operation_frames,
        lineage_digest,
    })
}

fn validate_planning_lineage(lineage: &spec::PlanningLineage) -> Result<()> {
    let expected = planning_lineage_digest(
        &lineage.active_operation_instances,
        &lineage.completed_operation_frames,
    )?;
    if lineage.lineage_digest != expected {
        return Err(problem(
            ProblemClass::InvalidDataMeaning,
            format!(
                "planning lineage {} is not content addressed",
                lineage.lineage_digest
            ),
        ));
    }
    Ok(())
}

fn planning_lineage_digest(
    active_operation_instances: &[OperationInstanceId],
    completed_operation_frames: &[ContentDigest],
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "active_instances": active_operation_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": completed_operation_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
    }))
}

fn scope_id_from_spec(
    parent_scope_id: Option<&ScopeId>,
    key: &str,
    operation_lineage: &spec::PlanningLineage,
) -> Result<ScopeId> {
    Ok(ScopeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "local_scope_key": key,
            "operation_lineage": operation_lineage.lineage_digest.as_str(),
            "parent_scope_id": parent_scope_id.map(ScopeId::as_str),
        }))?,
    ))
}

fn seed_id_from_spec(seed: &spec::SeedSpec) -> Result<mfm_ids::SeedId> {
    Ok(mfm_ids::SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "scope_id": seed.scope_id.as_str(),
            "seed_key": seed.seed_key.as_str(),
            "semantic_type_id": seed.semantic_type_id.as_str(),
            "schema_id": seed.schema_id.as_str(),
        }))?,
    ))
}

fn stable_author_key(value: &str) -> Result<spec::StableAuthorKey> {
    spec::StableAuthorKey::new(value).map_err(|error| lower(error.to_string()))
}

fn input_field_path(value: &str) -> Result<spec::PublicFieldPath> {
    let value = if value.is_empty() { "root" } else { value };
    spec::PublicFieldPath::new(value).map_err(|error| lower(error.to_string()))
}

fn lineage_ref(digest: &ContentDigest) -> spec::ValueLineageRef {
    spec::ValueLineageRef {
        lineage_digest: digest.clone(),
    }
}

fn value_lineage_digest(lineage: &spec::ValueLineage) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "config_ref_digest": lineage.config_ref_digest.as_ref().map(ContentDigest::as_str),
        "domain_keys": domain_key_refs_json(&sorted_domain_keys(lineage.domain_keys.clone())),
        "input_cells": sorted_cell_ids(lineage.input_cells.clone())
            .iter()
            .map(CellId::as_str)
            .collect::<Vec<_>>(),
        "operation_lineage": planning_lineage_json(&lineage.planning_lineage),
        "producer": cell_producer_json(&lineage.producer),
        "scope_id": lineage.scope_id.as_str(),
        "transform_policy": lineage_transform_policy_json(lineage.transform_policy),
    }))
}

fn sorted_cell_ids(mut values: Vec<CellId>) -> Vec<CellId> {
    values.sort();
    values
}

fn sorted_domain_keys(mut values: Vec<spec::StableDomainKeyRef>) -> Vec<spec::StableDomainKeyRef> {
    values.sort();
    values
}

fn cell_producer_json(producer: &spec::CellProducer) -> serde_json::Value {
    match producer {
        spec::CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        spec::CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

fn lineage_transform_policy_json(policy: spec::LineageTransformPolicy) -> &'static str {
    match policy {
        spec::LineageTransformPolicy::Source => "source",
        spec::LineageTransformPolicy::StateOutput => "state_output",
        spec::LineageTransformPolicy::SameValueBridge => "same_value_bridge",
    }
}

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

fn config_ref_digest(config_ref: &spec::ConfigRef) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "byte_len": config_ref.byte_len,
        "content_digest": config_ref.digest.as_str(),
        "schema_id": config_ref.schema_id.as_str(),
    }))
}

fn collect_input_cells(root: &spec::InputBindingNodeSpec) -> Vec<CellId> {
    let mut cells = Vec::new();
    collect_input_cells_into(root, &mut cells);
    cells
}

fn collect_input_cells_into(root: &spec::InputBindingNodeSpec, output: &mut Vec<CellId>) {
    match root {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => output.push(cell.cell_id.clone()),
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                collect_input_cells_into(element, output);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells_into(&field.node, output);
            }
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells_into(element, output);
            }
        }
    }
}

fn descriptor_ref_json(reference: spec::DescriptorRef) -> serde_json::Value {
    serde_json::json!({
        "descriptor_digest": reference.descriptor_digest.as_str(),
        "descriptor_family": reference.family.as_str(),
        "descriptor_id": reference.descriptor_id.as_str(),
    })
}

fn context_validator_ref_json(validator: &ContextValidator) -> serde_json::Value {
    let requirement = &validator.requirement;
    serde_json::json!({
        "canonicalizer_identity": requirement.canonicalizer_identity.as_str(),
        "context_descriptor_id": requirement.context_descriptor_id.as_str(),
        "schema_id": requirement.schema_id.as_str(),
        "semantic_type_id": requirement.semantic_type_id.as_str(),
    })
}

fn state_descriptor_ref_json(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<serde_json::Value> {
    let reference = spec::DescriptorIdentity::State(Box::new(descriptor.clone()))
        .descriptor_ref()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(descriptor_ref_json(reference))
}

fn operation_descriptor_ref_json(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<serde_json::Value> {
    let reference = spec::DescriptorIdentity::Operation(Box::new(descriptor.clone()))
        .descriptor_ref()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(descriptor_ref_json(reference))
}

fn canonical_json_bytes(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value).map_err(|error| canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|error| canonical(error.to_string()))
}

fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest> {
    Ok(canonical_json_bytes(value)?.content_digest())
}

fn digest_bytes_json(value: serde_json::Value) -> Result<DigestBytes> {
    Ok(*content_digest_json(value)?.digest())
}

fn parse_certificate(value: &serde_json::Value) -> Result<CertifiedSpecCertificate> {
    let object = json_object(value, "typed spec certificate")?;
    Ok(CertifiedSpecCertificate {
        certificate_hash: parse_identity(required_str(object, "certificate_hash")?)?,
        evidence: parse_certificate_evidence(required(object, "evidence")?)?,
    })
}

fn parse_certificate_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedSpecCertificateEvidence> {
    let object = json_object(value, "typed spec certificate evidence")?;
    let certificate_version = required_str(object, "certificate_version")?.to_owned();
    if certificate_version != CERTIFICATE_VERSION {
        return Err(certificate(format!(
            "unsupported certificate_version {certificate_version:?}"
        )));
    }
    let media_type = required_str(object, "media_type")?.to_owned();
    if media_type != CERTIFICATE_MEDIA_TYPE {
        return Err(certificate(format!(
            "unsupported certificate media_type {media_type:?}"
        )));
    }
    let certifier_algorithm = required_str(object, "certifier_algorithm")?.to_owned();
    if certifier_algorithm != CERTIFIER_ALGORITHM {
        return Err(certificate(format!(
            "unsupported certifier_algorithm {certifier_algorithm:?}"
        )));
    }
    Ok(CertifiedSpecCertificateEvidence {
        certificate_version,
        media_type,
        certifier_algorithm,
        spec_hash: parse_identity(required_str(object, "spec_hash")?)?,
        registry_digest: parse_identity(required_str(object, "registry_digest")?)?,
        descriptor_identities: parse_array(
            required(object, "descriptor_identities")?,
            parse_descriptor_evidence,
        )?,
        schema_role_grants: parse_array(
            required(object, "schema_role_grants")?,
            parse_schema_role_grant_evidence,
        )?,
        manual_authorization_verifiers: parse_array(
            required(object, "manual_authorization_verifiers")?,
            parse_manual_authorization_verifier_evidence,
        )?,
        operator_authority_snapshots: parse_array(
            required(object, "operator_authority_snapshots")?,
            parse_operator_authority_snapshot_evidence,
        )?,
    })
}

fn parse_schema_role_grant_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedSchemaRoleGrantEvidence> {
    let object = json_object(value, "schema role grant evidence")?;
    Ok(CertifiedSchemaRoleGrantEvidence {
        schema_id: parse_identity(required_str(object, "schema_id")?)?,
        role: CertifiedSchemaRole::parse(required_str(object, "role")?)?,
    })
}

fn parse_manual_authorization_verifier_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedManualAuthorizationVerifierEvidence> {
    let object = json_object(value, "manual authorization verifier evidence")?;
    Ok(CertifiedManualAuthorizationVerifierEvidence {
        verifier_id: spec::ManualAuthorizationVerifierId::new(required_str(object, "verifier_id")?)
            .map_err(|error| certificate(error.to_string()))?,
    })
}

fn parse_operator_authority_snapshot_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedOperatorAuthoritySnapshotEvidence> {
    let object = json_object(value, "operator authority snapshot evidence")?;
    Ok(CertifiedOperatorAuthoritySnapshotEvidence {
        authority_id: spec::OperatorAuthorityId::new(required_str(object, "authority_id")?)
            .map_err(|error| certificate(error.to_string()))?,
        authority_digest: parse_identity(required_str(object, "authority_digest")?)?,
        operators: parse_array(
            required(object, "operators")?,
            parse_operator_authority_member_evidence,
        )?,
    })
}

fn parse_operator_authority_member_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedOperatorAuthorityMemberEvidence> {
    let object = json_object(value, "operator authority member evidence")?;
    Ok(CertifiedOperatorAuthorityMemberEvidence {
        operator_id: spec::OperatorId::new(required_str(object, "operator_id")?)
            .map_err(|error| certificate(error.to_string()))?,
        public_identity: spec::OperatorPublicIdentity::new(required_str(
            object,
            "public_identity",
        )?)
        .map_err(|error| certificate(error.to_string()))?,
    })
}

fn parse_descriptor_evidence(value: &serde_json::Value) -> Result<CertifiedDescriptorEvidence> {
    let object = json_object(value, "descriptor certificate evidence")?;
    Ok(CertifiedDescriptorEvidence {
        descriptor_family: CertifiedDescriptorFamily::parse(required_str(
            object,
            "descriptor_family",
        )?)
        .map_err(|error| certificate(error.to_string()))?,
        descriptor_id: parse_identity(required_str(object, "descriptor_id")?)?,
        descriptor_digest: parse_identity(required_str(object, "descriptor_digest")?)?,
    })
}

fn parse_array<T>(
    value: &serde_json::Value,
    parser: fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    json_array(value, "array")?.iter().map(parser).collect()
}

fn parse_identity<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    value
        .parse()
        .map_err(|error| certificate(format!("identity parse failed: {error}")))
}

fn json_object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| certificate(format!("{context} must be a JSON object")))
}

fn json_array<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| certificate(format!("{context} must be a JSON array")))
}

fn required<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a serde_json::Value> {
    object
        .get(field)
        .ok_or_else(|| certificate(format!("missing required field {field}")))
}

fn required_str<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a str> {
    json_string(required(object, field)?, field)
}

fn json_string<'a>(value: &'a serde_json::Value, field: &'static str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| certificate(format!("{field} must be a string")))
}

fn descriptor_id_json(value: serde_json::Value) -> Result<DescriptorId> {
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    ))
}

fn state_descriptor_id_from_spec(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "capabilities": capability_set_json(&descriptor.capabilities),
        "config_schema_id": descriptor.config_schema_id.as_str(),
        "context": state_context_descriptor_json(&descriptor.context),
        "effect": {
            "class": descriptor.effect_class.as_str(),
            "kind": descriptor.effect_kind.as_str(),
            "name": descriptor.effect_name.as_str(),
            "version": descriptor.effect_version.as_str(),
        },
        "emitted_fact_descriptors": fact_descriptor_refs_json(&descriptor.emitted_fact_descriptors),
        "input_context": state_input_context_contract_json(&descriptor.input_context),
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.state_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_context": state_output_context_contract_json(&descriptor.output_context),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "output_semantic_type_id": descriptor.output_semantic_type_id.as_str(),
        "runner": descriptor.runner.as_str(),
        "side_effect_contract_digest": descriptor.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
        "version": descriptor.state_version.as_str(),
    }))
}

fn state_context_descriptor_json(context: &spec::StateContextDescriptorSpec) -> serde_json::Value {
    match context {
        spec::StateContextDescriptorSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateContextDescriptorSpec::Required(requirement) => {
            let spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id,
                schema_id,
                semantic_type_id,
                canonicalizer_identity,
            } = requirement.as_ref();
            serde_json::json!({
                "canonicalizer_identity": canonicalizer_identity.as_str(),
                "context_descriptor_id": context_descriptor_id.as_str(),
                "kind": "required",
                "schema_id": schema_id.as_str(),
                "semantic_type_id": semantic_type_id.as_str(),
            })
        }
    }
}

fn state_input_context_contract_json(
    contract: &spec::StateInputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateInputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateInputContextContractSpec::Required {
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn state_output_context_contract_json(
    contract: &spec::StateOutputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateOutputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => serde_json::json!({
            "kind": "produces",
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn operation_descriptor_id_from_spec(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "config_schema_id": descriptor.config_schema_id.as_str(),
        "expansion_abi": descriptor.expansion_abi.as_str(),
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.operation_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "version": descriptor.operation_version.as_str(),
    }))
}

fn state_kind_json(name: &str, value: serde_json::Value) -> Result<StateKind> {
    StateKind::new(
        "mfm.framework",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    )
    .map_err(|error| lower(error.to_string()))
}

fn framework_config_ref(kind: &str, node_id: &NodeId) -> Result<spec::ConfigRef> {
    spec::framework_config_ref(kind, node_id).map_err(|error| CertifyError::Spec(error.to_string()))
}

fn renderer_descriptor(public_schema_id: &SchemaId) -> Result<spec::RendererDescriptorIdentity> {
    let renderer_kind =
        spec::RendererKind::new("public-output/json").map_err(|error| lower(error.to_string()))?;
    let renderer_version = spec::RendererVersion::new("mfm.renderer.public_output_json.v1")
        .map_err(|error| lower(error.to_string()))?;
    let canonicalizer_identity = spec::CanonicalizerIdentity::new("sha256-jcs-v1")
        .map_err(|error| lower(error.to_string()))?;
    let descriptor_id = spec::RendererDescriptorIdentity {
        descriptor_id: DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        ),
        renderer_kind: renderer_kind.clone(),
        renderer_version: renderer_version.clone(),
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity: canonicalizer_identity.clone(),
    }
    .expected_descriptor_id()
    .map_err(|error| CertifyError::Spec(error.to_string()))?;
    Ok(spec::RendererDescriptorIdentity {
        descriptor_id,
        renderer_kind,
        renderer_version,
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity,
    })
}

fn framework_bridge_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "bridge_same_value",
        serde_json::json!({ "framework": "bridge_same_value" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.bridge_same_value.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.bridge_same_value",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context: spec::StateContextDescriptorSpec::no_context(),
        effect_kind: Pure::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "pure",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_side_effect_verify_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
    context: spec::StateContextDescriptorSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "side_effect_verify",
        serde_json::json!({ "framework": "side_effect_verify" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.side_effect_verify.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.side_effect_verify",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context,
        effect_kind: mfm_effects::ReadExternal::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "read_external",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_render_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
    context: spec::StateContextDescriptorSpec,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "render_public_outputs",
        serde_json::json!({ "framework": "render_public_outputs" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.render_public_outputs.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_state_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.render_public_outputs",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context,
        effect_kind: ManagedPlatformWrite::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "managed_platform_write",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_project_retention_manifest_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "project_retention_manifest",
        serde_json::json!({ "framework": "project_retention_manifest" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.project_retention_manifest.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_lifecycle_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.project_retention_manifest",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context: spec::StateContextDescriptorSpec::no_context(),
        effect_kind: ManagedPlatformWrite::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "managed_platform_write",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_complete_run_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "complete_run",
        serde_json::json!({ "framework": "complete_run" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.complete_run.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_lifecycle_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.complete_run",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context: spec::StateContextDescriptorSpec::no_context(),
        effect_kind: ManagedPlatformWrite::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "managed_platform_write",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_resolve_saga_terminal_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
) -> Result<spec::StateDescriptorIdentity> {
    let state_kind = state_kind_json(
        "resolve_saga_terminal",
        serde_json::json!({ "framework": "resolve_saga_terminal" }),
    )?;
    let state_version = StateVersion::new("mfm.framework.state.resolve_saga_terminal.v1")
        .map_err(|error| lower(error.to_string()))?;
    framework_lifecycle_descriptor(FrameworkStateDescriptorParts {
        name: "mfm.framework.resolve_saga_terminal",
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context: spec::StateContextDescriptorSpec::no_context(),
        effect_kind: ManagedPlatformWrite::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "managed_platform_write",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_lifecycle_descriptor(
    parts: FrameworkStateDescriptorParts<'_>,
) -> Result<spec::StateDescriptorIdentity> {
    framework_state_descriptor(parts)
}

struct FrameworkStateDescriptorParts<'a> {
    name: &'static str,
    state_kind: StateKind,
    state_version: StateVersion,
    config_schema_id: &'a SchemaId,
    input_schema_id: &'a SchemaId,
    output_schema_id: &'a SchemaId,
    output_semantic_type_id: &'a SemanticTypeId,
    context: spec::StateContextDescriptorSpec,
    effect_kind: EffectKind,
    runner: &'static str,
    capabilities: CapabilitySetDescriptor,
}

fn framework_state_descriptor(
    parts: FrameworkStateDescriptorParts<'_>,
) -> Result<spec::StateDescriptorIdentity> {
    let FrameworkStateDescriptorParts {
        name,
        state_kind,
        state_version,
        config_schema_id,
        input_schema_id,
        output_schema_id,
        output_semantic_type_id,
        context,
        effect_kind,
        runner,
        capabilities,
    } = parts;
    let effect = effect_descriptor_for_kind(&effect_kind)?;
    let descriptor_id = descriptor_id_json(serde_json::json!({
        "capabilities": capabilities.capabilities.iter().map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        }).collect::<Vec<_>>(),
        "config_schema_id": config_schema_id.as_str(),
        "context": state_context_descriptor_json(&context),
        "effect": {
            "class": effect.class.as_str(),
            "kind": effect.kind.as_str(),
            "name": effect.name,
            "version": effect.version.as_str(),
        },
        "emitted_fact_descriptors": [],
        "input_context": state_input_context_contract_json(&spec::StateInputContextContractSpec::no_context()),
        "input_schema_id": input_schema_id.as_str(),
        "kind": state_kind.as_str(),
        "name": name,
        "output_context": state_output_context_contract_json(&spec::StateOutputContextContractSpec::no_context()),
        "output_schema_id": output_schema_id.as_str(),
        "output_semantic_type_id": output_semantic_type_id.as_str(),
        "runner": runner,
        "side_effect_contract_digest": null,
        "version": state_version.as_str(),
    }))?;
    Ok(spec::StateDescriptorIdentity {
        descriptor_id,
        name: name.to_owned(),
        state_kind,
        state_version,
        context,
        input_context: spec::StateInputContextContractSpec::no_context(),
        output_context: spec::StateOutputContextContractSpec::no_context(),
        config_schema_id: config_schema_id.clone(),
        input_schema_id: input_schema_id.clone(),
        output_schema_id: output_schema_id.clone(),
        output_semantic_type_id: output_semantic_type_id.clone(),
        effect_kind,
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities,
        emitted_fact_descriptors: Vec::new(),
        runner: runner.to_owned(),
        side_effect_contract_digest: None,
    })
}

fn render_node_id(
    root_scope_id: &ScopeId,
    key: &str,
    output_spec_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "framework": "public_output_render",
            "key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "output_spec_digest": output_spec_digest.as_str(),
            "scope_id": root_scope_id.as_str(),
        }))?,
    ))
}

fn project_retention_manifest_node_id_from_spec(
    scope_id: &ScopeId,
    key: &str,
    retention: &spec::ProjectRetentionManifestNodeSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "project_retention_manifest",
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "public_output_receipt_cell": retention.public_output_receipt_cell.as_str(),
            "public_schema_id": retention.public_schema_id.as_str(),
            "scope_id": scope_id.as_str(),
        }))?,
    ))
}

fn complete_run_node_id_from_spec(
    scope_id: &ScopeId,
    key: &str,
    complete: &spec::CompleteRunNodeSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "complete_run",
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "public_schema_id": complete.public_schema_id.as_str(),
            "retention_manifest_receipt_cell": complete.retention_manifest_receipt_cell.as_str(),
            "scope_id": scope_id.as_str(),
        }))?,
    ))
}

fn resolve_saga_terminal_node_id_from_spec(
    scope_id: &ScopeId,
    key: &str,
    resolve: &spec::ResolveSagaTerminalNodeSpec,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "framework": "resolve_saga_terminal",
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "public_schema_id": resolve.public_schema_id.as_str(),
            "scope_id": scope_id.as_str(),
        }))?,
    ))
}

fn state_node_id_from_spec(
    node: &spec::NodeSpec,
    config_ref_digest: &ContentDigest,
) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": config_ref_digest.as_str(),
            "context": node_context_json(&node.context),
            "input_binding_digest": node.input_bindings.digest.as_str(),
            "local_node_key": node.stable_key.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "scope_id": node.scope_id.as_str(),
            "state_kind": node.state_kind.as_str(),
            "state_version": node.state_version.as_str(),
        }))?,
    ))
}

fn bridge_node_id_from_spec(key: &str, bridge: &spec::BridgeNodeSpec) -> Result<NodeId> {
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "bridge_kind": bridge_kind_json(bridge.bridge_kind),
            "local_node_key": key,
            "lowering_version": spec::LOWERING_VERSION,
            "policy": bridge_policy_json(bridge.policy),
            "source_cell_id": bridge.source_cell_id.as_str(),
            "source_scope_id": bridge.source_scope_id.as_str(),
            "target_scope_id": bridge.target_scope_id.as_str(),
        }))?,
    ))
}

fn operation_instance_id_from_spec(
    frame: &spec::OperationLineageFrameSpec,
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<OperationInstanceId> {
    Ok(OperationInstanceId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": frame.config_ref_digest.as_str(),
            "input_binding_digest": frame.input_binding_digest.as_str(),
            "operation_descriptor_id": descriptor.descriptor_id.as_str(),
            "operation_key": frame.operation_key.as_str(),
            "operation_kind": descriptor.operation_kind.as_str(),
            "operation_version": descriptor.operation_version.as_str(),
            "parent_operation_lineage": frame.parent_planning_lineage.lineage_digest.as_str(),
            "parent_scope_id": frame.scope_id.as_str(),
        }))?,
    ))
}

fn operation_lineage_frame_digest_from_spec(
    frame: &spec::OperationLineageFrameSpec,
    descriptor: &spec::OperationDescriptorIdentity,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "config_digest": frame.config_ref_digest.as_str(),
        "expansion_abi": descriptor.expansion_abi.as_str(),
        "input_digest": frame.input_binding_digest.as_str(),
        "operation_descriptor_id": descriptor.descriptor_id.as_str(),
        "operation_instance_id": frame.operation_instance_id.as_str(),
        "operation_key": frame.operation_key.as_str(),
        "operation_kind": descriptor.operation_kind.as_str(),
        "operation_version": descriptor.operation_version.as_str(),
        "parent_operation_lineage": frame.parent_planning_lineage.lineage_digest.as_str(),
        "output_handles": frame.output_cells
            .iter()
            .map(|cell_id| {
                let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!("operation frame references missing output cell {cell_id}"),
                    )
                })?;
                Ok(serde_json::json!({
                    "cell_id": cell.cell_id.as_str(),
                    "context": cell_context_json(&cell.context),
                    "schema_id": cell.schema_id.as_str(),
                    "scope_id": cell.scope_id.as_str(),
                    "semantic_type_id": cell.semantic_type_id.as_str(),
                    "value_lineage": cell.value_lineage.lineage_digest.as_str(),
                }))
            })
            .collect::<Result<Vec<_>>>()?,
        "scope_id": frame.scope_id.as_str(),
    }))
}

fn cell_context_json(context: &spec::CellContextSpec) -> serde_json::Value {
    match context {
        spec::CellContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "bound",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn framework_cell_id(
    scope_id: &ScopeId,
    node_id: &NodeId,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "output_index": 0,
            "producer": { "node_id": node_id.as_str(), "kind": "node" },
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

fn cell_id_from_parts(
    scope_id: &ScopeId,
    producer: &spec::CellProducer,
    semantic_type_id: &SemanticTypeId,
    schema_id: &SchemaId,
) -> Result<CellId> {
    Ok(CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "lowering_version": spec::LOWERING_VERSION,
            "output_index": 0,
            "producer": cell_producer_json(producer),
            "schema_id": schema_id.as_str(),
            "scope_id": scope_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?,
    ))
}

fn render_value_lineage_ref(
    scope_id: &ScopeId,
    node_id: &NodeId,
    input_cells: &[CellId],
    planning_lineage: &spec::PlanningLineage,
    config_ref_digest: &ContentDigest,
) -> Result<spec::ValueLineageRef> {
    Ok(spec::ValueLineageRef {
        lineage_digest: content_digest_json(serde_json::json!({
            "config_ref_digest": config_ref_digest.as_str(),
            "domain_keys": [],
            "input_cells": sorted_cell_ids(input_cells.to_vec())
                .iter()
                .map(CellId::as_str)
                .collect::<Vec<_>>(),
            "operation_lineage": planning_lineage_json(planning_lineage),
            "producer": { "kind": "node", "node_id": node_id.as_str() },
            "scope_id": scope_id.as_str(),
            "transform_policy": "state_output",
        }))?,
    })
}

fn effect_descriptor_for_kind(effect_kind: &EffectKind) -> Result<mfm_effects::EffectDescriptor> {
    let pure = Pure::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &pure.kind {
        return Ok(pure);
    }
    let managed = ManagedPlatformWrite::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &managed.kind {
        return Ok(managed);
    }
    let side_effect = ApplySideEffect::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &side_effect.kind {
        return Ok(side_effect);
    }
    let read = mfm_effects::ReadExternal::descriptor().map_err(|error| lower(error.to_string()))?;
    if effect_kind == &read.kind {
        return Ok(read);
    }
    Err(problem(
        ProblemClass::InvalidSemanticTransition,
        format!("unknown effect kind {effect_kind}"),
    ))
}

fn effect_class_for_kind(effect_kind: &EffectKind) -> Result<(EffectClass, &'static str)> {
    let effect = effect_descriptor_for_kind(effect_kind)?;
    Ok((effect.class, effect.name))
}

fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
    match node {
        spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "context": input_context_json(&cell.context),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": match cell.required_terminal {
                spec::RequiredTerminal::ProducedOnly => "produced_only",
                spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
            },
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields.iter().map(|field| {
                serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": input_node_json(&field.node),
                })
            }).collect::<Vec<_>>(),
            "kind": "struct",
        }),
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_key_refs_json(domain_keys),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

fn node_context_json(context: &spec::NodeContextSpec) -> serde_json::Value {
    match context {
        spec::NodeContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::NodeContextSpec::Required { context_ref } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "required",
        }),
    }
}

fn input_context_json(context: &spec::InputContextSpec) -> serde_json::Value {
    match context {
        spec::InputContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::InputContextSpec::Required {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn context_producer_json(producer: &spec::ContextProducerSpec) -> serde_json::Value {
    serde_json::json!({
        "producer_descriptor_ids": producer
            .producer_descriptor_ids
            .iter()
            .map(DescriptorId::as_str)
            .collect::<Vec<_>>(),
        "seed_producers_allowed": producer.seed_producers_allowed,
    })
}

fn domain_key_refs_json(domain_keys: &[spec::StableDomainKeyRef]) -> Vec<serde_json::Value> {
    domain_keys
        .iter()
        .map(|key| {
            serde_json::json!({
                "content_digest": key.content_digest.as_str(),
                "schema_id": key.schema_id.as_str(),
            })
        })
        .collect()
}

fn capability_set_json(descriptor: &CapabilitySetDescriptor) -> Vec<serde_json::Value> {
    descriptor
        .capabilities
        .iter()
        .map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        })
        .collect()
}

fn fact_descriptor_refs_json(refs: &[spec::FactDescriptorRef]) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|reference| {
            serde_json::json!({
                "descriptor_hash": reference.descriptor_hash.as_str(),
            })
        })
        .collect()
}

fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

fn planning_lineage_json(lineage: &spec::PlanningLineage) -> serde_json::Value {
    serde_json::json!({
        "active_instances": lineage
            .active_operation_instances
            .iter()
            .map(OperationInstanceId::as_str)
            .collect::<Vec<_>>(),
        "completed_frames": lineage
            .completed_operation_frames
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>(),
        "digest": lineage.lineage_digest.as_str(),
    })
}

#[cfg(test)]
mod tests;
