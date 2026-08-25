use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, SchemaId, SemanticTypeId};
use mfm_program::{
    capability_contract_ref, effect_capability_contract_ref, nominal_contract_ref,
    state_implementation_ref, Declaration, EffectState, Execution, Never, Program, PureState,
    ReadState, StateDeclaration,
};
use mfm_values::{canonicalize_mfm_value, EnumTagging, MfmValue, SchemaDescriptor, SchemaShape};
use serde_json::value::RawValue;

use crate::engine::{self, DriverContext, DriverDisposition};
use crate::{AdapterError, EffectAdapterOutcome, Result, RuntimeError};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub(crate) type EvidenceQualification =
    Box<dyn FnOnce() -> std::result::Result<QualifiedValue, mfm_values::ValueError> + Send>;
pub(crate) type ErasedReadAdapterCallback = dyn for<'a> Fn(
        &'a QualifiedValue,
    ) -> BoxFuture<'a, std::result::Result<EvidenceQualification, AdapterError>>
    + Send
    + Sync;
pub(crate) type ErasedEffectAdapterCallback = dyn for<'a> Fn(
        &'a EffectId,
        &'a QualifiedValue,
    ) -> BoxFuture<
        'a,
        std::result::Result<EffectAdapterOutcome<EvidenceQualification>, AdapterError>,
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
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StateAbiKey {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
}

impl From<&StateSignature> for StateAbiKey {
    fn from(signature: &StateSignature) -> Self {
        Self {
            state_implementation_ref: signature.state_implementation_ref.clone(),
            input_contract_ref: signature.input_contract_ref.clone(),
            output_contract_ref: signature.output_contract_ref.clone(),
            failure_contract_ref: signature.failure_contract_ref.clone(),
        }
    }
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

pub(crate) type PureStart =
    for<'a> fn(QualifiedValue, DriverContext<'a>) -> BoxFuture<'a, Result<DriverDisposition>>;
pub(crate) type ReadStart = for<'a> fn(
    QualifiedValue,
    DriverContext<'a>,
    Arc<ErasedReadAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>;
pub(crate) type EffectPrepareStart =
    for<'a> fn(QualifiedValue, DriverContext<'a>) -> BoxFuture<'a, Result<DriverDisposition>>;
pub(crate) type EffectPendingStart = for<'a> fn(
    QualifiedValue,
    EffectId,
    QualifiedValue,
    DriverContext<'a>,
    Arc<ErasedEffectAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>;
type ReadValidator = fn(&QualifiedValue, &QualifiedValue) -> Result<()>;
type EffectPrepareValidator = fn(&QualifiedValue, &QualifiedValue) -> Result<()>;
type EffectEvidenceValidator = fn(&EffectId, &QualifiedValue, &QualifiedValue) -> Result<()>;

struct RegisteredState {
    signature: StateSignature,
    mode: RegisteredMode,
}

enum RegisteredMode {
    Pure {
        start: PureStart,
    },
    Read {
        capability_contract_ref: ContentRef,
        start: ReadStart,
        validate_retained: ReadValidator,
    },
    Effect {
        capability_contract_ref: ContentRef,
        prepare: EffectPrepareStart,
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
                    capability_contract_ref,
                    ..
                },
                RegisteredMode::Read {
                    capability_contract_ref: other,
                    ..
                },
            ) => capability_contract_ref == other,
            (
                RegisteredMode::Effect {
                    capability_contract_ref,
                    ..
                },
                RegisteredMode::Effect {
                    capability_contract_ref: other,
                    ..
                },
            ) => capability_contract_ref == other,
            _ => false,
        }
    }
}

fn start_pure<'a, S: PureState>(
    input: QualifiedValue,
    context: DriverContext<'a>,
) -> BoxFuture<'a, Result<DriverDisposition>> {
    Box::pin(engine::start_pure::<S>(input, context))
}

