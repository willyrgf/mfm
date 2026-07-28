//! Adapter tests for reusable EVM balance collection.

use super::*;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{address, b256, Address, Bytes, B256, U256};
use mfm_capabilities::ProviderDiagnosticCode;
use mfm_evm::{
    evm_diagnostic, reduce_evm_balance_collection, CollectEvmBalancesState, EvmBalanceAsset,
    EvmBalanceCollectionConfig, EvmBalanceCollectionPlan, EvmBalanceSource, EvmBlockAnchor,
    EvmBlockSelector, EvmCall, EvmCode, EvmSessionEvidence, EvmSessionFuture,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, LocalPublicId};
use mfm_program::{ReadState, StateSpec, ValidatedConfig};

use super::balance_collection::{
    collect_evm_balances, ERC20_BALANCE_OF_SELECTOR, ERC20_DECIMALS_SELECTOR,
    EVM_READ_CONCURRENCY_LIMIT,
};

const ACCOUNT: Address = address!("000000000000000000000000000000000000dead");
const TOKEN: Address = address!("0000000000000000000000000000000000000001");
const ANCHOR_HASH: B256 = b256!("1111111111111111111111111111111111111111111111111111111111111111");
const REORG_HASH: B256 = b256!("2222222222222222222222222222222222222222222222222222222222222222");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PrototypeAuditedOperation {
    Bootstrap,
    LatestAnchor,
    TokenMetadata,
    Balance,
    ConfirmAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeReturned {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeObservation {
    Returned(PrototypeReturned),
    DidNotEnter,
    Indeterminate,
}

#[derive(Default)]
struct PrototypeAuditLedger {
    next_authorization: usize,
    authorizations: Vec<(usize, PrototypeAuditedOperation)>,
    observations: Vec<(usize, PrototypeObservation)>,
    boundary_entries: BTreeMap<usize, usize>,
}

impl PrototypeAuditLedger {
    fn authorize(
        ledger: &Arc<Mutex<Self>>,
        operation: PrototypeAuditedOperation,
    ) -> PrototypeAccessAuthority {
        let mut state = ledger.lock().expect("audit ledger");
        let authorization = state.next_authorization;
        state.next_authorization += 1;
        state.authorizations.push((authorization, operation));
        drop(state);
        PrototypeAccessAuthority {
            authorization,
            ledger: Arc::clone(ledger),
            entered: false,
            observed: false,
        }
    }
}

struct PrototypeAccessAuthority {
    authorization: usize,
    ledger: Arc<Mutex<PrototypeAuditLedger>>,
    entered: bool,
    observed: bool,
}

impl PrototypeAccessAuthority {
    fn enter(&mut self) -> Result<(), &'static str> {
        if self.entered || self.observed {
            return Err("one authorization permits at most one boundary entry");
        }
        self.entered = true;
        *self
            .ledger
            .lock()
            .expect("audit ledger")
            .boundary_entries
            .entry(self.authorization)
            .or_default() += 1;
        Ok(())
    }

    fn returned(&mut self, returned: PrototypeReturned) {
        assert!(self.entered, "returned requires boundary entry");
        self.observe(PrototypeObservation::Returned(returned));
    }

    fn did_not_enter(&mut self) {
        assert!(!self.entered, "DidNotEnter forbids boundary entry");
        self.observe(PrototypeObservation::DidNotEnter);
    }

    fn cancelled_after_possible_entry(&mut self) {
        assert!(
            self.entered,
            "Indeterminate requires possible boundary entry"
        );
        self.observe(PrototypeObservation::Indeterminate);
    }

    fn observe(&mut self, observation: PrototypeObservation) {
        assert!(!self.observed, "one authorization permits one observation");
        self.ledger
            .lock()
            .expect("audit ledger")
            .observations
            .push((self.authorization, observation));
        self.observed = true;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadRecord {
    Block(EvmBlockSelector),
    Balance(Address, EvmBlockSelector),
    Call(Address, Vec<u8>, EvmBlockSelector),
}

struct RecordingSession {
    evidence: EvmSessionEvidence,
    records: Mutex<Vec<ReadRecord>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    final_hash: B256,
}

struct UnavailableSession {
    evidence: EvmSessionEvidence,
}

impl UnavailableSession {
    fn new(binding: &EvmNetworkBinding) -> Self {
        Self {
            evidence: EvmSessionEvidence::new(
                binding,
                LocalPublicId::new("primary").expect("source ref"),
                LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
        }
    }

    fn unavailable<'a, T: Send + 'a>(&'a self) -> EvmSessionFuture<'a, T> {
        Box::pin(std::future::ready(Err(temporary_provider_failure())))
    }
}

impl EvmReadSession for UnavailableSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(
        &'a self,
        _selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        self.unavailable()
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        self.unavailable()
    }

    fn read_code<'a>(
        &'a self,
        _address: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        self.unavailable()
    }

    fn call<'a>(&'a self, _request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        self.unavailable()
    }
}

impl RecordingSession {
    fn new(binding: &EvmNetworkBinding, final_hash: B256) -> Self {
        Self {
            evidence: EvmSessionEvidence::new(
                binding,
                LocalPublicId::new("primary").expect("source ref"),
                LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            records: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            final_hash,
        }
    }

    fn record(&self, record: ReadRecord) {
        self.records.lock().expect("records").push(record);
    }

    fn records(&self) -> Vec<ReadRecord> {
        self.records.lock().expect("records").clone()
    }

    async fn concurrent_read(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl EvmReadSession for RecordingSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        self.record(ReadRecord::Block(selector.clone()));
        let result = match selector {
            EvmBlockSelector::Latest => Ok(EvmBlockAnchor::new(U256::from(100), ANCHOR_HASH)),
            EvmBlockSelector::Number(number) if *number == U256::from(100) => {
                Ok(EvmBlockAnchor::new(U256::from(100), self.final_hash))
            }
            _ => Err(provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }

    fn read_balance<'a>(
        &'a self,
        account: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        let selector = block.clone();
        self.record(ReadRecord::Balance(account, selector));
        Box::pin(async move {
            self.concurrent_read().await;
            Ok(U256::from(42))
        })
    }

    fn read_code<'a>(
        &'a self,
        _address: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        Box::pin(std::future::ready(Err(provider_failure())))
    }

    fn call<'a>(&'a self, request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        let input = request.input().to_vec();
        self.record(ReadRecord::Call(
            request.to(),
            input.clone(),
            request.block().clone(),
        ));
        Box::pin(async move {
            self.concurrent_read().await;
            let value = if input.starts_with(&ERC20_DECIMALS_SELECTOR) {
                U256::from(6)
            } else if input.starts_with(&ERC20_BALANCE_OF_SELECTOR) {
                U256::from(99)
            } else {
                return Err(provider_failure());
            };
            let word = value.to_be_bytes::<32>();
            Ok(Bytes::copy_from_slice(&word))
        })
    }
}

fn provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(
        ProviderDiagnosticCode::ProviderConfigurationInvalid,
    ))
}

