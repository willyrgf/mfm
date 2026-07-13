use super::*;

/// Typed node spec for state and framework nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSpec {
    /// Node id.
    pub node_id: NodeId,
    /// Stable node key.
    pub stable_key: StableAuthorKey,
    /// Owning scope id.
    pub scope_id: ScopeId,
    /// State kind or framework state kind.
    pub state_kind: StateKind,
    /// State version or framework state version.
    pub state_version: StateVersion,
    /// Descriptor id.
    pub descriptor_id: DescriptorId,
    /// Certified transition-context requirement.
    pub context: NodeContextSpec,
    /// Config reference.
    pub config_ref: ConfigRef,
    /// Input bindings.
    pub input_bindings: InputBindingSpec,
    /// Output cell id.
    pub output_cell: CellId,
    /// Effect kind.
    pub effect_kind: EffectKind,
    /// Capability bindings.
    pub capability_bindings: CapabilitySetDescriptor,
    /// Adapter bindings.
    pub adapter_bindings: Vec<AdapterBinding>,
    /// Fact descriptors this producing node may emit.
    pub fact_descriptor_allowlist: Vec<FactDescriptorRef>,
    /// Side-effect contract, when applicable.
    pub side_effect: Option<SideEffectContractSpec>,
    /// Framework node metadata, when framework-owned.
    pub framework: Option<FrameworkNodeSpec>,
    /// Planning lineage active for this node.
    pub planning_lineage: PlanningLineage,
    /// Deterministic predecessor node ids.
    pub deterministic_predecessors: Vec<NodeId>,
}

impl NodeSpec {
    pub(super) fn json(&self, descriptors: &DescriptorJsonIndex) -> Result<serde_json::Value> {
        let descriptor_ref = descriptors.require(&self.descriptor_id, DescriptorFamily::State)?;
        Ok(serde_json::json!({
            "adapter_bindings": self.adapter_bindings.iter().map(AdapterBinding::json).collect::<Vec<_>>(),
            "config_ref": self.config_ref.json(),
            "context": self.context.json(),
            "descriptor_ref": descriptor_ref.json(),
            "deterministic_predecessors": self.deterministic_predecessors
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            "fact_descriptor_allowlist": self.fact_descriptor_allowlist
                .iter()
                .map(FactDescriptorRef::json)
                .collect::<Vec<_>>(),
            "framework": self.framework.as_ref().map(FrameworkNodeSpec::json).transpose()?,
            "input_bindings": self.input_bindings.json(),
            "node_id": self.node_id.as_str(),
            "output_cell": self.output_cell.as_str(),
            "planning_lineage": self.planning_lineage.json(),
            "scope_id": self.scope_id.as_str(),
            "side_effect": self.side_effect.as_ref().map(SideEffectContractSpec::json),
            "stable_key": self.stable_key.as_str(),
        }))
    }
}

/// Adapter binding persisted for behaviorally relevant adapters/connectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterBinding {
    /// Adapter kind.
    pub adapter_kind: AdapterKind,
    /// Adapter version.
    pub adapter_version: AdapterVersion,
    /// Optional binding digest.
    pub binding_digest: Option<ContentDigest>,
}

impl AdapterBinding {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "adapter_kind": self.adapter_kind.as_str(),
            "adapter_version": self.adapter_version.as_str(),
            "binding_digest": self.binding_digest.as_ref().map(ContentDigest::as_str),
        })
    }
}

/// Resource claim declared by a side-effect node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceClaimSpec {
    /// The adapter records a concrete exclusive key before crossing the uncertainty boundary.
    Exclusive {
        /// Resource namespace for the exclusive lane.
        namespace: ResourceNamespace,
        /// Schema id for the adapter-recorded key evidence.
        key_schema: SchemaId,
    },
    /// The adapter records the exact touched key set as typed post-execution evidence.
    ExactTouchedSet {
        /// Resource namespace for the touched set evidence.
        namespace: ResourceNamespace,
        /// Schema id for the touched set evidence.
        evidence_schema: SchemaId,
    },
    /// No framework-derived cross-run concurrency claim is made.
    ManualOnly,
}

