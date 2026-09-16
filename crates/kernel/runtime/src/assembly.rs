use std::any::TypeId;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use mfm_capabilities::{AdapterError, EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, SemanticTypeId};
use mfm_program::{
    capability_contract_ref, effect_capability_contract_ref, nominal_contract_ref,
    state_implementation_ref, EffectState, Execution, Never, Program, PureState, ReadState,
    StateDeclaration,
};
use mfm_values::{canonicalize_mfm_value, MfmValue, Object, SchemaDescriptor};

use crate::engine::{self, DriverContext, DriverDisposition};
use crate::{EffectAdapterOutcome, Result, RuntimeError};

pub(crate) mod recovery;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub(crate) type ErasedReadAdapterCallback = dyn for<'a> Fn(
        ExecutionPosition,
        &'a ContentRef,
        &'a Object,
    ) -> BoxFuture<'a, Result<std::result::Result<Object, Object>>>
    + Send
    + Sync;
pub(crate) type ErasedEffectAdapterCallback = dyn for<'a> Fn(
        ExecutionPosition,
        &'a ContentRef,
        &'a EffectId,
        &'a Object,
    )
        -> BoxFuture<'a, Result<std::result::Result<EffectAdapterOutcome<Object>, Object>>>
    + Send
    + Sync;
pub(crate) struct ValueContract {
    owner_type: TypeId,
    semantic_id: SemanticTypeId,
    pub(crate) contract_ref: ContentRef,
    pub(crate) descriptor: SchemaDescriptor,
}
impl ValueContract {
    pub(crate) fn admit(&self, object: &Object) -> Result<()> {
        object.admit(&self.descriptor).map_err(RuntimeError::from)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct StateSignature {
    state_type: TypeId,
    abi: StateAbiKey,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StateAbiKey {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
}

impl From<&StateDeclaration> for StateAbiKey {
    fn from(declaration: &StateDeclaration) -> Self {
        Self {
            state_implementation_ref: declaration.state_implementation_ref().clone(),
            input_contract_ref: declaration.input_contract_ref().clone(),
            output_contract_ref: declaration.output_contract_ref().clone(),
            failure_contract_ref: declaration.failure_contract_ref().clone(),
        }
    }
}

pub(crate) type StateStart =
    for<'a> fn(DriverContext<'a>) -> BoxFuture<'a, Result<DriverDisposition>>;
pub(crate) type ReadStart = for<'a> fn(
    DriverContext<'a>,
    Arc<ErasedReadAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>;
pub(crate) type EffectPendingStart = for<'a> fn(
    DriverContext<'a>,
    Arc<ErasedEffectAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>;
struct RegisteredState {
    signature: StateSignature,
    mode: RegisteredMode,
}

enum RegisteredMode {
    Pure {
        start: StateStart,
    },
    Read {
        error_contract: Arc<ValueContract>,
        capability_contract_ref: ContentRef,
        start: ReadStart,
    },
    Effect {
        error_contract: Arc<ValueContract>,
        capability_contract_ref: ContentRef,
        prepare: StateStart,
        start_pending: EffectPendingStart,
    },
}

impl RegisteredState {
    fn has_same_registration(&self, other: &Self) -> bool {
        if self.signature != other.signature {
            return false;
        }
        match (&self.mode, &other.mode) {
            (RegisteredMode::Pure { .. }, RegisteredMode::Pure { .. }) => true,
            (
                RegisteredMode::Read {
                    error_contract,
                    capability_contract_ref,
                    ..
                },
                RegisteredMode::Read {
                    error_contract: other_error_contract,
                    capability_contract_ref: other,
                    ..
                },
            ) => {
                capability_contract_ref == other
                    && Arc::ptr_eq(error_contract, other_error_contract)
            }
            (
                RegisteredMode::Effect {
                    error_contract,
                    capability_contract_ref,
                    ..
                },
                RegisteredMode::Effect {
                    error_contract: other_error_contract,
                    capability_contract_ref: other,
                    ..
                },
            ) => {
                capability_contract_ref == other
                    && Arc::ptr_eq(error_contract, other_error_contract)
            }
            _ => false,
        }
    }
}

fn start_pure<'a, S: PureState>(
    context: DriverContext<'a>,
) -> BoxFuture<'a, Result<DriverDisposition>> {
    Box::pin(engine::start_pure::<S>(context))
}