fn temporary_provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(ProviderDiagnosticCode::TransportFailed))
}

fn source(account: Address, asset: EvmBalanceAsset) -> EvmBalanceSource {
    EvmBalanceSource::new(account, asset).expect("EVM source")
}

fn plan_for(sources: Vec<EvmBalanceSource>) -> EvmBalanceCollectionPlan {
    let config = EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, sources)
        .expect("collection config");
    let state = <CollectEvmBalancesState as StateSpec>::new(
        ValidatedConfig::new(config).expect("validated config"),
    )
    .expect("collection state");
    state
        .plan(&(), &mfm_program::CertifiedContext::no_context())
        .expect("collection plan")
}

fn capabilities(
    session: Arc<RecordingSession>,
    binds: Arc<AtomicUsize>,
) -> EvmReadRunnerCapabilities {
    let sessions = test_read_session_set(
        session as Arc<dyn EvmReadSession>,
        None,
        Arc::new(AtomicUsize::new(0)),
        binds,
    );
    EvmReadRunnerCapabilities::new(sessions)
}

fn registry() -> ErasedRunnerRegistry {
    ErasedRunnerRegistry::new(mfm_runtime::ExecutableIdentityTemplate::new(
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x42; 32]),
        ),
    ))
}

#[test]
fn balance_runner_registration_keeps_factory_identity_explicit() {
    assert_eq!(READ_FACTORY, "read_external");
    assert_eq!(ADAPTER_FACTORY, "evm_jsonrpc_adapter");
}

