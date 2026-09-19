//! Typed adapter invocation, independent of Runtime transition authority.
use super::CallbackFailure;
use mfm_capabilities::{
    EffectAdapter, EffectAdapterOutcome, EffectCapabilityContract, EffectImplementation,
    ReadAdapter, ReadCapabilityContract, ReadImplementation,
};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition};
use mfm_values::Object;
use std::{future::Future, pin::Pin, sync::Arc};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// Bound Read invocation, returning either evidence or one encoded original.
pub type ErasedReadAdapterCallback = dyn for<'a> Fn(
        ExecutionPosition,
        &'a Object,
    ) -> BoxFuture<'a, Result<Result<Object, Object>, CallbackFailure>>
    + Send
    + Sync;
/// Bound Effect invocation retaining Pending without fabricated evidence.
pub type ErasedEffectAdapterCallback = dyn for<'a> Fn(
        ExecutionPosition,
        &'a EffectId,
        &'a Object,
    ) -> BoxFuture<
        'a,
        Result<Result<EffectAdapterOutcome<Object>, Object>, CallbackFailure>,
    > + Send
    + Sync;
/// Binds one selected native Read adapter without invoking it.
pub fn read_adapter<C, I, A>(
    failure_contract: ContentRef,
    implementation: ContentRef,
    binding_ref: ContentRef,
    binding: Arc<I::Binding>,
    adapter: A,
) -> Arc<ErasedReadAdapterCallback>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<C>,
    A: ReadAdapter<I::NativeIntent, I::NativeEvidence, I::OperationalError>,
{
    let adapter = Arc::new(adapter);
    Arc::new(move |position, object| {
        let adapter = Arc::clone(&adapter);
        let object = object.clone();
        let failure_contract = failure_contract.clone();
        let implementation = implementation.clone();
        let binding_ref = binding_ref.clone();
        let binding = Arc::clone(&binding);
        Box::pin(async move {
            let semantic_ref = object.value_ref().clone();
            let (_, native) =
                super::native::read_intent::<C, I>(implementation, binding_ref, binding, object)
                    .await?;
            let native_ref = native.value_ref().clone();
            let intent = super::decode::<I::NativeIntent>(native).await?;
            match super::invoke_adapter(|| adapter.invoke(&semantic_ref, &native_ref, &intent))
                .await?
            {
                Ok(evidence) => super::encode(evidence).await.map(Ok),
                Err(original) => super::encode_failure(original, position, &failure_contract)
                    .await
                    .map(Err),
            }
        })
    })
}

/// Binds one selected native Effect adapter without invoking it.
pub fn effect_adapter<C, I, A>(
    failure_contract: ContentRef,
    implementation: ContentRef,
    binding_ref: ContentRef,
    binding: Arc<I::Binding>,
    adapter: A,
) -> Arc<ErasedEffectAdapterCallback>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C>,
    A: EffectAdapter<I::NativeCommand, I::NativeEvidence, I::OperationalError>,
{
    let adapter = Arc::new(adapter);
    Arc::new(move |position, effect_id, object| {
        let adapter = Arc::clone(&adapter);
        let object = object.clone();
        let failure_contract = failure_contract.clone();
        let implementation = implementation.clone();
        let binding_ref = binding_ref.clone();
        let binding = Arc::clone(&binding);
        Box::pin(async move {
            let semantic_ref = object.value_ref().clone();
            let (_, native_ref, command) =
                super::native::effect_command::<C, I>(implementation, binding_ref, binding, object)
                    .await?;
            match super::invoke_adapter(|| {
                adapter.invoke(effect_id, &semantic_ref, &native_ref, &command)
            })
            .await?
            {
                Ok(EffectAdapterOutcome::Pending) => Ok(Ok(EffectAdapterOutcome::Pending)),
                Ok(EffectAdapterOutcome::Settled(evidence)) => super::encode(evidence)
                    .await
                    .map(|evidence| Ok(EffectAdapterOutcome::Settled(evidence))),
                Err(original) => super::encode_failure(original, position, &failure_contract)
                    .await
                    .map(Err),
            }
        })
    })
}
