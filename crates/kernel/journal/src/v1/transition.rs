use mfm_canonical::CanonicalValue;
use mfm_ids::{
    ContentRef, EffectKey, FieldPath, NodeId, RequestDigest, RunSemanticStateDigest, SpecHash,
};

use super::codec::{
    array_field, content_ref_field, cv_array, cv_content_ref, cv_string, define_schema_value,
    effect_key_field, field_path_field, node_id_field, nullable_field, object, required_field,
    spec_hash_field, string_field, tagged_object, u32_field,
};
use super::{
    CapabilityBindingRef, FactEmission, InputManifestRef, JournalHead, NodePhase, ObservationRef,
    Result, RunPhase, TransitionRef, TransitionSlot, ValueRef,
};

define_schema_value! {
    /// Complete before-state proof embedded in a transition.
    pub struct TransitionBefore => "mfm.transition-before.v1";
    /// Complete closed transition body.
    pub struct TransitionBody => "mfm.transition-body.v1";
    /// Complete after-state proof embedded in a transition.
    pub struct TransitionAfter => "mfm.transition-after.v1";
    /// Successful or failed typed settlement.
    pub struct Settlement => "mfm.settlement.v1";
    /// Exact ordered semantic binding delta.
    pub struct BindingDelta => "mfm.binding-delta.v1";
    /// One closed semantic binding-delta entry.
    pub struct BindingDeltaEntry => "mfm.binding-delta-entry.v1";
    /// One output produced by a successful settlement.
    pub struct OutputBinding => "mfm.output-binding.v1";
    /// Closed destination of one unsatisfied dependency.
    pub struct BlockingDestination => "mfm.blocking-destination.v1";
    /// Closed producer requirement that could not be satisfied.
    pub struct BlockingProducerRequirement => "mfm.blocking-producer-requirement.v1";
    /// One terminal dependency blocking source.
    pub struct BlockingSource => "mfm.blocking-source.v1";
    /// Complete audited semantic transition.
    pub struct StateTransitionCommitted => "mfm.state-transition-committed.v1";
}

/// Typed fields of a transition before-state proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionBeforeFields {
    /// Exact physical predecessor head.
    pub journal_head: JournalHead,
    /// Semantic state digest before the transition.
    pub run_state_digest: RunSemanticStateDigest,
    /// Run phase before the transition.
    pub run_phase: RunPhase,
    /// Node phase before the transition.
    pub node_phase: NodePhase,
}

impl TransitionBefore {
    /// Constructs and validates a complete transition before-state proof.
    pub fn new(
        journal_head: &JournalHead,
        run_state_digest: &RunSemanticStateDigest,
        run_phase: RunPhase,
        node_phase: NodePhase,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("journal_head", journal_head.canonical_value()?),
            ("run_state_digest", cv_string(run_state_digest.as_str())),
            ("run_phase", cv_string(run_phase.as_str())),
            ("node_phase", cv_string(node_phase.as_str())),
        ])?)
    }

    /// Projects every before-state field.
    pub fn fields(&self) -> Result<TransitionBeforeFields> {
        Ok(TransitionBeforeFields {
            journal_head: JournalHead::from_canonical_value(required_field(self, "journal_head")?)?,
            run_state_digest: RunSemanticStateDigest::parse(string_field(
                self,
                "run_state_digest",
            )?)?,
            run_phase: RunPhase::parse(&string_field(self, "run_phase")?)?,
            node_phase: NodePhase::parse(&string_field(self, "node_phase")?)?,
        })
    }
}

/// Typed fields of a transition after-state proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionAfterFields {
    /// Semantic state digest after the transition.
    pub run_state_digest: RunSemanticStateDigest,
    /// Run phase after the transition.
    pub run_phase: RunPhase,
    /// Node phase after the transition.
    pub node_phase: NodePhase,
    /// Exact non-empty semantic mutation.
    pub binding_delta: BindingDelta,
}

