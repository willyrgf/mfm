use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    ComponentFuture, ReadAdapterCompletion, ReadAdapterInvoker, ReadCapabilityContract,
};
use mfm_certify::structured::{
    CertifiedAccessAuthorization, PhysicalBindingSelection, ProgramRegistryBuilder,
    QualifiedReadPhysicalBinding, QualifiedReadPhysicalBindingSource,
};
use mfm_evm::{
    balance_adapter_contract, register_evm_balance_process,
    structured_evm_balance_collection_program, ChainInstanceDeclaration,
    ChainInstanceRegistryAttestation, EvmAnchorConfirmationRequest, EvmBalanceAsset,
    EvmBalanceLaneInput, EvmBalanceSource, EvmBlockAnchor, EvmBlockResponse,
    EvmChainIdentityCapability, EvmChainIdentityRequest, EvmChainIdentityResponse,
    EvmConfirmAnchorCapability, EvmLatestAnchorCapability, EvmLatestAnchorRequest,
    EvmNativeBalanceCapability, EvmNativeBalanceRequest, EvmNetworkBinding, EvmQuantityResponse,
    EvmReadFailure, EvmRoutingGenerationRef, EvmSubmissionProcessQualification,
    EvmTokenBalanceCapability, EvmTokenBalanceRequest, EvmTokenDecimalsCapability,
    EvmTokenDecimalsRequest, EvmTokenDecimalsResponse, EvmWalletReference,
    STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID,
};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, InvocationIdentity, SchemaId,
    StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    HistoryObject, PriorRunFactSourceManifest, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_portfolio::{
    decode_portfolio_config, register_portfolio_process, structured_portfolio_lane_inputs,
    structured_portfolio_snapshot_program, AnchoredHoldingSource, EvmRoutingBinding,
    HoldingSourceConfig, NetworkPin, Observation, ObservationQuantity, ObservationValue,
    PortfolioId, PortfolioProcessQualification, PortfolioPublicOutputs, PortfolioQuoteTotal,
    PortfolioReport, PortfolioRoutingManifest, PortfolioSnapshot, PortfolioSnapshotInput,
    PortfolioSnapshotSelector, QuoteCode, WalletReport, WalletSnapshot,
    STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID,
};
use mfm_program::structured::{RuntimeReadAdapter, RuntimeReadCapability};
use mfm_runtime::history::StructuredAdmissionCommand;
use mfm_runtime::structured::DriveOutcome;
use mfm_spec::structured::{
    AuthoredBlock, AuthoredDeclaration, ExpandedBlock, ExpandedDeclaration,
    SecretFreeExecutableIdentity, SecretFreeImplementationDescriptor,
    SecretFreeQualificationArtifact, StructuredComponentKind, StructuredExpansionProfile,
};
use mfm_store::structured::{
    assemble_in_memory_runtime, PhysicalBindingAuthorization, PhysicalBindingSupersession,
    PhysicalTargetIdentity, ProposedCanonicalValue, PublicPhysicalBindingVerifier,
    StructuredAdmissionMaterial, StructuredStoreError, StructuredStoreIdentity,
};
use mfm_values::CanonicalJsonPersistedSchema;
use serde_json::json;

const EXECUTABLE_ID: &str = "mfm.portfolio.test/structured-executable";
const QUALIFICATION_ID: &str = "mfm.portfolio.test/structured-qualification";

trait TestBalanceCapability:
    RuntimeReadCapability<SafeFailure = EvmReadFailure> + ReadCapabilityContract
{
    fn adapter_id() -> &'static str;
    fn returned(request: &Self::Request, calls: &TestBalanceCalls) -> Self::Returned;
}

#[derive(Default)]
struct TestBalanceCalls {
    chain_identity: AtomicUsize,
    latest_anchor: AtomicUsize,
    token_decimals: AtomicUsize,
    native_balance: AtomicUsize,
    token_balance: AtomicUsize,
    confirm_anchor: AtomicUsize,
}

impl TestBalanceCalls {
    fn snapshot(&self) -> [usize; 6] {
        [
            self.chain_identity.load(Ordering::SeqCst),
            self.latest_anchor.load(Ordering::SeqCst),
            self.token_decimals.load(Ordering::SeqCst),
            self.native_balance.load(Ordering::SeqCst),
            self.token_balance.load(Ordering::SeqCst),
            self.confirm_anchor.load(Ordering::SeqCst),
        ]
    }
}

impl TestBalanceCapability for EvmChainIdentityCapability {
    fn adapter_id() -> &'static str {
        "mfm.evm.adapter/chain-identity"
    }

    fn returned(
        request: &EvmChainIdentityRequest,
        calls: &TestBalanceCalls,
    ) -> EvmChainIdentityResponse {
        calls.chain_identity.fetch_add(1, Ordering::SeqCst);
        EvmChainIdentityResponse {
            chain_id: request.binding().chain_id(),
            source_scope: format!("fixture-{}", request.binding().network_id()),
            implementation_id: "mfm-portfolio-test-balance-provider".to_owned(),
        }
    }
}