fn start_read<'a, S, C>(
    context: DriverContext<'a>,
    adapter: Arc<ErasedReadAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>
where
    S: ReadState<C>,
    C: ReadCapabilityContract,
    C::OperationalError: mfm_program::ClassifyError,
{
    Box::pin(engine::start_read::<S, C>(context, adapter))
}

fn prepare_effect<'a, S, C>(context: DriverContext<'a>) -> BoxFuture<'a, Result<DriverDisposition>>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
{
    Box::pin(engine::start_effect::<S, C>(context))
}

fn start_pending_effect<'a, S, C>(
    context: DriverContext<'a>,
    adapter: Arc<ErasedEffectAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
    C::OperationalError: mfm_program::ClassifyError,
{
    Box::pin(engine::start_pending_effect::<S, C>(context, adapter))
}

struct CatchAdapterPanic<'a, T, E> {
    inner: BoxFuture<'a, std::result::Result<T, AdapterError<E>>>,
}

impl<T, E> Future for CatchAdapterPanic<'_, T, E> {
    type Output = std::result::Result<T, AdapterError<E>>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner.as_mut().poll(context)
        }))
        .unwrap_or_else(|_| {
            Poll::Ready(Err(AdapterError::Invariant(
                mfm_values::InvocationDiagnostic::from_fields(
                    "task_failure",
                    "poll",
                    "panicked",
                    None,
                ),
            )))
        })
    }
}

fn erase_read_adapter<C, F>(callback: F) -> Arc<ErasedReadAdapterCallback>
where
    C: ReadCapabilityContract,
    F: for<'a> Fn(
            &'a ContentRef,
            &'a C::Intent,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = std::result::Result<
                            C::Evidence,
                            AdapterError<C::OperationalError>,
                        >,
                    > + Send
                    + 'a,
            >,
        > + Send
        + Sync
        + 'static,
{
    let callback = Arc::new(callback);
    Arc::new(move |position, failure_contract, object| {
        let callback = Arc::clone(&callback);
        let object = object.clone();
        Box::pin(async move {
            let input = object.clone();
            let intent = engine::run_blocking(crate::Operation::ReadAdapter, move || {
                input
                    .decode::<C::Intent>()
                    .map_err(|cause| RuntimeError::native(crate::Operation::ReadAdapter, cause))
            })
            .await?;
            let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback(object.value_ref(), &intent)
            }))
            .map_err(|_| {
                RuntimeError::native(
                    crate::Operation::ReadAdapter,
                    mfm_values::InvocationDiagnostic::from_fields(
                        "task_failure",
                        "erase_read_adapter",
                        "panicked",
                        None,
                    ),
                )
            })?;
            match (CatchAdapterPanic { inner: future }).await {
                Ok(evidence) => engine::encode(evidence, crate::Operation::ReadAdapter)
                    .await
                    .map(Ok),
                Err(AdapterError::Operational(error)) => engine::encode_failure(
                    error,
                    crate::Operation::ReadAdapter,
                    position,
                    failure_contract,
                )
                .await
                .map(Err),
                Err(AdapterError::Invariant(cause)) => {
                    Err(RuntimeError::native(crate::Operation::ReadAdapter, cause))
                }
            }
        })
    })
}

