use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{EvmCapability, EvmState, NonceReservationEvidence};
use mfm_evm_live::{
    evm_live_adapter_implementation_ref, public_signer_key_instance_ref,
    wallet_nonce_effect_domain, EvmAdapterBinding, EvmAdapterError, EvmPhysicalTarget, EvmProvider,
    EvmProviderResponse, WalletNonceAuthority,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId, TenantScopeId};
use mfm_program::{capability_contract_ref, state_implementation_ref, BindingDescriptor, State};
use mfm_runtime::BoxFuture;
use mfm_signing::{PublicSignerKeyInstance, Signer, SigningFuture, SigningRequest, SigningResult};

const SENDER: &str = "0x1111111111111111111111111111111111111111";
const NONCE_DOMAIN: &str = "wallet-main";

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

struct CountingNonceAuthority(Arc<AtomicUsize>);

impl WalletNonceAuthority for CountingNonceAuthority {
    fn reserve(
        self: Arc<Self>,
        _operation_key: StableId,
    ) -> BoxFuture<std::result::Result<NonceReservationEvidence, EvmAdapterError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(NonceReservationEvidence::Reserved { nonce: 0 }) })
    }
}

struct CountingSigner {
    identity: PublicSignerKeyInstance,
    calls: Arc<AtomicUsize>,
}

impl Signer for CountingSigner {
    fn public_identity(&self) -> &PublicSignerKeyInstance {
        &self.identity
    }

    fn sign(&self, _request: SigningRequest) -> SigningFuture<SigningResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(mfm_signing::SigningError::InvalidRequest) })
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
    signer: Option<ContentRef>,
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
        signer,
    )
    .expect("binding")
}

#[test]
fn invalid_adapter_role_target_effect_and_signer_never_enter_handles() {
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let nonce_calls = Arc::new(AtomicUsize::new(0));
    let signer_calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(CountingProvider(Arc::clone(&provider_calls)));
    let authority = Arc::new(CountingNonceAuthority(Arc::clone(&nonce_calls)));
    let signer = Arc::new(CountingSigner {
        identity: PublicSignerKeyInstance {
            signer_id: StableId::new("mfm.evm-live-test-signer").expect("signer"),
            key_instance_id: StableId::new("mfm.evm-live-test-key").expect("key"),
            algorithm: StableId::new("mfm.evm-live-test-algorithm").expect("algorithm"),
        },
        calls: Arc::clone(&signer_calls),
    });
    let tenant =
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").expect("tenant");
    let left_target = target("left");
    let right_target = target("right");

    let reserve = descriptor::<EvmState<0, 0>, EvmCapability<0>>(
        &left_target,
        Some(
            wallet_nonce_effect_domain(&tenant, SENDER, NONCE_DOMAIN).expect("nonce effect domain"),
        ),
        None,
    );
    assert!(EvmAdapterBinding::read(reserve, left_target.clone(), provider.clone()).is_err());

    let read = descriptor::<EvmState<0, 3>, EvmCapability<3>>(&left_target, None, None);
    assert!(EvmAdapterBinding::read(read, right_target, provider.clone()).is_err());

    let wrong_read = descriptor::<EvmState<0, 3>, EvmCapability<7>>(&left_target, None, None);
    assert!(EvmAdapterBinding::read(wrong_read, left_target.clone(), provider.clone()).is_err());

    let invalid_sender = descriptor::<EvmState<0, 2>, EvmCapability<1>>(
        &left_target,
        Some(StableId::new("mfm.evm-live-test-broadcast-effect").expect("effect")),
        Some(public_signer_key_instance_ref(signer.public_identity()).expect("signer reference")),
    );
    assert!(EvmAdapterBinding::broadcast(
        invalid_sender,
        left_target.clone(),
        "0xABC".to_owned(),
        NONCE_DOMAIN.to_owned(),
        provider.clone(),
        signer.clone(),
    )
    .is_err());

    let wrong_effect = descriptor::<EvmState<0, 0>, EvmCapability<0>>(
        &left_target,
        Some(StableId::new("mfm.evm-live-test-wrong-effect").expect("effect")),
        None,
    );
    assert!(EvmAdapterBinding::reserve_nonce(
        wrong_effect,
        left_target.clone(),
        tenant,
        SENDER.to_owned(),
        NONCE_DOMAIN.to_owned(),
        authority,
    )
    .is_err());

    let wrong_signer = descriptor::<EvmState<0, 2>, EvmCapability<1>>(
        &left_target,
        Some(StableId::new("mfm.evm-live-test-broadcast-effect").expect("effect")),
        Some(reference("other-signer")),
    );
    assert!(EvmAdapterBinding::broadcast(
        wrong_signer,
        left_target,
        SENDER.to_owned(),
        NONCE_DOMAIN.to_owned(),
        provider,
        signer,
    )
    .is_err());

    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(nonce_calls.load(Ordering::SeqCst), 0);
    assert_eq!(signer_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn signer_identity_has_one_stable_public_content_reference() {
    let identity = PublicSignerKeyInstance {
        signer_id: StableId::new("mfm.evm-live-test-signer").expect("signer"),
        key_instance_id: StableId::new("mfm.evm-live-test-key").expect("key"),
        algorithm: StableId::new("mfm.evm-live-test-algorithm").expect("algorithm"),
    };
    assert_eq!(
        public_signer_key_instance_ref(&identity).expect("first reference"),
        public_signer_key_instance_ref(&identity).expect("second reference")
    );
}

#[test]
fn provider_future_remains_send() {
    fn require_send<T: Send>() {}

    require_send::<BoxFuture<std::result::Result<EvmProviderResponse, EvmAdapterError>>>();
}