impl TestBalanceCapability for EvmLatestAnchorCapability {
    fn adapter_id() -> &'static str {
        "mfm.evm.adapter/latest-anchor"
    }

    fn returned(request: &EvmLatestAnchorRequest, calls: &TestBalanceCalls) -> EvmBlockResponse {
        calls.latest_anchor.fetch_add(1, Ordering::SeqCst);
        EvmBlockResponse {
            anchor: test_anchor(request.source().binding().chain_id()),
        }
    }
}

impl TestBalanceCapability for EvmTokenDecimalsCapability {
    fn adapter_id() -> &'static str {
        "mfm.evm.adapter/token-decimals"
    }

    fn returned(
        _request: &EvmTokenDecimalsRequest,
        calls: &TestBalanceCalls,
    ) -> EvmTokenDecimalsResponse {
        calls.token_decimals.fetch_add(1, Ordering::SeqCst);
        EvmTokenDecimalsResponse { decimals: 6 }
    }
}

impl TestBalanceCapability for EvmNativeBalanceCapability {
    fn adapter_id() -> &'static str {
        "mfm.evm.adapter/native-balance"
    }

    fn returned(
        request: &EvmNativeBalanceRequest,
        calls: &TestBalanceCalls,
    ) -> EvmQuantityResponse {
        calls.native_balance.fetch_add(1, Ordering::SeqCst);
        EvmQuantityResponse::new(
            U256::from(request.source().source().binding().chain_id())
                * U256::from(1_000_000_000_000_000_000_u128),
        )
    }
}

impl TestBalanceCapability for EvmTokenBalanceCapability {
    fn adapter_id() -> &'static str {
        "mfm.evm.adapter/token-balance"
    }

    fn returned(request: &EvmTokenBalanceRequest, calls: &TestBalanceCalls) -> EvmQuantityResponse {
        calls.token_balance.fetch_add(1, Ordering::SeqCst);
        EvmQuantityResponse::new(
            U256::from(request.source().source().binding().chain_id()) * U256::from(1_000_000_u64),
        )
    }
}

impl TestBalanceCapability for EvmConfirmAnchorCapability {
    fn adapter_id() -> &'static str {
        "mfm.evm.adapter/confirm-anchor"
    }

    fn returned(
        request: &EvmAnchorConfirmationRequest,
        calls: &TestBalanceCalls,
    ) -> EvmBlockResponse {
        calls.confirm_anchor.fetch_add(1, Ordering::SeqCst);
        EvmBlockResponse {
            anchor: request
                .source()
                .expect("valid structured confirmation request")
                .anchor()
                .clone(),
        }
    }
}

fn test_anchor(chain_id: u64) -> EvmBlockAnchor {
    EvmBlockAnchor::new(
        U256::from(chain_id * 100),
        B256::repeat_byte(u8::try_from(chain_id).expect("fixture chain byte")),
    )
}

struct TestBinding<C> {
    certificate: HistoryObject,
    calls: Arc<TestBalanceCalls>,
    capability: PhantomData<fn() -> C>,
}

impl<C> ReadAdapterInvoker<C> for TestBinding<C>
where
    C: TestBalanceCapability,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        let returned = C::returned(request, &self.calls);
        Box::pin(async move { ReadAdapterCompletion::Returned(returned) })
    }
}

impl<C> RuntimeReadAdapter<C> for TestBinding<C>
where
    C: TestBalanceCapability,
{
    fn contract() -> mfm_program::Result<mfm_spec::structured::StructuredLiveComponentContract> {
        balance_adapter_contract(C::adapter_id())
    }
}

impl<C> QualifiedReadPhysicalBinding<C> for TestBinding<C>
where
    C: TestBalanceCapability,
{
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }

    fn invoke_authorized<'a>(
        &'a self,
        request: &'a C::Request,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        drop(authorization);
        self.invoke(request)
    }
}

struct TestBindingSource<C> {
    certificate: HistoryObject,
    calls: Arc<TestBalanceCalls>,
    capability: PhantomData<fn() -> C>,
}

impl<C> QualifiedReadPhysicalBindingSource<C> for TestBindingSource<C>
where
    C: TestBalanceCapability,
{
    type Binding = TestBinding<C>;

    fn current_binding<'a>(
        &'a self,
        _selection: PhysicalBindingSelection<'a>,
        _request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = Arc::new(TestBinding {
            certificate: self.certificate.clone(),
            calls: Arc::clone(&self.calls),
            capability: PhantomData,
        });
        Box::pin(async move { Some(binding) })
    }
}

