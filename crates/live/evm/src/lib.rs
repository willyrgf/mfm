#![warn(missing_docs)]
//! Bounded EVM provider registration and durable transaction execution.
//!
//! The live adapter owns provider ingress, exact transaction wire encoding, and
//! signer/authority/provider orchestration, plus a pure transaction State registration helper.
//! Runtime construction remains a trusted composition responsibility.

use mfm_evm::ReadCapabilityFamily;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, EvmAnchorRead,
    EvmAnchoredContractCallRead, EvmBalanceRead, EvmChainIdentityRead, EvmPhysicalTarget,
    EvmReadEvidence, EvmReadIntent, EvmTransactionRoute,
};
use mfm_ids::ContentRef;
use mfm_runtime::{AdapterError, RuntimeAssemblyBuilder};

mod assembly;
pub use assembly::register_evm_transaction_states;

mod codec;
mod json_rpc;
mod transaction;

pub use json_rpc::{
    EvmAdapterLocator, EvmProviderBuildError, JsonRpcEvmProvider, MAX_EVM_ADAPTER_LOCATOR_BYTES,
};
pub use transaction::{
    register_evm_transaction_adapters, EvmTransactionProvider, ProviderReceipt,
    ProviderReceiptResult, EVM_EIP1559_SIGNING_PURPOSE_ID,
};

pub use codec::{ethereum_address, evm_keccak256, EvmCodecError};

/// Duplicate-safe typed provider future.
pub type ProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, AdapterError>> + Send + 'a>>;

/// Typed observational provider paired with one immutable public target.
///
/// The provider receives only checked domain values and Runtime's exact canonical intent value
/// reference. It must preserve that reference in every returned evidence variant. Broad balance
/// reads remain anchored to the subject's block, while anchored calls bracket code and call
/// observations with the exact authored block.
pub trait EvmReadProvider: Send + Sync + 'static {
    /// Observes one broad balance-collection read intent.
    fn observe<'a>(
        &'a self,
        intent_value_ref: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence>;

    /// Observes one exact anchored contract-call intent.
    fn observe_anchored_call<'a>(
        &'a self,
        intent_value_ref: &'a ContentRef,
        intent: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence>;
}

/// Registers the three surviving EVM Read capability callbacks for one target.
pub fn register_evm_reads(
    builder: &mut RuntimeAssemblyBuilder,
    target: EvmPhysicalTarget,
    provider: Arc<dyn EvmReadProvider>,
) -> mfm_runtime::Result<()> {
    macro_rules! register {
        ($capability:ty, $family:ident, $target:expr, $provider:expr) => {{
            let binding = $target;
            let callback_target = binding.clone();
            let callback_ref = binding
                .binding_ref()
                .map_err(|_| mfm_runtime::RuntimeError::Internal)?;
            let callback_provider = $provider;
            builder.register_adapter::<$capability, EvmPhysicalTarget, _>(
                binding,
                move |intent_value_ref, intent| {
                    let target = callback_target.clone();
                    let binding_ref = callback_ref.clone();
                    let provider = Arc::clone(&callback_provider);
                    Box::pin(async move {
                        read(
                            &target,
                            &binding_ref,
                            provider.as_ref(),
                            ReadCapabilityFamily::$family,
                            intent_value_ref,
                            intent,
                        )
                        .await
                    })
                },
            )
        }};
    }
    register!(
        EvmChainIdentityRead,
        ChainIdentity,
        target.clone(),
        Arc::clone(&provider)
    )?;
    register!(EvmAnchorRead, Anchor, target.clone(), Arc::clone(&provider))?;
    register!(EvmBalanceRead, Balance, target, provider)
}

