use super::*;

pub(super) fn verify_untrusted_spec_certificate_parts(
    persisted_parts: UntrustedSpecCertificateParts,
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    let UntrustedSpecCertificateParts {
        spec,
        certificate: expected_certificate,
    } = persisted_parts;
    expected_certificate.verify_hash()?;
    let actual_spec_hash = spec
        .spec()
        .spec_hash()
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
    if expected_certificate.evidence.spec_hash != actual_spec_hash {
        return Err(certificate(format!(
            "certificate/spec hash mismatch: certificate {}, recomputed {}",
            expected_certificate.evidence.spec_hash, actual_spec_hash
        )));
    }

    let expected_registry_digest = registry.digest()?;
    if expected_certificate.evidence.registry_digest != expected_registry_digest {
        return Err(certificate(format!(
            "registry digest mismatch: certificate {}, current {}",
            expected_certificate.evidence.registry_digest, expected_registry_digest
        )));
    }

    let spec_ref = spec.spec();
    let expected_descriptor_evidence = descriptor_evidence_for_spec(spec_ref)?;
    if expected_certificate.evidence.descriptor_identities != expected_descriptor_evidence {
        return Err(certificate(
            "descriptor identity evidence does not match persisted spec",
        ));
    }
    let expected_schema_role_grants = schema_role_grants_for_spec(spec_ref, registry)?;
    if expected_certificate.evidence.schema_role_grants != expected_schema_role_grants {
        return Err(certificate(
            "schema role grant evidence does not match persisted spec",
        ));
    }
    let expected_verifiers = manual_authorization_verifiers_for_spec(spec_ref, registry)?;
    if expected_certificate.evidence.manual_authorization_verifiers != expected_verifiers {
        return Err(certificate(
            "manual authorization verifier evidence does not match persisted spec",
        ));
    }
    let expected_authorities = operator_authority_snapshots_for_spec(spec_ref, registry)?;
    if expected_certificate.evidence.operator_authority_snapshots != expected_authorities {
        return Err(certificate(
            "operator authority snapshot evidence does not match persisted spec",
        ));
    }
    let certified = certify_typed_spec(spec, registry)?;
    if certified.certificate != expected_certificate {
        return Err(certificate(
            "persisted certificate does not match registry-backed certification",
        ));
    }
    Ok(certified)
}

pub(super) fn certificate_for_envelope(
    envelope: &spec::HashedSpecEnvelope,
    registry: &CertificationRegistry,
) -> Result<CertifiedSpecCertificate> {
    CertifiedSpecCertificate::from_evidence(CertifiedSpecCertificateEvidence {
        certificate_version: CERTIFICATE_VERSION.to_owned(),
        media_type: CERTIFICATE_MEDIA_TYPE.to_owned(),
        certifier_algorithm: CERTIFIER_ALGORITHM.to_owned(),
        spec_hash: envelope.spec_hash.clone(),
        registry_digest: registry.digest()?,
        descriptor_identities: descriptor_evidence_for_spec(&envelope.spec)?,
        schema_role_grants: schema_role_grants_for_spec(&envelope.spec, registry)?,
        manual_authorization_verifiers: manual_authorization_verifiers_for_spec(
            &envelope.spec,
            registry,
        )?,
        operator_authority_snapshots: operator_authority_snapshots_for_spec(
            &envelope.spec,
            registry,
        )?,
    })
}