#[test]
fn portfolio_and_balance_certification_is_depth_two_and_order_stable() {
    let lanes = lane_inputs();
    let forward = certified_programs(&lanes, false);
    let reverse = certified_programs(&lanes, true);

    assert_eq!(forward.portfolio.document(), reverse.portfolio.document());
    assert_eq!(forward.balance.document(), reverse.balance.document());
    assert_eq!(forward.portfolio.reference(), reverse.portfolio.reference());
    assert_eq!(forward.balance.reference(), reverse.balance.reference());

    let authored_shape = authored_shape(&lanes);
    assert_eq!(authored_shape.fan_outs, 3);
    assert_eq!(authored_shape.child_calls, 0);

    let portfolio_shape = expanded_shape(&forward.portfolio.expanded().root);
    assert_eq!(portfolio_shape.fan_outs, 3);
    assert_eq!(portfolio_shape.fragments, 0);
    assert_eq!(portfolio_shape.maximum_fan_out_depth, 2);

    let balance_shape = expanded_shape(&forward.balance.expanded().root);
    assert_eq!(balance_shape.fan_outs, 1);
    assert_eq!(balance_shape.fragments, 0);
    assert_eq!(balance_shape.maximum_fan_out_depth, 1);

    let portfolio_ref = forward.portfolio.reference().expect("portfolio reference");
    let balance_ref = forward.balance.reference().expect("balance reference");
    assert_eq!(
        portfolio_ref.content_digest().as_str(),
        "content:sha256-v1:646640e1a68c699865fa6a620a70078bac3c749ec3662d8da2b0f05b137b5ac4"
    );
    assert_eq!(
        balance_ref.content_digest().as_str(),
        "content:sha256-v1:6e2c5ca83101b821fcfd07520dfba8840431f8d8e97fd516f3aadcfc51992c51"
    );

    let registry = forward.verification;
    assert_eq!(
        registry
            .verify(
                &stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID),
                forward.portfolio.document(),
            )
            .expect("persisted portfolio verification")
            .reference()
            .expect("verified portfolio reference"),
        portfolio_ref,
    );
    assert_eq!(
        registry
            .verify(
                &stable(STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID),
                forward.balance.document(),
            )
            .expect("persisted balance verification")
            .reference()
            .expect("verified balance reference"),
        balance_ref,
    );
}

#[test]
fn one_nominal_input_signature_accepts_smaller_topology_subsets() {
    let lanes = lane_inputs();
    let registered = certified_programs(&lanes, false);

    let portfolio_id = stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID);
    let smaller_portfolio = structured_portfolio_snapshot_program(
        portfolio_id.clone(),
        stable("mfm.portfolio.test/portfolio-scope"),
        &lanes[..1],
    )
    .expect("smaller portfolio candidate");
    assert_eq!(smaller_portfolio.input_roots.len(), 1);
    let smaller_portfolio = registered
        .certification
        .certify(&portfolio_id, smaller_portfolio)
        .expect("portfolio support-envelope subset");

    let balance_id = stable(STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID);
    let smaller_balance = structured_evm_balance_collection_program(
        balance_id.clone(),
        stable("mfm.portfolio.test/balance-scope"),
        &lanes[..1],
    )
    .expect("smaller balance candidate");
    assert_eq!(smaller_balance.input_roots.len(), 1);
    let smaller_balance = registered
        .certification
        .certify(&balance_id, smaller_balance)
        .expect("balance support-envelope subset");

    registered
        .verification
        .verify(&portfolio_id, smaller_portfolio.document())
        .expect("persisted smaller portfolio");
    registered
        .verification
        .verify(&balance_id, smaller_balance.document())
        .expect("persisted smaller balance");
}

#[tokio::test]
async fn structured_runtime_executes_the_complete_portfolio_topology_matrix() {
    const NATIVE_ONLY: &[FixtureAssets] = &[FixtureAssets::Native];
    const TOKEN_ONLY: &[FixtureAssets] = &[FixtureAssets::Token];
    const MIXED: &[FixtureAssets] = &[FixtureAssets::Mixed];
    // Two networks cover both asset kinds without multiplying every balance lane.
    const MULTI_NETWORK: &[FixtureAssets] = &[FixtureAssets::Native, FixtureAssets::Token];

    for (case_index, (label, assets)) in [
        ("native-only", NATIVE_ONLY),
        ("token-only", TOKEN_ONLY),
        ("mixed", MIXED),
        ("multiple-network", MULTI_NETWORK),
    ]
    .into_iter()
    .enumerate()
    {
        execute_portfolio_case(
            u8::try_from(case_index + 70).expect("fixture discriminator"),
            label,
            assets,
        )
        .await;
    }
}

#[derive(Clone, Copy)]
enum FixtureAssets {
    Native,
    Token,
    Mixed,
}

impl FixtureAssets {
    const fn has_native(self) -> bool {
        matches!(self, Self::Native | Self::Mixed)
    }

    const fn has_token(self) -> bool {
        matches!(self, Self::Token | Self::Mixed)
    }
}

