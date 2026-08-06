use super::*;

use alloy_primitives::{Address, B256, U256};
use mfm_evm::{
    canonical_wallet_reference, derive_authenticated_intent_issuer_id,
    derive_evm_candidate_operation_key, derive_evm_chain_lineage_id,
    derive_evm_nonce_completion_key, derive_evm_nonce_reservation_key,
    derive_exact_candidate_activation_permit, derive_submission_intent_id,
    derive_submission_semantics_digest, derive_wallet_nonce_domain,
    evm_submission_expansion_policy_ref, evm_wallet_assurance_policy_ref,
    evm_wallet_nonce_policy_ref, ActivateEvmCandidateRequest, ActiveWalletCandidate,
    AttestedWalletCandidate, CanonicalTerminalOutcome, ChainInstanceDeclaration,
    ChainInstanceRegistryAttestation, CompleteEvmNonceRequest, CompletedWalletNonce,
    EvmCandidateFamily, EvmFinalizedHeadObservation, EvmInclusionBlockObservation,
    EvmReceiptLookupObservation, EvmRoutingGenerationRef, EvmTransactionIntent,
    EvmTransactionLookupObservation, EvmTransactionTarget, EvmWalletFeeCandidate,
    EvmWalletTransactionAction, EvmWalletTransactionTemplate, ExclusiveCurrentControl,
    ExecutionDisposition, ObservedPendingNonceFloor, PriorEffectDisposition,
    PriorResourceDisposition, QualifiedPendingNonceFloor, ReplayExclusionDisposition,
    ReserveEvmNonceRequest, ReservedWalletNonce, TerminalWitnesses, TransactionNonce,
    UnsignedWalletCandidate, WalletNonceDomainActivationAttestation,
    WalletNonceDomainActivationRecord, WalletNonceStoreIncarnation,
};
use mfm_ids::{StableId, TenantScopeId};
use mfm_journal::structured::{domain_content_digest, LexicalValueRef, TypedValueRef};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};

const SIGNING_SEED: [u8; 32] = [0x42; 32];
const WRONG_SIGNING_SEED: [u8; 32] = [0x24; 32];

struct CompletionProofFixture {
    mutation: ProviderMutation,
    completion: CompletedWalletNonce,
    state_input: LexicalValueRef,
    context: ProviderTargetContext,
    operation_key: String,
    provider_id: String,
}

