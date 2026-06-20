#![warn(missing_docs)]
//! Typed proof workflow operation.
//!
//! The proof workflow is authored through `mfm-program` and lowers to certified typed state
//! programs. It does not expose the legacy dynamic `PlannedOp`, `PortKey`, context-key, or generic
//! IO surfaces.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_proof::{proof_program_draft, ProofWorkflowConfig};
//!
//! let draft = proof_program_draft(ProofWorkflowConfig::default()).unwrap();
//! assert_eq!(draft.state_nodes().len(), 3);
//! ```

use mfm_certify::{certify_program_draft, CertifiedSchemaRole, CertifiedTypedSpec};
pub use mfm_collectors_proof::{
    proof_adapter_kind, proof_adapter_version, ProofApplyConfig, ProofApplySideEffectState,
    ProofAssembleConfig, ProofAssembleInput, ProofAssembleInputHandles, ProofAssembleOutputState,
    ProofConfirmation, ProofFact, ProofFactRequest, ProofFactResponse, ProofIdempotencyInput,
    ProofIntent, ProofMutationCapability, ProofOperationOutputs, ProofOutput, ProofPublicOutputs,
    ProofReadCapability, ProofReadConfig, ProofReadFactState, ProofReceipt, ProofReplayError,
    ProofReplayVerifier, ProofSideEffectResult, ProofSubmission, ProofWorkflowConfig,
    RecordedProofFacts, MANUAL_RESOLUTION_PROOF_ACTION,
};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion, SchemaId};
use mfm_program::{
    build_root_with_registries, ManualAuthorizationDraft, ManualAuthorizationVerifierId,
    ManualResolutionPolicyDraft, ManualSigningSchemeSpec, NonEmptyUniqueOperators, Operation,
    OperationExpansion, OperationKey, OperationRegistryBuilder, OperatorAuthorityId,
    OperatorAuthorityMemberSpec, OperatorAuthoritySnapshotDraft, OperatorId,
    OperatorPublicIdentity, PublicOutputKey, ResourceClaim, RootBuilder, ScopeKey,
    SideEffectSagaPolicy, StateKey, StateRegistryBuilder, ThresholdQuorum,
};

const PROOF_OPERATION_KIND_NAME: &str = "workflow";
const PROOF_OPERATION_VERSION: &str = "mfm.proof.operation.workflow.v1";
const ROOT_SCOPE: &str = "proof";
const OP_KEY: &str = "proof_workflow";
const PUBLIC_OUTPUT_KEY: &str = "proof";
const MANUAL_EVIDENCE_SCHEMA_NAME: &str = "mfm.proof.manual_resolution.evidence";
const MANUAL_VERIFIER_ID: &str = "mfm.proof.manual_resolution.verifier";
const MANUAL_AUTHORITY_ID: &str = "mfm.proof.manual_resolution.authority";
const MANUAL_OPERATOR_ID: &str = "operator.proof.manual_resolution";
const MANUAL_OPERATOR_PUBLIC_IDENTITY: &str = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";

/// Typed proof workflow operation.
pub struct ProofWorkflowOperation;

impl Operation for ProofWorkflowOperation {
    type Config = ProofWorkflowConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ProofOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            "mfm.proof",
            PROOF_OPERATION_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.proof.operation:workflow"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(PROOF_OPERATION_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.proof.workflow"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let fact =
            builder.state::<ProofReadFactState, _>(StateKey::new("read_fact")?, config.read, ())?;
        let side_effect = builder.side_effect::<ProofApplySideEffectState, _>(
            StateKey::new("apply_side_effect")?,
            config.apply,
            fact.clone(),
            ResourceClaim::manual_only(),
        )?;
        let output = builder.state::<ProofAssembleOutputState, _>(
            StateKey::new("assemble_output")?,
            ProofAssembleConfig {},
            ProofAssembleInputHandles {
                fact,
                side_effect: side_effect.into_handle(),
            },
        )?;
        Ok(ProofOperationOutputs { output })
    }
}

/// Builds the proof state registry used for authoring and certification.
pub fn proof_state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
    let mut states = StateRegistryBuilder::new();
    states.register::<ProofReadFactState>()?;
    states.register::<ProofApplySideEffectState>()?;
    states.register::<ProofAssembleOutputState>()?;
    Ok(states.into_snapshot())
}

/// Builds the proof operation registry used for authoring and certification.
pub fn proof_operation_registry() -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
    let mut operations = OperationRegistryBuilder::new();
    operations.register::<ProofWorkflowOperation>()?;
    Ok(operations.into_snapshot())
}