/// Registers the generic anchored contract-call Read callback for one transaction route.
pub fn register_evm_anchored_contract_calls(
    builder: &mut RuntimeAssemblyBuilder,
    route: EvmTransactionRoute,
    provider: Arc<dyn EvmReadProvider>,
) -> mfm_runtime::Result<()> {
    let callback_route = route.clone();
    let callback_ref = route
        .binding_ref()
        .map_err(|_| mfm_runtime::RuntimeError::Internal)?;
    builder.register_adapter::<EvmAnchoredContractCallRead, EvmTransactionRoute, _>(
        route,
        move |intent_value_ref, intent| {
            let route = callback_route.clone();
            let binding_ref = callback_ref.clone();
            let provider = Arc::clone(&provider);
            Box::pin(async move {
                read_anchored(
                    &route,
                    &binding_ref,
                    provider.as_ref(),
                    intent_value_ref,
                    intent,
                )
                .await
            })
        },
    )
}

async fn read_anchored(
    route: &EvmTransactionRoute,
    binding_ref: &ContentRef,
    provider: &dyn EvmReadProvider,
    intent_value_ref: &ContentRef,
    intent: &AnchoredContractCallIntent,
) -> std::result::Result<AnchoredContractCallEvidence, AdapterError> {
    if intent.chain_id() != route.chain_instance.chain_id || intent.route_ref() != binding_ref {
        return Err(AdapterError::Internal);
    }
    provider
        .observe_anchored_call(intent_value_ref, intent)
        .await
}

