use super::*;

pub(super) fn parse_certificate(value: &serde_json::Value) -> Result<CertifiedSpecCertificate> {
    let object = json_object(value, "typed spec certificate")?;
    Ok(CertifiedSpecCertificate {
        certificate_hash: parse_identity(required_str(object, "certificate_hash")?)?,
        evidence: parse_certificate_evidence(required(object, "evidence")?)?,
    })
}

pub(super) fn parse_certificate_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedSpecCertificateEvidence> {
    let object = json_object(value, "typed spec certificate evidence")?;
    let certificate_version = required_str(object, "certificate_version")?.to_owned();
    if certificate_version != CERTIFICATE_VERSION {
        return Err(certificate(format!(
            "unsupported certificate_version {certificate_version:?}"
        )));
    }
    let media_type = required_str(object, "media_type")?.to_owned();
    if media_type != CERTIFICATE_MEDIA_TYPE {
        return Err(certificate(format!(
            "unsupported certificate media_type {media_type:?}"
        )));
    }
    let certifier_algorithm = required_str(object, "certifier_algorithm")?.to_owned();
    if certifier_algorithm != CERTIFIER_ALGORITHM {
        return Err(certificate(format!(
            "unsupported certifier_algorithm {certifier_algorithm:?}"
        )));
    }
    Ok(CertifiedSpecCertificateEvidence {
        certificate_version,
        media_type,
        certifier_algorithm,
        spec_hash: parse_identity(required_str(object, "spec_hash")?)?,
        registry_digest: parse_identity(required_str(object, "registry_digest")?)?,
        descriptor_identities: parse_array(
            required(object, "descriptor_identities")?,
            parse_descriptor_evidence,
        )?,
        schema_role_grants: parse_array(
            required(object, "schema_role_grants")?,
            parse_schema_role_grant_evidence,
        )?,
        manual_authorization_verifiers: parse_array(
            required(object, "manual_authorization_verifiers")?,
            parse_manual_authorization_verifier_evidence,
        )?,
        operator_authority_snapshots: parse_array(
            required(object, "operator_authority_snapshots")?,
            parse_operator_authority_snapshot_evidence,
        )?,
    })
}

pub(super) fn parse_schema_role_grant_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedSchemaRoleGrantEvidence> {
    let object = json_object(value, "schema role grant evidence")?;
    Ok(CertifiedSchemaRoleGrantEvidence {
        schema_id: parse_identity(required_str(object, "schema_id")?)?,
        role: CertifiedSchemaRole::parse(required_str(object, "role")?)?,
    })
}

pub(super) fn parse_manual_authorization_verifier_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedManualAuthorizationVerifierEvidence> {
    let object = json_object(value, "manual authorization verifier evidence")?;
    Ok(CertifiedManualAuthorizationVerifierEvidence {
        verifier_id: spec::ManualAuthorizationVerifierId::new(required_str(object, "verifier_id")?)
            .map_err(|error| certificate(error.to_string()))?,
    })
}

pub(super) fn parse_operator_authority_snapshot_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedOperatorAuthoritySnapshotEvidence> {
    let object = json_object(value, "operator authority snapshot evidence")?;
    Ok(CertifiedOperatorAuthoritySnapshotEvidence {
        authority_id: spec::OperatorAuthorityId::new(required_str(object, "authority_id")?)
            .map_err(|error| certificate(error.to_string()))?,
        authority_digest: parse_identity(required_str(object, "authority_digest")?)?,
        operators: parse_array(
            required(object, "operators")?,
            parse_operator_authority_member_evidence,
        )?,
    })
}

pub(super) fn parse_operator_authority_member_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedOperatorAuthorityMemberEvidence> {
    let object = json_object(value, "operator authority member evidence")?;
    Ok(CertifiedOperatorAuthorityMemberEvidence {
        operator_id: spec::OperatorId::new(required_str(object, "operator_id")?)
            .map_err(|error| certificate(error.to_string()))?,
        public_identity: spec::OperatorPublicIdentity::new(required_str(
            object,
            "public_identity",
        )?)
        .map_err(|error| certificate(error.to_string()))?,
    })
}

