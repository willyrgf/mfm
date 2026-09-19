//! Deterministic native translation shared by invocation and evidence admission.

use super::{decode, encode, task_failure, CallbackFailure};
use mfm_capabilities::{
    EffectCapabilityContract, EffectImplementation, ReadCapabilityContract, ReadImplementation,
};
use mfm_ids::{ContentRef, EffectId};
use mfm_values::{InvocationDiagnostic, Object};
use std::sync::Arc;

pub(super) async fn read_intent<C, I>(
    implementation: ContentRef,
    binding_ref: ContentRef,
    binding: Arc<I::Binding>,
    intent: Object,
) -> Result<(C::Intent, Object), CallbackFailure>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<C>,
{
    let intent = decode::<C::Intent>(intent).await?;
    let (intent, native) = execute(move || {
        let native = I::encode_intent(&implementation, &binding_ref, &binding, &intent)?;
        Ok((intent, native))
    })
    .await?;
    Ok((intent, encode(native).await?))
}

pub(super) async fn effect_command<C, I>(
    implementation: ContentRef,
    binding_ref: ContentRef,
    binding: Arc<I::Binding>,
    command: Object,
) -> Result<(C::Command, ContentRef, I::NativeCommand), CallbackFailure>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C>,
{
    let command_ref = command.value_ref().clone();
    let command = decode::<C::Command>(command).await?;
    execute(move || {
        let (native_ref, native) = I::decode_command(
            &implementation,
            &binding_ref,
            &binding,
            &command_ref,
            &command,
        )?;
        let descriptor = I::NativeCommand::schema_descriptor()
            .map_err(|cause| cause.into_diagnostic("native_command_schema"))?;
        let schema = descriptor.identity().schema_id().map_err(|cause| {
            InvocationDiagnostic::from_fields(
                "identity_error",
                "native_command_schema",
                &cause,
                None,
            )
        })?;
        if native_ref.schema_id() != &schema {
            return Err(InvocationDiagnostic::from_fields(
                "native_command_contract",
                "decode_command",
                "schema_mismatch",
                None,
            )
            .into());
        }
        Ok((command, native_ref, native))
    })
    .await
}

pub(super) async fn read_evidence<C, I>(
    implementation: ContentRef,
    binding_ref: ContentRef,
    binding: Arc<I::Binding>,
    intent: Object,
    original: Object,
) -> Result<C::Evidence, CallbackFailure>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<C>,
{
    let intent_ref = intent.value_ref().clone();
    let (intent, native) = read_intent::<C, I>(
        implementation.clone(),
        binding_ref.clone(),
        Arc::clone(&binding),
        intent,
    )
    .await?;
    let native_ref = native.value_ref().clone();
    let native = decode::<I::NativeIntent>(native).await?;
    let evidence = decode::<I::NativeEvidence>(original.clone()).await?;
    execute(move || {
        let projected = I::project_evidence(
            &implementation,
            &binding_ref,
            &binding,
            &intent_ref,
            &intent,
            &native_ref,
            &native,
            &evidence,
            &original,
        )?;
        C::bind_evidence(&intent_ref, &intent, original.value_ref(), &projected)?;
        Ok(projected)
    })
    .await
}

pub(super) async fn effect_evidence<C, I>(
    implementation: ContentRef,
    binding_ref: ContentRef,
    binding: Arc<I::Binding>,
    effect_id: EffectId,
    command: Object,
    original: Object,
) -> Result<C::Evidence, CallbackFailure>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C>,
{
    let command_ref = command.value_ref().clone();
    let (command, _, native) = effect_command::<C, I>(
        implementation.clone(),
        binding_ref.clone(),
        Arc::clone(&binding),
        command,
    )
    .await?;
    let evidence = decode::<I::NativeEvidence>(original.clone()).await?;
    execute(move || {
        let projected = I::project_evidence(
            &implementation,
            &binding_ref,
            &binding,
            &effect_id,
            &command_ref,
            &command,
            &native,
            &evidence,
            &original,
        )?;
        C::bind_evidence(
            &effect_id,
            &command_ref,
            &command,
            original.value_ref(),
            &projected,
        )?;
        Ok(projected)
    })
    .await
}

use mfm_values::MfmValue;

async fn execute<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, CallbackFailure> + Send + 'static,
) -> Result<T, CallbackFailure> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|cause| CallbackFailure::Execute(task_failure("execute", cause)))?
}
