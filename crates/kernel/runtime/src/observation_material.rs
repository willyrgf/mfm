//! Exact reconstruction of values retained by one committed observation.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{EffectKey, FieldPath, RequestDigest};
use mfm_journal::{
    AuthorizationRef, CapabilityBindingRef, ExecutorEnsureResult, ExecutorEnsureResultFields,
    ObservationOutcomeFields, ProducerBindingFields, TerminalEffectEvidence, ValueRef,
};
use mfm_program::{
    ProgramError, TerminalEffectResolver, VerifiedReadOutcome, VerifiedValueMaterial,
};
use mfm_spec::RetainedValueContract;
use mfm_store::{VerifiedObservedAccess, VerifiedRunView};

use crate::access_protocol::{CommittedObservation, Ensure, Read};
use crate::{Result, RuntimeError};

const ENSURE_RESULT_PATH: &str = "executor.ensure_result";
const TERMINAL_EVIDENCE_PATH: &str = "executor.terminal_evidence";

/// Exact committed terminal evidence reconstructed from one ensure observation.
pub(crate) struct CommittedTerminalEvidence {
    evidence: TerminalEffectEvidence,
    evidence_ref: ValueRef,
}

impl CommittedTerminalEvidence {
    pub(crate) const fn evidence(&self) -> &TerminalEffectEvidence {
        &self.evidence
    }

    pub(crate) const fn evidence_ref(&self) -> &ValueRef {
        &self.evidence_ref
    }
}

pub(crate) fn committed_read_observation(
    view: &VerifiedRunView,
    observed: &VerifiedObservedAccess<'_>,
    returned_contract: &RetainedValueContract,
    safe_failure_contract: &RetainedValueContract,
    requires_fact_selection_attestation: bool,
) -> Result<CommittedObservation<Read, VerifiedReadOutcome>> {
    let audit = observed.audit();
    let observation_ref = observed.observation_ref();
    let observation = observed.observation();
    let fields = observation.fields()?;
    if fields.authorization_ref != *audit.authorization_ref()
        || fields.fact_selection_scan_attestation_ref.is_some()
            != requires_fact_selection_attestation
    {
        return Err(RuntimeError::InvalidCallbackResult);
    }
    let outcome = match fields.outcome.fields()? {
        ObservationOutcomeFields::Returned { result_ref } => {
            let path =
                FieldPath::new("outcome.result_ref").map_err(mfm_ids::IdentityError::from)?;
            VerifiedReadOutcome::Returned(verified_observation_value(
                view,
                audit.authorization_ref(),
                &path,
                &result_ref,
                returned_contract,
            )?)
        }
        ObservationOutcomeFields::DidNotEnter { safe_failure } => {
            let (metadata, diagnostic) = verified_safe_failure(
                view,
                audit.authorization_ref(),
                &safe_failure,
                safe_failure_contract,
            )?;
            VerifiedReadOutcome::DidNotEnter {
                metadata,
                diagnostic,
            }
        }
        ObservationOutcomeFields::Indeterminate { safe_failure } => {
            let (metadata, diagnostic) = verified_safe_failure(
                view,
                audit.authorization_ref(),
                &safe_failure,
                safe_failure_contract,
            )?;
            VerifiedReadOutcome::Indeterminate {
                metadata,
                diagnostic,
            }
        }
        ObservationOutcomeFields::NonDomainFailure { .. } => {
            return Err(RuntimeError::InvalidCallbackResult);
        }
    };
    Ok(CommittedObservation::new(observation_ref.clone(), outcome))
}

fn verified_safe_failure(
    view: &VerifiedRunView,
    authorization_ref: &AuthorizationRef,
    failure: &mfm_journal::SafeFailure,
    contract: &RetainedValueContract,
) -> Result<(
    mfm_store::SafeFailureMetadata,
    Option<VerifiedValueMaterial>,
)> {
    let fields = failure.fields()?;
    let metadata = mfm_store::SafeFailureMetadata::new(
        fields.safe_failure_contract_ref,
        fields.stable_code,
        fields.failure_class,
        fields.boundary_stage,
        fields.coarse_size_class,
    );
    let path = FieldPath::new("outcome.safe_failure.diagnostic_ref")
        .map_err(mfm_ids::IdentityError::from)?;
    let diagnostic = fields
        .diagnostic_ref
        .as_ref()
        .map(|diagnostic_ref| {
            verified_observation_value(view, authorization_ref, &path, diagnostic_ref, contract)
        })
        .transpose()?;
    Ok((metadata, diagnostic))
}

pub(crate) fn verified_observation_value(
    view: &VerifiedRunView,
    authorization_ref: &AuthorizationRef,
    field_path: &FieldPath,
    value_ref: &ValueRef,
    contract: &RetainedValueContract,
) -> Result<VerifiedValueMaterial> {
    value_ref
        .validate_contract(contract)
        .map_err(RuntimeError::Journal)?;
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::ExternalObservation {
            authorization_ref: producer_authorization,
            field_path: producer_path,
        } if producer_authorization == *authorization_ref && producer_path == *field_path => {}
        _ => return Err(RuntimeError::InvalidCallbackResult),
    }
    let retained = view.retained_value(value_ref)?;
    Ok(VerifiedValueMaterial::new(
        PlainCanonicalJsonBytes::from_canonical_json_slice(retained.bytes())?,
        value_ref.clone(),
    ))
}

