//! Read-only kernel access to immutable executable occurrences.

use crate::{callback, Classification, Handler, HandlerBinding, RecoveryContext, RecoveryRequest};
use mfm_values::{InvocationDiagnostic, Object};
use std::sync::Arc;

pub(crate) type Handle = fn(
    &Object,
    Classification,
    &RecoveryContext<'_>,
) -> Result<RecoveryRequest, mfm_capabilities::CallbackFailure>;

/// Mode-specific invocation functions; only Program construction installs an occurrence.
pub enum ExecutableMode {
    /// Deterministic State.
    Pure { callbacks: callback::PureCallbacks },
    /// Observational State and its already-bound adapter.
    Read {
        callbacks: callback::ReadCallbacks,
        adapter: Arc<callback::ErasedReadAdapterCallback>,
    },
    /// Effect State and its already-bound adapter.
    Effect {
        callbacks: callback::EffectCallbacks,
        adapter: Arc<callback::ErasedEffectAdapterCallback>,
    },
}

/// One mandatory executable corresponding to one checked declaration.
pub struct ExecutableState {
    pub(crate) mode: ExecutableMode,
    pub(crate) handle: Handle,
    pub(crate) params: Object,
    pub(crate) checkpoint: bool,
}
impl ExecutableState {
    /// Returns only the callbacks valid for this occurrence's execution mode.
    pub fn mode(&self) -> &ExecutableMode {
        &self.mode
    }
    /// Whether this occurrence starts a retained checkpoint.
    pub fn is_checkpoint(&self) -> bool {
        self.checkpoint
    }
    /// Invokes a deterministic handler proposal; Runtime retains authorization.
    pub fn request(
        &self,
        classification: Classification,
        context: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, mfm_capabilities::CallbackFailure> {
        (self.handle)(&self.params, classification, context)
    }
}

pub(crate) fn handle<H: Handler>(
    params: &Object,
    classification: Classification,
    context: &RecoveryContext<'_>,
) -> Result<RecoveryRequest, mfm_capabilities::CallbackFailure> {
    use mfm_capabilities::CallbackFailure;
    let params = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        params.decode::<H::Params>()
    }))
    .map_err(|_| {
        CallbackFailure::Decode(InvocationDiagnostic::from_fields(
            "task_failure",
            "handler_decode",
            "panicked",
            None,
        ))
    })?
    .map_err(CallbackFailure::Decode)?;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        H::handle(&params, classification, context)
    }))
    .map_err(|_| {
        CallbackFailure::Execute(InvocationDiagnostic::from_fields(
            "task_failure",
            "handle",
            "panicked",
            None,
        ))
    })?
    .map_err(CallbackFailure::Execute)
}

pub(crate) fn parameters(binding: &HandlerBinding) -> crate::Result<Object> {
    Object::from_canonical(
        binding.params().value_ref().clone(),
        binding.params().canonical_bytes(),
    )
    .map_err(|cause| crate::ProgramError::Diagnostic(cause.into_diagnostic("handler_parameters")))
}
