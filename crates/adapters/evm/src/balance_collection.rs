//! Live and replay bindings for reusable EVM balance collection.

use std::future::Future;
use std::sync::Arc;
use std::task::Poll;

use alloy_primitives::{Address, Bytes, U256};
use mfm_evm::{
    CollectEvmBalancesState, EvmBalanceAsset, EvmBalanceCollectionEvidence,
    EvmBalanceCollectionPlan, EvmBalanceReadEvidence, EvmBlockSelector, EvmCall, EvmReadSession,
    EvmTokenDecimalsEvidence,
};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    ErasedRunCtx, ExternalReadExecution, ExternalReadExecutionFuture, ExternalReadPlanExecutor,
    RunnerIngressContext,
};

use crate::{evm_ingress_runtime_error, evm_read_runtime_error, EvmReadRunnerCapabilities};

pub(crate) const EVM_READ_CONCURRENCY_LIMIT: usize = 16;
pub(crate) const ERC20_DECIMALS_SELECTOR: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];
pub(crate) const ERC20_BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

pub(crate) struct CollectEvmBalancesExecutor {
    pub(crate) capabilities: EvmReadRunnerCapabilities,
}

impl CollectEvmBalancesExecutor {
    pub(crate) async fn validate_read_route(
        &self,
        state: &CollectEvmBalancesState,
    ) -> mfm_runtime::Result<()> {
        let binding = state
            .config()
            .binding()
            .map_err(balance_state_runtime_error)?;
        self.capabilities
            .validate_read_route(binding)
            .await
            .map_err(evm_ingress_runtime_error)
    }
}

impl ExternalReadPlanExecutor<CollectEvmBalancesState> for CollectEvmBalancesExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        state: &'a CollectEvmBalancesState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async move { self.validate_read_route(state).await })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a EvmBalanceCollectionPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, EvmBalanceCollectionEvidence> {
        Box::pin(async move {
            let evidence = collect_evm_balances(plan, &self.capabilities).await?;
            Ok(ExternalReadExecution::primary(evidence))
        })
    }
}

pub(crate) async fn collect_evm_balances(
    plan: &EvmBalanceCollectionPlan,
    capabilities: &EvmReadRunnerCapabilities,
) -> mfm_runtime::Result<EvmBalanceCollectionEvidence> {
    let binding = plan.binding().map_err(balance_state_runtime_error)?;
    let session = capabilities
        .bind(binding)
        .await
        .map_err(evm_read_runtime_error)?;
    let latest = session
        .read_block(&EvmBlockSelector::Latest)
        .await
        .map_err(evm_read_runtime_error)?;
    let block_number = latest.number_quantity().map_err(|_| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM session returned an invalid block number".to_owned(),
        )
    })?;
    let block_hash = latest.hash_value().map_err(|_| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM session returned an invalid block hash".to_owned(),
        )
    })?;
    let exact = EvmBlockSelector::ExactHash(block_hash);

    let token_decimals = read_token_decimals(plan, Arc::clone(&session), exact.clone()).await?;
    let balances = read_balances(plan, Arc::clone(&session), exact).await?;
    let final_block = session
        .read_block(&EvmBlockSelector::Number(block_number))
        .await
        .map_err(evm_read_runtime_error)?;
    Ok(EvmBalanceCollectionEvidence::new(
        session.evidence(),
        latest,
        token_decimals,
        balances,
        final_block,
    ))
}

async fn read_token_decimals(
    plan: &EvmBalanceCollectionPlan,
    session: Arc<dyn EvmReadSession>,
    selector: EvmBlockSelector,
) -> mfm_runtime::Result<Vec<EvmTokenDecimalsEvidence>> {
    let contracts = plan.token_contracts();
    let mut evidence = Vec::with_capacity(contracts.len());
    for chunk in contracts.chunks(EVM_READ_CONCURRENCY_LIMIT) {
        let reads = chunk
            .iter()
            .map(|contract| {
                let contract = contract.clone();
                let session = Arc::clone(&session);
                let selector = selector.clone();
                async move {
                    let contract_address = contract.parse::<Address>().map_err(|_| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(
                            "certified ERC-20 contract address was invalid".to_owned(),
                        )
                    })?;
                    let call = EvmCall::new(
                        Address::ZERO,
                        contract_address,
                        U256::ZERO,
                        Bytes::copy_from_slice(&ERC20_DECIMALS_SELECTOR),
                        U256::from(100_000_u64),
                        Default::default(),
                        selector,
                        32,
                    )
                    .map_err(evm_read_runtime_error)?;
                    let result = session.call(&call).await.map_err(evm_read_runtime_error)?;
                    let decimals = u8::try_from(decode_abi_word(&result)?).map_err(|_| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(
                            "ERC-20 decimals result exceeded u8".to_owned(),
                        )
                    })?;
                    EvmTokenDecimalsEvidence::new(contract_address, decimals)
                        .map_err(balance_state_runtime_error)
                }
            })
            .collect::<Vec<_>>();
        evidence.extend(try_join_ordered(reads).await?);
    }
    evidence.sort();
    Ok(evidence)
}