fn erase_effect_adapter<C, F>(callback: F) -> Arc<ErasedEffectAdapterCallback>
where
    C: EffectCapabilityContract,
    F: for<'a> Fn(
            &'a EffectId,
            &'a ContentRef,
            &'a C::Command,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = std::result::Result<
                            EffectAdapterOutcome<C::Evidence>,
                            AdapterError<C::OperationalError>,
                        >,
                    > + Send
                    + 'a,
            >,
        > + Send
        + Sync
        + 'static,
{
    let callback = Arc::new(callback);
    Arc::new(move |position, failure_contract, effect_id, object| {
        let callback = Arc::clone(&callback);
        let object = object.clone();
        Box::pin(async move {
            let input = object.clone();
            let command = engine::run_blocking(crate::Operation::EffectAdapter, move || {
                input
                    .decode::<C::Command>()
                    .map_err(|cause| RuntimeError::native(crate::Operation::EffectAdapter, cause))
            })
            .await?;
            let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback(effect_id, object.value_ref(), &command)
            }))
            .map_err(|_| {
                RuntimeError::native(
                    crate::Operation::EffectAdapter,
                    mfm_values::InvocationDiagnostic::from_fields(
                        "task_failure",
                        "erase_effect_adapter",
                        "panicked",
                        None,
                    ),
                )
            })?;
            match (CatchAdapterPanic { inner: future }).await {
                Ok(EffectAdapterOutcome::Pending) => Ok(Ok(EffectAdapterOutcome::Pending)),
                Ok(EffectAdapterOutcome::Settled(evidence)) => {
                    engine::encode(evidence, crate::Operation::EffectAdapter)
                        .await
                        .map(|evidence| Ok(EffectAdapterOutcome::Settled(evidence)))
                }
                Err(AdapterError::Operational(error)) => engine::encode_failure(
                    error,
                    crate::Operation::EffectAdapter,
                    position,
                    failure_contract,
                )
                .await
                .map(Err),
                Err(AdapterError::Invariant(cause)) => {
                    Err(RuntimeError::native(crate::Operation::EffectAdapter, cause))
                }
            }
        })
    })
}

fn adapter_binding_ref<B: MfmValue>(binding: &B) -> Result<ContentRef> {
    canonicalize_mfm_value(binding)
        .map(|(_, value_ref)| value_ref)
        .map_err(|source| {
            RuntimeError::at(
                crate::Operation::Admission,
                crate::Stage::Execute,
                source.into_diagnostic("adapter_binding_ref"),
            )
        })
}

enum CapabilityRegistration {
    Read {
        capability_type: TypeId,
        error_contract: Arc<ValueContract>,
        intent_contract: Arc<ValueContract>,
        evidence_contract: Arc<ValueContract>,
        bindings: BTreeMap<ContentRef, Arc<ErasedReadAdapterCallback>>,
    },
    Effect {
        capability_type: TypeId,
        error_contract: Arc<ValueContract>,
        command_contract: Arc<ValueContract>,
        evidence_contract: Arc<ValueContract>,
        bindings: BTreeMap<ContentRef, Arc<ErasedEffectAdapterCallback>>,
    },
}

/// Mutable builder for one immutable Runtime assembly.
pub struct RuntimeAssemblyBuilder {
    recovery: recovery::Registrations,
    values: BTreeMap<ContentRef, Arc<ValueContract>>,
    states: BTreeMap<StateAbiKey, RegisteredState>,
    capabilities: BTreeMap<ContentRef, CapabilityRegistration>,
}

impl RuntimeAssemblyBuilder {
    /// Constructs a builder with the framework Never codec and common Stop handler installed.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::IncompatibleAssembly`] if the framework codec contract is invalid.
    pub fn new() -> Result<Self> {
        let mut builder = Self {
            recovery: recovery::Registrations::default(),
            values: BTreeMap::new(),
            states: BTreeMap::new(),
            capabilities: BTreeMap::new(),
        };
        builder.ensure_value::<Never>()?;
        builder.register_handler::<mfm_program::Stop>()?;
        Ok(builder)
    }

    /// Registers one exact value codec, idempotently.
    pub fn register_value<T: MfmValue>(&mut self) -> Result<()> {
        self.ensure_value::<T>().map(|_| ())
    }

    /// Registers one Pure State and its complete value ABI.
    pub fn register_pure<S: PureState>(&mut self) -> Result<()> {
        let input = self.ensure_value::<S::Input>()?;
        let output = self.ensure_value::<S::Output>()?;
        let failure = self.ensure_value::<S::Failure>()?;
        self.register_map::<mfm_program::Identity<S::Failure>>()?;
        self.register_state(RegisteredState {
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Pure {
                start: start_pure::<S>,
            },
        })
    }