fn completion_fixture() -> CompletionProofFixture {
    let common_ref = evm_wallet_nonce_policy_ref().expect("nonce policy reference");
    let declaration = ChainInstanceDeclaration::new(
        common_ref.clone(),
        StableId::new("mfm.evm.fixture/chain-instance").expect("chain instance id"),
        1,
        B256::repeat_byte(0x11),
        U256::from(10_u64),
        B256::repeat_byte(0x12),
    )
    .expect("chain declaration");
    let chain_attestation =
        ChainInstanceRegistryAttestation::new(declaration, common_ref.clone(), common_ref.clone())
            .expect("chain attestation");
    let chain_binding = chain_attestation.binding().expect("chain binding");
    let chain_lineage = derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())
        .expect("chain lineage");
    let sender = Address::repeat_byte(0x21);
    let nonce_domain = derive_wallet_nonce_domain(chain_lineage, sender).expect("nonce domain");
    let route_generation = EvmRoutingGenerationRef::from_content_ref(
        common_ref
            .to_content_ref()
            .expect("route content reference"),
    )
    .expect("route generation");
    let sender_inventory = domain_content_digest(
        "mfm.evm.fixture-sender-path-inventory.v1",
        &(nonce_domain.clone(), sender),
    )
    .expect("sender inventory digest");
    let activation_record = WalletNonceDomainActivationRecord {
            activation_contract_ref: common_ref.clone(),
            qualified_activation_registry_lineage_ref: common_ref.clone(),
            wallet_nonce_store_lineage_id: "mfm.evm.fixture/wallet-store".to_owned(),
            initial_store_incarnation_ref: common_ref.clone(),
            wallet_nonce_domain: nonce_domain.clone(),
            chain_instance_attestation: chain_attestation,
            initial_route_generation_ref: route_generation,
            initial_route_membership_issuance_ref: common_ref.clone(),
            sender_identity: format!("{sender:#x}"),
            issuer_namespace_contract_ref: common_ref.clone(),
            replay_exclusion_contract_ref: common_ref.clone(),
            replay_exclusion_disposition:
                ReplayExclusionDisposition::EveryPriorRequestReplayAndRetryIngressExcluded,
            finalized_sender_nonce_floor: 0,
            finalized_block_number: "10".to_owned(),
            finalized_block_hash: format!("{:#x}", B256::repeat_byte(0x12)),
            qualified_observation_proof_ref: common_ref.clone(),
            exhaustive_sender_path_inventory_digest: sender_inventory.as_str().to_owned(),
            exclusive_current_control:
                ExclusiveCurrentControl::EveryPriorWriterSignerRelayerOperatorStaleDeploymentAndDirectSubmitPathFenced,
            prior_effect_disposition: PriorEffectDisposition::NoUnresolvedPossibleEntry,
            prior_resource_disposition:
                PriorResourceDisposition::EveryPriorAllocationAndSubmittedCandidateTerminal,
            new_idempotency_epoch: "mfm.evm.fixture/idempotency-epoch".to_owned(),
        };
    activation_record.validate().expect("activation record");
    let activation_record_ref =
        canonical_wallet_reference(&activation_record).expect("activation record reference");
    let activation = WalletNonceDomainActivationAttestation {
        activation_record_ref,
        registry_issuance_ref: common_ref.clone(),
        activation_registry_lineage_ref: common_ref.clone(),
        initial_store_incarnation_ref: common_ref.clone(),
        current_schema_record: activation_record,
    };
    activation.validate().expect("activation attestation");

    let assurance_ref = evm_wallet_assurance_policy_ref().expect("assurance policy");
    let template = EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("mutation-proof-target").expect("transaction target"),
        EvmWalletTransactionAction::call(Address::repeat_byte(0x31)),
        U256::ZERO,
        [],
        Vec::new(),
        U256::from(21_000_u64),
    )
    .expect("transaction template");
    let intent = EvmTransactionIntent::new(
        chain_binding,
        nonce_domain.clone(),
        template,
        StableId::new("mfm.evm.fixture/semantic-signer").expect("signer id"),
        common_ref.clone(),
        common_ref.clone(),
        assurance_ref,
    )
    .expect("transaction intent");
    let candidate_family = EvmCandidateFamily::new(
        &intent,
        vec![
            EvmWalletFeeCandidate::new(U256::from(100_u64), U256::from(2_u64))
                .expect("candidate fee"),
        ],
    )
    .expect("candidate family");
    let tenant = TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000051")
        .expect("tenant scope");
    let principal = StableId::new("mfm.evm.fixture/principal").expect("principal");
    let issuer = derive_authenticated_intent_issuer_id(&tenant, &principal, &common_ref)
        .expect("intent issuer");
    let submission_intent_id =
        derive_submission_intent_id(&nonce_domain, &issuer, "mutation-proof-token")
            .expect("submission intent");
    let route_generation_ref = common_ref.clone();
    let expansion_ref = evm_submission_expansion_policy_ref().expect("expansion policy");
    let submission_semantics_digest = derive_submission_semantics_digest(
        &intent,
        &candidate_family,
        1,
        &expansion_ref,
        &route_generation_ref,
        &activation,
        &common_ref,
    )
    .expect("submission semantics");
    let reservation_key = derive_evm_nonce_reservation_key(&nonce_domain, &submission_intent_id)
        .expect("reservation key");
    let qualified_floor = QualifiedPendingNonceFloor {
        observed: ObservedPendingNonceFloor {
            nonce_domain: nonce_domain.clone(),
            route_generation_ref: route_generation_ref.clone(),
            pending_nonce: TransactionNonce::new(0).expect("pending nonce"),
        },
        pending_floor_policy_ref: common_ref.clone(),
    };
    let content_ref = common_ref
        .to_content_ref()
        .expect("state content reference");
    let state_input = LexicalValueRef::new(
        content_ref.clone(),
        TypedValueRef {
            contract_ref: content_ref.clone(),
            value_ref: content_ref,
        },
    );
    let reserve_request = ReserveEvmNonceRequest {
        nonce_domain: nonce_domain.clone(),
        domain_activation_attestation: activation.clone(),
        issuer_namespace_contract_ref: common_ref.clone(),
        submission_intent_id: submission_intent_id.clone(),
        submission_semantics_digest: submission_semantics_digest.clone(),
        transaction_intent: intent.clone(),
        candidate_family: candidate_family.clone(),
        observation_rounds: 1,
        reservation_key: reservation_key.clone(),
        qualified_floor: qualified_floor.clone(),
    };
    let observed_floor_ref = domain_content_digest(
        "mfm.evm.wallet-observed-floor-provenance.v1",
        &(&reserve_request.qualified_floor, &state_input),
    )
    .expect("observed-floor digest")
    .as_str()
    .to_owned();
    let mut reservation = ReservedWalletNonce {
        nonce_domain: nonce_domain.clone(),
        domain_activation_record_ref: activation.activation_record_ref.clone(),
        nonce: 0,
        semantic_reservation_key: reservation_key.clone(),
        submission_intent_id: submission_intent_id.clone(),
        transaction_intent_digest: intent.digest().to_owned(),
        candidate_family_ref: candidate_family.digest().to_owned(),
        observed_floor_ref,
        resource_lineage_ref: common_ref.clone(),
        reservation_evidence_ref: common_ref.clone(),
    };
    let reservation_evidence = domain_content_digest(
        "mfm.evm.wallet-reservation-evidence.v1",
        &(
            &reserve_request,
            &state_input,
            &reservation.resource_lineage_ref,
            reservation.nonce,
        ),
    )
    .expect("reservation evidence digest");
    reservation.reservation_evidence_ref = common_ref
        .with_content_digest(reservation_evidence)
        .expect("reservation evidence reference");

    let unsigned_envelope = intent
        .unsigned_candidate(reservation.nonce, &candidate_family.candidates()[0])
        .expect("unsigned candidate");
    let unsigned = UnsignedWalletCandidate {
        transaction_intent: intent.clone(),
        semantic_reservation_key: reservation.semantic_reservation_key.clone(),
        nonce: reservation.nonce,
        candidate_ordinal: 0,
        fee: candidate_family.candidates()[0].clone(),
        unsigned_candidate_digest: format!("{:#x}", unsigned_envelope.signing_digest()),
    };
    let attested = AttestedWalletCandidate {
        semantic_reservation_key: reservation.semantic_reservation_key.clone(),
        candidate_ordinal: 0,
        candidate_descriptor_ref: canonical_wallet_reference(&unsigned)
            .expect("candidate descriptor"),
        unsigned_candidate_digest: unsigned.unsigned_candidate_digest.clone(),
        transaction_hash: format!("{:#x}", B256::repeat_byte(0x41)),
        semantic_signer_id: intent.semantic_signer_id().to_owned(),
        signing_profile_contract_ref: common_ref.clone(),
        signer_attestation_ref: common_ref.clone(),
    };
    let activation_request = ActivateEvmCandidateRequest {
        nonce_domain: nonce_domain.clone(),
        candidate_operation_key: derive_evm_candidate_operation_key(
            &reservation.semantic_reservation_key,
            0,
        )
        .expect("candidate operation key"),
        next_candidate: attested.clone(),
        activation_permit: derive_exact_candidate_activation_permit(&reservation, &[], 0, 0)
            .expect("initial activation permit"),
    };
    let activation_evidence = domain_content_digest(
        "mfm.evm.wallet-candidate-activation-evidence.v1",
        &(
            &activation_request,
            &state_input,
            &reservation.resource_lineage_ref,
        ),
    )
    .expect("activation evidence digest");
    let activation_evidence_ref = common_ref
        .with_content_digest(activation_evidence)
        .expect("activation evidence reference");
    let active_candidate = ActiveWalletCandidate {
        attested_candidate: attested,
        activation_evidence_ref: activation_evidence_ref.clone(),
        provider_activation_attestation: "fixture-provider-attestation".to_owned(),
    };
    let transaction_hash = active_candidate.attested_candidate.transaction_hash.clone();
    let inclusion_block_hash = format!("{:#x}", B256::repeat_byte(0x42));
    let canonical_public_result = "{\"execution_disposition\":\"succeeded\"}".to_owned();
    let canonical_terminal_outcome = CanonicalTerminalOutcome {
        nonce_domain: nonce_domain.clone(),
        semantic_reservation_key: reservation.semantic_reservation_key.clone(),
        submission_intent_id: reservation.submission_intent_id.clone(),
        transaction_intent_digest: reservation.transaction_intent_digest.clone(),
        nonce: reservation.nonce,
        winning_candidate_ordinal: 0,
        winning_activation_evidence_ref: activation_evidence_ref,
        transaction_hash: transaction_hash.clone(),
        inclusion_block_number: "10".to_owned(),
        inclusion_block_hash: inclusion_block_hash.clone(),
        terminal_assurance_contract_ref: evm_wallet_assurance_policy_ref()
            .expect("assurance policy"),
        execution_disposition: ExecutionDisposition::Succeeded,
        canonical_public_result: canonical_public_result.clone(),
    };
    let terminal_witnesses = TerminalWitnesses {
        transaction: EvmTransactionLookupObservation::Found {
            transaction_hash: transaction_hash.clone(),
            block_number: Some("10".to_owned()),
            block_hash: Some(inclusion_block_hash.clone()),
        },
        receipt: EvmReceiptLookupObservation::Found {
            transaction_hash: transaction_hash.clone(),
            block_number: "10".to_owned(),
            block_hash: inclusion_block_hash.clone(),
            status: 1,
        },
        finalized_head: EvmFinalizedHeadObservation {
            block_number: "11".to_owned(),
            block_hash: format!("{:#x}", B256::repeat_byte(0x43)),
        },
        inclusion_block: EvmInclusionBlockObservation {
            block_number: "10".to_owned(),
            block_hash: inclusion_block_hash,
        },
        terminal_assurance_contract_ref: canonical_terminal_outcome
            .terminal_assurance_contract_ref
            .clone(),
        canonical_public_result: canonical_public_result.clone(),
    };
    terminal_witnesses
        .validate_against(&canonical_terminal_outcome)
        .expect("terminal witnesses");
    let completion_request = CompleteEvmNonceRequest {
        nonce_domain: nonce_domain.clone(),
        completion_key: derive_evm_nonce_completion_key(&reservation.semantic_reservation_key)
            .expect("completion key"),
        current_reservation: reservation.clone(),
        canonical_terminal_outcome,
        terminal_witnesses,
    };
    let completion_evidence = domain_content_digest(
        "mfm.evm.wallet-completion-evidence.v1",
        &(
            &completion_request,
            &state_input,
            &reservation.resource_lineage_ref,
        ),
    )
    .expect("completion evidence digest");
    let completion_evidence_ref = common_ref
        .with_content_digest(completion_evidence)
        .expect("completion evidence reference");
    let completion = CompletedWalletNonce::with_recovery_closure(
        completion_request.nonce_domain.clone(),
        completion_request.current_reservation.nonce,
        completion_request
            .current_reservation
            .semantic_reservation_key
            .clone(),
        completion_request.completion_key.clone(),
        completion_request.canonical_terminal_outcome.clone(),
        completion_request.terminal_witnesses.clone(),
        vec![active_candidate],
        canonical_wallet_reference(&completion_request.terminal_witnesses)
            .expect("terminal witness reference")
            .content_digest()
            .to_owned(),
        completion_evidence_ref,
        "fixture-provider-attestation".to_owned(),
        reservation,
        intent,
        candidate_family,
        activation,
        qualified_floor,
        route_generation_ref,
        common_ref.clone(),
        1,
        submission_semantics_digest,
        reserve_request,
        completion_request.clone(),
        vec![activation_request],
        vec![state_input.clone()],
        state_input.clone(),
        state_input.clone(),
    )
    .expect("completion recovery closure");
    completion.validate().expect("completion validation");
    let completion_preimage = completion
        .provider_mutation_preimage()
        .expect("completion provider preimage");
    let mutation = ProviderMutation::Completion {
        request: completion_request.clone(),
        completion: completion_preimage,
        state_input_ref: state_input.clone(),
    };
    let store_incarnation = WalletNonceStoreIncarnation {
        wallet_nonce_store_lineage_id: "mfm.evm.fixture/wallet-store".to_owned(),
        writer_epoch: 1,
        physical_target_instance_id: "mfm.evm.fixture/target".to_owned(),
        non_exportable_target_public_key_ref: common_ref.clone(),
        target_attestation_contract_ref: common_ref,
    };
    store_incarnation.validate().expect("store incarnation");
    let context = ProviderTargetContext {
        database_oid: 17,
        backend_pid: 23,
        transaction_id: Some(29),
        snapshot_id: None,
        schema_name: "fixture_schema".to_owned(),
        application_marker: "a".repeat(62),
        store_incarnation,
    };
    let operation_key = completion_request.completion_key.as_str().to_owned();
    CompletionProofFixture {
        mutation,
        completion,
        state_input,
        context,
        operation_key,
        provider_id: "mfm.evm.fixture/provider".to_owned(),
    }
}

fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    Ed25519KeyPair::from_seed_unchecked(seed)
        .expect("deterministic signing seed")
        .public_key()
        .as_ref()
        .try_into()
        .expect("Ed25519 public key length")
}

fn persisted_proof(
    context: &ProviderTargetContext,
    operation_key: &str,
    provider_id: &str,
    mutation: &ProviderMutation,
    seed: &[u8; 32],
) -> String {
    let payload_digest = domain_content_digest(
        "mfm.wallet-authority-provider.assertion-payload.v1",
        &(context, operation_key, mutation),
    )
    .expect("provider payload digest");
    let challenge = [0x33_u8; 32];
    let mut signed = Vec::with_capacity(
        ASSERTION_DOMAIN.len()
            + challenge.len()
            + "prepare-mutation".len()
            + payload_digest.as_str().len(),
    );
    signed.extend_from_slice(ASSERTION_DOMAIN);
    signed.extend_from_slice(&challenge);
    signed.extend_from_slice(b"prepare-mutation");
    signed.extend_from_slice(payload_digest.as_str().as_bytes());
    let signature = Ed25519KeyPair::from_seed_unchecked(seed)
        .expect("deterministic signing seed")
        .sign(&signed);
    serde_json::to_string(&PersistedMutationProof {
        provider_id: provider_id.to_owned(),
        challenge: hex::encode(challenge),
        context: context.clone(),
        operation_key: operation_key.to_owned(),
        payload_digest: payload_digest.as_str().to_owned(),
        signature: hex::encode(signature.as_ref()),
    })
    .expect("canonical persisted proof")
}