pub(super) fn descriptor_evidence_for_spec(
    spec: &spec::TypedExecutionSpec,
) -> Result<Vec<CertifiedDescriptorEvidence>> {
    let mut evidence = spec
        .descriptor_identities
        .iter()
        .map(|descriptor| {
            let reference = descriptor
                .descriptor_ref()
                .map_err(|error| CertifyError::Spec(error.to_string()))?;
            Ok(CertifiedDescriptorEvidence {
                descriptor_family: reference.family,
                descriptor_id: reference.descriptor_id,
                descriptor_digest: reference.descriptor_digest,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    evidence.sort_by(|left, right| {
        (
            left.descriptor_family,
            left.descriptor_id.as_str(),
            left.descriptor_digest.as_str(),
        )
            .cmp(&(
                right.descriptor_family,
                right.descriptor_id.as_str(),
                right.descriptor_digest.as_str(),
            ))
    });
    Ok(evidence)
}

pub(super) fn schema_role_grants_for_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<CertifiedSchemaRoleGrantEvidence>> {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for manual in manual_resolution_specs(spec) {
        let role = CertifiedSchemaRole::ManualResolutionEvidence;
        let key = (manual.evidence_schema.as_str().to_owned(), role);
        if !seen.insert(key) {
            continue;
        }
        let Some(roles) = registry.schema_roles.get(manual.evidence_schema.as_str()) else {
            return Err(certificate(format!(
                "manual evidence schema {} is missing from registry",
                manual.evidence_schema
            )));
        };
        if !roles.contains(&role) {
            return Err(certificate(format!(
                "manual evidence schema {} lacks certified role {}",
                manual.evidence_schema,
                role.as_str()
            )));
        }
        evidence.push(CertifiedSchemaRoleGrantEvidence {
            schema_id: manual.evidence_schema.clone(),
            role,
        });
    }
    evidence.sort_by(|left, right| {
        (left.schema_id.as_str(), left.role.as_str())
            .cmp(&(right.schema_id.as_str(), right.role.as_str()))
    });
    Ok(evidence)
}

pub(super) fn manual_authorization_verifiers_for_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<CertifiedManualAuthorizationVerifierEvidence>> {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for manual in manual_resolution_specs(spec) {
        let verifier_id = &manual.authorization.verifier_id;
        if !seen.insert(verifier_id.as_str().to_owned()) {
            continue;
        }
        if !registry
            .manual_authorization_verifiers
            .contains(verifier_id.as_str())
        {
            return Err(certificate(format!(
                "manual authorization verifier {} is missing from registry",
                verifier_id
            )));
        }
        evidence.push(CertifiedManualAuthorizationVerifierEvidence {
            verifier_id: verifier_id.clone(),
        });
    }
    evidence.sort_by(|left, right| left.verifier_id.as_str().cmp(right.verifier_id.as_str()));
    Ok(evidence)
}

pub(super) fn operator_authority_snapshots_for_spec(
    spec: &spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<CertifiedOperatorAuthoritySnapshotEvidence>> {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for manual in manual_resolution_specs(spec) {
        let authority_id = &manual.authorization.authority.authority_id;
        if !seen.insert(authority_id.as_str().to_owned()) {
            continue;
        }
        let Some(snapshot) = registry
            .operator_authority_snapshots
            .get(authority_id.as_str())
        else {
            return Err(certificate(format!(
                "operator authority snapshot {} is missing from registry",
                authority_id
            )));
        };
        evidence.push(operator_authority_snapshot_evidence(snapshot)?);
    }
    evidence.sort_by(|left, right| left.authority_id.as_str().cmp(right.authority_id.as_str()));
    Ok(evidence)
}

pub(super) fn operator_authority_snapshot_evidence(
    snapshot: &spec::OperatorAuthoritySnapshotSpec,
) -> Result<CertifiedOperatorAuthoritySnapshotEvidence> {
    Ok(CertifiedOperatorAuthoritySnapshotEvidence {
        authority_id: snapshot.authority_id.clone(),
        authority_digest: operator_authority_snapshot_digest(snapshot)?,
        operators: snapshot
            .operators
            .iter()
            .map(|operator| CertifiedOperatorAuthorityMemberEvidence {
                operator_id: operator.operator_id.clone(),
                public_identity: operator.public_identity.clone(),
            })
            .collect(),
    })
}

pub(super) fn operator_authority_snapshot_digest(
    snapshot: &spec::OperatorAuthoritySnapshotSpec,
) -> Result<ContentDigest> {
    content_digest_json(operator_authority_snapshot_json(snapshot))
}

pub(super) fn operator_authority_snapshot_json(
    snapshot: &spec::OperatorAuthoritySnapshotSpec,
) -> serde_json::Value {
    serde_json::json!({
        "authority_id": snapshot.authority_id.as_str(),
        "operators": snapshot
            .operators
            .iter()
            .map(|operator| {
                serde_json::json!({
                    "operator_id": operator.operator_id.as_str(),
                    "public_identity": operator.public_identity.as_str(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

pub(super) fn manual_resolution_specs(
    spec: &spec::TypedExecutionSpec,
) -> Vec<&spec::ManualResolutionEvidenceSpec> {
    let mut manuals = Vec::new();
    match &spec.saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manuals.push(manual),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manuals.push(manual.as_ref()),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted { .. } => {}
    }
    manuals
}
