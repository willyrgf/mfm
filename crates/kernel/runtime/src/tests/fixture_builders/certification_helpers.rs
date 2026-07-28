use super::*;

pub(in crate::tests::support) fn certify_fixture_spec(
    source: &CertifiedRuntimeSpec,
    spec: spec::TypedExecutionSpec,
    configure_registry: impl FnOnce(&mut mfm_certify::CertificationRegistry) -> mfm_certify::Result<()>,
) -> mfm_certify::Result<mfm_certify::CertifiedTypedSpec> {
    let mut registry = mfm_certify::CertificationRegistry::new();
    for descriptor in &source.spec().descriptor_identities {
        match descriptor {
            spec::DescriptorIdentity::State(state)
                if state.name == RuntimeContextSourceState::name() =>
            {
                registry.register_state::<RuntimeContextSourceState>()?;
            }
            spec::DescriptorIdentity::State(state)
                if state.name == RuntimeContextConsumerState::name() =>
            {
                registry.register_state::<RuntimeContextConsumerState>()?;
            }
            spec::DescriptorIdentity::State(state) if !state.name.starts_with("mfm.framework.") => {
                registry.register_state_descriptor(state.as_ref().clone())?;
            }
            spec::DescriptorIdentity::Operation(operation) => {
                registry.register_operation_descriptor(operation.as_ref().clone())?;
            }
            spec::DescriptorIdentity::State(_) | spec::DescriptorIdentity::Renderer(_) => {}
        }
    }
    configure_registry(&mut registry)?;
    mfm_certify::certify_typed_spec(spec::UntrustedTypedSpec::from_raw_spec(spec), &registry)
}

pub(in crate::tests::support) fn recertified_runtime_spec(
    source: &CertifiedRuntimeSpec,
) -> CertifiedRuntimeSpec {
    let certified = certify_fixture_spec(source, source.spec().clone(), |registry| {
        let manual = match &source.spec().saga {
            spec::SagaPolicySpec::ManualResolution { manual } => Some(manual.as_ref()),
            spec::SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved:
                    spec::RemediationUnresolvedSpec::ManualResolution { manual },
            } => Some(manual.as_ref()),
            spec::SagaPolicySpec::NoSideEffects
            | spec::SagaPolicySpec::FailWithoutAcdcClaim
            | spec::SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
            } => None,
        };
        if let Some(manual) = manual {
            registry.register_schema_role(
                manual.evidence_schema.clone(),
                mfm_certify::CertifiedSchemaRole::ManualResolutionEvidence,
            )?;
            registry
                .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())?;
            registry
                .register_operator_authority_snapshot(manual.authorization.authority.clone())?;
        }
        Ok(())
    })
    .expect("independent fixture recertification");
    CertifiedRuntimeSpec::new(certified).expect("recertified runtime spec")
}
