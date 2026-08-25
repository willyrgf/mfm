#![warn(missing_docs)]
//! Bounded EVM provider registration and durable transaction execution.
//!
//! The live adapter owns provider ingress, exact transaction wire encoding, and
//! signer/authority/provider orchestration. Domain State registration and Runtime
//! construction remain trusted composition responsibilities.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_evm::{
    EvmAnchorRead, EvmAnchoredContractCallRead, EvmBalanceRead, EvmChainIdentityRead,
    EvmPhysicalTarget, EvmReadEvidence, EvmReadIntent, EvmReadSubject, EvmReadValue,
    EvmTransactionRoute, EVM_ANCHORED_CONTRACT_CALL_OPERATION_ID,
};
use mfm_ids::StableId;
use mfm_runtime::{AdapterError, RuntimeAssemblyBuilder};

mod codec;
mod json_rpc;
mod transaction;

pub use json_rpc::{
    EvmAdapterLocator, EvmProviderBuildError, JsonRpcEvmProvider, MAX_EVM_ADAPTER_LOCATOR_BYTES,
};
pub use transaction::{
    register_evm_transaction_effect, EvmTransactionProvider, EvmTransactionProviderFuture,
    ProviderReceipt, ProviderReceiptResult, EVM_EIP1559_SIGNING_PURPOSE_ID,
};

pub use codec::{ethereum_address, evm_keccak256, EvmCodecError};

const MAX_EVM_REQUEST_BYTES: usize = 512 * 1024;

/// Typed provider response after bounded authenticated ingress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmProviderResponse {
    /// Authenticated structured Read value.
    Read(EvmReadValue),
    /// Reviewed provider rejection.
    Rejected,
    /// Reviewed definite safe failure.
    SafeFailure,
    /// Authenticated external integrity failure safe to conclude.
    IntegrityBlocked,
}

/// Provider transport paired with one immutable public target.
///
/// `request_bytes` is one serialized [`mfm_evm::EvmReadIntent`]. Decode it with the domain's own
/// checked deserializer; do not mirror that wire. Every operation below must observe the exact
/// subject the intent carries, on the target chain the adapter already checked.
///
/// | Operation | Subject | Required observation |
/// | --- | --- | --- |
/// | `mfm.evm.read-chain-identity@1` | `ChainIdentity` | the chain id |
/// | `mfm.evm.read-initial-anchor@1` | `InitialAnchor` | the anchor of the current head block |
/// | `mfm.evm.read-native-balance@1` | `NativeBalance { source, anchor }` | the native balance **at** `anchor` |
/// | `mfm.evm.read-token-decimals@1` | `TokenDecimals { source, anchor }` | the token decimal scale **at** `anchor` |
/// | `mfm.evm.read-token-balance@1` | `TokenBalance { source, anchor }` | the token balance **at** `anchor` |
/// | `mfm.evm.confirm-balance-anchor@1` | `ConfirmAnchor { source, anchor }` | the anchor of the block **that `anchor.number()` names** |
/// | `mfm.evm.read-anchored-contract-call@1` | `AnchoredContractCall { anchor, target, calldata }` | deployed code and call result **at** `anchor`, bracketed by named-block observations |
///
/// The confirmation is the one contract an implementor is most likely to get wrong. It must
/// re-observe the named committed block. It must never return the head. The EVM domain compares
/// the returned number and hash to the anchor it pinned before the balance reads: an equal pair
/// proves the block still stands, and a different hash at the same number proves a reorg replaced
/// it. A head read would instead report the ordinary progression of the chain, so every collection
/// on a chain that produces blocks would fail with stage `confirm_anchor`.
///
/// All balance and decimal reads are anchored for the same reason: one collection must observe one
/// block, so its sources cannot tear across chain progression.
///
/// Return [`EvmProviderResponse::IntegrityBlocked`] only for authenticated external evidence of an
/// integrity block, such as replacement of an authored anchored-call block. A local decode,
/// address, or operation mismatch is [`AdapterError::Internal`] before any IO. Reads must be
/// duplicate-safe: a dropped run repeats the call.
pub trait EvmProvider: Send + Sync + 'static {
    /// Performs one observational request for the supplied operation.
    fn request<'a>(
        &'a self,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<EvmProviderResponse, AdapterError>> + Send + 'a,
        >,
    >;
}

