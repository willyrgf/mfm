use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    AdapterError, AdapterInvariantError, EffectCapabilityContract, ReadCapabilityContract,
};
use mfm_ids::{ContentRef, EffectId, SemanticTypeId};
use mfm_program::{
    capability_contract_ref, effect_capability_contract_ref, nominal_contract_ref,
    state_implementation_ref, EffectState, Execution, Never, Program, PureState, ReadState,
    StateDeclaration,
};
use mfm_values::{canonicalize_mfm_value, MfmValue, SchemaDescriptor};

use crate::engine::{self, DriverContext, DriverDisposition};
use crate::{EffectAdapterOutcome, Result, RuntimeError};

pub(crate) mod recovery;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub(crate) type EvidenceQualification =
    Box<dyn FnOnce() -> std::result::Result<QualifiedValue, mfm_values::ValueError> + Send>;
pub(crate) type ErasedReadAdapterCallback = dyn for<'a> Fn(
        &'a QualifiedValue,
    ) -> BoxFuture<
        'a,
        std::result::Result<EvidenceQualification, AdapterError<EvidenceQualification>>,
    > + Send
    + Sync;
pub(crate) type ErasedEffectAdapterCallback = dyn for<'a> Fn(
        &'a EffectId,
        &'a QualifiedValue,
    ) -> BoxFuture<
        'a,
        std::result::Result<
            EffectAdapterOutcome<EvidenceQualification>,
            AdapterError<EvidenceQualification>,
        >,
    > + Send
    + Sync;

pub(crate) struct QualifiedValue {
    pub(crate) contract_ref: ContentRef,
    pub(crate) value_ref: ContentRef,
    pub(crate) canonical: PlainCanonicalJsonBytes,
    pub(crate) typed: Box<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for QualifiedValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QualifiedValue")
            .field("contract_ref", &self.contract_ref)
            .field("value_ref", &self.value_ref)
            .field("canonical", &"<redacted>")
            .finish()
    }
}

type ColdQualifier = fn(&ValueCodec, &ContentRef, &[u8]) -> std::result::Result<QualifiedValue, ()>;

pub(crate) struct ValueCodec {
    codec_type_id: TypeId,
    semantic_id: SemanticTypeId,
    contract_ref: ContentRef,
    descriptor: SchemaDescriptor,
    qualify: ColdQualifier,
}

impl ValueCodec {
    pub(crate) fn qualify(
        &self,
        value_ref: &ContentRef,
        canonical: &[u8],
    ) -> std::result::Result<QualifiedValue, ()> {
        (self.qualify)(self, value_ref, canonical)
    }
}

fn qualify_typed<T: MfmValue>(
    codec: &ValueCodec,
    value_ref: &ContentRef,
    canonical: &[u8],
) -> std::result::Result<QualifiedValue, ()> {
    if value_ref.schema_id() != codec.contract_ref.schema_id()
        || value_ref.content_digest() != &raw_content_digest(canonical)
    {
        return Err(());
    }
    let canonical =
        PlainCanonicalJsonBytes::from_canonical_json_slice(canonical).map_err(|_| ())?;
    codec
        .descriptor
        .identity()
        .validate_canonical_value(canonical.as_bytes())
        .map_err(|_| ())?;
    let typed = serde_json::from_slice::<T>(canonical.as_bytes()).map_err(|_| ())?;
    Ok(QualifiedValue {
        contract_ref: codec.contract_ref.clone(),
        value_ref: value_ref.clone(),
        canonical,
        typed: Box::new(typed),
    })
}

