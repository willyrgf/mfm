use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mfm_certify::{CertifiedSpecCertificate, CertifiedTypedSpec};
use mfm_ids::{CellId, ContentDigest, ContextRef, NodeId, SideEffectPairId, SpecHash};
use mfm_spec::v1 as spec;

use crate::{Result, RuntimeError};

/// Non-cloneable pre-admission runtime owner for one certified typed spec.
///
/// The authority moves into the store's verified run view at admission; only the disposable
/// position-and-ID index remains beside that view for post-admission runtime reads.
#[derive(Debug)]
pub struct CertifiedRuntimeSpec {
    certified: CertifiedTypedSpec,
    index: CurrentRuntimeIndex,
}

/// Position-only runtime indexes derived from one certified typed spec.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CurrentRuntimeIndex {
    context_positions: BTreeMap<ContextRef, usize>,
    topological_order: Vec<NodeId>,
}

impl CurrentRuntimeIndex {
    fn new(certified: &CertifiedTypedSpec) -> Result<Self> {
        let mut context_positions = BTreeMap::new();
        for (position, context) in certified
            .validated_spec()
            .spec()
            .contexts
            .iter()
            .enumerate()
        {
            if context_positions
                .insert(context.context_ref.clone(), position)
                .is_some()
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate certified context {}",
                    context.context_ref
                )));
            }
        }

        Ok(Self {
            context_positions,
            topological_order: compute_topological_order(certified)?,
        })
    }

    /// Checks that these positions and ids were derived from `certified`.
    pub(crate) fn validate_against(&self, certified: &CertifiedTypedSpec) -> Result<()> {
        if *self != Self::new(certified)? {
            return Err(RuntimeError::InvalidSpec(
                "current runtime index does not match the certified typed spec".to_owned(),
            ));
        }
        Ok(())
    }

    /// Borrows runtime access to the certified authority this index was checked against.
    pub(crate) fn runtime_spec<'a>(
        &'a self,
        certified: &'a CertifiedTypedSpec,
    ) -> CurrentRuntimeSpecRef<'a> {
        CurrentRuntimeSpecRef {
            certified,
            index: self,
        }
    }
}

/// Borrowed runtime access to one certified typed spec and its current indexes.
#[derive(Debug)]
pub struct CurrentRuntimeSpecRef<'a> {
    certified: &'a CertifiedTypedSpec,
    index: &'a CurrentRuntimeIndex,
}