/// Registers the three surviving EVM Read capability callbacks for one target.
pub fn register_evm_reads(
    builder: &mut RuntimeAssemblyBuilder,
    target: EvmPhysicalTarget,
    provider: Arc<dyn EvmProvider>,
) -> mfm_runtime::Result<()> {
    macro_rules! register {
        ($capability:ty, $target:expr, $provider:expr) => {{
            let binding = $target;
            let callback_target = binding.clone();
            let callback_provider = $provider;
            builder.register_adapter::<$capability, EvmPhysicalTarget, _>(
                binding,
                move |_intent_value_ref, intent| {
                    let target = callback_target.clone();
                    let provider = Arc::clone(&callback_provider);
                    Box::pin(async move { read(&target, provider.as_ref(), intent).await })
                },
            )
        }};
    }
    register!(EvmChainIdentityRead, target.clone(), Arc::clone(&provider))?;
    register!(EvmAnchorRead, target.clone(), Arc::clone(&provider))?;
    register!(EvmBalanceRead, target, provider)
}

/// Registers the generic anchored contract-call Read callback for one transaction route.
pub fn register_evm_anchored_contract_calls(
    builder: &mut RuntimeAssemblyBuilder,
    route: EvmTransactionRoute,
    provider: Arc<dyn EvmProvider>,
) -> mfm_runtime::Result<()> {
    let callback_route = route.clone();
    builder.register_adapter::<EvmAnchoredContractCallRead, EvmTransactionRoute, _>(
        route,
        move |_intent_value_ref, intent| {
            let route = callback_route.clone();
            let provider = Arc::clone(&provider);
            Box::pin(async move { read_anchored(&route, provider.as_ref(), intent).await })
        },
    )
}

async fn read_anchored(
    route: &EvmTransactionRoute,
    provider: &dyn EvmProvider,
    intent: &EvmReadIntent,
) -> std::result::Result<EvmReadEvidence, AdapterError> {
    let (operation, chain_id) = intent.operation_and_chain_id();
    let binding_ref = route.binding_ref().map_err(|_| AdapterError::Internal)?;
    if operation != EVM_ANCHORED_CONTRACT_CALL_OPERATION_ID
        || !matches!(
            intent.subject(),
            EvmReadSubject::AnchoredContractCall { .. }
        )
        || chain_id != route.chain_instance().chain_id()
        || intent.route_ref() != &binding_ref
    {
        return Err(AdapterError::Internal);
    }
    let operation = StableId::new(operation).map_err(|_| AdapterError::Internal)?;
    let request_bytes = serde_json::to_vec(intent).map_err(|_| AdapterError::Internal)?;
    if request_bytes.len() > MAX_EVM_REQUEST_BYTES {
        return Err(AdapterError::Internal);
    }
    map_provider_response(provider.request(operation, request_bytes).await?)
}

async fn read(
    target: &EvmPhysicalTarget,
    provider: &dyn EvmProvider,
    intent: &EvmReadIntent,
) -> std::result::Result<EvmReadEvidence, AdapterError> {
    let (operation, chain_id) = intent.operation_and_chain_id();
    let binding_ref = target.binding_ref().map_err(|_| AdapterError::Internal)?;
    if chain_id != target.chain_id() || intent.route_ref() != &binding_ref {
        return Err(AdapterError::Internal);
    }
    let operation = StableId::new(operation).map_err(|_| AdapterError::Internal)?;
    let request_bytes = serde_json::to_vec(intent).map_err(|_| AdapterError::Internal)?;
    if request_bytes.len() > MAX_EVM_REQUEST_BYTES {
        return Err(AdapterError::Internal);
    }
    map_provider_response(provider.request(operation, request_bytes).await?)
}

