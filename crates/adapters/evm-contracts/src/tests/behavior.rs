use super::replay_tests::sorted_json_keys;
use super::*;

#[test]
fn executable_identity_summary_matches_golden() {
    assert_eq!(
        executable_identity_summary([SIDE_EFFECT_FACTORY, READ_FACTORY]),
        [
            "factory=apply_side_effect;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
            "factory=read_external;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
        ]
    );
}

#[test]
fn event_block_selector_preserves_supported_tags() {
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Earliest,
            }),
            false
        )
        .expect("earliest selector"),
        EvmBlockSelector::Number(0)
    );
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Latest,
            }),
            true
        )
        .expect("latest selector"),
        EvmBlockSelector::Latest
    );
    assert_eq!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Pending,
            }),
            true
        )
        .expect("pending selector"),
        EvmBlockSelector::Pending
    );
    assert_eq!(
        block_selector(Some(&BlockSelector::Number { number: 42 }), false)
            .expect("number selector"),
        EvmBlockSelector::Number(42)
    );
}

#[test]
fn event_block_selector_rejects_unsupported_tags() {
    assert!(matches!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Safe,
            }),
            true
        ),
        Err(EvmContractAdapterError::UnsupportedBlockTag)
    ));
    assert!(matches!(
        block_selector(
            Some(&BlockSelector::Tag {
                tag: BlockTag::Finalized,
            }),
            false
        ),
        Err(EvmContractAdapterError::UnsupportedBlockTag)
    ));
}

#[test]
fn configure_action_transaction_inputs_handle_missing_artifact_by_call_set() {
    enum Expected {
        EmptyInputs,
        MissingArtifact,
    }

    for (label, calls, expected) in [
        ("empty calls", json!([]), Expected::EmptyInputs),
        (
            "non-empty calls",
            json!([
                {
                    "function": "configure",
                }
            ]),
            Expected::MissingArtifact,
        ),
    ] {
        let action = configure_action(calls);
        let result = configure_action_transaction_inputs(
            &action,
            None,
            "0x000000000000000000000000000000000000beef",
        );

        match expected {
            Expected::EmptyInputs => {
                let inputs = result.expect(label);
                assert!(inputs.is_empty(), "{label}");
            }
            Expected::MissingArtifact => {
                let error = match result {
                    Ok(_) => panic!("{label} should require artifact"),
                    Err(error) => error,
                };
                assert!(matches!(
                    error,
                    EvmContractAdapterError::MissingContractArtifact
                ));
            }
        }
    }
}

#[tokio::test]
async fn deploy_preparation_uses_requested_transaction_style_for_contract_creation() {
    for (style, expected_prepared, expected_signing) in [
        (
            "eip1559",
            PreparedContractTransactionStyle::Eip1559,
            SigningTransactionStyle::Eip1559,
        ),
        (
            "legacy",
            PreparedContractTransactionStyle::Legacy,
            SigningTransactionStyle::Legacy,
        ),
    ] {
        let (_fixture, prepared) = prepared_deploy_fixture(style).await;

        assert_eq!(prepared.evidence().transactions.len(), 1, "{style}");
        assert_eq!(
            prepared.evidence().transactions[0].style,
            expected_prepared,
            "{style}"
        );
        assert_eq!(
            prepared.evidence().transactions[0].to_address,
            None,
            "{style}"
        );
        assert_eq!(prepared.signing_requests()[0].style(), expected_signing);
    }
}

#[tokio::test]
async fn context_deploy_preparation_routes_from_certified_context() {
    let providers = TestEvmProviders::preparation_for("reth-dev", 31337);
    let adapter = adapter(&providers);
    let context = certified_contract_context("reth-dev", 31337);
    let action = deploy_action(31337);
    let state = ContextBoundDeployContractState::new(action.clone()).expect("state");
    let intent = state.prepare_intent(&(), &context).expect("intent");
    let artifact = contract_artifact_config();

    let prepared = adapter
        .prepare_context_deploy_invocation(&action, &context, &artifact, &intent)
        .await
        .expect("prepared");

    assert_eq!(prepared.evidence().network_id, "reth-dev");
    assert_eq!(prepared.evidence().expected_chain_id, 31337);
    assert_eq!(
        prepared.evidence().context_ref.as_context_ref(),
        context.context_ref()
    );
    assert_eq!(
        prepared.evidence().resource_stage,
        ContractLifecycleStage::Deployed
    );
    assert_eq!(
        prepared.evidence().transactions[0].chain_id,
        31337,
        "transaction signing chain id must come from certified context"
    );
    assert!(prepared
        .evidence()
        .evm_network_context_ref
        .starts_with("content:sha256-jcs-v1:"));
}

