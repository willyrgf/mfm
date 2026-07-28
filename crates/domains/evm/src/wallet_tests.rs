use alloy_primitives::{address, b256, TxKind, B256, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{DigestAlgorithm, SchemaId, TenantScopeId};
use mfm_program::{boundary_content_ref, encode_boundary};

use super::*;

const SENDER: Address = address!("1111111111111111111111111111111111111111");
const RECIPIENT: Address = address!("2222222222222222222222222222222222222222");

fn reference(label: &str) -> EvmWalletReference {
    let schema = SchemaId::new(
        "mfm.test.wallet-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"schema:mfm.test.wallet-reference:1"),
    )
    .expect("schema");
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&serde_json::json!({"label": label}).to_string())
            .expect("canonical");
    EvmWalletReference::from_content_ref(
        boundary_content_ref(schema, &canonical).expect("content ref"),
    )
}

fn fee_schedule() -> EvmWalletReplacementPolicy {
    EvmWalletReplacementPolicy::new(vec![
        EvmWalletFeeCandidate::new(U256::from(20), U256::from(2)).expect("fee"),
        EvmWalletFeeCandidate::new(U256::from(30), U256::from(3)).expect("fee"),
    ])
    .expect("schedule")
}

fn policy() -> EvmWalletPolicy {
    EvmWalletPolicy::new(
        reference("wallet-domain"),
        TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000001").expect("tenant"),
        reference("route-generation"),
        1,
        SENDER,
        reference("signer-binding"),
        7,
        reference("nonce-attestation"),
        fee_schedule(),
        reference("already-known"),
        reference("finality-policy"),
        reference("assurance-policy"),
        EvmWalletConvergencePlan::new(1, 1, 1, 1, 1, 8 * 1024).expect("plan"),
        EvidenceBounds::new(8, 32, 4 * 1024 * 1024, 2, 8 * 1024).expect("bounds"),
    )
    .expect("policy")
}

