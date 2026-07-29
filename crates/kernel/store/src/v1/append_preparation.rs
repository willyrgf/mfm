use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mfm_canonical::{PlainCanonicalJsonBytes, RecoverabilityContractV2};
use mfm_capabilities::SafeFailureOutcome;
use mfm_executor::{
    EffectExecutorOutcome, Ensure, ExecutorRetainedValueRelation, ProofBasis, SafeFailureCode,
    VerifiedEnsureResult,
};
use mfm_ids::{AppendRequestId, ContentRef, FieldPath, RequestDigest, StableId};
use mfm_journal::v1::{
    AuthorizationRef, AuthorizationScope, AuthorizationScopeFields, CapabilityBindingRef,
    ExecutorEnsureResult, ExecutorProofBasis, ExternalAccessAuthorized, ExternalAccessObserved,
    FrozenReadIntent, NodePhase, ObservationOutcome, ProducerBinding, ReadCapabilityBinding,
    SafeFailure, SemanticAnchor, TerminalEffectEvidence, TransitionRef, ValueRef,
};
use mfm_spec::v1::CertifiedStateExecution;

use super::fact_scan::prepare_fact_selection_observation;
use super::objects::{derive_value_ref, PreparedAuthority};
use super::preparation::{
    AuthorizationMaterial, ExistingRunAppendMaterial, ObservationMaterial, PreparedFrameParts,
    ProducedObjectRoot, ReadObservationMaterial, SafeFailureMetadata,
};
use super::{
    AuthorizeExternalAccess, ObserveExternalAccess, PreparedJournalAppend, PreparedObjectGraph,
    Result, StoreAuthorityContext, StoreError, VerifiedRunView,
};

const REQUEST_PREIMAGE_SCHEMA: &str = "mfm.request-digest-preimage.v1";
const READ_REQUEST_PATH: &str = "request_ref";
const FROZEN_READ_INTENT_PATH: &str = "frozen_read_intent_ref";
const OBSERVATION_RESULT_PATH: &str = "outcome.result_ref";
const OBSERVATION_FAILURE_PATH: &str = "outcome.safe_failure.diagnostic_ref";
const ENSURE_RESULT_PATH: &str = "executor.ensure_result";
const TERMINAL_EVIDENCE_PATH: &str = "executor.terminal_evidence";

impl VerifiedRunView {
    pub(super) fn prepare_existing_append(
        &self,
        authority: &StoreAuthorityContext,
        append_request_id: AppendRequestId,
        material: ExistingRunAppendMaterial,
    ) -> Result<PreparedJournalAppend> {
        match material {
            ExistingRunAppendMaterial::Transition(material) => self
                .prepare_transition_material(authority, append_request_id, *material)
                .map(PreparedJournalAppend::CommitTransition),
            ExistingRunAppendMaterial::Authorization(material) => {
                prepare_authorization(self, authority, append_request_id, *material)
                    .map(PreparedJournalAppend::AuthorizeExternalAccess)
            }
            ExistingRunAppendMaterial::Observation(material) => {
                prepare_observation(self, append_request_id, *material)
                    .map(PreparedJournalAppend::from)
            }
        }
    }
}

fn prepare_authorization(
    view: &VerifiedRunView,
    authority: &StoreAuthorityContext,
    append_request_id: AppendRequestId,
    material: AuthorizationMaterial,
) -> Result<AuthorizeExternalAccess> {
    match material {
        AuthorizationMaterial::Read {
            prepared_frame,
            immutable_request_root,
            routing_generation_ref,
        } => prepare_read_authorization(
            view,
            append_request_id,
            (*prepared_frame).into_parts_for(authority, view)?,
            *immutable_request_root,
            routing_generation_ref,
        ),
        AuthorizationMaterial::EnsureEffect {
            request_transition_ref,
        } => prepare_ensure_authorization(view, append_request_id, &request_transition_ref),
    }
}

