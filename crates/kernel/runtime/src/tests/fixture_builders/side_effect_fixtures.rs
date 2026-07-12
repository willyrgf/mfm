use super::*;

#[derive(Clone, Copy)]
pub(in crate::tests::support) enum RuntimeSideEffectClaim {
    ManualOnly,
    Exclusive,
    ExactTouchedSet,
}

impl RuntimeSideEffectClaim {
    fn into_resource_claim(self) -> ResourceClaim {
        match self {
            Self::ManualOnly => ResourceClaim::manual_only(),
            Self::Exclusive => ResourceClaim::exclusive(
                exclusive_resource_namespace(),
                <CertifierValue as mfm_values::MfmValue>::schema_id()
                    .expect("exclusive key schema"),
            ),
            Self::ExactTouchedSet => ResourceClaim::exact_touched_set(
                exact_touched_set_resource_namespace(),
                <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
                    .expect("touched-set evidence schema"),
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::tests::support) enum RuntimeSideEffectFixtureShape {
    Chained,
    IndependentSecond,
    CompensatingPairWithFailingTail,
}

pub(in crate::tests::support) fn runtime_side_effect_fixture(
    shape: RuntimeSideEffectFixtureShape,
    claim: RuntimeSideEffectClaim,
    verification: spec::SideEffectVerificationSpec,
) -> Fixture {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<RuntimeSubmitAState>()
        .expect("submit a registration");
    states
        .register::<RuntimeSubmitBState>()
        .expect("submit b registration");
    states
        .register::<RuntimeReadState>()
        .expect("read registration");
    states
        .register::<RuntimeSeedReadState>()
        .expect("seed read registration");
    states
        .register::<RuntimeTailState>()
        .expect("tail registration");
    let seed = CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed");
    let seed_digest = seed.content_digest().clone();
    let seed_byte_len = seed.byte_len() as u64;
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            if matches!(
                shape,
                RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail
            ) {
                root.set_saga_policy(SideEffectSagaPolicy::CompensateCompleted {
                    on_remediation_unresolved:
                        mfm_program::RemediationUnresolved::FailWithoutAcdcClaim,
                })?;
            } else {
                root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            }
            let input = root.seed(mfm_program::SeedKey::new("initial")?, seed.clone())?;
            match shape {
                RuntimeSideEffectFixtureShape::Chained => {
                    let forward = root.scope().side_effect::<RuntimeSubmitAState, _>(
                        StateKey::new("a")?,
                        NoContext,
                        CertifierConfig { multiplier: 3 },
                        input,
                        claim.into_resource_claim(),
                        verification.clone(),
                    )?;
                    let result = root.scope().state::<RuntimeReadState, _>(
                        StateKey::new("b")?,
                        NoContext,
                        CertifierConfig { multiplier: 5 },
                        forward.into_handle(),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &FixturePublicOutputs { result },
                    )
                }
                RuntimeSideEffectFixtureShape::IndependentSecond => {
                    let forward = root.scope().side_effect::<RuntimeSubmitAState, _>(
                        StateKey::new("a")?,
                        NoContext,
                        CertifierConfig { multiplier: 3 },
                        input.clone(),
                        claim.into_resource_claim(),
                        verification.clone(),
                    )?;
                    let side_effect = forward.into_handle();
                    let result = root.scope().state::<RuntimeSeedReadState, _>(
                        StateKey::new("b")?,
                        NoContext,
                        CertifierConfig { multiplier: 5 },
                        input,
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &DualFixturePublicOutputs {
                            result,
                            side_effect,
                        },
                    )
                }
                RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail => {
                    let (forward_a, _remediation_a) = root
                        .scope()
                        .side_effect_with_compensation::<
                            RuntimeSubmitAState,
                            RuntimeSubmitBState,
                            _,
                            _,
                            _,
                        >(
                            NoContext,
                            NoContext,
                            SideEffectNodeParams {
                                key: StateKey::new("a")?,
                                config: CertifierConfig { multiplier: 3 },
                                input,
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            RemediationNodeParams {
                                key: StateKey::new("remediate-a")?,
                                config: CertifierConfig { multiplier: 1 },
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            |forward| Ok(forward.into_handle()),
                        )?;
                    let (forward_b, _remediation_b) = root
                        .scope()
                        .side_effect_with_compensation::<
                            RuntimeSubmitBState,
                            RuntimeSubmitBState,
                            _,
                            _,
                            _,
                        >(
                            NoContext,
                            NoContext,
                            SideEffectNodeParams {
                                key: StateKey::new("b")?,
                                config: CertifierConfig { multiplier: 7 },
                                input: forward_a.into_handle(),
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            RemediationNodeParams {
                                key: StateKey::new("remediate-b")?,
                                config: CertifierConfig { multiplier: 1 },
                                resource_claim: claim.into_resource_claim(),
                                verification,
                            },
                            |forward| Ok(forward.into_handle()),
                        )?;
                    let result = root.scope().state::<RuntimeTailState, _>(
                        StateKey::new("c")?,
                        NoContext,
                        CertifierConfig { multiplier: 1 },
                        forward_b.into_handle(),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &FixturePublicOutputs { result },
                    )
                }
            }
        },
    )
    .expect("side-effect draft");
    let certified = mfm_certify::certify_program_draft(&draft).expect("certified side-effect spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    fixture_from_runtime_spec(shape, runtime_spec, seed_digest, seed_byte_len)
}
