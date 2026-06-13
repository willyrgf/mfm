use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mfm_capabilities::{EffectSpec, ManagedPlatformWrite};
use mfm_certify::{CertifiedSpecCertificate, CertifiedTypedSpec};
use mfm_ids::{CellId, DescriptorId, NodeId, SchemaId, SemanticTypeId, SpecHash};
#[cfg(test)]
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_spec::v1 as spec;

use crate::{config_ref_key, Result, RuntimeError};

/// Certified executable runtime spec with indexes used by the serial scheduler.
#[derive(Debug, Clone)]
pub struct CertifiedRuntimeSpec {
    envelope: spec::HashedSpecEnvelope,
    certificate: CertifiedSpecCertificate,
    state_descriptors: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
    nodes: BTreeMap<NodeId, spec::NodeSpec>,
    remediations: BTreeMap<NodeId, spec::NodeSpec>,
    cells: BTreeMap<CellId, spec::CellSpec>,
    topological_order: Vec<NodeId>,
}

impl CertifiedRuntimeSpec {
    /// Builds deterministic runtime indexes from certifier-backed typed-spec authority.
    pub fn new(certified: CertifiedTypedSpec) -> Result<Self> {
        let (envelope, certificate) = certified.into_parts();
        Self::from_verified_parts(envelope, certificate)
    }

