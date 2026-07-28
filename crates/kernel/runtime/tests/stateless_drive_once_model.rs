//! Retained test-only model for the recoverability cutover's stateless scheduler contract.
//!
//! This file deliberately does not switch the production runtime. It gives the target
//! `drive_once` contract one executable conformance model before the production cutover:
//!
//! ```text
//! verified run view
//!   -> settle committed usable evidence
//!   -> commit local pure/effect-request/dependency-skip work in certified order
//!   -> authorize by committed authorization count, then certified order
//!   -> close only when every occurrence is terminal
//! ```
//!
//! The model keeps the certified graph and append-only journal in `ModelStore`. Every call folds a
//! fresh `VerifiedRunView`; the zero-sized `StatelessDriver` receives only that view. Callback
//! verdicts are represented as sealed values already attached to verified observations, so this
//! prototype tests selection, evidence consumption, exact-head compare-and-swap, and restart
//! semantics rather than executable/catalog isolation or typed callback implementations.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NodeId(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AuthorizationId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ObservationId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    Pure,
    Read,
    Effect,
    DependencySkip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrozenReadIntent {
    input_manifest_ref: u16,
    state_contract_ref: u16,
    capability_binding_ref: u16,
    capability_operation_id: u16,
    request_ref: u16,
    request_schema_ref: u16,
    response_schema_ref: u16,
}

impl FrozenReadIntent {
    fn for_node(node: NodeId) -> Self {
        let base = u16::from(node.0) * 16;
        Self {
            input_manifest_ref: base + 1,
            state_contract_ref: base + 2,
            capability_binding_ref: base + 3,
            capability_operation_id: base + 4,
            request_ref: base + 5,
            request_schema_ref: base + 6,
            response_schema_ref: base + 7,
        }
    }