async fn execute_portfolio_case(discriminator: u8, label: &str, assets: &[FixtureAssets]) {
    let (input, lanes) = portfolio_execution_input(label, assets);
    assert_eq!(authored_shape(&lanes).maximum_fan_out_depth, 2);

    let calls = Arc::new(TestBalanceCalls::default());
    let operation_id = stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID);
    let mut builder = ProgramRegistryBuilder::new();
    let executable = builder
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable(format!("{EXECUTABLE_ID}/{label}")),
        })
        .expect("execution executable identity");
    let qualification = builder
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable(format!("{QUALIFICATION_ID}/{label}")),
        })
        .expect("execution qualification artifact");
    let evm = EvmSubmissionProcessQualification::new(executable.clone(), qualification.clone());
    let portfolio = PortfolioProcessQualification::new(executable, qualification);
    register_evm_balance_process(&mut builder, &evm).expect("execution balance process");
    register_portfolio_process(&mut builder, &portfolio).expect("execution portfolio process");
    register_test_adapters(&mut builder, &evm, false, Arc::clone(&calls));
    let program_scope = stable(format!("mfm.portfolio.test/runtime-{label}"));
    let authored =
        structured_portfolio_snapshot_program(operation_id.clone(), program_scope.clone(), &lanes)
            .expect("execution portfolio program");
    let support_program =
        structured_portfolio_snapshot_program(operation_id.clone(), program_scope, &lane_inputs())
            .expect("execution portfolio support envelope");
    builder
        .register_entry_point(operation_id.clone(), support_program, profile(2))
        .expect("execution portfolio entry");
    let registry = builder
        .build(std::slice::from_ref(&operation_id))
        .expect("execution qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("execution certifier")
        .certify(authored)
        .expect("execution certified program")
        .into_document();
    let assembled = assemble_in_memory_runtime(
        StructuredStoreIdentity {
            store_scope_id: StoreScopeId::new(format!(
                "{}{:032x}",
                StoreScopeId::PREFIX,
                discriminator
            ))
            .expect("execution store scope"),
            store_epoch: StoreEpoch::new(1),
            physical_target: Some(PhysicalTargetIdentity {
                target_key: format!("portfolio-fixture-target-{discriminator}"),
                database_oid: u32::from(discriminator),
                fence_generation: 1,
                release_epoch: 1,
                current_incarnation_ref: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(&[discriminator, 6]),
                ),
            }),
        },
        registry,
        Arc::new(TestPublicBindingVerifier),
    )
    .expect("runtime assembly");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(StructuredAdmissionCommand::new(
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "4".repeat(32)))
                .expect("execution tenant"),
            InvocationIdentity::new(format!(
                "00000000-0000-4000-8000-0000000000{discriminator:02x}"
            ))
            .expect("execution invocation"),
            operation_id,
            document,
            test_admission_material(label),
            vec![ProposedCanonicalValue::from_value(&input).expect("execution input")],
            AppendRequestId::new(format!("portfolio-runtime-{label}-admit"))
                .expect("execution append id"),
        ))
        .await
        .expect("execution admission");

    for drive_index in 0..256 {
        match runtime.drive_once(&run_id).await.expect("execution drive") {
            DriveOutcome::TransitionCommitted { closed: true } => break,
            DriveOutcome::TransitionCommitted { closed: false }
            | DriveOutcome::AccessObserved
            | DriveOutcome::ConcurrentProgress => {}
            other => panic!("portfolio {label} drive {drive_index} returned {other:?}"),
        }
        assert!(drive_index < 255, "portfolio {label} did not close");
    }

    let verified = reader
        .load_public(&run_id)
        .await
        .expect("execution verified history");
    let outcome = mfm_replay::structured::project_operation_outcome(&verified)
        .expect("execution root outcome")
        .expect("execution closed root");
    assert_eq!(outcome.kind(), "success");
    let outputs: PortfolioPublicOutputs =
        serde_json::from_slice(outcome.canonical_value().as_bytes())
            .expect("execution portfolio output");
    assert_portfolio_outputs(label, assets, input.portfolio(), &lanes, &outputs);

    let native = lanes
        .iter()
        .filter(|lane| matches!(lane.source().asset(), EvmBalanceAsset::Native))
        .count();
    let token = lanes.len() - native;
    assert_eq!(
        calls.snapshot(),
        [lanes.len(), lanes.len(), token, native, token, lanes.len()],
        "portfolio {label} callback counts"
    );
}

struct TestPublicBindingVerifier;

impl mfm_authority_seal::PhysicalBindingVerifierSeal for TestPublicBindingVerifier {}

impl PublicPhysicalBindingVerifier for TestPublicBindingVerifier {
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        candidate: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        let accepted = [
            EvmChainIdentityCapability::adapter_id(),
            EvmLatestAnchorCapability::adapter_id(),
            EvmTokenDecimalsCapability::adapter_id(),
            EvmNativeBalanceCapability::adapter_id(),
            EvmTokenBalanceCapability::adapter_id(),
            EvmConfirmAnchorCapability::adapter_id(),
        ]
        .into_iter()
        .any(|adapter| candidate == &certificate(adapter));
        if accepted
            && context.stable_resource_lineage_contract_ref.is_none()
            && context.minimum_lineage_head_ref.is_none()
        {
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }
}