impl ResourceClaimSpec {
    fn json(&self) -> serde_json::Value {
        match self {
            Self::Exclusive {
                namespace,
                key_schema,
            } => serde_json::json!({
                "kind": "exclusive",
                "exclusive": {
                    "key_schema": key_schema.as_str(),
                    "namespace": namespace.as_str(),
                },
            }),
            Self::ExactTouchedSet {
                namespace,
                evidence_schema,
            } => serde_json::json!({
                "kind": "exact_touched_set",
                "exact_touched_set": {
                    "evidence_schema": evidence_schema.as_str(),
                    "namespace": namespace.as_str(),
                },
            }),
            Self::ManualOnly => serde_json::json!({
                "kind": "manual_only",
            }),
        }
    }
}

/// Certified side-effect terminal verification policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideEffectVerificationSpec {
    /// Receipt evidence is terminal. This is final-at-risk if the external chain can reorg.
    Receipt,
    /// Confirmation/finality evidence at a certified positive depth is terminal.
    Finalized {
        /// Required confirmation/finality depth.
        depth: u64,
    },
}

impl SideEffectVerificationSpec {
    fn json(&self) -> serde_json::Value {
        match self {
            Self::Receipt => serde_json::json!({
                "kind": "receipt",
            }),
            Self::Finalized { depth } => serde_json::json!({
                "finalized": {
                    "depth": depth,
                },
                "kind": "finalized",
            }),
        }
    }
}

/// Side-effect contract persisted in node specs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectContractSpec {
    /// Side-effect contract digest.
    pub contract_digest: ContentDigest,
    /// Mandatory cross-run resource claim declaration.
    pub resource_claim: ResourceClaimSpec,
    /// Certified terminal verification policy.
    pub verification: SideEffectVerificationSpec,
}

impl SideEffectContractSpec {
    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "contract_digest": self.contract_digest.as_str(),
            "resource_claim": self.resource_claim.json(),
            "verification": self.verification.json(),
        })
    }
}

/// Framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameworkNodeSpec {
    /// Same-value bridge framework node.
    Bridge(BridgeNodeSpec),
    /// Side-effect verification framework node.
    SideEffectVerify(SideEffectVerifyNodeSpec),
    /// Public-output render framework node.
    PublicOutputRender(PublicOutputRenderNodeSpec),
    /// Retention-manifest projection lifecycle framework node.
    ProjectRetentionManifest(ProjectRetentionManifestNodeSpec),
    /// Complete-run lifecycle framework node.
    CompleteRun(CompleteRunNodeSpec),
    /// Saga-terminal resolution lifecycle framework node.
    ResolveSagaTerminal(ResolveSagaTerminalNodeSpec),
}

impl FrameworkNodeSpec {
    /// Returns the deterministic framework config kind persisted for this node.
    pub fn config_kind(&self) -> &'static str {
        match self {
            Self::Bridge(_) => "bridge_same_value",
            Self::SideEffectVerify(_) => "side_effect_verify",
            Self::PublicOutputRender(_) => "public_output_render",
            Self::ProjectRetentionManifest(_) => "project_retention_manifest",
            Self::CompleteRun(_) => "complete_run",
            Self::ResolveSagaTerminal(_) => "resolve_saga_terminal",
        }
    }

    fn json(&self) -> Result<serde_json::Value> {
        Ok(match self {
            Self::Bridge(spec) => serde_json::json!({
                "bridge": spec.json(),
                "kind": "bridge",
            }),
            Self::SideEffectVerify(spec) => serde_json::json!({
                "kind": "side_effect_verify",
                "side_effect_verify": spec.json(),
            }),
            Self::PublicOutputRender(spec) => serde_json::json!({
                "kind": "public_output_render",
                "public_output_render": spec.json()?,
            }),
            Self::ProjectRetentionManifest(spec) => serde_json::json!({
                "kind": "project_retention_manifest",
                "project_retention_manifest": spec.json(),
            }),
            Self::CompleteRun(spec) => serde_json::json!({
                "complete_run": spec.json(),
                "kind": "complete_run",
            }),
            Self::ResolveSagaTerminal(spec) => serde_json::json!({
                "kind": "resolve_saga_terminal",
                "resolve_saga_terminal": spec.json(),
            }),
        })
    }
}