#[tokio::test]
async fn source_run_import_rejects_missing_retained_evidence() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let value_digest = digest_for_value(&source_value).expect("value digest");
    let import: ImportDeployedSpec = serde_json::from_value(source_run_import_json::<
        DeployedContractInstance,
    >(
        &context, "deployed", &value_digest
    ))
    .expect("source-run import");
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );
    let error = runtime
        .import_deployed(&import, &context, &MissingRetainedArtifacts)
        .await
        .expect_err("missing proof must fail closed");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn source_run_import_accepts_retained_export_authority() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |_| {},
    )
    .await;
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );

    let imported = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect("source-run import");

    assert_eq!(imported.context_ref.as_context_ref(), context.context_ref());
    assert_eq!(imported.address, source_value.address);
    assert_eq!(imported.deploy_evidence.len(), 4);
}

#[tokio::test]
async fn source_run_import_without_registry_authority_fails_closed() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |_| {},
    )
    .await;
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new(evm);

    let error = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect_err("source-run import without registry authority");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn source_run_import_rejects_domain_local_export_certificate() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, mut artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |_| {},
    )
    .await;
    let ImportDeployedSpec::FromMfmRun {
        source,
        mut evidence,
    } = import
    else {
        panic!("expected source-run import");
    };
    let old_certificate = json!({
        "certificate_version": 1,
        "source_run_id": source.source_run_id.clone(),
        "source_spec_hash": source.source_spec_hash.clone(),
        "committed_stream_digest": content_digest_str(0x5c),
        "allowed_producer_descriptor_ids": [evidence.source_producer_descriptor_id.clone()],
        "allowed_context_descriptor_ids": [evidence.source_context_descriptor_id.clone()],
        "allowed_schema_ids": [evidence.source_cell_schema_id.clone()],
        "allowed_semantic_type_ids": [evidence.source_cell_semantic_type_id.clone()],
    });
    let old_certificate_bytes = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&old_certificate).expect("old certificate json"),
    )
    .expect("old certificate canonical")
    .to_vec();
    let (old_certificate_ref, old_certificate_store) =
        artifact_ref_for_raw_bytes(&old_certificate_bytes);
    artifacts.artifacts[1] = (old_certificate_store, old_certificate_bytes);
    evidence.source_spec_certificate_ref = old_certificate_ref;
    let import = ImportDeployedSpec::FromMfmRun { source, evidence };
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );

    let error = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect_err("domain-local certificate must not authorize imports");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn source_run_import_rejects_public_projection_and_raw_value_payloads() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let payloads = [
        (
            "public json",
            json!({
                "deployed": serde_json::to_value(&source_value).expect("source value json"),
            }),
        ),
        (
            "projection row",
            json!({
                "cell_id": cell_id_str(0x32),
                "content_digest": content_digest_str(0x33),
                "public_field_path": "deployed",
            }),
        ),
        (
            "raw payload",
            json!({
                "address": "0x1111111111111111111111111111111111111111",
                "context_ref": context.context_ref().to_string(),
            }),
        ),
    ];

    for (label, payload) in payloads {
        let (import, artifacts) = source_run_import_with_authority(
            &context,
            ContractLifecycleStage::Deployed,
            &source_value,
            |_| {},
        )
        .await;
        let (import, artifacts) =
            source_run_import_with_retargeted_source_value_payload(import, artifacts, payload);
        let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
        let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
            evm,
            source_run_certification_registry(),
        );

        let error = runtime
            .import_deployed(&import, &context, &artifacts)
            .await
            .expect_err(label);

        assert!(
            matches!(error, EvmContractAdapterError::SourceRunImportEvidence(_)),
            "{label} rejected with unexpected error: {error:?}"
        );
    }
}

#[tokio::test]
async fn source_run_import_rejects_stream_without_claimed_terminal_event() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let source_value = deployed_source_value(&context);
    let (import, artifacts) = source_run_import_with_authority(
        &context,
        ContractLifecycleStage::Deployed,
        &source_value,
        |evidence| {
            evidence.source_terminal_cell_or_output_event_ref =
                mfm_evm_contract_model::LifecycleEventIdRef::from(EventId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_with(0x7f),
                ));
        },
    )
    .await;
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new_with_source_run_import_registry(
        evm,
        source_run_certification_registry(),
    );

    let error = runtime
        .import_deployed(&import, &context, &artifacts)
        .await
        .expect_err("tampered terminal event must fail");

    assert!(matches!(
        error,
        EvmContractAdapterError::SourceRunImportEvidence(_)
    ));
}

