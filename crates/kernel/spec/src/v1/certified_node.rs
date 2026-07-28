use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{ContentRef, FieldPath, NodeId, StableId};
use mfm_values::RetainedValueContract;
use serde::{Deserialize, Serialize};

use crate::{Result, SpecError};

use super::CanonicalExpansionPath;

/// Exact source relation retained by a certified node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CertifiedSourceSelector {
    /// One field of the admitted entry-point input.
    RunAdmission {
        /// `None` selects the complete admitted value.
        source_field_path: Option<FieldPath>,
    },
    /// One field of the admitted configuration tree.
    Config {
        /// `None` selects the complete configured value.
        source_field_path: Option<FieldPath>,
    },
    /// One registration-declared admitted support root.
    QualifiedSupport {
        /// Exact member path in the sealed admitted support graph.
        member_path: FieldPath,
        /// `None` selects the complete support root.
        source_field_path: Option<FieldPath>,
    },
    /// One field of the admitted seed tree.
    Seed {
        /// `None` selects the complete seed value.
        source_field_path: Option<FieldPath>,
    },
    /// One field of the predecessor-visible context tree.
    Context {
        /// `None` selects the complete context value.
        source_field_path: Option<FieldPath>,
    },
    /// One effective same-run output.
    NodeOutput {
        /// Final producer occurrence.
        producer_node_id: NodeId,
        /// Certified output slot.
        output_ordinal: u32,
        /// `None` selects the complete output slot.
        source_field_path: Option<FieldPath>,
    },
    /// One same-run emitted fact.
    NodeFact {
        /// Final producer occurrence.
        producer_node_id: NodeId,
        /// Certified fact slot.
        emission_ordinal: u32,
    },
    /// One admitted cross-run effective output field.
    CrossRunEffectiveOutput {
        /// `None` selects the complete effective-output root.
        source_field_path: Option<FieldPath>,
    },
    /// One deliberately admitted cross-run evidence field.
    CrossRunEvidence {
        /// `None` selects the complete evidence root.
        source_field_path: Option<FieldPath>,
        /// Evidence-only role certified for this source.
        certified_evidence_role_ref: ContentRef,
    },
}

impl CertifiedSourceSelector {
    /// Returns the selected nested source field, or `None` for the whole value.
    pub const fn source_field_path(&self) -> Option<&FieldPath> {
        match self {
            Self::RunAdmission { source_field_path }
            | Self::Config { source_field_path }
            | Self::QualifiedSupport {
                source_field_path, ..
            }
            | Self::Seed { source_field_path }
            | Self::Context { source_field_path }
            | Self::NodeOutput {
                source_field_path, ..
            }
            | Self::CrossRunEffectiveOutput { source_field_path }
            | Self::CrossRunEvidence {
                source_field_path, ..
            } => source_field_path.as_ref(),
            Self::NodeFact { .. } => None,
        }
    }

    /// Returns the direct same-run producer, if any.
    pub const fn producer_node_id(&self) -> Option<&NodeId> {
        match self {
            Self::NodeOutput {
                producer_node_id, ..
            }
            | Self::NodeFact {
                producer_node_id, ..
            } => Some(producer_node_id),
            Self::RunAdmission { .. }
            | Self::Config { .. }
            | Self::QualifiedSupport { .. }
            | Self::Seed { .. }
            | Self::Context { .. }
            | Self::CrossRunEffectiveOutput { .. }
            | Self::CrossRunEvidence { .. } => None,
        }
    }
}

/// One exact source used for a frame config or context value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedFrameBinding {
    value_contract: RetainedValueContract,
    source: CertifiedSourceSelector,
}

impl CertifiedFrameBinding {
    /// Constructs one complete frame binding.
    pub const fn new(
        value_contract: RetainedValueContract,
        source: CertifiedSourceSelector,
    ) -> Self {
        Self {
            value_contract,
            source,
        }
    }

    /// Returns the exact value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns the exact source relation.
    pub const fn source(&self) -> &CertifiedSourceSelector {
        &self.source
    }
}