/// Side-effect verification framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectVerifyNodeSpec {
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Submit node that owns the external mutation boundary.
    pub submit_node_id: NodeId,
}

impl SideEffectVerifyNodeSpec {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "pair_id": self.pair_id.as_str(),
            "submit_node_id": self.submit_node_id.as_str(),
        })
    }
}

/// Resolved side-effect submit/verify pair authority derived from a typed spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectVerifyPairRef<'a> {
    /// Certified side-effect pair id.
    pub pair_id: &'a SideEffectPairId,
    /// Framework verify node.
    pub verify_node: &'a NodeSpec,
    /// Framework verify metadata.
    pub verify: &'a SideEffectVerifyNodeSpec,
    /// Submit node that owns the external mutation boundary.
    pub submit_node: &'a NodeSpec,
    /// Submit node side-effect contract.
    pub submit_contract: &'a SideEffectContractSpec,
    /// Submit node internal output cell used as the structural anchor.
    pub submit_output_cell: &'a CellId,
}

/// Resolves a side-effect verify pair from typed spec node collections.
pub fn resolve_side_effect_verify_pair<'a>(
    nodes: &'a [NodeSpec],
    remediations: &'a BTreeMap<NodeId, NodeSpec>,
    verify_node: &'a NodeSpec,
) -> Result<SideEffectVerifyPairRef<'a>> {
    let Some(FrameworkNodeSpec::SideEffectVerify(verify)) = &verify_node.framework else {
        return Err(side_effect_verify_pair_error(
            SideEffectVerifyPairErrorKind::NotVerifyNode,
            format!(
                "node {} is not a side-effect verify node",
                verify_node.node_id
            ),
        ));
    };
    let submit_node = nodes
        .iter()
        .chain(remediations.values())
        .find(|node| node.node_id == verify.submit_node_id)
        .ok_or_else(|| {
            side_effect_verify_pair_error(
                SideEffectVerifyPairErrorKind::MissingSubmitNode,
                format!(
                    "side-effect verify node {} references missing submit node {}",
                    verify_node.node_id, verify.submit_node_id
                ),
            )
        })?;
    if submit_node.framework.is_some() {
        return Err(side_effect_verify_pair_error(
            SideEffectVerifyPairErrorKind::FrameworkSubmitNode,
            format!(
                "side-effect verify node {} references framework submit node {}",
                verify_node.node_id, submit_node.node_id
            ),
        ));
    }
    let submit_contract = submit_node.side_effect.as_ref().ok_or_else(|| {
        side_effect_verify_pair_error(
            SideEffectVerifyPairErrorKind::NonSideEffectSubmitNode,
            format!(
                "side-effect verify node {} references non-side-effect submit node {}",
                verify_node.node_id, submit_node.node_id
            ),
        )
    })?;
    let expected_pair = side_effect_pair_id(
        &submit_node.node_id,
        &submit_node.output_cell,
        submit_contract,
    )?;
    if verify.pair_id != expected_pair {
        return Err(side_effect_verify_pair_error(
            SideEffectVerifyPairErrorKind::PairIdMismatch,
            format!(
                "side-effect verify node {} pair id is not stable-id derived",
                verify_node.node_id
            ),
        ));
    }
    Ok(SideEffectVerifyPairRef {
        pair_id: &verify.pair_id,
        verify_node,
        verify,
        submit_node,
        submit_contract,
        submit_output_cell: &submit_node.output_cell,
    })
}

fn side_effect_verify_pair_error(
    kind: SideEffectVerifyPairErrorKind,
    message: String,
) -> SpecError {
    SpecError::SideEffectVerifyPair { kind, message }
}