/// Reconstructs one verified terminal executor result, or `None` for a
/// structurally valid nonterminal observation.
pub(crate) fn committed_terminal_observation(
    view: &VerifiedRunView,
    observed: &VerifiedObservedAccess<'_>,
    executor_binding_ref: &CapabilityBindingRef,
    effect_key: &EffectKey,
    request_digest: &RequestDigest,
    ensure_result_contract: &RetainedValueContract,
    terminal_evidence_contract: &RetainedValueContract,
) -> Result<Option<CommittedObservation<Ensure, CommittedTerminalEvidence>>> {
    let audit = observed.audit();
    let observation_ref = observed.observation_ref();
    let observation = observed.observation();
    let observation = observation.fields()?;
    if observation.authorization_ref != *audit.authorization_ref()
        || observation.fact_selection_scan_attestation_ref.is_some()
    {
        return Err(RuntimeError::InvalidCallbackResult);
    }
    let ObservationOutcomeFields::Returned { result_ref } = observation.outcome.fields()? else {
        return Ok(None);
    };
    let result_path = FieldPath::new(ENSURE_RESULT_PATH).map_err(mfm_ids::IdentityError::from)?;
    let result = verified_observation_value(
        view,
        audit.authorization_ref(),
        &result_path,
        &result_ref,
        ensure_result_contract,
    )?;
    let result = ExecutorEnsureResult::strict_decode(result.canonical().as_bytes())?;
    let ExecutorEnsureResultFields::Terminal { evidence_ref } = result.fields()? else {
        return Ok(None);
    };
    let evidence_path =
        FieldPath::new(TERMINAL_EVIDENCE_PATH).map_err(mfm_ids::IdentityError::from)?;
    let material = verified_observation_value(
        view,
        audit.authorization_ref(),
        &evidence_path,
        &evidence_ref,
        terminal_evidence_contract,
    )?;
    let evidence = TerminalEffectEvidence::strict_decode(material.canonical().as_bytes())?;
    let fields = evidence.fields()?;
    if fields.executor_binding_ref != *executor_binding_ref
        || fields.effect_key != *effect_key
        || fields.request_digest != *request_digest
    {
        return Err(RuntimeError::EffectIdentityMismatch);
    }
    Ok(Some(CommittedObservation::new(
        observation_ref.clone(),
        CommittedTerminalEvidence {
            evidence,
            evidence_ref,
        },
    )))
}

/// Resolver restricted to the exact object family produced by one committed
/// executor observation.
pub(crate) struct CommittedEffectResolver<'view> {
    view: &'view VerifiedRunView,
    authorization_ref: &'view AuthorizationRef,
}

impl<'view> CommittedEffectResolver<'view> {
    pub(crate) const fn new(
        view: &'view VerifiedRunView,
        authorization_ref: &'view AuthorizationRef,
    ) -> Self {
        Self {
            view,
            authorization_ref,
        }
    }
}

impl TerminalEffectResolver for CommittedEffectResolver<'_> {
    fn resolve(
        &self,
        value_ref: &ValueRef,
    ) -> std::result::Result<PlainCanonicalJsonBytes, ProgramError> {
        let fields = value_ref.fields().map_err(|_| {
            ProgramError::Codec("terminal evidence reference is invalid".to_owned())
        })?;
        let ProducerBindingFields::ExternalObservation {
            authorization_ref,
            field_path,
        } = fields
            .producer_binding
            .fields()
            .map_err(|_| ProgramError::Codec("terminal evidence producer is invalid".to_owned()))?
        else {
            return Err(ProgramError::Codec(
                "terminal evidence is outside the committed observation".to_owned(),
            ));
        };
        let digest = fields.content_digest.digest().to_string();
        if authorization_ref != *self.authorization_ref || !executor_path(&field_path, &digest) {
            return Err(ProgramError::Codec(
                "terminal evidence is outside the committed executor closure".to_owned(),
            ));
        }
        let retained = self
            .view
            .retained_value(value_ref)
            .map_err(|_| ProgramError::Codec("terminal evidence is unreachable".to_owned()))?;
        PlainCanonicalJsonBytes::from_canonical_json_slice(retained.bytes())
            .map_err(|_| ProgramError::Codec("terminal evidence is not canonical".to_owned()))
    }
}

fn executor_path(path: &FieldPath, content_digest: &str) -> bool {
    let path = path.as_str();
    path == ENSURE_RESULT_PATH
        || path == TERMINAL_EVIDENCE_PATH
        || [
            "executor.delivery_audit.",
            "executor.frontier.",
            "executor.terminal_tombstone.",
            "executor.terminal_proof.",
            "executor.domain_evidence.",
        ]
        .iter()
        .any(|prefix| path.strip_prefix(prefix) == Some(content_digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_resolver_accepts_only_the_executor_observation_namespace() {
        for path in [
            ENSURE_RESULT_PATH,
            TERMINAL_EVIDENCE_PATH,
            "executor.delivery_audit.abc",
            "executor.frontier.abc",
            "executor.terminal_tombstone.abc",
            "executor.terminal_proof.abc",
            "executor.domain_evidence.abc",
        ] {
            assert!(
                executor_path(&FieldPath::new(path).unwrap(), "abc"),
                "{path}"
            );
        }
        for path in [
            "outcome.result_ref",
            "outcome.safe_failure.diagnostic_ref",
            "executor.unknown.abc",
            "executor.frontier",
        ] {
            assert!(
                !executor_path(&FieldPath::new(path).unwrap(), "abc"),
                "{path}"
            );
        }
    }
}
