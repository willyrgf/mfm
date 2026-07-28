use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::{FactProposal, ProposedFactValue};
use mfm_ids::{AppendRequestId, FieldPath, NodeId};
use mfm_journal::v1::{
    BindingDelta, BindingDeltaEntry, FactClaimEnvelope, FactContentIdentityPreimage, FactEmission,
    FactValueComponent, NodePhase, OutputBinding, ProducerBinding, Settlement,
    StateTransitionCommitted, TransitionBody, TransitionSlot, ValueRef,
};
use mfm_spec::v1::{
    CertifiedFactSlot, CertifiedNodeContract, CertifiedStateExecution, RetainedValueContract,
};

use super::objects::{derive_value_ref, PreparedAuthority};
use super::preparation::{PreparedFrameParts, ProducedOutputSlot};
use super::{
    CommitTransition, NodeTerminalOutcome, ObjectGraphProposal, PreparedObjectGraph, Result,
    SettlementMaterial, StoreAuthorityContext, StoreError, TransitionMaterial, VerifiedRunView,
};

struct PreparedSettlement {
    settlement: Settlement,
    output_bindings: Vec<OutputBinding>,
    fact_emissions: Vec<FactEmission>,
    authorities: Vec<PreparedAuthority>,
    outcome: NodeTerminalOutcome,
}

