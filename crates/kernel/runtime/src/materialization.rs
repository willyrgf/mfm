//! Pure dependency-source resolution for exact input materialization.

use mfm_journal::v2::NodePhase;
use mfm_spec::{CertifiedNodeContract, CertifiedSourceSelector};
use mfm_store::VerifiedRunView;

use crate::{Result, RuntimeError};

/// Whether one unstarted occurrence can now receive a store-prepared frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameReadiness {
    /// Every destination has an available source.
    Ready,
    /// No destination is permanently blocked, but at least one producer is live.
    Waiting,
    /// At least one destination has only terminal unavailable producers.
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceReadiness {
    Pending,
    Available,
    TerminalUnavailable,
}

/// Derives callback-frame readiness from the complete verified fold.
///
/// This is only scheduler input. The store independently derives and seals the
/// exact frame or the complete dependency-blocking set before append.
pub(crate) fn frame_readiness(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
) -> Result<FrameReadiness> {
    let mut states = Vec::with_capacity(
        1 + usize::from(node.context_binding().is_some()) + node.input_bindings().len(),
    );
    states.push(source_group_readiness(
        view,
        std::iter::once(node.config_binding().source()),
    )?);
    if let Some(context) = node.context_binding() {
        states.push(source_group_readiness(
            view,
            std::iter::once(context.source()),
        )?);
    }
    for binding in node.input_bindings() {
        states.push(source_group_readiness(
            view,
            binding.ordered_sources().iter(),
        )?);
    }

    Ok(combine_destination_readiness(states))
}

fn source_group_readiness<'a>(
    view: &VerifiedRunView,
    sources: impl IntoIterator<Item = &'a CertifiedSourceSelector>,
) -> Result<FrameReadiness> {
    source_group_from_states(
        sources
            .into_iter()
            .map(|source| source_readiness(view, source))
            .collect::<Result<Vec<_>>>()?,
    )
    .ok_or(RuntimeError::CatalogSelection)
}

fn source_readiness(
    view: &VerifiedRunView,
    source: &CertifiedSourceSelector,
) -> Result<SourceReadiness> {
    let (producer_node_id, available) = match source {
        CertifiedSourceSelector::NodeOutput {
            producer_node_id,
            output_ordinal,
            ..
        } => (
            producer_node_id,
            view.node_outputs(producer_node_id)
                .iter()
                .try_fold(false, |available, output| {
                    Ok::<_, RuntimeError>(
                        available || output.fields()?.output_ordinal == *output_ordinal,
                    )
                })?,
        ),
        CertifiedSourceSelector::NodeFact {
            producer_node_id,
            emission_ordinal,
        } => (
            producer_node_id,
            view.node_facts(producer_node_id)
                .iter()
                .try_fold(false, |available, fact| {
                    Ok::<_, RuntimeError>(
                        available || fact.fields()?.emission_ordinal == *emission_ordinal,
                    )
                })?,
        ),
        CertifiedSourceSelector::RunAdmission { .. }
        | CertifiedSourceSelector::Config { .. }
        | CertifiedSourceSelector::QualifiedSupport { .. }
        | CertifiedSourceSelector::Seed { .. }
        | CertifiedSourceSelector::Context { .. }
        | CertifiedSourceSelector::CrossRunEffectiveOutput { .. }
        | CertifiedSourceSelector::CrossRunEvidence { .. } => {
            return Ok(SourceReadiness::Available);
        }
    };
    if available {
        return Ok(SourceReadiness::Available);
    }
    match view.node_phase(producer_node_id) {
        Some(NodePhase::Terminal) => Ok(SourceReadiness::TerminalUnavailable),
        Some(NodePhase::Unstarted | NodePhase::AwaitingEffect) => Ok(SourceReadiness::Pending),
        None => Err(RuntimeError::CatalogSelection),
    }
}

fn source_group_from_states(
    states: impl IntoIterator<Item = SourceReadiness>,
) -> Option<FrameReadiness> {
    let mut saw_pending = false;
    let mut saw_terminal = false;
    for state in states {
        match state {
            SourceReadiness::Available => return Some(FrameReadiness::Ready),
            SourceReadiness::Pending => saw_pending = true,
            SourceReadiness::TerminalUnavailable => saw_terminal = true,
        }
    }
    if saw_pending {
        Some(FrameReadiness::Waiting)
    } else if saw_terminal {
        Some(FrameReadiness::Blocked)
    } else {
        None
    }
}

fn combine_destination_readiness(
    states: impl IntoIterator<Item = FrameReadiness>,
) -> FrameReadiness {
    let mut combined = FrameReadiness::Ready;
    for state in states {
        match state {
            FrameReadiness::Blocked => return FrameReadiness::Blocked,
            FrameReadiness::Waiting => combined = FrameReadiness::Waiting,
            FrameReadiness::Ready => {}
        }
    }
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_alternative_wins_within_one_destination() {
        assert_eq!(
            source_group_from_states([
                SourceReadiness::TerminalUnavailable,
                SourceReadiness::Available,
                SourceReadiness::Pending,
            ]),
            Some(FrameReadiness::Ready)
        );
    }

    #[test]
    fn live_alternative_prevents_a_destination_from_being_blocked() {
        assert_eq!(
            source_group_from_states([
                SourceReadiness::TerminalUnavailable,
                SourceReadiness::Pending,
            ]),
            Some(FrameReadiness::Waiting)
        );
    }

    #[test]
    fn one_permanently_blocked_destination_makes_the_node_impossible() {
        assert_eq!(
            combine_destination_readiness([
                FrameReadiness::Waiting,
                FrameReadiness::Blocked,
                FrameReadiness::Ready,
            ]),
            FrameReadiness::Blocked
        );
    }
}
