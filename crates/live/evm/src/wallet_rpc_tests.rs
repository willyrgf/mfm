use alloy_primitives::{address, b256, Address, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    EvmSubmitTransactionRequest, EvmTransactionTarget, EvmWalletAccessListEntry,
    EvmWalletConvergencePlan, EvmWalletFeeCandidate, EvmWalletPolicy, EvmWalletReference,
    EvmWalletReplacementPolicy, EvmWalletTransactionAction, EvmWalletTransactionCandidate,
    EvmWalletTransactionTemplate,
};
use mfm_executor::{EvidenceBounds, SchemaQualifiedCanonicalValue};
use mfm_ids::{DigestAlgorithm, SchemaId, TenantScopeId};
use mfm_program::boundary_content_ref;

use super::*;

const SENDER: Address = address!("1111111111111111111111111111111111111111");
const RECIPIENT: Address = address!("2222222222222222222222222222222222222222");

fn reference(label: &str) -> EvmWalletReference {
    let schema = SchemaId::new(
        "mfm.test.evm-wallet-live-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"schema:mfm.test.evm-wallet-live-reference:1"),
    )
    .expect("schema");
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&serde_json::json!({"label": label}).to_string())
            .expect("canonical");
    EvmWalletReference::from_content_ref(
        boundary_content_ref(schema, &canonical).expect("content ref"),
    )
}

fn request() -> EvmSubmitTransactionRequest {
    let template = EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("primary").expect("target"),
        EvmWalletTransactionAction::call(RECIPIENT),
        U256::from(5),
        [0xde, 0xad],
        vec![EvmWalletAccessListEntry::new(
            RECIPIENT,
            vec![b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )],
        )
        .expect("access list")],
        U256::from(75_000),
    )
    .expect("template");
    let replacement = EvmWalletReplacementPolicy::new(vec![
        EvmWalletFeeCandidate::new(U256::from(20), U256::from(2)).expect("fee"),
        EvmWalletFeeCandidate::new(U256::from(30), U256::from(3)).expect("fee"),
    ])
    .expect("replacement");
    let policy = EvmWalletPolicy::new(
        reference("wallet-domain"),
        TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000001").expect("tenant"),
        reference("route-generation"),
        1,
        SENDER,
        reference("signer-binding"),
        mfm_evm::evm_wallet_nonce_policy_ref().expect("nonce policy"),
        7,
        reference("initial-nonce"),
        replacement,
        evm_already_known_classifier_ref().expect("classifier"),
        reference("finality-policy"),
        reference("assurance-policy"),
        EvmWalletConvergencePlan::new(2, 2, 2, 2, 2, 16 * 1024).expect("plan"),
        EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 32 * 1024, 2, 32 * 1024).expect("bounds"),
    )
    .expect("policy");
    let template_ref = {
        let bytes = mfm_program::encode_boundary(&template).expect("template");
        let schema = EvmWalletTransactionTemplate::schema_id().expect("schema");
        EvmWalletReference::from_content_ref(
            SchemaQualifiedCanonicalValue::new(schema, bytes.as_bytes())
                .expect("value")
                .reference()
                .expect("reference"),
        )
    };
    let policy_ref = {
        let bytes = mfm_program::encode_boundary(&policy).expect("policy");
        let schema = EvmWalletPolicy::schema_id().expect("schema");
        EvmWalletReference::from_content_ref(
            SchemaQualifiedCanonicalValue::new(schema, bytes.as_bytes())
                .expect("value")
                .reference()
                .expect("reference"),
        )
    };
    EvmSubmitTransactionRequest::new(template_ref, template, policy_ref, policy).expect("request")
}

fn candidate(request: &EvmSubmitTransactionRequest) -> EvmWalletTransactionCandidate {
    EvmWalletTransactionCandidate::new(
        request.clone(),
        7,
        0,
        b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    )
    .expect("candidate")
}