    #[cfg(test)]
    pub(crate) fn from_verified_envelope(envelope: spec::HashedSpecEnvelope) -> Result<Self> {
        let certificate = mfm_certify::CertifiedSpecCertificate::from_evidence(
            mfm_certify::CertifiedSpecCertificateEvidence {
                certificate_version: mfm_certify::CERTIFICATE_VERSION.to_owned(),
                media_type: mfm_certify::CERTIFICATE_MEDIA_TYPE.to_owned(),
                certifier_version: "mfm-runtime-test-placeholder".to_owned(),
                certifier_algorithm: "mfm-runtime-test-placeholder".to_owned(),
                certificate_canonicalization: DigestAlgorithm::Sha256JcsV1,
                spec_hash: envelope.spec_hash.clone(),
                spec_canonicalization: envelope.spec.canonicalization,
                registry_digest: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    mfm_ids::DigestBytes::from_array([0; 32]),
                ),
                descriptor_identities: Vec::new(),
                lowering_version: envelope.spec.lowering_version.clone(),
                public_output_schema_id: envelope.spec.public_outputs.public_schema_id.clone(),
                public_output_canonicalizer_identity: envelope
                    .spec
                    .public_outputs
                    .renderer_descriptor
                    .canonicalizer_identity
                    .clone(),
                audit: mfm_certify::CertifiedSpecAuditMetadata {
                    problem_classes_covered: Vec::new(),
                    scope_count: envelope.spec.scopes.len() as u64,
                    operation_lineage_count: envelope.spec.planning_lineage.len() as u64,
                    node_count: envelope.spec.nodes.len() as u64,
                    cell_count: envelope.spec.cells.len() as u64,
                    seed_count: envelope.spec.seeds.len() as u64,
                    descriptor_count: envelope.spec.descriptor_identities.len() as u64,
                },
            },
        )
        .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        Self::from_verified_parts(envelope, certificate)
    }

    fn from_verified_parts(
        envelope: spec::HashedSpecEnvelope,
        certificate: CertifiedSpecCertificate,
    ) -> Result<Self> {
        envelope.verify_hash()?;
        let mut state_descriptors = BTreeMap::new();
        for descriptor in &envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                let previous = state_descriptors
                    .insert(identity.descriptor_id.clone(), identity.as_ref().clone());
                if previous.is_some() {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "duplicate state descriptor {}",
                        identity.descriptor_id
                    )));
                }
            }
        }

        let mut cells = BTreeMap::new();
        for cell in &envelope.spec.cells {
            if cells.insert(cell.cell_id.clone(), cell.clone()).is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate cell {}",
                    cell.cell_id
                )));
            }
        }

        let mut nodes = BTreeMap::new();
        for node in &envelope.spec.nodes {
            if nodes.insert(node.node_id.clone(), node.clone()).is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate node {}",
                    node.node_id
                )));
            }
        }
        let remediations = envelope.spec.remediations.clone();

        let runtime = Self {
            envelope,
            certificate,
            state_descriptors,
            nodes,
            remediations,
            cells,
            topological_order: Vec::new(),
        };
        runtime.validate_runtime_contract()?;
        let topological_order = runtime.compute_topological_order()?;
        Ok(Self {
            topological_order,
            ..runtime
        })
    }

    /// Returns the hash-only spec envelope.
    pub fn envelope(&self) -> &spec::HashedSpecEnvelope {
        &self.envelope
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.envelope.spec_hash
    }

    /// Returns the verified certificate evidence used to mint this runtime spec.
    pub fn certificate(&self) -> &CertifiedSpecCertificate {
        &self.certificate
    }

    /// Returns the hash-defining typed execution spec.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        &self.envelope.spec
    }

    /// Returns deterministic topological node ids.
    pub fn topological_order(&self) -> &[NodeId] {
        &self.topological_order
    }

    /// Returns a certified node by id.
    pub fn node(&self, node_id: &NodeId) -> Option<&spec::NodeSpec> {
        self.nodes.get(node_id).or_else(|| {
            self.remediations
                .values()
                .find(|node| node.node_id == *node_id)
        })
    }

    /// Returns the remediation node linked to a forward side-effect node.
    pub(crate) fn remediation_for_forward_node(
        &self,
        forward_node_id: &NodeId,
    ) -> Option<&spec::NodeSpec> {
        self.remediations.get(forward_node_id)
    }

    /// Returns the forward node id linked to a remediation node.
    pub(crate) fn forward_node_for_remediation(
        &self,
        remediation_node_id: &NodeId,
    ) -> Option<&NodeId> {
        self.remediations
            .iter()
            .find_map(|(forward_node_id, remediation)| {
                (remediation.node_id == *remediation_node_id).then_some(forward_node_id)
            })
    }

    /// Iterates certified remediation nodes keyed by their forward side-effect node id.
    pub(crate) fn remediations(&self) -> impl Iterator<Item = (&NodeId, &spec::NodeSpec)> {
        self.remediations.iter()
    }

    /// Returns a certified cell by id.
    pub fn cell(&self, cell_id: &CellId) -> Option<&spec::CellSpec> {
        self.cells.get(cell_id)
    }

    /// Returns the certified state descriptor for a node.
    pub fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&spec::StateDescriptorIdentity> {
        self.state_descriptors
            .get(&node.descriptor_id)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "node {} references missing state descriptor {}",
                    node.node_id, node.descriptor_id
                ))
            })
    }

    fn validate_runtime_contract(&self) -> Result<()> {
        let config_refs = self
            .spec()
            .config_refs
            .iter()
            .map(config_ref_key)
            .collect::<BTreeSet<_>>();

        for seed in &self.spec().seeds {
            let cell = self.cells.get(&seed.cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "seed {} references missing cell {}",
                    seed.seed_id, seed.cell_id
                ))
            })?;
            if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
                || cell.scope_id != seed.scope_id
                || cell.schema_id != seed.schema_id
                || cell.semantic_type_id != seed.semantic_type_id
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "seed {} cell metadata does not match certified cell {}",
                    seed.seed_id, seed.cell_id
                )));
            }
        }

        for node in self.nodes.values() {
            let descriptor = self.state_descriptor_for_node(node)?;
            self.validate_framework_descriptor_variant(node, descriptor)?;
            if descriptor.state_kind != node.state_kind
                || descriptor.state_version != node.state_version
                || descriptor.config_schema_id != node.config_ref.schema_id
                || descriptor.input_schema_id != node.input_bindings.input_schema_id
                || descriptor.output_schema_id
                    != self
                        .cells
                        .get(&node.output_cell)
                        .map(|cell| cell.schema_id.clone())
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "node {} output cell {} is missing",
                                node.node_id, node.output_cell
                            ))
                        })?
                || descriptor.output_semantic_type_id
                    != self
                        .cells
                        .get(&node.output_cell)
                        .map(|cell| cell.semantic_type_id.clone())
                        .expect("checked above")
                || descriptor.effect_kind != node.effect_kind
                || descriptor.capabilities != node.capability_bindings
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} does not match certified state descriptor {}",
                    node.node_id, node.descriptor_id
                )));
            }
            match (&node.side_effect, &descriptor.side_effect_contract_digest) {
                (Some(contract), Some(descriptor_digest))
                    if descriptor_digest == &contract.contract_digest => {}
                (Some(_), _) => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "side-effect node {} lacks matching descriptor side-effect contract",
                        node.node_id
                    )));
                }
                (None, Some(_)) => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "non-side-effect node {} references a descriptor with side-effect contract",
                        node.node_id
                    )));
                }
                (None, None) => {}
            }

            if !config_refs.contains(&config_ref_key(&node.config_ref)) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} references missing config artifact {}",
                    node.node_id, node.config_ref.artifact_id
                )));
            }

            let output = self.cells.get(&node.output_cell).expect("checked above");
            if output.producer != spec::CellProducer::Node(node.node_id.clone()) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} output cell {} has mismatched producer",
                    node.node_id, node.output_cell
                )));
            }

            let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
            let expected_predecessors = self.predecessors_for_input_cells(&input_cells)?;
            if node.deterministic_predecessors != expected_predecessors {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} deterministic predecessors do not match input cells",
                    node.node_id
                )));
            }
        }

        for output in &self.spec().public_outputs.outputs {
            let cell = self.cells.get(&output.cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "public output {} references missing cell {}",
                    output.public_field_path.as_str(),
                    output.cell_id
                ))
            })?;
            if cell.producer != output.producer
                || cell.scope_id != output.scope_id
                || cell.semantic_type_id != output.semantic_type_id
                || cell.schema_id != output.schema_id
                || cell.value_lineage != output.value_lineage
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "public output {} does not match certified cell {}",
                    output.public_field_path.as_str(),
                    output.cell_id
                )));
            }
        }
        self.validate_public_output_render_contract()?;
        self.validate_lifecycle_framework_contract()?;

        Ok(())
    }

    fn validate_public_output_render_contract(&self) -> Result<()> {
        let public_outputs = &self.spec().public_outputs;
        let output_spec_digest = public_outputs.digest()?;
        let render_nodes = self
            .nodes
            .values()
            .filter_map(|node| match &node.framework {
                Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Some((node, render)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if render_nodes.len() != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one public-output render node, found {}",
                render_nodes.len()
            )));
        }
        let (node, render) = render_nodes[0];
        if render.public_schema_id != public_outputs.public_schema_id
            || render.output_spec_digest != output_spec_digest
            || render.renderer_descriptor != public_outputs.renderer_descriptor
            || render.required_cells != public_outputs.outputs
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} does not match certified public outputs",
                node.node_id
            )));
        }
        let descriptor = self.state_descriptor_for_node(node)?;
        let managed_effect = ManagedPlatformWrite::descriptor()
            .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        if descriptor.name != "mfm.framework.render_public_outputs"
            || descriptor.runner != "managed_platform_write"
            || descriptor.effect_kind != managed_effect.kind
            || descriptor.effect_class != managed_effect.class.as_str()
            || descriptor.effect_name != managed_effect.name
            || !descriptor.capabilities.capabilities.is_empty()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} is not bound to the framework renderer",
                node.node_id
            )));
        }
        let output_cell = self.cells.get(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "public-output render node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let receipt_schema_id = spec::public_output_receipt_schema_id()?;
        if output_cell.schema_id != receipt_schema_id
            || output_cell.terminal_policy != spec::CellTerminalPolicy::ProducedOnly
            || output_cell.storage_policy != spec::StoragePolicy::PublicOutputArtifact
            || output_cell.redaction_policy != spec::RedactionPolicy::Public
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} output cell is not a public-output receipt",
                node.node_id
            )));
        }
        let input_cells = render
            .required_cells
            .iter()
            .map(|cell| cell.cell_id.clone())
            .collect::<Vec<_>>();
        let expected_predecessors = self.predecessors_for_input_cells(&input_cells)?;
        if node.deterministic_predecessors != expected_predecessors {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} predecessors do not match public cells",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_lifecycle_framework_contract(&self) -> Result<()> {
        let mut bootstrap_count = 0_usize;
        let mut retention_count = 0_usize;
        let mut completion_count = 0_usize;
        let mut lifecycle_outputs = BTreeMap::<CellId, &spec::NodeSpec>::new();
        let mut input_consumers = BTreeMap::<CellId, Vec<NodeId>>::new();
        for node in self.nodes.values() {
            for cell_id in self.validate_input_binding(&node.input_bindings.root)? {
                input_consumers
                    .entry(cell_id)
                    .or_default()
                    .push(node.node_id.clone());
            }
            if matches!(
                &node.framework,
                Some(
                    spec::FrameworkNodeSpec::BootstrapRun(_)
                        | spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                        | spec::FrameworkNodeSpec::CompleteRun(_)
                        | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
                )
            ) {
                lifecycle_outputs.insert(node.output_cell.clone(), node);
            }
        }

        for node in self.nodes.values() {
            if let Some(framework) = &node.framework {
                self.validate_framework_permissions(node)?;
                self.validate_framework_config_ref(node, framework.config_kind())?;
            }
            match &node.framework {
                Some(spec::FrameworkNodeSpec::BootstrapRun(_)) => {
                    bootstrap_count += 1;
                    self.validate_framework_descriptor(node, "mfm.framework.bootstrap_run")?;
                    self.validate_framework_output_cell(
                        node,
                        &spec::bootstrap_run_receipt_schema_id()?,
                        &spec::bootstrap_run_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_unit_input_binding("bootstrap_run")?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if !input_cells.is_empty() || !node.deterministic_predecessors.is_empty() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "bootstrap lifecycle node {} must not have inputs",
                            node.node_id
                        )));
                    }
                    self.validate_no_framework_receipt_consumers(node, &input_consumers)?;
                }
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
                    self.validate_framework_receipt_consumers(
                        node,
                        &input_consumers,
                        "project-retention-manifest lifecycle node",
                        |consumer| {
                            matches!(
                                &consumer.framework,
                                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                    if retention.public_output_receipt_cell == node.output_cell
                            )
                        },
                    )?;
                }
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                    retention_count += 1;
                    self.validate_framework_descriptor(
                        node,
                        "mfm.framework.project_retention_manifest",
                    )?;
                    if retention.public_schema_id != self.spec().public_outputs.public_schema_id {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "retention lifecycle node {} public schema mismatch",
                            node.node_id
                        )));
                    }
                    self.validate_framework_output_cell(
                        node,
                        &spec::retention_manifest_receipt_schema_id()?,
                        &spec::retention_manifest_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    let public_output_receipt_cell = self
                        .cells
                        .get(&retention.public_output_receipt_cell)
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "retention lifecycle node {} missing public-output receipt cell",
                                node.node_id
                            ))
                        })?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_receipt_input_binding(
                            "project_retention_manifest",
                            "public_output_receipt",
                            public_output_receipt_cell,
                        )?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if input_cells != vec![retention.public_output_receipt_cell.clone()] {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "retention lifecycle node {} must depend on the public-output receipt cell",
                            node.node_id
                        )));
                    }
                    let render_node = self.nodes.values().find(|candidate| {
                        candidate.output_cell == retention.public_output_receipt_cell
                            && matches!(
                                &candidate.framework,
                                Some(spec::FrameworkNodeSpec::PublicOutputRender(render))
                                    if render.public_schema_id == retention.public_schema_id
                            )
                    });
                    if render_node.is_none() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "retention lifecycle node {} is not ordered after public-output render",
                            node.node_id
                        )));
                    }
                    self.validate_framework_receipt_consumers(
                        node,
                        &input_consumers,
                        "complete-run lifecycle node",
                        |consumer| {
                            matches!(
                                &consumer.framework,
                                Some(spec::FrameworkNodeSpec::CompleteRun(complete))
                                    if complete.retention_manifest_receipt_cell == node.output_cell
                            ) || matches!(
                                &consumer.framework,
                                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve))
                                    if resolve.retention_manifest_receipt_cell == node.output_cell
                            )
                        },
                    )?;
                }
                Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => {
                    completion_count += 1;
                    self.validate_framework_descriptor(node, "mfm.framework.complete_run")?;
                    if complete.public_schema_id != self.spec().public_outputs.public_schema_id {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "completion lifecycle node {} public schema mismatch",
                            node.node_id
                        )));
                    }
                    self.validate_framework_output_cell(
                        node,
                        &spec::complete_run_receipt_schema_id()?,
                        &spec::complete_run_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    let retention_manifest_receipt_cell = self
                        .cells
                        .get(&complete.retention_manifest_receipt_cell)
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "completion lifecycle node {} missing retention receipt cell",
                                node.node_id
                            ))
                        })?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_receipt_input_binding(
                            "complete_run",
                            "retention_manifest_receipt",
                            retention_manifest_receipt_cell,
                        )?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if input_cells != vec![complete.retention_manifest_receipt_cell.clone()] {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "completion lifecycle node {} must depend on the retention receipt cell",
                            node.node_id
                        )));
                    }
                    let retention_node = lifecycle_outputs
                        .get(&complete.retention_manifest_receipt_cell)
                        .filter(|candidate| {
                            matches!(
                                &candidate.framework,
                                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                    if retention.public_schema_id == complete.public_schema_id
                            )
                        });
                    if retention_node.is_none() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "completion lifecycle node {} is not ordered after retention projection",
                            node.node_id
                        )));
                    }
                    self.validate_no_framework_receipt_consumers(node, &input_consumers)?;
                }
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve)) => {
                    self.validate_framework_descriptor(
                        node,
                        "mfm.framework.resolve_saga_terminal",
                    )?;
                    if resolve.public_schema_id != self.spec().public_outputs.public_schema_id {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "resolve-saga-terminal lifecycle node {} public schema mismatch",
                            node.node_id
                        )));
                    }
                    self.validate_framework_output_cell(
                        node,
                        &spec::resolve_saga_terminal_receipt_schema_id()?,
                        &spec::resolve_saga_terminal_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    let retention_manifest_receipt_cell = self
                        .cells
                        .get(&resolve.retention_manifest_receipt_cell)
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "resolve-saga-terminal lifecycle node {} missing retention receipt cell",
                                node.node_id
                            ))
                        })?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_receipt_input_binding(
                            "resolve_saga_terminal",
                            "retention_manifest_receipt",
                            retention_manifest_receipt_cell,
                        )?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if input_cells != vec![resolve.retention_manifest_receipt_cell.clone()] {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "resolve-saga-terminal lifecycle node {} must depend on the retention receipt cell",
                            node.node_id
                        )));
                    }
                    let retention_node = lifecycle_outputs
                        .get(&resolve.retention_manifest_receipt_cell)
                        .filter(|candidate| {
                            matches!(
                                &candidate.framework,
                                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                    if retention.public_schema_id == resolve.public_schema_id
                            )
                        });
                    if retention_node.is_none() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "resolve-saga-terminal lifecycle node {} is not ordered after retention projection",
                            node.node_id
                        )));
                    }
                    self.validate_no_framework_receipt_consumers(node, &input_consumers)?;
                }
                Some(spec::FrameworkNodeSpec::Bridge(_)) | None => {}
            }
        }

        if retention_count != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one retention lifecycle framework node, found {retention_count}"
            )));
        }
        if completion_count != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one completion lifecycle framework node, found {completion_count}"
            )));
        }
        if bootstrap_count != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one bootstrap lifecycle framework node, found {bootstrap_count}"
            )));
        }
        self.validate_lifecycle_tail_finality()?;
        Ok(())
    }

    fn validate_lifecycle_tail_finality(&self) -> Result<()> {
        let render_node = self
            .nodes
            .values()
            .find(|node| {
                matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                )
            })
            .ok_or_else(|| {
                RuntimeError::InvalidSpec("missing public-output render node".to_owned())
            })?;
        let mut render_ancestors = BTreeSet::new();
        self.collect_deterministic_ancestors(render_node, &mut render_ancestors)?;
        for node in self.nodes.values() {
            if node.node_id == render_node.node_id || render_ancestors.contains(&node.node_id) {
                continue;
            }
            match &node.framework {
                Some(
                    spec::FrameworkNodeSpec::BootstrapRun(_)
                    | spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_),
                ) => {}
                _ => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "node {} is outside the certified public-output lifecycle tail",
                        node.node_id
                    )));
                }
            }
        }
        Ok(())
    }

    fn collect_deterministic_ancestors(
        &self,
        node: &spec::NodeSpec,
        ancestors: &mut BTreeSet<NodeId>,
    ) -> Result<()> {
        for predecessor_id in &node.deterministic_predecessors {
            if ancestors.insert(predecessor_id.clone()) {
                let predecessor = self.nodes.get(predecessor_id).ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "node {} references missing predecessor {}",
                        node.node_id, predecessor_id
                    ))
                })?;
                self.collect_deterministic_ancestors(predecessor, ancestors)?;
            }
        }
        Ok(())
    }

    fn validate_framework_permissions(&self, node: &spec::NodeSpec) -> Result<()> {
        if node.side_effect.is_some()
            || !node.adapter_bindings.is_empty()
            || !node.capability_bindings.capabilities.is_empty()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} carries user-controlled execution permissions",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_config_ref(&self, node: &spec::NodeSpec, kind: &str) -> Result<()> {
        let expected = spec::framework_config_ref(kind, &node.node_id)?;
        if node.config_ref != expected {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} config ref is not deterministic for {kind}",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_input_binding(
        &self,
        node: &spec::NodeSpec,
        expected: &spec::InputBindingSpec,
    ) -> Result<()> {
        if &node.input_bindings != expected {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} input binding is not deterministic",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_descriptor_variant(
        &self,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<()> {
        let matches_variant = match descriptor.name.as_str() {
            "mfm.framework.bootstrap_run" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::BootstrapRun(_))
                )
            }
            "mfm.framework.bridge_same_value" => {
                matches!(&node.framework, Some(spec::FrameworkNodeSpec::Bridge(_)))
            }
            "mfm.framework.render_public_outputs" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                )
            }
            "mfm.framework.project_retention_manifest" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
                )
            }
            "mfm.framework.complete_run" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::CompleteRun(_))
                )
            }
            "mfm.framework.resolve_saga_terminal" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                )
            }
            _ => return Ok(()),
        };
        if !matches_variant {
            return Err(RuntimeError::InvalidSpec(format!(
                "node {} uses built-in framework descriptor {} without matching framework metadata",
                node.node_id, descriptor.name
            )));
        }
        Ok(())
    }

    fn validate_no_framework_receipt_consumers(
        &self,
        node: &spec::NodeSpec,
        consumers_by_cell: &BTreeMap<CellId, Vec<NodeId>>,
    ) -> Result<()> {
        if consumers_by_cell
            .get(&node.output_cell)
            .map(|consumers| !consumers.is_empty())
            .unwrap_or(false)
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework receipt cell {} must not be consumed",
                node.output_cell
            )));
        }
        Ok(())
    }

    fn validate_framework_receipt_consumers(
        &self,
        node: &spec::NodeSpec,
        consumers_by_cell: &BTreeMap<CellId, Vec<NodeId>>,
        expected_consumer: &str,
        mut allowed: impl FnMut(&spec::NodeSpec) -> bool,
    ) -> Result<()> {
        for consumer_id in consumers_by_cell
            .get(&node.output_cell)
            .into_iter()
            .flatten()
        {
            let consumer = self.nodes.get(consumer_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "framework receipt cell {} has missing consumer node {}",
                    node.output_cell, consumer_id
                ))
            })?;
            if !allowed(consumer) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "framework receipt cell {} may only feed {expected_consumer}",
                    node.output_cell
                )));
            }
        }
        Ok(())
    }

    fn validate_framework_descriptor(
        &self,
        node: &spec::NodeSpec,
        expected_name: &str,
    ) -> Result<()> {
        let descriptor = self.state_descriptor_for_node(node)?;
        let managed_effect = ManagedPlatformWrite::descriptor()
            .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        if descriptor.name != expected_name
            || descriptor.runner != "managed_platform_write"
            || descriptor.effect_kind != managed_effect.kind
            || descriptor.effect_class != managed_effect.class.as_str()
            || descriptor.effect_name != managed_effect.name
            || !descriptor.capabilities.capabilities.is_empty()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} is not bound to {expected_name}",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_output_cell(
        &self,
        node: &spec::NodeSpec,
        expected_schema_id: &SchemaId,
        expected_semantic_type_id: &SemanticTypeId,
        expected_storage_policy: spec::StoragePolicy,
    ) -> Result<()> {
        let output = self.cells.get(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "framework node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        if output.schema_id != *expected_schema_id
            || output.semantic_type_id != *expected_semantic_type_id
            || output.terminal_policy != spec::CellTerminalPolicy::ProducedOnly
            || output.storage_policy != expected_storage_policy
            || output.redaction_policy != spec::RedactionPolicy::Public
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} output cell contract mismatch",
                node.node_id
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_input_binding(
        &self,
        input: &spec::InputBindingNodeSpec,
    ) -> Result<Vec<CellId>> {
        let mut cells = Vec::new();
        self.validate_input_node(input, &mut cells)?;
        Ok(cells)
    }

    fn validate_input_node(
        &self,
        input: &spec::InputBindingNodeSpec,
        cells: &mut Vec<CellId>,
    ) -> Result<()> {
        match input {
            spec::InputBindingNodeSpec::Unit => {}
            spec::InputBindingNodeSpec::Cell(cell) => {
                let certified = self.cells.get(&cell.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "input binding references missing cell {}",
                        cell.cell_id
                    ))
                })?;
                if certified.semantic_type_id != cell.semantic_type_id
                    || certified.schema_id != cell.schema_id
                    || certified.value_lineage != cell.value_lineage
                {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "input binding for cell {} does not match certified cell metadata",
                        cell.cell_id
                    )));
                }
                cells.push(cell.cell_id.clone());
            }
            spec::InputBindingNodeSpec::Tuple(elements) => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Struct(fields) => {
                for field in fields {
                    self.validate_input_node(&field.node, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Vec { elements, .. }
            | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
        }
        Ok(())
    }

    fn predecessors_for_input_cells(&self, input_cells: &[CellId]) -> Result<Vec<NodeId>> {
        let mut predecessors = BTreeSet::new();
        for cell_id in input_cells {
            let cell = self.cells.get(cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!("input cell {cell_id} is missing"))
            })?;
            if let spec::CellProducer::Node(node_id) = &cell.producer {
                predecessors.insert(node_id.clone());
            }
        }
        Ok(predecessors.into_iter().collect())
    }

    fn compute_topological_order(&self) -> Result<Vec<NodeId>> {
        let mut indegree = BTreeMap::<NodeId, usize>::new();
        let mut successors = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
        for node_id in self.nodes.keys() {
            indegree.insert(node_id.clone(), 0);
            successors.insert(node_id.clone(), BTreeSet::new());
        }
        for node in self.nodes.values() {
            for predecessor in &node.deterministic_predecessors {
                if !self.nodes.contains_key(predecessor) {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "node {} references missing predecessor {}",
                        node.node_id, predecessor
                    )));
                }
                successors
                    .get_mut(predecessor)
                    .expect("predecessor exists")
                    .insert(node.node_id.clone());
                *indegree.get_mut(&node.node_id).expect("node exists") += 1;
            }
        }

        let mut ready = indegree
            .iter()
            .filter_map(|(node_id, count)| (*count == 0).then_some(node_id.clone()))
            .collect::<VecDeque<_>>();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(node_id) = ready.pop_front() {
            order.push(node_id.clone());
            for successor in successors.get(&node_id).expect("successors exist") {
                let count = indegree.get_mut(successor).expect("successor exists");
                *count -= 1;
                if *count == 0 {
                    let index = ready
                        .iter()
                        .position(|queued| successor < queued)
                        .unwrap_or(ready.len());
                    ready.insert(index, successor.clone());
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(RuntimeError::InvalidSpec(
                "certified node graph contains a cycle".to_owned(),
            ));
        }
        Ok(order)
    }
}