impl VerifiedRunView {
    pub(super) fn prepare_transition_material(
        &self,
        authority: &StoreAuthorityContext,
        append_request_id: AppendRequestId,
        material: TransitionMaterial,
    ) -> Result<CommitTransition> {
        let node_id = match &material {
            TransitionMaterial::PureSettled { prepared_frame, .. }
            | TransitionMaterial::ReadSettled { prepared_frame, .. }
            | TransitionMaterial::EffectRequested { prepared_frame, .. } => {
                prepared_frame.node_id().clone()
            }
            TransitionMaterial::EffectSettled {
                request_transition_ref,
                ..
            } => self
                .pending_effects()?
                .into_iter()
                .find(|pending| pending.request_transition_ref() == request_transition_ref)
                .map(|pending| pending.node_id().clone())
                .ok_or(StoreError::TransitionFoldMismatch {
                    field: "effect_request_transition_ref",
                })?,
            TransitionMaterial::DependencySkipped { node_id } => node_id.clone(),
        };
        let node = self
            .certified_spec()
            .nodes()
            .iter()
            .find(|candidate| candidate.node_id() == &node_id)
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        let before = self.transition_before(&node_id)?;
        let mut delta_entries = Vec::new();
        let mut authorities = Vec::new();
        let (slot, body, outcome, object_graph) = match material {
            TransitionMaterial::PureSettled {
                prepared_frame,
                settlement,
                object_graph,
            } => {
                require_execution(node, "pure")?;
                let frame =
                    exact_frame(&node_id, (*prepared_frame).into_parts_for(authority, self)?)?;
                authorities.extend(frame.authorities);
                let prepared = prepare_settlement(self, node, settlement)?;
                append_settlement_delta(&node_id, &mut delta_entries, &prepared)?;
                authorities.extend(prepared.authorities);
                (
                    TransitionSlot::Settlement,
                    TransitionBody::pure_settled(&frame.input_manifest_ref, &prepared.settlement)?,
                    Some(prepared.outcome),
                    object_graph,
                )
            }
            TransitionMaterial::ReadSettled {
                prepared_frame,
                immutable_request_ref,
                consumed_observation_ref,
                settlement,
                object_graph,
            } => {
                require_execution(node, "read")?;
                let frame =
                    exact_frame(&node_id, (*prepared_frame).into_parts_for(authority, self)?)?;
                authorities.extend(frame.authorities);
                let request = self.retained_value(&immutable_request_ref)?;
                authorities.push(PreparedAuthority::preexisting(
                    immutable_request_ref.clone(),
                    request.bytes().to_vec(),
                )?);
                let prepared = prepare_settlement(self, node, settlement)?;
                append_settlement_delta(&node_id, &mut delta_entries, &prepared)?;
                authorities.extend(prepared.authorities);
                (
                    TransitionSlot::Settlement,
                    TransitionBody::read_settled(
                        &frame.input_manifest_ref,
                        &immutable_request_ref,
                        &consumed_observation_ref,
                        &prepared.settlement,
                    )?,
                    Some(prepared.outcome),
                    object_graph,
                )
            }
            TransitionMaterial::EffectRequested {
                prepared_frame,
                semantic_request_root,
                effect_key,
                request_digest,
                executor_binding_ref,
                object_graph,
            } => {
                let CertifiedStateExecution::Effect {
                    request_contract,
                    executor_binding_ref: certified_binding,
                    ..
                } = node.execution()
                else {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "certified_execution_kind",
                    });
                };
                if semantic_request_root.value_contract() != request_contract
                    || executor_binding_ref.fields()? != *certified_binding
                {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "effect_request_contract",
                    });
                }
                let frame =
                    exact_frame(&node_id, (*prepared_frame).into_parts_for(authority, self)?)?;
                authorities.extend(frame.authorities);
                let request_path = FieldPath::new("body.semantic_request_ref")
                    .map_err(|_| StoreError::JournalContract)?;
                let request_ref = derive_value_ref(
                    request_contract,
                    &ProducerBinding::this_record(&request_path)?,
                    semantic_request_root.canonical().as_bytes(),
                )?;
                authorities.push(PreparedAuthority::produced(
                    request_ref.clone(),
                    semantic_request_root.canonical().to_vec(),
                )?);
                delta_entries.push(BindingDeltaEntry::node_phase_change(
                    &node_id,
                    NodePhase::AwaitingEffect,
                )?);
                delta_entries.push(BindingDeltaEntry::pending_effect_insert(
                    &effect_key,
                    &request_digest,
                )?);
                (
                    TransitionSlot::Request,
                    TransitionBody::effect_requested(
                        &frame.input_manifest_ref,
                        &effect_key,
                        &request_ref,
                        &request_digest,
                        &executor_binding_ref,
                    )?,
                    None,
                    object_graph,
                )
            }
            TransitionMaterial::EffectSettled {
                request_transition_ref,
                consumed_terminal_observation_ref,
                settlement,
                object_graph,
            } => {
                require_execution(node, "effect")?;
                let pending = self
                    .pending_effects()?
                    .into_iter()
                    .find(|pending| {
                        pending.node_id() == &node_id
                            && pending.request_transition_ref() == &request_transition_ref
                    })
                    .ok_or(StoreError::TransitionFoldMismatch {
                        field: "effect_request_transition_ref",
                    })?;
                let manifest_ref = pending.input_manifest_ref().clone();
                let manifest_value_ref = manifest_ref.value_ref()?;
                let manifest = self.retained_value(&manifest_value_ref)?;
                authorities.push(PreparedAuthority::preexisting(
                    manifest_value_ref,
                    manifest.bytes().to_vec(),
                )?);
                let prepared = prepare_settlement(self, node, settlement)?;
                append_settlement_delta(&node_id, &mut delta_entries, &prepared)?;
                delta_entries.push(BindingDeltaEntry::pending_effect_remove(
                    pending.effect_key(),
                )?);
                authorities.extend(prepared.authorities);
                (
                    TransitionSlot::Settlement,
                    TransitionBody::effect_settled(
                        &request_transition_ref,
                        &manifest_ref,
                        &consumed_terminal_observation_ref,
                        &prepared.settlement,
                    )?,
                    Some(prepared.outcome),
                    object_graph,
                )
            }
            TransitionMaterial::DependencySkipped {
                node_id: skipped_node_id,
            } => {
                if skipped_node_id != node_id {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "dependency_skip_node_id",
                    });
                }
                let blocking_sources = self.dependency_blocking_sources(&node_id)?;
                if blocking_sources.is_empty() {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "dependency_blocking_sources",
                    });
                }
                delta_entries.push(BindingDeltaEntry::node_phase_change(
                    &node_id,
                    NodePhase::Terminal,
                )?);
                (
                    TransitionSlot::Settlement,
                    TransitionBody::dependency_skipped(&blocking_sources)?,
                    Some(NodeTerminalOutcome::Skipped),
                    ObjectGraphProposal::empty(),
                )
            }
        };

        add_proposed_graph_authorities(&mut authorities, object_graph)?;
        if let Some(outcome) = outcome {
            prepare_terminal_delta(
                self,
                &node_id,
                outcome,
                &mut authorities,
                &mut delta_entries,
            )?;
        }
        let binding_delta = BindingDelta::new(&delta_entries)?;
        let resolver_graph = PreparedObjectGraph::prepare(Vec::new(), authorities.clone(), 1)?;
        let after = self.derive_transition_after_with_objects(
            &node_id,
            &body,
            &binding_delta,
            &resolver_graph,
        )?;
        let transition = StateTransitionCommitted::new(
            &self.certified_spec().spec_hash()?,
            &node_id,
            node.state_contract_ref(),
            slot,
            &before,
            &body,
            &after,
        )?;
        let objects = PreparedObjectGraph::prepare_for_record_values(
            &[transition.canonical_value()?],
            authorities,
        )?;
        CommitTransition::new(self, append_request_id, transition, objects)
    }
}