pub(crate) fn qualify_hot<T: MfmValue>(
    value: T,
) -> std::result::Result<QualifiedValue, mfm_values::ValueError> {
    let contract_ref = nominal_contract_ref::<T>()
        .map_err(|_| mfm_values::ValueError::Descriptor("invalid nominal contract".to_owned()))?;
    let (canonical, value_ref) = canonicalize_mfm_value(&value)?;
    Ok(QualifiedValue {
        contract_ref,
        value_ref,
        canonical,
        typed: Box::new(value),
    })
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
type ReadValidator = fn(&QualifiedValue, &QualifiedValue) -> Result<()>;
type EffectPrepareValidator = fn(&QualifiedValue, &QualifiedValue) -> Result<()>;
type EffectEvidenceValidator = fn(&EffectId, &QualifiedValue, &QualifiedValue) -> Result<()>;

type AdapterContextCallback =
    fn(&QualifiedValue, &QualifiedValue, &QualifiedValue) -> Result<QualifiedValue>;

pub(crate) struct AdapterIncidentContract {
    pub(crate) error_codec: Arc<ValueCodec>,
    pub(crate) context_codec: Arc<ValueCodec>,
    pub(crate) context: AdapterContextCallback,
}

impl AdapterIncidentContract {
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.error_codec, &other.error_codec)
            && Arc::ptr_eq(&self.context_codec, &other.context_codec)
    }
}

fn read_context<S: ReadState<C>, C: ReadCapabilityContract>(
    input: &QualifiedValue,
    intent: &QualifiedValue,
    error: &QualifiedValue,
) -> Result<QualifiedValue> {
    let context = S::adapter_context(
        input
            .typed
            .downcast_ref::<S::Input>()
            .ok_or(RuntimeError::Internal)?,
        intent
            .typed
            .downcast_ref::<C::Intent>()
            .ok_or(RuntimeError::Internal)?,
        error
            .typed
            .downcast_ref::<C::OperationalError>()
            .ok_or(RuntimeError::Internal)?,
    )
    .map_err(|_| RuntimeError::Internal)?;
    qualify_hot(context).map_err(RuntimeError::from)
}

fn effect_context<S: EffectState<C>, C: EffectCapabilityContract>(
    input: &QualifiedValue,
    command: &QualifiedValue,
    error: &QualifiedValue,
) -> Result<QualifiedValue> {
    let context = S::adapter_context(
        input
            .typed
            .downcast_ref::<S::Input>()
            .ok_or(RuntimeError::Internal)?,
        command
            .typed
            .downcast_ref::<C::Command>()
            .ok_or(RuntimeError::Internal)?,
        error
            .typed
            .downcast_ref::<C::OperationalError>()
            .ok_or(RuntimeError::Internal)?,
    )
    .map_err(|_| RuntimeError::Internal)?;
    qualify_hot(context).map_err(RuntimeError::from)
}

struct RegisteredState {
    classify: recovery::ClassifyCallback,
    signature: StateSignature,
    mode: RegisteredMode,
}

enum RegisteredMode {
    Pure {
        start: StateStart,
    },
    Read {
        incident: Arc<AdapterIncidentContract>,
        capability_contract_ref: ContentRef,
        start: ReadStart,
        validate_retained: ReadValidator,
    },
    Effect {
        incident: Arc<AdapterIncidentContract>,
        capability_contract_ref: ContentRef,
        prepare: StateStart,
        start_pending: EffectPendingStart,
        validate_prepare: EffectPrepareValidator,
        validate_evidence: EffectEvidenceValidator,
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
                    incident,
                    capability_contract_ref,
                    ..
                },
                RegisteredMode::Read {
                    incident: other_incident,
                    capability_contract_ref: other,
                    ..
                },
            ) => capability_contract_ref == other && incident.matches(other_incident),
            (
                RegisteredMode::Effect {
                    incident,
                    capability_contract_ref,
                    ..
                },
                RegisteredMode::Effect {
                    incident: other_incident,
                    capability_contract_ref: other,
                    ..
                },
            ) => capability_contract_ref == other && incident.matches(other_incident),
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
{
    Box::pin(engine::start_read::<S, C>(context, adapter))
}