fn persisted_completion_fixture() -> CompletionProofFixture {
    let mut fixture = completion_fixture();
    let proof = persisted_proof(
        &fixture.context,
        &fixture.operation_key,
        &fixture.provider_id,
        &fixture.mutation,
        &SIGNING_SEED,
    );
    fixture.completion = fixture
        .completion
        .with_provider_completion_attestation(proof)
        .expect("persist provider proof in completion closure");
    fixture
}

fn mutation_from_completion_closure(completion: &CompletedWalletNonce) -> ProviderMutation {
    let closure: serde_json::Value =
        serde_json::from_str(&completion.recovery_closure).expect("canonical closure JSON");
    let request: CompleteEvmNonceRequest = serde_json::from_value(
        closure
            .get("completion_request")
            .expect("completion request in closure")
            .clone(),
    )
    .expect("typed completion request in closure");
    let state_input_ref: LexicalValueRef = serde_json::from_value(
        closure
            .get("completion_state_input")
            .expect("completion state input in closure")
            .clone(),
    )
    .expect("typed completion state input in closure");
    ProviderMutation::Completion {
        request,
        completion: completion
            .provider_mutation_preimage()
            .expect("closure-derived completion preimage"),
        state_input_ref,
    }
}

/// Independent wire-shaped view used by the detached audit. This deliberately does not call the
/// storage verifier: it reconstructs the provider mutation from the persisted closure and hashes
/// that local representation before checking the provider signature.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DetachedAuditMutation {
    Completion {
        request: CompleteEvmNonceRequest,
        completion: CompletedWalletNonce,
        state_input_ref: LexicalValueRef,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DetachedAuditContext {
    database_oid: u32,
    backend_pid: i32,
    transaction_id: Option<u32>,
    snapshot_id: Option<String>,
    schema_name: String,
    application_marker: String,
    store_incarnation: WalletNonceStoreIncarnation,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DetachedAuditProof {
    provider_id: String,
    challenge: String,
    context: DetachedAuditContext,
    operation_key: String,
    payload_digest: String,
    signature: String,
}

struct DetachedAuditTrust {
    provider_id: String,
    public_key: [u8; 32],
    operation_key: String,
    context: DetachedAuditContext,
}

fn detached_audit_context(context: &ProviderTargetContext) -> DetachedAuditContext {
    serde_json::from_value(serde_json::to_value(context).expect("provider context JSON"))
        .expect("detached provider context")
}

fn detached_audit_mutation_from_closure(
    recovery_closure: &str,
) -> std::result::Result<DetachedAuditMutation, &'static str> {
    let closure: serde_json::Value =
        serde_json::from_str(recovery_closure).map_err(|_| "closure")?;
    let request: CompleteEvmNonceRequest = serde_json::from_value(
        closure
            .get("completion_request")
            .ok_or("completion request")?
            .clone(),
    )
    .map_err(|_| "completion request")?;
    let state_input_ref: LexicalValueRef = serde_json::from_value(
        closure
            .get("completion_state_input")
            .ok_or("completion state input")?
            .clone(),
    )
    .map_err(|_| "completion state input")?;
    let completion = CompletedWalletNonce::from_recovery_closure(recovery_closure)
        .map_err(|_| "completion closure")?
        .provider_mutation_preimage()
        .map_err(|_| "completion preimage")?;
    Ok(DetachedAuditMutation::Completion {
        request,
        completion,
        state_input_ref,
    })
}

fn independently_verify_detached_completion(
    recovery_closure: &str,
    proof_value: &str,
    trust: &DetachedAuditTrust,
) -> std::result::Result<String, &'static str> {
    const AUDIT_ASSERTION_DOMAIN: &[u8] = b"mfm.wallet-authority-provider.assertion.v1\0";

    if proof_value.is_empty()
        || proof_value.len() > MAX_PROVIDER_PROOF_BYTES
        || !proof_value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err("proof bounds");
    }
    let proof: DetachedAuditProof = serde_json::from_str(proof_value).map_err(|_| "proof JSON")?;
    if serde_json::to_string(&proof).map_err(|_| "proof encoding")? != proof_value {
        return Err("proof canonicality");
    }
    if proof.provider_id != trust.provider_id
        || proof.operation_key != trust.operation_key
        || proof.context != trust.context
    {
        return Err("proof binding");
    }
    let context = &proof.context;
    if context.database_oid == 0
        || context.backend_pid <= 0
        || context.transaction_id.is_none()
        || context.snapshot_id.is_some()
        || context.application_marker.len() != 62
        || !context
            .application_marker
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("provider context");
    }
    context
        .store_incarnation
        .validate()
        .map_err(|_| "store incarnation")?;

    let mutation = detached_audit_mutation_from_closure(recovery_closure)?;
    let payload_digest = domain_content_digest(
        "mfm.wallet-authority-provider.assertion-payload.v1",
        &(&proof.context, &proof.operation_key, &mutation),
    )
    .map_err(|_| "payload digest")?;
    if proof.payload_digest != payload_digest.as_str() {
        return Err("payload binding");
    }
    let challenge = hex::decode(&proof.challenge).map_err(|_| "challenge")?;
    if challenge.len() != 32 {
        return Err("challenge length");
    }
    let signature = hex::decode(&proof.signature).map_err(|_| "signature")?;
    if signature.len() != 64 {
        return Err("signature length");
    }
    let mut signed = Vec::with_capacity(
        AUDIT_ASSERTION_DOMAIN.len()
            + challenge.len()
            + "prepare-mutation".len()
            + proof.payload_digest.len(),
    );
    signed.extend_from_slice(AUDIT_ASSERTION_DOMAIN);
    signed.extend_from_slice(&challenge);
    signed.extend_from_slice(b"prepare-mutation");
    signed.extend_from_slice(proof.payload_digest.as_bytes());
    UnparsedPublicKey::new(&ED25519, trust.public_key)
        .verify(&signed, &signature)
        .map_err(|_| "provider signature")?;

    CompletedWalletNonce::project_public_result_from_recovery_closure(recovery_closure)
        .map_err(|_| "public projection")
}

