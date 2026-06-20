use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mfm_certify::{
    CertifiedDescriptorSet, CertifiedSpecCertificate, CertifiedSpecGraph, CertifiedTypedSpec,
};
use mfm_ids::{CellId, DescriptorId, NodeId, SpecHash};
#[cfg(test)]
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_spec::v1 as spec;

use crate::{Result, RuntimeError};

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
        let parts = certified.into_parts();
        let state_descriptors = certified_state_descriptors(parts.graph.descriptors());
        let _framework_lifecycle = parts.framework_lifecycle;
        Self::from_certified_graph(
            parts.envelope,
            parts.certificate,
            state_descriptors,
            parts.graph,
        )
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
                schema_role_grants: Vec::new(),
                manual_authorization_verifiers: Vec::new(),
                operator_authority_snapshots: Vec::new(),
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
        let state_descriptors = raw_state_descriptors(&envelope.spec)?;
        Self::from_verified_parts(envelope, certificate, state_descriptors)
    }

    #[cfg(test)]
    fn from_verified_parts(
        envelope: spec::HashedSpecEnvelope,
        certificate: CertifiedSpecCertificate,
        state_descriptors: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
    ) -> Result<Self> {
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

        Self::from_verified_indexes(
            envelope,
            certificate,
            state_descriptors,
            nodes,
            remediations,
            cells,
        )
    }

    fn from_certified_graph(
        envelope: spec::HashedSpecEnvelope,
        certificate: CertifiedSpecCertificate,
        state_descriptors: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
        graph: CertifiedSpecGraph,
    ) -> Result<Self> {
        let cells = graph
            .cells()
            .map(|(cell_id, cell)| (cell_id.clone(), cell.clone()))
            .collect();
        let nodes = graph
            .forward_nodes()
            .map(|node| (node.node_id.clone(), node.clone()))
            .collect();
        let remediations = graph
            .remediations()
            .map(|(forward_node_id, node)| (forward_node_id.clone(), node.clone()))
            .collect();

        Self::from_verified_indexes(
            envelope,
            certificate,
            state_descriptors,
            nodes,
            remediations,
            cells,
        )
    }

    fn from_verified_indexes(
        envelope: spec::HashedSpecEnvelope,
        certificate: CertifiedSpecCertificate,
        state_descriptors: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
        nodes: BTreeMap<NodeId, spec::NodeSpec>,
        remediations: BTreeMap<NodeId, spec::NodeSpec>,
        cells: BTreeMap<CellId, spec::CellSpec>,
    ) -> Result<Self> {
        envelope.verify_hash()?;

        let runtime = Self {
            envelope,
            certificate,
            state_descriptors,
            nodes,
            remediations,
            cells,
            topological_order: Vec::new(),
        };
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

fn certified_state_descriptors(
    descriptors: &CertifiedDescriptorSet,
) -> BTreeMap<DescriptorId, spec::StateDescriptorIdentity> {
    descriptors
        .state_descriptors()
        .map(|(descriptor_id, descriptor)| (descriptor_id.clone(), descriptor.clone()))
        .collect()
}

#[cfg(test)]
fn raw_state_descriptors(
    spec: &spec::TypedExecutionSpec,
) -> Result<BTreeMap<DescriptorId, spec::StateDescriptorIdentity>> {
    let mut state_descriptors = BTreeMap::new();
    for descriptor in &spec.descriptor_identities {
        if let spec::DescriptorIdentity::State(identity) = descriptor {
            let previous =
                state_descriptors.insert(identity.descriptor_id.clone(), identity.as_ref().clone());
            if previous.is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate state descriptor {}",
                    identity.descriptor_id
                )));
            }
        }
    }
    Ok(state_descriptors)
}
