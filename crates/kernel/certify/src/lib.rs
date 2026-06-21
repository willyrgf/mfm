#![warn(missing_docs)]
//! Certification contracts for MFM typed execution specs.
//!
//! This crate lowers typed program drafts into `mfm_spec::v1` specs and validates
//! that a v1 typed execution spec is the only semantic runtime contract.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{CapabilitySet, CapabilitySetDescriptor, NoCaps};
use mfm_effects::{ApplySideEffect, EffectClass, EffectSpec, ManagedPlatformWrite, Pure};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EffectKind,
    NodeId, OperationInstanceId, RunId, SchemaId, ScopeId, SemanticTypeId, SpecHash, StateKind,
    StateVersion,
};
use mfm_program as program;
use mfm_spec::v1 as spec;
use mfm_values::MfmConfig;

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
                Some(spec::FrameworkNodeSpec::Bridge(_)) | None => {}
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
            || !link.forward_confirmed
        {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked ledger outside certified confirmed forward node {}",
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
    /// Whether the linked forward ledger is confirmed.
    pub forward_confirmed: bool,
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

/// Source that validated supplied typed config bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigValidationSource {
    /// A registered typed config validator decoded and validated the bytes.
    RegisteredValidator,
    /// A trusted draft config reference matched the bytes exactly.
    TrustedExactRef,
}

#[derive(Clone)]
struct ConfigValidator {
    schema_id: SchemaId,
    validate: fn(&[u8]) -> Result<()>,
}

impl fmt::Debug for ConfigValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigValidator")
            .field("schema_id", &self.schema_id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ConfigValidator {
    fn eq(&self, other: &Self) -> bool {
        self.schema_id == other.schema_id
    }
}

impl Eq for ConfigValidator {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrustedConfigRef {
    schema_id: SchemaId,
    digest: ContentDigest,
    byte_len: u64,
}

/// Registry authority used when certifying an already-lowered typed spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CertificationRegistry {
    states: BTreeMap<String, spec::StateDescriptorIdentity>,
    operations: BTreeMap<String, spec::OperationDescriptorIdentity>,
    config_validators: BTreeMap<String, ConfigValidator>,
    trusted_config_refs: BTreeMap<String, TrustedConfigRef>,
    schema_roles: BTreeMap<String, BTreeSet<CertifiedSchemaRole>>,
    manual_authorization_verifiers: BTreeSet<String>,
    operator_authority_snapshots: BTreeMap<String, spec::OperatorAuthoritySnapshotSpec>,
}

impl CertificationRegistry {
    /// Creates an empty certification registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the deterministic digest of this registry authority.
    pub fn digest(&self) -> Result<ContentDigest> {
        let operations = self
            .operations
            .values()
            .map(operation_descriptor_ref_json)
            .collect::<Result<Vec<_>>>()?;
        let states = self
            .states
            .values()
            .map(state_descriptor_ref_json)
            .collect::<Result<Vec<_>>>()?;
        content_digest_json(serde_json::json!({
            "algorithm": REGISTRY_DIGEST_ALGORITHM,
            "operations": operations,
            "manual_authorization_verifiers": self
                .manual_authorization_verifiers
                .iter()
                .collect::<Vec<_>>(),
            "operator_authority_snapshots": self
                .operator_authority_snapshots
                .values()
                .map(operator_authority_snapshot_json)
                .collect::<Vec<_>>(),
            "schema_role_grants": self
                .schema_roles
                .iter()
                .flat_map(|(schema_id, roles)| {
                    roles.iter().map(move |role| {
                        serde_json::json!({
                            "role": role.as_str(),
                            "schema_id": schema_id,
                        })
                    })
                })
                .collect::<Vec<_>>(),
            "states": states,
        }))
    }

    /// Adds a framework-validated registered state descriptor to this registry.
    pub fn register_state<S>(&mut self, registered: &program::RegisteredState<S>) -> Result<()>
    where
        S: program::StateSpec,
    {
        self.insert_state(state_descriptor_identity_from_registered(
            registered.descriptor(),
            registered.runner(),
        )?)?;
        self.insert_config_validator(config_validator_for::<S::Config>()?)
    }

    /// Adds a framework-validated registered operation descriptor to this registry.
    pub fn register_operation<O>(
        &mut self,
        registered: &program::RegisteredOperation<O>,
    ) -> Result<()>
    where
        O: program::Operation,
    {
        self.insert_operation(operation_descriptor_identity_from_registered(
            registered.descriptor(),
        ))?;
        self.insert_config_validator(config_validator_for::<O::Config>()?)
    }

    /// Adds a trusted already-lowered state descriptor identity to this registry.
    ///
    /// This is for registry assembly code whose descriptor source is already trusted. Do not derive
    /// this identity from the same persisted spec that will be certified; persisted spec bytes are
    /// hostile until [`verify_persisted_spec_certificate`] succeeds.
    pub fn register_state_descriptor(
        &mut self,
        descriptor: spec::StateDescriptorIdentity,
    ) -> Result<()> {
        self.insert_state(descriptor)
    }

    /// Adds a trusted already-lowered operation descriptor identity to this registry.
    ///
    /// This is for registry assembly code whose descriptor source is already trusted. Do not derive
    /// this identity from the same persisted spec that will be certified; persisted spec bytes are
    /// hostile until [`verify_persisted_spec_certificate`] succeeds.
    pub fn register_operation_descriptor(
        &mut self,
        descriptor: spec::OperationDescriptorIdentity,
    ) -> Result<()> {
        self.insert_operation(descriptor)
    }

    /// Grants a schema id a certification role.
    pub fn register_schema_role(
        &mut self,
        schema_id: SchemaId,
        role: CertifiedSchemaRole,
    ) -> Result<()> {
        self.insert_schema_role(schema_id, role);
        Ok(())
    }

    /// Registers a manual authorization verifier identity.
    pub fn register_manual_authorization_verifier(
        &mut self,
        verifier_id: spec::ManualAuthorizationVerifierId,
    ) -> Result<()> {
        self.manual_authorization_verifiers
            .insert(verifier_id.as_str().to_owned());
        Ok(())
    }

    /// Registers an operator authority snapshot used by manual resolution certification.
    pub fn register_operator_authority_snapshot(
        &mut self,
        snapshot: spec::OperatorAuthoritySnapshotSpec,
    ) -> Result<()> {
        self.insert_operator_authority_snapshot(snapshot)
    }

    /// Builds the descriptor registry used by a framework-lowered program draft.
    pub fn from_program_draft(draft: &program::TypedProgramDraft) -> Result<Self> {
        let mut registry = Self::new();
        for node in draft.state_nodes() {
            registry.insert_state(state_descriptor_identity_from_program(node)?)?;
            registry.insert_trusted_config_binding(&node.config)?;
        }
        for frame in draft.operation_lineage() {
            registry.insert_operation(operation_descriptor_identity_from_program(frame))?;
            registry.insert_trusted_config_binding(&frame.config)?;
        }
        Ok(registry)
    }

    /// Returns the trusted registry subset named by a parsed spec's descriptor identities.
    ///
    /// The parsed spec supplies only selector keys. Operation identities must be present in this
    /// trusted registry. State identities are copied when the trusted registry owns them; framework
    /// generated states remain subject to full verifier validation before authority can be minted.
    pub fn scoped_for_spec(&self, spec: &spec::TypedExecutionSpec) -> Result<Self> {
        let mut scoped = Self::new();
        for descriptor in &spec.descriptor_identities {
            match descriptor {
                spec::DescriptorIdentity::State(identity) => {
                    if let Some(trusted) = self.states.get(identity.descriptor_id.as_str()) {
                        if trusted != identity.as_ref() {
                            return Err(certificate(format!(
                                "state descriptor {} does not match the trusted registry",
                                identity.descriptor_id
                            )));
                        }
                        scoped.insert_state(trusted.clone())?;
                        if let Some(validator) = self
                            .config_validators
                            .get(trusted.config_schema_id.as_str())
                        {
                            scoped.insert_config_validator(validator.clone())?;
                        }
                    }
                }
                spec::DescriptorIdentity::Operation(identity) => {
                    let Some(trusted) = self.operations.get(identity.descriptor_id.as_str()) else {
                        return Err(certificate(format!(
                            "operation descriptor {} is not present in the trusted registry",
                            identity.descriptor_id
                        )));
                    };
                    if trusted != identity.as_ref() {
                        return Err(certificate(format!(
                            "operation descriptor {} does not match the trusted registry",
                            identity.descriptor_id
                        )));
                    }
                    scoped.insert_operation(trusted.clone())?;
                    if let Some(validator) = self
                        .config_validators
                        .get(trusted.config_schema_id.as_str())
                    {
                        scoped.insert_config_validator(validator.clone())?;
                    }
                }
                spec::DescriptorIdentity::Renderer(_) => {}
            }
        }
        for config_ref in &spec.config_refs {
            let key = config_ref_key(config_ref);
            if let Some(trusted) = self.trusted_config_refs.get(&key) {
                if trusted.schema_id != config_ref.schema_id
                    || trusted.digest != config_ref.digest
                    || trusted.byte_len != config_ref.byte_len
                {
                    return Err(certificate(format!(
                        "trusted config ref {} does not match parsed spec",
                        config_ref.digest
                    )));
                }
                scoped.trusted_config_refs.insert(key, trusted.clone());
            }
        }
        for manual in manual_resolution_specs(spec) {
            scoped.copy_schema_role_from(
                self,
                &manual.evidence_schema,
                CertifiedSchemaRole::ManualResolutionEvidence,
            )?;
            scoped
                .copy_manual_authorization_verifier_from(self, &manual.authorization.verifier_id)?;
            scoped.copy_operator_authority_snapshot_from(
                self,
                &manual.authorization.authority.authority_id,
            )?;
        }
        Ok(scoped)
    }

    /// Validates supplied config bytes for a certified config reference when this registry owns
    /// either a typed config validator or an exact trusted draft reference.
    pub fn validate_config_ref_bytes(
        &self,
        config_ref: &spec::ConfigRef,
        bytes: &[u8],
    ) -> Result<Option<ConfigValidationSource>> {
        let canonical = canonical_config_bytes_for_ref(config_ref, bytes)?;
        if let Some(validator) = self.config_validators.get(config_ref.schema_id.as_str()) {
            (validator.validate)(canonical.as_bytes())?;
            return Ok(Some(ConfigValidationSource::RegisteredValidator));
        }
        let key = config_ref_key(config_ref);
        if self.trusted_config_refs.contains_key(&key) {
            return Ok(Some(ConfigValidationSource::TrustedExactRef));
        }
        Ok(None)
    }

    fn insert_state(&mut self, descriptor: spec::StateDescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id.as_str().to_owned();
        if let Some(existing) = self.states.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered state descriptor {key}"),
                ));
            }
        } else {
            self.states.insert(key, descriptor);
        }
        Ok(())
    }

    fn insert_operation(&mut self, descriptor: spec::OperationDescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id.as_str().to_owned();
        if let Some(existing) = self.operations.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered operation descriptor {key}"),
                ));
            }
        } else {
            self.operations.insert(key, descriptor);
        }
        Ok(())
    }

    fn insert_config_validator(&mut self, validator: ConfigValidator) -> Result<()> {
        let key = validator.schema_id.as_str().to_owned();
        if let Some(existing) = self.config_validators.get(&key) {
            if existing != &validator {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting registered config validator {key}"),
                ));
            }
        } else {
            self.config_validators.insert(key, validator);
        }
        Ok(())
    }

    fn insert_schema_role(&mut self, schema_id: SchemaId, role: CertifiedSchemaRole) {
        self.schema_roles
            .entry(schema_id.as_str().to_owned())
            .or_default()
            .insert(role);
    }

    fn copy_schema_role_from(
        &mut self,
        trusted: &CertificationRegistry,
        schema_id: &SchemaId,
        role: CertifiedSchemaRole,
    ) -> Result<()> {
        let Some(roles) = trusted.schema_roles.get(schema_id.as_str()) else {
            return Err(certificate(format!(
                "manual schema {} is not present in the trusted registry",
                schema_id
            )));
        };
        if !roles.contains(&role) {
            return Err(certificate(format!(
                "manual schema {} lacks certified role {}",
                schema_id,
                role.as_str()
            )));
        }
        self.insert_schema_role(schema_id.clone(), role);
        Ok(())
    }

    fn copy_manual_authorization_verifier_from(
        &mut self,
        trusted: &CertificationRegistry,
        verifier_id: &spec::ManualAuthorizationVerifierId,
    ) -> Result<()> {
        if !trusted
            .manual_authorization_verifiers
            .contains(verifier_id.as_str())
        {
            return Err(certificate(format!(
                "manual authorization verifier {} is not present in the trusted registry",
                verifier_id
            )));
        }
        self.manual_authorization_verifiers
            .insert(verifier_id.as_str().to_owned());
        Ok(())
    }

    fn copy_operator_authority_snapshot_from(
        &mut self,
        trusted: &CertificationRegistry,
        authority_id: &spec::OperatorAuthorityId,
    ) -> Result<()> {
        let Some(snapshot) = trusted
            .operator_authority_snapshots
            .get(authority_id.as_str())
        else {
            return Err(certificate(format!(
                "operator authority snapshot {} is not present in the trusted registry",
                authority_id
            )));
        };
        self.insert_operator_authority_snapshot(snapshot.clone())
    }

    fn insert_operator_authority_snapshot(
        &mut self,
        snapshot: spec::OperatorAuthoritySnapshotSpec,
    ) -> Result<()> {
        let key = snapshot.authority_id.as_str().to_owned();
        if let Some(existing) = self.operator_authority_snapshots.get(&key) {
            if existing != &snapshot {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting operator authority snapshot {key}"),
                ));
            }
        } else {
            self.operator_authority_snapshots.insert(key, snapshot);
        }
        Ok(())
    }

    fn insert_trusted_config_binding(
        &mut self,
        binding: &program::ConfigBindingSpec,
    ) -> Result<()> {
        let trusted = TrustedConfigRef {
            schema_id: binding.schema_id.clone(),
            digest: binding.content_digest.clone(),
            byte_len: binding.byte_len as u64,
        };
        let key = format!("{}:{}", trusted.schema_id, trusted.digest);
        if let Some(existing) = self.trusted_config_refs.get(&key) {
            if existing != &trusted {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!("conflicting trusted config ref {key}"),
                ));
            }
        } else {
            self.trusted_config_refs.insert(key, trusted);
        }
        Ok(())
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

#[derive(Debug, Clone)]
struct CellInfo {
    producer: spec::CellProducer,
    semantic_type_id: SemanticTypeId,
    schema_id: SchemaId,
    value_lineage: spec::ValueLineageRef,
}

struct DraftLowerer<'a> {
    draft: &'a program::TypedProgramDraft,
    cells: Vec<spec::CellSpec>,
    cell_info: BTreeMap<String, CellInfo>,
    config_refs: BTreeMap<String, spec::ConfigRef>,
    descriptor_identities: BTreeMap<String, spec::DescriptorIdentity>,
    value_lineages: BTreeMap<String, spec::ValueLineage>,
    nodes: Vec<spec::NodeSpec>,
}

impl<'a> DraftLowerer<'a> {
    fn new(draft: &'a program::TypedProgramDraft) -> Result<Self> {
        Ok(Self {
            draft,
            cells: Vec::new(),
            cell_info: BTreeMap::new(),
            config_refs: BTreeMap::new(),
            descriptor_identities: BTreeMap::new(),
            value_lineages: BTreeMap::new(),
            nodes: Vec::new(),
        })
    }