#[test]
fn detached_completion_proof_verifies_and_rehydrates_without_io() {
    let fixture = persisted_completion_fixture();
    let proof = fixture.completion.provider_completion_attestation.clone();
    assert_eq!(fixture.completion.provider_completion_attestation, proof);
    let closure_mutation = mutation_from_completion_closure(&fixture.completion);
    assert!(closure_mutation == fixture.mutation);
    let verified_context = verify_persisted_mutation_proof(
        &public_key(&SIGNING_SEED),
        &fixture.provider_id,
        &proof,
        Some(&fixture.context),
        &fixture.operation_key,
        &closure_mutation,
    )
    .expect("detached completion proof");
    assert_eq!(verified_context, fixture.context);
    assert_eq!(
        fixture.state_input,
        match &fixture.mutation {
            ProviderMutation::Completion {
                state_input_ref, ..
            } => state_input_ref.clone(),
            _ => panic!("completion mutation fixture"),
        }
    );
    let rehydrated =
        CompletedWalletNonce::from_recovery_closure(&fixture.completion.recovery_closure)
            .expect("rehydrate completion closure");
    assert_eq!(rehydrated, fixture.completion);
    assert_eq!(
        CompletedWalletNonce::project_public_result_from_recovery_closure(
            &fixture.completion.recovery_closure,
        )
        .expect("project public completion"),
        fixture
            .completion
            .canonical_terminal_outcome
            .canonical_public_result,
    );
}

