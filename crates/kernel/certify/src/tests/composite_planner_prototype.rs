//! Retained contract-shaping prototype for the accepted composite-planner cutover.
//!
//! These tests deliberately exercise a private model rather than add a second production planner.
//! The model freezes the pre-cutover conformance questions that must be answered before the
//! canonical package-B schema and package-C API switch.

#[path = "composite_planner_prototype/planner.rs"]
mod planner;
#[path = "composite_planner_prototype/terminal.rs"]
mod terminal;

use mfm_canonical::RecoverabilityContractV1;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes};
use planner::{
    expand, AuthoredNode, AuthoredProgram, Execution, ExecutionKind, ExecutorCatalog,
    ExecutorExpansion, FrameworkPolicy, InjectedState, PlanningProfile, PrototypePlanError,
    ReviewedContractRef,
};
use std::collections::BTreeMap;
use terminal::{
    disposition_for, node_id, validate_totality, DependencyContract, NodeDisposition,
    RunTerminalContract, TerminalContractError, TerminalGraph, TerminalNode, TerminalPhase,
};

const LEAF_EXECUTOR: &str = "executor.leaf.v1";
const AUDIT_EXECUTOR: &str = "executor.audit.v1";
const MAIN_EXECUTOR: &str = "executor.main.v1";

fn reviewed_contract_ref(label: &str, fixture_tag: u8) -> ReviewedContractRef {
    let contract = RecoverabilityContractV1::embedded().expect("embedded recoverability contract");
    let schema_id = contract
        .schema_id("mfm.component-implementation-descriptor.v1")
        .expect("registered component descriptor schema")
        .clone();
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        DigestBytes::from_array([fixture_tag; 32]),
    );
    ReviewedContractRef::new(
        label,
        ContentRef::new(schema_id, content_digest).expect("reviewed fixture content ref"),
    )
}

fn leaf_executor_ref() -> ReviewedContractRef {
    reviewed_contract_ref(LEAF_EXECUTOR, 0xe1)
}

fn audit_executor_ref() -> ReviewedContractRef {
    reviewed_contract_ref(AUDIT_EXECUTOR, 0xe2)
}

fn main_executor_ref() -> ReviewedContractRef {
    reviewed_contract_ref(MAIN_EXECUTOR, 0xe3)
}

#[test]
fn composite_expansion_covers_authored_shapes_and_stable_path_ordinals() {
    let forward = authored_program(false);
    let reversed = authored_program(true);
    let profile = planning_profile();
    let executors = executor_catalog();

    let forward = expand(&forward, &profile, &executors).expect("forward expansion");
    let reversed = expand(&reversed, &profile, &executors).expect("reversed expansion");
    assert_eq!(
        forward, reversed,
        "declaration order must not affect expansion"
    );

    assert_eq!(
        execution_for(&forward, "domain.normalize"),
        ExecutionKind::Pure
    );
    assert_eq!(
        execution_for(&forward, "domain.read.alpha"),
        ExecutionKind::Read
    );
    assert_eq!(
        execution_for(&forward, "domain.send"),
        ExecutionKind::Effect
    );
    assert_ne!(
        forward.protected_outputs["normalize"], forward.effective_outputs["normalize"],
        "pure authored occurrences must participate in framework expansion"
    );
    assert_ne!(
        forward.protected_outputs["read-alpha"], forward.effective_outputs["read-alpha"],
        "read authored occurrences must participate in framework expansion"
    );

    assert_eq!(
        authored_path(&forward, "read-alpha"),
        "entry/child[0]=branch-alpha/authored[0]=read"
    );
    assert_eq!(
        authored_path(&forward, "bridge-alpha"),
        "entry/child[0]=branch-alpha/bridge[0]=export"
    );
    assert_eq!(
        authored_path(&forward, "read-beta"),
        "entry/child[1]=branch-beta/authored[0]=read"
    );
    assert_eq!(authored_path(&forward, "join"), "entry/authored[0]=join");

    let join_entry = &forward.entry_nodes["join"];
    let join_entry = forward
        .nodes
        .iter()
        .find(|node| &node.node_id == join_entry)
        .expect("join entry node");
    assert_eq!(
        join_entry.input_nodes,
        vec![
            forward.effective_outputs["bridge-alpha"].clone(),
            forward.effective_outputs["bridge-beta"].clone(),
        ],
        "fan-in must resolve both child bridge outputs through their effective bindings"
    );
}

