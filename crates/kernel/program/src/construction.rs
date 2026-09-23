//! One private draft for fresh traversal and exact installed-code discovery.

use crate::{
    executable::{self, ExecutableMode, ExecutableState},
    program::{ProgramDocument, StateData},
    *,
};
use mfm_ids::{ContentRef, EntryPointId, StatePosition};
use mfm_values::{MfmValue, Object, SchemaDescriptor};
use std::{any::TypeId, collections::BTreeMap};

pub(crate) fn rejection(operation: &'static str, fields: &impl Serialize) -> ProgramError {
    ProgramError::Diagnostic(mfm_values::InvocationDiagnostic::from_fields(
        "program_contract",
        operation,
        fields,
        None,
    ))
}

/// Installed source roots, independent of the source value selected for this compilation.
pub trait ProgramEnvironment {
    /// Installed source types discovered for both fresh compilation and config-free loading.
    type Sources;
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StateAbi {
    implementation: ContentRef,
    input: ContentRef,
    output: ContentRef,
    failure: ContentRef,
    execution: Code,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Code {
    Pure,
    Read(NativeAbi),
    Effect(NativeAbi),
}
impl StateAbi {
    fn recorded(state: &StateDeclaration) -> Self {
        Self {
            implementation: state.state_implementation_ref().clone(),
            input: state.input_contract_ref().clone(),
            output: state.output_contract_ref().clone(),
            failure: state.failure_contract_ref().clone(),
            execution: match state.execution() {
                Execution::Pure {} => Code::Pure,
                Execution::Read { abi, .. } => Code::Read(abi.clone()),
                Execution::Effect { abi, .. } => Code::Effect(abi.clone()),
            },
        }
    }
}

pub(crate) struct Contracts {
    values: BTreeMap<ContentRef, (TypeId, SchemaDescriptor)>,
    native: BTreeMap<(ContentRef, StableId), (TypeId, NativeAbi)>,
    states: BTreeMap<StateAbi, TypeId>,
    handlers: BTreeMap<HandlerAbi, TypeId>,
}
impl Contracts {
    fn new() -> Self {
        Self {
            values: BTreeMap::new(),
            native: BTreeMap::new(),
            states: BTreeMap::new(),
            handlers: BTreeMap::new(),
        }
    }
    pub(crate) fn insert<T: MfmValue>(&mut self) -> Result<ContentRef> {
        let (reference, descriptor) = derive_nominal_contract::<T>()?;
        let claim = (TypeId::of::<T>(), descriptor);
        if let Some(old) = self.values.get(&reference) {
            if old != &claim {
                return Err(rejection(
                    "claim_value",
                    &serde_json::json!({
                        "reason": "conflicting_owner", "contract": reference,
                    }),
                ));
            }
        } else {
            self.values.insert(reference.clone(), claim);
        }
        Ok(reference)
    }
    fn state<S: State>(&mut self, execution: Code) -> Result<StateAbi> {
        let abi = StateAbi {
            implementation: state_implementation_ref::<S>()?,
            input: self.insert::<S::Input>()?,
            output: self.insert::<S::Output>()?,
            failure: self.insert::<S::Failure>()?,
            execution,
        };
        let owner = TypeId::of::<S>();
        if self.states.get(&abi).is_some_and(|old| *old != owner) {
            return Err(rejection(
                "claim_state",
                &serde_json::json!({
                    "reason": "conflicting_owner", "implementation": abi.implementation,
                    "input": abi.input, "output": abi.output, "failure": abi.failure,
                }),
            ));
        }
        self.states.insert(abi.clone(), owner);
        Ok(abi)
    }
    fn handler<H: Handler>(&mut self) -> Result<HandlerAbi> {
        let params = self.insert::<H::Params>()?;
        let abi = HandlerAbi::from_contract::<H>(params)?;
        let owner = TypeId::of::<H>();
        if self.handlers.get(&abi).is_some_and(|old| *old != owner) {
            return Err(rejection(
                "claim_handler",
                &serde_json::json!({
                    "reason": "conflicting_owner", "handler": abi,
                }),
            ));
        }
        self.handlers.insert(abi.clone(), owner);
        Ok(abi)
    }
    pub(crate) fn require_value<T: MfmValue>(&self) -> Result<ContentRef> {
        let (reference, descriptor) = derive_nominal_contract::<T>()?;
        let reason = match self.values.get(&reference) {
            Some((owner, installed)) if *owner == TypeId::of::<T>() && installed == &descriptor => {
                return Ok(reference)
            }
            Some(_) => "conflicting_owner",
            None => "value_not_installed",
        };
        Err(rejection(
            "select_value",
            &serde_json::json!({"reason": reason, "contract": reference}),
        ))
    }
    fn require_state<S: State>(&self, execution: Code) -> Result<StateAbi> {
        let abi = StateAbi {
            implementation: state_implementation_ref::<S>()?,
            input: self.require_value::<S::Input>()?,
            output: self.require_value::<S::Output>()?,
            failure: self.require_value::<S::Failure>()?,
            execution,
        };
        let reason = match self.states.get(&abi) {
            Some(owner) if *owner == TypeId::of::<S>() => return Ok(abi),
            Some(_) => "conflicting_owner",
            None => "state_not_installed",
        };
        Err(rejection(
            "select_state",
            &serde_json::json!({
                "reason": reason, "implementation": abi.implementation,
                "input": abi.input, "output": abi.output, "failure": abi.failure,
            }),
        ))
    }
    fn require_handler<H: Handler>(&self) -> Result<HandlerAbi> {
        let abi = HandlerAbi::from_contract::<H>(self.require_value::<H::Params>()?)?;
        let reason = match self.handlers.get(&abi) {
            Some(owner) if *owner == TypeId::of::<H>() => return Ok(abi),
            Some(_) => "conflicting_owner",
            None => "handler_not_installed",
        };
        Err(rejection(
            "select_handler",
            &serde_json::json!({"reason": reason, "handler": abi}),
        ))
    }
    fn finish(self) -> BTreeMap<ContentRef, SchemaDescriptor> {
        self.values
            .into_iter()
            .map(|(reference, (_, descriptor))| (reference, descriptor))
            .collect()
    }
}

#[derive(Clone)]
pub(crate) struct Policy {
    binding: HandlerBinding,
    allowances: RecoveryAllowances,
    targets: Vec<(u64, TypeId, ContentRef)>,
}
impl Policy {
    fn fallback(abi: HandlerAbi) -> Result<Self> {
        Ok(Self {
            binding: HandlerBinding::from_abi(abi, &NoParams)?,
            allowances: RecoveryAllowances::default(),
            targets: Vec::new(),
        })
    }
}

pub(crate) struct Draft<'a> {
    pub(crate) contracts: &'a Contracts,
    declarations: Vec<StateData>,
    pub(crate) policy: Policy,
    depth: u8,
    scope: u64,
    next_scope: u64,
    markers: BTreeMap<(u64, TypeId), (usize, ContentRef)>,
    targets: Vec<Vec<(u64, TypeId, ContentRef)>>,
    bindings: BTreeMap<ContentRef, Object>,
}
impl<'a> Draft<'a> {
    fn new(contracts: &'a Contracts) -> Result<Self> {
        let fallback = contracts.require_handler::<Stop>()?;
        Ok(Self {
            contracts,
            declarations: Vec::new(),
            policy: Policy::fallback(fallback)?,
            depth: 0,
            scope: 0,
            next_scope: 1,
            markers: BTreeMap::new(),
            targets: Vec::new(),
            bindings: BTreeMap::new(),
        })
    }
    pub(crate) fn pure<S: PureState>(&mut self) -> Result<()> {
        let abi = self.contracts.require_state::<S>(Code::Pure)?;
        self.emit(abi, Execution::Pure {})
    }
    fn emit(&mut self, abi: StateAbi, execution: Execution) -> Result<()> {
        if self.declarations.len() >= MAX_STATES {
            return Err(ProgramError::Capacity);
        }
        self.targets.push(self.policy.targets.clone());
        self.declarations.push(StateData {
            state_implementation_ref: abi.implementation,
            input_contract_ref: abi.input,
            output_contract_ref: abi.output,
            failure_contract_ref: abi.failure,
            execution,
            handler: self.policy.binding.clone(),
            recovery_targets: Vec::new(),
            allowances: self.policy.allowances,
        });
        Ok(())
    }
    pub(crate) fn nested<T>(&mut self, walk: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= 16 {
            return Err(ProgramError::Capacity);
        }
        self.depth += 1;
        let result = walk(self);
        self.depth -= 1;
        result
    }
    pub(crate) fn position(&self) -> usize {
        self.declarations.len()
    }
    pub(crate) fn checkpoint<M: CheckpointMarker>(&mut self) -> Result<()> {
        let input = self.contracts.require_value::<M::Context>()?;
        let position = self.position();
        if let Some((previous, _)) = self
            .markers
            .insert((self.scope, TypeId::of::<M>()), (position, input.clone()))
        {
            return Err(rejection(
                "checkpoint",
                &serde_json::json!({
                    "reason": "duplicate_marker", "scope": self.scope,
                    "position": position, "previous_position": previous, "input": input,
                }),
            ));
        }
        Ok(())
    }
    fn lower(&mut self) -> Result<()> {
        for (offset, input) in self.markers.values() {
            let observed = self
                .declarations
                .get(*offset)
                .map(|state| &state.input_contract_ref);
            if observed != Some(input) {
                return Err(rejection(
                    "lower_checkpoint",
                    &serde_json::json!({
                        "reason": if observed.is_none() { "missing_state" } else { "input_mismatch" },
                        "position": offset, "expected_input": input, "observed_input": observed,
                    }),
                ));
            }
        }
        for (index, targets) in self.targets.iter().enumerate() {
            let mut selected = std::collections::BTreeSet::new();
            for (scope, marker, input) in targets {
                let (offset, contract) = self.markers.get(&(*scope, *marker)).ok_or_else(|| {
                    rejection(
                        "lower_checkpoint",
                        &serde_json::json!({
                            "reason": "marker_outside_scope", "scope": scope,
                            "position": index, "input": input,
                        }),
                    )
                })?;
                let reason = if *offset > index {
                    Some("forward_target")
                } else if contract != input {
                    Some("input_mismatch")
                } else if !selected.insert(*offset) {
                    Some("duplicate_target")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Err(rejection(
                        "lower_checkpoint",
                        &serde_json::json!({
                            "reason": reason, "scope": scope, "position": index,
                            "target_position": offset, "expected_input": input, "observed_input": contract,
                        }),
                    ));
                }
                self.declarations[index]
                    .recovery_targets
                    .push(RecoveryTarget {
                        position: StatePosition::new(*offset)?,
                    });
            }
        }
        Ok(())
    }
    pub(crate) fn scope<P, C: ?Sized, T>(
        &mut self,
        config: &C,
        walk: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T>
    where
        P: ResolveDefaults<C>,
    {
        let handler_abi = self.contracts.require_handler::<P::Handler>()?;
        let selected = P::resolve(config)?;
        let parent = self.policy.clone();
        let parent_scope = self.scope;
        self.scope = self.next_scope;
        self.next_scope = self
            .next_scope
            .checked_add(1)
            .ok_or(ProgramError::Capacity)?;
        if let Some(params) = selected.handler {
            self.policy.binding = HandlerBinding::from_abi(handler_abi, &params)?;
            self.policy.targets = crate::typed_source::targets::<P::Targets>()?
                .into_iter()
                .map(|(marker, contract)| (self.scope, marker, contract))
                .collect();
        }
        self.policy.allowances = RecoveryAllowances::new(
            selected.retries.unwrap_or(parent.allowances.retries()),
            selected.restarts.unwrap_or(parent.allowances.restarts()),
        );
        let result = walk(self);
        self.policy = parent;
        self.scope = parent_scope;
        result
    }
}

pub(crate) trait Walk<C: ?Sized, R>: AuthoringSource {
    fn walk(&self, config: &C, resources: &R, draft: &mut Draft) -> Result<()>;
}
pub(crate) trait Discover<R> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()>;
}
type BindExecutable<R> = Box<dyn FnOnce(&R) -> Result<ExecutableState>>;
type Construct<R> =
    fn(&StateDeclaration, &[Object], executable::BoundHandler) -> Result<BindExecutable<R>>;
type ConstructHandler = fn(&HandlerBinding) -> Result<executable::BoundHandler>;
/// Kind of an installed executable or named authoring component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Named reusable authoring definition.
    Operation,
    /// Deterministic State.
    PureState,
    /// Observational State.
    ReadState,
    /// Mutating State.
    EffectState,
}
/// Inspection facts discovered from the same owners used for cold association.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Component {
    kind: ComponentKind,
    id: StableId,
    description: &'static str,
}
impl Component {
    /// Execution or authoring kind.
    pub fn kind(&self) -> ComponentKind {
        self.kind
    }
    /// Stable owning-definition identity.
    pub fn id(&self) -> &StableId {
        &self.id
    }
    /// Non-semantic owner description.
    pub fn description(&self) -> &'static str {
        self.description
    }
}

