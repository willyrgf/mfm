#![warn(missing_docs)]
//! Direct observational EVM provider registration.
//!
//! The live adapter owns only the provider boundary. Domain State registration
//! and Runtime construction remain trusted composition responsibilities.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_evm::{EvmCapability, EvmPhysicalTarget, EvmReadEvidence, EvmReadIntent, EvmReadValue};
use mfm_ids::StableId;
use mfm_runtime::{ReadAdapterError, RuntimeAssemblyBuilder};

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
pub trait EvmProvider: Send + Sync + 'static {
    /// Performs one observational request for the supplied operation.
    fn request<'a>(
        &'a self,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<EvmProviderResponse, ReadAdapterError>>
                + Send
                + 'a,
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
            builder.register_adapter::<$capability, EvmPhysicalTarget, _>(binding, move |intent| {
                let target = callback_target.clone();
                let provider = Arc::clone(&callback_provider);
                Box::pin(async move { read(&target, provider.as_ref(), intent).await })
            })
        }};
    }
    register!(EvmCapability<2>, target.clone(), Arc::clone(&provider))?;
    register!(EvmCapability<6>, target.clone(), Arc::clone(&provider))?;
    register!(EvmCapability<7>, target, provider)
}

async fn read(
    target: &EvmPhysicalTarget,
    provider: &dyn EvmProvider,
    intent: &EvmReadIntent,
) -> std::result::Result<EvmReadEvidence, ReadAdapterError> {
    let (operation, chain_id) = intent.operation_and_chain_id();
    let binding_ref = target
        .binding_ref()
        .map_err(|_| ReadAdapterError::Internal)?;
    if chain_id != target.chain_id() || intent.route_ref() != &binding_ref {
        return Err(ReadAdapterError::Internal);
    }
    let operation = StableId::new(operation).map_err(|_| ReadAdapterError::Internal)?;
    let request_bytes = serde_json::to_vec(intent).map_err(|_| ReadAdapterError::Internal)?;
    if request_bytes.len() > MAX_EVM_REQUEST_BYTES {
        return Err(ReadAdapterError::Internal);
    }
    match provider.request(operation, request_bytes).await? {
        EvmProviderResponse::Read(value) => Ok(EvmReadEvidence::Returned { value }),
        EvmProviderResponse::Rejected => Ok(EvmReadEvidence::Rejected),
        EvmProviderResponse::SafeFailure => Ok(EvmReadEvidence::SafeFailure),
        EvmProviderResponse::IntegrityBlocked => Ok(EvmReadEvidence::IntegrityBlocked),
    }
}

#[cfg(test)]
mod tests {
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
                dyn Future<Output = std::result::Result<EvmProviderResponse, ReadAdapterError>>
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
        EvmPhysicalTarget::new(chain_id, endpoint).expect("target")
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
                EvmProviderResponse::Read(EvmReadValue::ChainId(1)),
                EvmReadEvidence::Returned {
                    value: EvmReadValue::ChainId(1),
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
            Err(ReadAdapterError::Internal)
        );
        let wrong_route = target(1, 3);
        assert_eq!(
            read(&registered, &provider, &intent(&wrong_route)).await,
            Err(ReadAdapterError::Internal)
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

        let mut first = RuntimeAssemblyBuilder::new();
        register_evm_reads(&mut first, target.clone(), first_provider).expect("three callbacks");
        assert_eq!(target.binding_ref().expect("stable binding"), binding);
        assert_eq!(
            register_evm_reads(&mut first, target.clone(), second_provider.clone()),
            Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
        );

        let mut replacement = RuntimeAssemblyBuilder::new();
        register_evm_reads(&mut replacement, target.clone(), second_provider)
            .expect("replacement handle");
        assert_eq!(target.binding_ref().expect("durable identity"), binding);
        first.finish().expect("first assembly");
        replacement.finish().expect("replacement assembly");
    }
}