#[test]
fn framework_outer_executor_inner_expansion_rewires_only_final_outputs() {
    let authored = authored_program(false);
    let expanded =
        expand(&authored, &planning_profile(), &executor_catalog()).expect("combined expansion");
    let send_nodes = expanded
        .nodes
        .iter()
        .filter(|node| node.authored_output == "send")
        .collect::<Vec<_>>();
    let contracts = send_nodes
        .iter()
        .map(|node| node.state_contract_ref.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        contracts,
        vec![
            "executor.audit.prepare",
            "framework.audit.effect",
            "executor.audit.finish",
            "executor.main.prepare",
            "domain.send",
            "executor.main.confirm",
            "framework.release",
        ]
    );
    assert_eq!(
        send_nodes
            .iter()
            .map(|node| node.path.display())
            .collect::<Vec<_>>(),
        vec![
            "entry/authored[2]=send/framework-pre[0:0]=policy.guard.v1/executor-pre[0]=executor.audit.v1",
            "entry/authored[2]=send/framework-pre[0:0]=policy.guard.v1/executor-protected=executor.audit.v1",
            "entry/authored[2]=send/framework-pre[0:0]=policy.guard.v1/executor-post[0]=executor.audit.v1",
            "entry/authored[2]=send/framework-protected[0]=policy.guard.v1/executor-pre[0]=executor.main.v1",
            "entry/authored[2]=send/framework-protected[0]=policy.guard.v1/executor-protected=executor.main.v1",
            "entry/authored[2]=send/framework-protected[0]=policy.guard.v1/executor-post[0]=executor.main.v1",
            "entry/authored[2]=send/framework-post[0:0]=policy.guard.v1",
        ]
    );

    let injected_effect = send_nodes
        .iter()
        .find(|node| node.state_contract_ref == "framework.audit.effect")
        .expect("framework-injected effect");
    assert_eq!(injected_effect.path.framework_pre_count(), 1);
    assert_eq!(injected_effect.path.executor_protected_count(), 1);
    assert_eq!(
        send_nodes
            .iter()
            .filter(|node| node.state_contract_ref == "framework.audit.effect")
            .count(),
        1,
        "framework-injected effects must receive executor expansion exactly once"
    );
    let leaf_fragment_effect = send_nodes
        .iter()
        .find(|node| node.state_contract_ref == "executor.audit.prepare")
        .expect("leaf fragment effect");
    assert_eq!(leaf_fragment_effect.execution, ExecutionKind::Effect);
    assert_eq!(leaf_fragment_effect.path.executor_protected_count(), 0);

    let send_entry = &expanded.entry_nodes["send"];
    let send_entry = expanded
        .nodes
        .iter()
        .find(|node| &node.node_id == send_entry)
        .expect("send entry node");
    assert_eq!(
        send_entry.input_nodes,
        vec![expanded.effective_outputs["join"].clone()],
        "downstream input must use the producer's final effective output"
    );
    assert_ne!(
        expanded.protected_outputs["join"], expanded.effective_outputs["join"],
        "framework post-state must create a distinct effective output"
    );
    assert_eq!(
        expanded.public_outputs["result"],
        expanded.effective_outputs["send"]
    );
    assert_ne!(
        expanded.public_outputs["result"], expanded.protected_outputs["send"],
        "public output must not bypass executor or framework post chains"
    );

    let unexpanded = expand(&authored, &PlanningProfile::default(), &executor_catalog())
        .expect("executor-only expansion");
    assert_eq!(
        unexpanded
            .nodes
            .iter()
            .find(|node| node.state_contract_ref == "domain.send")
            .expect("executor-only protected effect")
            .path
            .display(),
        "entry/authored[2]=send/executor-protected=executor.main.v1"
    );
    assert_ne!(
        unexpanded.protected_outputs["send"], expanded.protected_outputs["send"],
        "the authored occurrence cannot carry a final id before profile expansion"
    );
}