/// Authority class of one state-input destination.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CertifiedInputDestination {
    /// Ordinary predecessor-visible semantic input.
    OrdinaryValue,
    /// Raw evidence with no effective-output authority.
    EvidenceOnly {
        /// Exact evidence role admitted at this destination.
        certified_evidence_role_ref: ContentRef,
    },
}

/// One complete typed input-tree binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedInputBinding {
    destination_field_path: FieldPath,
    destination: CertifiedInputDestination,
    value_contract: RetainedValueContract,
    ordered_sources: Vec<CertifiedSourceSelector>,
}

impl CertifiedInputBinding {
    /// Constructs one ordered alternative-source relation.
    pub fn new(
        destination_field_path: FieldPath,
        destination: CertifiedInputDestination,
        value_contract: RetainedValueContract,
        ordered_sources: Vec<CertifiedSourceSelector>,
    ) -> Result<Self> {
        if ordered_sources.is_empty() {
            return Err(SpecError::Invariant(
                "certified input binding must retain at least one source".to_owned(),
            ));
        }
        let unique = ordered_sources.iter().collect::<BTreeSet<_>>();
        if unique.len() != ordered_sources.len() {
            return Err(SpecError::Invariant(
                "certified input binding sources must be unique".to_owned(),
            ));
        }
        match &destination {
            CertifiedInputDestination::OrdinaryValue => {
                if ordered_sources.iter().any(|source| {
                    matches!(source, CertifiedSourceSelector::CrossRunEvidence { .. })
                }) {
                    return Err(SpecError::Invariant(
                        "cross-run evidence cannot bind an ordinary destination".to_owned(),
                    ));
                }
            }
            CertifiedInputDestination::EvidenceOnly {
                certified_evidence_role_ref,
            } => {
                if ordered_sources.iter().any(|source| {
                    !matches!(
                        source,
                        CertifiedSourceSelector::CrossRunEvidence {
                            certified_evidence_role_ref: source_role,
                            ..
                        } if source_role == certified_evidence_role_ref
                    )
                }) {
                    return Err(SpecError::Invariant(
                        "evidence-only destination requires the same certified source role"
                            .to_owned(),
                    ));
                }
            }
        }
        Ok(Self {
            destination_field_path,
            destination,
            value_contract,
            ordered_sources,
        })
    }

    /// Returns the exact destination path.
    pub const fn destination_field_path(&self) -> &FieldPath {
        &self.destination_field_path
    }

    /// Returns the destination authority class.
    pub const fn destination(&self) -> &CertifiedInputDestination {
        &self.destination
    }

    /// Returns the exact destination value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns alternative sources in certified order.
    pub fn ordered_sources(&self) -> &[CertifiedSourceSelector] {
        &self.ordered_sources
    }
}

/// Closed execution metadata certified inline for one node.
///
/// The owned contracts keep this cold certification value direct and
/// allocation-free across every consumer boundary.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CertifiedStateExecution {
    /// Pure callback with no external operation.
    Pure,
    /// One audited read operation.
    Read {
        /// Stable capability operation.
        capability_operation_id: StableId,
        /// Exact adapter/verifier binding.
        capability_binding_ref: ContentRef,
        /// Exact authored request contract.
        request_contract: RetainedValueContract,
        /// Exact returned-value contract.
        returned_contract: RetainedValueContract,
        /// Exact redaction-safe failure contract.
        safe_failure_contract: RetainedValueContract,
    },
    /// One recoverable keyed executor operation.
    Effect {
        /// Stable executor operation.
        executor_operation_id: StableId,
        /// Exact executor client/verifier binding.
        executor_binding_ref: ContentRef,
        /// Exact authored semantic request contract.
        request_contract: RetainedValueContract,
        /// Fixed verified ensure-result contract.
        ensure_result_contract: RetainedValueContract,
        /// Exact structurally verified terminal evidence contract.
        terminal_evidence_contract: RetainedValueContract,
        /// Exact state-owned domain converter result contract.
        domain_result_contract: RetainedValueContract,
    },
}