fn prepare_read_authorization(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    frame: PreparedFrameParts,
    immutable_request_root: ProducedObjectRoot,
    routing_generation_ref: ContentRef,
) -> Result<AuthorizeExternalAccess> {
    let node = view
        .certified_spec()
        .nodes()
        .iter()
        .find(|node| node.node_id() == &frame.node_id)
        .ok_or(StoreError::AuthorizationNotEligible)?;
    if view.node_phase(node.node_id()) != Some(NodePhase::Unstarted) {
        return Err(StoreError::AuthorizationNotEligible);
    }
    let CertifiedStateExecution::Read {
        capability_operation_id,
        capability_binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
    } = node.execution()
    else {
        return Err(StoreError::AuthorizationNotEligible);
    };
    if immutable_request_root.value_contract() != request_contract {
        return Err(StoreError::AuthorizationNotEligible);
    }
    validate_read_binding(
        view,
        capability_operation_id,
        capability_binding_ref,
        &routing_generation_ref,
    )?;

    let capability_binding_ref = CapabilityBindingRef::new(capability_binding_ref)?;
    let request_ref = derive_value_ref(
        request_contract,
        &ProducerBinding::this_record(&field_path(READ_REQUEST_PATH)?)?,
        immutable_request_root.canonical().as_bytes(),
    )?;
    let request_digest = derive_request_digest(
        request_contract.schema_id().as_str(),
        immutable_request_root.canonical(),
    )?;
    let frozen = FrozenReadIntent::new(
        node.node_id(),
        &frame.input_manifest_ref,
        node.state_contract_ref(),
        &capability_binding_ref,
        &routing_generation_ref,
        capability_operation_id,
        &request_ref,
        &request_digest,
        request_contract,
        returned_contract,
        safe_failure_contract,
    )?;
    validate_frozen_read_retry(view, node.node_id(), &request_ref, &frozen)?;
    let frozen_contract = view
        .certified_spec()
        .journal_protocol_contracts()
        .frozen_read_intent_contract();
    let frozen_ref = derive_value_ref(
        frozen_contract,
        &ProducerBinding::this_record(&field_path(FROZEN_READ_INTENT_PATH)?)?,
        frozen.as_bytes(),
    )?;
    let anchor = semantic_anchor(view, node.node_id(), NodePhase::Unstarted)?;
    let authorization = ExternalAccessAuthorized::new(
        &anchor,
        &AuthorizationScope::read(&frame.input_manifest_ref)?,
        &capability_binding_ref,
        capability_operation_id,
        &request_ref,
        Some(&frozen_ref),
    )?;
    let mut authorities = frame.authorities;
    authorities.push(PreparedAuthority::produced(
        request_ref,
        immutable_request_root.canonical().to_vec(),
    )?);
    authorities.push(PreparedAuthority::produced(
        frozen_ref,
        frozen.as_bytes().to_vec(),
    )?);
    let objects = PreparedObjectGraph::prepare_for_record_values(
        &[authorization.canonical_value()?],
        authorities,
    )?;
    AuthorizeExternalAccess::new(view, append_request_id, authorization, objects)
}

fn validate_frozen_read_retry(
    view: &VerifiedRunView,
    node_id: &mfm_ids::NodeId,
    request_ref: &ValueRef,
    candidate: &FrozenReadIntent,
) -> Result<()> {
    let history = view.access_history(node_id)?;
    let Some(first) = history.entries().next() else {
        return Ok(());
    };
    let authorization = first.authorization().fields()?;
    let frozen_ref = authorization
        .frozen_read_intent_ref
        .as_ref()
        .ok_or(StoreError::FrozenReadIntentConflict)?;
    let frozen = FrozenReadIntent::strict_decode(view.retained_value(frozen_ref)?.bytes())?;
    if &authorization.request_ref != request_ref || frozen.as_bytes() != candidate.as_bytes() {
        return Err(StoreError::FrozenReadIntentConflict);
    }
    Ok(())
}