fn portfolio_execution_input(
    label: &str,
    assets: &[FixtureAssets],
) -> (PortfolioSnapshotInput, Vec<EvmBalanceLaneInput>) {
    let available = lane_inputs();
    let mut networks = Vec::with_capacity(assets.len());
    let mut wallets = Vec::with_capacity(assets.len());
    let mut symbols = Vec::new();
    let mut routes = Vec::with_capacity(assets.len());

    for (index, asset_set) in assets.iter().copied().enumerate() {
        let ordinal = u32::try_from(index).expect("fixture collection ordinal");
        let binding = available
            .iter()
            .find(|lane| lane.collection_ordinal() == ordinal)
            .expect("fixture network binding")
            .binding();
        let network_id = binding.network_id();
        let wallet_id = format!("wallet-{}", char::from(b'a' + u8::try_from(index).unwrap()));
        let native_symbol = format!("native.{network_id}");
        let token_symbol = format!("token.{network_id}");
        let mut symbol_ids = Vec::new();

        networks.push(json!({
            "network_id": network_id,
            "family": "evm",
            "chain_id": binding.chain_id(),
            "native_decimals": 18,
            "metadata": {},
        }));
        routes.push(
            EvmRoutingBinding::new(
                network_id,
                binding.chain_instance().clone(),
                binding.routing_generation_ref().clone(),
            )
            .expect("fixture routing binding"),
        );

        if asset_set.has_native() {
            symbol_ids.push(native_symbol.clone());
            symbols.push(json!({
                "symbol_id": native_symbol,
                "display_symbol": format!("NATIVE{index}"),
                "network_id": network_id,
                "source": {"kind": "native"},
                "valuation": {"quotes": [{
                    "quote": "USD",
                    "priced_symbol_id": format!("native.{network_id}"),
                    "unit_price_dec": "1.00",
                }]},
                "metadata": {},
            }));
        }
        if asset_set.has_token() {
            symbol_ids.push(token_symbol.clone());
            let token_address = format!("0x{:040x}", 0x22_u64 + u64::try_from(index).unwrap());
            symbols.push(json!({
                "symbol_id": token_symbol,
                "display_symbol": format!("TOKEN{index}"),
                "network_id": network_id,
                "source": {"kind": "erc20", "contract_address": token_address},
                "valuation": {"quotes": [{
                    "quote": "USD",
                    "priced_symbol_id": format!("token.{network_id}"),
                    "unit_price_dec": "1.00",
                }]},
                "metadata": {},
            }));
        }
        wallets.push(json!({
            "wallet_id": wallet_id,
            "subject": {
                "kind": "evm_address",
                "address": format!("0x{:040x}", 0x11_u64 + u64::try_from(index).unwrap()),
            },
            "network_id": network_id,
            "implementation": {"kind": "address_only"},
            "symbol_ids": symbol_ids,
            "metadata": {},
        }));
    }

    let portfolio_id = format!("portfolio_{}", label.replace('-', "_"));
    let portfolio = decode_portfolio_config(&json!({
        "portfolio_id": portfolio_id,
        "quote_codes": ["USD"],
        "networks": networks,
        "wallets": wallets,
        "symbol_configs": symbols,
        "metadata": {},
    }))
    .expect("execution portfolio config");
    let selector = PortfolioSnapshotSelector::new(
        PortfolioId::new(portfolio.portfolio_id.as_str()).expect("execution portfolio id"),
    );
    let routing_manifest = PortfolioRoutingManifest::new(routes).expect("execution route manifest");
    let lanes = structured_portfolio_lane_inputs(
        selector.clone(),
        portfolio.clone(),
        routing_manifest.clone(),
    )
    .expect("execution lanes");
    (
        PortfolioSnapshotInput::new(selector, portfolio, routing_manifest),
        lanes,
    )
}