    fn lower(&mut self) -> Result<spec::TypedExecutionSpec> {
        let scopes = self.lower_scopes()?;
        let seeds = self.lower_seeds()?;
        self.lower_state_nodes()?;
        let remediations = self.lower_remediation_nodes()?;
        self.lower_bridge_nodes()?;
        let public_outputs = self.lower_public_outputs()?;
        let planning_lineage = self.lower_operation_lineage()?;

        spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
            authoring: self.authoring_provenance()?,
            saga: lower_saga_policy(self.draft.saga_policy()),
            scopes,
            seeds,
            descriptor_identities: self.descriptor_identities.values().cloned().collect(),
            config_refs: self.config_refs.values().cloned().collect(),
            nodes: self.nodes.clone(),
            remediations,
            cells: self.cells.clone(),
            value_lineages: self.value_lineages.values().cloned().collect(),
            planning_lineage,
            public_outputs,
        })
        .map_err(|error| CertifyError::Spec(error.to_string()))
    }

    fn lower_scopes(&self) -> Result<Vec<spec::ScopeSpec>> {
        self.draft
            .scopes()
            .iter()
            .map(|scope| {
                Ok(spec::ScopeSpec {
                    scope_id: scope.scope_id.clone(),
                    parent_scope_id: scope.parent_scope_id.clone(),
                    stable_key: stable_author_key(scope.key.as_str())?,
                    planning_lineage: lower_planning_lineage(&scope.planning_lineage),
                })
            })
            .collect()
    }

    fn lower_seeds(&mut self) -> Result<Vec<spec::SeedSpec>> {
        let mut lowered = Vec::new();
        for seed in self.draft.seeds() {
            let lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(seed.value_lineage.digest()),
                scope_id: seed.scope_id.clone(),
                producer: spec::CellProducer::Seed(seed.seed_id.clone()),
                input_cells: Vec::new(),
                config_ref_digest: None,
                planning_lineage: empty_planning_lineage()?,
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::Source,
            };
            self.insert_value_lineage(lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: seed.cell_id.clone(),
                producer: spec::CellProducer::Seed(seed.seed_id.clone()),
                scope_id: seed.scope_id.clone(),
                semantic_type_id: seed.semantic_type_id.clone(),
                schema_id: seed.schema_id.clone(),
                value_lineage: lineage_ref(seed.value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            })?;
            lowered.push(spec::SeedSpec {
                seed_id: seed.seed_id.clone(),
                seed_key: stable_author_key(seed.key.as_str())?,
                cell_id: seed.cell_id.clone(),
                scope_id: seed.scope_id.clone(),
                semantic_type_id: seed.semantic_type_id.clone(),
                schema_id: seed.schema_id.clone(),
                required_digest: Some(seed.content_digest.clone()),
            });
        }
        Ok(lowered)
    }

    fn lower_state_nodes(&mut self) -> Result<()> {
        for node in self.draft.state_nodes() {
            let lowered = self.lower_state_node(node)?;
            self.nodes.push(lowered);
        }
        Ok(())
    }

    fn lower_remediation_nodes(&mut self) -> Result<BTreeMap<NodeId, spec::NodeSpec>> {
        let mut remediations = BTreeMap::new();
        for (forward_node_id, node) in self.draft.remediation_nodes() {
            let lowered = self.lower_state_node(node)?;
            remediations.insert(forward_node_id.clone(), lowered);
        }
        Ok(remediations)
    }

    fn lower_state_node(&mut self, node: &program::StateNodeSpec) -> Result<spec::NodeSpec> {
        let config_ref = self.config_ref(&node.config)?;
        let input_bindings = lower_input_binding(&node.input)?;
        let input_cells = collect_input_cells(&input_bindings.root);
        let planning_lineage = lower_planning_lineage(&node.planning_lineage);
        let domain_keys = node
            .output_domain_keys
            .iter()
            .map(lower_domain_key_ref)
            .collect::<Vec<_>>();
        let output_lineage = spec::ValueLineage {
            lineage_ref: lineage_ref(node.output_value_lineage.digest()),
            scope_id: node.scope_id.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            input_cells: sorted_cell_ids(input_cells.clone()),
            config_ref_digest: Some(node.config.config_ref_digest.clone()),
            planning_lineage: planning_lineage.clone(),
            domain_keys,
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        };
        self.insert_value_lineage(output_lineage)?;
        self.insert_cell(spec::CellSpec {
            cell_id: node.output_cell_id.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            scope_id: node.scope_id.clone(),
            semantic_type_id: node.output_semantic_type_id.clone(),
            schema_id: node.output_schema_id.clone(),
            value_lineage: lineage_ref(node.output_value_lineage.digest()),
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        })?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            state_descriptor_identity_from_program(node)?,
        )))?;
        let side_effect = match (
            node.side_effect_contract_digest.as_ref(),
            node.side_effect_resource_claim.as_ref(),
        ) {
            (Some(digest), Some(resource_claim)) => Some(spec::SideEffectContractSpec {
                contract_digest: digest.clone(),
                resource_claim: resource_claim.clone(),
            }),
            (None, None) => None,
            (Some(_), None) => {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "side-effect node {} is missing resource claim",
                        node.node_id
                    ),
                ));
            }
            (None, Some(_)) => {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "non-side-effect node {} carries resource claim",
                        node.node_id
                    ),
                ));
            }
        };
        Ok(spec::NodeSpec {
            node_id: node.node_id.clone(),
            stable_key: stable_author_key(node.key.as_str())?,
            scope_id: node.scope_id.clone(),
            state_kind: node.state_kind.clone(),
            state_version: node.state_version.clone(),
            descriptor_id: node.state_descriptor_id.clone(),
            config_ref,
            input_bindings,
            output_cell: node.output_cell_id.clone(),
            effect_kind: node.effect_kind.clone(),
            capability_bindings: node.capability_bindings.clone(),
            adapter_bindings: node
                .adapter_bindings
                .iter()
                .map(|binding| spec::AdapterBinding {
                    adapter_kind: binding.adapter_kind.clone(),
                    adapter_version: binding.adapter_version.clone(),
                    binding_digest: None,
                })
                .collect(),
            side_effect,
            framework: None,
            planning_lineage,
            deterministic_predecessors: self.predecessors_for_inputs(&input_cells)?,
        })
    }

    fn lower_bridge_nodes(&mut self) -> Result<()> {
        for bridge in self.draft.bridge_nodes() {
            let source = self
                .cell_info
                .get(bridge.source_cell_id.as_str())
                .cloned()
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "bridge source cell {} has no producer",
                            bridge.source_cell_id
                        ),
                    )
                })?;
            let config_ref = framework_config_ref("bridge_same_value", &bridge.node_id)?;
            self.insert_config_ref(config_ref.clone())?;
            let input_binding =
                single_cell_input_binding(source.clone(), bridge.source_cell_id.clone(), "source")?;
            let planning_lineage = lower_planning_lineage(&bridge.planning_lineage);
            let lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(bridge.target_value_lineage.digest()),
                scope_id: bridge.target_scope_id.clone(),
                producer: spec::CellProducer::Node(bridge.node_id.clone()),
                input_cells: vec![bridge.source_cell_id.clone()],
                config_ref_digest: None,
                planning_lineage: planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::SameValueBridge,
            };
            self.insert_value_lineage(lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: bridge.target_cell_id.clone(),
                producer: spec::CellProducer::Node(bridge.node_id.clone()),
                scope_id: bridge.target_scope_id.clone(),
                semantic_type_id: bridge.semantic_type_id.clone(),
                schema_id: bridge.schema_id.clone(),
                value_lineage: lineage_ref(bridge.target_value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            })?;
            let descriptor = framework_bridge_descriptor(
                &bridge.schema_id,
                &bridge.semantic_type_id,
                &config_ref.schema_id,
                &input_binding.input_schema_id,
            )?;
            self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
                descriptor.clone(),
            )))?;
            self.nodes.push(spec::NodeSpec {
                node_id: bridge.node_id.clone(),
                stable_key: stable_author_key(bridge.key.as_str())?,
                scope_id: bridge.target_scope_id.clone(),
                state_kind: descriptor.state_kind,
                state_version: descriptor.state_version,
                descriptor_id: descriptor.descriptor_id,
                config_ref,
                input_bindings: input_binding,
                output_cell: bridge.target_cell_id.clone(),
                effect_kind: descriptor.effect_kind,
                capability_bindings: descriptor.capabilities,
                adapter_bindings: Vec::new(),
                side_effect: None,
                framework: Some(spec::FrameworkNodeSpec::Bridge(spec::BridgeNodeSpec {
                    bridge_kind: lower_bridge_kind(bridge.bridge_kind),
                    source_scope_id: bridge.source_scope_id.clone(),
                    target_scope_id: bridge.target_scope_id.clone(),
                    source_cell_id: bridge.source_cell_id.clone(),
                    target_cell_id: bridge.target_cell_id.clone(),
                    semantic_type_id: bridge.semantic_type_id.clone(),
                    schema_id: bridge.schema_id.clone(),
                    policy: lower_bridge_policy(bridge.policy),
                    provenance: spec::BridgeProvenance::FrameworkChildScopeV1,
                })),
                planning_lineage,
                deterministic_predecessors: self
                    .predecessors_for_inputs(std::slice::from_ref(&bridge.source_cell_id))?,
            });
        }
        Ok(())
    }

    fn lower_public_outputs(&mut self) -> Result<spec::PublicOutputSpec> {
        let mut outputs = Vec::new();
        for output in self.draft.public_output_spec().outputs() {
            let cell = output.cell();
            let info = self
                .cell_info
                .get(cell.cell_id().as_str())
                .cloned()
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTerminalShape,
                        format!("public output cell {} has no producer", cell.cell_id()),
                    )
                })?;
            outputs.push(spec::PublicOutputCell {
                public_field_path: spec::PublicFieldPath::new(output.public_field_path().as_str())
                    .map_err(|error| lower(error.to_string()))?,
                cell_id: cell.cell_id().clone(),
                producer: info.producer,
                scope_id: cell.scope_id().clone(),
                semantic_type_id: cell.semantic_type_id().clone(),
                schema_id: cell.schema_id().clone(),
                value_lineage: lineage_ref(cell.value_lineage().digest()),
                required_terminal: spec::RequiredTerminal::ProducedOnly,
            });
        }
        let renderer_descriptor =
            renderer_descriptor(self.draft.public_output_spec().public_schema_id())?;
        self.insert_descriptor(spec::DescriptorIdentity::Renderer(Box::new(
            renderer_descriptor.clone(),
        )))?;
        let public_outputs = spec::PublicOutputSpec {
            public_schema_id: self.draft.public_output_spec().public_schema_id().clone(),
            outputs,
            renderer_descriptor,
        };
        let render_receipt_cell = self.lower_public_output_render_node(&public_outputs)?;
        let retention_receipt_cell =
            self.lower_project_retention_manifest_node(&public_outputs, render_receipt_cell)?;
        self.lower_complete_run_node(&public_outputs, retention_receipt_cell.clone())?;
        self.lower_resolve_saga_terminal_node(&public_outputs)?;
        Ok(public_outputs)
    }

    fn lower_public_output_render_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
    ) -> Result<CellId> {
        let output_spec_digest = public_outputs
            .digest()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let node_id = render_node_id(
            self.draft.root_scope_id(),
            self.draft.public_output_spec().key().as_str(),
            &output_spec_digest,
        )?;
        let semantic_type_id = spec::public_output_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::public_output_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let output_cell = framework_cell_id(
            self.draft.root_scope_id(),
            &node_id,
            &semantic_type_id,
            &receipt_schema_id,
        )?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("public_output_render", &node_id)?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let required_cells = public_outputs.outputs.clone();
        let lineage_ref = render_value_lineage_ref(
            self.draft.root_scope_id(),
            &node_id,
            &required_cells
                .iter()
                .map(|output| output.cell_id.clone())
                .collect::<Vec<_>>(),
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        let input_root = spec::InputBindingNodeSpec::Struct(
            required_cells
                .iter()
                .map(|output| spec::NamedInputBindingSpec {
                    field_path: output.public_field_path.clone(),
                    node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: output.public_field_path.clone(),
                        cell_id: output.cell_id.clone(),
                        semantic_type_id: output.semantic_type_id.clone(),
                        schema_id: output.schema_id.clone(),
                        required_terminal: output.required_terminal,
                        value_lineage: output.value_lineage.clone(),
                    })),
                })
                .collect(),
        );
        let input_binding = spec::InputBindingSpec {
            input_schema_id: public_outputs.public_schema_id.clone(),
            input_descriptor_id: descriptor_id_json(serde_json::json!({
                "framework": "public_output_render_input",
                "public_schema_id": public_outputs.public_schema_id.as_str(),
            }))?,
            digest: content_digest_json(input_node_json(&input_root))?,
            root: input_root,
        };
        let descriptor = framework_render_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: self.draft.root_scope_id().clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: sorted_cell_ids(
                required_cells
                    .iter()
                    .map(|output| output.cell_id.clone())
                    .collect(),
            ),
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: self.draft.root_scope_id().clone(),
            semantic_type_id,
            schema_id: receipt_schema_id,
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::PublicOutputArtifact,
            redaction_policy: spec::RedactionPolicy::Public,
        })?;
        let input_cell_ids = required_cells
            .iter()
            .map(|output| output.cell_id.clone())
            .collect::<Vec<_>>();
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key: stable_author_key(self.draft.public_output_spec().key().as_str())?,
            scope_id: self.draft.root_scope_id().clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
                spec::PublicOutputRenderNodeSpec {
                    public_schema_id: public_outputs.public_schema_id.clone(),
                    output_spec_digest,
                    renderer_descriptor: public_outputs.renderer_descriptor.clone(),
                    required_cells,
                },
            )),
            planning_lineage,
            deterministic_predecessors: self.predecessors_for_inputs(&input_cell_ids)?,
        });
        Ok(output_cell)
    }

    fn lower_project_retention_manifest_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
        public_output_receipt_cell: CellId,
    ) -> Result<CellId> {
        let stable_key = stable_author_key("framework/project-retention-manifest")?;
        let retention = spec::ProjectRetentionManifestNodeSpec {
            public_schema_id: public_outputs.public_schema_id.clone(),
            public_output_receipt_cell: public_output_receipt_cell.clone(),
        };
        let node_id = project_retention_manifest_node_id_from_spec(
            self.draft.root_scope_id(),
            stable_key.as_str(),
            &retention,
        )?;
        let semantic_type_id = spec::retention_manifest_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::retention_manifest_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let output_cell = framework_cell_id(
            self.draft.root_scope_id(),
            &node_id,
            &semantic_type_id,
            &receipt_schema_id,
        )?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("project_retention_manifest", &node_id)?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let input_cell = self
            .cells
            .iter()
            .find(|cell| cell.cell_id == public_output_receipt_cell)
            .cloned()
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "retention lifecycle node input cell {public_output_receipt_cell} is missing"
                    ),
                )
            })?;
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "project_retention_manifest",
            "public_output_receipt",
            &input_cell,
        )
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let descriptor = framework_project_retention_manifest_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        let lineage_ref = render_value_lineage_ref(
            self.draft.root_scope_id(),
            &node_id,
            std::slice::from_ref(&public_output_receipt_cell),
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: self.draft.root_scope_id().clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: vec![public_output_receipt_cell.clone()],
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: self.draft.root_scope_id().clone(),
            semantic_type_id,
            schema_id: receipt_schema_id,
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        })?;
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key,
            scope_id: self.draft.root_scope_id().clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)),
            planning_lineage,
            deterministic_predecessors: self
                .predecessors_for_inputs(std::slice::from_ref(&public_output_receipt_cell))?,
        });
        Ok(output_cell)
    }

    fn lower_complete_run_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
        retention_manifest_receipt_cell: CellId,
    ) -> Result<CellId> {
        let stable_key = stable_author_key("framework/complete-run")?;
        let complete = spec::CompleteRunNodeSpec {
            public_schema_id: public_outputs.public_schema_id.clone(),
            retention_manifest_receipt_cell: retention_manifest_receipt_cell.clone(),
        };
        let node_id = complete_run_node_id_from_spec(
            self.draft.root_scope_id(),
            stable_key.as_str(),
            &complete,
        )?;
        let semantic_type_id = spec::complete_run_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::complete_run_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let output_cell = framework_cell_id(
            self.draft.root_scope_id(),
            &node_id,
            &semantic_type_id,
            &receipt_schema_id,
        )?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("complete_run", &node_id)?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let input_cell = self
            .cells
            .iter()
            .find(|cell| cell.cell_id == retention_manifest_receipt_cell)
            .cloned()
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "complete lifecycle node input cell {retention_manifest_receipt_cell} is missing"
                    ),
                )
            })?;
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "complete_run",
            "retention_manifest_receipt",
            &input_cell,
        )
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let descriptor = framework_complete_run_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        let lineage_ref = render_value_lineage_ref(
            self.draft.root_scope_id(),
            &node_id,
            std::slice::from_ref(&retention_manifest_receipt_cell),
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: self.draft.root_scope_id().clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: vec![retention_manifest_receipt_cell.clone()],
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: self.draft.root_scope_id().clone(),
            semantic_type_id,
            schema_id: receipt_schema_id,
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        })?;
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key,
            scope_id: self.draft.root_scope_id().clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::CompleteRun(complete)),
            planning_lineage,
            deterministic_predecessors: self
                .predecessors_for_inputs(std::slice::from_ref(&retention_manifest_receipt_cell))?,
        });
        Ok(output_cell)
    }

    fn lower_resolve_saga_terminal_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
    ) -> Result<CellId> {
        let stable_key = stable_author_key("framework/resolve-saga-terminal")?;
        let resolve = spec::ResolveSagaTerminalNodeSpec {
            public_schema_id: public_outputs.public_schema_id.clone(),
        };
        let node_id = resolve_saga_terminal_node_id_from_spec(
            self.draft.root_scope_id(),
            stable_key.as_str(),
            &resolve,
        )?;
        let semantic_type_id = spec::resolve_saga_terminal_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::resolve_saga_terminal_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let output_cell = framework_cell_id(
            self.draft.root_scope_id(),
            &node_id,
            &semantic_type_id,
            &receipt_schema_id,
        )?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("resolve_saga_terminal", &node_id)?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let input_binding = spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let descriptor = framework_resolve_saga_terminal_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        let lineage_ref = render_value_lineage_ref(
            self.draft.root_scope_id(),
            &node_id,
            &[],
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: self.draft.root_scope_id().clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: Vec::new(),
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: self.draft.root_scope_id().clone(),
            semantic_type_id,
            schema_id: receipt_schema_id,
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        })?;
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key,
            scope_id: self.draft.root_scope_id().clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve)),
            planning_lineage,
            deterministic_predecessors: Vec::new(),
        });
        Ok(output_cell)
    }

    fn lower_operation_lineage(&mut self) -> Result<Vec<spec::OperationLineageFrameSpec>> {
        let mut frames = Vec::new();
        for frame in self.draft.operation_lineage() {
            self.config_ref(&frame.config)?;
            let input = lower_operation_input_binding(&frame.input)?;
            self.insert_descriptor(spec::DescriptorIdentity::Operation(Box::new(
                operation_descriptor_identity_from_program(frame),
            )))?;
            for output in &frame.output_handles {
                if !self.cell_info.contains_key(output.cell_id().as_str()) {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "operation frame {} returned unknown cell {}",
                            frame.operation_instance_id,
                            output.cell_id()
                        ),
                    ));
                }
            }
            frames.push(spec::OperationLineageFrameSpec {
                operation_instance_id: frame.operation_instance_id.clone(),
                operation_key: stable_author_key(frame.key.as_str())?,
                scope_id: frame.scope_id.clone(),
                operation_descriptor_id: frame.operation_descriptor_id.clone(),
                config_ref_digest: frame.config.config_ref_digest.clone(),
                input_bindings: input.clone(),
                input_binding_digest: input.digest,
                parent_planning_lineage: lower_planning_lineage(&frame.parent_operation_lineage),
                output_cells: frame
                    .output_handles
                    .iter()
                    .map(|handle| handle.cell_id().clone())
                    .collect(),
                lineage_digest: frame.lineage_digest.clone(),
            });
        }
        Ok(frames)
    }

    fn config_ref(&mut self, binding: &program::ConfigBindingSpec) -> Result<spec::ConfigRef> {
        let config_ref = spec::ConfigRef {
            schema_id: binding.schema_id.clone(),
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                *binding.content_digest.digest(),
            ),
            digest: binding.content_digest.clone(),
            byte_len: binding.byte_len as u64,
            media_type: spec::MediaType::new("application/json")
                .map_err(|error| lower(error.to_string()))?,
        };
        self.insert_config_ref(config_ref.clone())?;
        Ok(config_ref)
    }

    fn insert_config_ref(&mut self, config_ref: spec::ConfigRef) -> Result<()> {
        let key = config_ref_key(&config_ref);
        if let Some(existing) = self.config_refs.get(&key) {
            if existing != &config_ref {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!(
                        "conflicting config ref for schema {} digest {}",
                        config_ref.schema_id, config_ref.digest
                    ),
                ));
            }
        } else {
            self.config_refs.insert(key, config_ref);
        }
        Ok(())
    }

    fn insert_descriptor(&mut self, descriptor: spec::DescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id().as_str().to_owned();
        if let Some(existing) = self.descriptor_identities.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting descriptor identity for {key}"),
                ));
            }
        } else {
            self.descriptor_identities.insert(key, descriptor);
        }
        Ok(())
    }

    fn insert_value_lineage(&mut self, lineage: spec::ValueLineage) -> Result<()> {
        let key = lineage.lineage_ref.lineage_digest.as_str().to_owned();
        if let Some(existing) = self.value_lineages.get(&key) {
            if existing != &lineage {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("conflicting value lineage for {key}"),
                ));
            }
        } else {
            self.value_lineages.insert(key, lineage);
        }
        Ok(())
    }

    fn insert_cell(&mut self, cell: spec::CellSpec) -> Result<()> {
        let key = cell.cell_id.as_str().to_owned();
        if self.cell_info.contains_key(&key) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate cell id {}", cell.cell_id),
            ));
        }
        self.cell_info.insert(
            key,
            CellInfo {
                producer: cell.producer.clone(),
                semantic_type_id: cell.semantic_type_id.clone(),
                schema_id: cell.schema_id.clone(),
                value_lineage: cell.value_lineage.clone(),
            },
        );
        self.cells.push(cell);
        Ok(())
    }

    fn predecessors_for_inputs(&self, input_cells: &[CellId]) -> Result<Vec<NodeId>> {
        let mut predecessors = BTreeSet::new();
        for cell_id in input_cells {
            match self
                .cell_info
                .get(cell_id.as_str())
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!("input cell {cell_id} has no producer"),
                    )
                })?
                .producer
                .clone()
            {
                spec::CellProducer::Node(node_id) => {
                    predecessors.insert(node_id);
                }
                spec::CellProducer::Seed(_) => {}
            }
        }
        Ok(predecessors.into_iter().collect())
    }

    fn authoring_provenance(&self) -> Result<spec::AuthoringProvenance> {
        let frame_digests = self
            .draft
            .operation_lineage()
            .iter()
            .map(|frame| frame.lineage_digest.as_str())
            .collect::<Vec<_>>();
        let descriptor = spec::CompositionDescriptor {
            descriptor_id: descriptor_id_json(serde_json::json!({
                "kind": "typed_program_draft",
                "root_scope_id": self.draft.root_scope_id().as_str(),
                "operation_frames": frame_digests,
                "state_nodes": self.draft.state_nodes().len(),
            }))?,
            name: "typed-program-draft".to_owned(),
            version: "mfm.typed.program_draft.v1".to_owned(),
        };
        let config_hash = content_digest_json(serde_json::json!({
            "root_key": self.draft.root_key().as_str(),
            "root_scope_id": self.draft.root_scope_id().as_str(),
        }))?;
        if self.draft.operation_lineage().is_empty() {
            Ok(spec::AuthoringProvenance::StateComposition {
                descriptor,
                config_hash,
            })
        } else {
            Ok(spec::AuthoringProvenance::MixedComposition {
                descriptor,
                config_hash,
            })
        }
    }
}