pub(super) fn parse_descriptor_evidence(
    value: &serde_json::Value,
) -> Result<CertifiedDescriptorEvidence> {
    let object = json_object(value, "descriptor certificate evidence")?;
    Ok(CertifiedDescriptorEvidence {
        descriptor_family: CertifiedDescriptorFamily::parse(required_str(
            object,
            "descriptor_family",
        )?)
        .map_err(|error| certificate(error.to_string()))?,
        descriptor_id: parse_identity(required_str(object, "descriptor_id")?)?,
        descriptor_digest: parse_identity(required_str(object, "descriptor_digest")?)?,
    })
}

pub(super) fn parse_array<T>(
    value: &serde_json::Value,
    parser: fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    json_array(value, "array")?.iter().map(parser).collect()
}

pub(super) fn parse_identity<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    value
        .parse()
        .map_err(|error| certificate(format!("identity parse failed: {error}")))
}

pub(super) fn json_object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| certificate(format!("{context} must be a JSON object")))
}

pub(super) fn json_array<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| certificate(format!("{context} must be a JSON array")))
}

pub(super) fn required<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a serde_json::Value> {
    object
        .get(field)
        .ok_or_else(|| certificate(format!("missing required field {field}")))
}

pub(super) fn required_str<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a str> {
    json_string(required(object, field)?, field)
}

pub(super) fn json_string<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| certificate(format!("{field} must be a string")))
}

pub(super) fn descriptor_id_json(value: serde_json::Value) -> Result<DescriptorId> {
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    ))
}

pub(super) fn state_descriptor_id_from_spec(
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "capabilities": capability_set_json(&descriptor.capabilities),
        "config_schema_id": descriptor.config_schema_id.as_str(),
        "context": state_context_descriptor_json(&descriptor.context),
        "effect": {
            "class": descriptor.effect_class.as_str(),
            "kind": descriptor.effect_kind.as_str(),
            "name": descriptor.effect_name.as_str(),
            "version": descriptor.effect_version.as_str(),
        },
        "emitted_fact_descriptors": fact_descriptor_refs_json(&descriptor.emitted_fact_descriptors),
        "input_context": state_input_context_contract_json(&descriptor.input_context),
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.state_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_context": state_output_context_contract_json(&descriptor.output_context),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "output_semantic_type_id": descriptor.output_semantic_type_id.as_str(),
        "runner": descriptor.runner.as_str(),
        "side_effect_contract_digest": descriptor.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
        "version": descriptor.state_version.as_str(),
    }))
}

pub(super) fn state_context_descriptor_json(
    context: &spec::StateContextDescriptorSpec,
) -> serde_json::Value {
    match context {
        spec::StateContextDescriptorSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateContextDescriptorSpec::Required(requirement) => {
            let spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id,
                schema_id,
                semantic_type_id,
                canonicalizer_identity,
            } = requirement.as_ref();
            serde_json::json!({
                "canonicalizer_identity": canonicalizer_identity.as_str(),
                "context_descriptor_id": context_descriptor_id.as_str(),
                "kind": "required",
                "schema_id": schema_id.as_str(),
                "semantic_type_id": semantic_type_id.as_str(),
            })
        }
    }
}

pub(super) fn state_input_context_contract_json(
    contract: &spec::StateInputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateInputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateInputContextContractSpec::Required {
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

pub(super) fn state_output_context_contract_json(
    contract: &spec::StateOutputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateOutputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => serde_json::json!({
            "kind": "produces",
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

pub(super) fn operation_descriptor_id_from_spec(
    descriptor: &spec::OperationDescriptorIdentity,
) -> Result<DescriptorId> {
    descriptor_id_json(serde_json::json!({
        "config_schema_id": descriptor.config_schema_id.as_str(),
        "expansion_abi": descriptor.expansion_abi.as_str(),
        "input_schema_id": descriptor.input_schema_id.as_str(),
        "kind": descriptor.operation_kind.as_str(),
        "name": descriptor.name.as_str(),
        "output_schema_id": descriptor.output_schema_id.as_str(),
        "version": descriptor.operation_version.as_str(),
    }))
}

pub(super) fn state_kind_json(name: &str, value: serde_json::Value) -> Result<StateKind> {
    StateKind::new(
        "mfm.framework",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes_json(value)?,
    )
    .map_err(|error| lower(error.to_string()))
}