#[test]
fn executor_expansion_rejects_unresolved_non_leaf_and_cyclic_fragments() {
    let authored = effect_only_program(main_executor_ref());
    let missing = ExecutorCatalog::default();
    assert_eq!(
        expand(&authored, &PlanningProfile::default(), &missing),
        Err(PrototypePlanError::UnresolvedExecutor(
            MAIN_EXECUTOR.to_owned()
        ))
    );

    let unresolved_nested = ExecutorCatalog::new([(
        main_executor_ref(),
        ExecutorExpansion::Chain {
            pre: vec![InjectedState::new(
                reviewed_contract_ref("executor.needs.missing", 0x70),
                Execution::effect(reviewed_contract_ref("executor.missing.v1", 0xe4)),
            )],
            post: Vec::new(),
        },
    )]);
    assert_eq!(
        expand(&authored, &PlanningProfile::default(), &unresolved_nested),
        Err(PrototypePlanError::UnresolvedExecutor(
            "executor.missing.v1".to_owned()
        ))
    );

    let non_leaf = ExecutorCatalog::new([
        (
            main_executor_ref(),
            ExecutorExpansion::Chain {
                pre: vec![InjectedState::new(
                    reviewed_contract_ref("executor.recursive", 0x71),
                    Execution::effect(audit_executor_ref()),
                )],
                post: Vec::new(),
            },
        ),
        (
            audit_executor_ref(),
            ExecutorExpansion::Chain {
                pre: vec![InjectedState::new(
                    reviewed_contract_ref("executor.audit.prepare", 0x72),
                    Execution::Pure,
                )],
                post: Vec::new(),
            },
        ),
    ]);
    assert_eq!(
        expand(&authored, &PlanningProfile::default(), &non_leaf),
        Err(PrototypePlanError::NonLeafExecutor {
            parent: MAIN_EXECUTOR.to_owned(),
            nested: AUDIT_EXECUTOR.to_owned(),
        })
    );

    let cyclic = ExecutorCatalog::new([
        (
            main_executor_ref(),
            ExecutorExpansion::Chain {
                pre: vec![InjectedState::new(
                    reviewed_contract_ref("executor.to.audit", 0x73),
                    Execution::effect(audit_executor_ref()),
                )],
                post: Vec::new(),
            },
        ),
        (
            audit_executor_ref(),
            ExecutorExpansion::Chain {
                pre: vec![InjectedState::new(
                    reviewed_contract_ref("executor.to.main", 0x74),
                    Execution::effect(main_executor_ref()),
                )],
                post: Vec::new(),
            },
        ),
    ]);
    assert_eq!(
        expand(&authored, &PlanningProfile::default(), &cyclic),
        Err(PrototypePlanError::ExecutorCycle(vec![
            MAIN_EXECUTOR.to_owned(),
            AUDIT_EXECUTOR.to_owned(),
            MAIN_EXECUTOR.to_owned(),
        ]))
    );
}

