use std::sync::Arc;

use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};
use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};
use mfm_signing::SignerRef;
use mfm_transports_evm::EvmJsonRpcClient;
use serde::Deserialize;
use uuid::Uuid;

const ENV_EVM_SIGNERS_JSON: &str = "MFM_EVM_SIGNERS_JSON";

#[derive(Clone)]
struct EnvEvmContractRuntimeFactory {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl EnvEvmContractRuntimeFactory {
    fn new(artifacts: FsTypedArtifactStore) -> Self {
        Self {
            artifacts: Arc::new(artifacts),
        }
    }
}

impl mfm_adapters_evm_contracts::EvmContractRuntimeFactory for EnvEvmContractRuntimeFactory {
    fn artifacts(&self) -> &dyn ArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn runtime_for(
        &self,
        network_id: &str,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractRuntime> {
        let route = mfm_adapters_evm_contracts::EvmContractRuntimeRoute::new(
            EvmSourceRef::new(network_id).map_err(runtime_evm_capability_error)?,
            EvmSourcePolicyId::new(network_id).map_err(runtime_evm_capability_error)?,
        );
        let evm = Arc::new(EvmJsonRpcClient::from_env().map_err(runtime_evm_transport_error)?);
        let signer = Arc::new(keystore_signer_provider_from_env()?);
        Ok(mfm_adapters_evm_contracts::EvmContractRuntime::new(
            route, evm, signer,
        ))
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeSignerConfig {
    signer_ref: String,
    entry_id: Uuid,
    keystore_env: String,
    unlock_file_env: String,
}

fn keystore_signer_provider_from_env() -> mfm_runtime::Result<KeystoreSignerProvider> {
    let raw = std::env::var(ENV_EVM_SIGNERS_JSON).unwrap_or_else(|_| "[]".to_owned());
    let entries = serde_json::from_str::<Vec<RuntimeSignerConfig>>(&raw)
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?
        .into_iter()
        .map(|entry| {
            let signer_ref = SignerRef::new(entry.signer_ref).map_err(runtime_signing_error)?;
            KeystoreSignerRegistryEntry::from_env_sources(
                signer_ref,
                entry.entry_id,
                entry.keystore_env,
                entry.unlock_file_env,
            )
            .map_err(runtime_signing_error)
        })
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
    Ok(KeystoreSignerProvider::new(entries))
}

/// Registers contract lifecycle runners in the process production runner registry.
pub(crate) fn register_contract_lifecycle_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: FsTypedArtifactStore,
) -> mfm_runtime::Result<()> {
    mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
        registry,
        Arc::new(EnvEvmContractRuntimeFactory::new(artifacts)),
    )
}

fn runtime_evm_capability_error(
    error: mfm_evm_capabilities::EvmCapabilityError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_evm_transport_error(
    error: mfm_transports_evm::EvmTransportError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_signing_error(error: mfm_signing::SigningError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        make_async_typed_services_with_certification_registry, new_run_id,
        prepare_entry_point_run_launch, DriveMode, EntryPointRunLaunchInput, RunLaunchRequest,
        RunServices, TypedRunMode,
    };
    use mfm_adapters_evm_contracts::{
        ensure_prepared_invocation_public, EvmContractRuntime, EvmContractRuntimeFactory,
        EvmContractRuntimeRoute, PreparedContractInvocation,
    };
    use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
    use mfm_capabilities::CapabilitySpec;
    use mfm_certify::CertificationRegistry;
    use mfm_core::crypto::EthereumPrivateKey;
    use mfm_events::v1 as events;
    use mfm_evm_capabilities::{
        EvmCallReadCapability, EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse,
        EvmCapabilityError, EvmCapabilityFuture, EvmChainIdentityCapability,
        EvmChainIdentityProvider, EvmChainIdentityRequest, EvmChainIdentityResponse,
        EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateProvider,
        EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogEntry, EvmLogsReadCapability,
        EvmLogsReadProvider, EvmLogsReadRequest, EvmLogsReadResponse, EvmNonceReadProvider,
        EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider, EvmReceiptReadRequest,
        EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef, EvmTransactionSubmitProvider,
        EvmTransactionSubmitRequest, EvmTransactionSubmitResponse, RedactedEvmSourceEvidence,
    };
    use mfm_evm_contract_config::{ConfigurePhaseConfig, DeployPhaseConfig, ValidatePhaseConfig};
    use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
    use mfm_ids::RunId;
    use mfm_op_evm_contract_lifecycle::ContractLifecycleConfig;
    use mfm_runtime::ErasedRunnerRegistry;
    use mfm_signing::{
        PublicSigningIdentity, SignatureBytes, SignerRef, SigningError, SigningFuture,
        SigningProvider, SigningRequest, SigningResult,
    };
    use mfm_store::v1::{self as store, AsyncTypedRunEventStore};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    const TEST_SIGNER_HEX: &str =
        "4c0883a69102937d6231471b5dbb6204fe512961708279c2f802d6a8ebf2d3a4";

    fn evm_forbidden_runtime_terms() -> Vec<String> {
        vec![
            ["raw", "_transaction"].concat(),
            ["raw", "_tx"].concat(),
            ["signed", "_payload"].concat(),
            "signature".to_owned(),
            ["key", "store"].concat(),
            ["key", "store", "_path"].concat(),
            ["private", "_key"].concat(),
            ["private", " key"].concat(),
            ["pass", "word"].concat(),
            ["mne", "monic"].concat(),
            ["seed", "_phrase"].concat(),
            ["rpc", "_url"].concat(),
            ["rpc", " url"].concat(),
            ["end", "point"].concat(),
            ["provider", "_kind"].concat(),
            ["provider", " kind"].concat(),
            ["author", "ization"].concat(),
            TEST_SIGNER_HEX.to_owned(),
        ]
    }

    fn assert_no_evm_runtime_surface(label: &str, rendered: &str) {
        let rendered = rendered.to_ascii_lowercase();
        for forbidden in evm_forbidden_runtime_terms() {
            assert!(
                !rendered.contains(&forbidden),
                "{label} contains forbidden EVM runtime surface"
            );
        }
    }

    fn assert_prepared_invocation_has_unsigned_provenance(prepared: &PreparedContractInvocation) {
        assert!(!prepared.signer_ref.is_empty());
        assert!(prepared.expected_signer_address.starts_with("0x"));
        assert!(
            !prepared.transactions.is_empty(),
            "prepared invocation must retain transaction provenance"
        );
        for transaction in &prepared.transactions {
            assert!(transaction
                .data_digest
                .starts_with("content:sha256-jcs-v1:"));
            assert!(transaction.signing_digest.starts_with("0x"));
            assert_eq!(transaction.signing_digest.len(), 66);
            assert!(transaction.gas_limit > 0);
        }
    }

    fn prepare_evm_entry_point_request<S>(
        services: &RunServices<S>,
        op: &'static str,
        config: serde_json::Value,
        run_id: RunId,
    ) -> RunLaunchRequest
    where
        S: store::AsyncTypedRunEventStore + Send + Sync,
    {
        let entry_point_registry =
            crate::entry_points::production_entry_point_op_registry().expect("entry points");
        let public_op_name = crate::PublicOpName::new(op).expect("public op name");
        let authored_config =
            AuthoredConfig::from_json_transport_value(Some(AuthoredConfigFormat::Json), &config)
                .expect("authored config");

        prepare_entry_point_run_launch(EntryPointRunLaunchInput {
            entry_point_registry: &entry_point_registry,
            public_op_name,
            op_version: None,
            authored_config,
            certification_registry: services.certification_registry(),
            run_id,
            drive: DriveMode::AppendOnly,
        })
        .expect("entry-point launch request")
        .request
    }

    #[tokio::test]
    async fn app_runner_resumes_replays_and_renders_validate_only_lifecycle_run() {
        let root = std::env::temp_dir().join(format!(
            "mfm-evm-contract-validate-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let config = validate_config();
        let configured = configured_contract();
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::new(artifacts.clone())),
        )
        .expect("contract runners");
        let services = make_async_typed_services_with_certification_registry(
            runners,
            store::AsyncInMemoryTypedRunStore::default(),
            artifacts.clone(),
            certification,
        );
        let run_id = new_run_id();
        let request = prepare_evm_entry_point_request(
            &services,
            "evm_contract_validate",
            json!({
                "config": config,
                "configured": configured,
            }),
            run_id.clone(),
        );
        let public_schema_id = request
            .certified_spec
            .envelope()
            .spec
            .public_outputs
            .public_schema_id
            .clone();

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume validate lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let replay = services
            .verify_replay_for_run(&run_id)
            .await
            .expect("replay validate lifecycle");
        assert_eq!(
            replay.run_mode,
            TypedRunMode::Completed,
            "validated lifecycle replay should report a completed run"
        );
        let public_output = services
            .typed_public_output(&run_id, &public_schema_id)
            .await
            .expect("public output");
        let rendered = public_output.json.expect("json");
        assert!(
            rendered.to_string().contains("\"valid\":true"),
            "rendered validation output must contain a valid report: {rendered}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_runner_records_distinct_validation_capability_facts() {
        let root = std::env::temp_dir().join(format!(
            "mfm-evm-contract-validation-facts-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("root dir");
        let artifacts = FsTypedArtifactStore::new(&root);
        let configured = configured_contract();
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::new(artifacts.clone())),
        )
        .expect("contract runners");
        let services = make_async_typed_services_with_certification_registry(
            runners,
            store::AsyncInMemoryTypedRunStore::default(),
            artifacts.clone(),
            certification,
        );
        let run_id = new_run_id();
        let request = prepare_evm_entry_point_request(
            &services,
            "evm_contract_validate",
            json!({
                "config": validate_config_with_assertions(),
                "configured": configured,
            }),
            run_id.clone(),
        );

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume validate lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let stream = services
            .store()
            .load_run_stream(&run_id)
            .await
            .expect("load run stream");
        let fact_kinds = stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::FactRecorded(fact) => {
                    Some(fact.capability_kind.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            fact_kinds.len(),
            3,
            "validation with chain, call, and logs must record separate facts"
        );
        assert!(fact_kinds
            .contains(&EvmChainIdentityCapability::kind().expect("chain identity capability")));
        assert!(fact_kinds.contains(&EvmCallReadCapability::kind().expect("call capability")));
        assert!(fact_kinds.contains(&EvmLogsReadCapability::kind().expect("logs capability")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_runner_resumes_replays_and_renders_deploy_lifecycle_run() {
        let root =
            std::env::temp_dir().join(format!("mfm-evm-contract-deploy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root dir");
        let artifacts = FsTypedArtifactStore::new(&root);
        let signer = test_contract_signer();
        let config = deploy_config(&signer.address);
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::with_signer(
                artifacts.clone(),
                Arc::new(signer.provider),
                true,
            )),
        )
        .expect("contract runners");
        let services = make_async_typed_services_with_certification_registry(
            runners,
            store::AsyncInMemoryTypedRunStore::default(),
            artifacts.clone(),
            certification,
        );
        let run_id = new_run_id();
        let request = prepare_evm_entry_point_request(
            &services,
            "evm_contract_deploy",
            serde_json::to_value(config).expect("deploy config json"),
            run_id.clone(),
        );
        let public_schema_id = request
            .certified_spec
            .envelope()
            .spec
            .public_outputs
            .public_schema_id
            .clone();

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume deploy lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        let replay = services
            .verify_replay_for_run(&run_id)
            .await
            .expect("replay deploy lifecycle");
        assert_eq!(
            replay.run_mode,
            TypedRunMode::Completed,
            "deploy lifecycle replay should report a completed run"
        );
        let public_output = services
            .typed_public_output(&run_id, &public_schema_id)
            .await
            .expect("public output");
        let rendered = public_output.json.expect("json");
        assert!(
            rendered.to_string().contains("contract_address"),
            "rendered deploy output must contain deployed contract evidence: {rendered}"
        );
        assert!(
            rendered.to_string().contains("deploy_receipt_evidence"),
            "rendered deploy output must contain receipt evidence refs: {rendered}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_runner_resumes_replays_and_renders_full_lifecycle_run() {
        let root = std::env::temp_dir().join(format!(
            "mfm-evm-contract-lifecycle-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("root dir");
        let artifacts = FsTypedArtifactStore::new(&root);
        let signer = test_contract_signer();
        let config = ContractLifecycleConfig::new(
            deploy_config(&signer.address),
            configure_config(&signer.address),
            validate_config(),
        );
        let mut certification = CertificationRegistry::new();
        mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
            &mut certification,
        )
        .expect("contract certification descriptors");
        let mut runners = ErasedRunnerRegistry::new();
        mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
            &mut runners,
            Arc::new(TestRuntimeFactory::with_signer(
                artifacts.clone(),
                Arc::new(signer.provider),
                false,
            )),
        )
        .expect("contract runners");
        let services = make_async_typed_services_with_certification_registry(
            runners,
            store::AsyncInMemoryTypedRunStore::default(),
            artifacts.clone(),
            certification,
        );
        let run_id = new_run_id();
        let request = prepare_evm_entry_point_request(
            &services,
            "evm_contract_lifecycle",
            serde_json::to_value(config).expect("lifecycle config json"),
            run_id.clone(),
        );
        let public_schema_id = request
            .certified_spec
            .envelope()
            .spec
            .public_outputs
            .public_schema_id
            .clone();

        let started = services.launch_run(request).await.expect("append start");
        assert_eq!(started.run_mode, TypedRunMode::Forward);
        let resumed = services
            .resume_stored_run(&run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume full lifecycle");
        assert_eq!(resumed.run_mode, TypedRunMode::Completed);
        services
            .verify_replay_for_run(&run_id)
            .await
            .expect("replay full lifecycle");
        let public_output = services
            .typed_public_output(&run_id, &public_schema_id)
            .await
            .expect("public output");
        let rendered = public_output.json.expect("json");
        assert!(
            rendered.to_string().contains("\"valid\":true"),
            "rendered lifecycle output must contain a valid validation report: {rendered}"
        );
        assert!(
            rendered.to_string().contains("configure_tx_hashes"),
            "rendered lifecycle output must contain configure transaction hashes: {rendered}"
        );
        assert!(
            rendered.to_string().contains("configure_receipt_evidence"),
            "rendered lifecycle output must contain configure receipt evidence refs: {rendered}"
        );
        assert_no_evm_runtime_surface("contract public output", &rendered.to_string());

        let stream = services
            .store()
            .load_run_stream(&run_id)
            .await
            .expect("load run stream");
        assert_eq!(
            contract_lifecycle_runner_output_summary(&stream),
            [
                "attempt-output:mfm.evm.contract/deploy:side_effect.intent_persisted+side_effect.claimed+side_effect.invocation_prepared+side_effect.invocation_started+retention_refs_appended[roles=side_effect_intent]+retention_refs_appended[roles=prepared_invocation]",
                "attempt-output:mfm.evm.contract/deploy:side_effect.submission_observed+retention_refs_appended[roles=submission]",
                "attempt-output:mfm.evm.contract/deploy:side_effect.receipt_observed+retention_refs_appended[roles=receipt]",
                "attempt-output:mfm.evm.contract/deploy:side_effect.confirmation_observed+retention_refs_appended[roles=confirmation]",
                "attempt-output:mfm.evm.contract/deploy:cell_produced+state_attempt_completed+artifact_referenced[role=state_output]+retention_refs_appended[roles=state_output]",
                "attempt-output:mfm.evm.contract/configure:side_effect.intent_persisted+side_effect.claimed+side_effect.invocation_prepared+side_effect.invocation_started+retention_refs_appended[roles=side_effect_intent]+retention_refs_appended[roles=prepared_invocation]",
                "attempt-output:mfm.evm.contract/configure:side_effect.submission_observed+retention_refs_appended[roles=submission]",
                "attempt-output:mfm.evm.contract/configure:side_effect.receipt_observed+retention_refs_appended[roles=receipt]",
                "attempt-output:mfm.evm.contract/configure:side_effect.confirmation_observed+retention_refs_appended[roles=confirmation]",
                "attempt-output:mfm.evm.contract/configure:cell_produced+state_attempt_completed+artifact_referenced[role=state_output]+retention_refs_appended[roles=state_output]",
                "attempt-output:mfm.evm.contract/validate:fact_recorded+cell_produced+state_attempt_completed+artifact_referenced[role=state_output]+artifact_referenced[role=fact_response]+retention_refs_appended[roles=fact_response]+retention_refs_appended[roles=state_output]",
            ]
        );
        let mut prepared_artifact_ids = Vec::new();
        for event in &stream {
            let payload_debug = format!("{:?}", event.payload());
            assert_no_evm_runtime_surface("contract event payload", &payload_debug);
            if let events::KernelEventPayload::SideEffectInvocationPrepared(payload) =
                event.payload()
            {
                prepared_artifact_ids.push(
                    payload
                        .prepared_artifact_id
                        .clone()
                        .expect("prepared invocation event has artifact id"),
                );
            }
        }
        assert_eq!(
            prepared_artifact_ids.len(),
            2,
            "full lifecycle should prepare deploy and configure side effects"
        );

        for artifact_id in prepared_artifact_ids {
            let (bytes, evidence) = services
                .artifacts()
                .get_artifact_by_id(&artifact_id)
                .await
                .expect("prepared invocation artifact");
            assert_eq!(
                evidence.artifact_role,
                events::ArtifactRole::PreparedInvocation
            );
            assert_eq!(evidence.schema_id, None);
            assert_eq!(evidence.semantic_type_id, None);
            let rendered =
                std::str::from_utf8(&bytes).expect("prepared invocation artifact is UTF-8");
            assert_no_evm_runtime_surface("prepared invocation artifact", rendered);
            let prepared = serde_json::from_slice::<PreparedContractInvocation>(&bytes)
                .expect("prepared invocation json");
            ensure_prepared_invocation_public(&prepared).expect("prepared invocation is public");
            assert_prepared_invocation_has_unsigned_provenance(&prepared);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[derive(Clone)]
    struct TestRuntimeFactory {
        artifacts: Arc<dyn ArtifactReadProvider>,
        runtime: EvmContractRuntime,
    }

    impl TestRuntimeFactory {
        fn new(artifacts: FsTypedArtifactStore) -> Self {
            let source_ref = EvmSourceRef::new("reth-dev").expect("source ref");
            let policy_id = EvmSourcePolicyId::new("reth-dev").expect("policy id");
            let reads = Arc::new(Mutex::new(TestEvmReads::default()));
            let evm = Arc::new(TestEvmProvider {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                chain_id: 31337,
                mutation: false,
                fail_repeated_prepare_reads: false,
                reads: Arc::clone(&reads),
            });
            Self {
                artifacts: Arc::new(artifacts),
                runtime: EvmContractRuntime::new(
                    EvmContractRuntimeRoute::new(source_ref, policy_id),
                    evm,
                    Arc::new(TestSigner),
                ),
            }
        }

        fn with_signer(
            artifacts: FsTypedArtifactStore,
            signer: Arc<dyn SigningProvider>,
            fail_repeated_prepare_reads: bool,
        ) -> Self {
            let source_ref = EvmSourceRef::new("reth-dev").expect("source ref");
            let policy_id = EvmSourcePolicyId::new("reth-dev").expect("policy id");
            let reads = Arc::new(Mutex::new(TestEvmReads::default()));
            let evm = Arc::new(TestEvmProvider {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                chain_id: 31337,
                mutation: true,
                fail_repeated_prepare_reads,
                reads: Arc::clone(&reads),
            });
            Self {
                artifacts: Arc::new(artifacts),
                runtime: EvmContractRuntime::new(
                    EvmContractRuntimeRoute::new(source_ref, policy_id),
                    evm,
                    signer,
                ),
            }
        }
    }

    impl EvmContractRuntimeFactory for TestRuntimeFactory {
        fn artifacts(&self) -> &dyn ArtifactReadProvider {
            self.artifacts.as_ref()
        }

        fn runtime_for(&self, _network_id: &str) -> mfm_runtime::Result<EvmContractRuntime> {
            Ok(self.runtime.clone())
        }
    }

    struct TestSigner;

    impl SigningProvider for TestSigner {
        fn sign<'a>(&'a self, _request: &'a SigningRequest) -> SigningFuture<'a> {
            Box::pin(async { Err(SigningError::redacted_provider_failure("test signer")) })
        }
    }

    #[derive(Clone)]
    struct TestEvmProvider {
        source_ref: EvmSourceRef,
        policy_id: EvmSourcePolicyId,
        chain_id: u64,
        mutation: bool,
        fail_repeated_prepare_reads: bool,
        reads: Arc<Mutex<TestEvmReads>>,
    }

    #[derive(Default)]
    struct TestEvmReads {
        nonce: u32,
        fee: u32,
        gas: u32,
        receipt: u32,
    }

    impl TestEvmProvider {
        fn evidence(&self) -> RedactedEvmSourceEvidence {
            RedactedEvmSourceEvidence {
                source_ref: self.source_ref.clone(),
                policy_id: self.policy_id.clone(),
                chain_id: self.chain_id,
            }
        }

        fn record_prepare_read(&self, kind: TestPrepareReadKind) -> Result<(), EvmCapabilityError> {
            if !self.fail_repeated_prepare_reads {
                return Ok(());
            }
            let mut reads = self
                .reads
                .lock()
                .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
            let count = match kind {
                TestPrepareReadKind::Nonce => &mut reads.nonce,
                TestPrepareReadKind::Fee => &mut reads.fee,
                TestPrepareReadKind::Gas => &mut reads.gas,
            };
            *count += 1;
            if *count > 1 {
                Err(EvmCapabilityError::redacted_provider_failure("test evm"))
            } else {
                Ok(())
            }
        }
    }

    enum TestPrepareReadKind {
        Nonce,
        Fee,
        Gas,
    }

    impl EvmChainIdentityProvider for TestEvmProvider {
        fn chain_identity<'a>(
            &'a self,
            _request: &'a EvmChainIdentityRequest,
        ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
            Box::pin(async move {
                Ok(EvmChainIdentityResponse {
                    evidence: self.evidence(),
                    chain_id: self.chain_id,
                    client_version: Some("mfm-test-evm".to_owned()),
                })
            })
        }
    }

    impl EvmNonceReadProvider for TestEvmProvider {
        fn read_nonce<'a>(
            &'a self,
            _request: &'a EvmNonceReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                self.record_prepare_read(TestPrepareReadKind::Nonce)?;
                Ok(EvmNonceReadResponse {
                    evidence: self.evidence(),
                    nonce: 7,
                })
            })
        }
    }

    impl EvmFeeReadProvider for TestEvmProvider {
        fn read_fee<'a>(
            &'a self,
            _request: &'a EvmFeeReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                self.record_prepare_read(TestPrepareReadKind::Fee)?;
                Ok(EvmFeeReadResponse {
                    evidence: self.evidence(),
                    base_fee_per_gas: Some(5),
                    priority_fee_per_gas: Some(3),
                    max_fee_per_gas: Some(11),
                    legacy_gas_price: Some(7),
                })
            })
        }
    }

    impl EvmGasEstimateProvider for TestEvmProvider {
        fn estimate_gas<'a>(
            &'a self,
            _request: &'a EvmGasEstimateRequest,
        ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                self.record_prepare_read(TestPrepareReadKind::Gas)?;
                Ok(EvmGasEstimateResponse {
                    evidence: self.evidence(),
                    gas_limit: 21_000,
                })
            })
        }
    }