impl<'a> CurrentRuntimeSpecRef<'a> {
    /// Returns the hash-only spec envelope.
    pub fn envelope(&self) -> &'a spec::HashedSpecEnvelope {
        self.certified.envelope()
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &'a SpecHash {
        self.certified.spec_hash()
    }

    /// Returns the verified certificate evidence used to mint this runtime spec.
    pub fn certificate(&self) -> &'a CertifiedSpecCertificate {
        self.certified.certificate()
    }

    /// Returns the hash-defining typed execution spec.
    pub fn spec(&self) -> &'a spec::TypedExecutionSpec {
        self.certified.validated_spec().spec()
    }

    /// Returns deterministic topological node ids.
    pub fn topological_order(&self) -> &'a [NodeId] {
        &self.index.topological_order
    }

    /// Returns a certified node by id.
    pub fn node(&self, node_id: &NodeId) -> Option<&'a spec::NodeSpec> {
        let graph = self.certified.validated_spec().graph();
        graph.forward_node(node_id).or_else(|| {
            graph.remediations().find_map(|(_, remediation)| {
                (remediation.node_id == *node_id).then_some(remediation)
            })
        })
    }

    /// Returns a certified cell by id.
    pub fn cell(&self, cell_id: &CellId) -> Option<&'a spec::CellSpec> {
        self.certified.validated_spec().graph().cell(cell_id)
    }

    /// Returns the certified state descriptor for a node.
    pub fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&'a spec::StateDescriptorIdentity> {
        self.certified
            .descriptor_set()
            .state(&node.descriptor_id)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "node {} references missing state descriptor {}",
                    node.node_id, node.descriptor_id
                ))
            })
    }

    pub(crate) fn executable_nodes(&self) -> Result<Vec<&'a spec::NodeSpec>> {
        let graph = self.certified.validated_spec().graph();
        let mut nodes = self
            .index
            .topological_order
            .iter()
            .map(|node_id| {
                graph.forward_node(node_id).ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "certified topological order references missing node {node_id}"
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        nodes.extend(graph.remediations().map(|(_, remediation)| remediation));
        Ok(nodes)
    }

    pub(crate) fn fact_descriptor_hashes(&self) -> BTreeSet<ContentDigest> {
        self.spec()
            .nodes
            .iter()
            .chain(self.spec().remediations.values())
            .flat_map(|node| {
                node.fact_descriptor_allowlist
                    .iter()
                    .map(|reference| reference.descriptor_hash.clone())
            })
            .collect()
    }

    pub(crate) fn remediation_for_forward_node(
        &self,
        forward_node_id: &NodeId,
    ) -> Option<&'a spec::NodeSpec> {
        self.certified
            .validated_spec()
            .graph()
            .remediation_for_forward_node(forward_node_id)
    }

    pub(crate) fn forward_node_for_remediation(
        &self,
        remediation_node_id: &NodeId,
    ) -> Option<&'a NodeId> {
        self.certified
            .validated_spec()
            .graph()
            .remediations()
            .find_map(|(forward_node_id, remediation)| {
                (remediation.node_id == *remediation_node_id).then_some(forward_node_id)
            })
    }

    pub(crate) fn side_effect_pair_for_submit_node(
        &self,
        submit_node_id: &NodeId,
    ) -> Option<&'a SideEffectPairId> {
        self.spec()
            .side_effect_verify_pair_for_submit_node(submit_node_id)
            .ok()
            .map(|pair| pair.pair_id)
    }

    pub(crate) fn context(
        &self,
        context_ref: &ContextRef,
    ) -> Option<&'a spec::CertifiedContextSpec> {
        self.index
            .context_positions
            .get(context_ref)
            .and_then(|position| self.spec().contexts.get(*position))
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
                let certified = self.cell(&cell.cell_id).ok_or_else(|| {
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
}

mod sealed {
    pub trait Sealed {}
}

/// Internal read contract shared by pre-admission and verified current-run spec authority.
pub(crate) trait CurrentSpecRead: sealed::Sealed {
    fn current_spec_ref(&self) -> CurrentRuntimeSpecRef<'_>;

    fn envelope(&self) -> &spec::HashedSpecEnvelope {
        self.current_spec_ref().envelope()
    }

    fn spec_hash(&self) -> &SpecHash {
        self.current_spec_ref().spec_hash()
    }

    fn certificate(&self) -> &CertifiedSpecCertificate {
        self.current_spec_ref().certificate()
    }

    fn spec(&self) -> &spec::TypedExecutionSpec {
        self.current_spec_ref().spec()
    }

    fn topological_order(&self) -> &[NodeId] {
        self.current_spec_ref().topological_order()
    }

    fn executable_nodes(&self) -> Result<Vec<&spec::NodeSpec>> {
        self.current_spec_ref().executable_nodes()
    }

    fn fact_descriptor_hashes(&self) -> BTreeSet<ContentDigest> {
        self.current_spec_ref().fact_descriptor_hashes()
    }

    fn node(&self, node_id: &NodeId) -> Option<&spec::NodeSpec> {
        self.current_spec_ref().node(node_id)
    }

    fn remediation_for_forward_node(&self, forward_node_id: &NodeId) -> Option<&spec::NodeSpec> {
        self.current_spec_ref()
            .remediation_for_forward_node(forward_node_id)
    }

    fn forward_node_for_remediation(&self, remediation_node_id: &NodeId) -> Option<&NodeId> {
        self.current_spec_ref()
            .forward_node_for_remediation(remediation_node_id)
    }

    fn side_effect_pair_for_submit_node(
        &self,
        submit_node_id: &NodeId,
    ) -> Option<&SideEffectPairId> {
        self.current_spec_ref()
            .side_effect_pair_for_submit_node(submit_node_id)
    }

    fn cell(&self, cell_id: &CellId) -> Option<&spec::CellSpec> {
        self.current_spec_ref().cell(cell_id)
    }

    fn context(&self, context_ref: &ContextRef) -> Option<&spec::CertifiedContextSpec> {
        self.current_spec_ref().context(context_ref)
    }

    fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&spec::StateDescriptorIdentity> {
        self.current_spec_ref().state_descriptor_for_node(node)
    }

    fn validate_input_binding(&self, input: &spec::InputBindingNodeSpec) -> Result<Vec<CellId>> {
        self.current_spec_ref().validate_input_binding(input)
    }
}

impl sealed::Sealed for CertifiedRuntimeSpec {}

impl CurrentSpecRead for CertifiedRuntimeSpec {
    fn current_spec_ref(&self) -> CurrentRuntimeSpecRef<'_> {
        self.current()
    }
}

impl sealed::Sealed for CurrentRuntimeSpecRef<'_> {}

