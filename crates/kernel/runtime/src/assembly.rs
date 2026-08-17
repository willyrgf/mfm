use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::{ContentRef, SchemaId, SemanticTypeId};
use mfm_program::{
    capability_contract_ref, nominal_contract_ref, state_implementation_ref, Declaration, Never,
    Program, PureState, ReadState, StateDeclaration,
};
use mfm_values::{canonicalize_mfm_value, EnumTagging, MfmValue, SchemaDescriptor, SchemaShape};
use serde_json::value::RawValue;

use crate::engine::{self, DriverContext, DriverDisposition};
use crate::{ReadAdapterError, Result, RuntimeError};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type AdapterCallback<C> = dyn for<'a> Fn(
        &'a <C as ReadCapabilityContract>::Intent,
    ) -> BoxFuture<
        'a,
        std::result::Result<<C as ReadCapabilityContract>::Evidence, ReadAdapterError>,
    > + Send
    + Sync;
pub(crate) type EvidenceQualification =
    Box<dyn FnOnce() -> std::result::Result<QualifiedValue, mfm_values::ValueError> + Send>;
pub(crate) type ErasedAdapterCallback = dyn for<'a> Fn(
        &'a QualifiedValue,
    ) -> BoxFuture<'a, std::result::Result<EvidenceQualification, ReadAdapterError>>
    + Send
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
enum StateMode {
    Pure,
    Read(Box<ReadSignature>),
}

#[derive(Clone, PartialEq, Eq)]
struct ReadSignature {
    capability_type: TypeId,
    capability_contract_ref: ContentRef,
    intent_contract_ref: ContentRef,
    evidence_contract_ref: ContentRef,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct StateSignature {
    state_type: TypeId,
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    mode: StateMode,
}

pub(crate) trait RegisteredState: Send + Sync {
    fn signature(&self) -> &StateSignature;

    fn associate(
        &self,
        declaration: &StateDeclaration,
        adapters: &AdapterRegistry,
    ) -> Result<Arc<dyn RegisteredState>>;

    fn validate_retained_read(
        &self,
        intent: &QualifiedValue,
        evidence: &QualifiedValue,
    ) -> Result<()>;

    fn start<'a>(
        &'a self,
        input: QualifiedValue,
        context: DriverContext<'a>,
    ) -> BoxFuture<'a, Result<DriverDisposition>>;
}

struct PureDriver<S> {
    signature: StateSignature,
    marker: std::marker::PhantomData<fn() -> S>,
}

impl<S: PureState> RegisteredState for PureDriver<S> {
    fn signature(&self) -> &StateSignature {
        &self.signature
    }

    fn associate(
        &self,
        declaration: &StateDeclaration,
        _adapters: &AdapterRegistry,
    ) -> Result<Arc<dyn RegisteredState>> {
        if !declaration.execution().is_pure() || !declaration_matches(&self.signature, declaration)
        {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        Ok(Arc::new(Self {
            signature: self.signature.clone(),
            marker: std::marker::PhantomData,
        }))
    }

    fn validate_retained_read(
        &self,
        _intent: &QualifiedValue,
        _evidence: &QualifiedValue,
    ) -> Result<()> {
        Err(RuntimeError::InvalidHistory)
    }

    fn start<'a>(
        &'a self,
        input: QualifiedValue,
        context: DriverContext<'a>,
    ) -> BoxFuture<'a, Result<DriverDisposition>> {
        Box::pin(engine::start_pure::<S>(input, context))
    }
}

struct ReadDriver<S, C>
where
    C: ReadCapabilityContract,
{
    signature: StateSignature,
    adapter: Option<Arc<ErasedAdapterCallback>>,
    marker: std::marker::PhantomData<fn() -> (S, C)>,
}

