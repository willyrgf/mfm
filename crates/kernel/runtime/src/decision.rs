//! Pure next-action selection for one verified run view.

use std::cmp::Ordering;

/// Callback verdict for one structurally consumable observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ObservationVerdict<T> {
    /// The observation is valid but cannot settle the occurrence.
    InsufficientEvidence,
    /// The observation settles the occurrence with the bound candidate.
    Settlement(T),
    /// The observation violates the selected state evidence contract.
    InvalidEvidence,
}

/// Result of scanning the complete ordered observation sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvidenceScan<T> {
    /// Every structurally consumable observation was insufficient.
    AllInsufficient,
    /// The first accepted observation and its settlement candidate.
    Settlement {
        /// Journal order within the occurrence.
        observation_order: usize,
        /// Candidate bound to the accepted committed observation.
        candidate: T,
    },
    /// The first invalid observation.
    InvalidEvidence {
        /// Journal order within the occurrence.
        observation_order: usize,
    },
}

/// Scans all observations in journal order.
///
/// A settlement is selected only after every earlier observation returned
/// [`ObservationVerdict::InsufficientEvidence`]. The scan still evaluates the
/// complete suffix: invalid evidence anywhere blocks before a remembered
/// settlement can be committed.
pub(crate) fn scan_observations<T>(
    verdicts: impl IntoIterator<Item = ObservationVerdict<T>>,
) -> EvidenceScan<T> {
    let mut first_settlement = None;
    for (observation_order, verdict) in verdicts.into_iter().enumerate() {
        match verdict {
            ObservationVerdict::InsufficientEvidence => {}
            ObservationVerdict::Settlement(candidate) => {
                if first_settlement.is_none() {
                    first_settlement = Some((observation_order, candidate));
                }
            }
            ObservationVerdict::InvalidEvidence => {
                return EvidenceScan::InvalidEvidence { observation_order };
            }
        }
    }
    match first_settlement {
        Some((observation_order, candidate)) => EvidenceScan::Settlement {
            observation_order,
            candidate,
        },
        None => EvidenceScan::AllInsufficient,
    }
}

/// One local transition that requires no ambient operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalAction {
    /// Settle a pure state.
    PureSettlement,
    /// Durably freeze an effect request before an ensure call.
    EffectRequest,
    /// Commit a structurally proven dependency skip.
    DependencySkip,
}

/// One registered ambient-operation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessAction {
    /// Perform one immutable read operation.
    Read,
    /// Perform one keyed effect ensure operation.
    Ensure,
}

/// Candidate derived for one certified occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OccurrenceCandidate<S, L, A> {
    /// Stable order in the certified graph.
    pub(crate) certified_order: usize,
    /// Settlement candidate selected by the complete observation scan.
    pub(crate) settlement: Option<(usize, S)>,
    /// Ready local work, if any.
    pub(crate) local: Option<(LocalAction, L)>,
    /// Legal live access, if any.
    pub(crate) access: Option<(usize, AccessAction, A)>,
    /// Whether the occurrence is semantically terminal.
    pub(crate) terminal: bool,
}

/// Integrity finding that must block before normal candidate ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegrityBlock<I> {
    /// Stable order in the certified graph.
    pub(crate) certified_order: usize,
    /// Journal order within the occurrence, when observation-derived.
    pub(crate) observation_order: usize,
    /// Opaque integrity detail retained only inside the runtime.
    pub(crate) detail: I,
}

/// Complete input to the closed scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecisionInput<S, L, A, I> {
    /// Candidates in any input order; certified order is explicit.
    pub(crate) occurrences: Vec<OccurrenceCandidate<S, L, A>>,
    /// Integrity findings derived before ranking.
    pub(crate) integrity_blocks: Vec<IntegrityBlock<I>>,
    /// An operational prerequisite that prevents otherwise legal work.
    pub(crate) operationally_blocked: bool,
    /// Whether semantic closure already exists.
    pub(crate) closed: bool,
    /// Integrity detail for the impossible open/all-terminal fold state.
    pub(crate) open_all_terminal: Option<I>,
}