fn exact_frame(node_id: &NodeId, frame: PreparedFrameParts) -> Result<PreparedFrameParts> {
    if &frame.node_id != node_id {
        return Err(StoreError::TransitionFoldMismatch {
            field: "prepared_frame_node_id",
        });
    }
    Ok(frame)
}

fn require_execution(node: &CertifiedNodeContract, expected: &'static str) -> Result<()> {
    let matches = matches!(
        (expected, node.execution()),
        ("pure", CertifiedStateExecution::Pure)
            | ("read", CertifiedStateExecution::Read { .. })
            | ("effect", CertifiedStateExecution::Effect { .. })
    );
    if matches {
        Ok(())
    } else {
        Err(StoreError::TransitionFoldMismatch {
            field: "certified_execution_kind",
        })
    }
}

fn prepare_settlement(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    material: SettlementMaterial,
) -> Result<PreparedSettlement> {
    match material {
        SettlementMaterial::Succeeded {
            output_roots,
            fact_roots,
        } => {
            if output_roots.len() != node.settlement_contract().output_slots().len()
                || fact_roots.len() > 4096
            {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "settlement_slot_count",
                });
            }
            let mut output_bindings = Vec::with_capacity(output_roots.len());
            let mut authorities = Vec::with_capacity(output_roots.len());
            for (proposed, certified) in output_roots
                .into_iter()
                .zip(node.settlement_contract().output_slots())
            {
                validate_output_slot(&proposed, certified)?;
                let value_ref = derive_value_ref(
                    certified.value_contract(),
                    &ProducerBinding::transition_output(
                        view.run_id(),
                        node.node_id(),
                        certified.output_ordinal(),
                    )?,
                    proposed.root().canonical().as_bytes(),
                )?;
                authorities.push(PreparedAuthority::produced(
                    value_ref.clone(),
                    proposed.root().canonical().to_vec(),
                )?);
                output_bindings.push(OutputBinding::new(
                    certified.output_ordinal(),
                    certified.field_path(),
                    &value_ref,
                )?);
            }
            let (fact_emissions, fact_authorities) =
                prepare_fact_emissions(view, node, fact_roots)?;
            authorities.extend(fact_authorities);
            Ok(PreparedSettlement {
                settlement: Settlement::succeeded(&output_bindings, &fact_emissions)?,
                output_bindings,
                fact_emissions,
                authorities,
                outcome: NodeTerminalOutcome::Succeeded,
            })
        }
        SettlementMaterial::Failed { typed_failure_root } => {
            let contract = node.settlement_contract().typed_failure_contract().ok_or(
                StoreError::TransitionFoldMismatch {
                    field: "typed_failure_contract",
                },
            )?;
            if typed_failure_root.value_contract() != contract {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "typed_failure_contract",
                });
            }
            let path = FieldPath::new("body.settlement.typed_failure_ref")
                .map_err(|_| StoreError::JournalContract)?;
            let value_ref = derive_value_ref(
                contract,
                &ProducerBinding::this_record(&path)?,
                typed_failure_root.canonical().as_bytes(),
            )?;
            Ok(PreparedSettlement {
                settlement: Settlement::failed(&value_ref)?,
                output_bindings: Vec::new(),
                fact_emissions: Vec::new(),
                authorities: vec![PreparedAuthority::produced(
                    value_ref,
                    typed_failure_root.canonical().to_vec(),
                )?],
                outcome: NodeTerminalOutcome::Failed,
            })
        }
    }
}