fn request(action: EvmWalletTransactionAction, input: &[u8]) -> EvmSubmitTransactionRequest {
    let template = EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("primary").expect("target"),
        action,
        U256::from(5),
        input,
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
    let policy = policy();
    EvmSubmitTransactionRequest::new(
        wallet_value_reference(&template).expect("template ref"),
        template,
        wallet_value_reference(&policy).expect("policy ref"),
        policy,
    )
    .expect("request")
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

fn terminal(
    request: &EvmSubmitTransactionRequest,
    status: EvmWalletReceiptStatus,
) -> EvmWalletTerminalEvidence {
    let candidate = candidate(request);
    let block = EvmBlockAnchor::new(
        U256::from(100),
        b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
    );
    let placement =
        EvmWalletTransactionPlacement::new(block.clone(), U256::from(2)).expect("placement");
    let fee = candidate.fee().expect("fee");
    let transaction = EvmWalletObservedTransaction::new(
        candidate.transaction_hash_value().expect("hash"),
        U256::from(request.policy().chain_id()),
        U256::from(candidate.allocated_nonce().expect("nonce")),
        SENDER,
        TxKind::Call(RECIPIENT),
        request.template().value_quantity().expect("value"),
        request.template().input_bytes().expect("input"),
        request.template().gas_limit_quantity().expect("gas"),
        fee.max_fee_quantity().expect("max fee"),
        fee.max_priority_fee_quantity().expect("priority"),
        request.template().access_list().to_vec(),
        Some(placement),
    )
    .expect("transaction");
    let receipt = EvmWalletReceipt::new(
        candidate.transaction_hash_value().expect("hash"),
        U256::from(2),
        block.clone(),
        SENDER,
        Some(RECIPIENT),
        None,
        status,
        U256::from(42_000),
        U256::from(42_000),
        Vec::new(),
    )
    .expect("receipt");
    let finalized = EvmBlockAnchor::new(
        U256::from(101),
        b256!("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"),
    );
    EvmWalletTerminalEvidence::new(
        request.clone(),
        vec![reference("attempt-result")],
        vec![candidate.clone()],
        candidate,
        transaction,
        receipt,
        finalized,
        block,
        reference("executor-generation"),
        reference("generation-fence"),
        request.policy().assurance_policy_ref().clone(),
    )
    .expect("terminal")
}

#[test]
fn selector_is_only_the_configured_target_value() {
    let selector =
        EvmSubmitTransactionSelector::new(EvmTransactionTarget::new("primary").expect("target"));
    assert_eq!(
        serde_json::to_value(selector).expect("selector"),
        serde_json::json!({"target": "primary"})
    );
}

#[test]
fn replacement_schedule_is_finite_and_strict_in_both_fee_dimensions() {
    let equal_priority = EvmWalletReplacementPolicy::new(vec![
        EvmWalletFeeCandidate::new(U256::from(20), U256::from(2)).expect("fee"),
        EvmWalletFeeCandidate::new(U256::from(30), U256::from(2)).expect("fee"),
    ]);
    assert_eq!(
        equal_priority,
        Err(EvmWalletError::InvalidReplacementSchedule)
    );

    let descending_max = EvmWalletReplacementPolicy::new(vec![
        EvmWalletFeeCandidate::new(U256::from(20), U256::from(2)).expect("fee"),
        EvmWalletFeeCandidate::new(U256::from(19), U256::from(3)).expect("fee"),
    ]);
    assert_eq!(
        descending_max,
        Err(EvmWalletError::InvalidReplacementSchedule)
    );
}

#[test]
fn request_is_float_free_and_contains_no_runtime_or_secret_surfaces() {
    let request = request(EvmWalletTransactionAction::call(RECIPIENT), &[0xde, 0xad]);
    let canonical = encode_boundary(&request).expect("canonical");
    let rendered = canonical.as_str();
    for forbidden in [
        "http://",
        "https://",
        "authorization",
        "private_key",
        "password",
        "signed_transaction",
        "signature",
        "pending_nonce",
    ] {
        assert!(!rendered.contains(forbidden));
    }
    assert!(!rendered.contains(".0"));
    assert_eq!(
        EvmSubmitTransactionRequest::strict_decode(canonical.as_bytes()).expect("decode"),
        request
    );
}

#[test]
fn candidate_reconstruction_fixes_every_non_fee_field() {
    let request = request(EvmWalletTransactionAction::call(RECIPIENT), &[1, 2, 3]);
    let candidate = candidate(&request);
    let envelope = candidate.unsigned_envelope().expect("envelope");
    assert_eq!(envelope.chain_id(), U256::from(1));
    assert_eq!(envelope.nonce(), U256::from(7));
    assert_eq!(envelope.to(), TxKind::Call(RECIPIENT));
    assert_eq!(envelope.value(), U256::from(5));
    assert_eq!(envelope.input().as_ref(), &[1, 2, 3]);
    assert_eq!(envelope.gas_limit(), U256::from(75_000));
}

#[test]
fn exact_broadcast_classifier_relation_is_purely_enforced() {
    let request = request(EvmWalletTransactionAction::call(RECIPIENT), &[]);
    let candidate = candidate(&request);
    let accepted = EvmWalletAttemptResult::Broadcast {
        candidate_ref: candidate.reference().expect("candidate ref"),
        transaction_hash: candidate.transaction_hash().to_owned(),
        status: EvmWalletBroadcastStatus::Accepted,
        classifier_ref: None,
    };
    accepted
        .validate_for_candidate(&candidate)
        .expect("accepted");

    let wrong_classifier = EvmWalletAttemptResult::Broadcast {
        candidate_ref: candidate.reference().expect("candidate ref"),
        transaction_hash: candidate.transaction_hash().to_owned(),
        status: EvmWalletBroadcastStatus::AlreadyKnown,
        classifier_ref: Some(reference("wrong-classifier")),
    };
    assert_eq!(
        wrong_classifier.validate_for_candidate(&candidate),
        Err(EvmWalletError::InconsistentEvidence)
    );
}

#[test]
fn success_and_revert_are_both_terminal_outputs() {
    let request = request(EvmWalletTransactionAction::call(RECIPIENT), &[]);
    assert!(matches!(
        terminal(&request, EvmWalletReceiptStatus::Success)
            .outcome()
            .expect("success"),
        EvmTransactionOutcome::Succeeded { .. }
    ));
    assert!(matches!(
        terminal(&request, EvmWalletReceiptStatus::Reverted)
            .outcome()
            .expect("revert"),
        EvmTransactionOutcome::Reverted { .. }
    ));
}

#[test]
fn receipt_disappearance_and_reorg_remain_nonterminal() {
    let request = request(EvmWalletTransactionAction::call(RECIPIENT), &[]);
    let candidate = candidate(&request);
    let disappeared = EvmWalletAttemptResult::ReceiptLookup {
        transaction_hash: candidate.transaction_hash().to_owned(),
        receipt: None,
    };
    assert_eq!(disappeared.terminal_outcome().expect("valid"), None);

    let terminal = terminal(&request, EvmWalletReceiptStatus::Success);
    let different = EvmBlockAnchor::new(U256::from(100), B256::from([0xee; 32]));
    let reorg = EvmWalletAttemptResult::CanonicalInclusion {
        block: Some(different),
        terminal: Some(terminal),
    };
    assert_eq!(reorg.validate(), Err(EvmWalletError::InconsistentEvidence));
}

#[test]
fn convergence_budget_must_fit_executor_bounds_at_construction() {
    let result = EvmWalletPolicy::new(
        reference("wallet-domain"),
        TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000001").expect("tenant"),
        reference("route"),
        1,
        SENDER,
        reference("signer"),
        0,
        reference("nonce"),
        fee_schedule(),
        reference("known"),
        reference("finality"),
        reference("assurance"),
        EvmWalletConvergencePlan::new(1, 1, 1, 1, 1, 4096).expect("plan"),
        EvidenceBounds::new(4, 32, 1024 * 1024, 2, 4096).expect("bounds"),
    );
    assert_eq!(
        result,
        Err(EvmWalletError::BoundExceeded("evidence_attempts"))
    );
}