fn validate_typed_spec(
    spec: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<ValidatedTypedExecutionSpec> {
    let envelope = spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    envelope
        .verify_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    let spec = &envelope.spec;
    spec.spec_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    validate_contract_header(spec)?;
    let scope_ids = validate_scopes(&spec.scopes)?;
    let descriptor_index = DescriptorIndex::new(&spec.descriptor_identities)?;
    validate_descriptor_authority(&descriptor_index, registry)?;
    let config_refs = validate_config_refs(&spec.config_refs)?;
    let lineage_index = validate_value_lineages(&spec.value_lineages, &scope_ids)?;
    let cell_index = validate_cells(&spec.cells, &scope_ids, &lineage_index)?;
    validate_seeds(&spec.seeds, &scope_ids, &cell_index)?;
    let node_index = validate_nodes(
        &spec.nodes,
        &scope_ids,
        &descriptor_index,
        &config_refs,
        &cell_index,
        &lineage_index,
        &spec.public_outputs,
    )?;
    validate_saga_structure(SagaValidationContext {
        typed: spec,
        scope_ids: &scope_ids,
        descriptors: &descriptor_index,
        config_refs: &config_refs,
        cells: &cell_index,
        lineages: &lineage_index,
        forward_nodes: &node_index,
        registry,
    })?;
    validate_operation_lineage(
        &spec.planning_lineage,
        &descriptor_index,
        &config_refs,
        &cell_index,
    )?;
    validate_planning_lineage_authority(spec)?;
    validate_public_outputs(
        &spec.public_outputs,
        &descriptor_index,
        &cell_index,
        &node_index,
    )?;
    validate_lineage_references(&spec.value_lineages, &cell_index, &config_refs)?;
    let graph = CertifiedSpecGraph::from_validated_parts(
        scope_ids,
        &descriptor_index,
        config_refs,
        lineage_index,
        cell_index,
        node_index,
        spec.remediations.clone(),
    );
    Ok(ValidatedTypedExecutionSpec::new(envelope, graph))
}

fn validate_contract_header(spec: &spec::TypedExecutionSpec) -> Result<()> {
    if spec.spec_version.as_str() != spec::SPEC_VERSION {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected spec version {}", spec.spec_version),
        ));
    }
    if spec.media_type.as_str() != spec::MEDIA_TYPE {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected spec media type {}", spec.media_type.as_str()),
        ));
    }
    if spec.canonicalization != DigestAlgorithm::Sha256JcsV1 {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "typed specs must use sha256-jcs-v1",
        ));
    }
    if spec.lowering_version.as_str() != spec::LOWERING_VERSION {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!("unexpected lowering version {}", spec.lowering_version),
        ));
    }
    Ok(())
}

fn validate_scopes(scopes: &[spec::ScopeSpec]) -> Result<BTreeSet<String>> {
    if scopes.is_empty() {
        return Err(problem(
            ProblemClass::InvalidTopology,
            "typed spec must contain at least one scope",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut parents = BTreeMap::new();
    let mut root_count = 0_usize;
    for scope in scopes {
        validate_planning_lineage(&scope.planning_lineage)?;
        if !ids.insert(scope.scope_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate scope id {}", scope.scope_id),
            ));
        }
        if scope.parent_scope_id.is_none() {
            root_count += 1;
        }
        let expected = scope_id_from_spec(
            scope.parent_scope_id.as_ref(),
            scope.stable_key.as_str(),
            &scope.planning_lineage,
        )?;
        if scope.scope_id != expected {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("scope {} is not stable-id derived", scope.scope_id),
            ));
        }
        parents.insert(
            scope.scope_id.as_str().to_owned(),
            scope
                .parent_scope_id
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
        );
    }
    if root_count != 1 {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!("typed spec must contain exactly one root scope, found {root_count}"),
        ));
    }
    for scope in scopes {
        if let Some(parent) = &scope.parent_scope_id {
            if !ids.contains(parent.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "scope {} references unknown parent {parent}",
                        scope.scope_id
                    ),
                ));
            }
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(scope.scope_id.as_str().to_owned());
        while let Some(scope_id) = current {
            if !seen.insert(scope_id.clone()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("scope {} participates in a parent cycle", scope.scope_id),
                ));
            }
            current = parents.get(&scope_id).cloned().flatten();
        }
    }
    Ok(ids)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigIndex {
    keys: BTreeSet<String>,
    ref_digests: BTreeSet<String>,
}

impl ConfigIndex {
    fn contains_ref(&self, config_ref: &spec::ConfigRef) -> bool {
        self.keys.contains(&config_ref_key(config_ref))
    }

    fn contains_ref_digest(&self, digest: &ContentDigest) -> bool {
        self.ref_digests.contains(digest.as_str())
    }
}

fn validate_config_refs(config_refs: &[spec::ConfigRef]) -> Result<ConfigIndex> {
    let mut keys = BTreeSet::new();
    let mut ref_digests = BTreeSet::new();
    for config in config_refs {
        if config.byte_len == 0 {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("config ref {} has zero byte length", config.digest),
            ));
        }
        let expected_artifact =
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *config.digest.digest());
        if config.artifact_id != expected_artifact {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!("config ref {} artifact id mismatch", config.digest),
            ));
        }
        if !keys.insert(config_ref_key(config)) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "duplicate config ref schema {} digest {}",
                    config.schema_id, config.digest
                ),
            ));
        }
        ref_digests.insert(config_ref_digest(config)?.as_str().to_owned());
    }
    Ok(ConfigIndex { keys, ref_digests })
}

#[derive(Debug)]
struct DescriptorIndex<'a> {
    states: BTreeMap<String, &'a spec::StateDescriptorIdentity>,
    operations: BTreeMap<String, &'a spec::OperationDescriptorIdentity>,
    renderers: BTreeMap<String, &'a spec::RendererDescriptorIdentity>,
}

impl<'a> DescriptorIndex<'a> {
    fn new(descriptors: &'a [spec::DescriptorIdentity]) -> Result<Self> {
        let mut index = Self {
            states: BTreeMap::new(),
            operations: BTreeMap::new(),
            renderers: BTreeMap::new(),
        };
        let mut all = BTreeSet::new();
        for descriptor in descriptors {
            let id = descriptor.descriptor_id().as_str().to_owned();
            if !all.insert(id.clone()) {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("duplicate descriptor identity {id}"),
                ));
            }
            match descriptor {
                spec::DescriptorIdentity::State(state) => {
                    index.states.insert(id, state);
                }
                spec::DescriptorIdentity::Operation(operation) => {
                    index.operations.insert(id, operation);
                }
                spec::DescriptorIdentity::Renderer(renderer) => {
                    index.renderers.insert(id, renderer);
                }
            }
        }
        Ok(index)
    }

    fn state(&self, id: &DescriptorId) -> Result<&'a spec::StateDescriptorIdentity> {
        self.states.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("missing state descriptor identity {id}"),
            )
        })
    }

    fn operation(&self, id: &DescriptorId) -> Result<&'a spec::OperationDescriptorIdentity> {
        self.operations.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("missing operation descriptor identity {id}"),
            )
        })
    }

    fn renderer(&self, id: &DescriptorId) -> Result<&'a spec::RendererDescriptorIdentity> {
        self.renderers.get(id.as_str()).copied().ok_or_else(|| {
            problem(
                ProblemClass::InvalidTerminalShape,
                format!("missing renderer descriptor identity {id}"),
            )
        })
    }
}

fn validate_descriptor_authority(
    descriptors: &DescriptorIndex<'_>,
    registry: &CertificationRegistry,
) -> Result<()> {
    for descriptor in descriptors.states.values() {
        validate_state_descriptor_identity(descriptor)?;
        if validate_builtin_framework_state_descriptor(descriptor)? {
            continue;
        }
        let Some(registered) = registry.states.get(descriptor.descriptor_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "state descriptor {} is not registered",
                    descriptor.descriptor_id
                ),
            ));
        };
        if registered != *descriptor {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "state descriptor {} does not match registry authority",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    for descriptor in descriptors.operations.values() {
        validate_operation_descriptor_identity(descriptor)?;
        let Some(registered) = registry.operations.get(descriptor.descriptor_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation descriptor {} is not registered",
                    descriptor.descriptor_id
                ),
            ));
        };
        if registered != *descriptor {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation descriptor {} does not match registry authority",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    for descriptor in descriptors.renderers.values() {
        let expected = descriptor
            .expected_descriptor_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        if descriptor.descriptor_id != expected {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "renderer descriptor {} is not content addressed",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_state_descriptor_identity(descriptor: &spec::StateDescriptorIdentity) -> Result<()> {
    let effect = effect_descriptor_for_kind(&descriptor.effect_kind)?;
    if descriptor.effect_class != effect.class.as_str()
        || descriptor.effect_name != effect.name
        || descriptor.effect_version != effect.version
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} effect metadata mismatch",
                descriptor.descriptor_id
            ),
        ));
    }
    let expected_runner = match effect.class {
        EffectClass::Pure => "pure",
        EffectClass::ReadExternal => "read_external",
        EffectClass::ManagedPlatformWrite => "managed_platform_write",
        EffectClass::ApplySideEffect => "apply_side_effect",
    };
    if descriptor.runner != expected_runner {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} runner mismatch",
                descriptor.descriptor_id
            ),
        ));
    }
    match (effect.class, &descriptor.side_effect_contract_digest) {
        (EffectClass::ApplySideEffect, Some(_)) => {}
        (EffectClass::ApplySideEffect, None) => {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "side-effect state descriptor {} is missing contract digest",
                    descriptor.descriptor_id
                ),
            ));
        }
        (_, None) => {}
        (_, Some(_)) => {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "non-side-effect state descriptor {} carries contract digest",
                    descriptor.descriptor_id
                ),
            ));
        }
    }
    let expected = state_descriptor_id_from_spec(descriptor)?;
    if descriptor.descriptor_id != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "state descriptor {} is not content addressed",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(())
}

fn validate_operation_descriptor_identity(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<()> {
    let expected = operation_descriptor_id_from_spec(descriptor)?;
    if descriptor.descriptor_id != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "operation descriptor {} is not content addressed",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(())
}

fn validate_builtin_framework_state_descriptor(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<bool> {
    let expected = match descriptor.name.as_str() {
        "mfm.framework.bridge_same_value" => Some(framework_bridge_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        "mfm.framework.render_public_outputs" => Some(framework_render_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        "mfm.framework.project_retention_manifest" => {
            Some(framework_project_retention_manifest_descriptor(
                &descriptor.output_schema_id,
                &descriptor.output_semantic_type_id,
                &descriptor.config_schema_id,
                &descriptor.input_schema_id,
            )?)
        }
        "mfm.framework.complete_run" => Some(framework_complete_run_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        "mfm.framework.resolve_saga_terminal" => Some(framework_resolve_saga_terminal_descriptor(
            &descriptor.output_schema_id,
            &descriptor.output_semantic_type_id,
            &descriptor.config_schema_id,
            &descriptor.input_schema_id,
        )?),
        _ => None,
    };
    let Some(expected) = expected else {
        return Ok(false);
    };
    if *descriptor != expected {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "framework state descriptor {} does not match built-in authority",
                descriptor.descriptor_id
            ),
        ));
    }
    Ok(true)
}

fn validate_value_lineages(
    lineages: &[spec::ValueLineage],
    scope_ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, spec::ValueLineage>> {
    let mut index = BTreeMap::new();
    for lineage in lineages {
        if !scope_ids.contains(lineage.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("lineage references unknown scope {}", lineage.scope_id),
            ));
        }
        validate_planning_lineage(&lineage.planning_lineage)?;
        validate_domain_keys(&lineage.domain_keys)?;
        let expected = value_lineage_digest(lineage)?;
        if lineage.lineage_ref.lineage_digest != expected {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "value lineage {} is not content addressed",
                    lineage.lineage_ref.lineage_digest
                ),
            ));
        }
        let key = lineage.lineage_ref.lineage_digest.as_str().to_owned();
        if index.insert(key.clone(), lineage.clone()).is_some() {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("duplicate value lineage {key}"),
            ));
        }
    }
    Ok(index)
}

fn validate_cells(
    cells: &[spec::CellSpec],
    scope_ids: &BTreeSet<String>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
) -> Result<BTreeMap<String, spec::CellSpec>> {
    let mut index = BTreeMap::new();
    for cell in cells {
        if !scope_ids.contains(cell.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "cell {} references unknown scope {}",
                    cell.cell_id, cell.scope_id
                ),
            ));
        }
        let lineage = lineages
            .get(cell.value_lineage.lineage_digest.as_str())
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("cell {} references missing lineage", cell.cell_id),
                )
            })?;
        if lineage.scope_id != cell.scope_id || lineage.producer != cell.producer {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "cell {} lineage does not match producer/scope",
                    cell.cell_id
                ),
            ));
        }
        let expected_cell_id = cell_id_from_parts(
            &cell.scope_id,
            &cell.producer,
            &cell.semantic_type_id,
            &cell.schema_id,
        )?;
        if cell.cell_id != expected_cell_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("cell {} is not stable-id derived", cell.cell_id),
            ));
        }
        if index
            .insert(cell.cell_id.as_str().to_owned(), cell.clone())
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate cell id {}", cell.cell_id),
            ));
        }
    }
    Ok(index)
}