impl CertifiedStateExecution {
    /// Returns the exact external operation and binding, when present.
    pub const fn operation_binding(&self) -> Option<(&StableId, &ContentRef)> {
        match self {
            Self::Read {
                capability_operation_id,
                capability_binding_ref,
                ..
            } => Some((capability_operation_id, capability_binding_ref)),
            Self::Effect {
                executor_operation_id,
                executor_binding_ref,
                ..
            } => Some((executor_operation_id, executor_binding_ref)),
            Self::Pure => None,
        }
    }

    /// Returns the effect executor binding, when present.
    pub const fn executor_binding_ref(&self) -> Option<&ContentRef> {
        match self {
            Self::Effect {
                executor_binding_ref,
                ..
            } => Some(executor_binding_ref),
            Self::Pure | Self::Read { .. } => None,
        }
    }
}

/// One certified successful output slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedOutputSlot {
    output_ordinal: u32,
    field_path: FieldPath,
    value_contract: RetainedValueContract,
}

impl CertifiedOutputSlot {
    /// Constructs one exact output slot.
    pub const fn new(
        output_ordinal: u32,
        field_path: FieldPath,
        value_contract: RetainedValueContract,
    ) -> Self {
        Self {
            output_ordinal,
            field_path,
            value_contract,
        }
    }

    /// Returns the output ordinal.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the typed output-tree path.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the exact retained-value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }
}

/// One certified same-run fact emission slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedFactSlot {
    fact_slot_ordinal: u32,
    minimum_emissions: u32,
    maximum_emissions: u32,
    fact_descriptor_ref: ContentRef,
    subject_contract: RetainedValueContract,
    response_contract: RetainedValueContract,
}

impl CertifiedFactSlot {
    /// Constructs one exact fact slot.
    pub fn new(
        fact_slot_ordinal: u32,
        minimum_emissions: u32,
        maximum_emissions: u32,
        fact_descriptor_ref: ContentRef,
        subject_contract: RetainedValueContract,
        response_contract: RetainedValueContract,
    ) -> Result<Self> {
        if !(1..=1024).contains(&maximum_emissions) || minimum_emissions > maximum_emissions {
            return Err(SpecError::Invariant(
                "fact-slot emission bounds are invalid".to_owned(),
            ));
        }
        Ok(Self {
            fact_slot_ordinal,
            minimum_emissions,
            maximum_emissions,
            fact_descriptor_ref,
            subject_contract,
            response_contract,
        })
    }

    /// Returns the dense homogeneous fact-slot ordinal.
    pub const fn fact_slot_ordinal(&self) -> u32 {
        self.fact_slot_ordinal
    }

    /// Returns the minimum emissions required for this slot.
    pub const fn minimum_emissions(&self) -> u32 {
        self.minimum_emissions
    }

    /// Returns the maximum emissions admitted for this slot.
    pub const fn maximum_emissions(&self) -> u32 {
        self.maximum_emissions
    }

    /// Returns the exact fact descriptor.
    pub const fn fact_descriptor_ref(&self) -> &ContentRef {
        &self.fact_descriptor_ref
    }

    /// Returns the exact subject contract.
    pub const fn subject_contract(&self) -> &RetainedValueContract {
        &self.subject_contract
    }

    /// Returns the exact response contract.
    pub const fn response_contract(&self) -> &RetainedValueContract {
        &self.response_contract
    }
}

/// Complete settlement contract for one state occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedSettlementContract {
    typed_failure_contract: Option<RetainedValueContract>,
    output_slots: Vec<CertifiedOutputSlot>,
    fact_slots: Vec<CertifiedFactSlot>,
}

impl CertifiedSettlementContract {
    /// Constructs exact failure, output, and fact slots.
    pub fn new(
        typed_failure_contract: Option<RetainedValueContract>,
        mut output_slots: Vec<CertifiedOutputSlot>,
        mut fact_slots: Vec<CertifiedFactSlot>,
    ) -> Result<Self> {
        output_slots.sort_by_key(CertifiedOutputSlot::output_ordinal);
        fact_slots.sort_by_key(CertifiedFactSlot::fact_slot_ordinal);
        if output_slots
            .windows(2)
            .any(|pair| pair[0].output_ordinal == pair[1].output_ordinal)
            || fact_slots.iter().enumerate().any(|(index, slot)| {
                slot.fact_slot_ordinal != u32::try_from(index).unwrap_or(u32::MAX)
                    || !(1..=1024).contains(&slot.maximum_emissions)
                    || slot.minimum_emissions > slot.maximum_emissions
            })
            || fact_slots
                .iter()
                .map(|slot| u64::from(slot.maximum_emissions))
                .sum::<u64>()
                > 4096
        {
            return Err(SpecError::Invariant(
                "settlement ordinals or fact-slot bounds are invalid".to_owned(),
            ));
        }
        Ok(Self {
            typed_failure_contract,
            output_slots,
            fact_slots,
        })
    }