struct PreparedFactEmission {
    emission: FactEmission,
    authorities: Vec<PreparedAuthority>,
}

fn prepare_fact_emissions(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    proposals: Vec<FactProposal>,
) -> Result<(Vec<FactEmission>, Vec<PreparedAuthority>)> {
    let mut proposals = proposals.into_iter().peekable();
    let mut emissions = Vec::new();
    let mut authorities = Vec::new();
    for certified in node.settlement_contract().fact_slots() {
        let mut count = 0_u32;
        while proposals
            .peek()
            .is_some_and(|proposal| proposal.fact_slot_ordinal() == certified.fact_slot_ordinal())
        {
            if count == certified.maximum_emissions() {
                return Err(StoreError::TransitionFoldMismatch {
                    field: "fact_slot_emission_bounds",
                });
            }
            let proposal = proposals.next().ok_or(StoreError::TransitionFoldMismatch {
                field: "fact_slot_group",
            })?;
            let actual_emission_ordinal =
                u32::try_from(emissions.len()).map_err(|_| StoreError::SequenceOverflow)?;
            let prepared = prepare_fact_emission(
                view,
                node.node_id(),
                actual_emission_ordinal,
                proposal,
                certified,
            )?;
            authorities.extend(prepared.authorities);
            emissions.push(prepared.emission);
            count = count.checked_add(1).ok_or(StoreError::SequenceOverflow)?;
        }
        if count < certified.minimum_emissions() {
            return Err(StoreError::TransitionFoldMismatch {
                field: "fact_slot_emission_bounds",
            });
        }
    }
    if proposals.next().is_some() {
        return Err(StoreError::TransitionFoldMismatch {
            field: "fact_slot_group",
        });
    }
    Ok((emissions, authorities))
}

fn prepare_fact_emission(
    view: &VerifiedRunView,
    node_id: &NodeId,
    actual_emission_ordinal: u32,
    proposed: FactProposal,
    certified: &CertifiedFactSlot,
) -> Result<PreparedFactEmission> {
    if proposed.fact_slot_ordinal() != certified.fact_slot_ordinal()
        || proposed.descriptor_ref() != certified.fact_descriptor_ref()
    {
        return Err(StoreError::TransitionFoldMismatch {
            field: "certified_fact_descriptor",
        });
    }
    validate_fact_value_contract(proposed.subject(), certified.subject_contract())?;
    validate_fact_value_contract(proposed.response(), certified.response_contract())?;

    let subject_ref = derive_value_ref(
        certified.subject_contract(),
        &ProducerBinding::transition_fact(
            view.run_id(),
            node_id,
            actual_emission_ordinal,
            FactValueComponent::Subject,
        )?,
        proposed.subject().canonical().as_bytes(),
    )?;
    let response_ref = derive_value_ref(
        certified.response_contract(),
        &ProducerBinding::transition_fact(
            view.run_id(),
            node_id,
            actual_emission_ordinal,
            FactValueComponent::Response,
        )?,
        proposed.response().canonical().as_bytes(),
    )?;
    let claim = FactClaimEnvelope::new(proposed.descriptor_ref(), &subject_ref, &response_ref)?;
    let claim_ref = derive_value_ref(
        view.certified_spec()
            .journal_protocol_contracts()
            .fact_claim_envelope_contract(),
        &ProducerBinding::transition_fact(
            view.run_id(),
            node_id,
            actual_emission_ordinal,
            FactValueComponent::Claim,
        )?,
        claim.as_bytes(),
    )?;
    let content_identity =
        FactContentIdentityPreimage::new(proposed.descriptor_ref(), &subject_ref, &response_ref)?
            .fact_content_identity()?;
    let emission = FactEmission::new(
        actual_emission_ordinal,
        certified.fact_slot_ordinal(),
        proposed.descriptor_ref(),
        &claim_ref,
        &content_identity,
    )?;
    Ok(PreparedFactEmission {
        emission,
        authorities: vec![
            PreparedAuthority::produced(subject_ref, proposed.subject().canonical().to_vec())?,
            PreparedAuthority::produced(response_ref, proposed.response().canonical().to_vec())?,
            PreparedAuthority::produced(claim_ref, claim.as_bytes().to_vec())?,
        ],
    })
}

