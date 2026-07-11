use super::*;

type PreparedInvocationMutation = Box<dyn FnOnce(&mut PreparedContractInvocation)>;

#[test]
fn prepared_invocation_guard_rejects_malformed_public_contract() {
    let cases: Vec<PreparedInvocationMutation> = vec![
        Box::new(|prepared| prepared.prepared_version = 2),
        Box::new(|prepared| {
            prepared.expected_signer_address =
                "0x0F65FE9276BC9A24AE7083AE28E2660EF72DF99E".to_owned();
        }),
        Box::new(|prepared| prepared.transactions[0].index = 7),
        Box::new(|prepared| prepared.transactions[0].chain_id = 2),
        Box::new(|prepared| prepared.transactions[0].value_wei = "0x1".to_owned()),
        Box::new(|prepared| prepared.transactions[0].gas_price = Some("7".to_owned())),
        Box::new(|prepared| prepared.transactions[0].data_digest = "not-a-digest".to_owned()),
        Box::new(|prepared| prepared.transactions[0].signing_digest = "not-a-hash".to_owned()),
    ];

    for mutate in cases {
        let mut prepared = prepared_invocation_fixture();
        mutate(&mut prepared);
        assert!(matches!(
            ensure_prepared_invocation_public(&prepared),
            Err(EvmContractAdapterError::InvalidPreparedInvocation)
        ));
    }
}

#[test]
fn prepared_invocation_deserialization_rejects_unknown_public_fields() {
    let mut top_level = serde_json::to_value(prepared_invocation_fixture()).expect("json");
    top_level
        .as_object_mut()
        .expect("object")
        .insert("raw_transaction".to_owned(), serde_json::json!("0x01"));
    assert!(serde_json::from_value::<PreparedContractInvocation>(top_level).is_err());

    let mut nested = serde_json::to_value(prepared_invocation_fixture()).expect("json");
    nested["transactions"][0]
        .as_object_mut()
        .expect("transaction object")
        .insert("signature".to_owned(), serde_json::json!("0x01"));
    assert!(serde_json::from_value::<PreparedContractInvocation>(nested).is_err());
}

#[test]
fn context_nonce_resource_key_uses_network_context_ref_and_account() {
    let key = account_nonce_resource_key(
        &ContractNonceResourceScope {
            evm_network_context_ref:
                "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
        },
        "0x000000000000000000000000000000000000dead",
    )
    .expect("resource key");

    assert_eq!(
        key.as_str(),
        r#"{"account":"0x000000000000000000000000000000000000dead","evm_network_context_ref":"content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
    );
    assert!(!key.as_str().contains("\"chain_id\""));
}

fn prepared_invocation_fixture() -> PreparedContractInvocation {
    PreparedContractInvocation {
        prepared_version: 1,
        phase: ContractMutationPhase::Deploy,
        context_ref: mfm_values::ContextRefValue::from(ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_with(0x44),
        )),
        evm_network_context_ref:
            "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
        resource_stage: ContractLifecycleStage::Deployed,
        network_id: "ethereum-mainnet".to_owned(),
        expected_chain_id: 1,
        signer_ref: "deployer".to_owned(),
        expected_signer_address: "0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e".to_owned(),
        transactions: vec![PreparedContractTransactionEvidence {
            index: 0,
            style: PreparedContractTransactionStyle::Eip1559,
            chain_id: 1,
            nonce: 7,
            to_address: None,
            value_wei: "0".to_owned(),
            gas_limit: 21_000,
            max_fee_per_gas: Some("11".to_owned()),
            max_priority_fee_per_gas: Some("3".to_owned()),
            gas_price: None,
            data_digest:
                "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            data_len: 2,
            signing_digest:
                "0x0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            expected_transaction_hash:
                "0x1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned(),
        }],
        poll_interval_ms: 1_000,
        max_receipt_polls: 10,
    }
}

fn wrong_context_prepared_invocation_fixture() -> PreparedContractInvocation {
    let mut prepared = prepared_invocation_fixture();
    prepared.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x92),
    ));
    prepared
}