fn prepare_ensure_authorization(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    request_transition_ref: &TransitionRef,
) -> Result<AuthorizeExternalAccess> {
    let pending = view
        .pending_effects()?
        .into_iter()
        .find(|pending| pending.request_transition_ref() == request_transition_ref)
        .ok_or(StoreError::AuthorizationNotEligible)?;
    let node = view
        .certified_spec()
        .nodes()
        .iter()
        .find(|node| node.node_id() == pending.node_id())
        .ok_or(StoreError::AuthorizationNotEligible)?;
    let CertifiedStateExecution::Effect {
        executor_operation_id,
        executor_binding_ref,
        ..
    } = node.execution()
    else {
        return Err(StoreError::AuthorizationNotEligible);
    };
    if view.node_phase(node.node_id()) != Some(NodePhase::AwaitingEffect)
        || pending.executor_binding_ref().fields()? != *executor_binding_ref
    {
        return Err(StoreError::AuthorizationNotEligible);
    }
    let admitted = view
        .capability_binding_manifest()
        .entries()
        .iter()
        .find(|entry| entry.operation_id == *executor_operation_id)
        .ok_or(StoreError::AuthorizationNotEligible)?;
    if admitted.binding_ref != *executor_binding_ref {
        return Err(StoreError::AuthorizationNotEligible);
    }
    let anchor = semantic_anchor(view, node.node_id(), NodePhase::AwaitingEffect)?;
    let authorization = ExternalAccessAuthorized::new(
        &anchor,
        &AuthorizationScope::ensure_effect(request_transition_ref)?,
        pending.executor_binding_ref(),
        executor_operation_id,
        pending.semantic_request_ref(),
        None,
    )?;
    let request = view.retained_value(pending.semantic_request_ref())?;
    let objects = PreparedObjectGraph::prepare_for_record_values(
        &[authorization.canonical_value()?],
        vec![PreparedAuthority::preexisting(
            pending.semantic_request_ref().clone(),
            request.bytes().to_vec(),
        )?],
    )?;
    AuthorizeExternalAccess::new(view, append_request_id, authorization, objects)
}

fn prepare_observation(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    material: ObservationMaterial,
) -> Result<ObserveExternalAccess> {
    match material {
        ObservationMaterial::Read {
            authorization_ref,
            outcome,
        } => prepare_read_observation(view, append_request_id, &authorization_ref, *outcome),
        ObservationMaterial::EnsureEffect {
            authorization_ref,
            outcome,
        } => prepare_effect_observation(view, append_request_id, &authorization_ref, *outcome),
        ObservationMaterial::FactSelection { completed_scan } => {
            let (observation, objects, pending) =
                prepare_fact_selection_observation(view, *completed_scan)?;
            ObserveExternalAccess::new(view, append_request_id, observation, objects, Some(pending))
        }
    }
}

fn prepare_read_observation(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    authorization_ref: &AuthorizationRef,
    material: ReadObservationMaterial,
) -> Result<ObserveExternalAccess> {
    let entry = unobserved_entry(view, authorization_ref)?;
    let authorization = entry.authorization().fields()?;
    let AuthorizationScopeFields::Read { .. } = authorization.scope.fields()? else {
        return Err(StoreError::UnknownAuthorization);
    };
    let frozen_ref = authorization
        .frozen_read_intent_ref
        .ok_or(StoreError::FrozenReadIntentConflict)?;
    let frozen =
        FrozenReadIntent::strict_decode(view.retained_value(&frozen_ref)?.bytes())?.fields()?;
    if frozen.capability_binding_ref != authorization.capability_binding_ref
        || frozen.capability_operation_id != authorization.capability_operation_id
        || frozen.request_ref != authorization.request_ref
    {
        return Err(StoreError::FrozenReadIntentConflict);
    }

    let (outcome, authorities) = match material {
        ReadObservationMaterial::Returned { returned_root } => {
            if returned_root.value_contract() != &frozen.returned_contract {
                return Err(StoreError::InvalidPreparedAppend {
                    purpose: "read_observation",
                    message: "returned value contract does not match the frozen read intent",
                });
            }
            let result_ref =
                observation_value_ref(authorization_ref, OBSERVATION_RESULT_PATH, &returned_root)?;
            (
                ObservationOutcome::returned(&result_ref)?,
                vec![PreparedAuthority::produced(
                    result_ref,
                    returned_root.canonical().to_vec(),
                )?],
            )
        }
        ReadObservationMaterial::DidNotEnter {
            diagnostic_root,
            metadata,
        } => prepare_safe_failure(
            authorization_ref,
            diagnostic_root,
            metadata,
            &frozen.safe_failure_contract,
            false,
        )?,
        ReadObservationMaterial::Indeterminate {
            diagnostic_root,
            metadata,
        } => prepare_safe_failure(
            authorization_ref,
            diagnostic_root,
            metadata,
            &frozen.safe_failure_contract,
            true,
        )?,
    };
    let observation = ExternalAccessObserved::new(authorization_ref, &outcome, None)?;
    let objects = PreparedObjectGraph::prepare_for_record_values(
        &[observation.canonical_value()?],
        authorities,
    )?;
    ObserveExternalAccess::new(view, append_request_id, observation, objects, None)
}