#[test]
fn dependency_and_terminal_contracts_are_total_and_direct() {
    let source_a = node_id("terminal.source-a");
    let source_b = node_id("terminal.source-b");
    let join = node_id("terminal.join");
    let graph = TerminalGraph::new(
        vec![
            TerminalNode::new(source_a.clone(), Vec::new()),
            TerminalNode::new(source_b.clone(), Vec::new()),
            TerminalNode::new(
                join.clone(),
                vec![vec![source_a.clone()], vec![source_b.clone()]],
            ),
        ],
        Some(RunTerminalContract::new(vec![join.clone()], join.clone())),
    );
    let report = validate_totality(&graph).expect("total terminal graph");
    assert_eq!(report.partial_assignments, 64);
    assert_eq!(report.terminal_assignments, 27);
    assert_eq!(report.classified_terminal_assignments, 27);

    let mut phases = BTreeMap::from([
        (source_a.clone(), TerminalPhase::Failed),
        (source_b.clone(), TerminalPhase::Unstarted),
        (join.clone(), TerminalPhase::Unstarted),
    ]);
    assert_eq!(
        disposition_for(&graph, &source_b, &phases).expect("independent source disposition"),
        NodeDisposition::Ready,
        "an unrelated source remains ready after a sibling failure"
    );
    assert_eq!(
        disposition_for(&graph, &join, &phases).expect("join disposition"),
        NodeDisposition::Skip {
            direct_blockers: vec![source_a.clone()],
        }
    );
    phases.insert(source_b.clone(), TerminalPhase::Skipped);
    let mut complete_blockers = vec![source_a.clone(), source_b.clone()];
    complete_blockers.sort();
    assert_eq!(
        disposition_for(&graph, &join, &phases).expect("complete blocker set"),
        NodeDisposition::Skip {
            direct_blockers: complete_blockers,
        }
    );

    let missing_contract = TerminalGraph::new(
        vec![
            TerminalNode::new(source_a.clone(), Vec::new()),
            TerminalNode::new(join.clone(), vec![vec![source_a.clone()]])
                .with_dependency_contract(None),
        ],
        Some(RunTerminalContract::new(vec![join.clone()], join.clone())),
    );
    assert_eq!(
        validate_totality(&missing_contract),
        Err(TerminalContractError::MissingDependencyContract(
            join.as_str().to_owned()
        ))
    );

    let transitive_source = node_id("terminal.transitive");
    let non_direct = TerminalGraph::new(
        vec![
            TerminalNode::new(source_a.clone(), Vec::new()),
            TerminalNode::new(transitive_source.clone(), vec![vec![source_a.clone()]]),
            TerminalNode::new(join.clone(), vec![vec![transitive_source]])
                .with_dependency_contract(Some(DependencyContract::closed(vec![vec![
                    source_a.clone()
                ]]))),
        ],
        Some(RunTerminalContract::new(vec![join.clone()], join.clone())),
    );
    assert_eq!(
        validate_totality(&non_direct),
        Err(TerminalContractError::NonDirectDependency(
            join.as_str().to_owned()
        ))
    );

    let undefined_rule = TerminalGraph::new(
        vec![
            TerminalNode::new(source_a.clone(), Vec::new()),
            TerminalNode::new(join.clone(), vec![vec![source_a.clone()]]).with_dependency_contract(
                Some(DependencyContract::undefined(vec![vec![source_a.clone()]])),
            ),
        ],
        Some(RunTerminalContract::new(vec![join.clone()], join.clone())),
    );
    assert_eq!(
        validate_totality(&undefined_rule),
        Err(TerminalContractError::UndefinedUnavailableRule(
            join.as_str().to_owned()
        ))
    );

    let missing_run_terminal =
        TerminalGraph::new(vec![TerminalNode::new(source_a.clone(), Vec::new())], None);
    assert_eq!(
        validate_totality(&missing_run_terminal),
        Err(TerminalContractError::MissingRunTerminalContract)
    );

    let unknown_public_output = node_id("terminal.unknown-public-output");
    let unknown_terminal = TerminalGraph::new(
        vec![TerminalNode::new(source_a.clone(), Vec::new())],
        Some(RunTerminalContract::new(
            vec![source_a.clone()],
            unknown_public_output.clone(),
        )),
    );
    assert_eq!(
        validate_totality(&unknown_terminal),
        Err(TerminalContractError::UnknownTerminalNode(
            unknown_public_output.as_str().to_owned()
        ))
    );

    let cycle = TerminalGraph::new(
        vec![
            TerminalNode::new(source_a.clone(), vec![vec![join.clone()]]),
            TerminalNode::new(join.clone(), vec![vec![source_a]]),
        ],
        Some(RunTerminalContract::new(vec![join.clone()], join)),
    );
    assert_eq!(
        validate_totality(&cycle),
        Err(TerminalContractError::DependencyCycle)
    );
}