fn start_read<'a, S, C>(
    input: QualifiedValue,
    context: DriverContext<'a>,
    adapter: Arc<ErasedReadAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>
where
    S: ReadState<C>,
    C: ReadCapabilityContract,
{
    Box::pin(engine::start_read::<S, C>(input, context, adapter))
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

fn prepare_effect<'a, S, C>(
    input: QualifiedValue,
    context: DriverContext<'a>,
) -> BoxFuture<'a, Result<DriverDisposition>>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
{
    Box::pin(engine::start_effect::<S, C>(input, context))
}

fn start_pending_effect<'a, S, C>(
    input: QualifiedValue,
    effect_id: EffectId,
    command: QualifiedValue,
    context: DriverContext<'a>,
    adapter: Arc<ErasedEffectAdapterCallback>,
) -> BoxFuture<'a, Result<DriverDisposition>>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
{
    Box::pin(engine::start_pending_effect::<S, C>(
        input, effect_id, command, context, adapter,
    ))
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

struct CatchAdapterPanic<'a, T> {
    inner: BoxFuture<'a, std::result::Result<T, AdapterError>>,
}

impl<T> Future for CatchAdapterPanic<'_, T> {
    type Output = std::result::Result<T, AdapterError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner.as_mut().poll(context)
        }))
        .unwrap_or(Poll::Ready(Err(AdapterError::Internal)))
    }
}