/// One selected scheduler action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectedAction<S, L, A, I> {
    /// Commit the first callback-accepted observation.
    Settle {
        /// Certified node order.
        certified_order: usize,
        /// Journal order within the occurrence.
        observation_order: usize,
        /// Settlement candidate.
        candidate: S,
    },
    /// Commit ready local work.
    Local {
        /// Certified node order.
        certified_order: usize,
        /// Closed local action family.
        action: LocalAction,
        /// Action payload.
        candidate: L,
    },
    /// Authorize and perform one live protocol operation.
    Access {
        /// Certified node order.
        certified_order: usize,
        /// Number of matching committed authorizations.
        authorization_count: usize,
        /// Closed access family.
        action: AccessAction,
        /// Action payload.
        candidate: A,
    },
    /// Semantic closure already exists.
    Closed,
    /// Invalid committed evidence or another integrity failure.
    IntegrityBlocked {
        /// Opaque internal detail.
        detail: I,
    },
    /// A deployment or other operational prerequisite is unavailable.
    OperationallyBlocked,
    /// No action is currently available.
    Waiting,
}

/// Selects exactly one action using the frozen scheduler priority.
pub(crate) fn select_action<S, L, A, I>(
    mut input: DecisionInput<S, L, A, I>,
) -> SelectedAction<S, L, A, I> {
    if let Some(block) = input.integrity_blocks.into_iter().min_by(|left, right| {
        (left.certified_order, left.observation_order)
            .cmp(&(right.certified_order, right.observation_order))
    }) {
        return SelectedAction::IntegrityBlocked {
            detail: block.detail,
        };
    }

    if input.closed {
        return SelectedAction::Closed;
    }

    if let Some(detail) = input.open_all_terminal {
        return SelectedAction::IntegrityBlocked { detail };
    }

    input
        .occurrences
        .sort_by_key(|candidate| candidate.certified_order);

    if let Some((certified_order, observation_order, candidate)) =
        input.occurrences.iter_mut().find_map(|occurrence| {
            occurrence
                .settlement
                .take()
                .map(|(order, settlement)| (occurrence.certified_order, order, settlement))
        })
    {
        return SelectedAction::Settle {
            certified_order,
            observation_order,
            candidate,
        };
    }

    if let Some((certified_order, action, candidate)) =
        input.occurrences.iter_mut().find_map(|occurrence| {
            occurrence
                .local
                .take()
                .map(|(action, candidate)| (occurrence.certified_order, action, candidate))
        })
    {
        return SelectedAction::Local {
            certified_order,
            action,
            candidate,
        };
    }

    let selected_access = input
        .occurrences
        .iter_mut()
        .filter_map(|occurrence| {
            occurrence
                .access
                .take()
                .map(|(authorization_count, action, candidate)| {
                    (
                        authorization_count,
                        occurrence.certified_order,
                        action,
                        candidate,
                    )
                })
        })
        .min_by(|left, right| match left.0.cmp(&right.0) {
            Ordering::Equal => left.1.cmp(&right.1),
            ordering => ordering,
        });
    if let Some((authorization_count, certified_order, action, candidate)) = selected_access {
        return SelectedAction::Access {
            certified_order,
            authorization_count,
            action,
            candidate,
        };
    }

    if input.operationally_blocked {
        return SelectedAction::OperationallyBlocked;
    }

    SelectedAction::Waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occurrence(
        certified_order: usize,
    ) -> OccurrenceCandidate<&'static str, &'static str, &'static str> {
        OccurrenceCandidate {
            certified_order,
            settlement: None,
            local: None,
            access: None,
            terminal: false,
        }
    }

    #[test]
    fn scan_accepts_only_after_the_complete_insufficient_prefix() {
        assert_eq!(
            scan_observations([
                ObservationVerdict::InsufficientEvidence,
                ObservationVerdict::Settlement("winner"),
                ObservationVerdict::Settlement("later"),
            ]),
            EvidenceScan::Settlement {
                observation_order: 1,
                candidate: "winner",
            }
        );
        assert_eq!(
            scan_observations([
                ObservationVerdict::<()>::InsufficientEvidence,
                ObservationVerdict::InvalidEvidence,
                ObservationVerdict::Settlement(()),
            ]),
            EvidenceScan::InvalidEvidence {
                observation_order: 1,
            }
        );
        assert_eq!(
            scan_observations([
                ObservationVerdict::Settlement("would-have-settled"),
                ObservationVerdict::InvalidEvidence,
            ]),
            EvidenceScan::InvalidEvidence {
                observation_order: 1,
            }
        );
    }

    #[test]
    fn integrity_blocks_before_every_normal_priority_class() {
        let mut first = occurrence(0);
        first.settlement = Some((0, "settlement"));
        let mut second = occurrence(1);
        second.local = Some((LocalAction::PureSettlement, "pure"));
        second.access = Some((0, AccessAction::Read, "read"));

        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![first, second],
                integrity_blocks: vec![IntegrityBlock {
                    certified_order: 1,
                    observation_order: 2,
                    detail: "invalid",
                }],
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::IntegrityBlocked { detail: "invalid" }
        );
    }

    #[test]
    fn priority_is_settlement_then_local_then_balanced_access() {
        let mut settlement = occurrence(2);
        settlement.settlement = Some((4, "settlement"));
        let mut local = occurrence(0);
        local.local = Some((LocalAction::EffectRequest, "request"));
        let mut access = occurrence(1);
        access.access = Some((0, AccessAction::Read, "read"));

        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![access.clone(), settlement, local.clone()],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::Settle {
                certified_order: 2,
                observation_order: 4,
                candidate: "settlement",
            }
        );

        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![access.clone(), local],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::Local {
                certified_order: 0,
                action: LocalAction::EffectRequest,
                candidate: "request",
            }
        );

        let mut repeated = occurrence(0);
        repeated.access = Some((2, AccessAction::Read, "repeated"));
        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![repeated, access],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::Access {
                certified_order: 1,
                authorization_count: 0,
                action: AccessAction::Read,
                candidate: "read",
            }
        );

        let mut terminal = occurrence(0);
        terminal.terminal = true;
        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![terminal],
                integrity_blocks: Vec::<IntegrityBlock<&str>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: Some("open terminal fold"),
            }),
            SelectedAction::IntegrityBlocked {
                detail: "open terminal fold",
            }
        );
    }

    #[test]
    fn an_already_closed_view_returns_the_fixed_closure_outcome() {
        let mut terminal = occurrence(0);
        terminal.terminal = true;
        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![terminal],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: true,
                open_all_terminal: None,
            }),
            SelectedAction::Closed
        );
    }

    #[test]
    fn certified_order_breaks_ties_in_each_priority_class() {
        let mut later_settlement = occurrence(4);
        later_settlement.settlement = Some((0, "later"));
        let mut earlier_settlement = occurrence(1);
        earlier_settlement.settlement = Some((9, "earlier"));
        assert!(matches!(
            select_action(DecisionInput {
                occurrences: vec![later_settlement, earlier_settlement],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::Settle {
                certified_order: 1,
                ..
            }
        ));

        let mut later_access = occurrence(7);
        later_access.access = Some((3, AccessAction::Ensure, "later"));
        let mut earlier_access = occurrence(2);
        earlier_access.access = Some((3, AccessAction::Read, "earlier"));
        assert!(matches!(
            select_action(DecisionInput {
                occurrences: vec![later_access, earlier_access],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::Access {
                certified_order: 2,
                ..
            }
        ));
    }

    #[test]
    fn an_unobserved_crash_gap_counts_for_fair_access_selection() {
        let mut crashed = occurrence(0);
        crashed.access = Some((1, AccessAction::Read, "retry"));
        let mut never_authorized = occurrence(1);
        never_authorized.access = Some((0, AccessAction::Ensure, "first"));

        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![crashed, never_authorized],
                integrity_blocks: Vec::<IntegrityBlock<()>>::new(),
                operationally_blocked: false,
                closed: false,
                open_all_terminal: None,
            }),
            SelectedAction::Access {
                certified_order: 1,
                authorization_count: 0,
                action: AccessAction::Ensure,
                candidate: "first",
            }
        );
    }

    #[test]
    fn open_all_terminal_state_is_integrity_blocked_not_scheduled_for_closure() {
        let mut terminal = occurrence(0);
        terminal.terminal = true;

        assert_eq!(
            select_action(DecisionInput {
                occurrences: vec![terminal],
                integrity_blocks: Vec::<IntegrityBlock<&str>>::new(),
                operationally_blocked: true,
                closed: false,
                open_all_terminal: Some("open terminal fold"),
            }),
            SelectedAction::IntegrityBlocked {
                detail: "open terminal fold",
            }
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum Shape {
        Settlement,
        Pure,
        EffectRequest,
        DependencySkip,
        Read0,
        Read1,
        Ensure0,
        Terminal,
        Waiting,
    }

    const SHAPES: [Shape; 9] = [
        Shape::Settlement,
        Shape::Pure,
        Shape::EffectRequest,
        Shape::DependencySkip,
        Shape::Read0,
        Shape::Read1,
        Shape::Ensure0,
        Shape::Terminal,
        Shape::Waiting,
    ];

    #[derive(Debug, PartialEq, Eq)]
    enum Summary {
        Settlement(usize),
        Local(usize, LocalAction),
        Access(usize, usize),
        InvalidOpenTerminal,
        Waiting,
    }

    fn shaped_occurrence(certified_order: usize, shape: Shape) -> OccurrenceCandidate<(), (), ()> {
        let mut candidate = OccurrenceCandidate {
            certified_order,
            settlement: None,
            local: None,
            access: None,
            terminal: false,
        };
        match shape {
            Shape::Settlement => candidate.settlement = Some((0, ())),
            Shape::Pure => candidate.local = Some((LocalAction::PureSettlement, ())),
            Shape::EffectRequest => candidate.local = Some((LocalAction::EffectRequest, ())),
            Shape::DependencySkip => candidate.local = Some((LocalAction::DependencySkip, ())),
            Shape::Read0 => candidate.access = Some((0, AccessAction::Read, ())),
            Shape::Read1 => candidate.access = Some((1, AccessAction::Read, ())),
            Shape::Ensure0 => candidate.access = Some((0, AccessAction::Ensure, ())),
            Shape::Terminal => candidate.terminal = true,
            Shape::Waiting => {}
        }
        candidate
    }

    fn expected_summary(shapes: [Shape; 3]) -> Summary {
        if let Some(order) = shapes
            .iter()
            .position(|shape| matches!(shape, Shape::Settlement))
        {
            return Summary::Settlement(order);
        }
        if let Some((order, action)) =
            shapes
                .iter()
                .enumerate()
                .find_map(|(order, shape)| match shape {
                    Shape::Pure => Some((order, LocalAction::PureSettlement)),
                    Shape::EffectRequest => Some((order, LocalAction::EffectRequest)),
                    Shape::DependencySkip => Some((order, LocalAction::DependencySkip)),
                    _ => None,
                })
        {
            return Summary::Local(order, action);
        }
        if let Some((order, count)) = shapes
            .iter()
            .enumerate()
            .filter_map(|(order, shape)| match shape {
                Shape::Read0 | Shape::Ensure0 => Some((order, 0)),
                Shape::Read1 => Some((order, 1)),
                _ => None,
            })
            .min_by_key(|(order, count)| (*count, *order))
        {
            return Summary::Access(order, count);
        }
        if shapes.iter().all(|shape| matches!(shape, Shape::Terminal)) {
            Summary::InvalidOpenTerminal
        } else {
            Summary::Waiting
        }
    }

    #[test]
    fn exhaustive_three_occurrence_priority_matches_the_closed_oracle() {
        let mut scenarios = 0;
        for first in SHAPES {
            for second in SHAPES {
                for third in SHAPES {
                    let shapes = [first, second, third];
                    let all_terminal = shapes.iter().all(|shape| matches!(shape, Shape::Terminal));
                    let decision = select_action(DecisionInput {
                        occurrences: shapes
                            .into_iter()
                            .enumerate()
                            .map(|(order, shape)| shaped_occurrence(order, shape))
                            .collect(),
                        integrity_blocks: Vec::new(),
                        operationally_blocked: false,
                        closed: false,
                        open_all_terminal: all_terminal.then_some(()),
                    });
                    let actual = match decision {
                        SelectedAction::Settle {
                            certified_order, ..
                        } => Summary::Settlement(certified_order),
                        SelectedAction::Local {
                            certified_order,
                            action,
                            ..
                        } => Summary::Local(certified_order, action),
                        SelectedAction::Access {
                            certified_order,
                            authorization_count,
                            ..
                        } => Summary::Access(certified_order, authorization_count),
                        SelectedAction::IntegrityBlocked { .. } if all_terminal => {
                            Summary::InvalidOpenTerminal
                        }
                        SelectedAction::Waiting => Summary::Waiting,
                        other => panic!("unexpected decision for {shapes:?}: {other:?}"),
                    };
                    assert_eq!(actual, expected_summary(shapes), "{shapes:?}");
                    scenarios += 1;
                }
            }
        }
        assert_eq!(scenarios, SHAPES.len().pow(3));
    }
}
