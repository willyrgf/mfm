//! Actual balance injection and cold association with an adapter that forbids IO.
//! Maintained source planning uses a fixture resource environment, not Application or Runtime.
use std::{future::Future, num::NonZeroU64, pin::Pin};

use mfm_capabilities::{AdapterError, ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_chain::balance::{
    BalanceCollectionMetadata, BalanceContext, BalanceExecutionConfig, BalanceRead, BalanceRequest,
    BalanceSource, BalanceSourceDefinition, ConsolidateBalanceCollection, DecimalScale,
    ObserveBalance, PreparedBalance,
};
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_evm::*;
use mfm_ids::{ContentRef, EntryPointId};
use mfm_program::{
    compile, load, BindRead, CapabilityFamily, Execution, Operation, ProgramEnvironment,
    ProgramLimits, Pure, ReadState, Resolve,
};
use mfm_values::{InvocationDiagnostic, Object, Unsigned256};

struct ConstructionOnly<const COLD: bool>;
impl<const COLD: bool> ProgramEnvironment for ConstructionOnly<COLD> {
    type Sources = (
        Operation<BalanceSourceDefinition<Unsigned256>>,
        Pure<ConsolidateBalanceCollection<Unsigned256>>,
    );
}
impl<const COLD: bool> CapabilityFamily<BalanceRead> for ConstructionOnly<COLD> {
    type Implementations = (EvmNativeBalance, EvmTokenBalance);
}
impl Resolve<BalanceSourceDefinition<Unsigned256>, BalanceRead> for ConstructionOnly<false> {
    fn implementation(
        source: &BalanceSourceDefinition<Unsigned256>,
    ) -> mfm_program::Result<mfm_ids::StableId> {
        Ok(
            if source
                .source()
                .target()
                .native()
                .decode::<EvmBalanceTarget>()
                .map_err(mfm_program::ProgramError::Diagnostic)?
                .token()
                .is_some()
            {
                EvmTokenBalance::implementation_id()?
            } else {
                EvmNativeBalance::implementation_id()?
            },
        )
    }
}
impl<const COLD: bool> ReadAdapter<EvmReadIntent, EvmReadEvidence, EvmOperationalError>
    for ConstructionOnly<COLD>
{
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a EvmReadIntent,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<EvmReadEvidence, AdapterError<EvmOperationalError>>>
                + Send
                + 'a,
        >,
    > {
        panic!("construction and cold association must not invoke provider IO")
    }
}
impl<C, I, const COLD: bool> BindRead<C, I> for ConstructionOnly<COLD>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<
        C,
        Binding = EvmBalanceBinding,
        NativeIntent = EvmReadIntent,
        NativeEvidence = EvmReadEvidence,
        OperationalError = EvmOperationalError,
    >,
{
    type Adapter = Self;
    fn bind_read(&self, _: &EvmBalanceBinding) -> Result<Self, InvocationDiagnostic> {
        Ok(Self)
    }
}

#[tokio::test]
async fn cold_balance_program_rejects_a_different_designated_route_before_provider_io() {
    let chain = NonZeroU64::new(1).unwrap();
    let endpoint = Object::from_value(&EvmEndpoint::new("fixture-rpc").unwrap()).unwrap();
    let route = EvmPhysicalTarget {
        chain_id: chain,
        endpoint_ref: endpoint.value_ref().clone(),
    };
    let route_object = Object::from_value(&route).unwrap();
    let ledger = LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(chain)).unwrap());
    let native = EvmBalanceTarget::new(EvmAddress::from_bytes([1; 20]), None);
    let token = EvmBalanceTarget::new(
        EvmAddress::from_bytes([1; 20]),
        Some(EvmAddress::from_bytes([2; 20])),
    );
    let scale = DecimalScale::new(6).unwrap();
    let context = BalanceContext::new(
        BalanceRequest::new(
            vec![
                BalanceSource::new(
                    "native".into(),
                    BalanceTarget::new(ledger.clone(), Object::from_value(&native).unwrap()),
                )
                .unwrap(),
                BalanceSource::new(
                    "token".into(),
                    BalanceTarget::new(ledger, Object::from_value(&token).unwrap()),
                )
                .unwrap(),
            ],
            scale,
        )
        .unwrap(),
        Unsigned256::from_u64(42),
        BalanceCollectionMetadata::new(0, "collection".into(), route_object.value_ref().clone())
            .unwrap(),
    );
    let source = (
        Operation::new(BalanceSourceDefinition::<Unsigned256>::new(
            context.request().sources()[0].clone(),
            0,
            scale,
            BalanceExecutionConfig::new(
                route_object.value_ref().clone(),
                Object::from_value(&EvmBalanceRoute::new(
                    chain,
                    EvmEndpoint::new("fixture-rpc").unwrap(),
                ))
                .unwrap(),
            ),
        )),
        Operation::new(BalanceSourceDefinition::<Unsigned256>::new(
            context.request().sources()[1].clone(),
            1,
            scale,
            BalanceExecutionConfig::new(
                route_object.value_ref().clone(),
                Object::from_value(&EvmBalanceRoute::new(
                    chain,
                    EvmEndpoint::new("fixture-rpc").unwrap(),
                ))
                .unwrap(),
            ),
        )),
        Pure::<ConsolidateBalanceCollection<Unsigned256>>::default(),
    );
    let program = compile(
        EntryPointId::new("mfm.proof/balance@1").unwrap(),
        &source,
        &context,
        &ConstructionOnly::<false>,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_eq!(program.declarations().len(), 10);
    assert_eq!(program.bindings().len(), 2);
    let cold = load(program.canonical_bytes(), &ConstructionOnly::<true>).unwrap();
    assert_eq!(cold.content_ref(), program.content_ref());
    assert_eq!(cold.canonical_bytes(), program.canonical_bytes());

    // Cold association admits an independently valid selected binding; execution must still
    // compare it with the caller's retained route before IO.
    let other_endpoint = Object::from_value(&EvmEndpoint::new("other-rpc").unwrap()).unwrap();
    let other_route = EvmPhysicalTarget {
        chain_id: chain,
        endpoint_ref: other_endpoint.value_ref().clone(),
    };
    let alternate = Object::from_value(&EvmBalanceBinding::new(
        other_route.clone(),
        0,
        context
            .active_source()
            .unwrap()
            .target()
            .native()
            .decode()
            .unwrap(),
        scale,
    ))
    .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_slice(program.canonical_bytes()).unwrap();
    document["declarations"][2]["execution"]["binding_ref"] =
        serde_json::to_value(alternate.value_ref()).unwrap();
    let mut bindings = program.bindings().to_vec();
    bindings.push(alternate.clone());
    bindings.sort_by(|left, right| left.value_ref().cmp(right.value_ref()));
    document["bindings"] = serde_json::to_value(&bindings).unwrap();
    let canonical =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&document.to_string()).unwrap();
    let mixed = load(canonical.as_bytes(), &ConstructionOnly::<true>).unwrap();
    assert_ne!(mixed.content_ref(), program.content_ref());
    assert_eq!(mixed.initial_value_ref(), program.initial_value_ref());
    let Execution::Read { binding_ref, .. } = mixed.declarations()[2].execution() else {
        panic!("expected designated Read")
    };
    let admitted_binding = mixed
        .bindings()
        .iter()
        .find(|binding| binding.value_ref() == binding_ref)
        .unwrap()
        .decode::<EvmBalanceBinding>()
        .unwrap();
    assert_eq!(admitted_binding.route(), &other_route);
    // Supporting qualification rejects the same route mismatch using the actual context facts.
    let support_intent = CheckEvmBalanceChain::<Unsigned256>::prepare(&context).unwrap();
    let support_abi =
        mfm_program::read_implementation_ref::<EvmChainIdentityRead, EvmBalanceObservation>()
            .unwrap();
    assert!(
        <EvmBalanceObservation as ReadImplementation<EvmChainIdentityRead>>::encode_intent(
            &support_abi,
            alternate.value_ref(),
            &admitted_binding,
            &support_intent,
        )
        .is_err()
    );
    let ledger = context.active_source().unwrap().target().ledger().clone();
    // The caller's preparation retains its route even when a cold document selects another
    // valid public binding. Native translation must reject it before provider invocation.
    let prepared = PreparedBalance::new(
        context,
        ObservationPoint::new(
            ledger,
            Object::from_value(&EvmBlockPoint::new(
                EvmU256::from_u64(10),
                EvmHash::from_bytes([3; 32]),
            ))
            .unwrap(),
        ),
        scale,
    )
    .unwrap();
    let semantic = ObserveBalance::<Unsigned256>::prepare(&prepared).unwrap();
    let abi = mfm_program::read_implementation_ref::<BalanceRead, EvmNativeBalance>().unwrap();
    let error =
        EvmNativeBalance::encode_intent(&abi, alternate.value_ref(), &admitted_binding, &semantic)
            .unwrap_err();
    let mfm_capabilities::CallbackFailure::Execute(error) = error else {
        panic!("binding phase")
    };
    assert_eq!(error.operation(), "qualify_balance_binding");
    assert_eq!(error.details().as_value(), &serde_json::json!("route"));
    let position = mfm_ids::ExecutionPosition {
        state: mfm_ids::StatePosition::new(2).unwrap(),
        visit: mfm_ids::VisitId::new(0),
    };
    let mfm_program::executable::ExecutableMode::Read { adapter, .. } =
        mixed.executable(position.state).unwrap().mode()
    else {
        panic!("expected designated Read")
    };
    let intent = Object::from_value(&semantic).unwrap();
    let Err(mfm_capabilities::CallbackFailure::Execute(cause)) = adapter(position, &intent).await
    else {
        panic!("route mismatch must fail before provider IO")
    };
    assert_eq!(cause.operation(), "qualify_balance_binding");
    assert_eq!(cause.details().as_value(), &serde_json::json!("route"));
}
