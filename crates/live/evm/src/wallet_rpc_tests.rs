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
        7,
        reference("nonce-attestation"),
        replacement,
        evm_already_known_classifier_ref().expect("classifier"),
        reference("finality-policy"),
        reference("assurance-policy"),
        EvmWalletConvergencePlan::new(2, 2, 2, 2, 2, 16 * 1024).expect("plan"),
        EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 2, 16 * 1024).expect("bounds"),
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

#[test]
fn already_known_classifier_is_exact_and_shape_closed() {
    assert!(EvmWalletRpcError::new(
        EXACT_ALREADY_KNOWN_CODE,
        EXACT_ALREADY_KNOWN_MESSAGE.to_owned(),
        true,
    )
    .is_exact_already_known());
    for error in [
        EvmWalletRpcError::new(EXACT_ALREADY_KNOWN_CODE, "already Known".to_owned(), true),
        EvmWalletRpcError::new(
            EXACT_ALREADY_KNOWN_CODE - 1,
            EXACT_ALREADY_KNOWN_MESSAGE.to_owned(),
            true,
        ),
        EvmWalletRpcError::new(
            EXACT_ALREADY_KNOWN_CODE,
            EXACT_ALREADY_KNOWN_MESSAGE.to_owned(),
            false,
        ),
    ] {
        assert!(!error.is_exact_already_known());
    }
}

#[test]
fn strict_transaction_parser_rejects_noncanonical_or_candidate_mismatched_fields() {
    let request = request();
    let candidate = candidate(&request);
    let valid = serde_json::json!({
        "accessList": [{
            "address": format!("{RECIPIENT:#x}"),
            "storageKeys": [
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ],
        }],
        "blockHash": null,
        "blockNumber": null,
        "chainId": "0x1",
        "from": format!("{SENDER:#x}"),
        "gas": "0x124f8",
        "hash": candidate.transaction_hash(),
        "input": "0xdead",
        "maxFeePerGas": "0x14",
        "maxPriorityFeePerGas": "0x2",
        "nonce": "0x7",
        "to": format!("{RECIPIENT:#x}"),
        "transactionIndex": null,
        "type": "0x2",
        "value": "0x5",
    });
    let parsed = parse_transaction(&valid).expect("strict transaction");
    assert!(parsed
        .matches_candidate(&candidate)
        .expect("candidate match"));

    let mut noncanonical = valid.clone();
    noncanonical["nonce"] = serde_json::Value::String("0x07".to_owned());
    assert_eq!(parse_transaction(&noncanonical), Err(()));

    let mut wrong_sender = valid;
    wrong_sender["from"] = serde_json::Value::String(format!(
        "{:#x}",
        address!("3333333333333333333333333333333333333333")
    ));
    let parsed = parse_transaction(&wrong_sender).expect("schema-valid transaction");
    assert!(!parsed
        .matches_candidate(&candidate)
        .expect("candidate mismatch"));
}

#[test]
fn receipt_parser_rejects_cross_block_log_splicing() {
    let transaction_hash = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let block_hash = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let mut receipt = serde_json::json!({
        "blockHash": block_hash,
        "blockNumber": "0x64",
        "contractAddress": null,
        "cumulativeGasUsed": "0xa410",
        "from": format!("{SENDER:#x}"),
        "gasUsed": "0xa410",
        "logs": [{
            "address": format!("{RECIPIENT:#x}"),
            "blockHash": block_hash,
            "blockNumber": "0x64",
            "data": "0x",
            "logIndex": "0x0",
            "removed": false,
            "topics": [],
            "transactionHash": transaction_hash,
            "transactionIndex": "0x2",
        }],
        "status": "0x1",
        "to": format!("{RECIPIENT:#x}"),
        "transactionHash": transaction_hash,
        "transactionIndex": "0x2",
        "type": "0x2",
    });
    parse_receipt(&receipt).expect("coherent receipt");
    receipt["logs"][0]["blockHash"] = serde_json::Value::String(
        "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_owned(),
    );
    assert_eq!(parse_receipt(&receipt), Err(()));
}