#[test]
fn candidate_specific_descriptors_round_trip_without_bearer_material() {
    let request = request();
    let candidate = candidate(&request);
    let descriptors = [
        EvmWalletTargetEntryDescriptor::broadcast(&request, &candidate).expect("broadcast"),
        EvmWalletTargetEntryDescriptor::transaction_lookup(&request, &candidate)
            .expect("transaction"),
        EvmWalletTargetEntryDescriptor::receipt_lookup(&request, &candidate).expect("receipt"),
        EvmWalletTargetEntryDescriptor::finalized_head(&request, &candidate).expect("finalized"),
        EvmWalletTargetEntryDescriptor::canonical_inclusion(&request, &candidate, U256::from(100))
            .expect("inclusion"),
    ];
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.wire.rpc_method.as_str())
            .collect::<Vec<_>>(),
        [
            EVM_SEND_RAW_TRANSACTION_METHOD,
            EVM_TRANSACTION_BY_HASH_METHOD,
            EVM_RECEIPT_BY_HASH_METHOD,
            EVM_BLOCK_BY_NUMBER_METHOD,
            EVM_BLOCK_BY_NUMBER_METHOD,
        ]
    );

    let refs = descriptors
        .iter()
        .map(EvmWalletTargetEntryDescriptor::content_ref)
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .expect("refs");
    assert_eq!(refs.len(), descriptors.len());
    for descriptor in descriptors {
        let decoded =
            EvmWalletTargetEntryDescriptor::strict_decode(descriptor.canonical()).expect("decode");
        assert_eq!(decoded, descriptor);
        assert_eq!(decoded.candidate(&request).expect("candidate"), candidate);
        let retained = decoded.canonical().as_str();
        for forbidden in [
            "http://",
            "https://",
            "authorization",
            "private_key",
            "signature",
            "signed_bytes",
            "provider_body",
        ] {
            assert!(!retained.contains(forbidden), "{forbidden}");
        }
    }
}

#[test]
fn already_known_classifier_uses_the_exact_live_decoder_contract() {
    let classifier: serde_json::Value =
        serde_json::from_slice(evm_already_known_classifier_canonical().unwrap().as_bytes())
            .expect("classifier JSON");
    assert_eq!(
        classifier["exact_error"]["code"].as_i64(),
        Some(EXACT_ALREADY_KNOWN_CODE)
    );
    assert_eq!(
        classifier["exact_error"]["message"].as_str(),
        Some(EXACT_ALREADY_KNOWN_MESSAGE)
    );
    assert_eq!(classifier["exact_error"]["other_fields"], false);
    assert_eq!(classifier["provider_text_persisted"], false);
}

#[test]
fn descriptor_decode_rejects_method_input_and_nonce_substitution() {
    let request = request();
    let candidate = candidate(&request);
    let descriptor =
        EvmWalletTargetEntryDescriptor::broadcast(&request, &candidate).expect("descriptor");
    let mut wire: serde_json::Value =
        serde_json::from_slice(descriptor.canonical().as_bytes()).expect("wire");
    wire["rpc_method"] = serde_json::Value::String("eth_getTransactionReceipt".to_owned());
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).expect("tampered canonical");
    let value = SchemaQualifiedCanonicalValue::new(
        descriptor.canonical().schema_id().clone(),
        canonical.as_bytes(),
    )
    .expect("tampered value");
    assert_eq!(
        EvmWalletTargetEntryDescriptor::strict_decode(&value),
        Err(EvmWalletLiveError::InvalidContract)
    );

    let mut wire: serde_json::Value =
        serde_json::from_slice(descriptor.canonical().as_bytes()).expect("wire");
    wire["allocated_nonce"] = serde_json::Value::String("007".to_owned());
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).expect("tampered canonical");
    let value = SchemaQualifiedCanonicalValue::new(
        descriptor.canonical().schema_id().clone(),
        canonical.as_bytes(),
    )
    .expect("tampered value");
    assert_eq!(
        EvmWalletTargetEntryDescriptor::strict_decode(&value),
        Err(EvmWalletLiveError::InvalidContract)
    );
}