    impl EvmTransactionSubmitProvider for TestEvmProvider {
        fn submit_transaction<'a>(
            &'a self,
            request: &'a EvmTransactionSubmitRequest,
        ) -> EvmCapabilityFuture<'a, EvmTransactionSubmitResponse> {
            if !self.mutation {
                return failed_evm();
            }
            Box::pin(async move {
                Ok(EvmTransactionSubmitResponse {
                    evidence: self.evidence(),
                    transaction_hash: request.signed_payload.transaction_hash(),
                })
            })
        }
    }

    impl EvmReceiptReadProvider for TestEvmProvider {
        fn read_receipt<'a>(
            &'a self,
            request: &'a EvmReceiptReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
            let reads = Arc::clone(&self.reads);
            if !self.mutation {
                return Box::pin(async move {
                    let mut reads = reads
                        .lock()
                        .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                    reads.receipt += 1;
                    Err(EvmCapabilityError::redacted_provider_failure("test evm"))
                });
            }
            Box::pin(async move {
                let mut reads = self
                    .reads
                    .lock()
                    .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                reads.receipt += 1;
                Ok(EvmReceiptReadResponse {
                    evidence: self.evidence(),
                    transaction_hash: request.transaction_hash,
                    block_number: 42,
                    status: true,
                })
            })
        }
    }

    impl EvmCallReadProvider for TestEvmProvider {
        fn read_call<'a>(
            &'a self,
            _request: &'a EvmCallReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
            Box::pin(async move {
                let mut return_data = vec![0_u8; 32];
                return_data[31] = 1;
                Ok(EvmCallReadResponse {
                    evidence: self.evidence(),
                    return_data,
                })
            })
        }
    }

    impl EvmLogsReadProvider for TestEvmProvider {
        fn read_logs<'a>(
            &'a self,
            request: &'a EvmLogsReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
            Box::pin(async move {
                Ok(EvmLogsReadResponse {
                    evidence: self.evidence(),
                    logs: vec![EvmLogEntry {
                        address: request.address.expect("validation log address"),
                        topics: request.topics.clone(),
                        data: Vec::new(),
                        block_number: Some(42),
                        transaction_hash: None,
                        log_index: Some(0),
                    }],
                })
            })
        }
    }

    fn failed_evm<'a, T>() -> EvmCapabilityFuture<'a, T> {
        Box::pin(async { Err(EvmCapabilityError::redacted_provider_failure("test evm")) })
    }

    struct TestSignerRuntime {
        provider: LocalTestSigner,
        address: String,
    }

    fn test_contract_signer() -> TestSignerRuntime {
        let signer_ref = SignerRef::new("deployer").expect("signer ref");
        let material = EthereumPrivateKey::from_hex_secret(TEST_SIGNER_HEX).expect("signer");
        let address = format!("{:?}", material.address().expect("address"));
        let provider = LocalTestSigner {
            signer_ref,
            address: address.clone(),
            material,
        };
        TestSignerRuntime { provider, address }
    }

    struct LocalTestSigner {
        signer_ref: SignerRef,
        address: String,
        material: EthereumPrivateKey,
    }

    impl SigningProvider for LocalTestSigner {
        fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
            let result = self.sign_request(request);
            Box::pin(async move { result })
        }
    }

    impl LocalTestSigner {
        fn sign_request(&self, request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
            if request.signer_ref() != &self.signer_ref {
                return Err(SigningError::redacted_provider_failure("test signer"));
            }
            let digest = request.digest().as_bytes();
            let signature = self
                .material
                .sign_hash_recoverable(digest)
                .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
            let identity = PublicSigningIdentity::new(
                request.algorithm().clone(),
                None,
                Some(self.address.clone()),
            )?;
            let signature = SignatureBytes::new(signature.as_bytes().to_vec())?;
            SigningResult::for_request(request, identity, signature)
        }
    }

    fn validate_config() -> ValidatePhaseConfig {
        serde_json::from_value(json!({
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            }
        }))
        .expect("validate config")
    }

    fn validate_config_with_assertions() -> ValidatePhaseConfig {
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            },
            "validation": {
                "read_assertions": [
                    {
                        "function": "ready",
                        "args": [],
                        "expected": {
                            "json_text": "true"
                        }
                    }
                ],
                "event_assertions": [
                    {
                        "event": "Configured",
                        "min_count": 1
                    }
                ]
            }
        }))
        .expect("validate config with assertions")
    }

    fn deploy_config(expected_signer_address: &str) -> DeployPhaseConfig {
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            },
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_signer_address
            },
            "transaction": {
                "style": "eip1559",
                "max_fee_per_gas": "11",
                "max_priority_fee_per_gas": "3"
            }
        }))
        .expect("deploy config")
    }

    fn configure_config(expected_signer_address: &str) -> ConfigurePhaseConfig {
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337
            },
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_signer_address
            },
            "calls": [
                {
                    "function": "configure",
                    "args": []
                },
                {
                    "function": "configure",
                    "args": []
                }
            ]
        }))
        .expect("configure config")
    }

    fn artifact_json() -> serde_json::Value {
        json!({
            "abi": {
                "json_text": json!([
                    {
                        "type": "constructor",
                        "inputs": []
                    },
                    {
                        "type": "function",
                        "name": "configure",
                        "inputs": [],
                        "outputs": [],
                        "stateMutability": "nonpayable"
                    },
                    {
                        "type": "function",
                        "name": "ready",
                        "inputs": [],
                        "outputs": [
                            {
                                "name": "",
                                "type": "bool"
                            }
                        ],
                        "stateMutability": "view"
                    },
                    {
                        "type": "event",
                        "name": "Configured",
                        "inputs": [],
                        "anonymous": false
                    }
                ]).to_string()
            },
            "bytecode": {
                "json_text": json!({"object": "0x6000"}).to_string()
            }
        })
    }

    fn configured_contract() -> ConfiguredContract {
        ConfiguredContract {
            lifecycle_version: 1,
            deployed: DeployedContract {
                lifecycle_version: 1,
                network_id: "reth-dev".to_owned(),
                expected_chain_id: 31337,
                contract_address: "0x000000000000000000000000000000000000dead".to_owned(),
                deploy_tx_hash: "0x01".to_owned(),
                deploy_receipt_evidence: None,
                deployed_block_number: Some(1),
            },
            configure_calls: Vec::new(),
            confirmation_read_assertions: Vec::new(),
            confirmation_event_assertions: Vec::new(),
            configure_tx_hashes: Vec::new(),
            configure_receipt_evidence: Vec::new(),
            configured_block_number: Some(1),
        }
    }

    fn contract_lifecycle_runner_output_summary(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<String> {
        attempt_output_commit_summaries(stream, "mfm.evm.contract/")
    }

    fn attempt_output_commit_summaries(
        stream: &[store::KernelEventEnvelope],
        state_kind_prefix: &str,
    ) -> Vec<String> {
        let state_kinds_by_node = state_kinds_by_node(stream);
        let mut summaries = Vec::new();
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == commit_key
            {
                end += 1;
            }
            if commit_key.as_str().starts_with("attempt-output:") {
                let state_kind = runner_output_node_id(&stream[index..end])
                    .and_then(|node_id| state_kinds_by_node.get(&node_id))
                    .map(String::as_str)
                    .unwrap_or("unknown");
                if state_kind.starts_with(state_kind_prefix) {
                    let payloads = stream[index..end]
                        .iter()
                        .map(|event| runner_payload_summary(event.payload()))
                        .collect::<Vec<_>>()
                        .join("+");
                    summaries.push(format!(
                        "{}:{state_kind}:{payloads}",
                        commit_key_class(commit_key.as_str())
                    ));
                }
            }
            index = end;
        }
        summaries
    }

    fn state_kinds_by_node(stream: &[store::KernelEventEnvelope]) -> BTreeMap<String, String> {
        stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::StateAttemptStarted(payload) => Some((
                    payload.node_id.as_str().to_owned(),
                    payload
                        .state_kind
                        .canonical_name()
                        .unwrap_or_else(|| payload.state_kind.as_str())
                        .to_owned(),
                )),
                _ => None,
            })
            .collect()
    }

    fn runner_output_node_id(events: &[store::KernelEventEnvelope]) -> Option<String> {
        events.iter().find_map(|event| match event.payload() {
            events::KernelEventPayload::FactRecorded(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::CellProduced(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectClaimed(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectClaimTakenOver(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                Some(payload.node_id.as_str().to_owned())
            }
            _ => None,
        })
    }

    fn runner_payload_summary(payload: &events::KernelEventPayload) -> String {
        match payload {
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                format!(
                    "artifact_referenced[role={}]",
                    payload.artifact_ref.role.as_str()
                )
            }
            events::KernelEventPayload::RetentionRefsAppended(payload) => {
                let roles = payload
                    .refs
                    .iter()
                    .map(|retention| retention.role.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("retention_refs_appended[roles={roles}]")
            }
            _ => payload_schema_name(payload).to_owned(),
        }
    }

    fn payload_schema_name(payload: &events::KernelEventPayload) -> &str {
        payload
            .schema_descriptor()
            .schema_name
            .strip_prefix("mfm.events.v1.")
            .unwrap_or_else(|| payload.schema_descriptor().schema_name)
    }

    fn commit_key_class(commit_key: &str) -> &str {
        if commit_key.starts_with("attempt-output:") {
            "attempt-output"
        } else {
            commit_key
        }
    }
}
