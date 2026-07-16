use mfm_evm_contract_model::*;
use mfm_ids::{ContentDigest, ContextRef, DigestAlgorithm, DigestBytes};
use mfm_program::StateContext;
use mfm_spec::v1::{CertifiedContextSpec, StateContextDescriptorSpec};
use mfm_values::{ContextBoundOutput, ContextRefValue, MfmValue};

fn digest_with(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn context_ref(byte: u8) -> ContextRef {
    ContextRef::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(byte))
}

fn contract_context(chain_id: u64) -> EvmContractContext {
    EvmContractContext {
        lifecycle_key: LifecycleKey::new("example-contract").expect("lifecycle key"),
        network: EvmNetworkContext::new(
            EvmNetworkId::new("ethereum-mainnet").expect("network id"),
            chain_id,
        )
        .expect("network context"),
        contract_profile: ContractProfile {
            profile_id: ContractProfileId::new("example-profile").expect("profile id"),
            artifact_digest: Some(ContractProfileDigestRef::from(content_digest(0x20))),
            artifact_ref: None,
            interface_digest: Some(ContractProfileDigestRef::from(content_digest(0x21))),
            creation_bytecode_digest: None,
            deployed_code_hash: None,
            selector_event_policy_digest: None,
        },
    }
}

fn derived_context_ref(context: &EvmContractContext) -> ContextRef {
    let StateContextDescriptorSpec::Required(requirement) =
        <EvmContractContext as StateContext>::descriptor().expect("descriptor")
    else {
        panic!("contract context must require context");
    };
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(context).expect("context JSON"),
    )
    .expect("canonical context");
    CertifiedContextSpec::derive_context_ref(
        &requirement.context_descriptor_id,
        &requirement.schema_id,
        &requirement.semantic_type_id,
        &requirement.canonicalizer_identity,
        &canonical,
    )
    .expect("context ref")
}

#[test]
fn expected_matches_normalizes_hex_wrappers() {
    let actual = ExpectedValue::from_json_value(&serde_json::json!("0xAA")).expect("actual");
    let expected = ExpectedValue::from_json_value(&serde_json::json!("0xaa")).expect("expected");

    assert!(expected_matches(&actual, &expected));
}

#[test]
fn typed_json_boundaries_reject_floats() {
    assert!(AbiArgumentValue::from_json_value(&serde_json::json!(1.5)).is_err());
    assert!(ExpectedValue::from_json_value(&serde_json::json!(1.5)).is_err());
    assert!(
        serde_json::from_value::<EvmNetworkContext>(serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1.5,
        }))
        .is_err()
    );
}

#[test]
fn contract_context_refs_are_stable_content_addresses() {
    assert_eq!(
        derived_context_ref(&contract_context(1)),
        derived_context_ref(&contract_context(1))
    );
    assert_ne!(
        derived_context_ref(&contract_context(1)),
        derived_context_ref(&contract_context(31337))
    );
}

#[test]
fn direct_contract_values_expose_only_direct_lineage() {
    let context = ContextRefValue::new(context_ref(0x40));
    let address =
        ContractAddress::new("0X1111111111111111111111111111111111111111").expect("address");
    let deployed = DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: context.clone(),
        address: address.clone(),
        deployed_block_number: 10,
    };
    let configured = ConfiguredContractInstance {
        lifecycle_version: 1,
        context_ref: context.clone(),
        address: address.clone(),
        configured_from: ConfiguredFrom {
            deployed_context_ref: context.clone(),
            deployed_address: address,
        },
    };

    assert_eq!(deployed.context_ref(), context.as_context_ref());
    assert_eq!(deployed.context_stage(), deployed_contract_stage());
    assert_eq!(configured.context_stage(), configured_contract_stage());
    assert_eq!(
        ConfiguredContractInstanceRef::from_configured(&configured).configured_from,
        configured.configured_from
    );
    let encoded = serde_json::to_value(&configured).expect("configured JSON");
    assert!(encoded.get("configuration_claim").is_none());
}

#[test]
fn lifecycle_values_have_current_schema_ids() {
    let schema_ids = [
        DeployedContractInstance::schema_id()
            .expect("schema")
            .to_string(),
        ConfiguredContractInstance::schema_id()
            .expect("schema")
            .to_string(),
        ConfiguredContractInstanceRef::schema_id()
            .expect("schema")
            .to_string(),
        ContextBoundValidationReport::schema_id()
            .expect("schema")
            .to_string(),
        ValidationSourceEvidence::schema_id()
            .expect("schema")
            .to_string(),
    ];

    assert!(schema_ids
        .iter()
        .all(|schema_id| schema_id.contains("mfm.evm.contract")));
}

#[test]
fn lifecycle_artifact_evidence_refs_round_trip_typed_ids() {
    let digest = content_digest(0x55);
    let evidence = LifecycleArtifactEvidenceRef::new(
        mfm_ids::ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
        digest.clone(),
        digest,
        128,
        None,
        None,
    );

    let decoded: LifecycleArtifactEvidenceRef =
        serde_json::from_value(serde_json::to_value(&evidence).expect("JSON"))
            .expect("decoded evidence");
    assert_eq!(decoded, evidence);
}