/// Same-value bridge framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeNodeSpec {
    /// Bridge direction.
    pub bridge_kind: BridgeKind,
    /// Source scope id.
    pub source_scope_id: ScopeId,
    /// Target scope id.
    pub target_scope_id: ScopeId,
    /// Source cell id.
    pub source_cell_id: CellId,
    /// Target cell id.
    pub target_cell_id: CellId,
    /// Bridge semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Bridge schema id.
    pub schema_id: SchemaId,
    /// Bridge policy.
    pub policy: BridgePolicy,
    /// Bridge provenance.
    pub provenance: BridgeProvenance,
}

impl BridgeNodeSpec {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "bridge_kind": self.bridge_kind.as_str(),
            "policy": self.policy.as_str(),
            "provenance": self.provenance.as_str(),
            "schema_id": self.schema_id.as_str(),
            "semantic_type_id": self.semantic_type_id.as_str(),
            "source_cell_id": self.source_cell_id.as_str(),
            "source_scope_id": self.source_scope_id.as_str(),
            "target_cell_id": self.target_cell_id.as_str(),
            "target_scope_id": self.target_scope_id.as_str(),
        })
    }
}

/// Bridge direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BridgeKind {
    /// Parent value imported into a child scope.
    ImportFromParent,
    /// Child value exported into its parent scope.
    ExportToParent,
}

impl BridgeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ImportFromParent => "import_from_parent",
            Self::ExportToParent => "export_to_parent",
        }
    }
}

/// Bridge policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BridgePolicy {
    /// Same-run same-value bridge.
    SameRunSameValue,
}

impl BridgePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::SameRunSameValue => "same_run_same_value",
        }
    }
}

/// Framework bridge provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BridgeProvenance {
    /// Framework child-scope bridge v1.
    FrameworkChildScopeV1,
}

impl BridgeProvenance {
    fn as_str(self) -> &'static str {
        match self {
            Self::FrameworkChildScopeV1 => "framework_child_scope_v1",
        }
    }
}

/// Public output render framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputRenderNodeSpec {
    /// Public output schema id.
    pub public_schema_id: SchemaId,
    /// Digest of the public output spec.
    pub output_spec_digest: ContentDigest,
    /// Renderer descriptor.
    pub renderer_descriptor: RendererDescriptorIdentity,
    /// Required public cells.
    pub required_cells: Vec<PublicOutputCell>,
}

impl PublicOutputRenderNodeSpec {
    fn json(&self) -> Result<serde_json::Value> {
        let renderer_ref = DescriptorIdentity::Renderer(Box::new(self.renderer_descriptor.clone()))
            .descriptor_ref()?;
        Ok(serde_json::json!({
            "output_spec_digest": self.output_spec_digest.as_str(),
            "public_schema_id": self.public_schema_id.as_str(),
            "renderer_descriptor_ref": renderer_ref.json(),
            "required_cells": self.required_cells.iter().map(PublicOutputCell::json).collect::<Vec<_>>(),
        }))
    }
}

/// Retention-manifest projection lifecycle framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRetentionManifestNodeSpec {
    /// Public output schema whose retained stream evidence is projected.
    pub public_schema_id: SchemaId,
    /// Public-output render receipt cell that orders this projection after rendering.
    pub public_output_receipt_cell: CellId,
}

impl ProjectRetentionManifestNodeSpec {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "public_output_receipt_cell": self.public_output_receipt_cell.as_str(),
            "public_schema_id": self.public_schema_id.as_str(),
        })
    }
}

/// Complete-run lifecycle framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteRunNodeSpec {
    /// Public output schema whose completion evidence terminates the run.
    pub public_schema_id: SchemaId,
    /// Retention-manifest projection receipt cell that orders completion after retention.
    pub retention_manifest_receipt_cell: CellId,
}

impl CompleteRunNodeSpec {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "public_schema_id": self.public_schema_id.as_str(),
            "retention_manifest_receipt_cell": self.retention_manifest_receipt_cell.as_str(),
        })
    }
}

/// Resolve-saga-terminal lifecycle framework node metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveSagaTerminalNodeSpec {
    /// Public output schema whose terminal status is resolved.
    pub public_schema_id: SchemaId,
}

impl ResolveSagaTerminalNodeSpec {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "public_schema_id": self.public_schema_id.as_str(),
        })
    }
}