fn map_provider_response(
    response: EvmProviderResponse,
) -> std::result::Result<EvmReadEvidence, AdapterError> {
    match response {
        EvmProviderResponse::Read(value) => Ok(EvmReadEvidence::Returned { value }),
        EvmProviderResponse::Rejected => Ok(EvmReadEvidence::Rejected),
        EvmProviderResponse::SafeFailure => Ok(EvmReadEvidence::SafeFailure),
        EvmProviderResponse::IntegrityBlocked => Ok(EvmReadEvidence::IntegrityBlocked),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};

    use super::*;

    struct Provider {
        calls: AtomicUsize,
        response: EvmProviderResponse,
    }

    impl EvmProvider for Provider {
        fn request<'a>(
            &'a self,
            _operation: StableId,
            _request_bytes: Vec<u8>,
        ) -> Pin<
            Box<
                dyn Future<Output = std::result::Result<EvmProviderResponse, AdapterError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(self.response.clone())
            })
        }
    }

    fn target(chain_id: u64, endpoint_byte: u8) -> EvmPhysicalTarget {
        let endpoint = ContentRef::new(
            SchemaId::new(
                "mfm.test.endpoint",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([1; 32]),
            )
            .expect("schema"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                DigestBytes::from_array([endpoint_byte; 32]),
            ),
        )
        .expect("endpoint");
        EvmPhysicalTarget::new(NonZeroU64::new(chain_id).expect("nonzero chain"), endpoint)
    }

    fn intent(target: &EvmPhysicalTarget) -> EvmReadIntent {
        serde_json::from_value(serde_json::json!({
            "operation": "mfm.evm.read-chain-identity@1",
            "chain_id": target.chain_id(),
            "subject": { "kind": "chain_identity" },
            "route_ref": target.binding_ref().expect("binding")
        }))
        .expect("intent")
    }

    fn assert_send<T: Send>(_: &T) {}

    #[tokio::test]
    async fn every_typed_provider_response_maps_to_exact_evidence() {
        let target = target(1, 2);
        let cases = [
            (
                EvmProviderResponse::Read(EvmReadValue::ChainId(
                    NonZeroU64::new(1).expect("nonzero chain"),
                )),
                EvmReadEvidence::Returned {
                    value: EvmReadValue::ChainId(NonZeroU64::new(1).expect("nonzero chain")),
                },
            ),
            (EvmProviderResponse::Rejected, EvmReadEvidence::Rejected),
            (
                EvmProviderResponse::SafeFailure,
                EvmReadEvidence::SafeFailure,
            ),
            (
                EvmProviderResponse::IntegrityBlocked,
                EvmReadEvidence::IntegrityBlocked,
            ),
        ];
        for (response, expected) in cases {
            let provider = Provider {
                calls: AtomicUsize::new(0),
                response,
            };
            let intent = intent(&target);
            let future = read(&target, &provider, &intent);
            assert_send(&future);
            assert_eq!(future.await.expect("accepted evidence"), expected);
            assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn target_mismatch_is_internal_and_never_enters_provider() {
        let registered = target(1, 2);
        let provider = Provider {
            calls: AtomicUsize::new(0),
            response: EvmProviderResponse::IntegrityBlocked,
        };

        let wrong_chain = target(2, 2);
        assert_eq!(
            read(&registered, &provider, &intent(&wrong_chain)).await,
            Err(AdapterError::Internal)
        );
        let wrong_route = target(1, 3);
        assert_eq!(
            read(&registered, &provider, &intent(&wrong_route)).await,
            Err(AdapterError::Internal)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn one_target_registers_three_capabilities_and_rejects_duplicate_keys() {
        let target = target(1, 2);
        let binding = target.binding_ref().expect("binding");
        let first_provider: Arc<dyn EvmProvider> = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            response: EvmProviderResponse::Rejected,
        });
        let second_provider: Arc<dyn EvmProvider> = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            response: EvmProviderResponse::SafeFailure,
        });

        let mut first = RuntimeAssemblyBuilder::new().expect("builder");
        register_evm_reads(&mut first, target.clone(), first_provider).expect("three callbacks");
        assert_eq!(target.binding_ref().expect("stable binding"), binding);
        assert_eq!(
            register_evm_reads(&mut first, target.clone(), second_provider.clone()),
            Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
        );

        let mut replacement = RuntimeAssemblyBuilder::new().expect("builder");
        register_evm_reads(&mut replacement, target.clone(), second_provider)
            .expect("replacement handle");
        assert_eq!(target.binding_ref().expect("durable identity"), binding);
        first.finish();
        replacement.finish();
    }

    #[tokio::test]
    async fn anchored_registration_is_route_keyed_and_rejects_other_read_intents_locally() {
        let physical = target(1, 2);
        let route = EvmTransactionRoute::new(
            mfm_evm::EvmChainInstance::new(
                NonZeroU64::new(1).expect("nonzero chain"),
                mfm_evm::EvmHash::new(
                    "0x1111111111111111111111111111111111111111111111111111111111111111",
                )
                .expect("genesis"),
            ),
            physical.endpoint_ref().clone(),
        );
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
            response: EvmProviderResponse::Rejected,
        });
        assert_eq!(
            read_anchored(&route, provider.as_ref(), &intent(&physical)).await,
            Err(AdapterError::Internal)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

        let provider: Arc<dyn EvmProvider> = provider;
        let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
        register_evm_anchored_contract_calls(&mut builder, route.clone(), Arc::clone(&provider))
            .expect("anchored callback");
        assert_eq!(
            register_evm_anchored_contract_calls(&mut builder, route, provider),
            Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
        );
        builder.finish();
    }
}