fn validate_read<C: ReadCapabilityContract>(
    intent: &QualifiedValue,
    evidence: &QualifiedValue,
) -> Result<()> {
    let typed_intent = intent
        .typed
        .downcast_ref::<C::Intent>()
        .ok_or(RuntimeError::InvalidHistory)?;
    let evidence = evidence
        .typed
        .downcast_ref::<C::Evidence>()
        .ok_or(RuntimeError::InvalidHistory)?;
    C::bind_evidence(&intent.value_ref, typed_intent, evidence)
        .map_err(|_| RuntimeError::InvalidHistory)
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
{
    Box::pin(engine::start_pending_effect::<S, C>(context, adapter))
}

fn validate_effect_prepare<S, C>(input: &QualifiedValue, command: &QualifiedValue) -> Result<()>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
{
    let input = input
        .typed
        .downcast_ref::<S::Input>()
        .ok_or(RuntimeError::InvalidHistory)?;
    let expected = S::prepare(input).map_err(|_| RuntimeError::InvalidHistory)?;
    let expected = qualify_hot(expected).map_err(|_| RuntimeError::InvalidHistory)?;
    ((expected.contract_ref == command.contract_ref)
        && (expected.value_ref == command.value_ref)
        && (expected.canonical == command.canonical))
        .then_some(())
        .ok_or(RuntimeError::InvalidHistory)
}

fn validate_effect_evidence<C: EffectCapabilityContract>(
    effect_id: &EffectId,
    command: &QualifiedValue,
    evidence: &QualifiedValue,
) -> Result<()> {
    let command = command
        .typed
        .downcast_ref::<C::Command>()
        .ok_or(RuntimeError::InvalidHistory)?;
    let evidence = evidence
        .typed
        .downcast_ref::<C::Evidence>()
        .ok_or(RuntimeError::InvalidHistory)?;
    C::bind_evidence(effect_id, command, evidence).map_err(|_| RuntimeError::InvalidHistory)
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
        .unwrap_or(Poll::Ready(Err(AdapterError::Invariant(
            AdapterInvariantError,
        ))))
    }
}

fn erase_adapter_error<E: MfmValue>(error: AdapterError<E>) -> AdapterError<EvidenceQualification> {
    match error {
        AdapterError::Operational(error) => {
            AdapterError::Operational(Box::new(move || qualify_hot(error)))
        }
        AdapterError::Invariant(error) => AdapterError::Invariant(error),
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
    Arc::new(move |qualified_intent| {
        let Some(intent) = qualified_intent.typed.downcast_ref::<C::Intent>() else {
            return Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) });
        };
        let future = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback(&qualified_intent.value_ref, intent)
        })) {
            Ok(future) => future,
            Err(_) => {
                return Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) })
            }
        };
        Box::pin(async move {
            let evidence = CatchAdapterPanic { inner: future }
                .await
                .map_err(erase_adapter_error)?;
            Ok(Box::new(move || qualify_hot(evidence)) as EvidenceQualification)
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
    Arc::new(move |effect_id, qualified_command| {
        let Some(command) = qualified_command.typed.downcast_ref::<C::Command>() else {
            return Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) });
        };
        let future = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback(effect_id, &qualified_command.value_ref, command)
        })) {
            Ok(future) => future,
            Err(_) => {
                return Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) })
            }
        };
        Box::pin(async move {
            match (CatchAdapterPanic { inner: future })
                .await
                .map_err(erase_adapter_error)?
            {
                EffectAdapterOutcome::Pending => Ok(EffectAdapterOutcome::Pending),
                EffectAdapterOutcome::Settled(evidence) => Ok(EffectAdapterOutcome::Settled(
                    Box::new(move || qualify_hot(evidence)) as EvidenceQualification,
                )),
            }
        })
    })
}

fn adapter_binding_ref<B: MfmValue>(binding: &B) -> Result<ContentRef> {
    canonicalize_mfm_value(binding)
        .map(|(_, value_ref)| value_ref)
        .map_err(|_| RuntimeError::IncompatibleAssembly)
}