fn prepare_safe_failure(
    authorization_ref: &AuthorizationRef,
    diagnostic_root: Option<ProducedObjectRoot>,
    metadata: SafeFailureMetadata,
    frozen_contract: &mfm_spec::v1::RetainedValueContract,
    indeterminate: bool,
) -> Result<(ObservationOutcome, Vec<PreparedAuthority>)> {
    if diagnostic_root
        .as_ref()
        .is_some_and(|root| root.value_contract() != frozen_contract)
    {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "read_observation",
            message: "safe failure does not match the frozen read intent",
        });
    }
    let diagnostic_ref = diagnostic_root
        .as_ref()
        .map(|root| observation_value_ref(authorization_ref, OBSERVATION_FAILURE_PATH, root))
        .transpose()?;
    let failure = SafeFailure::new(
        metadata.safe_failure_contract_ref(),
        metadata.stable_code(),
        metadata.failure_class(),
        metadata.boundary_stage(),
        metadata.coarse_size_class(),
        diagnostic_ref.as_ref(),
    )?;
    let outcome = if indeterminate {
        ObservationOutcome::indeterminate(&failure)?
    } else {
        ObservationOutcome::did_not_enter(&failure)?
    };
    Ok((
        outcome,
        diagnostic_ref
            .zip(diagnostic_root)
            .map(|(reference, root)| {
                PreparedAuthority::produced(reference, root.canonical().to_vec())
            })
            .transpose()?
            .into_iter()
            .collect(),
    ))
}

fn prepare_effect_observation(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    authorization_ref: &AuthorizationRef,
    outcome: EffectExecutorOutcome,
) -> Result<ObserveExternalAccess> {
    unobserved_entry(view, authorization_ref)?;
    match outcome.into_parts() {
        Ok(result) => {
            prepare_returned_effect_observation(view, append_request_id, authorization_ref, result)
        }
        Err((outcome, failure)) => prepare_effect_safe_failure_observation(
            view,
            append_request_id,
            authorization_ref,
            failure,
            outcome,
        ),
    }
}

fn prepare_effect_safe_failure_observation(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    authorization_ref: &AuthorizationRef,
    failure: mfm_executor::ReferenceSafeFailure,
    outcome: SafeFailureOutcome,
) -> Result<ObserveExternalAccess> {
    let stable_code = StableId::new(failure.stable_code().as_str())?;
    mfm_executor::verify_reference_safe_failure_tuple(
        failure.safe_failure_contract_ref().clone(),
        &stable_code,
        outcome,
        failure.failure_class(),
        failure.boundary_stage(),
        failure.coarse_size_class(),
        failure.diagnostic_ref().is_some(),
    )
    .map_err(|_| StoreError::AuthorizationNotEligible)?;
    let failure = SafeFailure::new(
        failure.safe_failure_contract_ref(),
        &stable_code,
        failure.failure_class(),
        failure.boundary_stage(),
        failure.coarse_size_class(),
        None,
    )?;
    let outcome = match outcome {
        SafeFailureOutcome::DidNotEnter => ObservationOutcome::did_not_enter(&failure)?,
        SafeFailureOutcome::Indeterminate => ObservationOutcome::indeterminate(&failure)?,
    };
    let observation = ExternalAccessObserved::new(authorization_ref, &outcome, None)?;
    let objects = PreparedObjectGraph::prepare_for_record_values(
        &[observation.canonical_value()?],
        Vec::new(),
    )?;
    ObserveExternalAccess::new(view, append_request_id, observation, objects, None)
}