/// Inspects installed source types without planning, configuration, resources, or IO.
#[allow(private_bounds)]
pub fn components<R>() -> Result<Vec<Component>>
where
    R: ProgramEnvironment,
    R::Sources: Discover<R>,
{
    let mut inventory = Inventory::<R>::new()?;
    R::Sources::discover(&mut inventory)?;
    Ok(inventory.components.into_values().collect())
}

pub(crate) struct Inventory<R> {
    contracts: Contracts,
    components: BTreeMap<(ComponentKind, StableId), Component>,
    states: BTreeMap<StateAbi, Construct<R>>,
    handlers: BTreeMap<HandlerAbi, ConstructHandler>,
}
impl<R> Inventory<R> {
    fn new() -> Result<Self> {
        let mut inventory = Self {
            contracts: Contracts::new(),
            components: BTreeMap::new(),
            states: BTreeMap::new(),
            handlers: BTreeMap::new(),
        };
        inventory.handler::<Stop>()?;
        Ok(inventory)
    }
    fn component(
        &mut self,
        kind: ComponentKind,
        id: StableId,
        description: &'static str,
    ) -> Result<()> {
        let component = Component {
            kind,
            id: id.clone(),
            description,
        };
        let key = (kind, id);
        if let Some(previous) = self.components.get(&key) {
            if previous != &component {
                return Err(rejection(
                    "inspect_component",
                    &serde_json::json!({
                        "reason": "conflicting_metadata", "previous": previous, "current": component,
                    }),
                ));
            }
        } else {
            self.components.insert(key, component);
        }
        Ok(())
    }
    pub(crate) fn operation<D: OperationDefinition>(&mut self) -> Result<()> {
        if let Some((id, description)) = D::metadata() {
            self.component(ComponentKind::Operation, StableId::new(id)?, description)?;
        }
        Ok(())
    }
    pub(crate) fn value<T: MfmValue>(&mut self) -> Result<()> {
        self.contracts.insert::<T>().map(|_| ())
    }
    pub(crate) fn handler<H: Handler>(&mut self) -> Result<()> {
        let abi = self.contracts.handler::<H>()?;
        self.handlers.insert(abi, executable::bind_handler::<H>);
        Ok(())
    }
    pub(crate) fn pure<S: PureState>(&mut self) -> Result<()> {
        let abi = self.contracts.state::<S>(Code::Pure)?;
        self.component(ComponentKind::PureState, S::state_id()?, S::description())?;
        self.states.insert(abi, |state, _, handle| {
            let failure = state.failure_contract_ref().clone();
            Ok(Box::new(move |_| {
                Ok(ExecutableState {
                    mode: ExecutableMode::Pure {
                        callbacks: callback::PureCallbacks::new::<S>(failure),
                    },
                    handle,
                    checkpoint: false,
                })
            }))
        });
        Ok(())
    }
}