enum CapabilityRegistration {
    Read {
        capability_type: TypeId,
        error_codec: Arc<ValueCodec>,
        intent_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
        bindings: BTreeMap<ContentRef, Arc<ErasedReadAdapterCallback>>,
    },
    Effect {
        capability_type: TypeId,
        error_codec: Arc<ValueCodec>,
        command_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
        bindings: BTreeMap<ContentRef, Arc<ErasedEffectAdapterCallback>>,
    },
}

/// Mutable builder for one immutable Runtime assembly.
pub struct RuntimeAssemblyBuilder {
    recovery: recovery::Registrations,
    values: BTreeMap<ContentRef, Arc<ValueCodec>>,
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
            classify: recovery::classify::<S::Failure, Never>,
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
        let incident = Arc::new(AdapterIncidentContract {
            error_codec: self.ensure_value::<C::OperationalError>()?,
            context_codec: self.ensure_value::<S::AdapterContext>()?,
            context: read_context::<S, C>,
        });
        self.register_state(RegisteredState {
            classify: recovery::classify::<S::Failure, C::OperationalError>,
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Read {
                incident,
                capability_contract_ref,
                start: start_read::<S, C>,
                validate_retained: validate_read::<C>,
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
        let incident = Arc::new(AdapterIncidentContract {
            error_codec: self.ensure_value::<C::OperationalError>()?,
            context_codec: self.ensure_value::<S::AdapterContext>()?,
            context: effect_context::<S, C>,
        });
        self.register_state(RegisteredState {
            classify: recovery::classify::<S::Failure, C::OperationalError>,
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Effect {
                incident,
                capability_contract_ref,
                prepare: prepare_effect::<S, C>,
                start_pending: start_pending_effect::<S, C>,
                validate_prepare: validate_effect_prepare::<S, C>,
                validate_evidence: validate_effect_evidence::<C>,
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

    fn ensure_value<T: MfmValue>(&mut self) -> Result<Arc<ValueCodec>> {
        let descriptor = T::schema_descriptor().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        let semantic_id = T::semantic_id().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_id) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let contract_ref =
            nominal_contract_ref::<T>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if let Some(previous) = self.values.get(&contract_ref) {
            return (previous.codec_type_id == TypeId::of::<T>()
                && previous.semantic_id == semantic_id
                && previous.descriptor == descriptor)
                .then(|| Arc::clone(previous))
                .ok_or(RuntimeError::IncompatibleAssembly);
        }
        let codec = Arc::new(ValueCodec {
            codec_type_id: TypeId::of::<T>(),
            semantic_id,
            contract_ref: contract_ref.clone(),
            descriptor,
            qualify: qualify_typed::<T>,
        });
        self.values.insert(contract_ref, Arc::clone(&codec));
        Ok(codec)
    }

    fn ensure_read_capability<C: ReadCapabilityContract>(&mut self) -> Result<ContentRef> {
        let intent_codec = self.ensure_value::<C::Intent>()?;
        let evidence_codec = self.ensure_value::<C::Evidence>()?;
        let error_codec = self.ensure_value::<C::OperationalError>()?;
        let capability_ref =
            capability_contract_ref::<C>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if let Some(registration) = self.capabilities.get(&capability_ref) {
            let CapabilityRegistration::Read {
                capability_type,
                intent_codec: registered_intent,
                evidence_codec: registered_evidence,
                error_codec: registered_error,
                ..
            } = registration
            else {
                return Err(RuntimeError::IncompatibleAssembly);
            };
            if *capability_type != TypeId::of::<C>()
                || !Arc::ptr_eq(registered_intent, &intent_codec)
                || !Arc::ptr_eq(registered_evidence, &evidence_codec)
                || !Arc::ptr_eq(registered_error, &error_codec)
            {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            return Ok(capability_ref);
        }
        self.capabilities.insert(
            capability_ref.clone(),
            CapabilityRegistration::Read {
                capability_type: TypeId::of::<C>(),
                error_codec,
                intent_codec,
                evidence_codec,
                bindings: BTreeMap::new(),
            },
        );
        Ok(capability_ref)
    }

    fn ensure_effect_capability<C: EffectCapabilityContract>(&mut self) -> Result<ContentRef> {
        let command_codec = self.ensure_value::<C::Command>()?;
        let evidence_codec = self.ensure_value::<C::Evidence>()?;
        let error_codec = self.ensure_value::<C::OperationalError>()?;
        let capability_ref = effect_capability_contract_ref::<C>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if let Some(registration) = self.capabilities.get(&capability_ref) {
            let CapabilityRegistration::Effect {
                capability_type,
                command_codec: registered_command,
                evidence_codec: registered_evidence,
                error_codec: registered_error,
                ..
            } = registration
            else {
                return Err(RuntimeError::IncompatibleAssembly);
            };
            if *capability_type != TypeId::of::<C>()
                || !Arc::ptr_eq(registered_command, &command_codec)
                || !Arc::ptr_eq(registered_evidence, &evidence_codec)
                || !Arc::ptr_eq(registered_error, &error_codec)
            {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            return Ok(capability_ref);
        }
        self.capabilities.insert(
            capability_ref.clone(),
            CapabilityRegistration::Effect {
                capability_type: TypeId::of::<C>(),
                error_codec,
                command_codec,
                evidence_codec,
                bindings: BTreeMap::new(),
            },
        );
        Ok(capability_ref)
    }
}

fn state_signature<S: mfm_program::State>(
    input: &ValueCodec,
    output: &ValueCodec,
    failure: &ValueCodec,
) -> Result<StateSignature> {
    Ok(StateSignature {
        state_type: TypeId::of::<S>(),
        abi: StateAbiKey {
            state_implementation_ref: state_implementation_ref::<S>()
                .map_err(|_| RuntimeError::IncompatibleAssembly)?,
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
    values: BTreeMap<ContentRef, Arc<ValueCodec>>,
    states: BTreeMap<StateAbiKey, RegisteredState>,
    capabilities: BTreeMap<ContentRef, CapabilityRegistration>,
}

impl RuntimeAssembly {
    pub(crate) fn associate(&self, program: Program) -> Result<ExecutableProgram> {
        let admitted_context_codec = self
            .codec(program.admitted_context_contract_ref())
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
            let output_codec = self
                .codec(state.output_contract_ref())
                .ok_or(RuntimeError::IncompatibleAssembly)?;
            let failure_codec = self
                .codec(state.failure_contract_ref())
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
                    context_contract_ref,
                    capability_contract_ref,
                    intent_contract_ref,
                    evidence_contract_ref,
                    binding_ref,
                    ..
                } => {
                    let RegisteredMode::Read {
                        incident,
                        capability_contract_ref: registered_capability_contract_ref,
                        start,
                        validate_retained,
                    } = &registered.mode
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    let Some(CapabilityRegistration::Read {
                        intent_codec,
                        evidence_codec,
                        bindings,
                        ..
                    }) = self
                        .inner
                        .capabilities
                        .get(registered_capability_contract_ref)
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    if error_contract_ref != &incident.error_codec.contract_ref
                        || context_contract_ref != &incident.context_codec.contract_ref
                        || capability_contract_ref != registered_capability_contract_ref
                        || intent_contract_ref != &intent_codec.contract_ref
                        || evidence_contract_ref != &evidence_codec.contract_ref
                    {
                        return Err(RuntimeError::IncompatibleAssembly);
                    }
                    let adapter = bindings
                        .get(binding_ref)
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    ExecutableMode::Read {
                        incident: Arc::clone(incident),
                        start: *start,
                        validate_retained: *validate_retained,
                        adapter: Arc::clone(adapter),
                        intent_codec: Arc::clone(intent_codec),
                        evidence_codec: Arc::clone(evidence_codec),
                    }
                }
                Execution::Effect {
                    error_contract_ref,
                    context_contract_ref,
                    capability_contract_ref,
                    command_contract_ref,
                    evidence_contract_ref,
                    binding_ref,
                    ..
                } => {
                    let RegisteredMode::Effect {
                        incident,
                        capability_contract_ref: registered_capability_contract_ref,
                        prepare,
                        start_pending,
                        validate_prepare,
                        validate_evidence,
                    } = &registered.mode
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    let Some(CapabilityRegistration::Effect {
                        command_codec,
                        evidence_codec,
                        bindings,
                        ..
                    }) = self
                        .inner
                        .capabilities
                        .get(registered_capability_contract_ref)
                    else {
                        return Err(RuntimeError::IncompatibleAssembly);
                    };
                    if error_contract_ref != &incident.error_codec.contract_ref
                        || context_contract_ref != &incident.context_codec.contract_ref
                        || capability_contract_ref != registered_capability_contract_ref
                        || command_contract_ref != &command_codec.contract_ref
                        || evidence_contract_ref != &evidence_codec.contract_ref
                    {
                        return Err(RuntimeError::IncompatibleAssembly);
                    }
                    let adapter = bindings
                        .get(binding_ref)
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    ExecutableMode::Effect {
                        incident: Arc::clone(incident),
                        prepare: *prepare,
                        start_pending: *start_pending,
                        validate_prepare: *validate_prepare,
                        validate_evidence: *validate_evidence,
                        adapter: Arc::clone(adapter),
                        command_codec: Arc::clone(command_codec),
                        evidence_codec: Arc::clone(evidence_codec),
                    }
                }
            };
            declarations.push(ExecutableState {
                is_checkpoint: false,
                output_codec,
                failure_codec,
                mode,
                recovery: self
                    .inner
                    .associate_recovery(registered.classify, state.handler())?,
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
        let root_failure_codec = self
            .inner
            .values
            .get(program.root_failure_contract_ref())
            .cloned()
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        Ok(ExecutableProgram {
            program,
            declarations,
            admitted_context_codec,
            root_failure_codec,
            _assembly: Arc::clone(&self.inner),
        })
    }

    pub(crate) fn codec(&self, contract_ref: &ContentRef) -> Option<Arc<ValueCodec>> {
        self.inner.values.get(contract_ref).cloned()
    }

    pub(crate) fn handle(&self) -> Arc<AssemblyInner> {
        Arc::clone(&self.inner)
    }
}

pub(crate) struct ExecutableProgram {
    pub(crate) program: Program,
    pub(crate) declarations: Vec<ExecutableState>,
    pub(crate) admitted_context_codec: Arc<ValueCodec>,
    pub(crate) root_failure_codec: Arc<ValueCodec>,
    pub(crate) _assembly: Arc<AssemblyInner>,
}

pub(crate) struct ExecutableState {
    pub(crate) is_checkpoint: bool,
    pub(crate) recovery: recovery::AssociatedRecovery,
    pub(crate) root_map: recovery::AssociatedRootMap,
    pub(crate) output_codec: Arc<ValueCodec>,
    pub(crate) failure_codec: Arc<ValueCodec>,
    pub(crate) mode: ExecutableMode,
}

pub(crate) enum ExecutableMode {
    Pure {
        start: StateStart,
    },
    Read {
        incident: Arc<AdapterIncidentContract>,
        start: ReadStart,
        validate_retained: ReadValidator,
        adapter: Arc<ErasedReadAdapterCallback>,
        intent_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
    },
    Effect {
        incident: Arc<AdapterIncidentContract>,
        prepare: StateStart,
        start_pending: EffectPendingStart,
        validate_prepare: EffectPrepareValidator,
        validate_evidence: EffectEvidenceValidator,
        adapter: Arc<ErasedEffectAdapterCallback>,
        command_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
    },
}