fn validate_seeds(
    seeds: &[spec::SeedSpec],
    scope_ids: &BTreeSet<String>,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<()> {
    let mut seed_ids = BTreeSet::new();
    for seed in seeds {
        if !seed_ids.insert(seed.seed_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate seed id {}", seed.seed_id),
            ));
        }
        let expected_seed_id = seed_id_from_spec(seed)?;
        if seed.seed_id != expected_seed_id {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("seed {} is not stable-id derived", seed.seed_id),
            ));
        }
        if !scope_ids.contains(seed.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "seed {} references unknown scope {}",
                    seed.seed_id, seed.scope_id
                ),
            ));
        }
        let cell = cells.get(seed.cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!(
                    "seed {} references missing cell {}",
                    seed.seed_id, seed.cell_id
                ),
            )
        })?;
        if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
            || cell.scope_id != seed.scope_id
            || cell.schema_id != seed.schema_id
            || cell.semantic_type_id != seed.semantic_type_id
        {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!("seed {} cell metadata mismatch", seed.seed_id),
            ));
        }
    }
    Ok(())
}

enum StateNodeIdDerivation {
    FrameworkAware,
    StateOnly,
}

struct StateNodeContractInput<'a> {
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    config_ref_digest: &'a ContentDigest,
    cells: &'a BTreeMap<String, spec::CellSpec>,
    lineages: &'a BTreeMap<String, spec::ValueLineage>,
    label: &'static str,
    id_derivation: StateNodeIdDerivation,
    enforce_framework_lineage_inputs: bool,
}

mod node_contract;

fn validate_nodes(
    nodes: &[spec::NodeSpec],
    scope_ids: &BTreeSet<String>,
    descriptors: &DescriptorIndex<'_>,
    config_refs: &ConfigIndex,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
    public_outputs: &spec::PublicOutputSpec,
) -> Result<BTreeMap<String, spec::NodeSpec>> {
    let mut index = BTreeMap::new();
    for node in nodes {
        if !scope_ids.contains(node.scope_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "node {} references unknown scope {}",
                    node.node_id, node.scope_id
                ),
            ));
        }
        if !config_refs.contains_ref(&node.config_ref) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "node {} references missing config {}",
                    node.node_id, node.config_ref.digest
                ),
            ));
        }
        let descriptor = descriptors.state(&node.descriptor_id)?;
        let config_ref_digest = config_ref_digest(&node.config_ref)?;
        framework_lifecycle::validate_framework_descriptor_variant(node, descriptor)?;
        if let Some(framework) = &node.framework {
            framework_lifecycle::validate_framework_config_ref(node, framework.config_kind())?;
        }
        node_contract::validate_state_node_contract(StateNodeContractInput {
            node,
            descriptor,
            config_ref_digest: &config_ref_digest,
            cells,
            lineages,
            label: "node",
            id_derivation: StateNodeIdDerivation::FrameworkAware,
            enforce_framework_lineage_inputs: true,
        })?;
        if index
            .insert(node.node_id.as_str().to_owned(), node.clone())
            .is_some()
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate node id {}", node.node_id),
            ));
        }
    }
    framework_lifecycle::validate_framework_nodes(nodes, cells, descriptors, public_outputs)?;
    validate_node_graph_acyclic(&index)?;
    Ok(index)
}

struct SagaValidationContext<'a, 'd> {
    typed: &'a spec::TypedExecutionSpec,
    scope_ids: &'a BTreeSet<String>,
    descriptors: &'a DescriptorIndex<'d>,
    config_refs: &'a ConfigIndex,
    cells: &'a BTreeMap<String, spec::CellSpec>,
    lineages: &'a BTreeMap<String, spec::ValueLineage>,
    forward_nodes: &'a BTreeMap<String, spec::NodeSpec>,
    registry: &'a CertificationRegistry,
}

fn validate_saga_structure(context: SagaValidationContext<'_, '_>) -> Result<()> {
    let forward_side_effects = context
        .forward_nodes
        .values()
        .filter(|node| is_apply_side_effect_node(node).unwrap_or(false))
        .collect::<Vec<_>>();

    match &context.typed.saga {
        spec::SagaPolicySpec::NoSideEffects => {
            if !forward_side_effects.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    "NoSideEffects policy cannot certify forward side-effect nodes",
                ));
            }
        }
        spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::ManualResolution { .. }
        | spec::SagaPolicySpec::CompensateCompleted { .. } => {
            if forward_side_effects.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    "side-effecting saga policy requires at least one forward side-effect node",
                ));
            }
        }
    }

    if !matches!(
        context.typed.saga,
        spec::SagaPolicySpec::CompensateCompleted { .. }
    ) && !context.typed.remediations.is_empty()
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            "remediations are valid only under CompensateCompleted policy",
        ));
    }

    let forward_side_effect_ids = forward_side_effects
        .iter()
        .map(|node| node.node_id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let mut remediation_node_ids = BTreeSet::new();
    for (forward_node_id, remediation) in &context.typed.remediations {
        if context
            .forward_nodes
            .contains_key(remediation.node_id.as_str())
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "remediation node {} also appears in the forward node collection",
                    remediation.node_id
                ),
            ));
        }
        if !remediation_node_ids.insert(remediation.node_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate remediation node id {}", remediation.node_id),
            ));
        }
        let Some(forward) = context.forward_nodes.get(forward_node_id.as_str()) else {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("remediation key {forward_node_id} references missing forward node"),
            ));
        };
        if !forward_side_effect_ids.contains(forward_node_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation key {forward_node_id} does not reference a forward side-effect node"
                ),
            ));
        }
        validate_remediation_node_contract(
            remediation,
            context.scope_ids,
            context.descriptors,
            context.config_refs,
            context.cells,
            context.lineages,
        )?;
        validate_remediation_binding_scope(
            forward,
            remediation,
            context.cells,
            context.forward_nodes,
        )?;
    }

    if matches!(
        context.typed.saga,
        spec::SagaPolicySpec::CompensateCompleted { .. }
    ) {
        for node in forward_side_effects {
            if !context.typed.remediations.contains_key(&node.node_id) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "compensating policy left forward side-effect node {} without remediation",
                        node.node_id
                    ),
                ));
            }
        }
    }

    validate_manual_resolution_authority(context.typed, context.registry)?;

    Ok(())
}

fn validate_manual_resolution_authority(
    typed: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<()> {
    for manual in manual_resolution_specs(typed) {
        validate_manual_evidence_schema(&manual.evidence_schema, registry)?;
        validate_manual_authorization_spec(&manual.authorization, registry)?;
    }
    Ok(())
}

fn validate_manual_evidence_schema(
    schema_id: &SchemaId,
    registry: &CertificationRegistry,
) -> Result<()> {
    let Some(roles) = registry.schema_roles.get(schema_id.as_str()) else {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("unknown manual evidence schema {schema_id}"),
        ));
    };
    if !roles.contains(&CertifiedSchemaRole::ManualResolutionEvidence) {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("manual evidence schema {schema_id} has wrong certified role"),
        ));
    }
    Ok(())
}

fn validate_manual_authorization_spec(
    authorization: &spec::ManualResolutionAuthorizationSpec,
    registry: &CertificationRegistry,
) -> Result<()> {
    if authorization.signing_scheme.as_str() != MANUAL_RESOLUTION_SIGNING_SCHEME {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "unsupported manual authorization signing scheme {}",
                authorization.signing_scheme
            ),
        ));
    }
    if !registry
        .manual_authorization_verifiers
        .contains(authorization.verifier_id.as_str())
    {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "unknown manual authorization verifier {}",
                authorization.verifier_id
            ),
        ));
    }
    validate_operator_authority_snapshot(&authorization.authority)?;
    let required_signatures = authorization.quorum.required_signatures() as usize;
    if required_signatures > authorization.authority.operators.len() {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "manual authorization quorum {} exceeds operator authority size {}",
                authorization.quorum.required_signatures(),
                authorization.authority.operators.len()
            ),
        ));
    }
    let authority_id = &authorization.authority.authority_id;
    let Some(registered) = registry
        .operator_authority_snapshots
        .get(authority_id.as_str())
    else {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("missing operator authority snapshot {authority_id}"),
        ));
    };
    if registered != &authorization.authority {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("operator authority snapshot {authority_id} does not match registry"),
        ));
    }
    Ok(())
}

fn validate_operator_authority_snapshot(
    authority: &spec::OperatorAuthoritySnapshotSpec,
) -> Result<()> {
    if authority.operators.is_empty() {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "operator authority snapshot {} has no operators",
                authority.authority_id
            ),
        ));
    }
    let mut operator_ids = BTreeSet::new();
    let mut public_identities = BTreeSet::new();
    for operator in &authority.operators {
        if !operator_ids.insert(operator.operator_id.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operator authority snapshot {} contains duplicate operator {}",
                    authority.authority_id, operator.operator_id
                ),
            ));
        }
        if !public_identities.insert(operator.public_identity.as_str()) {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operator authority snapshot {} contains duplicate public identity {}",
                    authority.authority_id, operator.public_identity
                ),
            ));
        }
    }
    Ok(())
}

fn validate_remediation_node_contract(
    node: &spec::NodeSpec,
    scope_ids: &BTreeSet<String>,
    descriptors: &DescriptorIndex<'_>,
    config_refs: &ConfigIndex,
    cells: &BTreeMap<String, spec::CellSpec>,
    lineages: &BTreeMap<String, spec::ValueLineage>,
) -> Result<()> {
    if node.framework.is_some() {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "remediation node {} must not be a framework node",
                node.node_id
            ),
        ));
    }
    if !scope_ids.contains(node.scope_id.as_str()) {
        return Err(problem(
            ProblemClass::InvalidTopology,
            format!(
                "remediation node {} references unknown scope {}",
                node.node_id, node.scope_id
            ),
        ));
    }
    if !config_refs.contains_ref(&node.config_ref) {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "remediation node {} references missing config {}",
                node.node_id, node.config_ref.digest
            ),
        ));
    }
    let descriptor = descriptors.state(&node.descriptor_id)?;
    let config_ref_digest = config_ref_digest(&node.config_ref)?;
    if !is_apply_side_effect_node(node)? {
        return Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!("remediation node {} is not side-effect-grade", node.node_id),
        ));
    }
    node_contract::validate_state_node_contract(StateNodeContractInput {
        node,
        descriptor,
        config_ref_digest: &config_ref_digest,
        cells,
        lineages,
        label: "remediation node",
        id_derivation: StateNodeIdDerivation::StateOnly,
        enforce_framework_lineage_inputs: false,
    })?;
    Ok(())
}

fn validate_remediation_binding_scope(
    forward: &spec::NodeSpec,
    remediation: &spec::NodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    forward_nodes: &BTreeMap<String, spec::NodeSpec>,
) -> Result<()> {
    let mut allowed = BTreeSet::new();
    allowed.insert(forward.output_cell.clone());
    for input_cell in collect_input_cells(&forward.input_bindings.root) {
        collect_forward_ancestor_cells(input_cell, cells, forward_nodes, &mut allowed)?;
    }

    for input_cell in collect_input_cells(&remediation.input_bindings.root) {
        if !allowed.contains(&input_cell) {
            return Err(problem(
                ProblemClass::InvalidInterfaceWiring,
                format!(
                    "remediation node {} references cell {} outside linked forward node {} scope",
                    remediation.node_id, input_cell, forward.node_id
                ),
            ));
        }
    }
    Ok(())
}

fn collect_forward_ancestor_cells(
    cell_id: CellId,
    cells: &BTreeMap<String, spec::CellSpec>,
    forward_nodes: &BTreeMap<String, spec::NodeSpec>,
    allowed: &mut BTreeSet<CellId>,
) -> Result<()> {
    if !allowed.insert(cell_id.clone()) {
        return Ok(());
    }
    let cell = cells.get(cell_id.as_str()).ok_or_else(|| {
        problem(
            ProblemClass::InvalidTopology,
            format!("ancestor cell {cell_id} is missing"),
        )
    })?;
    if let spec::CellProducer::Node(node_id) = &cell.producer {
        let Some(node) = forward_nodes.get(node_id.as_str()) else {
            return Ok(());
        };
        for input in collect_input_cells(&node.input_bindings.root) {
            collect_forward_ancestor_cells(input, cells, forward_nodes, allowed)?;
        }
    }
    Ok(())
}

fn is_apply_side_effect_node(node: &spec::NodeSpec) -> Result<bool> {
    let (class, _) = effect_class_for_kind(&node.effect_kind)?;
    Ok(class == EffectClass::ApplySideEffect)
}

fn validate_operation_lineage(
    frames: &[spec::OperationLineageFrameSpec],
    descriptors: &DescriptorIndex<'_>,
    config_refs: &ConfigIndex,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for frame in frames {
        if !ids.insert(frame.operation_instance_id.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "duplicate operation instance {}",
                    frame.operation_instance_id
                ),
            ));
        }
        validate_planning_lineage(&frame.parent_planning_lineage)?;
        let descriptor = descriptors.operation(&frame.operation_descriptor_id)?;
        if descriptor.input_schema_id != frame.input_bindings.input_schema_id {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "operation frame {} input schema does not match descriptor",
                    frame.operation_instance_id
                ),
            ));
        }
        if !config_refs.contains_ref_digest(&frame.config_ref_digest) {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "operation frame {} references missing config {}",
                    frame.operation_instance_id, frame.config_ref_digest
                ),
            ));
        }
        validate_input_binding(&frame.input_bindings, cells)?;
        if frame.input_binding_digest != frame.input_bindings.digest {
            return Err(problem(
                ProblemClass::InvalidDataShape,
                format!(
                    "operation frame {} input binding digest mismatch",
                    frame.operation_instance_id
                ),
            ));
        }
        let expected_instance = operation_instance_id_from_spec(frame, descriptor)?;
        if frame.operation_instance_id != expected_instance {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "operation instance {} is not stable-id derived",
                    frame.operation_instance_id
                ),
            ));
        }
        for cell_id in &frame.output_cells {
            if !cells.contains_key(cell_id.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "operation frame {} returned missing cell {}",
                        frame.operation_instance_id, cell_id
                    ),
                ));
            }
        }
        let expected_lineage = operation_lineage_frame_digest_from_spec(frame, descriptor, cells)?;
        if frame.lineage_digest != expected_lineage {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!(
                    "operation frame {} lineage is not content addressed",
                    frame.operation_instance_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_planning_lineage_authority(spec: &spec::TypedExecutionSpec) -> Result<()> {
    let frame_instances = spec
        .planning_lineage
        .iter()
        .map(|frame| frame.operation_instance_id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let frame_digests = spec
        .planning_lineage
        .iter()
        .map(|frame| frame.lineage_digest.as_str().to_owned())
        .collect::<BTreeSet<_>>();

    for scope in &spec.scopes {
        validate_planning_lineage_refs(&scope.planning_lineage, &frame_instances, &frame_digests)?;
    }
    for node in &spec.nodes {
        validate_planning_lineage_refs(&node.planning_lineage, &frame_instances, &frame_digests)?;
    }
    for node in spec.remediations.values() {
        validate_planning_lineage_refs(&node.planning_lineage, &frame_instances, &frame_digests)?;
    }
    for lineage in &spec.value_lineages {
        validate_planning_lineage_refs(
            &lineage.planning_lineage,
            &frame_instances,
            &frame_digests,
        )?;
    }
    for frame in &spec.planning_lineage {
        validate_planning_lineage_refs(
            &frame.parent_planning_lineage,
            &frame_instances,
            &frame_digests,
        )?;
    }
    Ok(())
}

fn validate_planning_lineage_refs(
    lineage: &spec::PlanningLineage,
    frame_instances: &BTreeSet<String>,
    frame_digests: &BTreeSet<String>,
) -> Result<()> {
    validate_planning_lineage(lineage)?;
    for instance in &lineage.active_operation_instances {
        if !frame_instances.contains(instance.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("planning lineage references missing operation instance {instance}"),
            ));
        }
    }
    for digest in &lineage.completed_operation_frames {
        if !frame_digests.contains(digest.as_str()) {
            return Err(problem(
                ProblemClass::InvalidDataMeaning,
                format!("planning lineage references missing operation frame {digest}"),
            ));
        }
    }
    Ok(())
}

fn validate_public_outputs(
    public_outputs: &spec::PublicOutputSpec,
    descriptors: &DescriptorIndex<'_>,
    cells: &BTreeMap<String, spec::CellSpec>,
    nodes: &BTreeMap<String, spec::NodeSpec>,
) -> Result<()> {
    if public_outputs.outputs.is_empty() {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            "public output spec must declare at least one output",
        ));
    }
    descriptors.renderer(&public_outputs.renderer_descriptor.descriptor_id)?;
    let mut fields = BTreeSet::new();
    for output in &public_outputs.outputs {
        if !fields.insert(output.public_field_path.as_str().to_owned()) {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "duplicate public output field {}",
                    output.public_field_path.as_str()
                ),
            ));
        }
        let cell = cells.get(output.cell_id.as_str()).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTerminalShape,
                format!("public output cell {} is missing", output.cell_id),
            )
        })?;
        if cell.producer != output.producer
            || cell.scope_id != output.scope_id
            || cell.semantic_type_id != output.semantic_type_id
            || cell.schema_id != output.schema_id
            || cell.value_lineage != output.value_lineage
        {
            return Err(problem(
                ProblemClass::InvalidTerminalShape,
                format!(
                    "public output {} does not match certified cell",
                    output.public_field_path.as_str()
                ),
            ));
        }
    }
    let output_digest = public_outputs
        .digest()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    let render_nodes = nodes.values().filter_map(|node| match &node.framework {
        Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Some((node, render)),
        _ => None,
    });
    let mut matching = 0_usize;
    for (node, render) in render_nodes {
        if render.public_schema_id == public_outputs.public_schema_id
            && render.output_spec_digest == output_digest
            && render.renderer_descriptor == public_outputs.renderer_descriptor
            && render.required_cells == public_outputs.outputs
        {
            matching += 1;
            let expected_predecessors = predecessor_nodes(
                &render
                    .required_cells
                    .iter()
                    .map(|cell| cell.cell_id.clone())
                    .collect::<Vec<_>>(),
                cells,
            )?;
            if node.deterministic_predecessors != expected_predecessors {
                return Err(problem(
                    ProblemClass::InvalidTerminalShape,
                    format!(
                        "render node {} predecessors do not match public outputs",
                        node.node_id
                    ),
                ));
            }
        }
    }
    if matching != 1 {
        return Err(problem(
            ProblemClass::InvalidTerminalShape,
            format!("expected exactly one public output render node, found {matching}"),
        ));
    }
    Ok(())
}