fn assert_portfolio_outputs(
    label: &str,
    assets: &[FixtureAssets],
    portfolio: &mfm_portfolio::PortfolioConfig,
    lanes: &[EvmBalanceLaneInput],
    outputs: &PortfolioPublicOutputs,
) {
    let network_pins = (0..assets.len())
        .map(|index| {
            let chain_id = u64::try_from(index + 1).expect("expected chain id");
            NetworkPin {
                network_id: format!("network-{}", char::from(b'a' + index as u8)),
                anchor: mfm_portfolio::ExecutionAnchor::Evm {
                    chain_id: std::num::NonZeroU64::new(chain_id).expect("non-zero chain id"),
                    block: test_anchor(chain_id),
                },
            }
        })
        .collect::<Vec<_>>();
    for (index, pin) in network_pins.iter().enumerate() {
        let chain_id = u64::try_from(index + 1).expect("expected chain id");
        let mfm_portfolio::ExecutionAnchor::Evm {
            chain_id: observed_chain,
            block,
        } = &pin.anchor
        else {
            panic!("portfolio {label} returned a non-EVM anchor")
        };
        assert_eq!(observed_chain.get(), chain_id);
        assert_eq!(block.number(), (chain_id * 100).to_string());
        assert_eq!(
            block.hash(),
            format!("{:#x}", B256::repeat_byte(chain_id as u8))
        );
    }

    let mut wallet_snapshots = Vec::with_capacity(portfolio.wallets.len());
    let mut wallet_summaries = Vec::with_capacity(portfolio.wallets.len());
    let mut portfolio_total = 0_u64;
    for wallet in &portfolio.wallets {
        let lane = lanes
            .iter()
            .find(|lane| lane.binding().network_id() == wallet.network_id.as_str())
            .expect("wallet network lane");
        let chain_id = lane.binding().chain_id();
        let pin = network_pins
            .iter()
            .find(|pin| pin.network_id == wallet.network_id.as_str())
            .expect("wallet network pin");
        let mut observations = Vec::with_capacity(wallet.symbol_ids.len());
        for symbol_id in &wallet.symbol_ids {
            let symbol = portfolio
                .symbol_configs
                .iter()
                .find(|symbol| &symbol.symbol_id == symbol_id)
                .expect("wallet symbol config");
            let decimals = match symbol.source {
                HoldingSourceConfig::Native => 18,
                HoldingSourceConfig::Erc20 { .. } => 6,
            };
            let multiplier = match decimals {
                18 => 1_000_000_000_000_000_000_u128,
                6 => 1_000_000_u128,
                _ => unreachable!("fixture decimal scale"),
            };
            let raw_dec = (u128::from(chain_id) * multiplier).to_string();
            let amount_dec = format!("{chain_id}.{}", "0".repeat(decimals.into()));
            observations.push(Observation {
                wallet_id: wallet.wallet_id.to_string(),
                symbol_id: symbol.symbol_id.to_string(),
                display_symbol: symbol.display_symbol.clone(),
                network_id: wallet.network_id.to_string(),
                quantity: ObservationQuantity {
                    raw_dec,
                    decimals,
                    amount_dec: amount_dec.clone(),
                },
                values: vec![ObservationValue {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: symbol.symbol_id.to_string(),
                    value_dec: amount_dec,
                    unit_price_dec: "1.00".to_owned(),
                }],
                source: AnchoredHoldingSource {
                    holding: symbol.source.clone(),
                    anchor: pin.anchor.clone(),
                },
                metadata: symbol.metadata.clone(),
            });
        }
        let wallet_total = chain_id
            .checked_mul(u64::try_from(observations.len()).expect("observation count"))
            .expect("wallet total");
        portfolio_total = portfolio_total
            .checked_add(wallet_total)
            .expect("portfolio total");
        wallet_snapshots.push(WalletSnapshot {
            wallet_id: wallet.wallet_id.to_string(),
            subject: wallet.subject.clone(),
            network_id: wallet.network_id.to_string(),
            observations,
        });
        wallet_summaries.push(WalletReport {
            wallet_id: wallet.wallet_id.to_string(),
            network_id: wallet.network_id.to_string(),
            totals_by_quote: vec![PortfolioQuoteTotal {
                quote: QuoteCode::Usd,
                total_value_dec: wallet_total.to_string(),
            }],
        });
    }
    let expected = PortfolioPublicOutputs {
        snapshot: PortfolioSnapshot {
            schema_version: PortfolioSnapshot::SCHEMA_VERSION,
            portfolio_id: portfolio.portfolio_id.to_string(),
            network_pins: network_pins.clone(),
            wallets: wallet_snapshots,
            symbol_configs: portfolio.symbol_configs.clone(),
        },
        report: PortfolioReport {
            schema_version: PortfolioReport::SCHEMA_VERSION,
            portfolio_id: portfolio.portfolio_id.to_string(),
            network_pins,
            wallet_summaries,
            totals_by_quote: vec![PortfolioQuoteTotal {
                quote: QuoteCode::Usd,
                total_value_dec: portfolio_total.to_string(),
            }],
        },
    };
    assert_eq!(outputs, &expected, "portfolio {label} exact output");
}

fn test_admission_material(label: &str) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        test_admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            &format!("mfm.portfolio.test.{label}.configuration"),
        ),
        test_admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            &format!("mfm.portfolio.test.{label}.context"),
        ),
        HistoryObject::from_persisted(
            &PriorRunFactSourceManifest::new(Vec::new())
                .expect("execution prior-run source manifest"),
        )
        .expect("execution prior-run source object"),
        test_admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            &format!("mfm.portfolio.test.{label}.routing"),
        ),
        Vec::new(),
    )
    .expect("execution admission material")
}

fn test_admission_object(object_type: &str, schema: &str) -> HistoryObject {
    test_history_object(stable(object_type), schema_id(schema), "{}")
}

struct CertifiedPrograms {
    portfolio: mfm_certify::structured::CertifiedProgram,
    balance: mfm_certify::structured::CertifiedProgram,
    certification: mfm_certify::structured::AdmissionCertificationRegistry,
    verification: mfm_certify::structured::AdmissionVerificationRegistry,
}