async fn read(
    target: &EvmPhysicalTarget,
    binding_ref: &ContentRef,
    provider: &dyn EvmReadProvider,
    registration: ReadCapabilityFamily,
    intent_value_ref: &ContentRef,
    intent: &EvmReadIntent,
) -> std::result::Result<EvmReadEvidence, AdapterError> {
    if intent.chain_id() != target.chain_id
        || intent.route_ref() != binding_ref
        || !registration.accepts(intent.subject())
    {
        return Err(AdapterError::Internal);
    }
    provider.observe(intent_value_ref, intent).await
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mfm_evm::{
        AnchoredContractCallResult, EvmAddress, EvmBlockAnchor, EvmHash, EvmReadSubject,
        EvmReadValue, EvmU256,
    };
    use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};

    use super::*;

    struct Provider {
        calls: AtomicUsize,
    }

    impl EvmReadProvider for Provider {
        fn observe<'a>(
            &'a self,
            intent_value_ref: &'a ContentRef,
            intent: &'a EvmReadIntent,
        ) -> ProviderFuture<'a, EvmReadEvidence> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(EvmReadEvidence::returned(
                    intent_value_ref.clone(),
                    EvmReadValue::ChainId(intent.chain_id()),
                ))
            })
        }

        fn observe_anchored_call<'a>(
            &'a self,
            intent_value_ref: &'a ContentRef,
            intent: &'a AnchoredContractCallIntent,
        ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let result = AnchoredContractCallResult::new(intent.anchor().clone(), vec![])
                    .map_err(|_| AdapterError::Internal)?;
                Ok(AnchoredContractCallEvidence::returned(
                    intent_value_ref.clone(),
                    result,
                ))
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
        EvmPhysicalTarget {
            chain_id: NonZeroU64::new(chain_id).expect("nonzero chain"),
            endpoint_ref: endpoint,
        }
    }

    fn intent(target: &EvmPhysicalTarget) -> EvmReadIntent {
        EvmReadIntent::new(
            target.chain_id,
            target.binding_ref().expect("binding"),
            EvmReadSubject::ChainIdentity,
        )
        .expect("intent")
    }

    fn route(physical: &EvmPhysicalTarget) -> EvmTransactionRoute {
        EvmTransactionRoute {
            chain_instance: mfm_evm::EvmChainInstance {
                chain_id: physical.chain_id,
                expected_genesis_hash: EvmHash::new(
                    "0x1111111111111111111111111111111111111111111111111111111111111111",
                )
                .expect("genesis"),
            },
            endpoint_ref: physical.endpoint_ref.clone(),
        }
    }

    fn anchored_intent(route: &EvmTransactionRoute) -> AnchoredContractCallIntent {
        AnchoredContractCallIntent::new(
            route.chain_instance.chain_id,
            route.binding_ref().expect("binding"),
            EvmBlockAnchor {
                number: EvmU256::from_u64(7),
                hash: EvmHash::new(
                    "0x2222222222222222222222222222222222222222222222222222222222222222",
                )
                .expect("anchor hash"),
            },
            EvmAddress::new("0x3333333333333333333333333333333333333333").expect("target"),
            vec![1, 2],
        )
        .expect("anchored intent")
    }

    fn assert_send<T: Send>(_: &T) {}

    #[tokio::test]
    async fn typed_provider_receives_and_preserves_the_runtime_intent_ref() {
        let target = target(1, 2);
        let provider = Provider {
            calls: AtomicUsize::new(0),
        };
        let intent = intent(&target);
        let intent_value_ref = target.endpoint_ref.clone();
        let binding_ref = target.binding_ref().unwrap();
        let future = read(
            &target,
            &binding_ref,
            &provider,
            ReadCapabilityFamily::ChainIdentity,
            &intent_value_ref,
            &intent,
        );
        assert_send(&future);
        let evidence = future.await.expect("accepted evidence");
        assert_eq!(evidence.intent_value_ref(), &intent_value_ref);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn target_mismatch_is_internal_and_never_enters_provider() {
        let registered = target(1, 2);
        let provider = Provider {
            calls: AtomicUsize::new(0),
        };
        let intent_value_ref = registered.endpoint_ref.clone();

        let wrong_chain = target(2, 2);
        assert_eq!(
            read(
                &registered,
                &registered.binding_ref().unwrap(),
                &provider,
                ReadCapabilityFamily::ChainIdentity,
                &intent_value_ref,
                &intent(&wrong_chain),
            )
            .await,
            Err(AdapterError::Internal)
        );
        let wrong_route = target(1, 3);
        assert_eq!(
            read(
                &registered,
                &registered.binding_ref().unwrap(),
                &provider,
                ReadCapabilityFamily::ChainIdentity,
                &intent_value_ref,
                &intent(&wrong_route),
            )
            .await,
            Err(AdapterError::Internal)
        );
        assert_eq!(
            read(
                &registered,
                &registered.binding_ref().unwrap(),
                &provider,
                ReadCapabilityFamily::Balance,
                &intent_value_ref,
                &intent(&registered),
            )
            .await,
            Err(AdapterError::Internal)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn one_target_registers_three_capabilities_and_rejects_duplicate_keys() {
        let target = target(1, 2);
        let binding = target.binding_ref().expect("binding");
        let first_provider: Arc<dyn EvmReadProvider> = Arc::new(Provider {
            calls: AtomicUsize::new(0),
        });
        let second_provider: Arc<dyn EvmReadProvider> = Arc::new(Provider {
            calls: AtomicUsize::new(0),
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
    async fn anchored_registration_is_route_keyed_and_rejects_mismatches_locally() {
        let physical = target(1, 2);
        let registered_route = route(&physical);
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
        });
        let wrong = route(&target(1, 3));
        let intent = anchored_intent(&wrong);
        let intent_value_ref = physical.endpoint_ref.clone();
        assert_eq!(
            read_anchored(
                &registered_route,
                &registered_route.binding_ref().unwrap(),
                provider.as_ref(),
                &intent_value_ref,
                &intent,
            )
            .await,
            Err(AdapterError::Internal)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

        let provider: Arc<dyn EvmReadProvider> = provider;
        let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
        register_evm_anchored_contract_calls(
            &mut builder,
            registered_route.clone(),
            Arc::clone(&provider),
        )
        .expect("anchored callback");
        assert_eq!(
            register_evm_anchored_contract_calls(&mut builder, registered_route, provider),
            Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
        );
        builder.finish();
    }
}
