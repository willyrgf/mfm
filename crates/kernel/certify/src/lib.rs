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
mod lowering_support;
pub(crate) use self::lowering_support::*;
#[path = "certificate_codec.rs"]
mod certificate_codec;
#[path = "certificate_evidence.rs"]
mod certificate_evidence;
#[path = "certificate_models.rs"]
mod certificate_models;
use self::certificate_evidence::*;
pub use self::certificate_models::{
    CertifiedDescriptorEvidence, CertifiedDescriptorFamily,
    CertifiedManualAuthorizationVerifierEvidence, CertifiedOperatorAuthorityMemberEvidence,
    CertifiedOperatorAuthoritySnapshotEvidence, CertifiedSchemaRole,
    CertifiedSchemaRoleGrantEvidence, CertifiedSpecCertificate, CertifiedSpecCertificateEvidence,
    PersistedSpecCertificateParts, UntrustedSpecCertificateParts,
};
#[path = "side_effect_contract.rs"]
mod side_effect_contract;
pub use self::side_effect_contract::{CertifiedRemediationLink, CertifiedSideEffectContract};
#[path = "framework_support.rs"]
mod framework_support;
#[path = "validation.rs"]
mod validation;
use self::certificate_codec::*;
pub(crate) use self::framework_support::*;
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

/// Defines matching program and certification registries from child registries plus local types.
#[macro_export]
macro_rules! define_program_descriptor_registry {
    (
        state_registry: $state_vis:vis $state_registry:ident,
        operation_registry: $operation_vis:vis $operation_registry:ident,
        certification: $cert_vis:vis $certification:ident,
        includes: [
            $({
                state_registry: $include_state:path,
                operation_registry: $include_operation:path,
                certification: $include_certification:path $(,)?
            }),* $(,)?
        ],
        states: [$($state:ty),* $(,)?],
        operations: [$($operation:ty),* $(,)?] $(,)?
    ) => {
        #[doc = "Builds the state registry used for typed authoring and certification."]
        $state_vis fn $state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
            let mut states = mfm_program::StateRegistryBuilder::new();
            $(states.include($include_state()?)?;)*
            $(states.register::<$state>()?;)*
            Ok(states.into_snapshot())
        }

        #[doc = "Builds the operation registry used for typed authoring and certification."]
        $operation_vis fn $operation_registry() -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
            let mut operations = mfm_program::OperationRegistryBuilder::new();
            $(operations.include($include_operation()?)?;)*
            $(operations.register::<$operation>()?;)*
            Ok(operations.into_snapshot())
        }

        #[doc = "Adds program descriptors to a trusted certification registry."]
        $cert_vis fn $certification(
            registry: &mut $crate::CertificationRegistry,
        ) -> $crate::Result<()> {
            $($include_certification(registry)?;)*
            $(registry.register_state::<$state>()?;)*
            $(registry.register_operation::<$operation>()?;)*

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

#[cfg(test)]
mod tests;