impl<S, C> RegisteredState for ReadDriver<S, C>
where
    S: ReadState<C>,
    C: ReadCapabilityContract,
{
    fn signature(&self) -> &StateSignature {
        &self.signature
    }

    fn associate(
        &self,
        declaration: &StateDeclaration,
        adapters: &AdapterRegistry,
    ) -> Result<Arc<dyn RegisteredState>> {
        if declaration.execution().is_pure() || !declaration_matches(&self.signature, declaration) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let StateMode::Read(read) = &self.signature.mode else {
            return Err(RuntimeError::IncompatibleAssembly);
        };
        let binding_ref = declaration
            .execution()
            .binding_ref()
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        let entry = adapters
            .get(&read.capability_contract_ref)
            .and_then(|bindings| bindings.get(binding_ref))
            .ok_or(RuntimeError::IncompatibleAssembly)?;
        if entry.capability_type != TypeId::of::<C>() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        Ok(Arc::new(Self {
            signature: self.signature.clone(),
            adapter: Some(Arc::clone(&entry.callback)),
            marker: std::marker::PhantomData,
        }))
    }

    fn validate_retained_read(
        &self,
        intent: &QualifiedValue,
        evidence: &QualifiedValue,
    ) -> Result<()> {
        let intent = intent
            .typed
            .downcast_ref::<C::Intent>()
            .ok_or(RuntimeError::InvalidHistory)?;
        let evidence = evidence
            .typed
            .downcast_ref::<C::Evidence>()
            .ok_or(RuntimeError::InvalidHistory)?;
        C::bind_evidence(intent, evidence).map_err(|_| RuntimeError::InvalidHistory)
    }

    fn start<'a>(
        &'a self,
        input: QualifiedValue,
        context: DriverContext<'a>,
    ) -> BoxFuture<'a, Result<DriverDisposition>> {
        let Some(adapter) = self.adapter.as_ref() else {
            return Box::pin(async { Err(RuntimeError::Internal) });
        };
        Box::pin(engine::start_read::<S, C>(
            input,
            context,
            Arc::clone(adapter),
        ))
    }
}

fn declaration_matches(signature: &StateSignature, declaration: &StateDeclaration) -> bool {
    if declaration.state_implementation_ref() != &signature.state_implementation_ref
        || declaration.input_contract_ref() != &signature.input_contract_ref
        || declaration.output_contract_ref() != &signature.output_contract_ref
        || declaration.failure_contract_ref() != &signature.failure_contract_ref
    {
        return false;
    }
    match &signature.mode {
        StateMode::Pure => declaration.execution().is_pure(),
        StateMode::Read(read) => {
            declaration.execution().capability_contract_ref() == Some(&read.capability_contract_ref)
                && declaration.execution().intent_contract_ref() == Some(&read.intent_contract_ref)
                && declaration.execution().evidence_contract_ref()
                    == Some(&read.evidence_contract_ref)
        }
    }
}

struct CatchAdapterPanic<'a, T> {
    inner: BoxFuture<'a, std::result::Result<T, ReadAdapterError>>,
}

impl<T> Future for CatchAdapterPanic<'_, T> {
    type Output = std::result::Result<T, ReadAdapterError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner.as_mut().poll(context)
        }))
        .unwrap_or(Poll::Ready(Err(ReadAdapterError::Internal)))
    }
}

fn erase_adapter<C, F>(callback: F) -> Arc<ErasedAdapterCallback>
where
    C: ReadCapabilityContract,
    F: for<'a> Fn(
            &'a C::Intent,
        ) -> Pin<
            Box<
                dyn Future<Output = std::result::Result<C::Evidence, ReadAdapterError>> + Send + 'a,
            >,
        > + Send
        + Sync
        + 'static,
{
    let callback: Arc<AdapterCallback<C>> = Arc::new(callback);
    Arc::new(move |qualified_intent| {
        let Some(intent) = qualified_intent.typed.downcast_ref::<C::Intent>() else {
            return Box::pin(async { Err(ReadAdapterError::Internal) });
        };
        let future =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (callback)(intent))) {
                Ok(future) => future,
                Err(_) => return Box::pin(async { Err(ReadAdapterError::Internal) }),
            };
        Box::pin(async move {
            let evidence = CatchAdapterPanic { inner: future }.await?;
            Ok(Box::new(move || qualify_hot(evidence)) as EvidenceQualification)
        })
    })
}

pub(crate) struct AdapterEntry {
    capability_type: TypeId,
    callback: Arc<ErasedAdapterCallback>,
}

type AdapterRegistry = BTreeMap<ContentRef, BTreeMap<ContentRef, AdapterEntry>>;

#[derive(Clone, PartialEq, Eq)]
struct CapabilitySignature {
    capability_type: TypeId,
    intent_contract_ref: ContentRef,
    evidence_contract_ref: ContentRef,
}

/// Mutable builder for one immutable Runtime assembly.
pub struct RuntimeAssemblyBuilder {
    values: BTreeMap<ContentRef, Arc<ValueCodec>>,
    semantics: BTreeMap<SemanticTypeId, ContentRef>,
    states: BTreeMap<ContentRef, Arc<dyn RegisteredState>>,
    capabilities: BTreeMap<ContentRef, CapabilitySignature>,
    adapters: AdapterRegistry,
    invalid: bool,
}