fn validate_fact_value_contract(
    proposed: &ProposedFactValue,
    certified: &RetainedValueContract,
) -> Result<()> {
    if proposed.schema_id() == certified.schema_id()
        && proposed.semantic_type_id() == certified.semantic_type_id()
        && proposed.role() == certified.role()
        && proposed.media_type() == certified.media_type()
        && proposed.evidence_contract_ref() == certified.evidence_contract_ref()
    {
        Ok(())
    } else {
        Err(StoreError::TransitionFoldMismatch {
            field: "certified_fact_value_contract",
        })
    }
}

fn validate_output_slot(
    proposed: &ProducedOutputSlot,
    certified: &mfm_spec::v1::CertifiedOutputSlot,
) -> Result<()> {
    if proposed.output_ordinal() != certified.output_ordinal()
        || proposed.field_path() != certified.field_path()
        || proposed.root().value_contract() != certified.value_contract()
    {
        return Err(StoreError::TransitionFoldMismatch {
            field: "certified_output_slot",
        });
    }
    Ok(())
}

fn append_settlement_delta(
    node_id: &NodeId,
    entries: &mut Vec<BindingDeltaEntry>,
    settlement: &PreparedSettlement,
) -> Result<()> {
    entries.push(BindingDeltaEntry::node_phase_change(
        node_id,
        NodePhase::Terminal,
    )?);
    for binding in &settlement.output_bindings {
        entries.push(BindingDeltaEntry::output_binding(binding)?);
    }
    for emission in &settlement.fact_emissions {
        entries.push(BindingDeltaEntry::fact_binding(emission)?);
    }
    Ok(())
}

fn add_proposed_graph_authorities(
    authorities: &mut Vec<PreparedAuthority>,
    graph: ObjectGraphProposal,
) -> Result<()> {
    for member in graph.into_members() {
        let producer = ProducerBinding::this_record(member.field_path())?;
        let value_ref = derive_value_ref(
            member.root().value_contract(),
            &producer,
            member.root().canonical().as_bytes(),
        )?;
        authorities.push(PreparedAuthority::produced(
            value_ref,
            member.root().canonical().to_vec(),
        )?);
    }
    Ok(())
}

fn prepare_terminal_delta(
    view: &VerifiedRunView,
    node_id: &NodeId,
    candidate_outcome: NodeTerminalOutcome,
    authorities: &mut Vec<PreparedAuthority>,
    entries: &mut Vec<BindingDeltaEntry>,
) -> Result<()> {
    let all_terminal = view.certified_spec().nodes().iter().all(|node| {
        node.node_id() == node_id || view.node_terminal_outcome(node.node_id()).is_some()
    });
    if !all_terminal {
        return Ok(());
    }
    let required_success_met = view
        .certified_spec()
        .run_terminal_contract()
        .required_success_nodes()
        .iter()
        .all(|required| {
            if required == node_id {
                candidate_outcome == NodeTerminalOutcome::Succeeded
            } else {
                view.node_terminal_outcome(required) == Some(NodeTerminalOutcome::Succeeded)
            }
        });
    if required_success_met
        && view
            .certified_spec()
            .run_terminal_contract()
            .requires_public_output()
    {
        let (value_ref, bytes) = assemble_public_output(view, node_id, authorities)?;
        authorities.push(PreparedAuthority::produced(value_ref.clone(), bytes)?);
        entries.push(BindingDeltaEntry::public_output_change(Some(&value_ref))?);
    }
    entries.push(BindingDeltaEntry::run_phase_change(
        mfm_journal::v1::RunPhase::Closed,
    )?);
    Ok(())
}