fn certified_side_effect_context_for_prepared(
    prepared: &PreparedContractInvocation,
) -> replay::CertifiedSideEffectContext {
    replay::CertifiedSideEffectContext {
        node_context: spec::NodeContextSpec::Required {
            context_ref: prepared.context_ref.as_context_ref().clone(),
        },
        output_context: spec::CellContextSpec::Bound {
            context_ref: prepared.context_ref.as_context_ref().clone(),
            resource_kind: mfm_evm_contract_model::contract_instance_resource_kind().clone(),
            stage: context_stage_for_lifecycle_stage(prepared.resource_stage).clone(),
            producer: Box::new(spec::ContextProducerSpec {
                producer_descriptor_ids: vec![DescriptorId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_with(0x45),
                )],
                seed_producers_allowed: false,
            }),
        },
    }
}

fn replay_artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
) -> (store::ArtifactEvidenceRef, Vec<u8>)
where
    T: MfmValue + Serialize,
{
    let bytes = canonical_value_bytes(value);
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    (
        store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
            digest,
            byte_len: bytes.len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
            schema_id: Some(T::schema_id().expect("schema id")),
            semantic_type_id: Some(T::semantic_id().expect("semantic id")),
            producer_node_id: Some(node_id(0x46)),
            producer_seed_id: None,
            artifact_role: role,
        },
        bytes,
    )
}

fn replay_attempt_id() -> AttemptId {
    AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x47))
}

fn replay_scope_id() -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x48))
}

fn replay_pair_id() -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x49))
}

fn replay_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("evm-contract-replay-test").expect("ledger key")
}

fn replay_deploy_intent(prepared: &PreparedContractInvocation) -> ContextContractDeployIntent {
    ContextContractDeployIntent {
        intent_version: 1,
        transaction: ContextContractTransactionIntent {
            intent_version: 1,
            context_ref: prepared.context_ref.clone(),
            network_id: prepared.network_id.clone(),
            expected_chain_id: prepared.expected_chain_id,
            signer_ref: prepared.signer_ref.clone(),
            expected_signer_address: prepared.expected_signer_address.clone(),
            to_address: None,
            value_wei: Some("0".to_owned()),
            data_ref: None,
            transaction: EvmTransactionPolicy::new(
                ConfigTransactionStyle::Eip1559,
                Some(prepared.transactions[0].gas_limit),
                prepared.transactions[0].max_fee_per_gas.clone(),
                prepared.transactions[0].max_priority_fee_per_gas.clone(),
                None,
            )
            .expect("transaction policy"),
        },
        constructor_args_len: 0,
        contract_profile_id: "test-profile".to_owned(),
    }
}

fn replay_intent_evidence(
    prepared: &PreparedContractInvocation,
) -> replay::SideEffectIntentReplayEvidence {
    let intent = replay_deploy_intent(prepared);
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(&intent, events::ArtifactRole::SideEffectIntent);
    let binding = evm_contract_lifecycle_adapter_binding().expect("adapter binding");
    replay::SideEffectIntentReplayEvidence {
        intent: side_effect::IntentPersisted {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            scope_id: replay_scope_id(),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            intent_schema_id: ContextContractDeployIntent::schema_id().expect("intent schema"),
            intent_hash: artifact.digest.clone(),
            intent_artifact_id: artifact.artifact_id.clone(),
            intent_artifact_evidence_hash: artifact.evidence_hash().expect("intent evidence hash"),
            idempotency_input_schema_id: ContractTransactionIdempotency::schema_id()
                .expect("idempotency schema"),
            idempotency_input_hash: content_digest(0x4b),
            idempotency_key: events::IdempotencyKeyRef::new("evm-contract-replay-test")
                .expect("idempotency key"),
            capability_kind: EvmTransactionSubmitCapability::kind().expect("capability kind"),
            capability_version: EvmTransactionSubmitCapability::version()
                .expect("capability version"),
            adapter_kind: binding.adapter_kind().clone(),
            adapter_version: binding.adapter_version().clone(),
        },
        artifact,
        artifact_bytes,
    }
}