#[allow(clippy::new_without_default)] // The frozen public surface intentionally has no Default impl.
impl RuntimeAssemblyBuilder {
    /// Constructs an empty builder with the framework Never codec installed.
    pub fn new() -> Self {
        let mut builder = Self {
            values: BTreeMap::new(),
            semantics: BTreeMap::new(),
            states: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            adapters: BTreeMap::new(),
            invalid: false,
        };
        if builder.register_value::<Never>().is_err() {
            builder.invalid = true;
        }
        builder
    }

    /// Registers one exact value codec, idempotently.
    pub fn register_value<T: MfmValue>(&mut self) -> Result<()> {
        if self.invalid {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let descriptor = T::schema_descriptor().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        let semantic = T::semantic_id().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let contract_ref =
            nominal_contract_ref::<T>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        let key = contract_ref.clone();
        if let Some(previous) = self.values.get(&key) {
            return (previous.codec_type_id == TypeId::of::<T>()
                && previous.semantic_id == semantic)
                .then_some(())
                .ok_or(RuntimeError::IncompatibleAssembly);
        }
        if self
            .semantics
            .get(&semantic)
            .is_some_and(|previous| previous != &key)
        {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        self.semantics.insert(semantic.clone(), key.clone());
        self.values.insert(
            key,
            Arc::new(ValueCodec {
                codec_type_id: TypeId::of::<T>(),
                semantic_id: semantic,
                contract_ref,
                descriptor,
                qualify: qualify_typed::<T>,
            }),
        );
        Ok(())
    }

    /// Registers one Pure State and its complete value ABI.
    pub fn register_pure<S: PureState>(&mut self) -> Result<()> {
        self.register_value::<S::Input>()?;
        self.register_value::<S::Output>()?;
        self.register_value::<S::Failure>()?;
        let signature = state_signature::<S>(StateMode::Pure)?;
        self.register_state(
            signature.clone(),
            Arc::new(PureDriver::<S> {
                signature,
                marker: std::marker::PhantomData,
            }),
        )
    }

    /// Registers one Read State, capability, and complete value ABI.
    pub fn register_read<S, C>(&mut self) -> Result<()>
    where
        S: ReadState<C>,
        C: ReadCapabilityContract,
    {
        self.register_value::<S::Input>()?;
        self.register_value::<S::Output>()?;
        self.register_value::<S::Failure>()?;
        self.register_value::<C::Intent>()?;
        self.register_value::<C::Evidence>()?;
        let capability = self.ensure_capability::<C>()?;
        let signature = state_signature::<S>(StateMode::Read(Box::new(ReadSignature {
            capability_type: TypeId::of::<C>(),
            capability_contract_ref: capability_contract_ref::<C>()
                .map_err(|_| RuntimeError::IncompatibleAssembly)?,
            intent_contract_ref: capability.intent_contract_ref,
            evidence_contract_ref: capability.evidence_contract_ref,
        })))?;
        self.register_state(
            signature.clone(),
            Arc::new(ReadDriver::<S, C> {
                signature,
                adapter: None,
                marker: std::marker::PhantomData,
            }),
        )
    }

    /// Registers one typed adapter under its internally derived binding ref.
    pub fn register_adapter<C, B, F>(&mut self, binding: B, callback: F) -> Result<()>
    where
        C: ReadCapabilityContract,
        B: MfmValue,
        F: for<'a> Fn(
                &'a C::Intent,
            ) -> Pin<
                Box<
                    dyn Future<Output = std::result::Result<C::Evidence, ReadAdapterError>>
                        + Send
                        + 'a,
                >,
            > + Send
            + Sync
            + 'static,
    {
        let capability = self.ensure_capability::<C>()?;
        let capability_ref =
            capability_contract_ref::<C>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if capability.capability_type != TypeId::of::<C>() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let (_, binding_ref) =
            canonicalize_mfm_value(&binding).map_err(|_| RuntimeError::IncompatibleAssembly)?;
        let bindings = self.adapters.entry(capability_ref).or_default();
        if bindings.contains_key(&binding_ref) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        bindings.insert(
            binding_ref,
            AdapterEntry {
                capability_type: TypeId::of::<C>(),
                callback: erase_adapter::<C, F>(callback),
            },
        );
        Ok(())
    }

    /// Finalizes the immutable assembly.
    pub fn finish(self) -> Result<RuntimeAssembly> {
        if self.invalid {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        Ok(RuntimeAssembly {
            inner: Arc::new(AssemblyInner {
                values: self.values,
                states: self.states,
                adapters: self.adapters,
            }),
        })
    }

    fn register_state(
        &mut self,
        signature: StateSignature,
        driver: Arc<dyn RegisteredState>,
    ) -> Result<()> {
        let key = signature.state_implementation_ref.clone();
        if let Some(previous) = self.states.get(&key) {
            return (previous.signature() == &signature)
                .then_some(())
                .ok_or(RuntimeError::IncompatibleAssembly);
        }
        self.states.insert(key, driver);
        Ok(())
    }

    fn ensure_capability<C: ReadCapabilityContract>(&mut self) -> Result<CapabilitySignature> {
        self.register_value::<C::Intent>()?;
        self.register_value::<C::Evidence>()?;
        let contract =
            capability_contract_ref::<C>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        let signature = CapabilitySignature {
            capability_type: TypeId::of::<C>(),
            intent_contract_ref: nominal_contract_ref::<C::Intent>()
                .map_err(|_| RuntimeError::IncompatibleAssembly)?,
            evidence_contract_ref: nominal_contract_ref::<C::Evidence>()
                .map_err(|_| RuntimeError::IncompatibleAssembly)?,
        };
        let key = contract;
        if let Some(previous) = self.capabilities.get(&key) {
            if previous != &signature {
                return Err(RuntimeError::IncompatibleAssembly);
            }
        } else {
            self.capabilities.insert(key, signature.clone());
        }
        Ok(signature)
    }
}

fn state_signature<S: mfm_program::State>(mode: StateMode) -> Result<StateSignature> {
    Ok(StateSignature {
        state_type: TypeId::of::<S>(),
        state_implementation_ref: state_implementation_ref::<S>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?,
        input_contract_ref: nominal_contract_ref::<S::Input>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?,
        output_contract_ref: nominal_contract_ref::<S::Output>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?,
        failure_contract_ref: nominal_contract_ref::<S::Failure>()
            .map_err(|_| RuntimeError::IncompatibleAssembly)?,
        mode,
    })
}

/// Finalized immutable typed assembly.
pub struct RuntimeAssembly {
    pub(crate) inner: Arc<AssemblyInner>,
}

pub(crate) struct AssemblyInner {
    values: BTreeMap<ContentRef, Arc<ValueCodec>>,
    states: BTreeMap<ContentRef, Arc<dyn RegisteredState>>,
    adapters: AdapterRegistry,
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
                    let registered = self
                        .inner
                        .states
                        .get(state.state_implementation_ref())
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    let bound = registered.associate(state, &self.inner.adapters)?;
                    let output_codec = self
                        .codec(state.output_contract_ref())
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    let failure_codec = self
                        .codec(state.failure_contract_ref())
                        .ok_or(RuntimeError::IncompatibleAssembly)?;
                    let read_codecs = match (
                        state.execution().intent_contract_ref(),
                        state.execution().evidence_contract_ref(),
                    ) {
                        (None, None) if state.execution().is_pure() => None,
                        (Some(intent), Some(evidence)) if !state.execution().is_pure() => {
                            Some(ReadCodecs {
                                intent: self
                                    .codec(intent)
                                    .ok_or(RuntimeError::IncompatibleAssembly)?,
                                evidence: self
                                    .codec(evidence)
                                    .ok_or(RuntimeError::IncompatibleAssembly)?,
                            })
                        }
                        _ => return Err(RuntimeError::IncompatibleAssembly),
                    };
                    declarations.push(ExecutableDeclaration::State(ExecutableState {
                        driver: bound,
                        output_codec,
                        failure_codec,
                        read_codecs,
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
    pub(crate) driver: Arc<dyn RegisteredState>,
    pub(crate) output_codec: Arc<ValueCodec>,
    pub(crate) failure_codec: Arc<ValueCodec>,
    pub(crate) read_codecs: Option<ReadCodecs>,
}

pub(crate) struct ReadCodecs {
    pub(crate) intent: Arc<ValueCodec>,
    pub(crate) evidence: Arc<ValueCodec>,
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

#[cfg(test)]
#[path = "assembly/tests.rs"]
mod tests;