#[test]
fn adapter_registration_accepts_a_fake_pure_session_set() {
    let mut registry = registry();
    let read_factory = registry.factory_binding(
        events::RunnerFactoryId::new(READ_FACTORY).expect("read factory identity"),
    );
    let adapter_factory = registry.factory_binding(
        events::RunnerFactoryId::new(ADAPTER_FACTORY).expect("adapter factory identity"),
    );
    let binding =
        EvmNetworkBinding::new(LocalPublicId::new("ethereum-mainnet").expect("network"), 1)
            .expect("binding");
    let capabilities = capabilities(
        Arc::new(RecordingSession::new(&binding, ANCHOR_HASH)),
        Arc::new(AtomicUsize::new(0)),
    );

    register_evm_balance_runners(
        &mut registry,
        capabilities.sessions,
        &read_factory,
        &adapter_factory,
    )
    .expect("fake-session-set registration");
}

#[tokio::test]
async fn one_session_uses_latest_then_exact_hash_reads_then_number_recheck() {
    let plan = plan_for(vec![
        source(ACCOUNT, EvmBalanceAsset::Native),
        source(ACCOUNT, EvmBalanceAsset::erc20(TOKEN).expect("token asset")),
    ]);
    let binding = plan.binding().expect("binding");
    let session = Arc::new(RecordingSession::new(&binding, ANCHOR_HASH));
    let binds = Arc::new(AtomicUsize::new(0));
    let capabilities = capabilities(Arc::clone(&session), Arc::clone(&binds));

    let evidence = collect_evm_balances(&plan, &capabilities)
        .await
        .expect("collection evidence");
    let (receipt, facts) =
        reduce_evm_balance_collection(&plan, &evidence).expect("reduced evidence");
    assert_eq!(receipt.sources().len(), 2);
    assert_eq!(facts.values().len(), 2);
    assert_eq!(binds.load(Ordering::SeqCst), 1);

    let records = session.records();
    assert_eq!(
        records.first(),
        Some(&ReadRecord::Block(EvmBlockSelector::Latest))
    );
    assert_eq!(
        records.last(),
        Some(&ReadRecord::Block(EvmBlockSelector::Number(U256::from(
            100
        ))))
    );
    let mut decimals_calls = 0;
    let mut balance_of_calls = 0;
    for record in &records[1..records.len() - 1] {
        match record {
            ReadRecord::Balance(_, EvmBlockSelector::ExactHash(hash)) => {
                assert_eq!(*hash, ANCHOR_HASH);
            }
            ReadRecord::Call(_, input, EvmBlockSelector::ExactHash(hash)) => {
                assert_eq!(*hash, ANCHOR_HASH);
                decimals_calls += usize::from(input.starts_with(&ERC20_DECIMALS_SELECTOR));
                balance_of_calls += usize::from(input.starts_with(&ERC20_BALANCE_OF_SELECTOR));
            }
            unexpected => panic!("unexpected EVM read between anchor checks: {unexpected:?}"),
        }
    }
    assert_eq!(decimals_calls, 1);
    assert_eq!(balance_of_calls, 1);
}

#[tokio::test]
async fn final_number_recheck_exposes_reorg_to_the_deterministic_reducer() {
    let plan = plan_for(vec![source(ACCOUNT, EvmBalanceAsset::Native)]);
    let session = Arc::new(RecordingSession::new(
        &plan.binding().expect("binding"),
        REORG_HASH,
    ));
    let capabilities = capabilities(session, Arc::new(AtomicUsize::new(0)));
    let evidence = collect_evm_balances(&plan, &capabilities)
        .await
        .expect("retained evidence");
    assert!(reduce_evm_balance_collection(&plan, &evidence)
        .expect_err("reorg must fail reduction")
        .to_string()
        .contains("no longer canonical"));
}