fn certified_programs(lanes: &[EvmBalanceLaneInput], reverse: bool) -> CertifiedPrograms {
    let mut builder = ProgramRegistryBuilder::new();
    let executable = builder
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable(EXECUTABLE_ID),
        })
        .expect("executable identity");
    let qualification = builder
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable(QUALIFICATION_ID),
        })
        .expect("qualification artifact");
    let evm = EvmSubmissionProcessQualification::new(executable.clone(), qualification.clone());
    let portfolio = PortfolioProcessQualification::new(executable.clone(), qualification.clone());
    register_evm_balance_process(&mut builder, &evm).expect("balance process");
    register_portfolio_process(&mut builder, &portfolio).expect("portfolio process");
    register_test_adapters(
        &mut builder,
        &evm,
        reverse,
        Arc::new(TestBalanceCalls::default()),
    );

    let portfolio_id = stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID);
    let portfolio_program = structured_portfolio_snapshot_program(
        portfolio_id.clone(),
        stable("mfm.portfolio.test/portfolio-scope"),
        lanes,
    )
    .expect("portfolio program");
    builder
        .register_entry_point(portfolio_id.clone(), portfolio_program.clone(), profile(2))
        .expect("portfolio entry");

    let balance_id = stable(STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID);
    let balance_program = structured_evm_balance_collection_program(
        balance_id.clone(),
        stable("mfm.portfolio.test/balance-scope"),
        &lanes[..2],
    )
    .expect("balance program");
    builder
        .register_entry_point(balance_id.clone(), balance_program.clone(), profile(1))
        .expect("balance entry");

    let qualified = builder
        .build(&[portfolio_id.clone(), balance_id.clone()])
        .expect("qualified registry");
    let verification = qualified.admission_verification_registry();
    let registry = qualified.admission_certification_registry();
    let portfolio = registry
        .certify(&portfolio_id, portfolio_program)
        .expect("portfolio certification");
    let balance = registry
        .certify(&balance_id, balance_program)
        .expect("balance certification");
    CertifiedPrograms {
        portfolio,
        balance,
        certification: registry,
        verification,
    }
}

fn register_test_adapters(
    builder: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    reverse: bool,
    calls: Arc<TestBalanceCalls>,
) {
    macro_rules! register {
        ($capability:ty) => {
            register_test_adapter::<$capability>(builder, qualification, Arc::clone(&calls))
        };
    }
    if reverse {
        register!(EvmConfirmAnchorCapability);
        register!(EvmTokenBalanceCapability);
        register!(EvmNativeBalanceCapability);
        register!(EvmTokenDecimalsCapability);
        register!(EvmLatestAnchorCapability);
        register!(EvmChainIdentityCapability);
    } else {
        register!(EvmChainIdentityCapability);
        register!(EvmLatestAnchorCapability);
        register!(EvmTokenDecimalsCapability);
        register!(EvmNativeBalanceCapability);
        register!(EvmTokenBalanceCapability);
        register!(EvmConfirmAnchorCapability);
    }
}

fn register_test_adapter<C>(
    builder: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    calls: Arc<TestBalanceCalls>,
) where
    C: TestBalanceCapability,
{
    let contract = balance_adapter_contract(C::adapter_id()).expect("adapter contract");
    let semantic_contract_ref = contract.content_ref().expect("adapter contract ref");
    let source = Arc::new(TestBindingSource::<C> {
        certificate: certificate(C::adapter_id()),
        calls,
        capability: PhantomData,
    });
    builder
        .register_read_adapter::<C, _>(
            SecretFreeImplementationDescriptor {
                component_kind: StructuredComponentKind::Adapter,
                semantic_contract_ref,
                implementation_id: stable(format!(
                    "mfm.portfolio.test/implementation/{}",
                    C::adapter_id().replace('/', "-")
                )),
                executable_identity_ref: qualification.executable_identity_ref().clone(),
                qualification_artifact_ref: qualification.qualification_artifact_ref().clone(),
            },
            source,
        )
        .expect("test adapter registration");
}