fn prepare_returned_effect_observation(
    view: &VerifiedRunView,
    append_request_id: AppendRequestId,
    authorization_ref: &AuthorizationRef,
    result: VerifiedEnsureResult,
) -> Result<ObserveExternalAccess> {
    let entry = unobserved_entry(view, authorization_ref)?;
    let authorization = entry.authorization().fields()?;
    let AuthorizationScopeFields::EnsureEffect {
        effect_request_transition_ref,
    } = authorization.scope.fields()?
    else {
        return Err(StoreError::UnknownAuthorization);
    };
    let pending = view
        .pending_effects()?
        .into_iter()
        .find(|pending| pending.request_transition_ref() == &effect_request_transition_ref)
        .ok_or(StoreError::AuthorizationNotEligible)?;
    validate_effect_identity(view, &pending, &result)?;
    let node = view
        .certified_spec()
        .nodes()
        .iter()
        .find(|node| node.node_id() == pending.node_id())
        .ok_or(StoreError::AuthorizationNotEligible)?;
    let CertifiedStateExecution::Effect {
        ensure_result_contract,
        terminal_evidence_contract,
        ..
    } = node.execution()
    else {
        return Err(StoreError::AuthorizationNotEligible);
    };

    let (_, ensure, _, closure) = result.into_parts();
    if closure.contract().ensure_result_contract() != ensure_result_contract
        || closure.contract().terminal_evidence_contract() != terminal_evidence_contract
    {
        return Err(StoreError::AuthorizationNotEligible);
    }
    let mut retained = BTreeMap::new();
    let mut authorities = Vec::new();
    for member in closure.into_members() {
        let reference = member
            .reference()
            .map_err(|_| StoreError::JournalContract)?;
        let path = retained_member_path(member.relation(), &reference)?;
        let value_ref = derive_value_ref(
            member.contract(),
            &ProducerBinding::external_observation(authorization_ref, &path)?,
            member.value().canonical_json().as_bytes(),
        )?;
        if retained
            .insert((member.relation(), reference), value_ref.clone())
            .is_some()
        {
            return Err(StoreError::InvalidObjectAuthority {
                message: "verified executor closure contains a duplicate relation",
            });
        }
        authorities.push(PreparedAuthority::produced(
            value_ref,
            member.value().canonical_json().to_vec(),
        )?);
    }

    let ensure_result = match ensure {
        Ensure::Pending { delivery_audit_ref } => {
            let delivery_audit_ref = retained_value(
                &retained,
                ExecutorRetainedValueRelation::DeliveryAudit,
                delivery_audit_ref.as_content_ref(),
            )?;
            ExecutorEnsureResult::pending(delivery_audit_ref)?
        }
        Ensure::Terminal { evidence } => {
            let claim = evidence.claim();
            let delivery_audit_ref = retained_value(
                &retained,
                ExecutorRetainedValueRelation::DeliveryAudit,
                claim.delivery_audit_ref().as_content_ref(),
            )?;
            let terminal_tombstone_ref = retained_value(
                &retained,
                ExecutorRetainedValueRelation::TerminalTombstone,
                claim.terminal_tombstone_ref().as_content_ref(),
            )?;
            let domain_evidence_ref = retained_value(
                &retained,
                ExecutorRetainedValueRelation::DomainEvidence,
                claim.domain_evidence_ref(),
            )?;
            let evidence_value = TerminalEffectEvidence::new(
                pending.executor_binding_ref(),
                pending.effect_key(),
                pending.request_digest(),
                delivery_audit_ref,
                terminal_tombstone_ref,
                &StableId::new(claim.external_operation_identity())?,
                &StableId::new(claim.terminal_outcome())?,
                claim.assurance_policy_ref(),
                &journal_proof_basis(claim.proof_basis())?,
                domain_evidence_ref,
            )?;
            let evidence_ref = derive_value_ref(
                terminal_evidence_contract,
                &ProducerBinding::external_observation(
                    authorization_ref,
                    &field_path(TERMINAL_EVIDENCE_PATH)?,
                )?,
                evidence_value.as_bytes(),
            )?;
            authorities.push(PreparedAuthority::produced(
                evidence_ref.clone(),
                evidence_value.as_bytes().to_vec(),
            )?);
            ExecutorEnsureResult::terminal(&evidence_ref)?
        }
    };
    let result_ref = derive_value_ref(
        ensure_result_contract,
        &ProducerBinding::external_observation(
            authorization_ref,
            &field_path(ENSURE_RESULT_PATH)?,
        )?,
        ensure_result.as_bytes(),
    )?;
    authorities.push(PreparedAuthority::produced(
        result_ref.clone(),
        ensure_result.as_bytes().to_vec(),
    )?);
    let observation = ExternalAccessObserved::new(
        authorization_ref,
        &ObservationOutcome::returned(&result_ref)?,
        None,
    )?;
    let objects = PreparedObjectGraph::prepare_for_record_values(
        &[observation.canonical_value()?],
        authorities,
    )?;
    ObserveExternalAccess::new(view, append_request_id, observation, objects, None)
}