fn validate_lineage_references(
    lineages: &[spec::ValueLineage],
    cells: &BTreeMap<String, spec::CellSpec>,
    config_refs: &ConfigIndex,
) -> Result<()> {
    for lineage in lineages {
        if let Some(config_ref_digest) = &lineage.config_ref_digest {
            if !config_refs.contains_ref_digest(config_ref_digest) {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("lineage references missing config {config_ref_digest}"),
                ));
            }
        }
        for input in &lineage.input_cells {
            if !cells.contains_key(input.as_str()) {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("lineage references missing input cell {input}"),
                ));
            }
        }
    }
    Ok(())
}

mod framework_lifecycle;

struct ExpectedNodeLineage {
    input_cells: Vec<CellId>,
    config_ref_digest: Option<ContentDigest>,
    transform_policy: spec::LineageTransformPolicy,
}

fn expected_node_lineage(
    node: &spec::NodeSpec,
    input_cells: &[CellId],
    config_ref_digest: &ContentDigest,
) -> Result<ExpectedNodeLineage> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::Bridge(bridge)) => Ok(ExpectedNodeLineage {
            input_cells: vec![bridge.source_cell_id.clone()],
            config_ref_digest: None,
            transform_policy: spec::LineageTransformPolicy::SameValueBridge,
        }),
        Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Ok(ExpectedNodeLineage {
            input_cells: render
                .required_cells
                .iter()
                .map(|cell| cell.cell_id.clone())
                .collect(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
            Ok(ExpectedNodeLineage {
                input_cells: vec![retention.public_output_receipt_cell.clone()],
                config_ref_digest: Some(config_ref_digest.clone()),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            })
        }
        Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => Ok(ExpectedNodeLineage {
            input_cells: vec![complete.retention_manifest_receipt_cell.clone()],
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) => Ok(ExpectedNodeLineage {
            input_cells: Vec::new(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
        None => Ok(ExpectedNodeLineage {
            input_cells: input_cells.to_vec(),
            config_ref_digest: Some(config_ref_digest.clone()),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        }),
    }
}

fn validate_node_graph_acyclic(nodes: &BTreeMap<String, spec::NodeSpec>) -> Result<()> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }

    fn visit(
        node_id: &str,
        nodes: &BTreeMap<String, spec::NodeSpec>,
        marks: &mut BTreeMap<String, Mark>,
    ) -> Result<()> {
        match marks.get(node_id) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("node graph contains a cycle at {node_id}"),
                ));
            }
            None => {}
        }
        let node = nodes.get(node_id).ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("node graph references missing predecessor {node_id}"),
            )
        })?;
        marks.insert(node_id.to_owned(), Mark::Visiting);
        for predecessor in &node.deterministic_predecessors {
            visit(predecessor.as_str(), nodes, marks)?;
        }
        marks.insert(node_id.to_owned(), Mark::Done);
        Ok(())
    }

    let mut marks = BTreeMap::new();
    for node_id in nodes.keys() {
        visit(node_id, nodes, &mut marks)?;
    }
    Ok(())
}

fn validate_effect_capabilities(
    effect_kind: &EffectKind,
    capabilities: &CapabilitySetDescriptor,
) -> Result<()> {
    let (class, name) = effect_class_for_kind(effect_kind)?;
    capabilities
        .validate_for_effect_class(class, name)
        .map_err(|error| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("invalid capability set for effect {effect_kind}: {error}"),
            )
        })
}

fn validate_side_effect_contract(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<()> {
    let (class, _) = effect_class_for_kind(&node.effect_kind)?;
    match (class, &node.side_effect) {
        (EffectClass::ApplySideEffect, Some(contract))
            if descriptor.side_effect_contract_digest.as_ref()
                == Some(&contract.contract_digest) =>
        {
            Ok(())
        }
        (EffectClass::ApplySideEffect, Some(_)) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "side-effect node {} contract digest does not match descriptor",
                node.node_id
            ),
        )),
        (EffectClass::ApplySideEffect, None) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "side-effect node {} is missing side-effect contract",
                node.node_id
            ),
        )),
        (_, Some(_)) => Err(problem(
            ProblemClass::InvalidSemanticTransition,
            format!(
                "non-side-effect node {} carries side-effect contract",
                node.node_id
            ),
        )),
        _ => Ok(()),
    }
}

fn validate_input_binding(
    binding: &spec::InputBindingSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
) -> Result<Vec<CellId>> {
    let expected_digest = content_digest_json(input_node_json(&binding.root))?;
    if binding.digest != expected_digest {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            format!(
                "input binding {} is not content addressed",
                binding.input_descriptor_id
            ),
        ));
    }
    let mut input_cells = Vec::new();
    validate_input_node(&binding.root, cells, &mut input_cells)?;
    Ok(input_cells)
}

fn validate_input_node(
    node: &spec::InputBindingNodeSpec,
    cells: &BTreeMap<String, spec::CellSpec>,
    input_cells: &mut Vec<CellId>,
) -> Result<()> {
    match node {
        spec::InputBindingNodeSpec::Unit => Ok(()),
        spec::InputBindingNodeSpec::Cell(cell) => {
            let produced = cells.get(cell.cell_id.as_str()).ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("input binding references missing cell {}", cell.cell_id),
                )
            })?;
            if produced.semantic_type_id != cell.semantic_type_id
                || produced.schema_id != cell.schema_id
                || produced.value_lineage != cell.value_lineage
            {
                return Err(problem(
                    ProblemClass::InvalidInterfaceWiring,
                    format!(
                        "input binding for cell {} does not match cell metadata",
                        cell.cell_id
                    ),
                ));
            }
            input_cells.push(cell.cell_id.clone());
            Ok(())
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            let mut seen = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.field_path.as_str().to_owned()) {
                    return Err(problem(
                        ProblemClass::InvalidInterfaceWiring,
                        format!("duplicate input field {}", field.field_path.as_str()),
                    ));
                }
                validate_input_node(&field.node, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => {
            validate_collection_ordering(false, elements, *ordering, domain_keys)?;
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => {
            validate_collection_ordering(true, elements, *ordering, domain_keys)?;
            for element in elements {
                validate_input_node(element, cells, input_cells)?;
            }
            Ok(())
        }
    }
}

fn validate_collection_ordering(
    non_empty: bool,
    elements: &[spec::InputBindingNodeSpec],
    ordering: spec::OrderingEvidence,
    domain_keys: &[spec::StableDomainKeyRef],
) -> Result<()> {
    if non_empty && elements.is_empty() {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "non-empty collection binding contains no elements",
        ));
    }
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => {
            if !domain_keys.is_empty() {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    "explicit-order vectors must not carry domain keys",
                ));
            }
        }
        spec::OrderingEvidence::StableDomainKey => {
            if domain_keys.len() != elements.len() {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    "domain-key ordered vectors must carry one key per element",
                ));
            }
            validate_domain_keys(domain_keys)?;
        }
    }
    Ok(())
}

fn validate_domain_keys(domain_keys: &[spec::StableDomainKeyRef]) -> Result<()> {
    let mut sorted = domain_keys.to_vec();
    sorted.sort();
    if sorted != domain_keys {
        return Err(problem(
            ProblemClass::InvalidDataShape,
            "stable domain keys are not canonical sorted",
        ));
    }
    let mut seen = BTreeSet::new();
    for key in domain_keys {
        let rendered = format!("{}:{}", key.schema_id, key.content_digest);
        if !seen.insert(rendered) {
            return Err(problem(
                ProblemClass::InvalidTopology,
                "duplicate stable domain key in collection",
            ));
        }
    }
    Ok(())
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
        config_schema_id: node.config.schema_id.clone(),
        input_schema_id: node.input.input_schema_id.clone(),
        output_schema_id: node.output_schema_id.clone(),
        output_semantic_type_id: node.output_semantic_type_id.clone(),
        effect_kind: node.effect_kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities: node.capability_bindings.clone(),
        runner: runner_kind_name(node.runner).to_owned(),
        side_effect_contract_digest: node.side_effect_contract_digest.clone(),
    })
}

fn state_descriptor_identity_from_registered(
    descriptor: &program::StateDescriptorIdentity,
    runner: program::RunnerKind,
) -> Result<spec::StateDescriptorIdentity> {
    let effect = descriptor.effect();
    Ok(spec::StateDescriptorIdentity {
        descriptor_id: descriptor.descriptor_id().clone(),
        name: descriptor.name().to_owned(),
        state_kind: descriptor.kind().clone(),
        state_version: descriptor.version().clone(),
        config_schema_id: descriptor.config_schema_id().clone(),
        input_schema_id: descriptor.input_schema_id().clone(),
        output_schema_id: descriptor.output_schema_id().clone(),
        output_semantic_type_id: descriptor.output_semantic_type_id().clone(),
        effect_kind: effect.kind.clone(),
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version.clone(),
        capabilities: descriptor.capabilities().clone(),
        runner: runner_kind_name(runner).to_owned(),
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

fn operation_descriptor_identity_from_registered(
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
        "effect": {
            "class": descriptor.effect_class.as_str(),
            "kind": descriptor.effect_kind.as_str(),
            "name": descriptor.effect_name.as_str(),
            "version": descriptor.effect_version.as_str(),
        },
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.state_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "output_semantic_type_id": descriptor.output_semantic_type_id.as_str(),
        "runner": descriptor.runner.as_str(),
        "side_effect_contract_digest": descriptor.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
        "version": descriptor.state_version.as_str(),
    }))
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
        effect_kind: Pure::descriptor()
            .map_err(|error| lower(error.to_string()))?
            .kind,
        runner: "pure",
        capabilities: NoCaps::descriptor().map_err(|error| lower(error.to_string()))?,
    })
}