#[tokio::test]
async fn external_adoption_records_replayable_code_evidence() {
    let context = certified_contract_context("ethereum-mainnet", 1);
    let import: ImportDeployedSpec = serde_json::from_value(json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000beef",
            "provenance_label": "audited-external"
        }
    }))
    .expect("external adoption import");
    let evm: Arc<dyn EvmContractReadProvider> = Arc::new(TestEvmProviders::preparation());
    let runtime = EvmContractReadRuntime::new(evm);

    let imported = runtime
        .import_deployed(&import, &context, &MissingRetainedArtifacts)
        .await
        .expect("external adoption");
    let evidence = imported
        .external_adoption_evidence
        .expect("external adoption evidence");

    assert_eq!(evidence.context_ref.as_context_ref(), context.context_ref());
    assert_eq!(evidence.resource_stage, ContractLifecycleStage::Deployed);
    assert_eq!(evidence.observed_chain_id, 1);
    let code = evidence.code_read_evidence.expect("code-read evidence");
    assert_eq!(
        code.address.as_str(),
        "0x000000000000000000000000000000000000beef"
    );
    assert_eq!(code.observed_code_byte_len, 2);
    assert_eq!(code.source.observed_chain_id, 1);
    assert_eq!(code.source.network_id, "ethereum-mainnet");
}

#[tokio::test]
async fn context_prepared_reconstruction_rejects_mismatched_context_ref() {
    let providers = TestEvmProviders::preparation_for("reth-dev", 31337);
    let adapter = adapter(&providers);
    let context = certified_contract_context("reth-dev", 31337);
    let action = deploy_action(31337);
    let state = ContextBoundDeployContractState::new(action.clone()).expect("state");
    let intent = state.prepare_intent(&(), &context).expect("intent");
    let artifact = contract_artifact_config();
    let prepared = adapter
        .prepare_context_deploy_invocation(&action, &context, &artifact, &intent)
        .await
        .expect("prepared");
    let mut evidence = prepared.evidence().clone();
    evidence.context_ref = mfm_values::ContextRefValue::from(ContextRef::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        digest_with(0x99),
    ));

    assert!(matches!(
        adapter.reconstruct_context_deploy_invocation(
            &action, &context, &artifact, &intent, &evidence
        ),
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    ));
}

#[tokio::test]
async fn prepared_invocation_evidence_excludes_live_and_secret_surfaces() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;

    ensure_prepared_invocation_public(prepared.evidence()).expect("public evidence");
    let evidence = prepared.evidence();
    let rendered_value = serde_json::to_value(evidence).expect("json value");
    assert_eq!(
        sorted_json_keys(&rendered_value),
        vec![
            "context_ref",
            "evm_network_context_ref",
            "expected_chain_id",
            "expected_signer_address",
            "max_receipt_polls",
            "network_id",
            "phase",
            "poll_interval_ms",
            "prepared_version",
            "resource_stage",
            "signer_ref",
            "transactions",
        ]
    );
    assert_eq!(evidence.signer_ref, "deployer");
    assert_eq!(
        evidence.expected_signer_address,
        expected_test_signer_address("eip1559")
    );
    let transaction = evidence.transactions.first().expect("transaction");
    let rendered_transaction = rendered_value["transactions"][0].clone();
    assert_eq!(
        sorted_json_keys(&rendered_transaction),
        vec![
            "chain_id",
            "data_digest",
            "data_len",
            "expected_transaction_hash",
            "gas_limit",
            "gas_price",
            "index",
            "max_fee_per_gas",
            "max_priority_fee_per_gas",
            "nonce",
            "signing_digest",
            "style",
            "to_address",
            "value_wei",
        ]
    );
    assert_eq!(transaction.nonce, 7);
    assert_eq!(transaction.gas_limit, 21_000);
    assert_eq!(transaction.max_fee_per_gas.as_deref(), Some("11"));
    assert_eq!(transaction.max_priority_fee_per_gas.as_deref(), Some("3"));
    assert!(transaction
        .data_digest
        .starts_with("content:sha256-jcs-v1:"));
    assert_eq!(transaction.data_len, 2);
    assert!(transaction.signing_digest.starts_with("0x"));
    assert_eq!(transaction.signing_digest.len(), 66);
    assert!(transaction.expected_transaction_hash.starts_with("0x"));
    assert_eq!(transaction.expected_transaction_hash.len(), 66);
}