impl TransitionAfter {
    /// Constructs and validates a complete transition after-state proof.
    pub fn new(
        run_state_digest: &RunSemanticStateDigest,
        run_phase: RunPhase,
        node_phase: NodePhase,
        binding_delta: &BindingDelta,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("run_state_digest", cv_string(run_state_digest.as_str())),
            ("run_phase", cv_string(run_phase.as_str())),
            ("node_phase", cv_string(node_phase.as_str())),
            ("binding_delta", binding_delta.canonical_value()?),
        ])?)
    }

    /// Projects every after-state field.
    pub fn fields(&self) -> Result<TransitionAfterFields> {
        Ok(TransitionAfterFields {
            run_state_digest: RunSemanticStateDigest::parse(string_field(
                self,
                "run_state_digest",
            )?)?,
            run_phase: RunPhase::parse(&string_field(self, "run_phase")?)?,
            node_phase: NodePhase::parse(&string_field(self, "node_phase")?)?,
            binding_delta: BindingDelta::from_canonical_value(required_field(
                self,
                "binding_delta",
            )?)?,
        })
    }
}

/// Typed fields of one output binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBindingFields {
    /// Output ordinal.
    pub output_ordinal: u32,
    /// Certified destination field path.
    pub field_path: FieldPath,
    /// Full retained output authority.
    pub value_ref: ValueRef,
}

impl OutputBinding {
    /// Constructs and validates one output binding.
    pub fn new(output_ordinal: u32, field_path: &FieldPath, value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "output_ordinal",
                CanonicalValue::Unsigned(u64::from(output_ordinal)),
            ),
            ("field_path", cv_string(field_path.as_str())),
            ("value_ref", value_ref.canonical_value()?),
        ])?)
    }

    /// Projects every output-binding field.
    pub fn fields(&self) -> Result<OutputBindingFields> {
        Ok(OutputBindingFields {
            output_ordinal: u32_field(self, "output_ordinal")?,
            field_path: field_path_field(self, "field_path")?,
            value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
        })
    }
}

/// Closed destination of one unsatisfied dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockingDestinationFields {
    /// Certified configuration input.
    Config,
    /// Certified context input.
    Context,
    /// One field in the certified state input.
    Input {
        /// Exact destination field.
        destination_field_path: FieldPath,
    },
}