#[tokio::test]
async fn temporary_provider_outage_blocks_the_balance_collector() {
    let plan = plan_for(vec![source(ACCOUNT, EvmBalanceAsset::Native)]);
    let session = Arc::new(UnavailableSession::new(&plan.binding().expect("binding")));
    let sessions = test_read_session_set(
        session as Arc<dyn EvmReadSession>,
        None,
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    );
    let capabilities = EvmReadRunnerCapabilities::new(sessions);

    let error = collect_evm_balances(&plan, &capabilities)
        .await
        .expect_err("temporary outage must block collection");

    assert!(matches!(error, mfm_runtime::RuntimeError::Blocked(_)));
}

#[tokio::test]
async fn balance_reads_never_exceed_the_hard_concurrency_limit() {
    let sources = (1_u64..=40)
        .map(|index| {
            source(
                Address::from_word(U256::from(index).into()),
                EvmBalanceAsset::Native,
            )
        })
        .collect();
    let plan = plan_for(sources);
    let session = Arc::new(RecordingSession::new(
        &plan.binding().expect("binding"),
        ANCHOR_HASH,
    ));
    let capabilities = capabilities(Arc::clone(&session), Arc::new(AtomicUsize::new(0)));
    collect_evm_balances(&plan, &capabilities)
        .await
        .expect("bounded collection");
    let max_active = session.max_active.load(Ordering::SeqCst);
    assert!(max_active > 1, "test must exercise concurrent reads");
    assert!(max_active <= EVM_READ_CONCURRENCY_LIMIT);
}