    /// Returns the optional typed-failure contract.
    pub const fn typed_failure_contract(&self) -> Option<&RetainedValueContract> {
        self.typed_failure_contract.as_ref()
    }

    /// Returns successful output slots in ordinal order.
    pub fn output_slots(&self) -> &[CertifiedOutputSlot] {
        &self.output_slots
    }

    /// Returns fact slots in emission order.
    pub fn fact_slots(&self) -> &[CertifiedFactSlot] {
        &self.fact_slots
    }

    /// Resolves one actual emission ordinal to its sole invariant fact slot.
    ///
    /// A variable-cardinality predecessor can leave actual ordinals whose
    /// semantic slot cannot be proven. Those ordinals are rejected rather
    /// than assigned by observation-time cardinality.
    pub fn invariant_fact_slot(&self, emission_ordinal: u32) -> Result<&CertifiedFactSlot> {
        let mut prior_minimum = 0_u64;
        let mut prior_maximum = 0_u64;
        let mut cores = Vec::with_capacity(self.fact_slots.len());
        for slot in &self.fact_slots {
            let invariant_end = prior_minimum
                .checked_add(u64::from(slot.minimum_emissions))
                .ok_or_else(|| {
                    SpecError::Invariant("fact minimum emission ordinal overflow".to_owned())
                })?;
            cores.push(FactInvariantCore {
                slot,
                invariant_start: prior_maximum,
                invariant_end,
            });
            prior_minimum = invariant_end;
            prior_maximum = prior_maximum
                .checked_add(u64::from(slot.maximum_emissions))
                .ok_or_else(|| {
                    SpecError::Invariant("fact maximum emission ordinal overflow".to_owned())
                })?;
        }
        select_unique_invariant_fact_slot(cores, emission_ordinal)
    }

    fn validate(&self) -> Result<()> {
        if self
            .output_slots
            .windows(2)
            .any(|pair| pair[0].output_ordinal >= pair[1].output_ordinal)
            || self.fact_slots.iter().enumerate().any(|(index, slot)| {
                slot.fact_slot_ordinal != u32::try_from(index).unwrap_or(u32::MAX)
                    || !(1..=1024).contains(&slot.maximum_emissions)
                    || slot.minimum_emissions > slot.maximum_emissions
            })
            || self
                .fact_slots
                .iter()
                .map(|slot| u64::from(slot.maximum_emissions))
                .sum::<u64>()
                > 4096
        {
            return Err(SpecError::Invariant(
                "certified settlement contract is not canonical".to_owned(),
            ));
        }
        Ok(())
    }
}

struct FactInvariantCore<'a> {
    slot: &'a CertifiedFactSlot,
    invariant_start: u64,
    invariant_end: u64,
}

fn select_unique_invariant_fact_slot<'a>(
    cores: impl IntoIterator<Item = FactInvariantCore<'a>>,
    emission_ordinal: u32,
) -> Result<&'a CertifiedFactSlot> {
    let ordinal = u64::from(emission_ordinal);
    let mut selected = None;
    for core in cores {
        if core.invariant_start <= ordinal
            && ordinal < core.invariant_end
            && selected.replace(core.slot).is_some()
        {
            return Err(SpecError::Invariant(
                "fact emission ordinal maps to multiple certified fact-slot invariant cores"
                    .to_owned(),
            ));
        }
    }
    selected.ok_or_else(|| {
        SpecError::Invariant(
            "fact emission ordinal does not fall in a certified fact-slot invariant core"
                .to_owned(),
        )
    })
}