    /// Registers one Read State, capability, and complete value ABI.
    pub fn register_read<S, C>(&mut self) -> Result<()>
    where
        S: ReadState<C>,
        C: ReadCapabilityContract,
        C::OperationalError: mfm_program::ClassifyError,
    {
        let input = self.ensure_value::<S::Input>()?;
        let output = self.ensure_value::<S::Output>()?;
        let failure = self.ensure_value::<S::Failure>()?;
        self.register_map::<mfm_program::Identity<S::Failure>>()?;
        let capability_contract_ref = self.ensure_read_capability::<C>()?;
        let error_contract = self.ensure_value::<C::OperationalError>()?;
        self.register_state(RegisteredState {
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Read {
                error_contract,
                capability_contract_ref,
                start: start_read::<S, C>,
            },
        })
    }

    /// Registers one Effect State, capability, and complete value ABI.
    pub fn register_effect<S, C>(&mut self) -> Result<()>
    where
        S: EffectState<C>,
        C: EffectCapabilityContract,
        C::OperationalError: mfm_program::ClassifyError,
    {
        let input = self.ensure_value::<S::Input>()?;
        let output = self.ensure_value::<S::Output>()?;
        let failure = self.ensure_value::<S::Failure>()?;
        self.register_map::<mfm_program::Identity<S::Failure>>()?;
        let capability_contract_ref = self.ensure_effect_capability::<C>()?;
        let error_contract = self.ensure_value::<C::OperationalError>()?;
        self.register_state(RegisteredState {
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Effect {
                error_contract,
                capability_contract_ref,
                prepare: prepare_effect::<S, C>,
                start_pending: start_pending_effect::<S, C>,
            },
        })
    }

