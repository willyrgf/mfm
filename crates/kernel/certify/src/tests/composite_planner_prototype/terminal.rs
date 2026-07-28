use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
use mfm_ids::{DigestAlgorithm, NodeId};
use std::collections::{BTreeMap, BTreeSet};

const TERMINAL_FIXTURE_NODE_DOMAIN: &str = "mfm.terminal-fixture-node.prototype.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalPhase {
    Unstarted,
    Succeeded,
    Failed,
    Skipped,
}

impl TerminalPhase {
    const fn is_terminal(self) -> bool {
        !matches!(self, Self::Unstarted)
    }

    const fn produced_output(self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnavailableInputRule {
    AllPossibleProducersTerminalWithoutOutput,
    Undefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DependencyContract {
    required_input_sources: Vec<Vec<NodeId>>,
    unavailable_input_rule: UnavailableInputRule,
}

impl DependencyContract {
    pub(super) fn closed(required_input_sources: Vec<Vec<NodeId>>) -> Self {
        Self {
            required_input_sources,
            unavailable_input_rule: UnavailableInputRule::AllPossibleProducersTerminalWithoutOutput,
        }
    }

    pub(super) fn undefined(required_input_sources: Vec<Vec<NodeId>>) -> Self {
        Self {
            required_input_sources,
            unavailable_input_rule: UnavailableInputRule::Undefined,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TerminalNode {
    node_id: NodeId,
    wired_input_sources: Vec<Vec<NodeId>>,
    dependency_contract: Option<DependencyContract>,
}

impl TerminalNode {
    pub(super) fn new(node_id: NodeId, wired_input_sources: Vec<Vec<NodeId>>) -> Self {
        Self {
            node_id,
            dependency_contract: Some(DependencyContract::closed(wired_input_sources.clone())),
            wired_input_sources,
        }
    }

    pub(super) fn with_dependency_contract(
        mut self,
        dependency_contract: Option<DependencyContract>,
    ) -> Self {
        self.dependency_contract = dependency_contract;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunTerminalContract {
    required_success_nodes: Vec<NodeId>,
    public_output_binding: NodeId,
}

impl RunTerminalContract {
    pub(super) fn new(required_success_nodes: Vec<NodeId>, public_output_binding: NodeId) -> Self {
        Self {
            required_success_nodes,
            public_output_binding,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TerminalGraph {
    nodes: Vec<TerminalNode>,
    run_terminal_contract: Option<RunTerminalContract>,
}

impl TerminalGraph {
    pub(super) fn new(
        nodes: Vec<TerminalNode>,
        run_terminal_contract: Option<RunTerminalContract>,
    ) -> Self {
        Self {
            nodes,
            run_terminal_contract,
        }
    }

    pub(super) fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|node| node.node_id.clone()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NodeDisposition {
    Ready,
    Waiting,
    Skip { direct_blockers: Vec<NodeId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalClassification {
    Legal(RunOutcome),
    Illegal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TotalityReport {
    pub(super) partial_assignments: usize,
    pub(super) terminal_assignments: usize,
    pub(super) classified_terminal_assignments: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum TerminalContractError {
    #[error("node {0} is missing its dependency contract")]
    MissingDependencyContract(String),
    #[error("node {0} has an empty required-input source set")]
    EmptyInputSourceSet(String),
    #[error("node {node} references unknown direct producer {producer}")]
    UnknownDirectProducer { node: String, producer: String },
    #[error("node {0} dependency sources do not equal its direct wired producers")]
    NonDirectDependency(String),
    #[error("node {0} has no closed unavailable-input rule")]
    UndefinedUnavailableRule(String),
    #[error("dependency graph contains a cycle")]
    DependencyCycle,
    #[error("run terminal contract is missing")]
    MissingRunTerminalContract,
    #[error("run terminal contract references unknown node {0}")]
    UnknownTerminalNode(String),
}

pub(super) fn node_id(label: &str) -> NodeId {
    let envelope = CanonicalValue::object([
        (
            "domain",
            CanonicalValue::String(TERMINAL_FIXTURE_NODE_DOMAIN.to_owned()),
        ),
        ("value", CanonicalValue::String(label.to_owned())),
    ])
    .expect("fixture digest envelope has unique keys");
    let envelope = CanonicalJsonBytes::from_value(&envelope);
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, envelope.digest_bytes())
}

pub(super) fn validate_totality(
    graph: &TerminalGraph,
) -> Result<TotalityReport, TerminalContractError> {
    let index = validate_contract_shape(graph)?;
    let node_ids = graph.node_ids();
    let assignment_count =
        4_usize.pow(u32::try_from(node_ids.len()).expect("prototype graph size fits u32"));
    let mut terminal_assignments = 0;
    let mut classified_terminal_assignments = 0;

    for ordinal in 0..assignment_count {
        let phases = decode_assignment(ordinal, &node_ids);
        for node in &graph.nodes {
            if phases[&node.node_id] == TerminalPhase::Unstarted {
                let _ = disposition(node, &phases)?;
            }
        }
        if phases.values().all(|phase| phase.is_terminal()) {
            terminal_assignments += 1;
            match classify_terminal_assignment(graph, &index, &phases)? {
                TerminalClassification::Legal(_) | TerminalClassification::Illegal => {
                    classified_terminal_assignments += 1;
                }
            }
        }
    }

    Ok(TotalityReport {
        partial_assignments: assignment_count,
        terminal_assignments,
        classified_terminal_assignments,
    })
}

pub(super) fn disposition_for(
    graph: &TerminalGraph,
    node_id: &NodeId,
    phases: &BTreeMap<NodeId, TerminalPhase>,
) -> Result<NodeDisposition, TerminalContractError> {
    validate_contract_shape(graph)?;
    let node = graph
        .nodes
        .iter()
        .find(|node| &node.node_id == node_id)
        .ok_or_else(|| TerminalContractError::UnknownTerminalNode(node_id.as_str().to_owned()))?;
    disposition(node, phases)
}

fn validate_contract_shape(
    graph: &TerminalGraph,
) -> Result<BTreeMap<NodeId, &TerminalNode>, TerminalContractError> {
    let index = graph
        .nodes
        .iter()
        .map(|node| (node.node_id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    for node in &graph.nodes {
        let dependency = node.dependency_contract.as_ref().ok_or_else(|| {
            TerminalContractError::MissingDependencyContract(node.node_id.as_str().to_owned())
        })?;
        if dependency.unavailable_input_rule
            != UnavailableInputRule::AllPossibleProducersTerminalWithoutOutput
        {
            return Err(TerminalContractError::UndefinedUnavailableRule(
                node.node_id.as_str().to_owned(),
            ));
        }
        if canonical_sources(&dependency.required_input_sources)
            != canonical_sources(&node.wired_input_sources)
        {
            return Err(TerminalContractError::NonDirectDependency(
                node.node_id.as_str().to_owned(),
            ));
        }
        for sources in &dependency.required_input_sources {
            if sources.is_empty() {
                return Err(TerminalContractError::EmptyInputSourceSet(
                    node.node_id.as_str().to_owned(),
                ));
            }
            for producer in sources {
                if !index.contains_key(producer) {
                    return Err(TerminalContractError::UnknownDirectProducer {
                        node: node.node_id.as_str().to_owned(),
                        producer: producer.as_str().to_owned(),
                    });
                }
            }
        }
    }
    if graph_has_cycle(&graph.nodes) {
        return Err(TerminalContractError::DependencyCycle);
    }

    let run_terminal = graph
        .run_terminal_contract
        .as_ref()
        .ok_or(TerminalContractError::MissingRunTerminalContract)?;
    for node_id in run_terminal
        .required_success_nodes
        .iter()
        .chain(std::iter::once(&run_terminal.public_output_binding))
    {
        if !index.contains_key(node_id) {
            return Err(TerminalContractError::UnknownTerminalNode(
                node_id.as_str().to_owned(),
            ));
        }
    }
    Ok(index)
}

fn canonical_sources(sources: &[Vec<NodeId>]) -> Vec<Vec<NodeId>> {
    let mut canonical = sources
        .iter()
        .map(|group| {
            let mut group = group.clone();
            group.sort();
            group
        })
        .collect::<Vec<_>>();
    canonical.sort();
    canonical
}

fn graph_has_cycle(nodes: &[TerminalNode]) -> bool {
    let dependencies = nodes
        .iter()
        .map(|node| {
            (
                node.node_id.clone(),
                node.wired_input_sources
                    .iter()
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    dependencies
        .keys()
        .any(|node_id| visit(node_id, &dependencies, &mut visiting, &mut visited))
}

fn visit(
    node_id: &NodeId,
    dependencies: &BTreeMap<NodeId, Vec<NodeId>>,
    visiting: &mut BTreeSet<NodeId>,
    visited: &mut BTreeSet<NodeId>,
) -> bool {
    if visited.contains(node_id) {
        return false;
    }
    if !visiting.insert(node_id.clone()) {
        return true;
    }
    if dependencies
        .get(node_id)
        .into_iter()
        .flatten()
        .any(|dependency| visit(dependency, dependencies, visiting, visited))
    {
        return true;
    }
    visiting.remove(node_id);
    visited.insert(node_id.clone());
    false
}

fn disposition(
    node: &TerminalNode,
    phases: &BTreeMap<NodeId, TerminalPhase>,
) -> Result<NodeDisposition, TerminalContractError> {
    let dependency = node.dependency_contract.as_ref().ok_or_else(|| {
        TerminalContractError::MissingDependencyContract(node.node_id.as_str().to_owned())
    })?;
    if dependency.required_input_sources.is_empty() {
        return Ok(NodeDisposition::Ready);
    }

    let mut waiting = false;
    let mut blockers = BTreeSet::new();
    for sources in &dependency.required_input_sources {
        if sources
            .iter()
            .any(|source| phases[source].produced_output())
        {
            continue;
        }
        if sources.iter().all(|source| phases[source].is_terminal()) {
            blockers.extend(
                sources
                    .iter()
                    .filter(|source| !phases[*source].produced_output())
                    .cloned(),
            );
        } else {
            waiting = true;
        }
    }
    if !blockers.is_empty() {
        return Ok(NodeDisposition::Skip {
            direct_blockers: blockers.into_iter().collect(),
        });
    }
    if waiting {
        Ok(NodeDisposition::Waiting)
    } else {
        Ok(NodeDisposition::Ready)
    }
}

fn decode_assignment(mut ordinal: usize, node_ids: &[NodeId]) -> BTreeMap<NodeId, TerminalPhase> {
    node_ids
        .iter()
        .map(|node_id| {
            let phase = match ordinal % 4 {
                0 => TerminalPhase::Unstarted,
                1 => TerminalPhase::Succeeded,
                2 => TerminalPhase::Failed,
                3 => TerminalPhase::Skipped,
                _ => unreachable!("modulo four"),
            };
            ordinal /= 4;
            (node_id.clone(), phase)
        })
        .collect()
}

fn classify_terminal_assignment(
    graph: &TerminalGraph,
    index: &BTreeMap<NodeId, &TerminalNode>,
    phases: &BTreeMap<NodeId, TerminalPhase>,
) -> Result<TerminalClassification, TerminalContractError> {
    for node in &graph.nodes {
        let phase = phases[&node.node_id];
        let disposition = disposition(node, phases)?;
        let legal = match phase {
            TerminalPhase::Succeeded | TerminalPhase::Failed => {
                matches!(disposition, NodeDisposition::Ready)
            }
            TerminalPhase::Skipped => {
                matches!(disposition, NodeDisposition::Skip { .. })
            }
            TerminalPhase::Unstarted => false,
        };
        if !legal {
            return Ok(TerminalClassification::Illegal);
        }
    }

    let terminal = graph
        .run_terminal_contract
        .as_ref()
        .ok_or(TerminalContractError::MissingRunTerminalContract)?;
    let required_succeeded = terminal
        .required_success_nodes
        .iter()
        .all(|node_id| phases[node_id] == TerminalPhase::Succeeded);
    let public_output_exists = phases[&terminal.public_output_binding] == TerminalPhase::Succeeded;
    debug_assert!(index.contains_key(&terminal.public_output_binding));
    Ok(TerminalClassification::Legal(
        if required_succeeded && public_output_exists {
            RunOutcome::Succeeded
        } else {
            RunOutcome::Failed
        },
    ))
}
