#[path = "../support/types.rs"]
mod types;

use types::*;

fn main() -> mfm_program::Result<()> {
    let mut registry = mfm_program::StateRegistryBuilder::new();
    registry.register::<TryPureState>()?;
    registry.register::<TrySideEffectState>()?;
    registry.register::<TryCompensationState>()?;

    let _draft = mfm_program::build_root_with_registry(
        mfm_program::ScopeKey::new("root")?,
        registry.snapshot(),
        |root| {
            root.set_saga_policy(mfm_program::SagaPolicy::CompensateCompleted {
                on_remediation_unresolved: mfm_program::RemediationUnresolved::FailWithoutAcdcClaim,
            })?;
            let seed = root.seed(mfm_program::SeedKey::new("input")?, seed()?)?;
            let (_forward, remediation) = root
                .scope()
                .side_effect_with_compensation::<
                    TrySideEffectState,
                    TryCompensationState,
                    _,
                    _,
                    _,
                >(
                    mfm_program::StateKey::new("forward")?,
                    TryConfig { multiplier: 2 },
                    seed,
                    mfm_program::StateKey::new("compensate-forward")?,
                    TryConfig { multiplier: 3 },
                    |forward| Ok(forward),
                )?;
            let result = root.scope().state::<TryPureState, _>(
                mfm_program::StateKey::new("uses-remediation")?,
                TryConfig { multiplier: 4 },
                remediation,
            )?;
            root.bind_public_outputs(
                mfm_program::PublicOutputKey::new("terminal")?,
                &TryPublicOutputs { result },
            )
        },
    )?;

    Ok(())
}