/// Compiles a source into one complete Program using the environment's installed support.
#[allow(private_bounds)]
pub fn compile<S, R>(
    entry: EntryPointId,
    source: &S,
    input: &S::Input,
    resources: &R,
    limits: ProgramLimits,
) -> Result<Program>
where
    S: AuthoringSource + Walk<<S as AuthoringSource>::Input, R>,
    R: ProgramEnvironment,
    R::Sources: Discover<R>,
{
    let mut inventory = Inventory::new()?;
    R::Sources::discover(&mut inventory)?;
    let mut draft = Draft::new(&inventory.contracts)?;
    let input_contract = draft.contracts.require_value::<S::Input>()?;
    let output_contract = draft.contracts.require_value::<S::Output>()?;
    let initial = Object::from_value(input)
        .map_err(|cause| ProgramError::Diagnostic(cause.into_diagnostic("compile_input")))?;
    source.walk(input, resources, &mut draft)?;
    draft.lower()?;
    let document = ProgramDocument::new(
        entry,
        input_contract,
        initial.value_ref().clone(),
        output_contract,
        draft.declarations,
        limits,
        draft.bindings.into_values().collect(),
    )?;
    inventory.associate(document, resources)
}

/// Reconstructs exact executable code from the retained document and installed source types.
#[allow(private_bounds)]
pub fn load<R>(canonical_program: &[u8], resources: &R) -> Result<Program>
where
    R: ProgramEnvironment,
    R::Sources: Discover<R>,
{
    let document = ProgramDocument::decode(canonical_program)?;
    let mut inventory = Inventory::new()?;
    R::Sources::discover(&mut inventory)?;
    inventory.associate(document, resources)
}

