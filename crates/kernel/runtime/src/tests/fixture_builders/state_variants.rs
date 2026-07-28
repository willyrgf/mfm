use super::*;

pub(in crate::tests::support) fn fixture_with_first_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(in crate::tests::support) fn fixture_with_first_exclusive_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(in crate::tests::support) fn fixture_with_first_finalized_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(in crate::tests::support) fn fixture_with_first_exact_touched_set_side_effect_state() -> Fixture
{
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ExactTouchedSet,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(in crate::tests::support) fn fixture_with_first_exact_touched_set_finalized_side_effect_state(
) -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ExactTouchedSet,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(in crate::tests::support) fn fixture_with_manual_resolution_side_effect_state() -> Fixture {
    let mut fixture = fixture_with_first_side_effect_state();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let manual = spec::ManualResolutionEvidenceSpec {
        evidence_schema: fixture.seed_ref.schema_id.clone(),
        authorization: manual_authorization(0xe0),
    };
    envelope.spec.saga = spec::SagaPolicySpec::ManualResolution {
        manual: Box::new(manual),
    };
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

pub(in crate::tests::support) fn manual_authorization(
    byte: u8,
) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

pub(in crate::tests::support) fn fixture_with_independent_second_node_and_first_side_effect_state(
) -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::IndependentSecond,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

pub(in crate::tests::support) fn fixture_with_independent_second_node_and_first_exclusive_finalized_side_effect_state(
) -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::IndependentSecond,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

pub(in crate::tests::support) fn fixture_with_two_side_effects_and_failing_tail() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}