impl BlockingDestination {
    /// Constructs a configuration destination.
    pub fn config() -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "config",
            std::iter::empty::<(&str, CanonicalValue)>(),
        )?)
    }

    /// Constructs a context destination.
    pub fn context() -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "context",
            std::iter::empty::<(&str, CanonicalValue)>(),
        )?)
    }

    /// Constructs an exact state-input destination.
    pub fn input(destination_field_path: &FieldPath) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "input",
            [(
                "destination_field_path",
                cv_string(destination_field_path.as_str()),
            )],
        )?)
    }

    /// Projects the closed blocking destination.
    pub fn fields(&self) -> Result<BlockingDestinationFields> {
        match super::codec::tag(self)?.as_str() {
            "config" => Ok(BlockingDestinationFields::Config),
            "context" => Ok(BlockingDestinationFields::Context),
            "input" => Ok(BlockingDestinationFields::Input {
                destination_field_path: field_path_field(self, "destination_field_path")?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Closed producer requirement that could not be satisfied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockingProducerRequirementFields {
    /// One producer output.
    Output {
        /// Certified output ordinal.
        output_ordinal: u32,
        /// Optional field selected from the output root.
        source_field_path: Option<FieldPath>,
    },
    /// One complete emitted fact.
    Fact {
        /// Certified emission ordinal.
        emission_ordinal: u32,
    },
}

impl BlockingProducerRequirement {
    /// Constructs one unsatisfied output requirement.
    pub fn output(output_ordinal: u32, source_field_path: Option<&FieldPath>) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "output",
            [
                (
                    "output_ordinal",
                    CanonicalValue::Unsigned(u64::from(output_ordinal)),
                ),
                (
                    "source_field_path",
                    source_field_path
                        .map(|path| cv_string(path.as_str()))
                        .unwrap_or(CanonicalValue::Null),
                ),
            ],
        )?)
    }

    /// Constructs one unsatisfied fact requirement.
    pub fn fact(emission_ordinal: u32) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "fact",
            [(
                "emission_ordinal",
                CanonicalValue::Unsigned(u64::from(emission_ordinal)),
            )],
        )?)
    }

    /// Projects the closed producer requirement.
    pub fn fields(&self) -> Result<BlockingProducerRequirementFields> {
        match super::codec::tag(self)?.as_str() {
            "output" => Ok(BlockingProducerRequirementFields::Output {
                output_ordinal: u32_field(self, "output_ordinal")?,
                source_field_path: nullable_field(self, "source_field_path")?
                    .map(|value| match value {
                        CanonicalValue::String(value) => FieldPath::new(value).map_err(Into::into),
                        _ => Err(super::JournalError::Projection),
                    })
                    .transpose()?,
            }),
            "fact" => Ok(BlockingProducerRequirementFields::Fact {
                emission_ordinal: u32_field(self, "emission_ordinal")?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of one dependency blocking source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingSourceFields {
    /// Terminal producer node.
    pub producer_node_id: NodeId,
    /// Producer terminal transition.
    pub producer_terminal_transition_ref: TransitionRef,
    /// Exact blocked destination.
    pub destination: BlockingDestination,
    /// Exact producer requirement that remained unavailable.
    pub requirement: BlockingProducerRequirement,
}

impl BlockingSource {
    /// Constructs and validates one dependency blocking source.
    pub fn new(
        producer_node_id: &NodeId,
        producer_terminal_transition_ref: &TransitionRef,
        destination: &BlockingDestination,
        requirement: &BlockingProducerRequirement,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("producer_node_id", cv_string(producer_node_id.as_str())),
            (
                "producer_terminal_transition_ref",
                producer_terminal_transition_ref.canonical_value()?,
            ),
            ("destination", destination.canonical_value()?),
            ("requirement", requirement.canonical_value()?),
        ])?)
    }

    /// Projects every blocking-source field.
    pub fn fields(&self) -> Result<BlockingSourceFields> {
        Ok(BlockingSourceFields {
            producer_node_id: node_id_field(self, "producer_node_id")?,
            producer_terminal_transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "producer_terminal_transition_ref",
            )?)?,
            destination: BlockingDestination::from_canonical_value(required_field(
                self,
                "destination",
            )?)?,
            requirement: BlockingProducerRequirement::from_canonical_value(required_field(
                self,
                "requirement",
            )?)?,
        })
    }
}

/// Closed settlement fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementFields {
    /// Callback settlement succeeded.
    Succeeded {
        /// Ordered produced outputs.
        output_bindings: Vec<OutputBinding>,
        /// Ordered facts published atomically with the transition.
        fact_emissions: Vec<FactEmission>,
    },
    /// Callback settlement failed with one typed retained failure.
    Failed {
        /// Operation-selected typed failure.
        typed_failure_ref: ValueRef,
    },
}

impl Settlement {
    /// Constructs a successful settlement with dense, slot-grouped fact emissions.
    pub fn succeeded(
        output_bindings: &[OutputBinding],
        fact_emissions: &[FactEmission],
    ) -> Result<Self> {
        validate_fact_emissions(fact_emissions)?;
        Self::from_canonical_value(tagged_object(
            "succeeded",
            [
                (
                    "output_bindings",
                    cv_array(output_bindings.iter().map(|value| value.canonical_value()))?,
                ),
                (
                    "fact_emissions",
                    cv_array(fact_emissions.iter().map(|value| value.canonical_value()))?,
                ),
            ],
        )?)
    }