#[test]
fn independent_detached_audit_verifies_provider_proof_and_public_projection() {
    let fixture = persisted_completion_fixture();
    let trust = DetachedAuditTrust {
        provider_id: fixture.provider_id.clone(),
        public_key: public_key(&SIGNING_SEED),
        operation_key: fixture.operation_key.clone(),
        context: detached_audit_context(&fixture.context),
    };
    let public_result = independently_verify_detached_completion(
        &fixture.completion.recovery_closure,
        &fixture.completion.provider_completion_attestation,
        &trust,
    )
    .expect("independent detached completion audit");
    assert_eq!(public_result, r#"{"execution_disposition":"succeeded"}"#);
    assert_eq!(
        public_result,
        CompletedWalletNonce::project_public_result_from_recovery_closure(
            &fixture.completion.recovery_closure,
        )
        .expect("closure-only public projection"),
    );
    assert_eq!(
        serde_json::to_value(&fixture.mutation).expect("storage mutation JSON"),
        serde_json::to_value(
            &detached_audit_mutation_from_closure(&fixture.completion.recovery_closure)
                .expect("detached mutation from closure"),
        )
        .expect("detached mutation JSON"),
    );
}

#[test]
fn independent_detached_audit_rejects_provider_projection_operation_and_context_substitutions() {
    let fixture = persisted_completion_fixture();
    let trust = DetachedAuditTrust {
        provider_id: fixture.provider_id.clone(),
        public_key: public_key(&SIGNING_SEED),
        operation_key: fixture.operation_key.clone(),
        context: detached_audit_context(&fixture.context),
    };
    let mut forged_proof: DetachedAuditProof =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid detached proof");
    forged_proof.provider_id = "mfm.evm.fixture/substituted-provider".to_owned();
    let forged_proof = serde_json::to_string(&forged_proof).expect("substituted proof");
    assert!(independently_verify_detached_completion(
        &fixture.completion.recovery_closure,
        &forged_proof,
        &trust,
    )
    .is_err());

    let mut forged_operation: DetachedAuditProof =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid detached proof");
    forged_operation.operation_key = "mfm.evm.fixture/other-operation".to_owned();
    let forged_operation = serde_json::to_string(&forged_operation).expect("operation proof");
    assert!(independently_verify_detached_completion(
        &fixture.completion.recovery_closure,
        &forged_operation,
        &trust,
    )
    .is_err());

    let mut forged_context: DetachedAuditProof =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid detached proof");
    forged_context.context.schema_name = "other_schema".to_owned();
    let forged_context = serde_json::to_string(&forged_context).expect("context proof");
    assert!(independently_verify_detached_completion(
        &fixture.completion.recovery_closure,
        &forged_context,
        &trust,
    )
    .is_err());

    let forged_projection = fixture.completion.recovery_closure.replace(
        r#"\"execution_disposition\":\"succeeded\""#,
        r#"\"execution_disposition\":\"reverted\""#,
    );
    assert!(independently_verify_detached_completion(
        &forged_projection,
        &fixture.completion.provider_completion_attestation,
        &trust,
    )
    .is_err());
    assert!(
        CompletedWalletNonce::project_public_result_from_recovery_closure(&forged_projection)
            .is_err()
    );
}

#[test]
fn independent_detached_audit_rejects_payload_crypto_challenge_and_wire_substitutions() {
    let fixture = persisted_completion_fixture();
    let trust = DetachedAuditTrust {
        provider_id: fixture.provider_id.clone(),
        public_key: public_key(&SIGNING_SEED),
        operation_key: fixture.operation_key.clone(),
        context: detached_audit_context(&fixture.context),
    };

    let mut forged_digest: DetachedAuditProof =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid detached proof");
    forged_digest.payload_digest =
        "sha256v1:0000000000000000000000000000000000000000000000000000000000000000".to_owned();
    let forged_digest = serde_json::to_string(&forged_digest).expect("digest proof");
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &forged_digest,
            &trust,
        ),
        Err("payload binding"),
    );

    let mut forged_signature: DetachedAuditProof =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid detached proof");
    forged_signature.signature.replace_range(0..2, "00");
    let forged_signature = serde_json::to_string(&forged_signature).expect("signature proof");
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &forged_signature,
            &trust,
        ),
        Err("provider signature"),
    );

    let wrong_key_trust = DetachedAuditTrust {
        provider_id: fixture.provider_id.clone(),
        public_key: public_key(&WRONG_SIGNING_SEED),
        operation_key: fixture.operation_key.clone(),
        context: detached_audit_context(&fixture.context),
    };
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &fixture.completion.provider_completion_attestation,
            &wrong_key_trust,
        ),
        Err("provider signature"),
    );

    for challenge in ["not-hex", "00"] {
        let mut forged_challenge: DetachedAuditProof =
            serde_json::from_str(&fixture.completion.provider_completion_attestation)
                .expect("valid detached proof");
        forged_challenge.challenge = challenge.to_owned();
        let forged_challenge = serde_json::to_string(&forged_challenge).expect("challenge proof");
        let expected = if challenge == "not-hex" {
            "challenge"
        } else {
            "challenge length"
        };
        assert_eq!(
            independently_verify_detached_completion(
                &fixture.completion.recovery_closure,
                &forged_challenge,
                &trust,
            ),
            Err(expected),
        );
    }
    let mut forged_challenge: DetachedAuditProof =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid detached proof");
    forged_challenge.challenge = "00".repeat(33);
    let forged_challenge = serde_json::to_string(&forged_challenge).expect("long challenge proof");
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &forged_challenge,
            &trust,
        ),
        Err("challenge length"),
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&fixture.completion.provider_completion_attestation)
            .expect("valid proof JSON");
    let reordered = serde_json::to_string(&parsed).expect("reordered proof");
    assert_ne!(
        reordered,
        fixture.completion.provider_completion_attestation
    );
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &reordered,
            &trust,
        ),
        Err("proof canonicality"),
    );
}

