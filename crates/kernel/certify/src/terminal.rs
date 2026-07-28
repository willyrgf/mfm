use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::NodeId;
use mfm_spec::{CertifiedNodeContract, CertifiedSourceSelector, ExpandedCertifiedSpec};

use crate::{CertifyError, Result};

/// Runtime-independent phase used to evaluate certified dependency totality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalPhase {
    /// The occurrence has no terminal transition.
    Unstarted,
    /// The occurrence produced its output.
    Produced,
    /// The occurrence reached typed domain failure.
    Failed,
    /// The occurrence was graph-derived skipped.
    Skipped,
}

impl TerminalPhase {
    const fn terminal(self) -> bool {
        !matches!(self, Self::Unstarted)
    }

    const fn produced(self) -> bool {
        matches!(self, Self::Produced)
    }
}

/// Total disposition of one unstarted node under committed producer phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeDisposition {
    /// Every required input group has a producer output.
    Ready,
    /// At least one group could still produce an output.
    Waiting,
    /// At least one group is terminal without any producer output.
    Skip {
        /// Exact direct producers proving unavailability.
        direct_blockers: Vec<CertifiedSourceSelector>,
    },
}

/// Availability of one exact certified source selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSourcePhase {
    /// The source can still become available.
    Unresolved,
    /// The exact source value is available.
    Available,
    /// The source is terminally unavailable.
    Unavailable,
}

/// Terminal classification of the whole certified run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunDisposition {
    /// Some certified occurrence is not terminal.
    Waiting,
    /// Every occurrence is terminal and every required-success occurrence
    /// produced output.
    Succeeded,
    /// Every occurrence is terminal but a required-success occurrence did not
    /// produce output.
    Failed,
}

/// Structural totality audit for one certified graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTotalityReport {
    node_count: usize,
    required_input_group_count: usize,
}

impl TerminalTotalityReport {
    /// Returns the number of certified occurrences covered.
    pub const fn node_count(self) -> usize {
        self.node_count
    }

    /// Returns the number of closed direct-producer groups covered.
    pub const fn required_input_group_count(self) -> usize {
        self.required_input_group_count
    }
}

/// Validates that every node has the one closed unavailable-input rule and
/// names only direct certified producers.
pub fn validate_terminal_totality(spec: &ExpandedCertifiedSpec) -> Result<TerminalTotalityReport> {
    let known = spec
        .nodes()
        .iter()
        .map(|node| node.node_id().clone())
        .collect::<BTreeSet<_>>();
    let mut group_count = 0;
    for node in spec.nodes() {
        let groups = std::iter::once(node.config_binding())
            .chain(node.context_binding())
            .map(|binding| std::slice::from_ref(binding.source()))
            .chain(
                node.input_bindings()
                    .iter()
                    .map(|binding| binding.ordered_sources()),
            );
        for group in groups {
            group_count += 1;
            if group.is_empty()
                || group.iter().any(|source| {
                    source
                        .producer_node_id()
                        .is_some_and(|id| !known.contains(id))
                })
            {
                return Err(CertifyError::Certification(format!(
                    "node {} has a non-total direct dependency group",
                    node.node_id()
                )));
            }
        }
    }
    if spec
        .run_terminal_contract()
        .required_success_nodes()
        .iter()
        .any(|id| !known.contains(id))
    {
        return Err(CertifyError::Certification(
            "run terminal contract references an unknown node".to_owned(),
        ));
    }
    Ok(TerminalTotalityReport {
        node_count: spec.nodes().len(),
        required_input_group_count: group_count,
    })
}

/// Evaluates the one closed direct-input rule for an unstarted occurrence.
pub fn node_disposition(
    node: &CertifiedNodeContract,
    phases: &BTreeMap<CertifiedSourceSelector, InputSourcePhase>,
) -> Result<NodeDisposition> {
    let groups = std::iter::once(node.config_binding())
        .chain(node.context_binding())
        .map(|binding| std::slice::from_ref(binding.source()))
        .chain(
            node.input_bindings()
                .iter()
                .map(|binding| binding.ordered_sources()),
        )
        .collect::<Vec<_>>();
    if groups.is_empty() {
        return Ok(NodeDisposition::Ready);
    }
    let mut waiting = false;
    let mut blockers = BTreeSet::new();
    for group in groups {
        let mut available = false;
        let mut all_unavailable = true;
        for source in group {
            let phase = phases.get(source).ok_or_else(|| {
                CertifyError::Certification(format!(
                    "dependency phase is missing for certified source {source:?}"
                ))
            })?;
            available |= matches!(phase, InputSourcePhase::Available);
            all_unavailable &= matches!(phase, InputSourcePhase::Unavailable);
        }
        if available {
            continue;
        }
        if all_unavailable {
            blockers.extend(group.iter().cloned());
        } else {
            waiting = true;
        }
    }
    if !blockers.is_empty() {
        Ok(NodeDisposition::Skip {
            direct_blockers: blockers.into_iter().collect(),
        })
    } else if waiting {
        Ok(NodeDisposition::Waiting)
    } else {
        Ok(NodeDisposition::Ready)
    }
}

/// Classifies the run only after every certified occurrence is terminal.
pub fn run_disposition(
    spec: &ExpandedCertifiedSpec,
    phases: &BTreeMap<NodeId, TerminalPhase>,
) -> Result<RunDisposition> {
    for node in spec.nodes() {
        let phase = phases.get(node.node_id()).ok_or_else(|| {
            CertifyError::Certification(format!(
                "terminal phase is missing for node {}",
                node.node_id()
            ))
        })?;
        if !phase.terminal() {
            return Ok(RunDisposition::Waiting);
        }
    }
    if spec
        .run_terminal_contract()
        .required_success_nodes()
        .iter()
        .all(|id| phases.get(id).is_some_and(|phase| phase.produced()))
    {
        Ok(RunDisposition::Succeeded)
    } else {
        Ok(RunDisposition::Failed)
    }
}
