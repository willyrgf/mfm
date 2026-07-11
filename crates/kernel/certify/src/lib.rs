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
#[path = "certificate_codec.rs"]
mod certificate_codec;
#[path = "certificate_models.rs"]
mod certificate_models;
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

#[cfg(test)]
mod tests;
