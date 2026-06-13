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

use mfm_certify::{certify_program_draft, CertifiedTypedSpec};
pub use mfm_collectors_proof::{
    proof_adapter_kind, proof_adapter_version, ProofApplyConfig, ProofApplySideEffectState,
    ProofAssembleConfig, ProofAssembleInput, ProofAssembleInputHandles, ProofAssembleOutputState,
    ProofConfirmation, ProofFact, ProofFactRequest, ProofFactResponse, ProofIdempotencyInput,
    ProofIntent, ProofMutationCapability, ProofOperationOutputs, ProofOutput, ProofPublicOutputs,
    ProofReadCapability, ProofReadConfig, ProofReadFactState, ProofReceipt, ProofReplayError,
    ProofReplayVerifier, ProofSideEffectResult, ProofSubmission, ProofWorkflowConfig,
    RecordedProofFacts,
};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, Operation, OperationExpansion, OperationKey,
    OperationRegistryBuilder, PublicOutputKey, RootBuilder, SagaPolicy, ScopeKey, StateKey,
    StateRegistryBuilder,
};

const PROOF_OPERATION_KIND_NAME: &str = "workflow";
const PROOF_OPERATION_VERSION: &str = "mfm.proof.operation.workflow.v1";
const ROOT_SCOPE: &str = "proof";
const OP_KEY: &str = "proof_workflow";
const PUBLIC_OUTPUT_KEY: &str = "proof";

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
        config: Self::Config,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        if config.workflow_version != 1 {
            return Err(mfm_program::PlanError::Key(
                "unsupported proof workflow config version".to_owned(),
            ));
        }
        let fact = builder.state::<ProofReadFactState, _>(
            StateKey::new("read_fact")?,
            ProofReadConfig {
                fact_n: config.fact_n,
            },
            (),
        )?;
        let side_effect = builder.state::<ProofApplySideEffectState, _>(
            StateKey::new("apply_side_effect")?,
            ProofApplyConfig {
                action: config.action,
            },
            fact.clone(),
        )?;
        let output = builder.state::<ProofAssembleOutputState, _>(
            StateKey::new("assemble_output")?,
            ProofAssembleConfig { output_version: 1 },
            ProofAssembleInputHandles { fact, side_effect },
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
    Ok(())
}

/// Builds a typed proof program draft.
pub fn proof_program_draft(
    config: ProofWorkflowConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        proof_state_registry()?,
        proof_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(SagaPolicy::FailWithoutAcdcClaim)?;
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

/// Builds and certifies the typed proof program.
pub fn certified_proof_spec(
    config: ProofWorkflowConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = proof_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    certify_program_draft(&draft)
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
}
