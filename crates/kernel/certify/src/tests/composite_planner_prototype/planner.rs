use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{DigestAlgorithm, NodeId};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const NODE_IDENTITY_DOMAIN: &str = "mfm.node-occurrence.v1";
const NODE_IDENTITY_CONTRACT_VERSION: &str = "mfm.node-occurrence.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Execution {
    Pure,
    Read,
    Effect { executor_contract_ref: String },
}

impl Execution {
    pub(super) fn effect(executor_contract_ref: impl Into<String>) -> Self {
        Self::Effect {
            executor_contract_ref: executor_contract_ref.into(),
        }
    }

    pub(super) const fn kind(&self) -> ExecutionKind {
        match self {
            Self::Pure => ExecutionKind::Pure,
            Self::Read => ExecutionKind::Read,
            Self::Effect { .. } => ExecutionKind::Effect,
        }
    }

    fn executor_contract_ref(&self) -> Option<&str> {
        match self {
            Self::Effect {
                executor_contract_ref,
            } => Some(executor_contract_ref),
            Self::Pure | Self::Read => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ExecutionKind {
    Pure,
    Read,
    Effect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AuthoredKind {
    State,
    Bridge,
}

impl AuthoredKind {
    const fn path_tag(self) -> &'static str {
        match self {
            Self::State => "authored",
            Self::Bridge => "bridge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuthoredNode {
    logical_output: String,
    child_scopes: Vec<String>,
    stable_key: String,
    state_contract_ref: String,
    execution: Execution,
    input_outputs: Vec<String>,
    kind: AuthoredKind,
}

impl AuthoredNode {
    pub(super) fn state(
        logical_output: impl Into<String>,
        child_scopes: impl IntoIterator<Item = impl Into<String>>,
        stable_key: impl Into<String>,
        state_contract_ref: impl Into<String>,
        execution: Execution,
        input_outputs: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            logical_output: logical_output.into(),
            child_scopes: child_scopes.into_iter().map(Into::into).collect(),
            stable_key: stable_key.into(),
            state_contract_ref: state_contract_ref.into(),
            execution,
            input_outputs: input_outputs.into_iter().map(Into::into).collect(),
            kind: AuthoredKind::State,
        }
    }

    pub(super) fn bridge(
        logical_output: impl Into<String>,
        child_scopes: impl IntoIterator<Item = impl Into<String>>,
        stable_key: impl Into<String>,
        state_contract_ref: impl Into<String>,
        input_output: impl Into<String>,
    ) -> Self {
        Self {
            logical_output: logical_output.into(),
            child_scopes: child_scopes.into_iter().map(Into::into).collect(),
            stable_key: stable_key.into(),
            state_contract_ref: state_contract_ref.into(),
            execution: Execution::Pure,
            input_outputs: vec![input_output.into()],
            kind: AuthoredKind::Bridge,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuthoredProgram {
    nodes: Vec<AuthoredNode>,
    public_outputs: BTreeMap<String, String>,
}

impl AuthoredProgram {
    pub(super) fn new(
        nodes: Vec<AuthoredNode>,
        public_outputs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self {
            nodes,
            public_outputs: public_outputs
                .into_iter()
                .map(|(field, output)| (field.into(), output.into()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InjectedState {
    state_contract_ref: String,
    execution: Execution,
}

impl InjectedState {
    pub(super) fn new(state_contract_ref: impl Into<String>, execution: Execution) -> Self {
        Self {
            state_contract_ref: state_contract_ref.into(),
            execution,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameworkPolicy {
    policy_ref: String,
    eligible_state_contracts: BTreeSet<String>,
    pre: Vec<InjectedState>,
    post: Vec<InjectedState>,
}

impl FrameworkPolicy {
    pub(super) fn new(
        policy_ref: impl Into<String>,
        eligible_state_contracts: impl IntoIterator<Item = impl Into<String>>,
        pre: Vec<InjectedState>,
        post: Vec<InjectedState>,
    ) -> Self {
        Self {
            policy_ref: policy_ref.into(),
            eligible_state_contracts: eligible_state_contracts
                .into_iter()
                .map(Into::into)
                .collect(),
            pre,
            post,
        }
    }

    fn applies_to(&self, state_contract_ref: &str) -> bool {
        self.eligible_state_contracts.contains(state_contract_ref)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PlanningProfile {
    framework_policies: Vec<FrameworkPolicy>,
}

impl PlanningProfile {
    pub(super) fn new(framework_policies: Vec<FrameworkPolicy>) -> Self {
        Self { framework_policies }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ExecutorExpansion {
    Leaf,
    Chain {
        pre: Vec<InjectedState>,
        post: Vec<InjectedState>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ExecutorCatalog {
    contracts: BTreeMap<String, ExecutorExpansion>,
}

impl ExecutorCatalog {
    pub(super) fn new(
        contracts: impl IntoIterator<Item = (impl Into<String>, ExecutorExpansion)>,
    ) -> Self {
        Self {
            contracts: contracts
                .into_iter()
                .map(|(contract_ref, expansion)| (contract_ref.into(), expansion))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum PrototypePlanError {
    #[error("duplicate authored logical output {0}")]
    DuplicateLogicalOutput(String),
    #[error("duplicate authored occurrence {0}")]
    DuplicateAuthoredOccurrence(String),
    #[error("authored node {node} references unknown logical output {output}")]
    UnknownLogicalInput { node: String, output: String },
    #[error("public field {field} references unknown logical output {output}")]
    UnknownPublicOutput { field: String, output: String },
    #[error("executor contract {0} is unresolved")]
    UnresolvedExecutor(String),
    #[error("executor expansion cycle: {0:?}")]
    ExecutorCycle(Vec<String>),
    #[error("executor fragment for {parent} selects non-leaf executor {nested}")]
    NonLeafExecutor { parent: String, nested: String },
    #[error("duplicate final expansion path {0}")]
    DuplicateFinalPath(String),
    #[error("canonical node identity failed: {0}")]
    CanonicalIdentity(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PathStep {
    EntryPoint,
    Child {
        stable_key: String,
        ordinal: usize,
    },
    Authored {
        stable_key: String,
        ordinal: usize,
    },
    Bridge {
        stable_key: String,
        ordinal: usize,
    },
    FrameworkPre {
        policy_ref: String,
        policy_ordinal: usize,
        state_ordinal: usize,
    },
    FrameworkProtected {
        policy_ref: String,
        policy_ordinal: usize,
    },
    FrameworkPost {
        policy_ref: String,
        policy_ordinal: usize,
        state_ordinal: usize,
    },
    ExecutorPre {
        executor_contract_ref: String,
        state_ordinal: usize,
    },
    ExecutorProtected {
        executor_contract_ref: String,
    },
    ExecutorPost {
        executor_contract_ref: String,
        state_ordinal: usize,
    },
}

impl PathStep {
    fn json(&self) -> serde_json::Value {
        match self {
            Self::EntryPoint => serde_json::json!({
                "kind": "entry_point",
            }),
            Self::Child {
                stable_key,
                ordinal,
            } => serde_json::json!({
                "kind": "nested_child",
                "ordinal": ordinal,
                "stable_key": stable_key,
            }),
            Self::Authored {
                stable_key,
                ordinal,
            } => serde_json::json!({
                "kind": "authored",
                "ordinal": ordinal,
                "stable_key": stable_key,
            }),
            Self::Bridge {
                stable_key,
                ordinal,
            } => serde_json::json!({
                "kind": "bridge",
                "ordinal": ordinal,
                "stable_key": stable_key,
            }),
            Self::FrameworkPre {
                policy_ref,
                policy_ordinal,
                state_ordinal,
            } => serde_json::json!({
                "kind": "framework_pre",
                "policy_ordinal": policy_ordinal,
                "policy_ref": policy_ref,
                "state_ordinal": state_ordinal,
            }),
            Self::FrameworkProtected {
                policy_ref,
                policy_ordinal,
            } => serde_json::json!({
                "kind": "framework_protected",
                "policy_ordinal": policy_ordinal,
                "policy_ref": policy_ref,
            }),
            Self::FrameworkPost {
                policy_ref,
                policy_ordinal,
                state_ordinal,
            } => serde_json::json!({
                "kind": "framework_post",
                "policy_ordinal": policy_ordinal,
                "policy_ref": policy_ref,
                "state_ordinal": state_ordinal,
            }),
            Self::ExecutorPre {
                executor_contract_ref,
                state_ordinal,
            } => serde_json::json!({
                "executor_contract_ref": executor_contract_ref,
                "kind": "executor_pre",
                "state_ordinal": state_ordinal,
            }),
            Self::ExecutorProtected {
                executor_contract_ref,
            } => serde_json::json!({
                "executor_contract_ref": executor_contract_ref,
                "kind": "executor_protected",
            }),
            Self::ExecutorPost {
                executor_contract_ref,
                state_ordinal,
            } => serde_json::json!({
                "executor_contract_ref": executor_contract_ref,
                "kind": "executor_post",
                "state_ordinal": state_ordinal,
            }),
        }
    }

    fn display(&self) -> String {
        match self {
            Self::EntryPoint => "entry".to_owned(),
            Self::Child {
                stable_key,
                ordinal,
            } => format!("child[{ordinal}]={stable_key}"),
            Self::Authored {
                stable_key,
                ordinal,
            } => format!("authored[{ordinal}]={stable_key}"),
            Self::Bridge {
                stable_key,
                ordinal,
            } => format!("bridge[{ordinal}]={stable_key}"),
            Self::FrameworkPre {
                policy_ref,
                policy_ordinal,
                state_ordinal,
            } => format!("framework-pre[{policy_ordinal}:{state_ordinal}]={policy_ref}"),
            Self::FrameworkProtected {
                policy_ref,
                policy_ordinal,
            } => format!("framework-protected[{policy_ordinal}]={policy_ref}"),
            Self::FrameworkPost {
                policy_ref,
                policy_ordinal,
                state_ordinal,
            } => format!("framework-post[{policy_ordinal}:{state_ordinal}]={policy_ref}"),
            Self::ExecutorPre {
                executor_contract_ref,
                state_ordinal,
            } => format!("executor-pre[{state_ordinal}]={executor_contract_ref}"),
            Self::ExecutorProtected {
                executor_contract_ref,
            } => format!("executor-protected={executor_contract_ref}"),
            Self::ExecutorPost {
                executor_contract_ref,
                state_ordinal,
            } => format!("executor-post[{state_ordinal}]={executor_contract_ref}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CanonicalExpansionPath {
    steps: Vec<PathStep>,
}

impl CanonicalExpansionPath {
    fn entry_point() -> Self {
        Self {
            steps: vec![PathStep::EntryPoint],
        }
    }

    fn with_step(&self, step: PathStep) -> Self {
        let mut path = self.clone();
        path.steps.push(step);
        path
    }

    fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes, PrototypePlanError> {
        let value = self.steps.iter().map(PathStep::json).collect::<Vec<_>>();
        let json = serde_json::to_string(&value)
            .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))
    }

    pub(super) fn display(&self) -> String {
        self.steps
            .iter()
            .map(PathStep::display)
            .collect::<Vec<_>>()
            .join("/")
    }

    pub(super) fn framework_pre_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, PathStep::FrameworkPre { .. }))
            .count()
    }

    pub(super) fn executor_protected_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, PathStep::ExecutorProtected { .. }))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingSource {
    AuthoredOutput(String),
    Node(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingNode {
    path: CanonicalExpansionPath,
    state_contract_ref: String,
    execution: Execution,
    inputs: Vec<PendingSource>,
    authored_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpandedNode {
    pub(super) node_id: NodeId,
    pub(super) path: CanonicalExpansionPath,
    pub(super) state_contract_ref: String,
    pub(super) execution: ExecutionKind,
    pub(super) input_nodes: Vec<NodeId>,
    pub(super) authored_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpandedProgram {
    pub(super) nodes: Vec<ExpandedNode>,
    pub(super) entry_nodes: BTreeMap<String, NodeId>,
    pub(super) protected_outputs: BTreeMap<String, NodeId>,
    pub(super) effective_outputs: BTreeMap<String, NodeId>,
    pub(super) public_outputs: BTreeMap<String, NodeId>,
}

pub(super) fn expand(
    authored: &AuthoredProgram,
    profile: &PlanningProfile,
    executors: &ExecutorCatalog,
) -> Result<ExpandedProgram, PrototypePlanError> {
    validate_authored_program(authored)?;

    let mut positioned = authored
        .nodes
        .iter()
        .map(|node| authored_path(node, &authored.nodes).map(|path| (path, node)))
        .collect::<Result<Vec<_>, _>>()?;
    positioned.sort_by(|left, right| left.0.cmp(&right.0));

    let mut pending = Vec::new();
    let mut entry_nodes = BTreeMap::new();
    let mut protected_outputs = BTreeMap::new();
    let mut effective_outputs = BTreeMap::new();
    for (base_path, authored_node) in positioned {
        let occurrence =
            expand_authored_occurrence(authored_node, base_path, profile, executors, &mut pending)?;
        entry_nodes.insert(authored_node.logical_output.clone(), occurrence.entry);
        protected_outputs.insert(authored_node.logical_output.clone(), occurrence.protected);
        effective_outputs.insert(authored_node.logical_output.clone(), occurrence.effective);
    }

    let mut seen_paths = BTreeSet::new();
    for node in &pending {
        let path = node.path.display();
        if !seen_paths.insert(path.clone()) {
            return Err(PrototypePlanError::DuplicateFinalPath(path));
        }
    }

    let ids = pending
        .iter()
        .map(|node| final_node_id(&node.path, &node.state_contract_ref))
        .collect::<Result<Vec<_>, _>>()?;
    let logical_ids = effective_outputs
        .iter()
        .map(|(logical, pending_index)| (logical.clone(), ids[*pending_index].clone()))
        .collect::<BTreeMap<_, _>>();
    let nodes = pending
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let input_nodes = node
                .inputs
                .iter()
                .map(|source| match source {
                    PendingSource::AuthoredOutput(logical) => logical_ids
                        .get(logical)
                        .cloned()
                        .ok_or_else(|| PrototypePlanError::UnknownLogicalInput {
                            node: node.authored_output.clone(),
                            output: logical.clone(),
                        }),
                    PendingSource::Node(source_index) => Ok(ids[*source_index].clone()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ExpandedNode {
                node_id: ids[index].clone(),
                path: node.path,
                state_contract_ref: node.state_contract_ref,
                execution: node.execution.kind(),
                input_nodes,
                authored_output: node.authored_output,
            })
        })
        .collect::<Result<Vec<_>, PrototypePlanError>>()?;

    let entry_nodes = resolve_pending_map(entry_nodes, &ids);
    let protected_outputs = resolve_pending_map(protected_outputs, &ids);
    let effective_outputs = resolve_pending_map(effective_outputs, &ids);
    let public_outputs = authored
        .public_outputs
        .iter()
        .map(|(field, logical)| {
            effective_outputs
                .get(logical)
                .cloned()
                .map(|node_id| (field.clone(), node_id))
                .ok_or_else(|| PrototypePlanError::UnknownPublicOutput {
                    field: field.clone(),
                    output: logical.clone(),
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    Ok(ExpandedProgram {
        nodes,
        entry_nodes,
        protected_outputs,
        effective_outputs,
        public_outputs,
    })
}

fn resolve_pending_map(
    pending: BTreeMap<String, usize>,
    ids: &[NodeId],
) -> BTreeMap<String, NodeId> {
    pending
        .into_iter()
        .map(|(logical, index)| (logical, ids[index].clone()))
        .collect()
}

fn validate_authored_program(authored: &AuthoredProgram) -> Result<(), PrototypePlanError> {
    let mut logical_outputs = BTreeSet::new();
    let mut occurrences = BTreeSet::new();
    for node in &authored.nodes {
        if !logical_outputs.insert(node.logical_output.clone()) {
            return Err(PrototypePlanError::DuplicateLogicalOutput(
                node.logical_output.clone(),
            ));
        }
        let occurrence = (
            node.child_scopes.clone(),
            node.kind,
            node.stable_key.clone(),
        );
        if !occurrences.insert(occurrence) {
            return Err(PrototypePlanError::DuplicateAuthoredOccurrence(format!(
                "{}/{}:{}",
                node.child_scopes.join("/"),
                node.kind.path_tag(),
                node.stable_key
            )));
        }
    }
    for node in &authored.nodes {
        for input in &node.input_outputs {
            if !logical_outputs.contains(input) {
                return Err(PrototypePlanError::UnknownLogicalInput {
                    node: node.logical_output.clone(),
                    output: input.clone(),
                });
            }
        }
    }
    Ok(())
}

fn authored_path(
    node: &AuthoredNode,
    all_nodes: &[AuthoredNode],
) -> Result<CanonicalExpansionPath, PrototypePlanError> {
    let mut path = CanonicalExpansionPath::entry_point();
    for (depth, stable_key) in node.child_scopes.iter().enumerate() {
        let parent = &node.child_scopes[..depth];
        let sibling_keys = all_nodes
            .iter()
            .filter(|candidate| {
                candidate.child_scopes.len() > depth && candidate.child_scopes[..depth] == *parent
            })
            .map(|candidate| candidate.child_scopes[depth].clone())
            .collect::<BTreeSet<_>>();
        let ordinal = sibling_keys
            .iter()
            .position(|candidate| candidate == stable_key)
            .ok_or_else(|| PrototypePlanError::DuplicateAuthoredOccurrence(stable_key.clone()))?;
        path = path.with_step(PathStep::Child {
            stable_key: stable_key.clone(),
            ordinal,
        });
    }

    let sibling_keys = all_nodes
        .iter()
        .filter(|candidate| {
            candidate.child_scopes == node.child_scopes && candidate.kind == node.kind
        })
        .map(|candidate| candidate.stable_key.clone())
        .collect::<BTreeSet<_>>();
    let ordinal = sibling_keys
        .iter()
        .position(|candidate| candidate == &node.stable_key)
        .ok_or_else(|| PrototypePlanError::DuplicateAuthoredOccurrence(node.stable_key.clone()))?;
    let occurrence = match node.kind {
        AuthoredKind::State => PathStep::Authored {
            stable_key: node.stable_key.clone(),
            ordinal,
        },
        AuthoredKind::Bridge => PathStep::Bridge {
            stable_key: node.stable_key.clone(),
            ordinal,
        },
    };
    Ok(path.with_step(occurrence))
}

struct OccurrenceExpansion {
    entry: usize,
    protected: usize,
    effective: usize,
}

fn expand_authored_occurrence(
    authored: &AuthoredNode,
    base_path: CanonicalExpansionPath,
    profile: &PlanningProfile,
    executors: &ExecutorCatalog,
    pending: &mut Vec<PendingNode>,
) -> Result<OccurrenceExpansion, PrototypePlanError> {
    let applicable = profile
        .framework_policies
        .iter()
        .enumerate()
        .filter(|(_, policy)| {
            authored.kind == AuthoredKind::State && policy.applies_to(&authored.state_contract_ref)
        })
        .collect::<Vec<_>>();
    let original_inputs = authored
        .input_outputs
        .iter()
        .cloned()
        .map(PendingSource::AuthoredOutput)
        .collect::<Vec<_>>();
    let mut current_inputs = original_inputs.clone();
    let mut entry = None;

    for (policy_ordinal, policy) in &applicable {
        let policy_base = framework_policy_base(&base_path, &applicable, *policy_ordinal);
        for (state_ordinal, state) in policy.pre.iter().enumerate() {
            let path = policy_base.with_step(PathStep::FrameworkPre {
                policy_ref: policy.policy_ref.clone(),
                policy_ordinal: *policy_ordinal,
                state_ordinal,
            });
            let expanded = append_state(
                pending,
                path,
                state,
                current_inputs,
                &authored.logical_output,
                executors,
            )?;
            entry.get_or_insert(expanded.entry);
            current_inputs = vec![PendingSource::Node(expanded.effective)];
        }
    }

    let protected_path = applicable
        .iter()
        .fold(base_path.clone(), |path, (ordinal, policy)| {
            path.with_step(PathStep::FrameworkProtected {
                policy_ref: policy.policy_ref.clone(),
                policy_ordinal: *ordinal,
            })
        });
    let protected = append_state(
        pending,
        protected_path,
        &InjectedState::new(
            authored.state_contract_ref.clone(),
            authored.execution.clone(),
        ),
        current_inputs,
        &authored.logical_output,
        executors,
    )?;
    entry.get_or_insert(protected.entry);
    let authored_protected = protected.protected;
    current_inputs = vec![PendingSource::Node(protected.effective)];

    for (policy_ordinal, policy) in applicable.iter().rev() {
        let policy_base = framework_policy_base(&base_path, &applicable, *policy_ordinal);
        for (state_ordinal, state) in policy.post.iter().enumerate() {
            let path = policy_base.with_step(PathStep::FrameworkPost {
                policy_ref: policy.policy_ref.clone(),
                policy_ordinal: *policy_ordinal,
                state_ordinal,
            });
            let expanded = append_state(
                pending,
                path,
                state,
                current_inputs,
                &authored.logical_output,
                executors,
            )?;
            current_inputs = vec![PendingSource::Node(expanded.effective)];
        }
    }

    let effective = match current_inputs.as_slice() {
        [PendingSource::Node(index)] => *index,
        _ => authored_protected,
    };
    Ok(OccurrenceExpansion {
        entry: entry.unwrap_or(protected.entry),
        protected: authored_protected,
        effective,
    })
}

fn framework_policy_base(
    base_path: &CanonicalExpansionPath,
    applicable: &[(usize, &FrameworkPolicy)],
    target_ordinal: usize,
) -> CanonicalExpansionPath {
    applicable
        .iter()
        .take_while(|(ordinal, _)| *ordinal < target_ordinal)
        .fold(base_path.clone(), |path, (ordinal, policy)| {
            path.with_step(PathStep::FrameworkProtected {
                policy_ref: policy.policy_ref.clone(),
                policy_ordinal: *ordinal,
            })
        })
}

struct StateExpansion {
    entry: usize,
    protected: usize,
    effective: usize,
}

fn append_state(
    pending: &mut Vec<PendingNode>,
    base_path: CanonicalExpansionPath,
    state: &InjectedState,
    inputs: Vec<PendingSource>,
    authored_output: &str,
    executors: &ExecutorCatalog,
) -> Result<StateExpansion, PrototypePlanError> {
    let Some(executor_contract_ref) = state.execution.executor_contract_ref() else {
        let index = push_pending(pending, base_path, state, inputs, authored_output);
        return Ok(StateExpansion {
            entry: index,
            protected: index,
            effective: index,
        });
    };
    let expansion =
        resolve_executor_expansion(executor_contract_ref, executors, &mut Vec::new(), true)?;
    match expansion {
        ExecutorExpansion::Leaf => {
            let index = push_pending(pending, base_path, state, inputs, authored_output);
            Ok(StateExpansion {
                entry: index,
                protected: index,
                effective: index,
            })
        }
        ExecutorExpansion::Chain { pre, post } => {
            let mut current_inputs = inputs;
            let mut entry = None;
            for (state_ordinal, pre_state) in pre.iter().enumerate() {
                ensure_fragment_executor_is_leaf(executor_contract_ref, pre_state, executors)?;
                let index = push_pending(
                    pending,
                    base_path.with_step(PathStep::ExecutorPre {
                        executor_contract_ref: executor_contract_ref.to_owned(),
                        state_ordinal,
                    }),
                    pre_state,
                    current_inputs,
                    authored_output,
                );
                entry.get_or_insert(index);
                current_inputs = vec![PendingSource::Node(index)];
            }
            let protected = push_pending(
                pending,
                base_path.with_step(PathStep::ExecutorProtected {
                    executor_contract_ref: executor_contract_ref.to_owned(),
                }),
                state,
                current_inputs,
                authored_output,
            );
            entry.get_or_insert(protected);
            let mut effective = protected;
            for (state_ordinal, post_state) in post.iter().enumerate() {
                ensure_fragment_executor_is_leaf(executor_contract_ref, post_state, executors)?;
                effective = push_pending(
                    pending,
                    base_path.with_step(PathStep::ExecutorPost {
                        executor_contract_ref: executor_contract_ref.to_owned(),
                        state_ordinal,
                    }),
                    post_state,
                    vec![PendingSource::Node(effective)],
                    authored_output,
                );
            }
            Ok(StateExpansion {
                entry: entry.unwrap_or(protected),
                protected,
                effective,
            })
        }
    }
}

fn push_pending(
    pending: &mut Vec<PendingNode>,
    path: CanonicalExpansionPath,
    state: &InjectedState,
    inputs: Vec<PendingSource>,
    authored_output: &str,
) -> usize {
    let index = pending.len();
    pending.push(PendingNode {
        path,
        state_contract_ref: state.state_contract_ref.clone(),
        execution: state.execution.clone(),
        inputs,
        authored_output: authored_output.to_owned(),
    });
    index
}

fn ensure_fragment_executor_is_leaf(
    parent: &str,
    state: &InjectedState,
    executors: &ExecutorCatalog,
) -> Result<(), PrototypePlanError> {
    let Some(nested) = state.execution.executor_contract_ref() else {
        return Ok(());
    };
    let expansion =
        resolve_executor_expansion(nested, executors, &mut vec![parent.to_owned()], false)?;
    if matches!(expansion, ExecutorExpansion::Leaf) {
        Ok(())
    } else {
        Err(PrototypePlanError::NonLeafExecutor {
            parent: parent.to_owned(),
            nested: nested.to_owned(),
        })
    }
}

fn resolve_executor_expansion<'a>(
    contract_ref: &str,
    executors: &'a ExecutorCatalog,
    stack: &mut Vec<String>,
    root: bool,
) -> Result<&'a ExecutorExpansion, PrototypePlanError> {
    if let Some(cycle_start) = stack.iter().position(|entry| entry == contract_ref) {
        let mut cycle = stack[cycle_start..].to_vec();
        cycle.push(contract_ref.to_owned());
        return Err(PrototypePlanError::ExecutorCycle(cycle));
    }
    let expansion = executors
        .contracts
        .get(contract_ref)
        .ok_or_else(|| PrototypePlanError::UnresolvedExecutor(contract_ref.to_owned()))?;
    if matches!(expansion, ExecutorExpansion::Leaf) {
        return Ok(expansion);
    }

    stack.push(contract_ref.to_owned());
    if let ExecutorExpansion::Chain { pre, post } = expansion {
        for state in pre.iter().chain(post) {
            let Some(nested) = state.execution.executor_contract_ref() else {
                continue;
            };
            let nested_expansion = resolve_executor_expansion(nested, executors, stack, false)?;
            if !root && !matches!(nested_expansion, ExecutorExpansion::Leaf) {
                stack.pop();
                return Err(PrototypePlanError::NonLeafExecutor {
                    parent: contract_ref.to_owned(),
                    nested: nested.to_owned(),
                });
            }
        }
    }
    stack.pop();
    Ok(expansion)
}

fn final_node_id(
    path: &CanonicalExpansionPath,
    state_contract_ref: &str,
) -> Result<NodeId, PrototypePlanError> {
    let path_json = path.canonical_json()?;
    let path_value: serde_json::Value = serde_json::from_slice(path_json.as_bytes())
        .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))?;
    let preimage = serde_json::json!({
        "canonical_expansion_path": path_value,
        "identity_contract_version": NODE_IDENTITY_CONTRACT_VERSION,
        "state_contract_ref": state_contract_ref,
    });
    let envelope = serde_json::json!({
        "domain": NODE_IDENTITY_DOMAIN,
        "value": preimage,
    });
    let envelope = serde_json::to_string(&envelope)
        .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&envelope)
        .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))?;
    node_id_from_canonical_envelope(canonical.as_bytes())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrototypeDigestEnvelope {
    domain: String,
    value: serde_json::Value,
}

fn node_id_from_canonical_envelope(bytes: &[u8]) -> Result<NodeId, PrototypePlanError> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))?;
    let envelope: PrototypeDigestEnvelope = serde_json::from_slice(canonical.as_bytes())
        .map_err(|error| PrototypePlanError::CanonicalIdentity(error.to_string()))?;
    if envelope.domain != NODE_IDENTITY_DOMAIN {
        return Err(PrototypePlanError::CanonicalIdentity(
            "node identity digest domain mismatch".to_owned(),
        ));
    }
    let _ = envelope.value;
    Ok(NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

#[test]
fn node_identity_digest_uses_one_canonical_domain_value_envelope() {
    let path = CanonicalExpansionPath::entry_point();
    let path_json = path.canonical_json().expect("canonical path");
    let path_value: serde_json::Value =
        serde_json::from_slice(path_json.as_bytes()).expect("path value");
    let value = serde_json::json!({
        "canonical_expansion_path": path_value,
        "identity_contract_version": NODE_IDENTITY_CONTRACT_VERSION,
        "state_contract_ref": "domain.read",
    });
    let raw_value = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&value).expect("raw value JSON"),
    )
    .expect("canonical raw value");
    let accepted_envelope = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&serde_json::json!({
            "domain": NODE_IDENTITY_DOMAIN,
            "value": value,
        }))
        .expect("envelope JSON"),
    )
    .expect("canonical envelope");
    let accepted =
        final_node_id(&path, "domain.read").expect("universally enveloped node identity");
    assert_eq!(
        accepted,
        node_id_from_canonical_envelope(accepted_envelope.as_bytes())
            .expect("exact envelope representation")
    );

    let raw_value_digest =
        NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, raw_value.digest_bytes());
    let mut prefix_preimage = NODE_IDENTITY_DOMAIN.as_bytes().to_vec();
    prefix_preimage.extend_from_slice(raw_value.as_bytes());
    let prefix_digest = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&prefix_preimage),
    );
    let mut nul_prefix_preimage = NODE_IDENTITY_DOMAIN.as_bytes().to_vec();
    nul_prefix_preimage.push(0);
    nul_prefix_preimage.extend_from_slice(raw_value.as_bytes());
    let nul_prefix_digest = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&nul_prefix_preimage),
    );
    let wrong_domain_envelope = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&serde_json::json!({
            "domain": "mfm.node-occurrence.wrong.v1",
            "value": serde_json::from_slice::<serde_json::Value>(raw_value.as_bytes())
                .expect("raw identity value"),
        }))
        .expect("wrong-domain envelope JSON"),
    )
    .expect("canonical wrong-domain envelope");
    let wrong_domain_digest = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        wrong_domain_envelope.digest_bytes(),
    );

    for confused in [
        raw_value_digest,
        prefix_digest,
        nul_prefix_digest,
        wrong_domain_digest,
    ] {
        assert_ne!(accepted, confused);
    }

    let noncanonical = format!(
        r#"{{"value":{},"domain":"{NODE_IDENTITY_DOMAIN}"}}"#,
        raw_value.as_str()
    );
    let float = format!(r#"{{"domain":"{NODE_IDENTITY_DOMAIN}","value":1.5}}"#);
    let duplicate_key = format!(
        r#"{{"domain":"{NODE_IDENTITY_DOMAIN}","domain":"{NODE_IDENTITY_DOMAIN}","value":{}}}"#,
        raw_value.as_str()
    );
    for hostile in [
        raw_value.as_bytes(),
        prefix_preimage.as_slice(),
        nul_prefix_preimage.as_slice(),
        wrong_domain_envelope.as_bytes(),
        noncanonical.as_bytes(),
        float.as_bytes(),
        duplicate_key.as_bytes(),
    ] {
        assert!(
            node_id_from_canonical_envelope(hostile).is_err(),
            "hostile digest representation must reject"
        );
    }
}