fn authored_program(reverse: bool) -> AuthoredProgram {
    let mut nodes = vec![
        AuthoredNode::state(
            "normalize",
            Vec::<String>::new(),
            "normalize",
            reviewed_contract_ref("domain.normalize", 0x01),
            Execution::Pure,
            Vec::<String>::new(),
        ),
        AuthoredNode::state(
            "read-alpha",
            ["branch-alpha"],
            "read",
            reviewed_contract_ref("domain.read.alpha", 0x02),
            Execution::Read,
            ["normalize"],
        ),
        AuthoredNode::bridge(
            "bridge-alpha",
            ["branch-alpha"],
            "export",
            reviewed_contract_ref("framework.bridge", 0x03),
            "read-alpha",
        ),
        AuthoredNode::state(
            "read-beta",
            ["branch-beta"],
            "read",
            reviewed_contract_ref("domain.read.beta", 0x04),
            Execution::Read,
            ["normalize"],
        ),
        AuthoredNode::bridge(
            "bridge-beta",
            ["branch-beta"],
            "export",
            reviewed_contract_ref("framework.bridge", 0x03),
            "read-beta",
        ),
        AuthoredNode::state(
            "join",
            Vec::<String>::new(),
            "join",
            reviewed_contract_ref("domain.join", 0x06),
            Execution::Pure,
            ["bridge-alpha", "bridge-beta"],
        ),
        AuthoredNode::state(
            "send",
            Vec::<String>::new(),
            "send",
            reviewed_contract_ref("domain.send", 0x07),
            Execution::effect(main_executor_ref()),
            ["join"],
        ),
    ];
    if reverse {
        nodes.reverse();
    }
    AuthoredProgram::new(nodes, [("result", "send")])
}

fn effect_only_program(executor_contract_ref: ReviewedContractRef) -> AuthoredProgram {
    AuthoredProgram::new(
        vec![AuthoredNode::state(
            "effect",
            Vec::<String>::new(),
            "effect",
            reviewed_contract_ref("domain.effect", 0x08),
            Execution::effect(executor_contract_ref),
            Vec::<String>::new(),
        )],
        [("result", "effect")],
    )
}

fn planning_profile() -> PlanningProfile {
    PlanningProfile::new(vec![FrameworkPolicy::new(
        reviewed_contract_ref("policy.guard.v1", 0xd1),
        [
            reviewed_contract_ref("domain.normalize", 0x01),
            reviewed_contract_ref("domain.read.alpha", 0x02),
            reviewed_contract_ref("domain.read.beta", 0x04),
            reviewed_contract_ref("domain.join", 0x06),
            reviewed_contract_ref("domain.send", 0x07),
        ],
        vec![InjectedState::new(
            reviewed_contract_ref("framework.audit.effect", 0x20),
            Execution::effect(audit_executor_ref()),
        )],
        vec![InjectedState::new(
            reviewed_contract_ref("framework.release", 0x21),
            Execution::Pure,
        )],
    )])
}

fn executor_catalog() -> ExecutorCatalog {
    ExecutorCatalog::new([
        (leaf_executor_ref(), ExecutorExpansion::Leaf),
        (
            audit_executor_ref(),
            ExecutorExpansion::Chain {
                pre: vec![InjectedState::new(
                    reviewed_contract_ref("executor.audit.prepare", 0x30),
                    Execution::effect(leaf_executor_ref()),
                )],
                post: vec![InjectedState::new(
                    reviewed_contract_ref("executor.audit.finish", 0x31),
                    Execution::Pure,
                )],
            },
        ),
        (
            main_executor_ref(),
            ExecutorExpansion::Chain {
                pre: vec![InjectedState::new(
                    reviewed_contract_ref("executor.main.prepare", 0x32),
                    Execution::Pure,
                )],
                post: vec![InjectedState::new(
                    reviewed_contract_ref("executor.main.confirm", 0x33),
                    Execution::Read,
                )],
            },
        ),
    ])
}

fn authored_path(expanded: &planner::ExpandedProgram, authored_output: &str) -> String {
    expanded
        .nodes
        .iter()
        .find(|node| {
            node.authored_output == authored_output
                && (node.state_contract_ref.starts_with("domain.")
                    || node.path.display().contains("/bridge["))
        })
        .expect("authored protected node")
        .path
        .display()
        .split("/framework-")
        .next()
        .expect("authored path prefix")
        .to_owned()
}

fn execution_for(expanded: &planner::ExpandedProgram, state_contract_ref: &str) -> ExecutionKind {
    expanded
        .nodes
        .iter()
        .find(|node| node.state_contract_ref == state_contract_ref)
        .expect("state contract")
        .execution
}