#[test]
fn independent_detached_audit_rejects_context_and_proof_boundary_substitutions() {
    let fixture = persisted_completion_fixture();
    let assert_context = |mutate: fn(&mut ProviderTargetContext), expected: &'static str| {
        let mut context = fixture.context.clone();
        mutate(&mut context);
        let proof = persisted_proof(
            &context,
            &fixture.operation_key,
            &fixture.provider_id,
            &fixture.mutation,
            &SIGNING_SEED,
        );
        let trust = DetachedAuditTrust {
            provider_id: fixture.provider_id.clone(),
            public_key: public_key(&SIGNING_SEED),
            operation_key: fixture.operation_key.clone(),
            context: detached_audit_context(&context),
        };
        assert_eq!(
            independently_verify_detached_completion(
                &fixture.completion.recovery_closure,
                &proof,
                &trust,
            ),
            Err(expected),
        );
    };

    assert_context(|context| context.database_oid = 0, "provider context");
    assert_context(|context| context.backend_pid = 0, "provider context");
    assert_context(|context| context.transaction_id = None, "provider context");
    assert_context(
        |context| context.snapshot_id = Some("snapshot".to_owned()),
        "provider context",
    );
    assert_context(
        |context| context.application_marker = "a".repeat(61),
        "provider context",
    );
    assert_context(
        |context| context.application_marker = "g".repeat(62),
        "provider context",
    );
    assert_context(
        |context| context.store_incarnation.writer_epoch = 0,
        "store incarnation",
    );

    let valid = fixture.completion.provider_completion_attestation.clone();
    for proof_value in [String::new(), "é".to_owned()] {
        let trust = DetachedAuditTrust {
            provider_id: fixture.provider_id.clone(),
            public_key: public_key(&SIGNING_SEED),
            operation_key: fixture.operation_key.clone(),
            context: detached_audit_context(&fixture.context),
        };
        assert_eq!(
            independently_verify_detached_completion(
                &fixture.completion.recovery_closure,
                &proof_value,
                &trust,
            ),
            Err("proof bounds"),
        );
    }
    let oversized = "x".repeat(MAX_PROVIDER_PROOF_BYTES + 1);
    let trust = DetachedAuditTrust {
        provider_id: fixture.provider_id.clone(),
        public_key: public_key(&SIGNING_SEED),
        operation_key: fixture.operation_key.clone(),
        context: detached_audit_context(&fixture.context),
    };
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &oversized,
            &trust,
        ),
        Err("proof bounds"),
    );

    let mut unknown_value: serde_json::Value = serde_json::from_str(&valid).expect("valid proof");
    unknown_value
        .as_object_mut()
        .expect("proof object")
        .insert("unknown".to_owned(), serde_json::json!(true));
    let unknown = serde_json::to_string(&unknown_value).expect("unknown proof");
    assert_eq!(
        independently_verify_detached_completion(
            &fixture.completion.recovery_closure,
            &unknown,
            &trust,
        ),
        Err("proof JSON"),
    );
}