fn lane_inputs() -> Vec<EvmBalanceLaneInput> {
    let registry = ContentRef::new(
        schema_id("mfm.portfolio.test.route"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            mfm_canonical::sha256_digest_bytes(b"portfolio structured registry"),
        ),
    )
    .expect("registry ref");
    let registry_ref = EvmWalletReference::from_content_ref(registry);
    let generation = |network: &str| {
        EvmRoutingGenerationRef::from_content_ref(
            ContentRef::new(
                schema_id("mfm.portfolio.test.route"),
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    mfm_canonical::sha256_digest_bytes(network.as_bytes()),
                ),
            )
            .expect("network route ref"),
        )
        .expect("generation ref")
    };
    let chain_binding = |chain_id: u64, namespace: &str| {
        ChainInstanceRegistryAttestation::new(
            ChainInstanceDeclaration::new(
                registry_ref.clone(),
                stable(namespace),
                chain_id,
                B256::repeat_byte(u8::try_from(chain_id).expect("chain byte")),
                U256::from(1_u64),
                B256::repeat_byte(u8::try_from(chain_id + 10).expect("anchor byte")),
            )
            .expect("chain declaration"),
            registry_ref.clone(),
            registry_ref.clone(),
        )
        .and_then(|attestation| attestation.binding())
        .expect("chain binding")
    };
    let binding_a = EvmNetworkBinding::new(
        "network-a",
        chain_binding(1, "mfm.portfolio.test/chain-a"),
        generation("portfolio structured route A"),
    )
    .expect("binding");
    let binding_b = EvmNetworkBinding::new(
        "network-b",
        chain_binding(2, "mfm.portfolio.test/chain-b"),
        generation("portfolio structured route B"),
    )
    .expect("binding");
    let account = Address::repeat_byte(0x11);
    vec![
        lane(0, binding_a.clone(), account, EvmBalanceAsset::Native),
        lane(
            0,
            binding_a,
            account,
            EvmBalanceAsset::erc20(Address::repeat_byte(0x22)).expect("token"),
        ),
        lane(1, binding_b.clone(), account, EvmBalanceAsset::Native),
        lane(
            1,
            binding_b,
            account,
            EvmBalanceAsset::erc20(Address::repeat_byte(0x33)).expect("token"),
        ),
    ]
}

fn lane(
    collection_ordinal: u32,
    binding: EvmNetworkBinding,
    account: Address,
    asset: EvmBalanceAsset,
) -> EvmBalanceLaneInput {
    EvmBalanceLaneInput::new(
        collection_ordinal,
        "{}".to_owned(),
        binding,
        18,
        EvmBalanceSource::new(account, asset).expect("source"),
    )
    .expect("lane")
}

fn certificate(discriminator: &str) -> HistoryObject {
    test_history_object(
        stable("mfm.portfolio.test/physical-certificate"),
        schema_id("mfm.portfolio.test.physical-certificate"),
        format!("{{\"binding\":{discriminator:?}}}"),
    )
}

fn test_history_object(
    object_type: StableId,
    schema_id: SchemaId,
    canonical_json: impl Into<String>,
) -> HistoryObject {
    let canonical_json = canonical_json.into();
    HistoryObject {
        object_type,
        content_ref: ContentRef::new(
            schema_id,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical_json.as_bytes()),
            ),
        )
        .expect("test object content reference"),
        canonical_json,
    }
}

fn profile(max_fan_out_depth: u8) -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 4_096,
        max_declarations: 8_192,
        max_lanes: 4_096,
        max_fan_out_depth,
        max_branch_depth: 64,
    }
}

#[derive(Default)]
struct ProgramShape {
    fan_outs: usize,
    child_calls: usize,
    fragments: usize,
    maximum_fan_out_depth: usize,
}

fn authored_shape(lanes: &[EvmBalanceLaneInput]) -> ProgramShape {
    let program = structured_portfolio_snapshot_program(
        stable(STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID),
        stable("mfm.portfolio.test/shape-scope"),
        lanes,
    )
    .expect("shape program");
    let mut shape = ProgramShape::default();
    visit_authored(&program.root, 0, &mut shape);
    shape
}

fn visit_authored(block: &AuthoredBlock, depth: usize, shape: &mut ProgramShape) {
    for declaration in &block.declarations {
        match declaration {
            AuthoredDeclaration::State(_) => {}
            AuthoredDeclaration::OperationCall(_) => shape.child_calls += 1,
            AuthoredDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    visit_authored(&arm.body, depth, shape);
                }
            }
            AuthoredDeclaration::FanOut(group) => {
                shape.fan_outs += 1;
                shape.maximum_fan_out_depth = shape.maximum_fan_out_depth.max(depth + 1);
                for lane in &group.lanes {
                    visit_authored(&lane.body, depth + 1, shape);
                }
            }
        }
    }
}

fn expanded_shape(block: &ExpandedBlock) -> ProgramShape {
    let mut shape = ProgramShape::default();
    visit_expanded(block, 0, &mut shape);
    shape
}

fn visit_expanded(block: &ExpandedBlock, depth: usize, shape: &mut ProgramShape) {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(_) => {}
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    visit_expanded(&arm.body, depth, shape);
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                shape.fan_outs += 1;
                shape.maximum_fan_out_depth = shape.maximum_fan_out_depth.max(depth + 1);
                for lane in &group.lanes {
                    visit_expanded(&lane.body, depth + 1, shape);
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                shape.fragments += 1;
                visit_expanded(&fragment.body, depth, shape);
            }
        }
    }
}

fn schema_id(name: &str) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(name.as_bytes()),
    )
    .expect("schema id")
}

fn stable(value: impl AsRef<str>) -> StableId {
    StableId::new(value.as_ref()).expect("stable id")
}