fn assemble_public_output(
    view: &VerifiedRunView,
    candidate_node_id: &NodeId,
    authorities: &[PreparedAuthority],
) -> Result<(ValueRef, Vec<u8>)> {
    let mut root = serde_json::Value::Object(serde_json::Map::new());
    for binding in view.certified_spec().public_output_contract().bindings() {
        let bytes = if binding.source_node_id() == candidate_node_id {
            let authority = authorities
                .iter()
                .find(|authority| {
                    authority.value_ref().fields().is_ok_and(|fields| {
                        matches!(
                            fields.producer_binding.fields(),
                            Ok(
                            mfm_journal::v1::ProducerBindingFields::TransitionOutput {
                                ref run_id,
                                ref node_id,
                                output_ordinal,
                            }) if run_id == view.run_id()
                                && node_id == candidate_node_id
                                && output_ordinal == binding.output_ordinal()
                        )
                    })
                })
                .ok_or(StoreError::TransitionFoldMismatch {
                    field: "public_output_source",
                })?;
            authority.bytes().to_vec()
        } else {
            let value_ref = view
                .node_outputs(binding.source_node_id())
                .iter()
                .find(|output| {
                    output
                        .fields()
                        .is_ok_and(|fields| fields.output_ordinal == binding.output_ordinal())
                })
                .ok_or(StoreError::TransitionFoldMismatch {
                    field: "public_output_source",
                })?
                .fields()?
                .value_ref;
            view.retained_value(&value_ref)?.bytes().to_vec()
        };
        let selected =
            super::frame_preparation::select_canonical(&bytes, binding.source_field_path())?;
        let value: serde_json::Value =
            serde_json::from_slice(selected.as_bytes()).map_err(|_| StoreError::JournalContract)?;
        insert_json_path(&mut root, binding.destination_field_path(), value)?;
    }
    let encoded = serde_json::to_string(&root).map_err(|_| StoreError::JournalContract)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&encoded)
        .map_err(|_| StoreError::JournalContract)?;
    let value_ref = derive_value_ref(
        view.certified_spec()
            .public_output_contract()
            .value_contract(),
        &ProducerBinding::public_output_assembly(view.run_id())?,
        canonical.as_bytes(),
    )?;
    Ok((value_ref, canonical.to_vec()))
}

fn insert_json_path(
    root: &mut serde_json::Value,
    field_path: &FieldPath,
    value: serde_json::Value,
) -> Result<()> {
    let segments = field_path.as_str().split('.').collect::<Vec<_>>();
    insert_json_segments(root, &segments, value)
}

fn insert_json_segments(
    current: &mut serde_json::Value,
    segments: &[&str],
    value: serde_json::Value,
) -> Result<()> {
    let (segment, remaining) =
        segments
            .split_first()
            .ok_or(StoreError::TransitionFoldMismatch {
                field: "public_output_destination",
            })?;
    let object = current
        .as_object_mut()
        .ok_or(StoreError::TransitionFoldMismatch {
            field: "public_output_destination",
        })?;
    if remaining.is_empty() {
        if object.insert((*segment).to_owned(), value).is_some() {
            return Err(StoreError::TransitionFoldMismatch {
                field: "public_output_destination",
            });
        }
        return Ok(());
    }
    let nested = object
        .entry((*segment).to_owned())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    insert_json_segments(nested, remaining, value)
}