    /// Constructs a failed settlement.
    pub fn failed(typed_failure_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "failed",
            [("typed_failure_ref", typed_failure_ref.canonical_value()?)],
        )?)
    }

    /// Projects the closed settlement fields.
    pub fn fields(&self) -> Result<SettlementFields> {
        match super::codec::tag(self)?.as_str() {
            "succeeded" => Ok(SettlementFields::Succeeded {
                output_bindings: array_field(self, "output_bindings")?
                    .into_iter()
                    .map(OutputBinding::from_canonical_value)
                    .collect::<Result<_>>()?,
                fact_emissions: validated_fact_emissions(
                    array_field(self, "fact_emissions")?
                        .into_iter()
                        .map(FactEmission::from_canonical_value)
                        .collect::<Result<_>>()?,
                )?,
            }),
            "failed" => Ok(SettlementFields::Failed {
                typed_failure_ref: ValueRef::from_canonical_value(required_field(
                    self,
                    "typed_failure_ref",
                )?)?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }

    /// Returns the exact fact emissions, or an empty slice-equivalent vector for failure.
    pub fn fact_emissions(&self) -> Result<Vec<FactEmission>> {
        match self.fields()? {
            SettlementFields::Succeeded { fact_emissions, .. } => Ok(fact_emissions),
            SettlementFields::Failed { .. } => Ok(Vec::new()),
        }
    }
}

fn validated_fact_emissions(fact_emissions: Vec<FactEmission>) -> Result<Vec<FactEmission>> {
    validate_fact_emissions(&fact_emissions)?;
    Ok(fact_emissions)
}

fn validate_fact_emissions(fact_emissions: &[FactEmission]) -> Result<()> {
    let mut previous_slot = None;
    for (expected_emission_ordinal, emission) in fact_emissions.iter().enumerate() {
        let expected_emission_ordinal = u32::try_from(expected_emission_ordinal)
            .map_err(|_| super::JournalError::Projection)?;
        let fields = emission.fields()?;
        if fields.emission_ordinal != expected_emission_ordinal
            || previous_slot.is_some_and(|previous| fields.fact_slot_ordinal < previous)
        {
            return Err(super::JournalError::Projection);
        }
        previous_slot = Some(fields.fact_slot_ordinal);
    }
    Ok(())
}

/// Closed transition body fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionBodyFields {
    /// Pure state settlement.
    PureSettled {
        /// Exact frozen input manifest.
        input_manifest_ref: InputManifestRef,
        /// Callback settlement.
        settlement: Settlement,
    },
    /// Read-state settlement consuming one committed observation.
    ReadSettled {
        /// Exact frozen input manifest.
        input_manifest_ref: InputManifestRef,
        /// Authored request authority.
        request_ref: ValueRef,
        /// Consumed observation.
        consumed_observation_ref: ObservationRef,
        /// Callback settlement.
        settlement: Settlement,
    },
    /// Durable effect request committed before external execution.
    EffectRequested {
        /// Exact frozen input manifest.
        input_manifest_ref: InputManifestRef,
        /// Deterministic effect key.
        effect_key: EffectKey,
        /// Exact semantic request authority.
        semantic_request_ref: ValueRef,
        /// Semantic request digest.
        request_digest: RequestDigest,
        /// Immutable executor binding.
        executor_binding_ref: CapabilityBindingRef,
    },
    /// Effect settlement consuming terminal observed evidence.
    EffectSettled {
        /// Exact request transition.
        request_transition_ref: TransitionRef,
        /// Exact request input manifest.
        request_input_manifest_ref: InputManifestRef,
        /// Consumed terminal observation.
        consumed_terminal_observation_ref: ObservationRef,
        /// Callback settlement.
        settlement: Settlement,
    },
    /// Deterministic dependency skip.
    DependencySkipped {
        /// Complete canonically ordered terminal blocking set.
        blocking_sources: Vec<BlockingSource>,
    },
}

impl TransitionBody {
    /// Constructs a pure settlement body.
    pub fn pure_settled(
        input_manifest_ref: &InputManifestRef,
        settlement: &Settlement,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "pure_settled",
            [
                ("input_manifest_ref", input_manifest_ref.canonical_value()?),
                ("settlement", settlement.canonical_value()?),
            ],
        )?)
    }

    /// Constructs a read settlement body.
    pub fn read_settled(
        input_manifest_ref: &InputManifestRef,
        request_ref: &ValueRef,
        consumed_observation_ref: &ObservationRef,
        settlement: &Settlement,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "read_settled",
            [
                ("input_manifest_ref", input_manifest_ref.canonical_value()?),
                ("request_ref", request_ref.canonical_value()?),
                (
                    "consumed_observation_ref",
                    consumed_observation_ref.canonical_value()?,
                ),
                ("settlement", settlement.canonical_value()?),
            ],
        )?)
    }

    /// Constructs a durable effect request body.
    pub fn effect_requested(
        input_manifest_ref: &InputManifestRef,
        effect_key: &EffectKey,
        semantic_request_ref: &ValueRef,
        request_digest: &RequestDigest,
        executor_binding_ref: &CapabilityBindingRef,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "effect_requested",
            [
                ("input_manifest_ref", input_manifest_ref.canonical_value()?),
                ("effect_key", cv_string(effect_key.as_str())),
                (
                    "semantic_request_ref",
                    semantic_request_ref.canonical_value()?,
                ),
                ("request_digest", cv_string(request_digest.as_str())),
                (
                    "executor_binding_ref",
                    executor_binding_ref.canonical_value()?,
                ),
            ],
        )?)
    }

    /// Constructs an effect settlement body.
    pub fn effect_settled(
        request_transition_ref: &TransitionRef,
        request_input_manifest_ref: &InputManifestRef,
        consumed_terminal_observation_ref: &ObservationRef,
        settlement: &Settlement,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "effect_settled",
            [
                (
                    "request_transition_ref",
                    request_transition_ref.canonical_value()?,
                ),
                (
                    "request_input_manifest_ref",
                    request_input_manifest_ref.canonical_value()?,
                ),
                (
                    "consumed_terminal_observation_ref",
                    consumed_terminal_observation_ref.canonical_value()?,
                ),
                ("settlement", settlement.canonical_value()?),
            ],
        )?)
    }

    /// Constructs a deterministic dependency skip body.
    pub fn dependency_skipped(blocking_sources: &[BlockingSource]) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "dependency_skipped",
            [(
                "blocking_sources",
                cv_array(blocking_sources.iter().map(|value| value.canonical_value()))?,
            )],
        )?)
    }

    /// Projects the closed transition body.
    pub fn fields(&self) -> Result<TransitionBodyFields> {
        match super::codec::tag(self)?.as_str() {
            "pure_settled" => Ok(TransitionBodyFields::PureSettled {
                input_manifest_ref: InputManifestRef::from_canonical_value(required_field(
                    self,
                    "input_manifest_ref",
                )?)?,
                settlement: Settlement::from_canonical_value(required_field(self, "settlement")?)?,
            }),
            "read_settled" => Ok(TransitionBodyFields::ReadSettled {
                input_manifest_ref: InputManifestRef::from_canonical_value(required_field(
                    self,
                    "input_manifest_ref",
                )?)?,
                request_ref: ValueRef::from_canonical_value(required_field(self, "request_ref")?)?,
                consumed_observation_ref: ObservationRef::from_canonical_value(required_field(
                    self,
                    "consumed_observation_ref",
                )?)?,
                settlement: Settlement::from_canonical_value(required_field(self, "settlement")?)?,
            }),
            "effect_requested" => Ok(TransitionBodyFields::EffectRequested {
                input_manifest_ref: InputManifestRef::from_canonical_value(required_field(
                    self,
                    "input_manifest_ref",
                )?)?,
                effect_key: effect_key_field(self, "effect_key")?,
                semantic_request_ref: ValueRef::from_canonical_value(required_field(
                    self,
                    "semantic_request_ref",
                )?)?,
                request_digest: RequestDigest::parse(string_field(self, "request_digest")?)?,
                executor_binding_ref: CapabilityBindingRef::from_canonical_value(required_field(
                    self,
                    "executor_binding_ref",
                )?)?,
            }),
            "effect_settled" => Ok(TransitionBodyFields::EffectSettled {
                request_transition_ref: TransitionRef::from_canonical_value(required_field(
                    self,
                    "request_transition_ref",
                )?)?,
                request_input_manifest_ref: InputManifestRef::from_canonical_value(
                    required_field(self, "request_input_manifest_ref")?,
                )?,
                consumed_terminal_observation_ref: ObservationRef::from_canonical_value(
                    required_field(self, "consumed_terminal_observation_ref")?,
                )?,
                settlement: Settlement::from_canonical_value(required_field(self, "settlement")?)?,
            }),
            "dependency_skipped" => Ok(TransitionBodyFields::DependencySkipped {
                blocking_sources: array_field(self, "blocking_sources")?
                    .into_iter()
                    .map(BlockingSource::from_canonical_value)
                    .collect::<Result<_>>()?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }

    /// Returns every fact emission carried by this body's settlement.
    pub fn fact_emissions(&self) -> Result<Vec<FactEmission>> {
        match self.fields()? {
            TransitionBodyFields::PureSettled { settlement, .. }
            | TransitionBodyFields::ReadSettled { settlement, .. }
            | TransitionBodyFields::EffectSettled { settlement, .. } => settlement.fact_emissions(),
            TransitionBodyFields::EffectRequested { .. }
            | TransitionBodyFields::DependencySkipped { .. } => Ok(Vec::new()),
        }
    }

    /// Returns the exact consumed observation, when this is a read or effect settlement.
    pub fn consumed_observation_ref(&self) -> Result<Option<ObservationRef>> {
        match self.fields()? {
            TransitionBodyFields::ReadSettled {
                consumed_observation_ref,
                ..
            } => Ok(Some(consumed_observation_ref)),
            TransitionBodyFields::EffectSettled {
                consumed_terminal_observation_ref,
                ..
            } => Ok(Some(consumed_terminal_observation_ref)),
            TransitionBodyFields::PureSettled { .. }
            | TransitionBodyFields::EffectRequested { .. }
            | TransitionBodyFields::DependencySkipped { .. } => Ok(None),
        }
    }
}

impl BindingDelta {
    /// Constructs and validates a non-empty ordered binding delta.
    pub fn new(entries: &[BindingDeltaEntry]) -> Result<Self> {
        Self::from_canonical_value(cv_array(
            entries.iter().map(|value| value.canonical_value()),
        )?)
    }

    /// Projects every ordered binding-delta entry.
    pub fn entries(&self) -> Result<Vec<BindingDeltaEntry>> {
        let CanonicalValue::Array(values) = self.canonical_value()? else {
            return Err(super::JournalError::Projection);
        };
        values
            .into_iter()
            .map(BindingDeltaEntry::from_canonical_value)
            .collect()
    }
}

/// Closed binding-delta entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingDeltaEntryFields {
    /// Node phase changed.
    NodePhaseChange {
        /// Node occurrence.
        node_id: NodeId,
        /// New node phase.
        phase: NodePhase,
    },
    /// Output binding was inserted.
    OutputBinding(OutputBinding),
    /// Fact binding was inserted.
    FactBinding(FactEmission),
    /// Pending effect was inserted.
    PendingEffectInsert {
        /// Effect identity.
        effect_key: EffectKey,
        /// Immutable request digest.
        request_digest: RequestDigest,
    },
    /// Pending effect was removed.
    PendingEffectRemove {
        /// Effect identity.
        effect_key: EffectKey,
    },
    /// Public output changed.
    PublicOutputChange(Option<ValueRef>),
    /// Run phase changed.
    RunPhaseChange(RunPhase),
}

impl BindingDeltaEntry {
    /// Constructs a node-phase change.
    pub fn node_phase_change(node_id: &NodeId, phase: NodePhase) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "node_phase_change",
            [
                ("node_id", cv_string(node_id.as_str())),
                ("phase", cv_string(phase.as_str())),
            ],
        )?)
    }

    /// Constructs insertion of one output binding.
    pub fn output_binding(binding: &OutputBinding) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "output_binding",
            [("binding", binding.canonical_value()?)],
        )?)
    }

    /// Constructs insertion of one fact binding.
    pub fn fact_binding(emission: &FactEmission) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "fact_binding",
            [("emission", emission.canonical_value()?)],
        )?)
    }

    /// Constructs insertion of one unresolved effect.
    pub fn pending_effect_insert(
        effect_key: &EffectKey,
        request_digest: &RequestDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "pending_effect_insert",
            [
                ("effect_key", cv_string(effect_key.as_str())),
                ("request_digest", cv_string(request_digest.as_str())),
            ],
        )?)
    }

    /// Constructs removal of one resolved effect.
    pub fn pending_effect_remove(effect_key: &EffectKey) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "pending_effect_remove",
            [("effect_key", cv_string(effect_key.as_str()))],
        )?)
    }

    /// Constructs replacement or removal of the certified public output.
    pub fn public_output_change(value_ref: Option<&ValueRef>) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "public_output_change",
            [(
                "value_ref",
                value_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            )],
        )?)
    }

    /// Constructs a run-phase change.
    pub fn run_phase_change(run_phase: RunPhase) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "run_phase_change",
            [("run_phase", cv_string(run_phase.as_str()))],
        )?)
    }

    /// Projects the closed binding-delta entry.
    pub fn fields(&self) -> Result<BindingDeltaEntryFields> {
        match super::codec::tag(self)?.as_str() {
            "node_phase_change" => Ok(BindingDeltaEntryFields::NodePhaseChange {
                node_id: node_id_field(self, "node_id")?,
                phase: NodePhase::parse(&string_field(self, "phase")?)?,
            }),
            "output_binding" => Ok(BindingDeltaEntryFields::OutputBinding(
                OutputBinding::from_canonical_value(required_field(self, "binding")?)?,
            )),
            "fact_binding" => Ok(BindingDeltaEntryFields::FactBinding(
                FactEmission::from_canonical_value(required_field(self, "emission")?)?,
            )),
            "pending_effect_insert" => Ok(BindingDeltaEntryFields::PendingEffectInsert {
                effect_key: effect_key_field(self, "effect_key")?,
                request_digest: RequestDigest::parse(string_field(self, "request_digest")?)?,
            }),
            "pending_effect_remove" => Ok(BindingDeltaEntryFields::PendingEffectRemove {
                effect_key: effect_key_field(self, "effect_key")?,
            }),
            "public_output_change" => Ok(BindingDeltaEntryFields::PublicOutputChange(
                super::codec::nullable_field(self, "value_ref")?
                    .map(ValueRef::from_canonical_value)
                    .transpose()?,
            )),
            "run_phase_change" => Ok(BindingDeltaEntryFields::RunPhaseChange(RunPhase::parse(
                &string_field(self, "run_phase")?,
            )?)),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of a complete state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransitionCommittedFields {
    /// Certified spec hash.
    pub spec_hash: SpecHash,
    /// Certified node occurrence.
    pub node_id: NodeId,
    /// Exact state contract.
    pub state_contract_ref: ContentRef,
    /// Request or settlement logical slot.
    pub slot: TransitionSlot,
    /// Complete before-state proof.
    pub before: TransitionBefore,
    /// Closed transition body.
    pub body: TransitionBody,
    /// Complete after-state proof.
    pub after: TransitionAfter,
}

impl StateTransitionCommitted {
    /// Constructs and validates a complete audited semantic transition.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        spec_hash: &SpecHash,
        node_id: &NodeId,
        state_contract_ref: &ContentRef,
        slot: TransitionSlot,
        before: &TransitionBefore,
        body: &TransitionBody,
        after: &TransitionAfter,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.state-transition-committed.v1".to_owned()),
            ),
            ("spec_hash", cv_string(spec_hash.as_str())),
            ("node_id", cv_string(node_id.as_str())),
            ("state_contract_ref", cv_content_ref(state_contract_ref)?),
            ("slot", cv_string(slot.as_str())),
            ("before", before.canonical_value()?),
            ("body", body.canonical_value()?),
            ("after", after.canonical_value()?),
        ])?)
    }

    /// Projects every transition field.
    pub fn fields(&self) -> Result<StateTransitionCommittedFields> {
        Ok(StateTransitionCommittedFields {
            spec_hash: spec_hash_field(self, "spec_hash")?,
            node_id: node_id_field(self, "node_id")?,
            state_contract_ref: content_ref_field(self, "state_contract_ref")?,
            slot: TransitionSlot::parse(&string_field(self, "slot")?)?,
            before: TransitionBefore::from_canonical_value(required_field(self, "before")?)?,
            body: TransitionBody::from_canonical_value(required_field(self, "body")?)?,
            after: TransitionAfter::from_canonical_value(required_field(self, "after")?)?,
        })
    }

    /// Returns every fact emitted atomically by this transition.
    pub fn fact_emissions(&self) -> Result<Vec<FactEmission>> {
        self.fields()?.body.fact_emissions()
    }

    /// Returns the exact consumed observation, when present.
    pub fn consumed_observation_ref(&self) -> Result<Option<ObservationRef>> {
        self.fields()?.body.consumed_observation_ref()
    }
}