#[test]
fn replay_intent_rejects_prepared_transaction_mismatches() {
    #[derive(Clone, Copy)]
    enum Mismatch {
        Count,
        Destination,
        Policy,
    }

    for mismatch in [Mismatch::Count, Mismatch::Destination, Mismatch::Policy] {
        let original = prepared_invocation_fixture();
        let intent = replay_intent_evidence(&original);
        let mut prepared = original;
        match mismatch {
            Mismatch::Count => {
                let mut extra = prepared.transactions[0].clone();
                extra.index = 1;
                prepared.transactions.push(extra);
            }
            Mismatch::Destination => {
                prepared.transactions[0].to_address =
                    Some("0x000000000000000000000000000000000000beef".to_owned());
            }
            Mismatch::Policy => {
                prepared.transactions[0].style = PreparedContractTransactionStyle::Legacy;
                prepared.transactions[0].gas_price = Some("9".to_owned());
                prepared.transactions[0].max_fee_per_gas = None;
                prepared.transactions[0].max_priority_fee_per_gas = None;
            }
        }

        assert!(verify_replay_intent_matches_prepared(&intent, &prepared).is_err());
    }
}

#[test]
fn replay_prepared_transaction_data_must_match_certified_inputs() {
    let tx_inputs = vec![PreparedTransactionInput {
        to: None,
        value_wei: 0,
        data: vec![0x60, 0x00],
    }];
    let mut prepared = prepared_invocation_fixture();
    prepared.transactions[0].data_digest = digest_bytes(&tx_inputs[0].data).to_string();
    prepared.transactions[0].data_len = tx_inputs[0].data.len() as u64;

    verify_prepared_transaction_data_matches_inputs(&prepared, &tx_inputs)
        .expect("matching prepared transaction data");

    let mut wrong_digest = prepared.clone();
    wrong_digest.transactions[0].data_digest = digest_bytes(&[0x61, 0x00]).to_string();
    let error = verify_prepared_transaction_data_matches_inputs(&wrong_digest, &tx_inputs)
        .expect_err("mismatched data digest rejects");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_len = prepared;
    wrong_len.transactions[0].data_len += 1;
    let error = verify_prepared_transaction_data_matches_inputs(&wrong_len, &tx_inputs)
        .expect_err("mismatched data length rejects");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

fn replay_prepared_evidence(
    prepared: &PreparedContractInvocation,
) -> replay::PreparedInvocationReplayEvidence {
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(prepared, events::ArtifactRole::PreparedInvocation);
    replay::PreparedInvocationReplayEvidence {
        prepared: side_effect::InvocationPrepared {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: side_effect::ClaimFencingToken::new("claim-token")
                .expect("claim token"),
            resource_key: None,
            prepared_artifact_id: Some(artifact.artifact_id.clone()),
            prepared_hash: Some(artifact.digest.clone()),
            prepared_artifact_evidence_hash: Some(
                artifact.evidence_hash().expect("prepared evidence hash"),
            ),
        },
        artifact,
        artifact_bytes,
    }
}

fn replay_submission_evidence(
    prepared: &PreparedContractInvocation,
) -> replay::SubmissionReplayEvidence {
    let submissions = prepared_anchor_submissions(prepared).expect("submissions");
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(&submissions, events::ArtifactRole::Submission);
    replay::SubmissionReplayEvidence {
        submission: side_effect::SubmissionObserved {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: 1,
            submission_schema_id: ContractTransactionSubmissions::schema_id()
                .expect("submission schema"),
            submission_hash: artifact.digest.clone(),
            submission_artifact_id: artifact.artifact_id.clone(),
            submission_artifact_evidence_hash: artifact
                .evidence_hash()
                .expect("submission evidence hash"),
        },
        artifact,
        artifact_bytes,
    }
}

fn replay_receipt_evidence(prepared: &PreparedContractInvocation) -> replay::ReceiptReplayEvidence {
    let receipt = ContractDeployReceipt {
        receipt_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: ContractLifecycleStage::Deployed,
        contract_address: "0x000000000000000000000000000000000000beef".to_owned(),
        receipt: ContractTransactionReceipt {
            receipt_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: ContractLifecycleStage::Deployed,
            transaction_hash: prepared.transactions[0].expected_transaction_hash.clone(),
            block_number: 7,
            status: true,
            receipt_evidence: None,
        },
    };
    let (artifact, artifact_bytes) =
        replay_artifact_for_value(&receipt, events::ArtifactRole::Receipt);
    replay::ReceiptReplayEvidence {
        receipt: side_effect::ReceiptObserved {
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x4a)),
            node_id: node_id(0x46),
            attempt_id: replay_attempt_id(),
            ledger_key: replay_ledger_key(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: replay_pair_id(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: 1,
            receipt_schema_id: ContractDeployReceipt::schema_id().expect("receipt schema"),
            receipt_hash: artifact.digest.clone(),
            receipt_artifact_id: artifact.artifact_id.clone(),
            receipt_artifact_evidence_hash: artifact
                .evidence_hash()
                .expect("receipt evidence hash"),
            replay_verifier_id: replay_verifier_id().expect("replay verifier id"),
            resource_touched_set: None,
        },
        artifact,
        artifact_bytes,
    }
}

fn receipt_polling_invocation(transaction_hash: &str) -> PreparedContractInvocation {
    let mut prepared = prepared_invocation_fixture();
    prepared.network_id = "reth-dev".to_owned();
    prepared.expected_chain_id = 31337;
    prepared.expected_signer_address = "0x0000000000000000000000000000000000000001".to_owned();
    prepared.poll_interval_ms = 25;
    prepared.max_receipt_polls = 3;

    let transaction = &mut prepared.transactions[0];
    transaction.chain_id = 31337;
    transaction.data_len = 0;
    transaction.signing_digest = transaction_hash.to_owned();
    transaction.expected_transaction_hash = transaction_hash.to_owned();
    prepared
}

fn receipt_polling_submissions(
    prepared: &PreparedContractInvocation,
    transaction_hash: &str,
) -> ContractTransactionSubmissions {
    ContractTransactionSubmissions {
        submissions_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: prepared.resource_stage,
        transactions: vec![ContractTransactionSubmission {
            submission_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: prepared.resource_stage,
            transaction_hash: transaction_hash.to_owned(),
            signer_public_key: None,
        }],
    }
}

fn finality_receipt() -> ContractTransactionReceipt {
    ContractTransactionReceipt {
        receipt_version: 1,
        context_ref: prepared_invocation_fixture().context_ref,
        evm_network_context_ref: prepared_invocation_fixture().evm_network_context_ref,
        resource_stage: ContractLifecycleStage::Deployed,
        transaction_hash: TEST_TRANSACTION_HASH.to_owned(),
        block_number: 63,
        status: true,
        receipt_evidence: None,
    }
}

fn external_source_evidence_for_context(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> ExternalEvmSourceEvidence {
    ExternalEvmSourceEvidence {
        network_id: context.value().network.network_id.as_str().to_owned(),
        expected_chain_id: context.value().network.expected_chain_id(),
        observed_chain_id: context.value().network.expected_chain_id(),
        source_ref: "adapter-test".to_owned(),
        policy_id: "adapter-test".to_owned(),
    }
}

fn expected_bool(value: bool) -> ExpectedValue {
    ExpectedValue::from_json_value(&json!(value)).expect("expected bool")
}

fn expected_bool_json(value: bool) -> serde_json::Value {
    json!({ "json_text": value.to_string() })
}

fn validation_read_result(passed: bool) -> ValidationReadResult {
    ValidationReadResult {
        function: "owner".to_owned(),
        args: Vec::new(),
        expected: expected_bool(true),
        actual: expected_bool(passed),
        passed,
    }
}

fn validation_event_result(passed: bool) -> ValidationEventResult {
    ValidationEventResult {
        event: "Configured".to_owned(),
        min_count: 1,
        observed_count: if passed { 1 } else { 0 },
        passed,
    }
}

fn external_adoption_for_replay(
    evidence_policy: serde_json::Value,
) -> mfm_evm_contract_model::AdoptExternalAddress {
    serde_json::from_value(json!({
        "address": "0x000000000000000000000000000000000000beef",
        "provenance_label": "adapter-test",
        "evidence_policy": evidence_policy,
    }))
    .expect("external adoption")
}

fn external_adoption_evidence_for_replay(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    stage: ContractLifecycleStage,
    adoption: &mfm_evm_contract_model::AdoptExternalAddress,
) -> ExternalAdoptionEvidence {
    ExternalAdoptionEvidence {
        evidence_policy_digest: digest_for_value(&adoption.evidence_policy)
            .expect("evidence policy digest"),
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)
            .expect("network context ref"),
        resource_stage: stage,
        observed_chain_id: context.value().network.expected_chain_id(),
        code_read_evidence: None,
        read_assertion_evidence: Vec::new(),
        event_assertion_evidence: Vec::new(),
    }
}

async fn verified_test_finality(
    runtime: &EvmContractRuntime,
    receipt: &ContractTransactionReceipt,
    required_confirmations: u64,
) -> mfm_runtime::Result<u64> {
    verified_finality_confirmations(
        runtime,
        std::slice::from_ref(receipt),
        required_confirmations,
    )
    .await
}

pub(super) fn sorted_json_keys(value: &serde_json::Value) -> Vec<&str> {
    let mut keys = value
        .as_object()
        .expect("json object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[derive(Clone, Copy)]
pub(super) enum RecoveryReceiptMode {
    Landed,
    Pending,
    ProviderFailure,
}

#[derive(Clone, Copy)]
pub(super) enum RecoveryOccupancyMode {
    Unknown,
    Occupied { transaction_hash: B256 },
}

#[test]
fn replay_verifier_uses_contract_namespace() {
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");

    assert_eq!(verifier.verifier_id().as_str(), REPLAY_VERIFIER_ID);
    assert!(!verifier
        .verifier_id()
        .as_str()
        .contains(&["d", "cv"].concat()));
}

#[test]
fn replay_verifier_accepts_context_configure_schemas() {
    let receipt_schema = ContextContractConfigureReceipt::schema_id().expect("receipt schema");
    verify_contract_receipt_schema(&receipt_schema).expect("context configure receipt schema");

    let confirmation = ContextContractConfigureConfirmation {
        confirmation_version: 1,
        context_ref: prepared_invocation_fixture().context_ref,
        evm_network_context_ref: prepared_invocation_fixture().evm_network_context_ref,
        resource_stage: ContractLifecycleStage::Configured,
        confirmations: 3,
        configure_node: LifecycleNodeIdRef::from(node_id(0x44)),
        receipts: Vec::new(),
        call_evidence_refs: Vec::new(),
        confirmation_evidence_refs: Vec::new(),
        configured_block_number: None,
    };
    let confirmation_schema =
        ContextContractConfigureConfirmation::schema_id().expect("confirmation schema");
    let confirmation_bytes = serde_json::to_vec(&confirmation).expect("confirmation json");

    verify_contract_confirmation_schema(&confirmation_schema, &confirmation_bytes, 3)
        .expect("sufficient context configure confirmation depth");
    let error = verify_contract_confirmation_schema(&confirmation_schema, &confirmation_bytes, 4)
        .expect_err("insufficient context configure confirmation depth");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_validation_report_evidence_must_match_retained_read_and_event_results() {
    let context = certified_contract_context("reth-dev", 31337);
    let read_result = validation_read_result(true);
    let event_result = validation_event_result(true);
    let read_evidence = ExternalReadAssertionEvidence {
        source: external_source_evidence_for_context(&context),
        result: read_result.clone(),
    };
    let event_evidence = ExternalEventAssertionEvidence {
        source: external_source_evidence_for_context(&context),
        result: event_result.clone(),
    };

    verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        std::slice::from_ref(&read_evidence),
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect("matching validation evidence");

    let missing = verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        &[],
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect_err("missing validation read evidence must reject");
    assert_eq!(missing.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut mismatched = read_evidence.clone();
    mismatched.result = validation_read_result(false);
    let error = verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        std::slice::from_ref(&mismatched),
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect_err("mismatched validation read evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_context = read_evidence;
    wrong_context.source.observed_chain_id += 1;
    let error = verify_validation_source_evidence(
        &context,
        std::slice::from_ref(&read_result),
        std::slice::from_ref(&wrong_context),
        std::slice::from_ref(&event_result),
        std::slice::from_ref(&event_evidence),
    )
    .expect_err("wrong-context validation read evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_validation_report_results_must_match_certified_validate_action() {
    let action: ValidateAction = serde_json::from_value(json!({
        "read_assertions": [
            {
                "function": "owner",
                "expected": expected_bool_json(true)
            }
        ],
        "event_assertions": [
            {
                "event": "Configured",
                "min_count": 1
            }
        ]
    }))
    .expect("validate action");
    let context = certified_contract_context("reth-dev", 31337);
    let report = ContextBoundValidationReport {
        report_version: 1,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        configured_instance: mfm_evm_contract_model::ConfiguredContractInstanceRef {
            context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
            address: ContractAddress::new("0x000000000000000000000000000000000000beef")
                .expect("address"),
            configuration_claim: ConfigurationClaim::ExternalClaimedConfigured {
                provenance_label: serde_json::from_value(json!("adapter-test"))
                    .expect("provenance label"),
                evidence_policy_digest: digest_for_value(&json!({"adapter": "test"}))
                    .expect("digest"),
            },
        },
        observed_chain_id: 31337,
        configuration_read_results: Vec::new(),
        configuration_event_results: Vec::new(),
        read_results: vec![validation_read_result(true)],
        event_results: vec![validation_event_result(true)],
        validation_read_evidence: Vec::new(),
        validation_event_evidence: Vec::new(),
        evidence_refs: Vec::new(),
        valid: true,
    };

    verify_validation_results_match_action(&report, &action)
        .expect("report results match validate action");

    let mut mismatched = report;
    mismatched.read_results[0].expected = expected_bool(false);
    let error = verify_validation_results_match_action(&mismatched, &action)
        .expect_err("mismatched read action evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_external_adoption_evidence_must_match_context_stage_and_policy() {
    let context = certified_contract_context("reth-dev", 31337);
    let adoption = external_adoption_for_replay(json!({
        "require_code": false,
        "allow_external_claimed_configured": true,
        "initial_read_assertions": [
            {
                "function": "owner",
                "expected": expected_bool_json(true)
            }
        ]
    }));
    let mut evidence = external_adoption_evidence_for_replay(
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    );
    evidence
        .read_assertion_evidence
        .push(ExternalReadAssertionEvidence {
            source: external_source_evidence_for_context(&context),
            result: validation_read_result(true),
        });

    verify_replayed_external_adoption(
        &evidence,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect("matching external adoption evidence");

    let mut missing = evidence.clone();
    missing.read_assertion_evidence.clear();
    let error = verify_replayed_external_adoption(
        &missing,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("missing external adoption assertion evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_context = evidence.clone();
    wrong_context.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x92),
    ));
    let error = verify_replayed_external_adoption(
        &wrong_context,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("wrong-context external adoption evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);

    let mut wrong_stage = evidence;
    wrong_stage.resource_stage = ContractLifecycleStage::Deployed;
    let error = verify_replayed_external_adoption(
        &wrong_stage,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("wrong-stage external adoption evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_external_adoption_requires_code_evidence_when_policy_requires_code() {
    let context = certified_contract_context("reth-dev", 31337);
    let adoption = external_adoption_for_replay(json!({
        "require_code": true,
        "allow_external_claimed_configured": true
    }));
    let evidence = external_adoption_evidence_for_replay(
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    );

    let error = verify_replayed_external_adoption(
        &evidence,
        &context,
        ContractLifecycleStage::Configured,
        &adoption,
    )
    .expect_err("missing required code evidence must reject");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_submission_and_receipt_reject_context_mismatch() {
    enum Case {
        Submission,
        Receipt,
    }

    for case in [Case::Submission, Case::Receipt] {
        let prepared = prepared_invocation_fixture();
        match case {
            Case::Submission => {
                let mut submissions = prepared_anchor_submissions(&prepared).expect("submissions");
                submissions.context_ref = mfm_values::ContextRefValue::from(
                    ContextRef::from_digest(DigestAlgorithm::Sha256JcsV1, digest_with(0x90)),
                );

                assert!(matches!(
                    verify_prepared_submissions(&prepared, &submissions),
                    Err(EvmContractAdapterError::InvalidPreparedInvocation)
                ));
            }
            Case::Receipt => {
                let mut receipt = ContractDeployReceipt {
                    receipt_version: 1,
                    context_ref: prepared.context_ref.clone(),
                    evm_network_context_ref: prepared.evm_network_context_ref.clone(),
                    resource_stage: ContractLifecycleStage::Deployed,
                    contract_address: "0x000000000000000000000000000000000000beef".to_owned(),
                    receipt: ContractTransactionReceipt {
                        receipt_version: 1,
                        context_ref: prepared.context_ref.clone(),
                        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
                        resource_stage: ContractLifecycleStage::Deployed,
                        transaction_hash: prepared.transactions[0]
                            .expected_transaction_hash
                            .clone(),
                        block_number: 7,
                        status: true,
                        receipt_evidence: None,
                    },
                };
                receipt.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_with(0x91),
                ));
                let schema = ContractDeployReceipt::schema_id().expect("schema");
                let bytes = serde_json::to_vec(&receipt).expect("receipt json");

                let error = verify_contract_receipt_artifact(&schema, &bytes, &prepared)
                    .expect_err("mismatched receipt context");
                assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
            }
        }
    }
}

#[test]
fn replay_verifier_accepts_prepared_context_matching_certified_node_context() {
    let prepared = prepared_invocation_fixture();
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");
    let input = replay::SideEffectSubmissionReplayInput {
        intent: replay_intent_evidence(&prepared),
        certified_context: certified_side_effect_context_for_prepared(&prepared),
        prepared_invocation: Some(replay_prepared_evidence(&prepared)),
        submission: replay_submission_evidence(&prepared),
    };

    verifier
        .verify_submission(&input)
        .expect("matching certified context");
}

#[test]
fn replay_verifier_rejects_consistently_wrong_prepared_context() {
    enum Case {
        Submission,
        Receipt,
    }

    for case in [Case::Submission, Case::Receipt] {
        let certified_prepared = prepared_invocation_fixture();
        let wrong_prepared = wrong_context_prepared_invocation_fixture();
        let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");
        let error = match case {
            Case::Submission => {
                let input = replay::SideEffectSubmissionReplayInput {
                    intent: replay_intent_evidence(&wrong_prepared),
                    certified_context: certified_side_effect_context_for_prepared(
                        &certified_prepared,
                    ),
                    prepared_invocation: Some(replay_prepared_evidence(&wrong_prepared)),
                    submission: replay_submission_evidence(&wrong_prepared),
                };
                verifier
                    .verify_submission(&input)
                    .expect_err("prepared and submission artifacts agree with the wrong context")
            }
            Case::Receipt => {
                let input = replay::SideEffectReceiptReplayInput {
                    intent: replay_intent_evidence(&wrong_prepared),
                    certified_context: certified_side_effect_context_for_prepared(
                        &certified_prepared,
                    ),
                    prepared_invocation: Some(replay_prepared_evidence(&wrong_prepared)),
                    submission: Some(replay_submission_evidence(&wrong_prepared)),
                    receipt: replay_receipt_evidence(&wrong_prepared),
                };
                verifier.verify_receipt(&input).expect_err(
                    "prepared, submission, and receipt artifacts agree with the wrong context",
                )
            }
        };

        assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
    }
}

#[tokio::test]
async fn receipt_polling_does_not_retry_permanent_capability_failures() {
    let reads = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(TestEvmProviders::receipt_failure(Arc::clone(&reads)));
    let transaction_hash = TEST_TRANSACTION_HASH;
    let prepared = receipt_polling_invocation(transaction_hash);
    let submissions = receipt_polling_submissions(&prepared, transaction_hash);

    let error = read_receipts_with_poll(&runtime, &prepared, &submissions)
        .await
        .expect_err("permanent capability failure");

    assert!(error.to_string().contains("EVM capability failed"));
    assert_eq!(*reads.lock().expect("reads"), 1);
}

#[tokio::test]
async fn receipt_polling_rejects_tampered_submission_before_provider_read() {
    let reads = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(TestEvmProviders::receipt_failure(Arc::clone(&reads)));
    let prepared = receipt_polling_invocation(TEST_TRANSACTION_HASH);
    let submissions = receipt_polling_submissions(
        &prepared,
        "0x2222222222222222222222222222222222222222222222222222222222222222",
    );

    let error = read_receipts_with_poll(&runtime, &prepared, &submissions)
        .await
        .expect_err("tampered submission hash");

    assert!(error.to_string().contains("EVM transaction hash mismatch"));
    assert_eq!(*reads.lock().expect("reads"), 0);
}

#[tokio::test]
async fn finality_confirmation_requires_certified_depth() {
    let runtime = runtime_from_provider(TestEvmProviders::finality());
    let receipt = finality_receipt();

    let confirmations = verified_test_finality(&runtime, &receipt, 2)
        .await
        .expect("sufficient confirmations");
    assert_eq!(confirmations, 2);

    let error = verified_test_finality(&runtime, &receipt, 3)
        .await
        .expect_err("insufficient confirmations");
    assert!(matches!(error, mfm_runtime::RuntimeError::Blocked(_)));
}

#[test]
fn replay_confirmation_depth_rejects_insufficient_certified_depth() {
    assert!(ensure_replay_confirmation_depth(3, 3).is_ok());
    let error = ensure_replay_confirmation_depth(2, 3).expect_err("insufficient replay depth");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}