fn framework_render_descriptor(
    output_schema_id: &SchemaId,
    output_semantic_type_id: &SemanticTypeId,
    config_schema_id: &SchemaId,
    input_schema_id: &SchemaId,
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
        "effect": {
            "class": effect.class.as_str(),
            "kind": effect.kind.as_str(),
            "name": effect.name,
            "version": effect.version.as_str(),
        },
        "input_schema_id": input_schema_id.as_str(),
        "kind": state_kind.as_str(),
        "name": name,
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
        config_schema_id: config_schema_id.clone(),
        input_schema_id: input_schema_id.clone(),
        output_schema_id: output_schema_id.clone(),
        output_semantic_type_id: output_semantic_type_id.clone(),
        effect_kind,
        effect_class: effect.class.as_str().to_owned(),
        effect_name: effect.name.to_owned(),
        effect_version: effect.version,
        capabilities,
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
mod tests {
    use super::*;
    use mfm_capabilities::{CapabilitySpec, ExternalMutationAuthorityRole, NoCaps};
    use mfm_ids::{
        AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, OperationKind,
        OperationVersion,
    };
    use mfm_program::{
        build_root_with_registries, CanonicalSeed, IdempotencyKey, Operation, OperationKey,
        OperationRegistryBuilder, PublicOutputKey, PureState, ResourceClaim, RootBuilder, ScopeKey,
        SideEffectState, StateKey, StateRegistryBuilder, StateResult, StateSpec,
    };
    use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs};
    use serde::{Deserialize, Serialize};

    fn certify_raw_typed_spec(
        typed: spec::TypedExecutionSpec,
        registry: &CertificationRegistry,
    ) -> Result<CertifiedTypedSpec> {
        certify_typed_spec(spec::UntrustedTypedSpec::from_raw_spec(typed), registry)
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
    #[mfm(
        namespace = "mfm.certify.test",
        name = "value",
        version = "1",
        schema = "mfm.certify.test.value"
    )]
    struct TestValue {
        amount: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct TestConfig {
        multiplier: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct OperationConfig {
        multiplier: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct DefaultedConfig {
        #[serde(default)]
        multiplier: Option<u64>,
    }

    #[derive(PublicOutputs)]
    #[mfm(schema = "mfm.certify.test.public_outputs")]
    struct TestPublicOutputs<'p, 's> {
        result: mfm_program::Handle<'p, 's, TestValue>,
    }

    #[derive(OperationOutput)]
    #[mfm(schema = "mfm.certify.test.operation_outputs")]
    struct TestOperationOutputs<'p, 's> {
        result: mfm_program::Handle<'p, 's, TestValue>,
    }

    struct MultiplyState {
        config: TestConfig,
    }

    impl StateSpec for MultiplyState {
        type Config = TestConfig;
        type Input = TestValue;
        type Output = TestValue;
        type Effect = Pure;
        type Caps = NoCaps;

        fn kind() -> program::Result<StateKind> {
            StateKind::new(
                "mfm.certify.test",
                "multiply",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x11),
            )
            .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn version() -> program::Result<StateVersion> {
            StateVersion::new("mfm.certify.test.multiply.v1")
                .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.multiply"
        }

        fn new(config: program::ValidatedConfig<Self::Config>) -> program::Result<Self> {
            Ok(Self {
                config: config.into_inner(),
            })
        }
    }

    impl PureState for MultiplyState {
        fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
            Ok(TestValue {
                amount: input.amount * self.config.multiplier,
            })
        }
    }

    struct MutationCap;

    impl CapabilitySpec for MutationCap {
        type Role = ExternalMutationAuthorityRole;

        fn kind() -> mfm_capabilities::Result<CapabilityKind> {
            CapabilityKind::new(
                "mfm.certify.test",
                "mutation",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x61),
            )
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn version() -> mfm_capabilities::Result<CapabilityVersion> {
            CapabilityVersion::new("mfm.certify.test.mutation.v1")
                .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.mutation"
        }
    }

    struct MutatingState {
        config: TestConfig,
    }

    impl StateSpec for MutatingState {
        type Config = TestConfig;
        type Input = TestValue;
        type Output = TestValue;
        type Effect = ApplySideEffect;
        type Caps = (MutationCap,);

        fn kind() -> program::Result<StateKind> {
            StateKind::new(
                "mfm.certify.test",
                "mutating",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x62),
            )
            .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn version() -> program::Result<StateVersion> {
            StateVersion::new("mfm.certify.test.mutating.v1")
                .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.mutating"
        }

        fn new(config: program::ValidatedConfig<Self::Config>) -> program::Result<Self> {
            Ok(Self {
                config: config.into_inner(),
            })
        }
    }

    impl SideEffectState for MutatingState {
        type Intent = TestValue;
        type IdempotencyInput = TestValue;
        type Submission = TestValue;
        type Receipt = TestValue;
        type Confirmation = TestValue;
        type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

        fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
            Ok(TestValue {
                amount: input.amount * self.config.multiplier,
            })
        }

        fn idempotency_input(
            &self,
            _input: &Self::Input,
            intent: &Self::Intent,
        ) -> StateResult<Self::IdempotencyInput> {
            Ok(intent.clone())
        }

        fn submit<'a>(
            &'a self,
            intent: &'a Self::Intent,
            _key: &'a IdempotencyKey<Self::IdempotencyInput>,
            _caps: &'a Self::Caps,
        ) -> Self::SubmitFuture<'a> {
            std::future::ready(Ok(intent.clone()))
        }

        fn output_from_confirmation(
            &self,
            _input: &Self::Input,
            _intent: &Self::Intent,
            confirmation: &Self::Confirmation,
        ) -> StateResult<Self::Output> {
            Ok(confirmation.clone())
        }
    }

    struct MultiplyOperation;

    impl Operation for MultiplyOperation {
        type Config = OperationConfig;
        type Input<'p, 's> = mfm_program::Handle<'p, 's, TestValue>;
        type Output<'p, 's> = TestOperationOutputs<'p, 's>;

        fn kind() -> program::Result<OperationKind> {
            OperationKind::new(
                "mfm.certify.test",
                "operation-multiply",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x12),
            )
            .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn version() -> program::Result<OperationVersion> {
            OperationVersion::new("mfm.certify.test.operation_multiply.v1")
                .map_err(|error| program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.certify.test.operation_multiply"
        }

        fn expand<'p, 's>(
            &self,
            config: program::ValidatedConfig<Self::Config>,
            input: Self::Input<'p, 's>,
            builder: &mut mfm_program::OperationExpansion<'p, 's>,
            _dispatch: mfm_program::OperationExpansionDispatch<Self>,
        ) -> program::Result<Self::Output<'p, 's>> {
            let result = builder.state::<MultiplyState, _>(
                StateKey::new("multiply-state")?,
                TestConfig {
                    multiplier: config.as_ref().multiplier,
                },
                input,
            )?;
            Ok(TestOperationOutputs { result })
        }
    }

    fn reference_draft() -> program::TypedProgramDraft {
        let mut states = StateRegistryBuilder::new();
        states
            .register::<MultiplyState>()
            .expect("state registration");
        let mut operations = OperationRegistryBuilder::new();
        operations
            .register::<MultiplyOperation>()
            .expect("operation registration");
        build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            operations.snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                let seed = root.seed(
                    mfm_program::SeedKey::new("initial").expect("seed key"),
                    CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
                )?;
                let result = root.scope().call::<MultiplyOperation, _>(
                    OperationKey::new("multiply-operation")?,
                    MultiplyOperation,
                    OperationConfig { multiplier: 3 },
                    seed,
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &TestPublicOutputs {
                        result: result.result,
                    },
                )
            },
        )
        .expect("reference draft")
    }

    fn side_effect_draft() -> program::TypedProgramDraft {
        let mut states = StateRegistryBuilder::new();
        states
            .register::<MutatingState>()
            .expect("state registration");
        build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            OperationRegistryBuilder::new().snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                root.set_saga_policy(program::SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
                let seed = root.seed(
                    mfm_program::SeedKey::new("initial").expect("seed key"),
                    CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
                )?;
                let result = root.scope().side_effect::<MutatingState, _>(
                    StateKey::new("mutating-state")?,
                    TestConfig { multiplier: 3 },
                    seed,
                    ResourceClaim::manual_only(),
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &TestPublicOutputs {
                        result: result.into_handle(),
                    },
                )
            },
        )
        .expect("side effect draft")
    }

    fn compensating_draft() -> program::TypedProgramDraft {
        let mut states = StateRegistryBuilder::new();
        states
            .register::<MutatingState>()
            .expect("state registration");
        build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            OperationRegistryBuilder::new().snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                root.set_saga_policy(program::SideEffectSagaPolicy::CompensateCompleted {
                    on_remediation_unresolved: program::RemediationUnresolved::FailWithoutAcdcClaim,
                })?;
                let seed = root.seed(
                    mfm_program::SeedKey::new("initial").expect("seed key"),
                    CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
                )?;
                let (forward, _remediation) = root
                    .scope()
                    .side_effect_with_compensation::<MutatingState, MutatingState, _, _, _>(
                        program::SideEffectNodeParams {
                            key: StateKey::new("mutating-state")?,
                            config: TestConfig { multiplier: 3 },
                            input: seed,
                            resource_claim: ResourceClaim::manual_only(),
                        },
                        program::RemediationNodeParams {
                            key: StateKey::new("compensating-state")?,
                            config: TestConfig { multiplier: 1 },
                            resource_claim: ResourceClaim::manual_only(),
                        },
                        Ok,
                    )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &TestPublicOutputs {
                        result: forward.into_handle(),
                    },
                )
            },
        )
        .expect("compensating draft")
    }

    fn config_ref_for_bytes<C: mfm_values::MfmConfig>(
        bytes: &PlainCanonicalJsonBytes,
    ) -> spec::ConfigRef {
        let digest = bytes.content_digest();
        spec::ConfigRef {
            schema_id: C::schema_id().expect("config schema"),
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: bytes.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
        }
    }

    #[test]
    fn registered_config_validator_rejects_schema_shape_mismatch() {
        let mut states = StateRegistryBuilder::new();
        let registered = states
            .register::<MultiplyState>()
            .expect("state registration");
        let mut registry = CertificationRegistry::new();
        registry
            .register_state(&registered)
            .expect("certification state registration");
        let invalid = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":"bad"}"#)
            .expect("canonical invalid config");
        let config_ref = config_ref_for_bytes::<TestConfig>(&invalid);

        let err = registry
            .validate_config_ref_bytes(&config_ref, invalid.as_bytes())
            .expect_err("invalid typed config shape rejects");
        assert!(err
            .to_string()
            .contains("typed config did not match registered schema"));
    }

    #[test]
    fn registered_config_validator_rejects_noncanonical_bytes() {
        let mut states = StateRegistryBuilder::new();
        let registered = states
            .register::<MultiplyState>()
            .expect("state registration");
        let mut registry = CertificationRegistry::new();
        registry
            .register_state(&registered)
            .expect("certification state registration");
        let canonical = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":2}"#)
            .expect("canonical config");
        let config_ref = config_ref_for_bytes::<TestConfig>(&canonical);

        let err = registry
            .validate_config_ref_bytes(&config_ref, br#"{ "multiplier": 2 }"#)
            .expect_err("noncanonical config rejects");
        let rendered = err.to_string();
        assert!(rendered.contains("typed config"));
        assert!(rendered.contains("not normalized canonical JSON"));
    }

    #[test]
    fn registered_config_validator_rejects_non_authoritative_canonical_encoding() {
        let mut registry = CertificationRegistry::new();
        registry
            .insert_config_validator(config_validator_for::<DefaultedConfig>().expect("validator"))
            .expect("insert validator");
        let supplied = PlainCanonicalJsonBytes::from_json_str(r#"{}"#)
            .expect("canonical but not authoritative config");
        let config_ref = config_ref_for_bytes::<DefaultedConfig>(&supplied);

        let err = registry
            .validate_config_ref_bytes(&config_ref, supplied.as_bytes())
            .expect_err("non-authoritative canonical config rejects");
        assert!(err
            .to_string()
            .contains("did not match registered canonical encoding"));
    }

    #[test]
    fn trusted_draft_config_ref_accepts_exact_bytes_without_descriptor_validator() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("draft registry");
        let config = draft
            .state_nodes()
            .first()
            .expect("state node")
            .config
            .clone();
        let config_ref = spec::ConfigRef {
            schema_id: config.schema_id.clone(),
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                *config.content_digest.digest(),
            ),
            digest: config.content_digest.clone(),
            byte_len: config.byte_len as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
        };

        let source = registry
            .validate_config_ref_bytes(&config_ref, config.canonical_json.as_bytes())
            .expect("trusted exact config validates");
        assert_eq!(source, Some(ConfigValidationSource::TrustedExactRef));
    }

    #[test]
    fn certifies_reference_program_draft() {
        let draft = reference_draft();
        let expected_public_schema = draft.public_output_spec().public_schema_id().clone();
        let certified = certify_program_draft(&draft).expect("certified");
        certified.envelope().verify_hash().expect("hash verifies");
        certified
            .certificate()
            .verify_hash()
            .expect("certificate hash verifies");
        assert_eq!(
            certified.certificate().evidence.certifier_algorithm,
            CERTIFIER_ALGORITHM
        );
        assert_eq!(
            certified.certificate().evidence.spec_hash,
            *certified.spec_hash()
        );
        assert_eq!(
            certified.certificate_hash().as_str(),
            "content:sha256-jcs-v1:b10ace9a11f373d953d1e0056bba864d524667b3fa54fa386203622987496b93"
        );
        assert_eq!(
            certified.envelope().spec.public_outputs.public_schema_id,
            expected_public_schema
        );
        assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        )));
        assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        )));
        assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        )));
        assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        )));
    }

    #[test]
    fn verifies_persisted_spec_certificate_parts() {
        let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
        let verified = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            persisted_parts.certificate_bytes(),
            &registry,
        )
        .expect("verified persisted spec/certificate");
        assert_eq!(verified.spec_hash(), certified.spec_hash());
        assert_eq!(verified.certificate_hash(), certified.certificate_hash());
    }

    #[test]
    fn registry_digest_mismatch_rejects_persisted_parts() {
        let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
        let mut evidence = certified.certificate().evidence.clone();
        evidence.registry_digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x72));
        let certificate = CertifiedSpecCertificate::from_evidence(evidence).expect("certificate");
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            &certificate_bytes(&certificate),
            &registry,
        )
        .expect_err("registry mismatch rejects");
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
    }

    #[test]
    fn descriptor_identity_or_digest_mismatch_rejects_persisted_parts() {
        let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();

        let mut identity_mismatch = certified.certificate().evidence.clone();
        identity_mismatch.descriptor_identities[0].descriptor_id =
            DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x73));
        let identity_certificate =
            CertifiedSpecCertificate::from_evidence(identity_mismatch).expect("certificate");
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            &certificate_bytes(&identity_certificate),
            &registry,
        )
        .expect_err("descriptor identity mismatch rejects");
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");

        let mut digest_mismatch = certified.certificate().evidence.clone();
        digest_mismatch.descriptor_identities[0].descriptor_digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x74));
        let digest_certificate =
            CertifiedSpecCertificate::from_evidence(digest_mismatch).expect("certificate");
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            &certificate_bytes(&digest_certificate),
            &registry,
        )
        .expect_err("descriptor digest mismatch rejects");
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
    }

    #[test]
    fn certificate_spec_hash_mismatch_rejects_persisted_parts() {
        let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
        let mut evidence = certified.certificate().evidence.clone();
        evidence.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x75));
        let certificate = CertifiedSpecCertificate::from_evidence(evidence).expect("certificate");
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            &certificate_bytes(&certificate),
            &registry,
        )
        .expect_err("spec hash mismatch rejects");
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
    }

    #[test]
    fn parsed_persisted_parts_are_untrusted_until_verifier_succeeds() {
        let (_registry, _certified, persisted_parts) = reference_persisted_spec_certificate_parts();
        let untrusted = persisted_parts
            .parse_untrusted()
            .expect("parsed untrusted persisted parts");
        assert_eq!(
            untrusted.spec().spec_hash().expect("untrusted spec hash"),
            untrusted.certificate().evidence.spec_hash
        );
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            persisted_parts.certificate_bytes(),
            &CertificationRegistry::new(),
        )
        .expect_err("parsed persisted parts need registry-backed verifier success");
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
    }

    #[test]
    fn typed_certification_rejects_problem_taxonomy() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.cells.push(spec.cells[0].clone());
        });
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidInterfaceWiring,
            |spec| {
                let node = spec
                    .nodes
                    .iter_mut()
                    .find(|node| {
                        matches!(
                            node.input_bindings.root,
                            spec::InputBindingNodeSpec::Cell(_)
                        )
                    })
                    .expect("cell-input node");
                let first_input = match &mut node.input_bindings.root {
                    spec::InputBindingNodeSpec::Cell(cell) => cell,
                    _ => panic!("expected cell input"),
                };
                first_input.schema_id = SchemaId::new(
                    "mfm.certify.test.wrong",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x44),
                )
                .expect("schema id");
                node.input_bindings.digest =
                    content_digest_json(input_node_json(&node.input_bindings.root))
                        .expect("input digest");
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.nodes[0].effect_kind = ApplySideEffect::descriptor()
                    .expect("side effect descriptor")
                    .kind;
            },
        );
        assert_rejects(&registry, &base, ProblemClass::InvalidDataShape, |spec| {
            spec.config_refs.clear();
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidDataMeaning, |spec| {
            let seed_lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| matches!(lineage.producer, spec::CellProducer::Seed(_)))
                .expect("seed lineage");
            seed_lineage.config_ref_digest = Some(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x55),
            ));
        });
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidTerminalShape,
            |spec| {
                spec.public_outputs.outputs.clear();
            },
        );
    }

    #[test]
    fn typed_spec_requires_registry_authority() {
        let spec = certify_program_draft(&reference_draft())
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();
        let error =
            certify_raw_typed_spec(spec, &CertificationRegistry::new()).expect_err("must reject");
        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition)
        );
    }

    #[test]
    fn certification_rejects_forged_framework_descriptor_bypass() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let mut forged = spec
                    .descriptor_identities
                    .iter()
                    .find_map(|descriptor| match descriptor {
                        spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                        _ => None,
                    })
                    .expect("state descriptor");
                forged.name = "mfm.framework.forged".to_owned();
                forged.state_kind =
                    state_kind_json("forged", serde_json::json!({ "framework": "forged" }))
                        .expect("state kind");
                forged.state_version =
                    StateVersion::new("mfm.framework.state.forged.v1").expect("state version");
                forged.descriptor_id =
                    state_descriptor_id_from_spec(&forged).expect("descriptor id");
                spec.descriptor_identities
                    .push(spec::DescriptorIdentity::State(Box::new(forged)));
            },
        );
    }

    #[test]
    fn certification_accepts_valid_lifecycle_framework_node_shapes() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let spec = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        certify_raw_typed_spec(spec, &registry).expect("lifecycle framework nodes certify");
    }

    #[test]
    fn certification_mints_descriptor_set_and_lifecycle_views() {
        let certified = certify_program_draft(&reference_draft()).expect("certified");
        let spec = certified.validated_spec().spec();
        let graph = certified.validated_spec().graph();
        let descriptors = certified.descriptor_set();
        let lifecycle = certified.framework_lifecycle();

        assert_eq!(graph.scope_count(), spec.scopes.len());
        assert_eq!(graph.config_ref_count(), spec.config_refs.len());
        assert_eq!(graph.value_lineage_count(), spec.value_lineages.len());
        assert_eq!(graph.cell_count(), spec.cells.len());
        assert_eq!(graph.forward_node_count(), spec.nodes.len());
        assert_eq!(graph.remediation_count(), spec.remediations.len());
        assert_eq!(graph.descriptors(), descriptors);
        assert!(descriptors.state_descriptors().count() > 0);
        assert!(descriptors
            .renderer(&spec.public_outputs.renderer_descriptor.descriptor_id)
            .is_some());
        assert!(descriptors
            .state(lifecycle.render().descriptor_id())
            .is_some());
        assert!(descriptors
            .state(lifecycle.retention().descriptor_id())
            .is_some());
        assert!(descriptors
            .state(lifecycle.complete().descriptor_id())
            .is_some());
        assert!(descriptors
            .state(lifecycle.resolve().descriptor_id())
            .is_some());
        assert!(graph.cell(lifecycle.render().output_cell()).is_some());
        assert!(graph.forward_node(lifecycle.render().node_id()).is_some());
        assert_ne!(
            lifecycle.render().node_id(),
            lifecycle.retention().node_id()
        );
        assert_ne!(
            lifecycle.retention().output_cell(),
            lifecycle.complete().output_cell()
        );
    }

    #[test]
    fn certification_rejects_missing_retention_lifecycle_node() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.nodes.retain(|node| {
                !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
                )
            });
        });
    }

    #[test]
    fn certification_rejects_forged_lifecycle_framework_variant() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let forged = spec
                    .nodes
                    .iter()
                    .find_map(|node| match &node.framework {
                        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                            Some(retention.clone())
                        }
                        _ => None,
                    })
                    .expect("retention metadata");
                let node = spec
                    .nodes
                    .iter_mut()
                    .find(|node| node.framework.is_none())
                    .expect("user node");
                node.framework = Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(forged));
            },
        );
    }

    #[test]
    fn certification_rejects_lifecycle_descriptor_mismatch() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let mut forged = spec
                    .descriptor_identities
                    .iter()
                    .find_map(|descriptor| match descriptor {
                        spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                        _ => None,
                    })
                    .expect("state descriptor");
                forged.name = "mfm.framework.complete_run".to_owned();
                forged.state_kind = state_kind_json(
                    "complete_run",
                    serde_json::json!({ "framework": "complete_run" }),
                )
                .expect("complete state kind");
                forged.state_version = StateVersion::new("mfm.framework.state.complete_run.v1")
                    .expect("complete state version");
                forged.runner = "forged".to_owned();
                forged.descriptor_id =
                    state_descriptor_id_from_spec(&forged).expect("descriptor id");
                spec.descriptor_identities
                    .push(spec::DescriptorIdentity::State(Box::new(forged)));
            },
        );
    }

    #[test]
    fn certification_rejects_invalid_lifecycle_ordering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let public_cell = spec
                .public_outputs
                .outputs
                .first()
                .expect("public output")
                .cell_id
                .clone();
            append_retention_lifecycle_node(spec, public_cell);
        });
    }

    #[test]
    fn certification_rejects_lifecycle_framework_permissions() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
                node.adapter_bindings.push(spec::AdapterBinding {
                    adapter_kind: AdapterKind::new(
                        "mfm.certify.test",
                        "forged-adapter",
                        DigestAlgorithm::Sha256JcsV1,
                        digest_byte(0xa1),
                    )
                    .expect("adapter kind"),
                    adapter_version: AdapterVersion::new("mfm.certify.test.adapter.v1")
                        .expect("adapter version"),
                    binding_digest: None,
                });
            },
        );
    }

    #[test]
    fn certification_rejects_user_consumers_of_lifecycle_receipts() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let render_receipt = lifecycle_render_receipt_cell(spec);
            append_user_receipt_consumer(spec, render_receipt, "user/render-receipt");
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let retention_receipt = lifecycle_retention_receipt_cell(spec);
            append_user_receipt_consumer(spec, retention_receipt, "user/retention-receipt");
        });
    }

    #[test]
    fn certification_rejects_executable_nodes_outside_lifecycle_tail() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            append_independent_user_node(spec, "user/outside-lifecycle-tail");
        });
    }

    #[test]
    fn certification_rejects_forged_lifecycle_config_ref() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
                node.config_ref.byte_len += 1;
            },
        );
    }

    #[test]
    fn certification_rejects_builtin_lifecycle_descriptors_without_framework_metadata() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                clear_lifecycle_framework_metadata::<spec::ProjectRetentionManifestNodeSpec>(spec);
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                clear_lifecycle_framework_metadata::<spec::CompleteRunNodeSpec>(spec);
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                clear_lifecycle_framework_metadata::<spec::ResolveSagaTerminalNodeSpec>(spec);
            },
        );
    }

    #[test]
    fn certification_rejects_forged_lifecycle_input_binding() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidInterfaceWiring,
            |spec| {
                let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
                let current = match &node.input_bindings.root {
                    spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                    _ => panic!("expected cell input"),
                };
                node.input_bindings.root =
                    spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                        field_path: spec::PublicFieldPath::new("public_output_receipt")
                            .expect("field path"),
                        node: spec::InputBindingNodeSpec::Cell(current),
                    }]);
                node.input_bindings.digest =
                    content_digest_json(input_node_json(&node.input_bindings.root))
                        .expect("input digest");
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidInterfaceWiring,
            |spec| {
                let node = find_lifecycle_node_mut::<spec::CompleteRunNodeSpec>(spec);
                let current = match &node.input_bindings.root {
                    spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                    _ => panic!("expected cell input"),
                };
                node.input_bindings.root =
                    spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                        field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                            .expect("field path"),
                        node: spec::InputBindingNodeSpec::Cell(current),
                    }]);
                node.input_bindings.digest =
                    content_digest_json(input_node_json(&node.input_bindings.root))
                        .expect("input digest");
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidInterfaceWiring,
            |spec| {
                let input_cell = spec.cells.first().expect("input cell").clone();
                let node = find_lifecycle_node_mut::<spec::ResolveSagaTerminalNodeSpec>(spec);
                node.input_bindings.root =
                    spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                        field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                            .expect("field path"),
                        node: spec::InputBindingNodeSpec::Cell(Box::new(
                            spec::InputBindingCellSpec {
                                field_path: spec::PublicFieldPath::new(
                                    "retention_manifest_receipt",
                                )
                                .expect("field path"),
                                cell_id: input_cell.cell_id,
                                semantic_type_id: input_cell.semantic_type_id,
                                schema_id: input_cell.schema_id,
                                required_terminal: spec::RequiredTerminal::ProducedOnly,
                                value_lineage: input_cell.value_lineage,
                            },
                        )),
                    }]);
                node.input_bindings.digest =
                    content_digest_json(input_node_json(&node.input_bindings.root))
                        .expect("input digest");
            },
        );
    }

    #[test]
    fn certification_rejects_operation_input_binding_tampering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let frame = spec
                .planning_lineage
                .first_mut()
                .expect("operation lineage frame");
            let input_cell = match &mut frame.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell,
                _ => panic!("expected operation cell input"),
            };
            input_cell.cell_id =
                CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x91));
            frame.input_bindings.digest =
                content_digest_json(input_node_json(&frame.input_bindings.root))
                    .expect("input digest");
            frame.input_binding_digest = frame.input_bindings.digest.clone();
        });
    }

    #[test]
    fn certification_rejects_stable_id_and_scope_tampering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.nodes[0].stable_key =
                spec::StableAuthorKey::new("renamed-node").expect("node key");
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.scopes[0].parent_scope_id = Some(spec.scopes[0].scope_id.clone());
        });
    }

    #[test]
    fn certification_rejects_framework_lineage_tampering() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidDataMeaning, |spec| {
            spec.planning_lineage.clear();
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidDataMeaning, |spec| {
            let lineage = spec
                .nodes
                .first_mut()
                .expect("node with operation planning lineage");
            lineage.planning_lineage.active_operation_instances =
                vec![OperationInstanceId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x92),
                )];
            lineage.planning_lineage.lineage_digest = planning_lineage_digest(
                &lineage.planning_lineage.active_operation_instances,
                &lineage.planning_lineage.completed_operation_frames,
            )
            .expect("planning lineage digest");
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidDataMeaning, |spec| {
            let render_node_id = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                    )
                })
                .expect("render node")
                .node_id
                .clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(render_node_id.clone())
                })
                .expect("render lineage");
            lineage.input_cells.clear();
        });
    }

    #[test]
    fn certification_rejects_side_effect_contract_mismatch() {
        let draft = side_effect_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let node = spec
                    .nodes
                    .iter_mut()
                    .find(|node| node.side_effect.is_some())
                    .expect("side-effect node");
                node.side_effect = Some(spec::SideEffectContractSpec {
                    contract_digest: ContentDigest::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        digest_byte(0x63),
                    ),
                    resource_claim: spec::ResourceClaimSpec::ManualOnly,
                });
            },
        );
    }

    #[test]
    fn certifies_compensating_draft_with_separate_remediation_collection() {
        let draft = compensating_draft();
        let certified = certify_program_draft(&draft).expect("certified compensating draft");
        assert!(matches!(
            certified.validated_spec().spec().saga,
            spec::SagaPolicySpec::CompensateCompleted { .. }
        ));
        assert_eq!(certified.validated_spec().spec().remediations.len(), 1);
        let validated = certified.validated_spec().spec();
        let (forward_id, remediation) = validated.remediations.iter().next().expect("remediation");
        assert!(validated
            .nodes
            .iter()
            .any(|node| node.node_id == *forward_id));
        assert!(!validated
            .nodes
            .iter()
            .any(|node| node.node_id == remediation.node_id));
    }

    #[test]
    fn certified_side_effect_contract_validates_resource_evidence() {
        let (_, mut typed) = side_effect_spec_with_manual(manual_resolution_spec(0xc0));
        let node = typed
            .nodes
            .iter_mut()
            .find(|node| node.side_effect.is_some())
            .expect("side-effect node");
        let node_id = node.node_id.clone();
        let namespace = spec::ResourceNamespace::new("mfm.certify.test.wallet").expect("namespace");
        let key_schema = SchemaId::new(
            "mfm.certify.test.resource_key",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc1),
        )
        .expect("key schema");
        node.side_effect
            .as_mut()
            .expect("side effect")
            .resource_claim = spec::ResourceClaimSpec::Exclusive {
            namespace: namespace.clone(),
            key_schema: key_schema.clone(),
        };
        let contract =
            CertifiedSideEffectContract::for_node(&typed, &node_id).expect("certified contract");
        let key = events::ResourceKeyEvidence {
            namespace: namespace.clone(),
            key_schema_id: key_schema,
            key: events::ResourceKey::new("wallet-1").expect("resource key"),
        };
        assert!(contract.validate_resource_key(Some(&key)).is_ok());
        assert!(matches!(
            contract
                .validate_resource_key(None)
                .expect_err("missing key")
                .problem_class(),
            Some(ProblemClass::InvalidSemanticTransition)
        ));
        let touched_set = events::ResourceTouchedSetEvidence {
            namespace,
            evidence_schema_id: SchemaId::new(
                "mfm.certify.test.touched_set",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0xc2),
            )
            .expect("touched schema"),
            evidence_hash: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0xc3),
            ),
            evidence_artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0xc3),
            ),
        };
        assert!(contract.validate_touched_set(Some(&touched_set)).is_err());
    }

    #[test]
    fn certified_side_effect_contract_validates_remediation_links() {
        let (_, typed) = compensating_registry_and_spec();
        let (forward_node_id, remediation) = typed.remediations.iter().next().expect("remediation");
        let contract = CertifiedSideEffectContract::for_node(&typed, &remediation.node_id)
            .expect("remediation contract");
        let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xc4));
        let purpose = events::SideEffectLedgerPurpose::Remediation {
            forward_ledger_key: events::SideEffectLedgerKey::new("forward-ledger")
                .expect("forward ledger"),
        };
        let forward_purpose = events::SideEffectLedgerPurpose::Forward;

        assert!(contract
            .validate_remediation_link(CertifiedRemediationLink {
                remediation_run_id: &run_id,
                ledger_purpose: &purpose,
                forward_run_id: None,
                forward_node_id: None,
                forward_ledger_purpose: None,
                forward_confirmed: false,
            })
            .is_err());
        assert!(contract
            .validate_remediation_link(CertifiedRemediationLink {
                remediation_run_id: &run_id,
                ledger_purpose: &purpose,
                forward_run_id: Some(&run_id),
                forward_node_id: Some(forward_node_id),
                forward_ledger_purpose: Some(&forward_purpose),
                forward_confirmed: true,
            })
            .is_ok());
    }

    #[test]
    fn certification_rejects_saga_policy_graph_disagreement() {
        let draft = side_effect_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let side_effecting = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(
            &registry,
            &side_effecting,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.saga = spec::SagaPolicySpec::NoSideEffects;
            },
        );

        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let pure = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();
        assert_rejects(
            &registry,
            &pure,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.saga = spec::SagaPolicySpec::FailWithoutAcdcClaim;
            },
        );
    }

    #[test]
    fn certification_rejects_invalid_remediation_keys_and_coverage() {
        let (registry, base) = compensating_registry_and_spec();
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let (_, mut remediation) = spec.remediations.pop_first().expect("remediation");
            remediation.node_id = node_id(0xb1);
            spec.remediations.insert(node_id(0xb0), remediation);
        });
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                let remediation = spec
                    .remediations
                    .values()
                    .next()
                    .expect("remediation")
                    .clone();
                let mut cloned = remediation;
                cloned.node_id = node_id(0xb2);
                let lifecycle_node =
                    find_lifecycle_node_mut::<spec::PublicOutputRenderNodeSpec>(spec)
                        .node_id
                        .clone();
                spec.remediations.insert(lifecycle_node, cloned);
            },
        );
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.remediations.clear();
        });
    }

    #[test]
    fn certification_rejects_invalid_remediation_node_shapes() {
        let (registry, base) = compensating_registry_and_spec();
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.saga = spec::SagaPolicySpec::FailWithoutAcdcClaim;
            },
        );
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let forward_id = spec
                .remediations
                .keys()
                .next()
                .expect("forward key")
                .clone();
            spec.remediations
                .get_mut(&forward_id)
                .expect("remediation")
                .node_id = forward_id.clone();
        });
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.remediations
                    .values_mut()
                    .next()
                    .expect("remediation")
                    .side_effect = None;
            },
        );
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidSemanticTransition,
            |spec| {
                spec.remediations
                    .values_mut()
                    .next()
                    .expect("remediation")
                    .state_version =
                    StateVersion::new("mfm.certify.test.forged.v1").expect("state version");
            },
        );
    }

    #[test]
    fn certification_rejects_out_of_scope_remediation_bindings() {
        let (registry, base) = compensating_registry_and_spec();
        assert_rejects(
            &registry,
            &base,
            ProblemClass::InvalidInterfaceWiring,
            |spec| {
                let render_receipt = lifecycle_render_receipt_cell(spec);
                retarget_first_remediation_input(spec, render_receipt);
            },
        );
    }

    #[test]
    fn certification_rejects_manual_policy_without_schemas_at_decode_boundary() {
        let draft = side_effect_draft();
        let mut typed = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();
        let operator_schema = SchemaId::new(
            "mfm.certify.test.operator_ref",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc1),
        )
        .expect("operator schema");
        typed.saga = spec::SagaPolicySpec::ManualResolution {
            manual: spec::ManualResolutionEvidenceSpec {
                evidence_schema: operator_schema.clone(),
                authorization: spec::ManualResolutionAuthorizationSpec {
                    verifier_id: spec::ManualAuthorizationVerifierId::new(
                        "mfm.certify.test.manual.verifier",
                    )
                    .expect("verifier id"),
                    signing_scheme: spec::ManualSigningSchemeSpec::new(
                        "mfm.manual_resolution.digest_signature.v1",
                    )
                    .expect("signing scheme"),
                    authority: spec::OperatorAuthoritySnapshotSpec {
                        authority_id: spec::OperatorAuthorityId::new(
                            "mfm.certify.test.manual.authority",
                        )
                        .expect("authority id"),
                        operators: vec![spec::OperatorAuthorityMemberSpec {
                            operator_id: spec::OperatorId::new("operator.certify")
                                .expect("operator id"),
                            public_identity: spec::OperatorPublicIdentity::new(
                                "operator-certify-public",
                            )
                            .expect("operator public identity"),
                        }],
                    },
                    quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
                },
            },
        };
        let mut value: serde_json::Value =
            serde_json::from_str(typed.canonical_json().expect("canonical spec").as_str())
                .expect("spec JSON");
        value["saga"]["manual"]
            .as_object_mut()
            .expect("manual object")
            .remove("evidence_schema");
        let input = serde_json::to_string(&value).expect("JSON");
        let error = spec::TypedExecutionSpec::from_json_str(&input)
            .expect_err("missing manual evidence schema rejects");
        assert!(error.to_string().contains("evidence_schema"), "{error}");
    }

    #[test]
    fn certification_records_manual_authority_evidence() {
        let manual = manual_resolution_spec(0xc1);
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        register_manual_authority(&mut registry, &manual);

        let certified = certify_raw_typed_spec(typed, &registry).expect("certified manual spec");

        assert_eq!(
            certified
                .certificate()
                .evidence
                .schema_role_grants
                .as_slice(),
            &[CertifiedSchemaRoleGrantEvidence {
                schema_id: manual.evidence_schema.clone(),
                role: CertifiedSchemaRole::ManualResolutionEvidence,
            }]
        );
        assert_eq!(
            certified
                .certificate()
                .evidence
                .manual_authorization_verifiers
                .as_slice(),
            &[CertifiedManualAuthorizationVerifierEvidence {
                verifier_id: manual.authorization.verifier_id.clone(),
            }]
        );
        assert_eq!(
            certified
                .certificate()
                .evidence
                .operator_authority_snapshots
                .len(),
            1
        );
        assert_eq!(
            certified
                .certificate()
                .evidence
                .operator_authority_snapshots[0]
                .authority_id,
            manual.authorization.authority.authority_id
        );
    }

    #[test]
    fn certification_rejects_unknown_manual_evidence_schema() {
        let manual = manual_resolution_spec(0xc2);
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        registry
            .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
            .expect("register verifier");
        registry
            .register_operator_authority_snapshot(manual.authorization.authority.clone())
            .expect("register authority");

        let error = certify_raw_typed_spec(typed, &registry).expect_err("unknown schema rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error.to_string().contains("unknown manual evidence schema"));
    }

    #[test]
    fn certification_rejects_manual_schema_with_wrong_role() {
        let manual = manual_resolution_spec(0xc3);
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        registry
            .register_schema_role(
                manual.evidence_schema.clone(),
                CertifiedSchemaRole::ManualResolutionAuthorization,
            )
            .expect("register wrong schema role");
        registry
            .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
            .expect("register verifier");
        registry
            .register_operator_authority_snapshot(manual.authorization.authority.clone())
            .expect("register authority");

        let error = certify_raw_typed_spec(typed, &registry).expect_err("wrong role rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error.to_string().contains("wrong certified role"));
    }

    #[test]
    fn certification_rejects_unknown_manual_authorization_verifier() {
        let manual = manual_resolution_spec(0xc4);
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        registry
            .register_schema_role(
                manual.evidence_schema.clone(),
                CertifiedSchemaRole::ManualResolutionEvidence,
            )
            .expect("register schema role");
        registry
            .register_operator_authority_snapshot(manual.authorization.authority.clone())
            .expect("register authority");

        let error = certify_raw_typed_spec(typed, &registry).expect_err("unknown verifier rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error
            .to_string()
            .contains("unknown manual authorization verifier"));
    }

    #[test]
    fn certification_rejects_operator_authority_snapshot_mismatch() {
        let manual = manual_resolution_spec(0xc5);
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        registry
            .register_schema_role(
                manual.evidence_schema.clone(),
                CertifiedSchemaRole::ManualResolutionEvidence,
            )
            .expect("register schema role");
        registry
            .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
            .expect("register verifier");
        let mut mismatched = manual.authorization.authority.clone();
        mismatched.operators[0].public_identity =
            spec::OperatorPublicIdentity::new("operator-certify-public-mismatch")
                .expect("operator public identity");
        registry
            .register_operator_authority_snapshot(mismatched)
            .expect("register mismatched authority");

        let error =
            certify_raw_typed_spec(typed, &registry).expect_err("authority mismatch rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error.to_string().contains("does not match registry"));
    }

    #[test]
    fn certification_rejects_empty_operator_authority() {
        let mut manual = manual_resolution_spec(0xc6);
        manual.authorization.authority.operators.clear();
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        register_manual_authority(&mut registry, &manual);

        let error = certify_raw_typed_spec(typed, &registry).expect_err("empty authority rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error.to_string().contains("has no operators"));
    }

    #[test]
    fn certification_rejects_unsupported_manual_signing_scheme_and_quorum() {
        let mut manual = manual_resolution_spec(0xc7);
        manual.authorization.signing_scheme =
            spec::ManualSigningSchemeSpec::new("mfm.certify.test.unsupported-signing.v1")
                .expect("signing scheme");
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        register_manual_authority(&mut registry, &manual);

        let error = certify_raw_typed_spec(typed, &registry).expect_err("signing scheme rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error
            .to_string()
            .contains("unsupported manual authorization signing scheme"));

        let mut manual = manual_resolution_spec(0xc8);
        manual.authorization.quorum = spec::ManualAuthorizationQuorumSpec::new(2).expect("quorum");
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        register_manual_authority(&mut registry, &manual);

        let error = certify_raw_typed_spec(typed, &registry).expect_err("quorum rejects");

        assert_eq!(
            error.problem_class(),
            Some(ProblemClass::InvalidSemanticTransition),
            "{error}"
        );
        assert!(error.to_string().contains("quorum 2 exceeds"));
    }

    #[test]
    fn certification_rejects_missing_or_duplicate_resolve_saga_terminal_node() {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let base = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();

        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            spec.nodes.retain(|node| {
                !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                )
            });
        });
        assert_rejects(&registry, &base, ProblemClass::InvalidTopology, |spec| {
            let duplicate = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                    )
                })
                .expect("resolve node")
                .clone();
            spec.nodes.push(duplicate);
        });
    }

    fn assert_rejects(
        registry: &CertificationRegistry,
        base: &spec::TypedExecutionSpec,
        expected: ProblemClass,
        mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
    ) {
        let mut mutated = base.clone();
        mutate(&mut mutated);
        let error = certify_raw_typed_spec(mutated, registry).expect_err("mutation must reject");
        assert_eq!(error.problem_class(), Some(expected), "{error}");
    }

    fn compensating_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
        let draft = compensating_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let spec = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();
        (registry, spec)
    }

    fn side_effect_spec_with_manual(
        manual: spec::ManualResolutionEvidenceSpec,
    ) -> (CertificationRegistry, spec::TypedExecutionSpec) {
        let draft = side_effect_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let mut typed = certify_program_draft(&draft)
            .expect("certified")
            .validated_spec()
            .spec()
            .clone();
        typed.saga = spec::SagaPolicySpec::ManualResolution { manual };
        (registry, typed)
    }

    fn manual_resolution_spec(byte: u8) -> spec::ManualResolutionEvidenceSpec {
        let evidence_schema = SchemaId::new(
            "mfm.certify.test.manual_evidence",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(byte),
        )
        .expect("manual evidence schema");
        spec::ManualResolutionEvidenceSpec {
            evidence_schema,
            authorization: spec::ManualResolutionAuthorizationSpec {
                verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
                    "mfm.certify.test.manual.verifier.{byte:02x}"
                ))
                .expect("verifier id"),
                signing_scheme: spec::ManualSigningSchemeSpec::new(
                    MANUAL_RESOLUTION_SIGNING_SCHEME,
                )
                .expect("signing scheme"),
                authority: spec::OperatorAuthoritySnapshotSpec {
                    authority_id: spec::OperatorAuthorityId::new(format!(
                        "mfm.certify.test.manual.authority.{byte:02x}"
                    ))
                    .expect("authority id"),
                    operators: vec![spec::OperatorAuthorityMemberSpec {
                        operator_id: spec::OperatorId::new(format!("operator.certify.{byte:02x}"))
                            .expect("operator id"),
                        public_identity: spec::OperatorPublicIdentity::new(format!(
                            "operator-certify-public-{byte:02x}"
                        ))
                        .expect("operator public identity"),
                    }],
                },
                quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
            },
        }
    }

    fn register_manual_authority(
        registry: &mut CertificationRegistry,
        manual: &spec::ManualResolutionEvidenceSpec,
    ) {
        registry
            .register_schema_role(
                manual.evidence_schema.clone(),
                CertifiedSchemaRole::ManualResolutionEvidence,
            )
            .expect("register schema role");
        registry
            .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
            .expect("register verifier");
        registry
            .register_operator_authority_snapshot(manual.authorization.authority.clone())
            .expect("register authority");
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(byte))
    }

    fn retarget_first_remediation_input(typed: &mut spec::TypedExecutionSpec, cell_id: CellId) {
        let input_cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == cell_id)
            .expect("input cell")
            .clone();
        let root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: spec::PublicFieldPath::new("input").expect("field path"),
            cell_id: cell_id.clone(),
            semantic_type_id: input_cell.semantic_type_id,
            schema_id: input_cell.schema_id,
            required_terminal: spec::RequiredTerminal::ProducedOnly,
            value_lineage: input_cell.value_lineage,
        }));
        let digest = content_digest_json(input_node_json(&root)).expect("input digest");
        let predecessors = predecessors_for_test_inputs(typed, std::slice::from_ref(&cell_id));
        let original_output_cell = typed
            .remediations
            .values()
            .next()
            .expect("remediation")
            .output_cell
            .clone();
        let original_output = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == original_output_cell)
            .expect("remediation output cell")
            .clone();
        let (
            old_node_id,
            old_output_cell,
            node_id,
            output_cell,
            scope_id,
            planning_lineage,
            config_digest,
        ) = {
            let remediation = typed.remediations.values_mut().next().expect("remediation");
            let old_node_id = remediation.node_id.clone();
            remediation.input_bindings.root = root;
            remediation.input_bindings.digest = digest;
            remediation.deterministic_predecessors = predecessors;
            let config_digest = config_ref_digest(&remediation.config_ref).expect("config digest");
            remediation.node_id =
                state_node_id_from_spec(remediation, &config_digest).expect("remediation node id");
            remediation.output_cell = cell_id_from_parts(
                &remediation.scope_id,
                &spec::CellProducer::Node(remediation.node_id.clone()),
                &original_output.semantic_type_id,
                &original_output.schema_id,
            )
            .expect("remediation output cell");
            (
                old_node_id,
                original_output_cell,
                remediation.node_id.clone(),
                remediation.output_cell.clone(),
                remediation.scope_id.clone(),
                remediation.planning_lineage.clone(),
                config_digest,
            )
        };
        let lineage = render_value_lineage_ref(
            &scope_id,
            &node_id,
            std::slice::from_ref(&cell_id),
            &planning_lineage,
            &config_digest,
        )
        .expect("lineage");
        typed
            .cells
            .iter_mut()
            .find(|cell| cell.cell_id == old_output_cell)
            .expect("remediation output cell")
            .cell_id = output_cell.clone();
        let output = typed
            .cells
            .iter_mut()
            .find(|cell| cell.cell_id == output_cell)
            .expect("retargeted remediation output cell");
        output.producer = spec::CellProducer::Node(node_id.clone());
        output.value_lineage = lineage.clone();
        let value_lineage = typed
            .value_lineages
            .iter_mut()
            .find(|lineage| lineage.producer == spec::CellProducer::Node(old_node_id.clone()))
            .expect("remediation value lineage");
        value_lineage.producer = spec::CellProducer::Node(node_id);
        value_lineage.lineage_ref = lineage;
        value_lineage.input_cells = vec![cell_id];
        value_lineage.config_ref_digest = Some(config_digest);
        value_lineage.planning_lineage = planning_lineage;
    }

    fn lifecycle_render_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
        typed
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                )
            })
            .expect("render node")
            .output_cell
            .clone()
    }

    fn lifecycle_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
        typed
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
                )
            })
            .expect("retention node")
            .output_cell
            .clone()
    }

    trait LifecycleVariant {
        fn matches(framework: &spec::FrameworkNodeSpec) -> bool;
    }

    impl LifecycleVariant for spec::ProjectRetentionManifestNodeSpec {
        fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
            matches!(
                framework,
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
            )
        }
    }

    impl LifecycleVariant for spec::CompleteRunNodeSpec {
        fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
            matches!(framework, spec::FrameworkNodeSpec::CompleteRun(_))
        }
    }

    impl LifecycleVariant for spec::ResolveSagaTerminalNodeSpec {
        fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
            matches!(framework, spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        }
    }

    impl LifecycleVariant for spec::PublicOutputRenderNodeSpec {
        fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
            matches!(framework, spec::FrameworkNodeSpec::PublicOutputRender(_))
        }
    }

    fn find_lifecycle_node_mut<T: LifecycleVariant>(
        typed: &mut spec::TypedExecutionSpec,
    ) -> &mut spec::NodeSpec {
        typed
            .nodes
            .iter_mut()
            .find(|node| node.framework.as_ref().is_some_and(T::matches))
            .expect("lifecycle node")
    }

    fn clear_lifecycle_framework_metadata<T: LifecycleVariant>(
        typed: &mut spec::TypedExecutionSpec,
    ) {
        find_lifecycle_node_mut::<T>(typed).framework = None;
    }

    fn append_retention_lifecycle_node(
        typed: &mut spec::TypedExecutionSpec,
        public_output_receipt_cell: CellId,
    ) -> CellId {
        let scope_id = typed.scopes[0].scope_id.clone();
        let stable_key = spec::StableAuthorKey::new("framework/project-retention-manifest")
            .expect("retention stable key");
        let retention = spec::ProjectRetentionManifestNodeSpec {
            public_schema_id: typed.public_outputs.public_schema_id.clone(),
            public_output_receipt_cell: public_output_receipt_cell.clone(),
        };
        let node_id = project_retention_manifest_node_id_from_spec(
            &scope_id,
            stable_key.as_str(),
            &retention,
        )
        .expect("retention node id");
        let receipt_schema =
            spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
        let receipt_semantic = spec::retention_manifest_receipt_semantic_type_id()
            .expect("retention receipt semantic");
        let output_cell =
            framework_cell_id(&scope_id, &node_id, &receipt_semantic, &receipt_schema)
                .expect("retention output cell");
        let config_ref =
            framework_config_ref("project_retention_manifest", &node_id).expect("config ref");
        let config_digest = config_ref_digest(&config_ref).expect("config digest");
        let input_cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == public_output_receipt_cell)
            .expect("public-output receipt cell");
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "project_retention_manifest",
            "public_output_receipt",
            input_cell,
        )
        .expect("input binding");
        let descriptor = framework_project_retention_manifest_descriptor(
            &receipt_schema,
            &receipt_semantic,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )
        .expect("retention descriptor");
        let planning_lineage = typed.scopes[0].planning_lineage.clone();
        let lineage = render_value_lineage_ref(
            &scope_id,
            &node_id,
            std::slice::from_ref(&public_output_receipt_cell),
            &planning_lineage,
            &config_digest,
        )
        .expect("retention lineage");

        push_lifecycle_node(LifecycleNodeParts {
            typed,
            node_id,
            stable_key,
            scope_id,
            descriptor,
            config_ref,
            input_binding,
            output_cell: output_cell.clone(),
            receipt_schema,
            receipt_semantic,
            framework: spec::FrameworkNodeSpec::ProjectRetentionManifest(retention),
            planning_lineage,
            lineage,
            input_cells: vec![public_output_receipt_cell],
        });
        output_cell
    }

    struct LifecycleNodeParts<'a> {
        typed: &'a mut spec::TypedExecutionSpec,
        node_id: NodeId,
        stable_key: spec::StableAuthorKey,
        scope_id: ScopeId,
        descriptor: spec::StateDescriptorIdentity,
        config_ref: spec::ConfigRef,
        input_binding: spec::InputBindingSpec,
        output_cell: CellId,
        receipt_schema: SchemaId,
        receipt_semantic: SemanticTypeId,
        framework: spec::FrameworkNodeSpec,
        planning_lineage: spec::PlanningLineage,
        lineage: spec::ValueLineageRef,
        input_cells: Vec<CellId>,
    }

    fn push_lifecycle_node(parts: LifecycleNodeParts<'_>) {
        let LifecycleNodeParts {
            typed,
            node_id,
            stable_key,
            scope_id,
            descriptor,
            config_ref,
            input_binding,
            output_cell,
            receipt_schema,
            receipt_semantic,
            framework,
            planning_lineage,
            lineage,
            input_cells,
        } = parts;
        let predecessors = predecessors_for_test_inputs(typed, &input_cells);
        typed.config_refs.push(config_ref.clone());
        typed
            .descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(
                descriptor.clone(),
            )));
        typed.cells.push(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: scope_id.clone(),
            semantic_type_id: receipt_semantic,
            schema_id: receipt_schema,
            value_lineage: lineage.clone(),
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        typed.value_lineages.push(spec::ValueLineage {
            lineage_ref: lineage,
            scope_id: scope_id.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells,
            config_ref_digest: Some(config_ref_digest(&config_ref).expect("config digest")),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        });
        typed.nodes.push(spec::NodeSpec {
            node_id,
            stable_key,
            scope_id,
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell,
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(framework),
            planning_lineage,
            deterministic_predecessors: predecessors,
        });
    }

    fn predecessors_for_test_inputs(
        typed: &spec::TypedExecutionSpec,
        input_cells: &[CellId],
    ) -> Vec<NodeId> {
        let mut predecessors = BTreeSet::new();
        for input_cell in input_cells {
            let cell = typed
                .cells
                .iter()
                .find(|cell| cell.cell_id == *input_cell)
                .expect("input cell");
            if let spec::CellProducer::Node(node_id) = &cell.producer {
                predecessors.insert(node_id.clone());
            }
        }
        predecessors.into_iter().collect()
    }

    fn append_user_receipt_consumer(
        typed: &mut spec::TypedExecutionSpec,
        receipt_cell: CellId,
        stable_key: &str,
    ) -> NodeId {
        let template = typed
            .nodes
            .iter()
            .find(|node| node.framework.is_none())
            .expect("user node template")
            .clone();
        let descriptor = typed
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state)
                    if state.descriptor_id == template.descriptor_id =>
                {
                    Some(state.as_ref().clone())
                }
                _ => None,
            })
            .expect("template descriptor");
        let input_cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == receipt_cell)
            .expect("receipt cell")
            .clone();
        let root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: spec::PublicFieldPath::new("receipt").expect("field path"),
            cell_id: receipt_cell.clone(),
            semantic_type_id: input_cell.semantic_type_id.clone(),
            schema_id: input_cell.schema_id.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
            value_lineage: input_cell.value_lineage.clone(),
        }));
        let input_binding = spec::InputBindingSpec {
            input_schema_id: descriptor.input_schema_id.clone(),
            input_descriptor_id: descriptor_id_json(serde_json::json!({
                "input": "framework_receipt_consumer",
                "receipt_cell": receipt_cell.as_str(),
                "stable_key": stable_key,
            }))
            .expect("input descriptor"),
            digest: content_digest_json(input_node_json(&root)).expect("input digest"),
            root,
        };
        let config_digest = config_ref_digest(&template.config_ref).expect("config digest");
        let mut node = spec::NodeSpec {
            node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xf0)),
            stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
            scope_id: template.scope_id.clone(),
            state_kind: descriptor.state_kind.clone(),
            state_version: descriptor.state_version.clone(),
            descriptor_id: descriptor.descriptor_id.clone(),
            config_ref: template.config_ref.clone(),
            input_bindings: input_binding,
            output_cell: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xf1)),
            effect_kind: descriptor.effect_kind.clone(),
            capability_bindings: descriptor.capabilities.clone(),
            adapter_bindings: template.adapter_bindings.clone(),
            side_effect: None,
            framework: None,
            planning_lineage: template.planning_lineage.clone(),
            deterministic_predecessors: predecessors_for_test_inputs(
                typed,
                std::slice::from_ref(&receipt_cell),
            ),
        };
        node.node_id = state_node_id_from_spec(&node, &config_digest).expect("consumer node id");
        node.output_cell = cell_id_from_parts(
            &node.scope_id,
            &spec::CellProducer::Node(node.node_id.clone()),
            &descriptor.output_semantic_type_id,
            &descriptor.output_schema_id,
        )
        .expect("consumer output cell");
        let lineage = render_value_lineage_ref(
            &node.scope_id,
            &node.node_id,
            std::slice::from_ref(&receipt_cell),
            &node.planning_lineage,
            &config_digest,
        )
        .expect("consumer lineage");
        typed.cells.push(spec::CellSpec {
            cell_id: node.output_cell.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            scope_id: node.scope_id.clone(),
            semantic_type_id: descriptor.output_semantic_type_id,
            schema_id: descriptor.output_schema_id,
            value_lineage: lineage.clone(),
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        typed.value_lineages.push(spec::ValueLineage {
            lineage_ref: lineage,
            scope_id: node.scope_id.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            input_cells: vec![receipt_cell],
            config_ref_digest: Some(config_digest),
            planning_lineage: node.planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        });
        let node_id = node.node_id.clone();
        typed.nodes.push(node);
        node_id
    }

    fn append_independent_user_node(
        typed: &mut spec::TypedExecutionSpec,
        stable_key: &str,
    ) -> NodeId {
        let template = typed
            .nodes
            .iter()
            .find(|node| node.framework.is_none())
            .expect("user node template")
            .clone();
        let descriptor = typed
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state)
                    if state.descriptor_id == template.descriptor_id =>
                {
                    Some(state.as_ref().clone())
                }
                _ => None,
            })
            .expect("template descriptor");
        let input_cells = collect_input_cells(&template.input_bindings.root);
        let config_digest = config_ref_digest(&template.config_ref).expect("config digest");
        let mut node = spec::NodeSpec {
            node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xe0)),
            stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
            scope_id: template.scope_id.clone(),
            state_kind: descriptor.state_kind.clone(),
            state_version: descriptor.state_version.clone(),
            descriptor_id: descriptor.descriptor_id.clone(),
            config_ref: template.config_ref.clone(),
            input_bindings: template.input_bindings.clone(),
            output_cell: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xe1)),
            effect_kind: descriptor.effect_kind.clone(),
            capability_bindings: descriptor.capabilities.clone(),
            adapter_bindings: template.adapter_bindings.clone(),
            side_effect: template.side_effect.clone(),
            framework: None,
            planning_lineage: template.planning_lineage.clone(),
            deterministic_predecessors: predecessors_for_test_inputs(typed, &input_cells),
        };
        node.node_id = state_node_id_from_spec(&node, &config_digest).expect("node id");
        node.output_cell = cell_id_from_parts(
            &node.scope_id,
            &spec::CellProducer::Node(node.node_id.clone()),
            &descriptor.output_semantic_type_id,
            &descriptor.output_schema_id,
        )
        .expect("output cell");
        let lineage = render_value_lineage_ref(
            &node.scope_id,
            &node.node_id,
            &input_cells,
            &node.planning_lineage,
            &config_digest,
        )
        .expect("lineage");
        typed.cells.push(spec::CellSpec {
            cell_id: node.output_cell.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            scope_id: node.scope_id.clone(),
            semantic_type_id: descriptor.output_semantic_type_id,
            schema_id: descriptor.output_schema_id,
            value_lineage: lineage.clone(),
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        typed.value_lineages.push(spec::ValueLineage {
            lineage_ref: lineage,
            scope_id: node.scope_id.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            input_cells,
            config_ref_digest: Some(config_digest),
            planning_lineage: node.planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        });
        let node_id = node.node_id.clone();
        typed.nodes.push(node);
        node_id
    }

    fn digest_byte(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }

    fn reference_persisted_spec_certificate_parts() -> (
        CertificationRegistry,
        CertifiedTypedSpec,
        PersistedSpecCertificateParts,
    ) {
        let draft = reference_draft();
        let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
        let certified = certify_program_draft(&draft).expect("certified");
        assert_eq!(
            certified.certificate().evidence.registry_digest,
            registry.digest().expect("registry digest")
        );
        let persisted_parts = certified
            .to_persisted_parts()
            .expect("persisted spec/certificate parts");
        (registry, certified, persisted_parts)
    }

    fn certificate_bytes(certificate: &CertifiedSpecCertificate) -> Vec<u8> {
        certificate
            .canonical_json()
            .expect("certificate canonical json")
            .to_vec()
    }

    #[test]
    fn certification_summary_keys_are_stable() {
        let keys = [
            ProblemClass::InvalidTopology.summary_key(),
            ProblemClass::InvalidInterfaceWiring.summary_key(),
            ProblemClass::InvalidSemanticTransition.summary_key(),
            ProblemClass::InvalidDataShape.summary_key(),
            ProblemClass::InvalidDataMeaning.summary_key(),
            ProblemClass::InvalidTerminalShape.summary_key(),
        ];
        assert_eq!(
            keys,
            [
                "invalid_topology_rejected",
                "invalid_interface_wiring_rejected",
                "invalid_semantic_transition_rejected",
                "invalid_data_shape_rejected",
                "invalid_data_meaning_rejected",
                "invalid_terminal_shape_rejected"
            ]
        );
    }
}