impl CurrentSpecRead for CurrentRuntimeSpecRef<'_> {
    fn current_spec_ref(&self) -> CurrentRuntimeSpecRef<'_> {
        CurrentRuntimeSpecRef {
            certified: self.certified,
            index: self.index,
        }
    }
}

impl CertifiedRuntimeSpec {
    /// Builds deterministic runtime indexes from certifier-backed typed-spec authority.
    pub fn new(certified: CertifiedTypedSpec) -> Result<Self> {
        let index = CurrentRuntimeIndex::new(&certified)?;
        Ok(Self { certified, index })
    }

    pub(crate) fn into_parts(self) -> (CertifiedTypedSpec, CurrentRuntimeIndex) {
        (self.certified, self.index)
    }

    fn current(&self) -> CurrentRuntimeSpecRef<'_> {
        self.index.runtime_spec(&self.certified)
    }

    /// Returns the hash-only spec envelope.
    pub fn envelope(&self) -> &spec::HashedSpecEnvelope {
        CurrentSpecRead::envelope(self)
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        CurrentSpecRead::spec_hash(self)
    }

    /// Returns the verified certificate evidence used to mint this runtime spec.
    pub fn certificate(&self) -> &CertifiedSpecCertificate {
        CurrentSpecRead::certificate(self)
    }

    /// Returns the hash-defining typed execution spec.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        CurrentSpecRead::spec(self)
    }

    /// Returns deterministic topological node ids.
    pub fn topological_order(&self) -> &[NodeId] {
        CurrentSpecRead::topological_order(self)
    }

    /// Returns certified fact descriptor hashes required by any executable node.
    pub(crate) fn fact_descriptor_hashes(&self) -> BTreeSet<ContentDigest> {
        CurrentSpecRead::fact_descriptor_hashes(self)
    }

    /// Returns a certified node by id.
    pub fn node(&self, node_id: &NodeId) -> Option<&spec::NodeSpec> {
        CurrentSpecRead::node(self, node_id)
    }

    /// Returns a certified cell by id.
    pub fn cell(&self, cell_id: &CellId) -> Option<&spec::CellSpec> {
        CurrentSpecRead::cell(self, cell_id)
    }

    /// Returns the certified invocation context required by a node.
    pub fn invocation_context_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<crate::CertifiedInvocationContext> {
        crate::CertifiedInvocationContext::for_node(self, node)
    }

    /// Returns the certified state descriptor for a node.
    pub fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&spec::StateDescriptorIdentity> {
        CurrentSpecRead::state_descriptor_for_node(self, node)
    }
}

fn compute_topological_order(certified: &CertifiedTypedSpec) -> Result<Vec<NodeId>> {
    let graph = certified.validated_spec().graph();
    let mut indegree = BTreeMap::<NodeId, usize>::new();
    let mut successors = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
    for node in graph.forward_nodes() {
        indegree.insert(node.node_id.clone(), 0);
        successors.insert(node.node_id.clone(), BTreeSet::new());
    }
    for node in graph.forward_nodes() {
        for predecessor in &node.deterministic_predecessors {
            if graph.forward_node(predecessor).is_none() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} references missing predecessor {}",
                    node.node_id, predecessor
                )));
            }
            successors
                .get_mut(predecessor)
                .ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "node {} references unindexed predecessor {}",
                        node.node_id, predecessor
                    ))
                })?
                .insert(node.node_id.clone());
            let node_indegree = indegree.get_mut(&node.node_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "certified graph omitted node {} from its runtime index",
                    node.node_id
                ))
            })?;
            *node_indegree = node_indegree.checked_add(1).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "node {} has too many deterministic predecessors",
                    node.node_id
                ))
            })?;
        }
    }

    let mut ready = indegree
        .iter()
        .filter_map(|(node_id, count)| (*count == 0).then_some(node_id.clone()))
        .collect::<VecDeque<_>>();
    let mut order = Vec::with_capacity(indegree.len());
    while let Some(node_id) = ready.pop_front() {
        order.push(node_id.clone());
        let node_successors = successors.get(&node_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "certified graph omitted node {node_id} from its successor index"
            ))
        })?;
        for successor in node_successors {
            let count = indegree.get_mut(successor).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "certified successor index references missing node {successor}"
                ))
            })?;
            *count = count.checked_sub(1).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "certified successor index underflowed node {successor}"
                ))
            })?;
            if *count == 0 {
                let index = ready
                    .iter()
                    .position(|queued| successor < queued)
                    .unwrap_or(ready.len());
                ready.insert(index, successor.clone());
            }
        }
    }
    if order.len() != indegree.len() {
        return Err(RuntimeError::InvalidSpec(
            "certified node graph contains a cycle".to_owned(),
        ));
    }
    Ok(order)
}