/// Complete self-sufficient runtime contract for one expanded occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedNodeContract {
    node_id: NodeId,
    canonical_expansion_path: CanonicalExpansionPath,
    state_contract_ref: ContentRef,
    config_binding: CertifiedFrameBinding,
    context_binding: Option<CertifiedFrameBinding>,
    input_contract: RetainedValueContract,
    input_bindings: Vec<CertifiedInputBinding>,
    execution: CertifiedStateExecution,
    settlement_contract: CertifiedSettlementContract,
}

impl CertifiedNodeContract {
    /// Constructs one complete node contract and rederives its identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        canonical_expansion_path: CanonicalExpansionPath,
        state_contract_ref: ContentRef,
        config_binding: CertifiedFrameBinding,
        context_binding: Option<CertifiedFrameBinding>,
        input_contract: RetainedValueContract,
        mut input_bindings: Vec<CertifiedInputBinding>,
        execution: CertifiedStateExecution,
        settlement_contract: CertifiedSettlementContract,
    ) -> Result<Self> {
        input_bindings.sort_by(|left, right| {
            left.destination_field_path
                .cmp(&right.destination_field_path)
        });
        if input_bindings.windows(2).any(|pair| {
            paths_overlap(
                &pair[0].destination_field_path,
                &pair[1].destination_field_path,
            )
        }) {
            return Err(SpecError::Invariant(
                "certified input destinations must be unique and nonoverlapping".to_owned(),
            ));
        }
        let node_id = canonical_expansion_path.node_id(&state_contract_ref)?;
        Ok(Self {
            node_id,
            canonical_expansion_path,
            state_contract_ref,
            config_binding,
            context_binding,
            input_contract,
            input_bindings,
            execution,
            settlement_contract,
        })
    }

    /// Returns the final node identity.
    pub const fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the complete canonical expansion path.
    pub const fn canonical_expansion_path(&self) -> &CanonicalExpansionPath {
        &self.canonical_expansion_path
    }

    /// Returns the exact semantic state contract.
    pub const fn state_contract_ref(&self) -> &ContentRef {
        &self.state_contract_ref
    }

    /// Returns the required exact configuration binding.
    pub const fn config_binding(&self) -> &CertifiedFrameBinding {
        &self.config_binding
    }

    /// Returns the optional exact context binding.
    pub const fn context_binding(&self) -> Option<&CertifiedFrameBinding> {
        self.context_binding.as_ref()
    }

    /// Returns the assembled input-root contract.
    pub const fn input_contract(&self) -> &RetainedValueContract {
        &self.input_contract
    }

    /// Returns typed input bindings in destination-path order.
    pub fn input_bindings(&self) -> &[CertifiedInputBinding] {
        &self.input_bindings
    }

    /// Returns the closed execution contract.
    pub const fn execution(&self) -> &CertifiedStateExecution {
        &self.execution
    }

    /// Returns the complete settlement contract.
    pub const fn settlement_contract(&self) -> &CertifiedSettlementContract {
        &self.settlement_contract
    }

    /// Returns direct same-run producer identities for graph validation.
    ///
    /// This is not a readiness projection. Runtime evaluates every full
    /// ordered source relation so a non-node alternative can satisfy its
    /// destination.
    pub fn direct_producer_ids(&self) -> BTreeSet<NodeId> {
        std::iter::once(&self.config_binding)
            .chain(self.context_binding.iter())
            .filter_map(|binding| binding.source.producer_node_id())
            .chain(
                self.input_bindings
                    .iter()
                    .flat_map(|binding| &binding.ordered_sources)
                    .filter_map(CertifiedSourceSelector::producer_node_id),
            )
            .cloned()
            .collect()
    }

    pub(super) fn validate(&self) -> Result<()> {
        if self
            .canonical_expansion_path
            .node_id(&self.state_contract_ref)?
            != self.node_id
        {
            return Err(SpecError::Invariant(
                "certified node id does not match its preimage".to_owned(),
            ));
        }
        self.settlement_contract.validate()?;
        Ok(())
    }
}