impl<R> Inventory<R> {
    fn associate(self, document: ProgramDocument, resources: &R) -> Result<Program> {
        for contract in [
            document.admitted_context_contract_ref(),
            document.root_success_contract_ref(),
        ] {
            if !self.contracts.values.contains_key(contract) {
                return Err(rejection(
                    "associate",
                    &serde_json::json!({
                        "reason": "endpoint_contract_not_installed", "contract": contract,
                    }),
                ));
            }
        }
        let selected = document
            .declarations()
            .iter()
            .enumerate()
            .map(|(position, state)| {
                let reject = |reason| {
                    rejection(
                        "associate",
                        &serde_json::json!({
                            "reason": reason, "position": position,
                            "implementation": state.state_implementation_ref(),
                            "input": state.input_contract_ref(), "output": state.output_contract_ref(),
                            "failure": state.failure_contract_ref(), "execution": state.execution(),
                            "handler": state.handler().abi(),
                            "handler_params": state.handler().params().value_ref(),
                        }),
                    )
                };
                let construct = self
                    .states
                    .get(&StateAbi::recorded(state))
                    .ok_or_else(|| reject("state_not_installed"))?;
                let construct_handler = self
                    .handlers
                    .get(state.handler().abi())
                    .ok_or_else(|| reject("handler_not_installed"))?;
                if let Execution::Read { abi, binding_ref } | Execution::Effect { abi, binding_ref } =
                    state.execution()
                {
                    let index = document
                        .bindings
                        .binary_search_by(|object| object.value_ref().cmp(binding_ref))
                        .map_err(|_| reject("binding_not_retained"))?;
                    let descriptor = &self
                        .contracts
                        .values
                        .get(abi.binding())
                        .ok_or_else(|| reject("binding_contract_not_installed"))?
                        .1;
                    document.bindings[index]
                        .admit(descriptor)
                        .map_err(|cause| {
                            ProgramError::Diagnostic(cause.into_diagnostic("associate_binding"))
                        })?;
                }
                let handle = construct_handler(state.handler())?;
                construct(state, &document.bindings, handle)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut executables = selected
            .into_iter()
            .map(|bind| bind(resources))
            .collect::<Result<Vec<_>>>()?;
        for state in document.declarations() {
            for target in state.recovery_targets() {
                executables[target.position().index()].checkpoint = true;
            }
        }
        Program::freeze(document, executables, self.contracts.finish())
    }
}

// Both modes share the same binding custody and exact-descriptor admission. The semantic/native
// associated types and typed adapter calls remain monomorphized at each concrete leaf.
macro_rules! native_leaf {
    ($method:ident, $require:ident, $mode:ident, $state:ident, $capability:ident, $implementation:ident, $binder:ident,
     $bind:ident, $leaf:ident, $callbacks:ident, $adapter:ident, $request:ident, $native:ident, $from_contracts:ident) => {
        impl Contracts {
            fn $method<C, I>(&mut self) -> Result<NativeAbi>
            where
                C: mfm_capabilities::$capability,
                I: mfm_capabilities::$implementation<C>,
            {
                let contracts = [
                    self.insert::<C::$request>()?,
                    self.insert::<C::Evidence>()?,
                    self.insert::<I::$native>()?,
                    self.insert::<I::NativeEvidence>()?,
                    self.insert::<I::OperationalError>()?,
                    self.insert::<I::Binding>()?,
                ];
                let abi = NativeAbi::$from_contracts::<C, I>(contracts)?;
                let key = (abi.capability.clone(), I::implementation_id()?);
                let claim = (TypeId::of::<(C, I)>(), abi.clone());
                if self.native.get(&key).is_some_and(|old| old != &claim) {
                    return Err(rejection("claim_native", &serde_json::json!({
                        "reason": "conflicting_owner", "capability": key.0,
                        "implementation": key.1, "abi": abi,
                    })));
                }
                self.native.insert(key, claim);
                Ok(abi)
            }
        }
        impl Contracts {
            fn $require<C, I>(&self) -> Result<NativeAbi>
            where C: mfm_capabilities::$capability, I: mfm_capabilities::$implementation<C>,
            {
                let contracts = [
                    self.require_value::<C::$request>()?, self.require_value::<C::Evidence>()?,
                    self.require_value::<I::$native>()?, self.require_value::<I::NativeEvidence>()?,
                    self.require_value::<I::OperationalError>()?, self.require_value::<I::Binding>()?,
                ];
                let abi = NativeAbi::$from_contracts::<C, I>(contracts)?;
                let key = (abi.capability.clone(), I::implementation_id()?);
                let reason = match self.native.get(&key) {
                    Some((owner, installed)) if *owner == TypeId::of::<(C, I)>() && installed == &abi => return Ok(abi),
                    Some(_) => "conflicting_owner",
                    None => "native_not_installed",
                };
                Err(rejection("select_native", &serde_json::json!({
                    "reason": reason, "capability": key.0, "implementation": key.1, "abi": abi,
                })))
            }
        }
        impl Draft<'_> {
            pub(crate) fn $method<S, C, I>(
                &mut self,
                binding: &I::Binding,
            ) -> Result<()>
            where
                S: $state<C>,
                C: mfm_capabilities::$capability,
                I: mfm_capabilities::$implementation<C>,
                I::OperationalError: ClassifyError,
            {
                let abi = self.contracts.$require::<C, I>()?;
                let state_abi = self.contracts.require_state::<S>(Code::$mode(abi.clone()))?;
                let object = Object::from_value(binding).map_err(|cause| {
                    ProgramError::Diagnostic(cause.into_diagnostic("compile_binding"))
                })?;
                self.emit(
                    state_abi,
                    Execution::$mode {
                        abi,
                        binding_ref: object.value_ref().clone(),
                    },
                )?;
                self.bindings.insert(object.value_ref().clone(), object);
                Ok(())
            }
        }
        impl<R> Inventory<R> {
            pub(crate) fn $method<S, C, I>(&mut self) -> Result<()>
            where
                S: $state<C>,
                C: mfm_capabilities::$capability,
                I: mfm_capabilities::$implementation<C>,
                I::OperationalError: ClassifyError,
                R: $binder<C, I>,
            {
                let native = self.contracts.$method::<C, I>()?;
                self.component(ComponentKind::$state, S::state_id()?, S::description())?;
                let abi = self.contracts.state::<S>(Code::$mode(native))?;
                self.states
                    .insert(abi, |state, bindings, handle| {
                        let Execution::$mode { abi, binding_ref } = state.execution() else {
                            return Err(rejection("bind_native", &serde_json::json!({
                                "reason": "execution_mode_mismatch", "expected": stringify!($mode),
                                "implementation": state.state_implementation_ref(),
                                "input": state.input_contract_ref(), "output": state.output_contract_ref(),
                                "failure": state.failure_contract_ref(), "execution": state.execution(),
                                "handler": state.handler().abi(),
                                "handler_params": state.handler().params().value_ref(),
                            })));
                        };
                        let index = bindings
                            .binary_search_by(|object| object.value_ref().cmp(binding_ref))
                            .map_err(|_| rejection("bind_native", &serde_json::json!({
                                "reason": "binding_not_retained", "binding": binding_ref, "abi": abi,
                            })))?;
                        let object = &bindings[index];
                        $leaf::<S, C, I, R>(object, abi, state.failure_contract_ref(), handle)
                    });
                Ok(())
            }
        }
        fn $leaf<S, C, I, R>(
            object: &Object,
            abi: &NativeAbi,
            failure: &ContentRef,
            handle: executable::BoundHandler,
        ) -> Result<BindExecutable<R>>
        where
            S: $state<C>,
            C: mfm_capabilities::$capability,
            I: mfm_capabilities::$implementation<C>,
            I::OperationalError: ClassifyError,
            R: $binder<C, I>,
        {
            let binding = object.decode::<I::Binding>().map_err(ProgramError::Diagnostic)?;
            let abi = abi.clone();
            let failure = failure.clone();
            let reference = object.value_ref().clone();
            Ok(Box::new(move |resources| {
                let adapter = resources.$bind(&binding).map_err(ProgramError::Diagnostic)?;
                let binding = std::sync::Arc::new(binding);
                Ok(ExecutableState {
                    mode: ExecutableMode::$mode {
                        callbacks: callback::$callbacks::new::<S, C, I>(
                            failure,
                            abi.implementation.clone(),
                            reference.clone(),
                            std::sync::Arc::clone(&binding),
                        ),
                        adapter: callback::$adapter::<C, I, _>(
                            abi.operational_error,
                            abi.implementation,
                            reference,
                            binding,
                            adapter,
                        ),
                    },
                    handle,
                    checkpoint: false,
                })
            }))
        }

    };
}

native_leaf!(
    read,
    require_read,
    Read,
    ReadState,
    ReadCapabilityContract,
    ReadImplementation,
    BindRead,
    bind_read,
    read_executable,
    ReadCallbacks,
    read_adapter,
    Intent,
    NativeIntent,
    read_from_contracts
);
native_leaf!(
    effect,
    require_effect,
    Effect,
    EffectState,
    EffectCapabilityContract,
    EffectImplementation,
    BindEffect,
    bind_effect,
    effect_executable,
    EffectCallbacks,
    effect_adapter,
    Command,
    NativeCommand,
    effect_from_contracts
);