fn erase_read_adapter<C, F>(callback: F) -> Arc<ErasedReadAdapterCallback>
where
    C: ReadCapabilityContract,
    F: for<'a> Fn(
            &'a ContentRef,
            &'a C::Intent,
        ) -> Pin<
            Box<dyn Future<Output = std::result::Result<C::Evidence, AdapterError>> + Send + 'a>,
        > + Send
        + Sync
        + 'static,
{
    Arc::new(move |qualified_intent| {
        let Some(intent) = qualified_intent.typed.downcast_ref::<C::Intent>() else {
            return Box::pin(async { Err(AdapterError::Internal) });
        };
        let future = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback(&qualified_intent.value_ref, intent)
        })) {
            Ok(future) => future,
            Err(_) => return Box::pin(async { Err(AdapterError::Internal) }),
        };
        Box::pin(async move {
            let evidence = CatchAdapterPanic { inner: future }.await?;
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
                            AdapterError,
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
            return Box::pin(async { Err(AdapterError::Internal) });
        };
        let future = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback(effect_id, &qualified_command.value_ref, command)
        })) {
            Ok(future) => future,
            Err(_) => return Box::pin(async { Err(AdapterError::Internal) }),
        };
        Box::pin(async move {
            match (CatchAdapterPanic { inner: future }).await? {
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
        intent_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
        bindings: BTreeMap<ContentRef, Arc<ErasedReadAdapterCallback>>,
    },
    Effect {
        capability_type: TypeId,
        command_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
        bindings: BTreeMap<ContentRef, Arc<ErasedEffectAdapterCallback>>,
    },
}

/// Mutable builder for one immutable Runtime assembly.
pub struct RuntimeAssemblyBuilder {
    values: BTreeMap<ContentRef, Arc<ValueCodec>>,
    states: BTreeMap<StateAbiKey, RegisteredState>,
    capabilities: BTreeMap<ContentRef, CapabilityRegistration>,
}

impl RuntimeAssemblyBuilder {
    /// Constructs an empty builder with the framework Never codec installed.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::IncompatibleAssembly`] if the framework codec contract is invalid.
    pub fn new() -> Result<Self> {
        let mut builder = Self {
            values: BTreeMap::new(),
            states: BTreeMap::new(),
            capabilities: BTreeMap::new(),
        };
        builder.ensure_value::<Never>()?;
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
    {
        let input = self.ensure_value::<S::Input>()?;
        let output = self.ensure_value::<S::Output>()?;
        let failure = self.ensure_value::<S::Failure>()?;
        let capability_contract_ref = self.ensure_read_capability::<C>()?;
        self.register_state(RegisteredState {
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Read {
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
    {
        let input = self.ensure_value::<S::Input>()?;
        let output = self.ensure_value::<S::Output>()?;
        let failure = self.ensure_value::<S::Failure>()?;
        let capability_contract_ref = self.ensure_effect_capability::<C>()?;
        self.register_state(RegisteredState {
            signature: state_signature::<S>(&input, &output, &failure)?,
            mode: RegisteredMode::Effect {
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
                    dyn Future<Output = std::result::Result<C::Evidence, AdapterError>> + Send + 'a,
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
                                AdapterError,
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
                values: self.values,
                states: self.states,
                capabilities: self.capabilities,
            }),
        }
    }

    fn register_state(&mut self, state: RegisteredState) -> Result<()> {
        let key = StateAbiKey::from(&state.signature);
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
        let capability_ref =
            capability_contract_ref::<C>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if let Some(registration) = self.capabilities.get(&capability_ref) {
            let CapabilityRegistration::Read {
                capability_type,
                intent_codec: registered_intent,
                evidence_codec: registered_evidence,
                ..
            } = registration
            else {
                return Err(RuntimeError::IncompatibleAssembly);
            };
            if *capability_type != TypeId::of::<C>()
                || !Arc::ptr_eq(registered_intent, &intent_codec)
                || !Arc::ptr_eq(registered_evidence, &evidence_codec)
            {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            return Ok(capability_ref);
        }
        self.capabilities.insert(
            capability_ref.clone(),
            CapabilityRegistration::Read {
                capability_type: TypeId::of::<C>(),
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
        let capability_ref = effect_capability_contract_ref::<C>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if let Some(registration) = self.capabilities.get(&capability_ref) {
            let CapabilityRegistration::Effect {
                capability_type,
                command_codec: registered_command,
                evidence_codec: registered_evidence,
                ..
            } = registration
            else {
                return Err(RuntimeError::IncompatibleAssembly);
            };
            if *capability_type != TypeId::of::<C>()
                || !Arc::ptr_eq(registered_command, &command_codec)
                || !Arc::ptr_eq(registered_evidence, &evidence_codec)
            {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            return Ok(capability_ref);
        }
        self.capabilities.insert(
            capability_ref.clone(),
            CapabilityRegistration::Effect {
                capability_type: TypeId::of::<C>(),
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
        state_implementation_ref: state_implementation_ref::<S>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?,
        input_contract_ref: input.contract_ref.clone(),
        output_contract_ref: output.contract_ref.clone(),
        failure_contract_ref: failure.contract_ref.clone(),
    })
}

/// Finalized immutable typed assembly.
pub struct RuntimeAssembly {
    pub(crate) inner: Arc<AssemblyInner>,
}

pub(crate) struct AssemblyInner {
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
        for declaration in program.declarations() {
            match declaration {
                Declaration::State(state) => {
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
                        Execution::Pure => {
                            let RegisteredMode::Pure { start } = &registered.mode else {
                                return Err(RuntimeError::IncompatibleAssembly);
                            };
                            ExecutableMode::Pure { start: *start }
                        }
                        Execution::Read {
                            capability_contract_ref,
                            intent_contract_ref,
                            evidence_contract_ref,
                            binding_ref,
                        } => {
                            let RegisteredMode::Read {
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
                            if capability_contract_ref != registered_capability_contract_ref
                                || intent_contract_ref != &intent_codec.contract_ref
                                || evidence_contract_ref != &evidence_codec.contract_ref
                            {
                                return Err(RuntimeError::IncompatibleAssembly);
                            }
                            let adapter = bindings
                                .get(binding_ref)
                                .ok_or(RuntimeError::IncompatibleAssembly)?;
                            ExecutableMode::Read {
                                start: *start,
                                validate_retained: *validate_retained,
                                adapter: Arc::clone(adapter),
                                intent_codec: Arc::clone(intent_codec),
                                evidence_codec: Arc::clone(evidence_codec),
                            }
                        }
                        Execution::Effect {
                            capability_contract_ref,
                            command_contract_ref,
                            evidence_contract_ref,
                            binding_ref,
                        } => {
                            let RegisteredMode::Effect {
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
                            if capability_contract_ref != registered_capability_contract_ref
                                || command_contract_ref != &command_codec.contract_ref
                                || evidence_contract_ref != &evidence_codec.contract_ref
                            {
                                return Err(RuntimeError::IncompatibleAssembly);
                            }
                            let adapter = bindings
                                .get(binding_ref)
                                .ok_or(RuntimeError::IncompatibleAssembly)?;
                            ExecutableMode::Effect {
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
                    declarations.push(ExecutableDeclaration::State(ExecutableState {
                        output_codec,
                        failure_codec,
                        mode,
                    }));
                }
                Declaration::Match(selector) => {
                    let codec = self
                        .codec(selector.selector_contract_ref())
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    declarations.push(ExecutableDeclaration::Match(associate_match(
                        codec,
                        selector.variants(),
                        program.declarations(),
                        &self.inner,
                    )?));
                }
            }
        }
        Ok(ExecutableProgram {
            program,
            declarations,
            admitted_context_codec,
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
    pub(crate) declarations: Vec<ExecutableDeclaration>,
    pub(crate) admitted_context_codec: Arc<ValueCodec>,
    pub(crate) _assembly: Arc<AssemblyInner>,
}

pub(crate) enum ExecutableDeclaration {
    State(ExecutableState),
    Match(MatchProjection),
}

pub(crate) struct ExecutableState {
    pub(crate) output_codec: Arc<ValueCodec>,
    pub(crate) failure_codec: Arc<ValueCodec>,
    pub(crate) mode: ExecutableMode,
}

pub(crate) enum ExecutableMode {
    Pure {
        start: PureStart,
    },
    Read {
        start: ReadStart,
        validate_retained: ReadValidator,
        adapter: Arc<ErasedReadAdapterCallback>,
        intent_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
    },
    Effect {
        prepare: EffectPrepareStart,
        start_pending: EffectPendingStart,
        validate_prepare: EffectPrepareValidator,
        validate_evidence: EffectEvidenceValidator,
        adapter: Arc<ErasedEffectAdapterCallback>,
        command_codec: Arc<ValueCodec>,
        evidence_codec: Arc<ValueCodec>,
    },
}

pub(crate) struct MatchProjection {
    selector_contract_ref: ContentRef,
    tagging: EnumTagging,
    variants: Vec<VariantProjection>,
}

struct VariantProjection {
    tag: String,
    entry_index: u16,
    codec: Arc<ValueCodec>,
}

impl MatchProjection {
    pub(crate) fn project(&self, selector: QualifiedValue) -> Result<(u16, QualifiedValue)> {
        if selector.contract_ref != self.selector_contract_ref {
            return Err(RuntimeError::Internal);
        }

        let (selected_tag, payload_bytes) =
            selected_match_payload(&self.tagging, &selector.canonical)?;
        let variant = self
            .variants
            .iter()
            .find(|variant| variant.tag == selected_tag)
            .ok_or(RuntimeError::Internal)?;
        let value_ref = ContentRef::new(
            variant.codec.contract_ref.schema_id().clone(),
            raw_content_digest(payload_bytes),
        )
        .map_err(|_| RuntimeError::Internal)?;
        let payload = variant
            .codec
            .qualify(&value_ref, payload_bytes)
            .map_err(|_| RuntimeError::Internal)?;
        Ok((variant.entry_index, payload))
    }
}

fn selected_match_payload<'a>(
    tagging: &EnumTagging,
    selector: &'a PlainCanonicalJsonBytes,
) -> Result<(String, &'a [u8])> {
    let fields: BTreeMap<String, &'a RawValue> =
        serde_json::from_slice(selector.as_bytes()).map_err(|_| RuntimeError::Internal)?;

    match tagging {
        EnumTagging::External if fields.len() == 1 => {
            let (tag, payload) = fields.into_iter().next().ok_or(RuntimeError::Internal)?;
            Ok((tag, payload.get().as_bytes()))
        }
        EnumTagging::Adjacent { tag, content } if fields.len() == 2 => {
            let selected = serde_json::from_str::<String>(
                fields.get(tag).ok_or(RuntimeError::Internal)?.get(),
            )
            .map_err(|_| RuntimeError::Internal)?;
            let payload = fields.get(content).ok_or(RuntimeError::Internal)?;
            Ok((selected, payload.get().as_bytes()))
        }
        EnumTagging::External | EnumTagging::Adjacent { .. } | EnumTagging::Internal { .. } => {
            Err(RuntimeError::Internal)
        }
    }
}

fn associate_match(
    selector_codec: Arc<ValueCodec>,
    arms: &[mfm_program::MatchVariant],
    declarations: &[Declaration],
    assembly: &AssemblyInner,
) -> Result<MatchProjection> {
    let shape = selector_codec
        .descriptor
        .identity()
        .canonical_json_shape()
        .map_err(|_| RuntimeError::IncompatibleAssembly)?;
    let SchemaShape::Enum { tagging, variants } = shape else {
        return Err(RuntimeError::IncompatibleAssembly);
    };
    if !matches!(
        tagging,
        EnumTagging::External | EnumTagging::Adjacent { .. }
    ) {
        return Err(RuntimeError::IncompatibleAssembly);
    }
    if variants.len() != arms.len() {
        return Err(RuntimeError::IncompatibleAssembly);
    }
    let mut projections = Vec::with_capacity(variants.len());
    for (variant, arm) in variants.iter().zip(arms) {
        if variant.name != arm.tag().as_str() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let (schema_id, semantic_id, serialized_shape) =
            match_payload_descriptor(&variant.shape).ok_or(RuntimeError::IncompatibleAssembly)?;
        let payload_contract =
            ContentRef::new(schema_id.clone(), raw_content_digest(b"mfm.contract.v1"))
                .map_err(|_| RuntimeError::IncompatibleAssembly)?;
        let target = declarations
            .get(usize::from(arm.entry_index()))
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        let Declaration::State(target) = target else {
            return Err(RuntimeError::IncompatibleAssembly);
        };
        if target.input_contract_ref() != &payload_contract {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let codec = assembly
            .values
            .get(&payload_contract)
            .cloned()
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        let codec_shape = codec
            .descriptor
            .identity()
            .canonical_json_shape()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if &codec.semantic_id != semantic_id || codec_shape != serialized_shape {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        projections.push(VariantProjection {
            tag: variant.name.clone(),
            entry_index: arm.entry_index(),
            codec,
        });
    }
    Ok(MatchProjection {
        selector_contract_ref: selector_codec.contract_ref.clone(),
        tagging: tagging.clone(),
        variants: projections,
    })
}

fn match_payload_descriptor(
    shape: &SchemaShape,
) -> Option<(&SchemaId, &SemanticTypeId, &SchemaShape)> {
    let payload = match shape {
        SchemaShape::Tuple(elements) if elements.len() == 1 => &elements[0],
        other => other,
    };

    match payload {
        SchemaShape::InlineValue {
            schema_id,
            semantic_type_id,
            serialized_shape,
        } => Some((schema_id, semantic_type_id, serialized_shape.as_ref())),
        SchemaShape::Generic {
            constructor,
            arguments,
            serialized_shape,
        } if constructor == "mfm/generic-value" => {
            let [argument] = arguments.as_slice() else {
                return None;
            };
            Some((
                &argument.schema_id,
                &argument.semantic_type_id,
                serialized_shape.as_ref(),
            ))
        }
        _ => None,
    }
}
