use mfm_capabilities::NoCaps;
use mfm_effects::Pure;
use mfm_evm_contract_model::{
    contract_instance_resource_kind, deployed_contract_stage, ContextBoundValidationReport,
    ContractAddress, DeployedContractInstance, EvmBlockHash, EvmContractContext,
};
use mfm_ids::{DigestAlgorithm, StateKind, StateVersion};
use mfm_program::{
    build_root_with_registries, PublicOutputKey, PureState, ScopeKey, SideEffectSagaPolicy,
    SideEffectVerificationSpec, StateError, StateKey, StateResult, StateSpec,
};
use mfm_program_derive::PublicOutputs;
use mfm_state_evm_contracts::{
    account_nonce_resource_claim, ConfigureAction, ContextBoundConfigureContractState,
    ContextBoundDeployContractState, ContextBoundValidateContractState,
    ContextConfigureContractInputHandles, ContextValidateContractInputHandles, DeployAction,
    ValidateAction,
};
use mfm_values::ContextRefValue;

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.test.direct_state_outputs")]
struct DirectStateOutputs<'program, 'scope> {
    validation: mfm_program::Handle<'program, 'scope, ContextBoundValidationReport>,
}

struct UnapprovedDeployedProducer;

impl StateSpec for UnapprovedDeployedProducer {
    type Config = DeployAction;
    type Context = EvmContractContext;
    type Input = ();
    type Output = DeployedContractInstance;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.evm.contract.test",
            "unapproved_deployed_producer",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(
                b"mfm.evm.contract.test.unapproved_deployed_producer",
            ),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.evm.contract.test.unapproved_deployed_producer.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.contract.test.unapproved_deployed_producer"
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        Ok(mfm_program::StateOutputContextContractSpec::Produces {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: deployed_contract_stage().clone(),
        })
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for UnapprovedDeployedProducer {
    fn run(
        &self,
        _input: Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(DeployedContractInstance {
            lifecycle_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            address: ContractAddress::new("0x000000000000000000000000000000000000beef")
                .map_err(|error| StateError::Message(error.to_string()))?,
            deployed_block_number: 1,
            deployed_block_hash: EvmBlockHash::new(format!("0x{}", "01".repeat(32)))
                .map_err(|error| StateError::Message(error.to_string()))?,
        })
    }
}

fn deploy_action() -> DeployAction {
    serde_json::from_value(serde_json::json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead"
        }
    }))
    .expect("deploy action")
}

fn configure_action() -> ConfigureAction {
    serde_json::from_value(serde_json::json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead"
        },
        "calls": []
    }))
    .expect("configure action")
}

fn contract_context() -> EvmContractContext {
    serde_json::from_value(serde_json::json!({
        "lifecycle_key": "state-test-contract",
        "network": {"network_id": "ethereum-mainnet", "expected_chain_id": 1},
        "contract_profile": {"profile_id": "state-test-profile"}
    }))
    .expect("contract context")
}

fn state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
    let mut states = mfm_program::StateRegistryBuilder::new();
    states.register::<ContextBoundDeployContractState>()?;
    states.register::<ContextBoundConfigureContractState>()?;
    states.register::<ContextBoundValidateContractState>()?;
    states.register::<UnapprovedDeployedProducer>()?;
    Ok(states.into_snapshot())
}

fn direct_state_graph(
    use_unapproved_deployed_producer: bool,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new("evm_contract_direct_state_test")?,
        state_registry()?,
        mfm_program::OperationRegistryBuilder::new().into_snapshot(),
        |root| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let context = root.scope().declare_context(contract_context())?;
            let deployed = if use_unapproved_deployed_producer {
                root.scope().state::<UnapprovedDeployedProducer, _>(
                    StateKey::new("unapproved_deploy")?,
                    &context,
                    deploy_action(),
                    (),
                )?
            } else {
                root.scope()
                    .side_effect::<ContextBoundDeployContractState, _>(
                        StateKey::new("deploy")?,
                        &context,
                        deploy_action(),
                        (),
                        account_nonce_resource_claim()?,
                        SideEffectVerificationSpec::Finalized { depth: 1 },
                    )?
                    .into_handle()
            };
            let configured = root
                .scope()
                .side_effect::<ContextBoundConfigureContractState, _>(
                    StateKey::new("configure")?,
                    &context,
                    configure_action(),
                    ContextConfigureContractInputHandles { deployed },
                    account_nonce_resource_claim()?,
                    SideEffectVerificationSpec::Finalized { depth: 1 },
                )?
                .into_handle();
            let validation = root.scope().state::<ContextBoundValidateContractState, _>(
                StateKey::new("validate")?,
                &context,
                ValidateAction::default(),
                ContextValidateContractInputHandles { configured },
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("validation")?,
                &DirectStateOutputs { validation },
            )
        },
    )
}

#[test]
fn direct_deploy_configure_validate_graph_certifies_without_an_operation_or_app_registration() {
    let graph = direct_state_graph(false).expect("direct state graph");
    mfm_certify::certify_program_draft(&graph).expect("direct state graph certifies");
}

#[test]
fn same_output_type_from_an_unapproved_descriptor_fails_certification() {
    let graph = direct_state_graph(true).expect("graph construction defers producer checking");
    let error = mfm_certify::certify_program_draft(&graph)
        .expect_err("unapproved deployed producer must fail certification");
    assert!(
        error.to_string().contains("InvalidSemanticTransition"),
        "{error}"
    );
}