#[test]
fn detached_completion_proof_rejects_crypto_and_target_substitutions() {
    let fixture = completion_fixture();
    let valid = persisted_proof(
        &fixture.context,
        &fixture.operation_key,
        &fixture.provider_id,
        &fixture.mutation,
        &SIGNING_SEED,
    );
    let mut forged_signature = decode_persisted_mutation_proof(&valid).expect("valid proof");
    forged_signature.signature.replace_range(0..2, "00");
    let forged_signature = serde_json::to_string(&forged_signature).expect("forged proof");
    assert!(matches!(
        verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &forged_signature,
            None,
            &fixture.operation_key,
            &fixture.mutation,
        ),
        Err(PostgresEvmWalletError::FenceRejected)
    ));
    assert!(matches!(
        verify_persisted_mutation_proof(
            &public_key(&WRONG_SIGNING_SEED),
            &fixture.provider_id,
            &valid,
            None,
            &fixture.operation_key,
            &fixture.mutation,
        ),
        Err(PostgresEvmWalletError::FenceRejected)
    ));

    let mut tampered_mutation = fixture.mutation.clone();
    if let ProviderMutation::Completion { request, .. } = &mut tampered_mutation {
        request.canonical_terminal_outcome.canonical_public_result =
            "{\"execution_disposition\":\"reverted\"}".to_owned();
    }
    assert!(matches!(
        verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &valid,
            None,
            &fixture.operation_key,
            &tampered_mutation,
        ),
        Err(PostgresEvmWalletError::InvalidAuthority)
    ));

    let mut substituted_provider = decode_persisted_mutation_proof(&valid).expect("valid proof");
    substituted_provider.provider_id = "mfm.evm.fixture/other-provider".to_owned();
    let substituted_provider =
        serde_json::to_string(&substituted_provider).expect("provider substitution");
    assert!(matches!(
        verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &substituted_provider,
            None,
            &fixture.operation_key,
            &fixture.mutation,
        ),
        Err(PostgresEvmWalletError::InvalidAuthority)
    ));

    let mut substituted_operation = decode_persisted_mutation_proof(&valid).expect("valid proof");
    substituted_operation.operation_key = "mfm.evm.fixture/other-operation".to_owned();
    let substituted_operation =
        serde_json::to_string(&substituted_operation).expect("operation substitution");
    assert!(matches!(
        verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &substituted_operation,
            None,
            &fixture.operation_key,
            &fixture.mutation,
        ),
        Err(PostgresEvmWalletError::InvalidAuthority)
    ));

    let mut substituted_context = fixture.context.clone();
    substituted_context.schema_name = "other_schema".to_owned();
    let substituted_context_proof = persisted_proof(
        &substituted_context,
        &fixture.operation_key,
        &fixture.provider_id,
        &fixture.mutation,
        &SIGNING_SEED,
    );
    assert!(validate_retained_mutation_target(
        &decode_persisted_mutation_proof(&substituted_context_proof)
            .expect("substituted context proof")
            .context,
        &fixture.context.store_incarnation,
        &fixture.context.schema_name,
        fixture.context.database_oid,
    )
    .is_err());
    assert!(verify_persisted_mutation_proof(
        &public_key(&SIGNING_SEED),
        &fixture.provider_id,
        &substituted_context_proof,
        Some(&fixture.context),
        &fixture.operation_key,
        &fixture.mutation,
    )
    .is_err());

    for substitution in [
        |context: &mut ProviderTargetContext| context.database_oid = 18,
        |context: &mut ProviderTargetContext| {
            context.store_incarnation.wallet_nonce_store_lineage_id =
                "mfm.evm.fixture/other-lineage".to_owned()
        },
        |context: &mut ProviderTargetContext| context.store_incarnation.writer_epoch = 2,
    ] {
        let mut context = fixture.context.clone();
        substitution(&mut context);
        let proof = persisted_proof(
            &context,
            &fixture.operation_key,
            &fixture.provider_id,
            &fixture.mutation,
            &SIGNING_SEED,
        );
        let parsed = decode_persisted_mutation_proof(&proof).expect("substitution proof");
        assert!(validate_retained_mutation_target(
            &parsed.context,
            &fixture.context.store_incarnation,
            &fixture.context.schema_name,
            fixture.context.database_oid,
        )
        .is_err());
    }
}

#[test]
fn detached_completion_proof_rejects_digest_challenge_and_canonicality_tampering() {
    let fixture = completion_fixture();
    let valid = persisted_proof(
        &fixture.context,
        &fixture.operation_key,
        &fixture.provider_id,
        &fixture.mutation,
        &SIGNING_SEED,
    );
    let mut bad_digest = decode_persisted_mutation_proof(&valid).expect("valid proof");
    bad_digest.payload_digest =
        "sha256v1:0000000000000000000000000000000000000000000000000000000000000000".to_owned();
    let bad_digest = serde_json::to_string(&bad_digest).expect("bad digest proof");
    assert!(matches!(
        verify_persisted_mutation_proof(
            &public_key(&SIGNING_SEED),
            &fixture.provider_id,
            &bad_digest,
            None,
            &fixture.operation_key,
            &fixture.mutation,
        ),
        Err(PostgresEvmWalletError::InvalidAuthority)
    ));

    for challenge in ["not-hex", "00", &"00".repeat(33)] {
        let mut forged = decode_persisted_mutation_proof(&valid).expect("valid proof");
        forged.challenge = challenge.to_owned();
        let forged = serde_json::to_string(&forged).expect("challenge proof");
        assert!(matches!(
            verify_persisted_mutation_proof(
                &public_key(&SIGNING_SEED),
                &fixture.provider_id,
                &forged,
                None,
                &fixture.operation_key,
                &fixture.mutation,
            ),
            Err(PostgresEvmWalletError::InvalidAuthority)
        ));
    }

    let unknown = valid.replacen(
        "{\"provider_id\"",
        "{\"unknown\":\"field\",\"provider_id\"",
        1,
    );
    assert!(decode_persisted_mutation_proof(&unknown).is_err());
    let duplicate = valid.replacen(
        "\"provider_id\":",
        "\"provider_id\":\"mfm.evm.fixture/duplicate\",\"provider_id\":",
        1,
    );
    assert!(decode_persisted_mutation_proof(&duplicate).is_err());
    let parsed: serde_json::Value = serde_json::from_str(&valid).expect("valid JSON");
    let reordered = serde_json::to_string(&parsed).expect("reordered JSON");
    assert_ne!(reordered, valid);
    assert!(decode_persisted_mutation_proof(&reordered).is_err());
    assert!(decode_persisted_mutation_proof(&format!(" {valid} ")).is_err());
    assert!(decode_persisted_mutation_proof("not-json").is_err());
    assert!(decode_persisted_mutation_proof("é").is_err());
    assert!(decode_persisted_mutation_proof(&"x".repeat(MAX_PROVIDER_PROOF_BYTES + 1)).is_err());
}
