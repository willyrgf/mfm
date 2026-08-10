use super::{
    expand_export_source_closure, ExportRunEvidence, ExportSourceClosureError,
    MAX_PORTABLE_FACT_ROUTES, MAX_PORTABLE_SOURCE_RUNS,
};
use mfm_ids::RunId;
use std::collections::{BTreeMap, BTreeSet};

fn run(digit: u8) -> RunId {
    let hex = format!("{digit:x}").repeat(64);
    RunId::parse(format!("run:sha256-jcs-v1:{hex}")).expect("run id")
}

#[test]
fn multi_hop_shared_and_deterministic() {
    let root = run(1);
    let mid = run(2);
    let shared = run(3);
    let other = run(4);
    let graph = BTreeMap::from([
        (root.clone(), BTreeSet::from([mid.clone(), other.clone()])),
        (mid.clone(), BTreeSet::from([shared.clone()])),
        (other.clone(), BTreeSet::from([shared.clone()])),
        (shared.clone(), BTreeSet::new()),
    ]);
    let first = expand_export_source_closure(&root, graph[&root].clone(), |id| {
        Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
    })
    .expect("shared dag");
    let second = expand_export_source_closure(&root, graph[&root].clone(), |id| {
        Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
    })
    .expect("shared dag again");
    assert_eq!(first, second);
    assert_eq!(first, BTreeSet::from([mid, other, shared]));
}

#[test]
fn cyclic_source_graph_is_rejected() {
    let root = run(1);
    let a = run(2);
    let b = run(3);
    let graph = BTreeMap::from([
        (root.clone(), BTreeSet::from([a.clone()])),
        (a.clone(), BTreeSet::from([b.clone()])),
        (b.clone(), BTreeSet::from([a.clone()])),
    ]);
    let error = expand_export_source_closure(&root, graph[&root].clone(), |id| {
        Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
    })
    .expect_err("cycle");
    assert_eq!(error, ExportSourceClosureError::Cycle);
}

#[test]
fn over_budget_source_graph_is_rejected() {
    let root = run(0);
    let mut pending = BTreeSet::new();
    let mut graph = BTreeMap::new();
    for index in 1..=(MAX_PORTABLE_SOURCE_RUNS + 1) {
        let hex = format!("{index:064x}");
        let source = RunId::parse(format!("run:sha256-jcs-v1:{hex}")).expect("distinct run id");
        pending.insert(source.clone());
        graph.insert(source, BTreeSet::new());
    }
    graph.insert(root.clone(), pending.clone());
    let error = expand_export_source_closure(&root, pending, |id| {
        Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
    })
    .expect_err("over budget");
    assert_eq!(error, ExportSourceClosureError::OverBudget);
}

#[test]
fn fanout_pending_bound_rejects_before_enqueue() {
    let root = run(1);
    let first = run(2);
    let mut fanout = BTreeSet::new();
    let mut graph = BTreeMap::new();
    for index in 3..=(MAX_PORTABLE_SOURCE_RUNS + 2) {
        let hex = format!("{index:064x}");
        let leaf = RunId::parse(format!("run:sha256-jcs-v1:{hex}")).expect("leaf run id");
        fanout.insert(leaf.clone());
        graph.insert(leaf, BTreeSet::new());
    }
    graph.insert(root.clone(), BTreeSet::from([first.clone()]));
    graph.insert(first.clone(), fanout);
    let error = expand_export_source_closure(&root, graph[&root].clone(), |id| {
        Ok::<_, ExportSourceClosureError>(graph.get(id).cloned().unwrap_or_default())
    })
    .expect_err("pending fanout bound");
    assert_eq!(error, ExportSourceClosureError::OverBudget);
}

fn rename_export(
    evidence: &mut ExportRunEvidence,
    run_id: RunId,
    direct_source_run_ids: BTreeSet<RunId>,
) {
    evidence.fragment.run_id = run_id.clone();
    evidence.fragment.header.run_id = run_id.clone();
    evidence.header.run_id = run_id;
    evidence.fragment.direct_source_run_ids = direct_source_run_ids;
}

#[tokio::test]
async fn flattened_multi_hop_source_closure_is_accepted() {
    let root_id = run(1);
    let middle_id = run(2);
    let leaf_id = run(3);
    let mut root = super::super::test_support::zero_state_export(1)
        .await
        .expect("root fixture")
        .export;
    let mut middle = super::super::test_support::zero_state_export(1)
        .await
        .expect("middle fixture")
        .export;
    let mut leaf = super::super::test_support::zero_state_export(1)
        .await
        .expect("leaf fixture")
        .export;
    rename_export(
        &mut root,
        root_id.clone(),
        BTreeSet::from([middle_id.clone()]),
    );
    rename_export(
        &mut middle,
        middle_id.clone(),
        BTreeSet::from([leaf_id.clone()]),
    );
    rename_export(&mut leaf, leaf_id.clone(), BTreeSet::new());

    let sealed = root
        .with_authorized_sources(None, vec![(middle, None), (leaf, None)])
        .expect("flattened recursive closure");
    let source_ids = sealed
        .authorized_source_prefixes()
        .map(|source| source.run_id().clone())
        .collect::<Vec<_>>();
    assert_eq!(source_ids, vec![middle_id, leaf_id]);
}

#[test]
fn fact_route_bound_rejects_before_insert() {
    assert!(super::ensure_fact_route_capacity(MAX_PORTABLE_FACT_ROUTES - 1).is_ok());
    assert!(super::ensure_fact_route_capacity(MAX_PORTABLE_FACT_ROUTES).is_err());
}