#[tokio::test]
async fn concurrent_reads_are_issued_in_certified_plan_order() {
    let first_token = Address::from_word(U256::from(11).into());
    let second_token = Address::from_word(U256::from(12).into());
    let plan = plan_for(vec![
        source(
            Address::from_word(U256::from(3).into()),
            EvmBalanceAsset::Native,
        ),
        source(
            Address::from_word(U256::from(2).into()),
            EvmBalanceAsset::Native,
        ),
        source(
            Address::from_word(U256::from(1).into()),
            EvmBalanceAsset::erc20(second_token).expect("second token"),
        ),
        source(
            Address::from_word(U256::from(1).into()),
            EvmBalanceAsset::erc20(first_token).expect("first token"),
        ),
    ]);
    let session = Arc::new(RecordingSession::new(
        &plan.binding().expect("binding"),
        ANCHOR_HASH,
    ));
    let capabilities = capabilities(Arc::clone(&session), Arc::new(AtomicUsize::new(0)));

    collect_evm_balances(&plan, &capabilities)
        .await
        .expect("ordered collection");
    let records = session.records();
    let decimals_targets = records
        .iter()
        .filter_map(|record| match record {
            ReadRecord::Call(target, input, _) if input.starts_with(&ERC20_DECIMALS_SELECTOR) => {
                Some(*target)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(decimals_targets, vec![first_token, second_token]);

    let planned_accounts = plan
        .sources()
        .iter()
        .map(|source| source.account_address().expect("planned account"))
        .collect::<Vec<_>>();
    let issued_accounts = records
        .iter()
        .filter_map(|record| match record {
            ReadRecord::Balance(account, _) => Some(*account),
            ReadRecord::Call(_, input, _) if input.starts_with(&ERC20_BALANCE_OF_SELECTOR) => {
                Some(Address::from_slice(&input[16..36]))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(issued_accounts, planned_accounts);
}

#[tokio::test]
async fn wrong_chain_session_is_rejected_before_any_network_read() {
    let plan = plan_for(vec![source(ACCOUNT, EvmBalanceAsset::Native)]);
    let wrong_binding = EvmNetworkBinding::new(
        LocalPublicId::new("ethereum-mainnet").expect("network id"),
        2,
    )
    .expect("wrong binding");
    let session = Arc::new(RecordingSession::new(&wrong_binding, ANCHOR_HASH));
    let capabilities = capabilities(Arc::clone(&session), Arc::new(AtomicUsize::new(0)));

    collect_evm_balances(&plan, &capabilities)
        .await
        .expect_err("wrong-chain session must fail binding");
    assert!(session.records().is_empty());
}

#[test]
fn recoverability_prototype_classifies_every_operation_failure_and_cancellation() {
    let ledger = Arc::new(Mutex::new(PrototypeAuditLedger::default()));
    let operations = [
        PrototypeAuditedOperation::Bootstrap,
        PrototypeAuditedOperation::LatestAnchor,
        PrototypeAuditedOperation::TokenMetadata,
        PrototypeAuditedOperation::Balance,
        PrototypeAuditedOperation::ConfirmAnchor,
    ];

    for operation in operations {
        let mut failed = PrototypeAuditLedger::authorize(&ledger, operation);
        failed.enter().expect("one boundary entry");
        assert_eq!(
            failed.enter(),
            Err("one authorization permits at most one boundary entry")
        );
        failed.returned(PrototypeReturned::Failed);

        let mut cancelled = PrototypeAuditLedger::authorize(&ledger, operation);
        cancelled.enter().expect("possible boundary entry");
        cancelled.cancelled_after_possible_entry();

        let mut rejected = PrototypeAuditLedger::authorize(&ledger, operation);
        rejected.did_not_enter();
    }

    let state = ledger.lock().expect("audit ledger");
    assert_eq!(state.authorizations.len(), operations.len() * 3);
    assert_eq!(state.observations.len(), operations.len() * 3);
    assert!(state.boundary_entries.values().all(|entries| *entries == 1));
    for operation in operations {
        let authorizations = state
            .authorizations
            .iter()
            .filter(|(_, observed)| *observed == operation)
            .map(|(authorization, _)| *authorization)
            .collect::<Vec<_>>();
        let observations = state
            .observations
            .iter()
            .filter(|(authorization, _)| authorizations.contains(authorization))
            .map(|(_, observation)| *observation)
            .collect::<Vec<_>>();
        assert_eq!(
            observations,
            [
                PrototypeObservation::Returned(PrototypeReturned::Failed),
                PrototypeObservation::Indeterminate,
                PrototypeObservation::DidNotEnter,
            ]
        );
    }
}

#[test]
fn recoverability_prototype_preserves_partial_fanout_observations() {
    let ledger = Arc::new(Mutex::new(PrototypeAuditLedger::default()));
    let mut metadata =
        PrototypeAuditLedger::authorize(&ledger, PrototypeAuditedOperation::TokenMetadata);
    let mut failed_balance =
        PrototypeAuditLedger::authorize(&ledger, PrototypeAuditedOperation::Balance);
    let mut cancelled_balance =
        PrototypeAuditLedger::authorize(&ledger, PrototypeAuditedOperation::Balance);
    metadata.enter().expect("metadata entry");
    failed_balance.enter().expect("failed balance entry");
    cancelled_balance.enter().expect("cancelled balance entry");

    metadata.returned(PrototypeReturned::Succeeded);
    failed_balance.returned(PrototypeReturned::Failed);
    cancelled_balance.cancelled_after_possible_entry();

    let state = ledger.lock().expect("audit ledger");
    assert_eq!(
        state.observations,
        [
            (
                metadata.authorization,
                PrototypeObservation::Returned(PrototypeReturned::Succeeded),
            ),
            (
                failed_balance.authorization,
                PrototypeObservation::Returned(PrototypeReturned::Failed),
            ),
            (
                cancelled_balance.authorization,
                PrototypeObservation::Indeterminate,
            ),
        ],
        "one sibling failure must not swallow completed or in-flight sibling history"
    );
    assert!(
        state
            .observations
            .iter()
            .any(|(_, observation)| *observation
                == PrototypeObservation::Returned(PrototypeReturned::Succeeded)),
        "completed metadata remains independently auditable"
    );
}

#[test]
fn recoverability_prototype_abrupt_loss_leaves_an_unmatched_authorization() {
    let ledger = Arc::new(Mutex::new(PrototypeAuditLedger::default()));
    let authorization = {
        let mut access =
            PrototypeAuditLedger::authorize(&ledger, PrototypeAuditedOperation::LatestAnchor);
        access.enter().expect("boundary entry");
        access.authorization
    };

    let state = ledger.lock().expect("audit ledger");
    assert!(state
        .authorizations
        .iter()
        .any(|(candidate, _)| *candidate == authorization));
    assert!(state
        .observations
        .iter()
        .all(|(candidate, _)| *candidate != authorization));
    assert_eq!(state.boundary_entries.get(&authorization), Some(&1));
}