    fn with_changed_request(self) -> Self {
        Self {
            request_ref: self.request_ref + 1_000,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EffectRequest {
    request_transition_ref: u16,
    executor_binding_ref: u16,
    request_ref: u16,
    request_digest: u16,
}

impl EffectRequest {
    fn for_node(node: NodeId) -> Self {
        let base = u16::from(node.0) * 16;
        Self {
            request_transition_ref: base + 8,
            executor_binding_ref: base + 9,
            request_ref: base + 10,
            request_digest: base + 11,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CertifiedNode {
    id: NodeId,
    protocol: Protocol,
    structurally_ready: bool,
    read_intent: Option<FrozenReadIntent>,
    effect_request: Option<EffectRequest>,
}

impl CertifiedNode {
    fn pure(id: NodeId) -> Self {
        Self {
            id,
            protocol: Protocol::Pure,
            structurally_ready: true,
            read_intent: None,
            effect_request: None,
        }
    }

    fn read(id: NodeId) -> Self {
        Self {
            id,
            protocol: Protocol::Read,
            structurally_ready: true,
            read_intent: Some(FrozenReadIntent::for_node(id)),
            effect_request: None,
        }
    }

    fn blocked_read(id: NodeId) -> Self {
        Self {
            structurally_ready: false,
            ..Self::read(id)
        }
    }

    fn effect(id: NodeId) -> Self {
        Self {
            id,
            protocol: Protocol::Effect,
            structurally_ready: true,
            read_intent: None,
            effect_request: Some(EffectRequest::for_node(id)),
        }
    }

    fn dependency_skip(id: NodeId) -> Self {
        Self {
            id,
            protocol: Protocol::DependencySkip,
            structurally_ready: true,
            read_intent: None,
            effect_request: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CertifiedSpec {
    nodes: Vec<CertifiedNode>,
}

impl CertifiedSpec {
    fn new(nodes: Vec<CertifiedNode>) -> Self {
        let distinct = nodes.iter().map(|node| node.id).collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), nodes.len(), "duplicate certified node id");
        Self { nodes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalTransition {
    Pure,
    EffectRequested,
    DependencySkipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessAnchor {
    Read(FrozenReadIntent),
    EnsureEffect(EffectRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntentSource {
    Candidate,
    Frozen,
    CommittedEffectRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compatibility {
    Exact,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceVerdict {
    Settlement,
    InsufficientEvidence,
    InvalidEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservationEvidence {
    Read(EvidenceVerdict),
    EffectPending,
    EffectTerminal(EvidenceVerdict),
    CrashAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JournalRecord {
    Local {
        node: NodeId,
        transition: LocalTransition,
    },
    Authorized {
        authorization: AuthorizationId,
        node: NodeId,
        anchor: AccessAnchor,
    },
    Observed {
        observation: ObservationId,
        authorization: AuthorizationId,
        compatibility: Compatibility,
        evidence: ObservationEvidence,
    },
    Settled {
        node: NodeId,
        observation: ObservationId,
    },
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodePhase {
    Unstarted,
    AwaitingEffect(EffectRequest),
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VerifiedAuthorization {
    id: AuthorizationId,
    anchor: AccessAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VerifiedObservation {
    id: ObservationId,
    authorization: AuthorizationId,
    compatibility: Compatibility,
    evidence: ObservationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedOccurrence {
    node: CertifiedNode,
    phase: NodePhase,
    frozen_read_intent: Option<FrozenReadIntent>,
    authorizations: Vec<VerifiedAuthorization>,
    observations: Vec<VerifiedObservation>,
    consumed_observation: Option<ObservationId>,
}

impl VerifiedOccurrence {
    fn authorization_count(&self) -> usize {
        self.authorizations.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedRunView {
    head: u64,
    occurrences: Vec<VerifiedOccurrence>,
    closed: bool,
}

impl VerifiedRunView {
    fn verify(spec: &CertifiedSpec, records: &[JournalRecord]) -> Result<Self, &'static str> {
        let mut view = Self {
            head: 0,
            occurrences: spec
                .nodes
                .iter()
                .cloned()
                .map(|node| VerifiedOccurrence {
                    node,
                    phase: NodePhase::Unstarted,
                    frozen_read_intent: None,
                    authorizations: Vec::new(),
                    observations: Vec::new(),
                    consumed_observation: None,
                })
                .collect(),
            closed: false,
        };

        for record in records {
            if view.closed && !matches!(record, JournalRecord::Observed { .. }) {
                return Err("semantic record follows closure");
            }
            match *record {
                JournalRecord::Local { node, transition } => {
                    let occurrence = view.occurrence_mut(node)?;
                    if occurrence.phase != NodePhase::Unstarted
                        || !occurrence.node.structurally_ready
                    {
                        return Err("local transition is not ready");
                    }
                    match (occurrence.node.protocol, transition) {
                        (Protocol::Pure, LocalTransition::Pure)
                        | (Protocol::DependencySkip, LocalTransition::DependencySkipped) => {
                            occurrence.phase = NodePhase::Terminal
                        }
                        (Protocol::Effect, LocalTransition::EffectRequested) => {
                            occurrence.phase = NodePhase::AwaitingEffect(
                                occurrence
                                    .node
                                    .effect_request
                                    .ok_or("effect request is missing")?,
                            );
                        }
                        _ => return Err("local transition violates certified protocol"),
                    }
                }
                JournalRecord::Authorized {
                    authorization,
                    node,
                    anchor,
                } => {
                    if view.authorization(authorization).is_some() {
                        return Err("duplicate authorization id");
                    }
                    let occurrence = view.occurrence_mut(node)?;
                    match (occurrence.node.protocol, occurrence.phase, anchor) {
                        (Protocol::Read, NodePhase::Unstarted, AccessAnchor::Read(intent))
                            if occurrence.node.structurally_ready
                                && occurrence.node.read_intent == Some(intent) =>
                        {
                            if let Some(frozen) = occurrence.frozen_read_intent {
                                if frozen != intent {
                                    return Err("read authorization changed frozen intent");
                                }
                            } else {
                                occurrence.frozen_read_intent = Some(intent);
                            }
                        }
                        (
                            Protocol::Effect,
                            NodePhase::AwaitingEffect(request),
                            AccessAnchor::EnsureEffect(candidate),
                        ) if request == candidate => {}
                        _ => return Err("authorization is not access-eligible"),
                    }
                    occurrence.authorizations.push(VerifiedAuthorization {
                        id: authorization,
                        anchor,
                    });
                }
                JournalRecord::Observed {
                    observation,
                    authorization,
                    compatibility,
                    evidence,
                } => {
                    if view.observation(observation).is_some() {
                        return Err("duplicate observation id");
                    }
                    let (node, anchor) = view
                        .authorization_owner(authorization)
                        .ok_or("observation has no authorization")?;
                    if view
                        .occurrences
                        .iter()
                        .flat_map(|occurrence| &occurrence.observations)
                        .any(|candidate| candidate.authorization == authorization)
                    {
                        return Err("authorization already has an observation");
                    }
                    if compatibility == Compatibility::Exact
                        && !evidence_role_matches(anchor, evidence)
                    {
                        return Err("compatible observation has the wrong role");
                    }
                    view.occurrence_mut(node)?
                        .observations
                        .push(VerifiedObservation {
                            id: observation,
                            authorization,
                            compatibility,
                            evidence,
                        });
                }
                JournalRecord::Settled { node, observation } => {
                    let occurrence = view.occurrence(node)?;
                    let selected = occurrence
                        .observations
                        .iter()
                        .find(|candidate| candidate.id == observation)
                        .copied()
                        .ok_or("settlement observation is absent")?;
                    if first_usable_evidence(occurrence) != EvidenceScan::Settlement(observation)
                        || !matches!(
                            selected.evidence,
                            ObservationEvidence::Read(EvidenceVerdict::Settlement)
                                | ObservationEvidence::EffectTerminal(EvidenceVerdict::Settlement)
                        )
                    {
                        return Err("settlement did not consume first usable evidence");
                    }
                    let occurrence = view.occurrence_mut(node)?;
                    occurrence.phase = NodePhase::Terminal;
                    occurrence.consumed_observation = Some(observation);
                }
                JournalRecord::Closed => {
                    if !view
                        .occurrences
                        .iter()
                        .all(|occurrence| occurrence.phase == NodePhase::Terminal)
                    {
                        return Err("closure has a nonterminal occurrence");
                    }
                    view.closed = true;
                }
            }
            view.head += 1;
        }
        Ok(view)
    }

    fn occurrence(&self, node: NodeId) -> Result<&VerifiedOccurrence, &'static str> {
        self.occurrences
            .iter()
            .find(|occurrence| occurrence.node.id == node)
            .ok_or("unknown certified node")
    }

    fn occurrence_mut(&mut self, node: NodeId) -> Result<&mut VerifiedOccurrence, &'static str> {
        self.occurrences
            .iter_mut()
            .find(|occurrence| occurrence.node.id == node)
            .ok_or("unknown certified node")
    }

    fn authorization(&self, id: AuthorizationId) -> Option<&VerifiedAuthorization> {
        self.occurrences
            .iter()
            .flat_map(|occurrence| &occurrence.authorizations)
            .find(|authorization| authorization.id == id)
    }

    fn authorization_owner(&self, id: AuthorizationId) -> Option<(NodeId, AccessAnchor)> {
        self.occurrences.iter().find_map(|occurrence| {
            occurrence
                .authorizations
                .iter()
                .find(|authorization| authorization.id == id)
                .map(|authorization| (occurrence.node.id, authorization.anchor))
        })
    }

    fn observation(&self, id: ObservationId) -> Option<&VerifiedObservation> {
        self.occurrences
            .iter()
            .flat_map(|occurrence| &occurrence.observations)
            .find(|observation| observation.id == id)
    }

    fn next_authorization_id(&self) -> AuthorizationId {
        AuthorizationId(
            self.occurrences
                .iter()
                .flat_map(|occurrence| &occurrence.authorizations)
                .map(|authorization| authorization.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        )
    }

    fn next_observation_id(&self) -> ObservationId {
        ObservationId(
            self.occurrences
                .iter()
                .flat_map(|occurrence| &occurrence.observations)
                .map(|observation| observation.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        )
    }
}

fn evidence_role_matches(anchor: AccessAnchor, evidence: ObservationEvidence) -> bool {
    matches!(
        (anchor, evidence),
        (
            AccessAnchor::Read(_),
            ObservationEvidence::Read(_) | ObservationEvidence::CrashAmbiguous,
        ) | (
            AccessAnchor::EnsureEffect(_),
            ObservationEvidence::EffectPending
                | ObservationEvidence::EffectTerminal(_)
                | ObservationEvidence::CrashAmbiguous,
        )
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceScan {
    None,
    Settlement(ObservationId),
    Invalid(ObservationId),
}

fn first_usable_evidence(occurrence: &VerifiedOccurrence) -> EvidenceScan {
    let open_protocol = match (occurrence.node.protocol, occurrence.phase) {
        (Protocol::Read, NodePhase::Unstarted) if occurrence.node.structurally_ready => {
            Protocol::Read
        }
        (Protocol::Effect, NodePhase::AwaitingEffect(_)) => Protocol::Effect,
        _ => return EvidenceScan::None,
    };

    for observation in &occurrence.observations {
        if observation.compatibility != Compatibility::Exact {
            continue;
        }
        let verdict = match (open_protocol, observation.evidence) {
            (Protocol::Read, ObservationEvidence::Read(verdict))
            | (Protocol::Effect, ObservationEvidence::EffectTerminal(verdict)) => verdict,
            _ => continue,
        };
        match verdict {
            EvidenceVerdict::Settlement => {
                return EvidenceScan::Settlement(observation.id);
            }
            EvidenceVerdict::InvalidEvidence => {
                return EvidenceScan::Invalid(observation.id);
            }
            EvidenceVerdict::InsufficientEvidence => {}
        }
    }
    EvidenceScan::None
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DriveAction {
    Settle {
        node: NodeId,
        observation: ObservationId,
    },
    CommitPure {
        node: NodeId,
    },
    CommitEffectRequest {
        node: NodeId,
        request: EffectRequest,
    },
    CommitDependencySkip {
        node: NodeId,
    },
    Authorize {
        authorization: AuthorizationId,
        node: NodeId,
        anchor: AccessAnchor,
        source: IntentSource,
    },
    Close,
}

impl DriveAction {
    fn into_record(self) -> JournalRecord {
        match self {
            Self::Settle { node, observation } => JournalRecord::Settled { node, observation },
            Self::CommitPure { node } => JournalRecord::Local {
                node,
                transition: LocalTransition::Pure,
            },
            Self::CommitEffectRequest { node, .. } => JournalRecord::Local {
                node,
                transition: LocalTransition::EffectRequested,
            },
            Self::CommitDependencySkip { node } => JournalRecord::Local {
                node,
                transition: LocalTransition::DependencySkipped,
            },
            Self::Authorize {
                authorization,
                node,
                anchor,
                ..
            } => JournalRecord::Authorized {
                authorization,
                node,
                anchor,
            },
            Self::Close => JournalRecord::Closed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DriveDecision {
    Act(DriveAction),
    Waiting,
    BlockedIntegrity {
        node: NodeId,
        observation: ObservationId,
    },
    Closed,
}

#[derive(Debug, Default, Clone, Copy)]
struct StatelessDriver;

impl StatelessDriver {
    fn drive_once(self, view: &VerifiedRunView) -> DriveDecision {
        if view.closed {
            return DriveDecision::Closed;
        }

        let mut first_settlement = None;
        for occurrence in &view.occurrences {
            match first_usable_evidence(occurrence) {
                EvidenceScan::Settlement(observation) => {
                    first_settlement.get_or_insert((occurrence.node.id, observation));
                }
                EvidenceScan::Invalid(observation) => {
                    return DriveDecision::BlockedIntegrity {
                        node: occurrence.node.id,
                        observation,
                    };
                }
                EvidenceScan::None => {}
            }
        }
        if let Some((node, observation)) = first_settlement {
            return DriveDecision::Act(DriveAction::Settle { node, observation });
        }

        for occurrence in &view.occurrences {
            if occurrence.phase != NodePhase::Unstarted || !occurrence.node.structurally_ready {
                continue;
            }
            let action = match occurrence.node.protocol {
                Protocol::Pure => Some(DriveAction::CommitPure {
                    node: occurrence.node.id,
                }),
                Protocol::Effect => Some(DriveAction::CommitEffectRequest {
                    node: occurrence.node.id,
                    request: occurrence
                        .node
                        .effect_request
                        .expect("verified effect request"),
                }),
                Protocol::DependencySkip => Some(DriveAction::CommitDependencySkip {
                    node: occurrence.node.id,
                }),
                Protocol::Read => None,
            };
            if let Some(action) = action {
                return DriveDecision::Act(action);
            }
        }

        let next_authorization = view.next_authorization_id();
        let mut access_candidates = Vec::new();
        for (certified_order, occurrence) in view.occurrences.iter().enumerate() {
            let candidate = match (occurrence.node.protocol, occurrence.phase) {
                (Protocol::Read, NodePhase::Unstarted) if occurrence.node.structurally_ready => {
                    let (intent, source) = occurrence
                        .frozen_read_intent
                        .map(|intent| (intent, IntentSource::Frozen))
                        .unwrap_or_else(|| {
                            (
                                occurrence
                                    .node
                                    .read_intent
                                    .expect("verified read intent candidate"),
                                IntentSource::Candidate,
                            )
                        });
                    Some((AccessAnchor::Read(intent), source))
                }
                (Protocol::Effect, NodePhase::AwaitingEffect(request)) => Some((
                    AccessAnchor::EnsureEffect(request),
                    IntentSource::CommittedEffectRequest,
                )),
                _ => None,
            };
            if let Some((anchor, source)) = candidate {
                access_candidates.push((
                    occurrence.authorization_count(),
                    certified_order,
                    occurrence.node.id,
                    anchor,
                    source,
                ));
            }
        }
        if let Some((_, _, node, anchor, source)) = access_candidates
            .into_iter()
            .min_by_key(|(count, order, ..)| (*count, *order))
        {
            return DriveDecision::Act(DriveAction::Authorize {
                authorization: next_authorization,
                node,
                anchor,
                source,
            });
        }

        if view
            .occurrences
            .iter()
            .all(|occurrence| occurrence.phase == NodePhase::Terminal)
        {
            DriveDecision::Act(DriveAction::Close)
        } else {
            DriveDecision::Waiting
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitError {
    Stale { expected: u64, actual: u64 },
    Invalid,
}

#[derive(Debug, PartialEq, Eq)]
struct AccessPermit {
    authorization: AuthorizationId,
}

#[derive(Debug, PartialEq, Eq)]
enum CommitReceipt {
    Advanced,
    Authorized(AccessPermit),
}

#[derive(Debug, Clone)]
struct ModelStore {
    spec: CertifiedSpec,
    records: Vec<JournalRecord>,
}

impl ModelStore {
    fn new(spec: CertifiedSpec) -> Self {
        Self {
            spec,
            records: Vec::new(),
        }
    }

    fn restart(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            records: self.records.clone(),
        }
    }

    fn head(&self) -> u64 {
        self.records.len() as u64
    }

    fn verified_view(&self) -> VerifiedRunView {
        VerifiedRunView::verify(&self.spec, &self.records).expect("valid model journal")
    }

    fn commit_action(
        &mut self,
        expected_head: u64,
        action: DriveAction,
    ) -> Result<CommitReceipt, CommitError> {
        if expected_head != self.head() {
            return Err(CommitError::Stale {
                expected: expected_head,
                actual: self.head(),
            });
        }
        if StatelessDriver.drive_once(&self.verified_view()) != DriveDecision::Act(action.clone()) {
            return Err(CommitError::Invalid);
        }
        let authorization = match action {
            DriveAction::Authorize { authorization, .. } => Some(authorization),
            _ => None,
        };
        self.try_append_record(expected_head, action.into_record())?;
        Ok(
            authorization.map_or(CommitReceipt::Advanced, |authorization| {
                CommitReceipt::Authorized(AccessPermit { authorization })
            }),
        )
    }

    fn observe(
        &mut self,
        expected_head: u64,
        permit: AccessPermit,
        compatibility: Compatibility,
        evidence: ObservationEvidence,
    ) -> Result<ObservationId, CommitError> {
        let observation = self.verified_view().next_observation_id();
        self.try_append_record(
            expected_head,
            JournalRecord::Observed {
                observation,
                authorization: permit.authorization,
                compatibility,
                evidence,
            },
        )?;
        Ok(observation)
    }

    fn try_append_record(
        &mut self,
        expected_head: u64,
        record: JournalRecord,
    ) -> Result<(), CommitError> {
        if expected_head != self.head() {
            return Err(CommitError::Stale {
                expected: expected_head,
                actual: self.head(),
            });
        }
        let mut candidate = self.records.clone();
        candidate.push(record);
        VerifiedRunView::verify(&self.spec, &candidate).map_err(|_| CommitError::Invalid)?;
        self.records = candidate;
        Ok(())
    }

    fn fixture_local(&mut self, node: NodeId, transition: LocalTransition) {
        self.try_append_record(self.head(), JournalRecord::Local { node, transition })
            .expect("fixture local transition");
    }

    fn fixture_authorize(&mut self, node: NodeId) -> AccessPermit {
        let view = self.verified_view();
        let occurrence = view.occurrence(node).expect("fixture occurrence");
        let anchor = match (occurrence.node.protocol, occurrence.phase) {
            (Protocol::Read, NodePhase::Unstarted) => AccessAnchor::Read(
                occurrence
                    .frozen_read_intent
                    .or(occurrence.node.read_intent)
                    .expect("fixture read intent"),
            ),
            (Protocol::Effect, NodePhase::AwaitingEffect(request)) => {
                AccessAnchor::EnsureEffect(request)
            }
            _ => panic!("fixture occurrence is not access-eligible"),
        };
        let authorization = view.next_authorization_id();
        self.try_append_record(
            self.head(),
            JournalRecord::Authorized {
                authorization,
                node,
                anchor,
            },
        )
        .expect("fixture authorization");
        AccessPermit { authorization }
    }

    fn fixture_observe(
        &mut self,
        permit: AccessPermit,
        compatibility: Compatibility,
        evidence: ObservationEvidence,
    ) -> ObservationId {
        self.observe(self.head(), permit, compatibility, evidence)
            .expect("fixture observation")
    }
}

fn authorized(receipt: CommitReceipt) -> AccessPermit {
    let CommitReceipt::Authorized(permit) = receipt else {
        panic!("authorization receipt expected");
    };
    permit
}

fn crash_process<T>(_transient_process_value: T) {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Terminal,
    Settlement,
    Pure,
    EffectRequest,
    DependencySkip,
    Read0,
    Read1,
    Read2,
    Effect0,
    Effect1,
    Effect2,
    Idle,
}

const SHAPES: [Shape; 12] = [
    Shape::Terminal,
    Shape::Settlement,
    Shape::Pure,
    Shape::EffectRequest,
    Shape::DependencySkip,
    Shape::Read0,
    Shape::Read1,
    Shape::Read2,
    Shape::Effect0,
    Shape::Effect1,
    Shape::Effect2,
    Shape::Idle,
];

fn node_for_shape(id: NodeId, shape: Shape) -> CertifiedNode {
    match shape {
        Shape::Terminal | Shape::Pure => CertifiedNode::pure(id),
        Shape::Settlement | Shape::Read0 | Shape::Read1 | Shape::Read2 => CertifiedNode::read(id),
        Shape::EffectRequest | Shape::Effect0 | Shape::Effect1 | Shape::Effect2 => {
            CertifiedNode::effect(id)
        }
        Shape::DependencySkip => CertifiedNode::dependency_skip(id),
        Shape::Idle => CertifiedNode::blocked_read(id),
    }
}

fn scenario_store(shapes: [Shape; 3]) -> ModelStore {
    let spec = CertifiedSpec::new(
        shapes
            .iter()
            .enumerate()
            .map(|(index, shape)| node_for_shape(NodeId(index as u8), *shape))
            .collect(),
    );
    let mut store = ModelStore::new(spec);
    for (index, shape) in shapes.into_iter().enumerate() {
        let node = NodeId(index as u8);
        match shape {
            Shape::Terminal => store.fixture_local(node, LocalTransition::Pure),
            Shape::Settlement => {
                let permit = store.fixture_authorize(node);
                store.fixture_observe(
                    permit,
                    Compatibility::Exact,
                    ObservationEvidence::Read(EvidenceVerdict::Settlement),
                );
            }
            Shape::Read1 => {
                let permit = store.fixture_authorize(node);
                store.fixture_observe(
                    permit,
                    Compatibility::Exact,
                    ObservationEvidence::Read(EvidenceVerdict::InsufficientEvidence),
                );
            }
            Shape::Read2 => {
                let _unmatched = store.fixture_authorize(node);
                let permit = store.fixture_authorize(node);
                store.fixture_observe(
                    permit,
                    Compatibility::Exact,
                    ObservationEvidence::Read(EvidenceVerdict::InsufficientEvidence),
                );
            }
            Shape::Effect0 | Shape::Effect1 | Shape::Effect2 => {
                store.fixture_local(node, LocalTransition::EffectRequested);
                let count = match shape {
                    Shape::Effect0 => 0,
                    Shape::Effect1 => 1,
                    Shape::Effect2 => 2,
                    _ => unreachable!(),
                };
                for ordinal in 0..count {
                    let permit = store.fixture_authorize(node);
                    if ordinal == 0 {
                        store.fixture_observe(
                            permit,
                            Compatibility::Exact,
                            ObservationEvidence::EffectPending,
                        );
                    }
                }
            }
            Shape::Pure
            | Shape::EffectRequest
            | Shape::DependencySkip
            | Shape::Read0
            | Shape::Idle => {}
        }
    }
    store
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionSummary {
    Settle(NodeId),
    Local(NodeId, LocalTransition),
    Authorize(NodeId, usize),
    Close,
    Waiting,
}

fn summarize(decision: &DriveDecision, view: &VerifiedRunView) -> DecisionSummary {
    match decision {
        DriveDecision::Act(DriveAction::Settle { node, .. }) => DecisionSummary::Settle(*node),
        DriveDecision::Act(DriveAction::CommitPure { node }) => {
            DecisionSummary::Local(*node, LocalTransition::Pure)
        }
        DriveDecision::Act(DriveAction::CommitEffectRequest { node, .. }) => {
            DecisionSummary::Local(*node, LocalTransition::EffectRequested)
        }
        DriveDecision::Act(DriveAction::CommitDependencySkip { node }) => {
            DecisionSummary::Local(*node, LocalTransition::DependencySkipped)
        }
        DriveDecision::Act(DriveAction::Authorize { node, .. }) => DecisionSummary::Authorize(
            *node,
            view.occurrence(*node)
                .expect("summarized occurrence")
                .authorization_count(),
        ),
        DriveDecision::Act(DriveAction::Close) => DecisionSummary::Close,
        DriveDecision::Waiting => DecisionSummary::Waiting,
        DriveDecision::BlockedIntegrity { .. } | DriveDecision::Closed => {
            panic!("scenario generated an unexpected terminal decision")
        }
    }
}

fn expected_summary(shapes: [Shape; 3]) -> DecisionSummary {
    if let Some(index) = shapes.iter().position(|shape| *shape == Shape::Settlement) {
        return DecisionSummary::Settle(NodeId(index as u8));
    }
    if let Some((index, transition)) =
        shapes
            .iter()
            .enumerate()
            .find_map(|(index, shape)| match shape {
                Shape::Pure => Some((index, LocalTransition::Pure)),
                Shape::EffectRequest => Some((index, LocalTransition::EffectRequested)),
                Shape::DependencySkip => Some((index, LocalTransition::DependencySkipped)),
                _ => None,
            })
    {
        return DecisionSummary::Local(NodeId(index as u8), transition);
    }
    if let Some((index, count)) = shapes
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| {
            let count = match shape {
                Shape::Read0 | Shape::Effect0 => 0,
                Shape::Read1 | Shape::Effect1 => 1,
                Shape::Read2 | Shape::Effect2 => 2,
                _ => return None,
            };
            Some((index, count))
        })
        .min_by_key(|(index, count)| (*count, *index))
    {
        return DecisionSummary::Authorize(NodeId(index as u8), count);
    }
    if shapes.iter().all(|shape| *shape == Shape::Terminal) {
        DecisionSummary::Close
    } else {
        DecisionSummary::Waiting
    }
}

#[test]
fn exhaustive_fixed_priority_matches_the_closed_oracle() {
    let mut scenarios = 0;
    for first in SHAPES {
        for second in SHAPES {
            for third in SHAPES {
                let shapes = [first, second, third];
                let store = scenario_store(shapes);
                let view = store.verified_view();
                let actual = summarize(&StatelessDriver.drive_once(&view), &view);
                assert_eq!(
                    actual,
                    expected_summary(shapes),
                    "priority mismatch for {shapes:?}"
                );
                scenarios += 1;
            }
        }
    }
    assert_eq!(scenarios, SHAPES.len().pow(3));
}

#[test]
fn evidence_scan_ignores_unmatched_incompatible_and_non_consumable_audit_records() {
    let node = NodeId(0);
    let mut store = ModelStore::new(CertifiedSpec::new(vec![CertifiedNode::read(node)]));

    let _unmatched = store.fixture_authorize(node);

    let incompatible = store.fixture_authorize(node);
    store.fixture_observe(
        incompatible,
        Compatibility::Incompatible,
        ObservationEvidence::Read(EvidenceVerdict::InvalidEvidence),
    );

    let crash_ambiguous = store.fixture_authorize(node);
    store.fixture_observe(
        crash_ambiguous,
        Compatibility::Exact,
        ObservationEvidence::CrashAmbiguous,
    );

    let insufficient = store.fixture_authorize(node);
    let insufficient_observation = store.fixture_observe(
        insufficient,
        Compatibility::Exact,
        ObservationEvidence::Read(EvidenceVerdict::InsufficientEvidence),
    );

    let winner = store.fixture_authorize(node);
    let winning_observation = store.fixture_observe(
        winner,
        Compatibility::Exact,
        ObservationEvidence::Read(EvidenceVerdict::Settlement),
    );

    let later = store.fixture_authorize(node);
    let later_observation = store.fixture_observe(
        later,
        Compatibility::Exact,
        ObservationEvidence::Read(EvidenceVerdict::Settlement),
    );

    let view = store.verified_view();
    assert_eq!(view.occurrence(node).unwrap().authorization_count(), 6);
    assert_eq!(
        StatelessDriver.drive_once(&view),
        DriveDecision::Act(DriveAction::Settle {
            node,
            observation: winning_observation,
        })
    );
    assert_ne!(winning_observation, insufficient_observation);
    assert_ne!(winning_observation, later_observation);
}

#[test]
fn two_driver_authorization_race_reloads_rebalances_and_preserves_frozen_intent() {
    let first = NodeId(0);
    let second = NodeId(1);
    let mut store = ModelStore::new(CertifiedSpec::new(vec![
        CertifiedNode::read(first),
        CertifiedNode::read(second),
    ]));
    let driver_a_view = store.verified_view();
    let driver_b_view = driver_a_view.clone();
    let driver_a_action = match StatelessDriver.drive_once(&driver_a_view) {
        DriveDecision::Act(action @ DriveAction::Authorize { node, source, .. }) => {
            assert_eq!(node, first);
            assert_eq!(source, IntentSource::Candidate);
            action
        }
        decision => panic!("first driver did not authorize: {decision:?}"),
    };
    let driver_b_action = match StatelessDriver.drive_once(&driver_b_view) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("second driver did not authorize: {decision:?}"),
    };
    assert_eq!(driver_a_action, driver_b_action);

    let winner = store
        .commit_action(driver_a_view.head, driver_a_action)
        .expect("winning authorization");
    assert!(matches!(winner, CommitReceipt::Authorized(_)));
    assert_eq!(
        store.commit_action(driver_b_view.head, driver_b_action),
        Err(CommitError::Stale {
            expected: driver_b_view.head,
            actual: store.head(),
        })
    );

    let reloaded = store.verified_view();
    let first_occurrence = reloaded.occurrence(first).unwrap();
    assert_eq!(first_occurrence.authorization_count(), 1);
    assert_eq!(
        first_occurrence.frozen_read_intent,
        first_occurrence.node.read_intent
    );
    assert!(first_occurrence.node.structurally_ready);
    let rebalanced_action = match StatelessDriver.drive_once(&reloaded) {
        DriveDecision::Act(
            action @ DriveAction::Authorize {
                node,
                source,
                anchor: AccessAnchor::Read(intent),
                ..
            },
        ) => {
            assert_eq!(node, second, "CAS loser must reload and re-rank");
            assert_eq!(source, IntentSource::Candidate);
            assert_eq!(intent, FrozenReadIntent::for_node(second));
            action
        }
        decision => panic!("reloaded driver did not rebalance: {decision:?}"),
    };
    store
        .commit_action(reloaded.head, rebalanced_action)
        .expect("unrelated sibling authorization");

    let after_interleaving = store.verified_view();
    let first_occurrence = after_interleaving.occurrence(first).unwrap();
    assert_eq!(
        first_occurrence.frozen_read_intent,
        Some(FrozenReadIntent::for_node(first))
    );
    assert!(first_occurrence.node.structurally_ready);

    let wrong_intent = first_occurrence
        .frozen_read_intent
        .unwrap()
        .with_changed_request();
    let head = store.head();
    assert_eq!(
        store.try_append_record(
            head,
            JournalRecord::Authorized {
                authorization: after_interleaving.next_authorization_id(),
                node: first,
                anchor: AccessAnchor::Read(wrong_intent),
            },
        ),
        Err(CommitError::Invalid)
    );
    assert_eq!(store.head(), head);
}

#[test]
fn stale_settlement_loser_becomes_audit_only_and_does_not_delay_closure() {
    let node = NodeId(0);
    let mut store = ModelStore::new(CertifiedSpec::new(vec![CertifiedNode::read(node)]));
    let winner_permit = store.fixture_authorize(node);
    let losing_permit = store.fixture_authorize(node);
    let winner_observation = store.fixture_observe(
        winner_permit,
        Compatibility::Exact,
        ObservationEvidence::Read(EvidenceVerdict::Settlement),
    );

    let driver_a_view = store.verified_view();
    let driver_b_view = driver_a_view.clone();
    let action = DriveAction::Settle {
        node,
        observation: winner_observation,
    };
    assert_eq!(
        StatelessDriver.drive_once(&driver_a_view),
        DriveDecision::Act(action.clone())
    );
    assert_eq!(
        StatelessDriver.drive_once(&driver_b_view),
        DriveDecision::Act(action.clone())
    );
    store
        .commit_action(driver_a_view.head, action.clone())
        .expect("winning settlement");
    assert_eq!(
        store.commit_action(driver_b_view.head, action),
        Err(CommitError::Stale {
            expected: driver_b_view.head,
            actual: store.head(),
        })
    );

    let terminal_view = store.verified_view();
    assert_eq!(
        terminal_view.occurrence(node).unwrap().consumed_observation,
        Some(winner_observation)
    );
    assert_eq!(
        StatelessDriver.drive_once(&terminal_view),
        DriveDecision::Act(DriveAction::Close),
        "an unmatched pre-closure authorization is not a semantic phase"
    );
    store
        .commit_action(terminal_view.head, DriveAction::Close)
        .expect("semantic closure");

    let losing_observation = store
        .observe(
            store.head(),
            losing_permit,
            Compatibility::Exact,
            ObservationEvidence::Read(EvidenceVerdict::Settlement),
        )
        .expect("late audit observation");
    let closed = store.verified_view();
    assert_eq!(StatelessDriver.drive_once(&closed), DriveDecision::Closed);
    assert_eq!(
        closed.occurrence(node).unwrap().consumed_observation,
        Some(winner_observation)
    );
    assert_ne!(losing_observation, winner_observation);
}

#[test]
fn pending_and_insufficient_effect_evidence_keep_the_exact_request_awaiting() {
    let node = NodeId(0);
    let request = EffectRequest::for_node(node);
    let mut store = ModelStore::new(CertifiedSpec::new(vec![CertifiedNode::effect(node)]));

    let initial = store.verified_view();
    assert_eq!(
        StatelessDriver.drive_once(&initial),
        DriveDecision::Act(DriveAction::CommitEffectRequest { node, request })
    );
    store
        .commit_action(
            initial.head,
            DriveAction::CommitEffectRequest { node, request },
        )
        .expect("effect request");

    let pending_authorization_view = store.verified_view();
    let pending_action = match StatelessDriver.drive_once(&pending_authorization_view) {
        DriveDecision::Act(
            action @ DriveAction::Authorize {
                node: selected,
                anchor: AccessAnchor::EnsureEffect(anchor),
                source,
                ..
            },
        ) => {
            assert_eq!(selected, node);
            assert_eq!(anchor, request);
            assert_eq!(source, IntentSource::CommittedEffectRequest);
            action
        }
        decision => panic!("effect was not authorized: {decision:?}"),
    };
    let pending_permit = authorized(
        store
            .commit_action(pending_authorization_view.head, pending_action)
            .expect("pending authorization"),
    );
    store
        .observe(
            store.head(),
            pending_permit,
            Compatibility::Exact,
            ObservationEvidence::EffectPending,
        )
        .expect("pending evidence");

    let insufficient_action = match StatelessDriver.drive_once(&store.verified_view()) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("pending evidence incorrectly settled: {decision:?}"),
    };
    let insufficient_permit = authorized(
        store
            .commit_action(store.head(), insufficient_action)
            .expect("insufficient authorization"),
    );
    store
        .observe(
            store.head(),
            insufficient_permit,
            Compatibility::Exact,
            ObservationEvidence::EffectTerminal(EvidenceVerdict::InsufficientEvidence),
        )
        .expect("insufficient terminal evidence");

    let terminal_action = match StatelessDriver.drive_once(&store.verified_view()) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("insufficient evidence incorrectly settled: {decision:?}"),
    };
    let terminal_permit = authorized(
        store
            .commit_action(store.head(), terminal_action)
            .expect("terminal authorization"),
    );
    let terminal_observation = store
        .observe(
            store.head(),
            terminal_permit,
            Compatibility::Exact,
            ObservationEvidence::EffectTerminal(EvidenceVerdict::Settlement),
        )
        .expect("terminal evidence");

    let view = store.verified_view();
    assert_eq!(
        view.occurrence(node).unwrap().phase,
        NodePhase::AwaitingEffect(request)
    );
    assert_eq!(
        StatelessDriver.drive_once(&view),
        DriveDecision::Act(DriveAction::Settle {
            node,
            observation: terminal_observation,
        })
    );
}

#[test]
fn invalid_usable_evidence_blocks_before_other_priority_classes() {
    let settlement_node = NodeId(0);
    let invalid_node = NodeId(1);
    let local_node = NodeId(2);
    let mut store = ModelStore::new(CertifiedSpec::new(vec![
        CertifiedNode::read(settlement_node),
        CertifiedNode::read(invalid_node),
        CertifiedNode::pure(local_node),
    ]));
    let settlement_permit = store.fixture_authorize(settlement_node);
    store.fixture_observe(
        settlement_permit,
        Compatibility::Exact,
        ObservationEvidence::Read(EvidenceVerdict::Settlement),
    );
    let invalid_permit = store.fixture_authorize(invalid_node);
    let invalid_observation = store.fixture_observe(
        invalid_permit,
        Compatibility::Exact,
        ObservationEvidence::Read(EvidenceVerdict::InvalidEvidence),
    );

    assert_eq!(
        StatelessDriver.drive_once(&store.verified_view()),
        DriveDecision::BlockedIntegrity {
            node: invalid_node,
            observation: invalid_observation,
        }
    );
}

#[test]
fn fresh_process_recovers_after_every_step_without_process_semantic_state() {
    assert_eq!(std::mem::size_of::<StatelessDriver>(), 0);

    let pure = NodeId(0);
    let effect = NodeId(1);
    let skipped = NodeId(2);
    let request = EffectRequest::for_node(effect);
    let mut store = ModelStore::new(CertifiedSpec::new(vec![
        CertifiedNode::pure(pure),
        CertifiedNode::effect(effect),
        CertifiedNode::dependency_skip(skipped),
    ]));

    let pure_view = store.verified_view();
    let pure_action = DriveAction::CommitPure { node: pure };
    assert_eq!(
        StatelessDriver.drive_once(&pure_view),
        DriveDecision::Act(pure_action.clone())
    );
    store
        .commit_action(pure_view.head, pure_action)
        .expect("pure transition");
    store = store.restart();

    let request_view = store.verified_view();
    let request_action = DriveAction::CommitEffectRequest {
        node: effect,
        request,
    };
    assert_eq!(
        StatelessDriver.drive_once(&request_view),
        DriveDecision::Act(request_action.clone())
    );
    store
        .commit_action(request_view.head, request_action)
        .expect("effect request transition");
    store = store.restart();

    let skip_view = store.verified_view();
    let skip_action = DriveAction::CommitDependencySkip { node: skipped };
    assert_eq!(
        StatelessDriver.drive_once(&skip_view),
        DriveDecision::Act(skip_action.clone())
    );
    store
        .commit_action(skip_view.head, skip_action)
        .expect("dependency skip");
    store = store.restart();

    let first_authorization_view = store.verified_view();
    let first_authorization = match StatelessDriver.drive_once(&first_authorization_view) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("missing first ensure authorization: {decision:?}"),
    };
    let first_permit = authorized(
        store
            .commit_action(first_authorization_view.head, first_authorization)
            .expect("first ensure authorization"),
    );
    crash_process(first_permit); // No process state can recreate its authority.
    store = store.restart();

    let returned_but_lost_view = store.verified_view();
    let returned_but_lost = match StatelessDriver.drive_once(&returned_but_lost_view) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("missing retry after unmatched authorization: {decision:?}"),
    };
    let returned_but_lost_permit = authorized(
        store
            .commit_action(returned_but_lost_view.head, returned_but_lost)
            .expect("second ensure authorization"),
    );
    crash_process(returned_but_lost_permit); // Live return was lost before observation append.
    store = store.restart();

    let pending_view = store.verified_view();
    let pending_action = match StatelessDriver.drive_once(&pending_view) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("missing pending ensure authorization: {decision:?}"),
    };
    let pending_permit = authorized(
        store
            .commit_action(pending_view.head, pending_action)
            .expect("pending ensure authorization"),
    );
    store
        .observe(
            store.head(),
            pending_permit,
            Compatibility::Exact,
            ObservationEvidence::EffectPending,
        )
        .expect("pending observation");
    store = store.restart();

    let terminal_view = store.verified_view();
    let terminal_action = match StatelessDriver.drive_once(&terminal_view) {
        DriveDecision::Act(action @ DriveAction::Authorize { .. }) => action,
        decision => panic!("missing terminal ensure authorization: {decision:?}"),
    };
    let terminal_permit = authorized(
        store
            .commit_action(terminal_view.head, terminal_action)
            .expect("terminal ensure authorization"),
    );
    let terminal_observation = store
        .observe(
            store.head(),
            terminal_permit,
            Compatibility::Exact,
            ObservationEvidence::EffectTerminal(EvidenceVerdict::Settlement),
        )
        .expect("terminal observation");
    store = store.restart();

    let settle_view = store.verified_view();
    let settle_action = DriveAction::Settle {
        node: effect,
        observation: terminal_observation,
    };
    assert_eq!(
        StatelessDriver.drive_once(&settle_view),
        DriveDecision::Act(settle_action.clone())
    );
    store
        .commit_action(settle_view.head, settle_action)
        .expect("effect settlement");
    store = store.restart();

    let close_view = store.verified_view();
    assert!(close_view
        .occurrences
        .iter()
        .all(|occurrence| occurrence.phase == NodePhase::Terminal));
    assert_eq!(
        StatelessDriver.drive_once(&close_view),
        DriveDecision::Act(DriveAction::Close)
    );
    store
        .commit_action(close_view.head, DriveAction::Close)
        .expect("run closure");
    store = store.restart();

    assert_eq!(
        StatelessDriver.drive_once(&store.verified_view()),
        DriveDecision::Closed
    );
}
