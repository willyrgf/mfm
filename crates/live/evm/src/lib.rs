#![warn(missing_docs)]
//! Explicit native resources, bounded EVM providers, and durable transaction execution.
//!
//! The live adapter owns provider ingress, exact transaction wire encoding, and
//! signer/authority/provider orchestration. Native clients own configuration and product conversion;
//! complete Programs bind explicit resources through the compiler environment.

use mfm_evm::ReadCapabilityFamily;

use std::future::Future;
use std::pin::Pin;

use crate::error::{invariant, AdapterFailure};
use mfm_capabilities::AdapterError;
/// Native product clients with explicit configuration and retained-output boundaries.
pub mod client;
mod error;
use mfm_evm::EvmOperationalError;
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, EvmPhysicalTarget, EvmReadEvidence,
    EvmReadIntent, EvmTransactionRoute,
};
use mfm_ids::ContentRef;

mod resources;
pub use resources::{EvmBindingView, EvmResources, EvmTransactionResource, MAX_EVM_BINDINGS};

mod codec;
mod json_rpc;
mod transaction;

pub use json_rpc::{
    EvmAdapterLocator, EvmLocatorCheck, EvmProviderBuildError, JsonRpcEvmProvider,
    MAX_EVM_ADAPTER_LOCATOR_BYTES,
};
pub use transaction::{
    EvmTransactionProvider, ProviderReceipt, ProviderReceiptResult, EVM_EIP1559_SIGNING_PURPOSE_ID,
};

pub use codec::{ethereum_address, evm_keccak256, EvmCodecError};

/// Duplicate-safe typed provider future.
pub type ProviderFuture<'a, T> = Pin<
    Box<dyn Future<Output = std::result::Result<T, AdapterError<EvmOperationalError>>> + Send + 'a>,
>;

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

async fn read_anchored(
    route: &EvmTransactionRoute,
    binding_ref: &ContentRef,
    provider: &dyn EvmReadProvider,
    intent_value_ref: &ContentRef,
    intent: &AnchoredContractCallIntent,
) -> std::result::Result<AnchoredContractCallEvidence, AdapterError<EvmOperationalError>> {
    if intent.chain_id() != route.chain_instance.chain_id || intent.route_ref() != binding_ref {
        return Err(invariant(AdapterFailure::ReadBinding {
            expected_route: binding_ref.clone(),
            observed_route: intent.route_ref().clone(),
            expected_chain: route.chain_instance.chain_id.get(),
            observed_chain: intent.chain_id().get(),
        }));
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
) -> std::result::Result<EvmReadEvidence, AdapterError<EvmOperationalError>> {
    if intent.chain_id() != target.chain_id || intent.route_ref() != binding_ref {
        return Err(invariant(AdapterFailure::ReadBinding {
            expected_route: binding_ref.clone(),
            observed_route: intent.route_ref().clone(),
            expected_chain: target.chain_id.get(),
            observed_chain: intent.chain_id().get(),
        }));
    }
    if !registration.accepts(intent.subject()) {
        return Err(invariant(AdapterFailure::ReadSubject {
            expected: registration,
            observed: intent.subject().clone(),
        }));
    }
    provider.observe(intent_value_ref, intent).await
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
                    .map_err(invariant)?;
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
            0,
            mfm_chain::balance::DecimalScale::new(18).unwrap(),
            target.chain_id,
            mfm_evm::EvmBalanceTarget::new(EvmAddress::from_bytes([1; 20]), None),
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

    // The adapter must carry Runtime's exact request identity through the provider into evidence.
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

    // A local route or intent mismatch must retain diagnostic context without issuing a provider call.
    #[tokio::test]
    async fn target_mismatch_is_internal_and_never_enters_provider() {
        let registered = target(1, 2);
        let provider = Provider {
            calls: AtomicUsize::new(0),
        };
        for (other, family, expected_kind) in [
            (
                target(2, 2),
                ReadCapabilityFamily::ChainIdentity,
                "read_binding",
            ),
            (
                target(1, 3),
                ReadCapabilityFamily::ChainIdentity,
                "read_binding",
            ),
            (
                registered.clone(),
                ReadCapabilityFamily::Balance,
                "read_subject",
            ),
        ] {
            let error = read(
                &registered,
                &registered.binding_ref().unwrap(),
                &provider,
                family,
                &registered.endpoint_ref,
                &intent(&other),
            )
            .await
            .unwrap_err();
            let AdapterError::Invariant(cause) = error else {
                panic!("local invariant")
            };
            let fields = cause.details().as_value();
            assert!(fields.get(expected_kind).is_some());
            if let Some(binding) = fields.get("read_binding") {
                assert_eq!(binding["expected_chain"], registered.chain_id.get());
                assert_eq!(binding["observed_chain"], other.chain_id.get());
                assert_eq!(
                    binding["expected_route"],
                    serde_json::to_value(registered.binding_ref().unwrap()).unwrap()
                );
                assert_eq!(
                    binding["observed_route"],
                    serde_json::to_value(other.binding_ref().unwrap()).unwrap()
                );
            }
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn exact_resource_routes_reject_duplicates_and_preserve_identity_across_handles() {
        let route = mfm_evm::EvmBalanceRoute::new(
            NonZeroU64::new(1).unwrap(),
            mfm_evm::EvmEndpoint::new("endpoint").unwrap(),
        );
        let first: Arc<dyn EvmReadProvider> = Arc::new(Provider {
            calls: AtomicUsize::new(0),
        });
        let second: Arc<dyn EvmReadProvider> = Arc::new(Provider {
            calls: AtomicUsize::new(0),
        });
        assert!(EvmResources::<()>::new(
            vec![
                (route.clone(), first.clone()),
                (route.clone(), second.clone())
            ],
            vec![]
        )
        .is_err());
        let hot = EvmResources::<()>::new(vec![(route.clone(), first)], vec![]).unwrap();
        let cold = EvmResources::<()>::new(vec![(route, second)], vec![]).unwrap();
        assert_eq!(hot.bindings().unwrap(), cold.bindings().unwrap());
    }

    // An anchored call must use its registered route; mismatches must fail before provider IO.
    #[tokio::test]
    async fn anchored_binding_rejects_mismatches_before_provider_io() {
        let physical = target(1, 2);
        let registered_route = route(&physical);
        let provider = Arc::new(Provider {
            calls: AtomicUsize::new(0),
        });
        let wrong = route(&target(1, 3));
        let intent = anchored_intent(&wrong);
        let intent_value_ref = physical.endpoint_ref.clone();
        let error = read_anchored(
            &registered_route,
            &registered_route.binding_ref().unwrap(),
            provider.as_ref(),
            &intent_value_ref,
            &intent,
        )
        .await
        .unwrap_err();
        let AdapterError::Invariant(cause) = error else {
            panic!("local anchored binding")
        };
        assert!(cause.details().as_value().get("read_binding").is_some());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}