fn validate_effect_identity(
    view: &VerifiedRunView,
    pending: &super::FoldedPendingEffect,
    result: &VerifiedEnsureResult,
) -> Result<()> {
    let identity = result.identity();
    if identity.tenant_scope_id() != view.tenant_scope_id()
        || identity.executor_binding_ref().as_content_ref()
            != &pending.executor_binding_ref().fields()?
        || identity.effect_key() != pending.effect_key()
        || identity.request_digest() != pending.request_digest()
    {
        return Err(StoreError::AuthorizationNotEligible);
    }
    Ok(())
}

fn retained_value<'a>(
    retained: &'a BTreeMap<(ExecutorRetainedValueRelation, ContentRef), ValueRef>,
    relation: ExecutorRetainedValueRelation,
    reference: &ContentRef,
) -> Result<&'a ValueRef> {
    retained
        .get(&(relation, reference.clone()))
        .ok_or(StoreError::InvalidObjectAuthority {
            message: "verified executor closure omits a required relation",
        })
}

fn retained_member_path(
    relation: ExecutorRetainedValueRelation,
    reference: &ContentRef,
) -> Result<FieldPath> {
    let relation = match relation {
        ExecutorRetainedValueRelation::DeliveryAudit => "delivery_audit",
        ExecutorRetainedValueRelation::ExecutorFrontier => "frontier",
        ExecutorRetainedValueRelation::TerminalTombstone => "terminal_tombstone",
        ExecutorRetainedValueRelation::TerminalProof => "terminal_proof",
        ExecutorRetainedValueRelation::DomainEvidence => "domain_evidence",
    };
    field_path(&format!(
        "executor.{relation}.{}",
        reference.content_digest().digest()
    ))
}

fn journal_proof_basis(proof: &ProofBasis) -> Result<ExecutorProofBasis> {
    match proof {
        ProofBasis::SelfAuthenticatingProof => {
            ExecutorProofBasis::self_authenticating_proof().map_err(Into::into)
        }
        ProofBasis::ExecutorAttestation {
            evidence_authority_ref,
        } => ExecutorProofBasis::executor_attestation(evidence_authority_ref).map_err(Into::into),
        ProofBasis::TrustedObserver {
            evidence_authority_ref,
        } => ExecutorProofBasis::trusted_observer(evidence_authority_ref).map_err(Into::into),
    }
}