#[tokio::test]
async fn prepared_invocation_reconstruction_preserves_submission_anchor() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture
        .adapter()
        .reconstruct_context_deploy_invocation(
            &fixture.action,
            &fixture.context,
            &fixture.artifact,
            &fixture.intent,
            prepared.evidence(),
        )
        .expect("reconstructed");

    assert_eq!(reconstructed.evidence(), prepared.evidence());
    assert_eq!(
        reconstructed.evidence().transactions[0].expected_transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[tokio::test]
async fn submit_rejects_resigned_hash_mismatch_before_broadcast() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture.reconstruct_with_hash(&prepared, MISMATCH_HASH);
    let adapter = fixture.adapter();

    assert!(matches!(
        adapter.submit_prepared(&reconstructed).await,
        Err(EvmContractAdapterError::TransactionHashMismatch)
    ));
}

#[tokio::test]
async fn recovery_observes_or_rebroadcasts_anchor_by_receipt_state() {
    for (receipt_mode, expected_submit_count) in [
        (RecoveryReceiptMode::Landed, 0),
        (RecoveryReceiptMode::Pending, 1),
    ] {
        assert_recovery_observed(receipt_mode, expected_submit_count).await;
    }
}

#[tokio::test]
async fn recovery_does_not_rebroadcast_when_resign_hash_mismatches_anchor() {
    let (fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let reconstructed = fixture.reconstruct_with_hash(&prepared, MISMATCH_HASH);
    let (decision, submit_count) = recover_with_provider(
        RecoveryReceiptMode::Pending,
        7,
        RecoveryOccupancyMode::Unknown,
        &reconstructed,
    )
    .await;

    let evidence = assert_hash_mismatch_ambiguity(decision, &submit_count, 0);
    assert_eq!(evidence.transactions[0].transaction_hash, MISMATCH_HASH);
}

#[tokio::test]
async fn fresh_submit_records_ambiguity_when_provider_returns_mismatched_hash() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let mismatched_hash = B256::from([0x11; 32]);
    let (decision, submit_count) =
        start_with_submit_hash_mismatch(mismatched_hash, &prepared).await;

    let evidence = assert_hash_mismatch_ambiguity(decision, &submit_count, 1);
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        expected_transaction_hash(&prepared)
    );
    assert_ne!(
        evidence.transactions[0].transaction_hash,
        format!("{mismatched_hash:?}")
    );
}

#[tokio::test]
async fn recovery_proves_not_submitted_when_foreign_transaction_occupies_nonce() {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let occupying_hash = "0x2222222222222222222222222222222222222222222222222222222222222222"
        .parse::<B256>()
        .expect("occupying hash");
    let (decision, submit_count) = recover_with_provider(
        RecoveryReceiptMode::Pending,
        8,
        RecoveryOccupancyMode::Occupied {
            transaction_hash: occupying_hash,
        },
        &prepared,
    )
    .await;

    let SideEffectSubmissionDecision::NotSubmitted(proof) = decision else {
        panic!("expected not-submitted proof");
    };
    assert_submit_count(&submit_count, 0);
    assert_eq!(
        proof.expected_transaction_hash,
        expected_transaction_hash(&prepared)
    );
    assert_eq!(
        proof.occupying_transaction_hash,
        "0x2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert_eq!(proof.nonce, prepared.evidence().transactions[0].nonce);
    assert_eq!(proof.evidence_chain_id, 1);
}

#[tokio::test]
async fn recovery_records_unknown_without_occupancy_proof_or_on_anchor_read_failure() {
    for (receipt_mode, pending_nonce) in [
        (RecoveryReceiptMode::Pending, 8),
        (RecoveryReceiptMode::ProviderFailure, 7),
    ] {
        assert_recovery_unknown(receipt_mode, pending_nonce).await;
    }
}

#[test]
fn adapter_source_has_no_concrete_store_or_signer_provider_coupling() {
    let source = include_str!("../lib.rs");
    for forbidden in [
        ["artifact", "_store", "_fs"].concat(),
        ["Fs", "Typed", "Artifact", "Store"].concat(),
        ["Key", "store"].concat(),
        ["Runtime", "Secret", "Source"].concat(),
        ["Signer", "Provider", "Runtime", "Config"].concat(),
        ["MFM", "_EVM", "_RPC"].concat(),
    ] {
        assert!(
            !source.contains(&forbidden),
            "adapter source contains forbidden coupling {forbidden}"
        );
    }
}

fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm-contracts",
        "evm-contract-lifecycle",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("executable identity template");
    factories
        .into_iter()
        .map(|factory| {
            let identity = executable_identities
                .executable(events::RunnerFactoryId::new(factory).expect("factory id"));
            format!(
                "factory={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                identity.factory_id,
                identity.cargo_package_digest,
                identity.binary_digest,
                identity.nix_derivation_hash.is_some(),
                identity.nix_output_hash.is_some()
            )
        })
        .collect()
}