/// Adds proof workflow descriptors to a trusted certification registry.
pub fn register_proof_certification_descriptors(
    registry: &mut mfm_certify::CertificationRegistry,
) -> mfm_certify::Result<()> {
    let mut states = StateRegistryBuilder::new();
    registry.register_state(
        &states
            .register::<ProofReadFactState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ProofApplySideEffectState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ProofAssembleOutputState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    let mut operations = OperationRegistryBuilder::new();
    registry.register_operation(
        &operations
            .register::<ProofWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    register_proof_manual_resolution_authority(registry)?;
    Ok(())
}

/// Builds a typed proof program draft.
pub fn proof_program_draft(
    config: ProofWorkflowConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    proof_program_draft_with_saga_policy(config, SideEffectSagaPolicy::FailWithoutAcdcClaim)
}

/// Builds a typed proof program draft that blocks for certified manual resolution.
pub fn manual_resolution_proof_program_draft(
    mut config: ProofWorkflowConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    config.apply = ProofApplyConfig::new(MANUAL_RESOLUTION_PROOF_ACTION)
        .map_err(mfm_program::PlanError::ManualPolicy)?;
    proof_program_draft_with_saga_policy(
        config,
        SideEffectSagaPolicy::ManualResolution {
            manual: proof_manual_resolution_policy()?,
        },
    )
}

fn proof_program_draft_with_saga_policy(
    config: ProofWorkflowConfig,
    saga_policy: SideEffectSagaPolicy,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        proof_state_registry()?,
        proof_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(saga_policy)?;
            let result = root.scope().call::<ProofWorkflowOperation, _>(
                OperationKey::new(OP_KEY)?,
                ProofWorkflowOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &ProofPublicOutputs {
                    output: result.output,
                },
            )
        },
    )
}

/// Builds the certified manual-resolution policy used by the proof manual-resolution scenario.
pub fn proof_manual_resolution_policy() -> mfm_program::Result<ManualResolutionPolicyDraft> {
    let operator = OperatorAuthorityMemberSpec {
        operator_id: OperatorId::new(MANUAL_OPERATOR_ID).map_err(program_key_error)?,
        public_identity: OperatorPublicIdentity::new(MANUAL_OPERATOR_PUBLIC_IDENTITY)
            .map_err(program_key_error)?,
    };
    let authority = OperatorAuthoritySnapshotDraft::new(
        OperatorAuthorityId::new(MANUAL_AUTHORITY_ID).map_err(program_key_error)?,
        NonEmptyUniqueOperators::new(operator, Vec::new())?,
    );
    let authorization = ManualAuthorizationDraft::threshold(
        ManualAuthorizationVerifierId::new(MANUAL_VERIFIER_ID).map_err(program_key_error)?,
        ManualSigningSchemeSpec::new(mfm_certify::MANUAL_RESOLUTION_SIGNING_SCHEME)
            .map_err(program_key_error)?,
        authority,
        ThresholdQuorum::new(1)?,
    )?;
    Ok(ManualResolutionPolicyDraft::new(
        proof_manual_resolution_evidence_schema_id()?,
        authorization,
    ))
}

/// Returns the certified schema id used for proof manual-resolution evidence.
pub fn proof_manual_resolution_evidence_schema_id() -> mfm_program::Result<SchemaId> {
    SchemaId::new(
        MANUAL_EVIDENCE_SCHEMA_NAME,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(MANUAL_EVIDENCE_SCHEMA_NAME.as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Builds and certifies the typed proof program.
pub fn certified_proof_spec(
    config: ProofWorkflowConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = proof_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    certify_program_draft(&draft)
}

/// Builds and certifies the proof workflow variant that awaits manual resolution.
pub fn certified_manual_resolution_proof_spec(
    config: ProofWorkflowConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = manual_resolution_proof_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    let mut registry = mfm_certify::CertificationRegistry::from_program_draft(&draft)?;
    register_proof_manual_resolution_authority(&mut registry)?;
    let lowered = mfm_certify::lower_program_draft(&draft)?;
    mfm_certify::certify_typed_spec(lowered, &registry)
}

fn register_proof_manual_resolution_authority(
    registry: &mut mfm_certify::CertificationRegistry,
) -> mfm_certify::Result<()> {
    let manual = proof_manual_resolution_policy()
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?
        .to_spec();
    registry.register_schema_role(
        manual.evidence_schema.clone(),
        CertifiedSchemaRole::ManualResolutionEvidence,
    )?;
    registry.register_manual_authorization_verifier(manual.authorization.verifier_id.clone())?;
    registry.register_operator_authority_snapshot(manual.authorization.authority)?;
    Ok(())
}

fn program_key_error(error: impl std::fmt::Display) -> mfm_program::PlanError {
    mfm_program::PlanError::Key(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_program_lowers_to_typed_state_contracts() {
        let draft = proof_program_draft(ProofWorkflowConfig::default()).expect("draft");
        assert_eq!(draft.state_nodes().len(), 3);
        assert!(
            draft
                .state_nodes()
                .iter()
                .all(|node| !node.state_descriptor_name.contains("DynContext")),
            "proof state descriptors must not expose dynamic context"
        );

        let certified = certify_program_draft(&draft).expect("certified proof spec");
        certified.envelope().verify_hash().expect("hash verifies");
        let side_effect = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "apply_side_effect")
            .expect("side-effect node");
        assert!(
            side_effect.side_effect.is_some(),
            "proof side effect declares a typed side-effect contract"
        );
        assert_eq!(side_effect.adapter_bindings.len(), 1);
    }

    #[test]
    fn proof_manual_resolution_program_certifies() {
        let certified = certified_manual_resolution_proof_spec(ProofWorkflowConfig::default())
            .expect("certified manual proof spec");
        assert!(matches!(
            certified.envelope().spec.saga,
            mfm_spec::v1::SagaPolicySpec::ManualResolution { .. }
        ));
        assert!(
            certified.envelope().spec.nodes.iter().any(|node| matches!(
                node.framework,
                Some(mfm_spec::v1::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )),
            "manual-resolution proof spec must include a saga terminal resolver"
        );
    }
}