fn unobserved_entry<'a>(
    view: &'a VerifiedRunView,
    authorization_ref: &AuthorizationRef,
) -> Result<&'a super::FoldedAccessAuditEntry> {
    let entry = view
        .access_audit_entries()
        .find(|entry| entry.authorization_ref() == authorization_ref)
        .ok_or(StoreError::UnknownAuthorization)?;
    if entry.observation().is_some() {
        return Err(StoreError::ObservationAlreadyCommitted);
    }
    Ok(entry)
}

fn semantic_anchor(
    view: &VerifiedRunView,
    node_id: &mfm_ids::NodeId,
    node_phase: NodePhase,
) -> Result<SemanticAnchor> {
    SemanticAnchor::new(
        view.journal_head(),
        view.run_state_digest(),
        node_id,
        node_phase,
    )
    .map_err(Into::into)
}

fn observation_value_ref(
    authorization_ref: &AuthorizationRef,
    path: &str,
    root: &ProducedObjectRoot,
) -> Result<ValueRef> {
    let path = field_path(path)?;
    derive_value_ref(
        root.value_contract(),
        &ProducerBinding::external_observation(authorization_ref, &path)?,
        root.canonical().as_bytes(),
    )
}

fn derive_request_digest(
    request_schema_id: &str,
    request: &PlainCanonicalJsonBytes,
) -> Result<RequestDigest> {
    let request_value: serde_json::Value =
        serde_json::from_slice(request.as_bytes()).map_err(|_| StoreError::JournalContract)?;
    let preimage = serde_json::json!({
        "request_schema_id": request_schema_id,
        "request_value": request_value,
    });
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&preimage).map_err(|_| StoreError::JournalContract)?,
    )
    .map_err(|_| StoreError::JournalContract)?;
    let contract = RecoverabilityContractV2::embedded()?;
    let validated = contract.strict_decode(REQUEST_PREIMAGE_SCHEMA, canonical.as_bytes())?;
    contract
        .derive_request_digest(&validated)
        .map_err(Into::into)
}

fn validate_read_binding(
    view: &VerifiedRunView,
    operation_id: &StableId,
    binding_ref: &ContentRef,
    routing_generation_ref: &ContentRef,
) -> Result<()> {
    let admitted = view
        .capability_binding_manifest()
        .entries()
        .iter()
        .find(|entry| entry.operation_id == *operation_id)
        .ok_or(StoreError::AuthorizationNotEligible)?;
    if admitted.binding_ref != *binding_ref {
        return Err(StoreError::AuthorizationNotEligible);
    }
    let binding = ReadCapabilityBinding::strict_decode(view.retained_content(binding_ref)?)?;
    let binding = binding.fields()?;
    view.retained_content(routing_generation_ref)?;
    if !content_ref_reachable(view, &binding.routing_catalog_ref, routing_generation_ref)? {
        return Err(StoreError::AuthorizationNotEligible);
    }
    Ok(())
}

fn content_ref_reachable(
    view: &VerifiedRunView,
    root: &ContentRef,
    target: &ContentRef,
) -> Result<bool> {
    let mut queue = VecDeque::from([root.clone()]);
    let mut visited = BTreeSet::new();
    while let Some(reference) = queue.pop_front() {
        if !visited.insert(reference.clone()) {
            continue;
        }
        if &reference == target {
            return Ok(true);
        }
        let bytes = match view.retained_content(&reference) {
            Ok(bytes) => bytes,
            Err(StoreError::ObjectNotReachable) => continue,
            Err(error) => return Err(error),
        };
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| StoreError::JournalContract)?;
        collect_content_refs(&value, &mut queue);
    }
    Ok(false)
}

fn collect_content_refs(value: &serde_json::Value, output: &mut VecDeque<ContentRef>) {
    match value {
        serde_json::Value::Object(entries) => {
            if let Ok(reference) = serde_json::from_value::<ContentRef>(value.clone()) {
                output.push_back(reference);
                return;
            }
            for nested in entries.values() {
                collect_content_refs(nested, output);
            }
        }
        serde_json::Value::Array(entries) => {
            for nested in entries {
                collect_content_refs(nested, output);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn field_path(value: &str) -> Result<FieldPath> {
    FieldPath::new(value).map_err(Into::into)
}