/// Typed binding from one effective node output into a public field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedOutputBinding {
    destination_field_path: FieldPath,
    source_node_id: NodeId,
    output_ordinal: u32,
    source_field_path: Option<FieldPath>,
    value_contract: RetainedValueContract,
}

impl CertifiedOutputBinding {
    /// Constructs one exact public-output binding.
    pub const fn new(
        destination_field_path: FieldPath,
        source_node_id: NodeId,
        output_ordinal: u32,
        source_field_path: Option<FieldPath>,
        value_contract: RetainedValueContract,
    ) -> Self {
        Self {
            destination_field_path,
            source_node_id,
            output_ordinal,
            source_field_path,
            value_contract,
        }
    }

    /// Returns the public destination field.
    pub const fn destination_field_path(&self) -> &FieldPath {
        &self.destination_field_path
    }

    /// Returns the effective producer node.
    pub const fn source_node_id(&self) -> &NodeId {
        &self.source_node_id
    }

    /// Returns the producer output ordinal.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the selected producer field, or `None` for the complete output.
    pub const fn source_field_path(&self) -> Option<&FieldPath> {
        self.source_field_path.as_ref()
    }

    /// Returns the exact value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }
}

/// Exact public-output projection of one certified entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicOutputContract {
    value_contract: RetainedValueContract,
    bindings: Vec<CertifiedOutputBinding>,
}

impl PublicOutputContract {
    /// Constructs a uniquely field-bound public-output projection.
    pub fn new(
        value_contract: RetainedValueContract,
        mut bindings: Vec<CertifiedOutputBinding>,
    ) -> Result<Self> {
        bindings.sort_by(|left, right| {
            left.destination_field_path
                .cmp(&right.destination_field_path)
        });
        if bindings.is_empty()
            || bindings.windows(2).any(|pair| {
                paths_overlap(
                    &pair[0].destination_field_path,
                    &pair[1].destination_field_path,
                )
            })
        {
            return Err(SpecError::Invariant(
                "public output bindings must be nonempty and uniquely ordered".to_owned(),
            ));
        }
        Ok(Self {
            value_contract,
            bindings,
        })
    }

    /// Returns the assembled public-output value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }

    /// Returns field bindings in destination-path order.
    pub fn bindings(&self) -> &[CertifiedOutputBinding] {
        &self.bindings
    }
}

fn paths_overlap(left: &FieldPath, right: &FieldPath) -> bool {
    let left = left.as_str();
    let right = right.as_str();
    left == right
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('.'))
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

pub(super) fn validate_certified_graph(nodes: &[CertifiedNodeContract]) -> Result<()> {
    let known = nodes
        .iter()
        .map(|node| (node.node_id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    for node in nodes {
        node.validate()?;
        for producer in node.direct_producer_ids() {
            if !known.contains_key(&producer) {
                return Err(SpecError::Invariant(
                    "certified input binding names an unknown producer".to_owned(),
                ));
            }
        }
        validate_node_fact_source(
            node.config_binding.source(),
            node.config_binding.value_contract(),
            &known,
        )?;
        if let Some(binding) = &node.context_binding {
            validate_node_fact_source(binding.source(), binding.value_contract(), &known)?;
        }
        for binding in &node.input_bindings {
            for source in &binding.ordered_sources {
                validate_node_fact_source(source, binding.value_contract(), &known)?;
            }
        }
    }
    Ok(())
}

fn validate_node_fact_source(
    source: &CertifiedSourceSelector,
    destination_contract: &RetainedValueContract,
    known: &BTreeMap<NodeId, &CertifiedNodeContract>,
) -> Result<()> {
    let CertifiedSourceSelector::NodeFact {
        producer_node_id,
        emission_ordinal,
    } = source
    else {
        return Ok(());
    };
    let producer = known.get(producer_node_id).ok_or_else(|| {
        SpecError::Invariant("certified input binding names an unknown producer".to_owned())
    })?;
    let slot = producer
        .settlement_contract()
        .invariant_fact_slot(*emission_ordinal)?;
    if slot.response_contract() != destination_contract {
        return Err(SpecError::Invariant(
            "certified node-fact source differs from its destination retained-value contract"
                .to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "certified_node_tests.rs"]
mod tests;