async fn read_balances(
    plan: &EvmBalanceCollectionPlan,
    session: Arc<dyn EvmReadSession>,
    selector: EvmBlockSelector,
) -> mfm_runtime::Result<Vec<EvmBalanceReadEvidence>> {
    let mut evidence = Vec::with_capacity(plan.sources().len());
    for chunk in plan.sources().chunks(EVM_READ_CONCURRENCY_LIMIT) {
        let reads = chunk
            .iter()
            .map(|source| {
                let source = source.clone();
                let session = Arc::clone(&session);
                let selector = selector.clone();
                async move {
                    let account = source
                        .account_address()
                        .map_err(balance_state_runtime_error)?;
                    let raw_units = match source.asset() {
                        EvmBalanceAsset::Native => session
                            .read_balance(account, &selector)
                            .await
                            .map_err(evm_read_runtime_error)?,
                        EvmBalanceAsset::Erc20 { .. } => {
                            let contract = source
                                .contract_address_value()
                                .map_err(balance_state_runtime_error)?
                                .ok_or_else(|| {
                                    mfm_runtime::RuntimeError::InvalidRunnerOutput(
                                        "certified ERC-20 source lacked a contract".to_owned(),
                                    )
                                })?;
                            let call = EvmCall::new(
                                Address::ZERO,
                                contract,
                                U256::ZERO,
                                erc20_balance_of_calldata(account),
                                U256::from(100_000_u64),
                                Default::default(),
                                selector,
                                32,
                            )
                            .map_err(evm_read_runtime_error)?;
                            let result =
                                session.call(&call).await.map_err(evm_read_runtime_error)?;
                            decode_abi_word(&result)?
                        }
                    };
                    Ok::<_, mfm_runtime::RuntimeError>(EvmBalanceReadEvidence::new(
                        source, raw_units,
                    ))
                }
            })
            .collect::<Vec<_>>();
        evidence.extend(try_join_ordered(reads).await?);
    }
    evidence.sort();
    Ok(evidence)
}

fn erc20_balance_of_calldata(account: Address) -> Bytes {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&ERC20_BALANCE_OF_SELECTOR);
    calldata.extend_from_slice(&[0_u8; 12]);
    calldata.extend_from_slice(account.as_slice());
    Bytes::from(calldata)
}

fn decode_abi_word(bytes: &Bytes) -> mfm_runtime::Result<U256> {
    if bytes.len() != 32 {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "ERC-20 call result was not one ABI word".to_owned(),
        ));
    }
    Ok(U256::from_be_slice(bytes.as_ref()))
}

async fn try_join_ordered<T, F>(reads: Vec<F>) -> mfm_runtime::Result<Vec<T>>
where
    F: Future<Output = mfm_runtime::Result<T>>,
{
    let mut reads = reads
        .into_iter()
        .map(|read| Some(Box::pin(read)))
        .collect::<Vec<_>>();
    let mut outputs = (0..reads.len()).map(|_| None).collect::<Vec<_>>();
    std::future::poll_fn(|context| {
        for (read, output) in reads.iter_mut().zip(outputs.iter_mut()) {
            let Some(future) = read.as_mut() else {
                continue;
            };
            match future.as_mut().poll(context) {
                Poll::Ready(Ok(value)) => {
                    *output = Some(value);
                    *read = None;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => {}
            }
        }
        if reads.iter().any(Option::is_some) {
            return Poll::Pending;
        }
        let mut ordered = Vec::with_capacity(outputs.len());
        for output in std::mem::take(&mut outputs) {
            let Some(output) = output else {
                return Poll::Ready(Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "ordered EVM read completed without output".to_owned(),
                )));
            };
            ordered.push(output);
        }
        Poll::Ready(Ok(ordered))
    })
    .await
}

/// Verifies EVM balance collection and atomic fact publication from retained material only.
pub fn verify_evm_balance_collection_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    replay::verify_external_read_state::<CollectEvmBalancesState>(broker)
}

fn balance_state_runtime_error(
    error: mfm_evm::EvmBalanceCollectionError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}