    /// Registers one typed Read adapter under its internally derived binding ref.
    ///
    /// Runtime supplies the callback with the exact qualified intent value ref, followed by the
    /// typed intent. The reference is the intent instance identity, not its codec contract.
    pub fn register_adapter<C, B, F>(&mut self, binding: B, callback: F) -> Result<()>
    where
        C: ReadCapabilityContract,
        B: MfmValue,
        F: for<'a> Fn(
                &'a ContentRef,
                &'a C::Intent,
            ) -> Pin<
                Box<
                    dyn Future<
                            Output = std::result::Result<
                                C::Evidence,
                                AdapterError<C::OperationalError>,
                            >,
                        > + Send
                        + 'a,
                >,
            > + Send
            + Sync
            + 'static,
    {
        let capability_ref = self.ensure_read_capability::<C>()?;
        let binding_ref = adapter_binding_ref(&binding)?;
        let Some(CapabilityRegistration::Read {
            capability_type,
            bindings,
            ..
        }) = self.capabilities.get_mut(&capability_ref)
        else {
            return Err(RuntimeError::IncompatibleAssembly);
        };
        if *capability_type != TypeId::of::<C>() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        if bindings.contains_key(&binding_ref) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        bindings.insert(binding_ref, erase_read_adapter::<C, F>(callback));
        Ok(())
    }

    /// Registers one typed Effect adapter under its internally derived binding ref.
    ///
    /// Runtime supplies the exact Effect ID, qualified command value ref, and typed command. The
    /// command reference is the instance identity used to derive the Effect ID, not its codec
    /// contract.
    pub fn register_effect_adapter<C, B, F>(&mut self, binding: B, callback: F) -> Result<()>
    where
        C: EffectCapabilityContract,
        B: MfmValue,
        F: for<'a> Fn(
                &'a EffectId,
                &'a ContentRef,
                &'a C::Command,
            ) -> Pin<
                Box<
                    dyn Future<
                            Output = std::result::Result<
                                EffectAdapterOutcome<C::Evidence>,
                                AdapterError<C::OperationalError>,
                            >,
                        > + Send
                        + 'a,
                >,
            > + Send
            + Sync
            + 'static,
    {
        let capability_ref = self.ensure_effect_capability::<C>()?;
        let binding_ref = adapter_binding_ref(&binding)?;
        let Some(CapabilityRegistration::Effect {
            capability_type,
            bindings,
            ..
        }) = self.capabilities.get_mut(&capability_ref)
        else {
            return Err(RuntimeError::IncompatibleAssembly);
        };
        if *capability_type != TypeId::of::<C>() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        if bindings.contains_key(&binding_ref) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        bindings.insert(binding_ref, erase_effect_adapter::<C, F>(callback));
        Ok(())
    }

    /// Finalizes the immutable assembly.
    pub fn finish(self) -> RuntimeAssembly {
        RuntimeAssembly {
            inner: Arc::new(AssemblyInner {
                recovery: self.recovery,
                values: self.values,
                states: self.states,
                capabilities: self.capabilities,
            }),
        }
    }

    fn register_state(&mut self, state: RegisteredState) -> Result<()> {
        let key = state.signature.abi.clone();
        if let Some(previous) = self.states.get(&key) {
            return previous
                .has_same_registration(&state)
                .then_some(())
                .ok_or(RuntimeError::IncompatibleAssembly);
        }
        self.states.insert(key, state);
        Ok(())
    }

    fn ensure_value<T: MfmValue>(&mut self) -> Result<Arc<ValueContract>> {
        let descriptor = T::schema_descriptor().map_err(|source| {
            RuntimeError::at(
                crate::Operation::Admission,
                crate::Stage::Execute,
                source.into_diagnostic("ensure_value"),
            )
        })?;
        let semantic_id = T::semantic_id().map_err(|source| {
            RuntimeError::at(
                crate::Operation::Admission,
                crate::Stage::Execute,
                source.into_diagnostic("ensure_value"),
            )
        })?;
        if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_id) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let contract_ref = nominal_contract_ref::<T>().map_err(|source| {
            RuntimeError::at(
                crate::Operation::Admission,
                crate::Stage::Execute,
                mfm_values::InvocationDiagnostic::from_fields(
                    "runtime_invariant",
                    "ensure_value",
                    &source,
                    None,
                ),
            )
        })?;
        if let Some(previous) = self.values.get(&contract_ref) {
            return (previous.owner_type == TypeId::of::<T>()
                && previous.semantic_id == semantic_id
                && previous.descriptor == descriptor)
                .then(|| Arc::clone(previous))
                .ok_or(RuntimeError::IncompatibleAssembly);
        }
        let codec = Arc::new(ValueContract {
            owner_type: TypeId::of::<T>(),
            semantic_id,
            contract_ref: contract_ref.clone(),
            descriptor,
        });
        self.values.insert(contract_ref, Arc::clone(&codec));
        Ok(codec)
    }

    fn ensure_read_capability<C: ReadCapabilityContract>(&mut self) -> Result<ContentRef> {
        let intent_contract = self.ensure_value::<C::Intent>()?;
        let evidence_contract = self.ensure_value::<C::Evidence>()?;
        let error_contract = self.ensure_value::<C::OperationalError>()?;
        let capability_ref = capability_contract_ref::<C>().map_err(|source| {
            RuntimeError::at(
                crate::Operation::Admission,
                crate::Stage::Execute,
                mfm_values::InvocationDiagnostic::from_fields(
                    "runtime_invariant",
                    "ensure_read_capability",
                    &source,
                    None,
                ),
            )
        })?;
        if let Some(registration) = self.capabilities.get(&capability_ref) {
            let CapabilityRegistration::Read {
                capability_type,
                intent_contract: registered_intent,
                evidence_contract: registered_evidence,
                error_contract: registered_error,
                ..
            } = registration
            else {
                return Err(RuntimeError::IncompatibleAssembly);
            };
            if *capability_type != TypeId::of::<C>()
                || !Arc::ptr_eq(registered_intent, &intent_contract)
                || !Arc::ptr_eq(registered_evidence, &evidence_contract)
                || !Arc::ptr_eq(registered_error, &error_contract)
            {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            return Ok(capability_ref);
        }
        self.capabilities.insert(
            capability_ref.clone(),
            CapabilityRegistration::Read {
                capability_type: TypeId::of::<C>(),
                error_contract,
                intent_contract,
                evidence_contract,
                bindings: BTreeMap::new(),
            },
        );
        Ok(capability_ref)
    }

    fn ensure_effect_capability<C: EffectCapabilityContract>(&mut self) -> Result<ContentRef> {
        let command_contract = self.ensure_value::<C::Command>()?;
        let evidence_contract = self.ensure_value::<C::Evidence>()?;
        let error_contract = self.ensure_value::<C::OperationalError>()?;
        let capability_ref = effect_capability_contract_ref::<C>().map_err(|source| {
            RuntimeError::at(
                crate::Operation::Admission,
                crate::Stage::Execute,
                mfm_values::InvocationDiagnostic::from_fields(
                    "runtime_invariant",
                    "ensure_effect_capability",
                    &source,
                    None,
                ),
            )
        })?;
        if let Some(registration) = self.capabilities.get(&capability_ref) {
            let CapabilityRegistration::Effect {
                capability_type,
                command_contract: registered_command,
                evidence_contract: registered_evidence,
                error_contract: registered_error,
                ..
            } = registration
            else {
                return Err(RuntimeError::IncompatibleAssembly);
            };
            if *capability_type != TypeId::of::<C>()
                || !Arc::ptr_eq(registered_command, &command_contract)
                || !Arc::ptr_eq(registered_evidence, &evidence_contract)
                || !Arc::ptr_eq(registered_error, &error_contract)
            {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            return Ok(capability_ref);
        }
        self.capabilities.insert(
            capability_ref.clone(),
            CapabilityRegistration::Effect {
                capability_type: TypeId::of::<C>(),
                error_contract,
                command_contract,
                evidence_contract,
                bindings: BTreeMap::new(),
            },
        );
        Ok(capability_ref)
    }
}

fn state_signature<S: mfm_program::State>(
    input: &ValueContract,
    output: &ValueContract,
    failure: &ValueContract,
) -> Result<StateSignature> {
    Ok(StateSignature {
        state_type: TypeId::of::<S>(),
        abi: StateAbiKey {
            state_implementation_ref: state_implementation_ref::<S>().map_err(|source| {
                RuntimeError::at(
                    crate::Operation::Admission,
                    crate::Stage::Execute,
                    mfm_values::InvocationDiagnostic::from_fields(
                        "runtime_invariant",
                        "ensure_effect_capability",
                        &source,
                        None,
                    ),
                )
            })?,
            input_contract_ref: input.contract_ref.clone(),
            output_contract_ref: output.contract_ref.clone(),
            failure_contract_ref: failure.contract_ref.clone(),
        },
    })
}

/// Finalized immutable typed assembly.
pub struct RuntimeAssembly {
    pub(crate) inner: Arc<AssemblyInner>,
}

pub(crate) struct AssemblyInner {
    recovery: recovery::Registrations,
    pub(crate) values: BTreeMap<ContentRef, Arc<ValueContract>>,
    states: BTreeMap<StateAbiKey, RegisteredState>,
    capabilities: BTreeMap<ContentRef, CapabilityRegistration>,
}

impl RuntimeAssembly {
    pub(crate) fn associate(&self, program: Program) -> Result<ExecutableProgram> {
        let _admitted_context_contract = self
            .contract(program.admitted_context_contract_ref())
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        for contract_ref in [
            program.root_success_contract_ref(),
            program.root_failure_contract_ref(),
        ] {
            self.inner
                .values
                .get(contract_ref)
                .ok_or(RuntimeError::IncompatibleAssembly)?;
        }
        let mut declarations = Vec::with_capacity(program.declarations().len());
        for state in program.declarations() {
            let key = StateAbiKey::from(state);
            let registered = self
                .inner
                .states
                .get(&key)
                .ok_or(RuntimeError::IncompatibleAssembly)?;
            let _output_contract = self
                .contract(state.output_contract_ref())
                .ok_or(RuntimeError::IncompatibleAssembly)?;
            let _failure_contract = self
                .contract(state.failure_contract_ref())
                .ok_or(RuntimeError::IncompatibleAssembly)?;
            let mode = match state.execution() {
                Execution::Pure { .. } => {
                    let RegisteredMode::Pure { start } = &registered.mode else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    ExecutableMode::Pure { start: *start }
                }
                Execution::Read {
                    error_contract_ref,
                    capability_contract_ref,
                    intent_contract_ref,
                    evidence_contract_ref,
                    binding_ref,
                    ..
                } => {
                    let RegisteredMode::Read {
                        error_contract,
                        capability_contract_ref: registered_capability_contract_ref,
                        start,
                    } = &registered.mode
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    let Some(CapabilityRegistration::Read {
                        intent_contract,
                        evidence_contract,
                        bindings,
                        ..
                    }) = self
                        .inner
                        .capabilities
                        .get(registered_capability_contract_ref)
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    if error_contract_ref != &error_contract.contract_ref
                        || capability_contract_ref != registered_capability_contract_ref
                        || intent_contract_ref != &intent_contract.contract_ref
                        || evidence_contract_ref != &evidence_contract.contract_ref
                    {
                        return Err(RuntimeError::IncompatibleAssembly);
                    }
                    let adapter = bindings
                        .get(binding_ref)
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    ExecutableMode::Read {
                        start: *start,

                        adapter: Arc::clone(adapter),
                    }
                }
                Execution::Effect {
                    error_contract_ref,
                    capability_contract_ref,
                    command_contract_ref,
                    evidence_contract_ref,
                    binding_ref,
                    ..
                } => {
                    let RegisteredMode::Effect {
                        error_contract,
                        capability_contract_ref: registered_capability_contract_ref,
                        prepare,
                        start_pending,
                    } = &registered.mode
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    let Some(CapabilityRegistration::Effect {
                        command_contract,
                        evidence_contract,
                        bindings,
                        ..
                    }) = self
                        .inner
                        .capabilities
                        .get(registered_capability_contract_ref)
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    if error_contract_ref != &error_contract.contract_ref
                        || capability_contract_ref != registered_capability_contract_ref
                        || command_contract_ref != &command_contract.contract_ref
                        || evidence_contract_ref != &evidence_contract.contract_ref
                    {
                        return Err(RuntimeError::IncompatibleAssembly);
                    }
                    let adapter = bindings
                        .get(binding_ref)
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    ExecutableMode::Effect {
                        prepare: *prepare,
                        start_pending: *start_pending,

                        adapter: Arc::clone(adapter),
                    }
                }
            };
            declarations.push(ExecutableState {
                is_checkpoint: false,

                mode,
                recovery: self.inner.associate_recovery(state.handler())?,
                root_map: self.inner.associate_root_map(
                    state.failure_contract_ref(),
                    program.root_failure_contract_ref(),
                    state.root_maps(),
                )?,
            });
        }
        for state in program.declarations() {
            for target in state.recovery_targets() {
                declarations[target.position().index()].is_checkpoint = true;
            }
        }
        let _root_failure_contract = self
            .inner
            .values
            .get(program.root_failure_contract_ref())
            .cloned()
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        Ok(ExecutableProgram {
            program,
            declarations,

            _assembly: Arc::clone(&self.inner),
        })
    }

    pub(crate) fn contract(&self, contract_ref: &ContentRef) -> Option<Arc<ValueContract>> {
        self.inner.values.get(contract_ref).cloned()
    }

    pub(crate) fn handle(&self) -> Arc<AssemblyInner> {
        Arc::clone(&self.inner)
    }
}

pub(crate) struct ExecutableProgram {
    pub(crate) program: Program,
    pub(crate) declarations: Vec<ExecutableState>,

    pub(crate) _assembly: Arc<AssemblyInner>,
}

pub(crate) struct ExecutableState {
    pub(crate) is_checkpoint: bool,
    pub(crate) recovery: recovery::AssociatedRecovery,
    pub(crate) root_map: recovery::AssociatedRootMap,

    pub(crate) mode: ExecutableMode,
}

pub(crate) enum ExecutableMode {
    Pure {
        start: StateStart,
    },
    Read {
        start: ReadStart,

        adapter: Arc<ErasedReadAdapterCallback>,
    },
    Effect {
        prepare: StateStart,
        start_pending: EffectPendingStart,

        adapter: Arc<ErasedEffectAdapterCallback>,
    },
}
