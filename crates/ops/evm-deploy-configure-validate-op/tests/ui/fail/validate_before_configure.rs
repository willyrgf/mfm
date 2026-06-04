use mfm_op_evm_deploy_configure_validate::{
    ConfiguredContract, DeployConfigureValidateDeployConfig, DeployConfigureValidateValidateConfig,
    DeployContractState, ValidateContractState,
};
use mfm_program::{Handle, ScopeBuilder, StateKey};

fn validate_before_configure<'program, 'scope>(
    builder: &mut ScopeBuilder<'program, 'scope>,
    deploy_config: DeployConfigureValidateDeployConfig,
    validate_config: DeployConfigureValidateValidateConfig,
) -> mfm_program::Result<()> {
    let deployed = builder.state::<DeployContractState, _>(
        StateKey::new("deploy_contract")?,
        deploy_config,
        (),
    )?;

    let configured: Handle<'program, 'scope, ConfiguredContract> = deployed;

    let _invalid = builder.state::<ValidateContractState, _>(
        StateKey::new("validate_contract")?,
        validate_config,
        configured,
    )?;

    Ok(())
}

fn main() {}
