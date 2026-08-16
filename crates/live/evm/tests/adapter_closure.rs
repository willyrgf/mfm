use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{EvmCapability, EvmState};
use mfm_evm_live::{
    evm_live_adapter_implementation_ref, EvmAdapterBinding, EvmAdapterError, EvmPhysicalTarget,
    EvmProvider, EvmProviderResponse,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
use mfm_portfolio::PortfolioContinuation;
use mfm_program::{capability_contract_ref, state_implementation_ref, BindingDescriptor, State};
use mfm_runtime::BoxFuture;

struct CountingProvider(Arc<AtomicUsize>);

impl EvmProvider for CountingProvider {
    fn request(
        &self,
        _call_id: StableId,
        _operation: StableId,
        _request_bytes: Vec<u8>,
    ) -> BoxFuture<std::result::Result<EvmProviderResponse, EvmAdapterError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(EvmAdapterError::Authentication) })
    }
}

fn reference(label: &str) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.evm-live-adapter-test",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema"),
        raw_content_digest(label.as_bytes()),
    )
    .expect("reference")
}

fn target(label: &str) -> EvmPhysicalTarget {
    EvmPhysicalTarget {
        chain_id: 1,
        endpoint_ref: reference(label),
    }
}

fn descriptor<S, C>(
    target: &EvmPhysicalTarget,
    effect_domain: Option<StableId>,
) -> BindingDescriptor
where
    S: State,
    C: AccessCapabilityContract,
{
    BindingDescriptor::new(
        state_implementation_ref::<S>().expect("state"),
        Some(capability_contract_ref::<C>().expect("capability")),
        Some(evm_live_adapter_implementation_ref().expect("adapter")),
        target.content_ref().expect("target"),
        effect_domain,
        None,
    )
    .expect("binding")
}

#[test]
fn only_exact_balance_read_bindings_are_accepted_without_entering_the_provider() {
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(CountingProvider(Arc::clone(&provider_calls)));
    let left_target = target("left");
    let right_target = target("right");

    let read =
        descriptor::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(&left_target, None);
    assert!(EvmAdapterBinding::read(read, left_target.clone(), provider.clone()).is_ok());

    let wrong_target =
        descriptor::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(&left_target, None);
    assert!(EvmAdapterBinding::read(wrong_target, right_target, provider.clone()).is_err());

    let wrong_role =
        descriptor::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<7>>(&left_target, None);
    assert!(EvmAdapterBinding::read(wrong_role, left_target.clone(), provider.clone()).is_err());

    let effect = descriptor::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(
        &left_target,
        Some(StableId::new("mfm.evm-live-test-effect").expect("effect")),
    );
    assert!(EvmAdapterBinding::read(effect, left_target, provider).is_err());
    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn provider_future_remains_send() {
    fn require_send<T: Send>() {}

    require_send::<BoxFuture<std::result::Result<EvmProviderResponse, EvmAdapterError>>>();
}
