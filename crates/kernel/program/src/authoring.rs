use std::marker::PhantomData;

use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EntryPointId, StatePosition};
use mfm_values::MfmValue;

use crate::recovery::{
    defaults::{RecoveryDefaults, SelectedRecovery},
    scope::{ScopeId, ScopedBoundary},
};
use crate::*;

const MAX_AUTHORING_CALLBACK_DEPTH: u8 = 16;

// Retain the designated descriptor on the heap while nested prefix callbacks execute.
struct PendingOccurrence {
    execution: Execution,
    selected: SelectedRecovery,
}

/// Deterministic authoring-only composition of typed States and child Operations.
pub trait Operation: Sized {
    /// Complete input contract.
    type Input: MfmValue;
    /// Complete normal success contract.
    type Output: MfmValue;
    /// Local root failure contract.
    type Failure: MfmValue;
    /// Checks that a root input agrees with this Operation's checked planning assumptions.
    /// Child inputs are established by the parent's planning and deterministic State contracts.
    fn validate_input(&self, input: &Self::Input) -> Result<()>;
    /// Emits the complete immutable sequence through the sole compiler context.
    fn expand(
        &self,
        expansion: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> Result<()>;
}

/// Expands one immutable linear Program, including selected recovery and complete size bounds.
pub fn expand_program<O: Operation>(
    entry: EntryPointId,
    root: &O,
    input: &O::Input,
    limits: ProgramLimits,
) -> Result<Program> {
    root.validate_input(input)?;
    let (_, initial_value_ref) =
        mfm_values::canonicalize_mfm_value(input).map_err(|_| ProgramError::InvalidContract)?;
    let mut expansion =
        OperationExpansion::<O::Input, O::Output, O::Failure>::new(1, RecoveryDefaults::default())?;
    root.expand(&mut expansion)?;
    expansion.finish()?;
    let declarations = expansion.draft.lower()?;
    Program::new(
        entry,
        nominal_contract_ref::<O::Input>()?,
        initial_value_ref,
        nominal_contract_ref::<O::Output>()?,
        nominal_contract_ref::<O::Failure>()?,
        declarations,
        limits,
    )
}

/// One scoped linear authoring context; defaults are resolved at typed State emission.
pub struct OperationExpansion<I, O, F> {
    draft: ExpansionDraft,
    recovery_defaults: RecoveryDefaults,
    depth: u8,
    marker: PhantomData<fn(I) -> (O, F)>,
}
impl<I: MfmValue, O: MfmValue, F: MfmValue> OperationExpansion<I, O, F> {
    fn new(depth: u8, recovery_defaults: RecoveryDefaults) -> Result<Self> {
        if depth > MAX_AUTHORING_CALLBACK_DEPTH {
            return Err(ProgramError::Capacity);
        }
        Ok(Self {
            draft: ExpansionDraft::new(nominal_contract_ref::<I>()?),
            recovery_defaults,
            depth,
            marker: PhantomData,
        })
    }
    fn child_depth(&self) -> Result<u8> {
        self.depth
            .checked_add(1)
            .filter(|depth| *depth <= MAX_AUTHORING_CALLBACK_DEPTH)
            .ok_or(ProgramError::Capacity)
    }

    fn finish(&self) -> Result<()> {
        self.draft.require_current(&nominal_contract_ref::<O>()?)
    }
    /// Installs the nearest explicit classifier family, without changing handler defaults.
    pub fn classifiers(&mut self, family: Classifiers) -> Result<()> {
        self.recovery_defaults.classifiers = Some(family);
        Ok(())
    }
    /// Installs a handler family after checking every attached checkpoint's ownership.
    pub fn handlers(&mut self, family: Handlers) -> Result<()> {
        family.require_scope(&self.draft.scope)?;
        self.recovery_defaults.handlers = Some(family);
        Ok(())
    }
    /// Sets local committed-decision allowances; descendants may override them independently.
    pub fn allowances(&mut self, allowances: RecoveryAllowances) -> Result<()> {
        self.recovery_defaults.allowances = allowances;
        Ok(())
    }
    /// Captures the current exact input boundary in this authoring scope.
    pub fn checkpoint<T: MfmValue>(&mut self) -> Result<Checkpoint<T>> {
        let input = nominal_contract_ref::<T>()?;
        self.draft.require_current(&input)?;
        Ok(Checkpoint::at(
            self.draft.scope.clone(),
            self.draft.states.len(),
            input,
        ))
    }
    /// Appends one Pure State with an explicit root map, policy overrides, and conclusion bound.
    pub fn pure<S, M>(
        &mut self,
        root_map: M::Params,
        policy: Occurrence,
        bound: ConclusionBound,
    ) -> Result<()>
    where
        S: PureState,
        M: ValueMap<Input = S::Failure, Output = F>,
    {
        let selected = self
            .recovery_defaults
            .resolve::<S::Failure, Never, NoContext>(&self.draft.scope, &policy)?;
        self.draft.append::<S>(
            Execution::Pure { bound },
            selected,
            vec![MapBinding::new::<M>(&root_map)?],
        )
    }
    /// Expands one child, composing each local root map with the explicit parent map.
    pub fn operation<C, M>(&mut self, child: &C, root_map: M::Params) -> Result<()>
    where
        C: Operation,
        M: ValueMap<Input = C::Failure, Output = F>,
    {
        let depth = self.child_depth()?;
        self.draft
            .require_current(&nominal_contract_ref::<C::Input>()?)?;
        let mut expansion = OperationExpansion::<C::Input, C::Output, C::Failure>::new(
            depth,
            self.recovery_defaults.clone(),
        )?;
        child.expand(&mut expansion)?;
        expansion.finish()?;
        expansion
            .draft
            .extend_root(MapBinding::new::<M>(&root_map)?);
        self.draft.merge(expansion.draft)
    }
    /// Appends an exact Read pair, preserving capability-owned typed injection setup.
    pub fn read<S, C, M>(
        &mut self,
        setup: &C::Setup,
        root_map: M::Params,
        policy: Occurrence,
        bound: ConclusionBound,
    ) -> Result<()>
    where
        C: ReadCapabilityContract + CapabilityInjection<S>,
        S: ReadState<C>,
        M: ValueMap<Input = C::ExpandedFailure, Output = F>,
    {
        let depth = self.child_depth()?;
        let selected = self
            .recovery_defaults
            .resolve::<S::Failure, C::OperationalError, S::AdapterContext>(
                &self.draft.scope,
                &policy,
            )?;
        let binding_ref = C::original_binding_ref(setup)?;
        let execution = Execution::Read {
            capability_contract_ref: capability_contract_ref::<C>()?,
            intent_contract_ref: nominal_contract_ref::<C::Intent>()?,
            evidence_contract_ref: nominal_contract_ref::<C::Evidence>()?,
            error_contract_ref: nominal_contract_ref::<C::OperationalError>()?,
            context_contract_ref: nominal_contract_ref::<S::AdapterContext>()?,
            binding_ref,
            bound,
        };
        self.capability::<S, C, M>(
            setup,
            root_map,
            depth,
            Box::new(PendingOccurrence {
                execution,
                selected,
            }),
        )
    }
    /// Appends an exact Effect pair with separate preparation and conclusion bounds.
    pub fn effect<S, C, M>(
        &mut self,
        setup: &C::Setup,
        root_map: M::Params,
        policy: Occurrence,
        bounds: EffectBounds,
    ) -> Result<()>
    where
        C: EffectCapabilityContract + CapabilityInjection<S>,
        S: EffectState<C>,
        M: ValueMap<Input = C::ExpandedFailure, Output = F>,
    {
        let depth = self.child_depth()?;
        let selected = self
            .recovery_defaults
            .resolve::<S::Failure, C::OperationalError, S::AdapterContext>(
                &self.draft.scope,
                &policy,
            )?;
        let binding_ref = C::original_binding_ref(setup)?;
        let execution = Execution::Effect {
            capability_contract_ref: effect_capability_contract_ref::<C>()?,
            command_contract_ref: nominal_contract_ref::<C::Command>()?,
            evidence_contract_ref: nominal_contract_ref::<C::Evidence>()?,
            error_contract_ref: nominal_contract_ref::<C::OperationalError>()?,
            context_contract_ref: nominal_contract_ref::<S::AdapterContext>()?,
            binding_ref,
            bounds,
        };
        self.capability::<S, C, M>(
            setup,
            root_map,
            depth,
            Box::new(PendingOccurrence {
                execution,
                selected,
            }),
        )
    }
    fn capability<S, C, M>(
        &mut self,
        setup: &C::Setup,
        root_map: M::Params,
        depth: u8,
        pending: Box<PendingOccurrence>,
    ) -> Result<()>
    where
        S: State,
        C: CapabilityInjection<S>,
        M: ValueMap<Input = C::ExpandedFailure, Output = F>,
    {
        self.draft
            .require_current(&nominal_contract_ref::<C::ExpandedInput>()?)?;
        let mut before = OperationExpansion::<C::ExpandedInput, S::Input, C::ExpandedFailure>::new(
            depth,
            self.recovery_defaults.clone(),
        )?;
        C::write_before(setup, &mut before)?;
        before.finish()?;
        let PendingOccurrence {
            execution,
            selected,
        } = *pending;
        before.draft.append::<S>(
            execution,
            selected,
            vec![MapBinding::new::<C::FailureMap>(&C::failure_map_params(
                setup,
            )?)?],
        )?;
        let mut after =
            OperationExpansion::<S::Output, C::ExpandedOutput, C::ExpandedFailure>::new(
                depth,
                self.recovery_defaults.clone(),
            )?;
        C::write_after(setup, &mut after)?;
        after.finish()?;
        before.draft.merge(after.draft)?;
        before.draft.extend_root(MapBinding::new::<M>(&root_map)?);
        self.draft.merge(before.draft)
    }
}

/// Deterministic expansion of one exact capability/State pair.
pub trait CapabilityInjection<S: State> {
    /// Checked immutable authoring setup.
    type Setup;
    /// Complete input of the injected sequence.
    type ExpandedInput: MfmValue;
    /// Complete output of the injected sequence.
    type ExpandedOutput: MfmValue;
    /// Failure contract shared by the injected sequence's local root paths.
    type ExpandedFailure: MfmValue;
    /// Explicit designated-State failure conversion into the expanded failure contract.
    type FailureMap: ValueMap<Input = S::Failure, Output = Self::ExpandedFailure>;
    /// Exact parameters for the designated State's root failure conversion.
    fn failure_map_params(setup: &Self::Setup) -> Result<<Self::FailureMap as ValueMap>::Params>;
    /// Resolves the designated occurrence's immutable binding exactly once.
    fn original_binding_ref(setup: &Self::Setup) -> Result<ContentRef>;
    /// Authors the prefix in its own default/checkpoint scope.
    fn write_before(
        _setup: &Self::Setup,
        _scope: &mut OperationExpansion<Self::ExpandedInput, S::Input, Self::ExpandedFailure>,
    ) -> Result<()> {
        Ok(())
    }
    /// Authors the successful suffix; it is not a finally handler.
    fn write_after(
        _setup: &Self::Setup,
        _scope: &mut OperationExpansion<S::Output, Self::ExpandedOutput, Self::ExpandedFailure>,
    ) -> Result<()> {
        Ok(())
    }
}

struct DraftState {
    state: StateDeclaration,
    checkpoints: Vec<ScopedBoundary>,
}
struct ExpansionDraft {
    scope: ScopeId,
    origins: Vec<(ScopeId, usize)>,
    input: ContentRef,
    states: Vec<DraftState>,
}
impl ExpansionDraft {
    fn new(input: ContentRef) -> Self {
        let scope = ScopeId::new();
        Self {
            origins: vec![(scope.clone(), 0)],
            scope,
            input,
            states: Vec::new(),
        }
    }
    fn current(&self) -> &ContentRef {
        self.states
            .last()
            .map_or(&self.input, |s| s.state.output_contract_ref())
    }
    fn require_current(&self, expected: &ContentRef) -> Result<()> {
        if self.current() != expected {
            return Err(ProgramError::InvalidContract);
        }
        Ok(())
    }
    fn append<S: State>(
        &mut self,
        execution: Execution,
        selected: SelectedRecovery,
        root_maps: Vec<MapBinding>,
    ) -> Result<()> {
        let input_contract_ref = nominal_contract_ref::<S::Input>()?;
        self.require_current(&input_contract_ref)?;
        if self.states.len() >= MAX_STATES {
            return Err(ProgramError::Capacity);
        }
        self.states.push(DraftState {
            state: StateDeclaration {
                state_implementation_ref: state_implementation_ref::<S>()?,
                input_contract_ref,
                output_contract_ref: nominal_contract_ref::<S::Output>()?,
                failure_contract_ref: nominal_contract_ref::<S::Failure>()?,
                execution,
                classifier: selected.classifier,
                handler: selected.handler,
                root_maps,
                recovery_targets: Vec::new(),
                allowances: selected.allowances,
            },
            checkpoints: selected.checkpoints,
        });
        Ok(())
    }
    fn extend_root(&mut self, map: MapBinding) {
        for state in &mut self.states {
            state.state.root_maps.push(map.clone());
        }
    }
    fn merge(&mut self, mut child: Self) -> Result<()> {
        self.require_current(&child.input)?;
        let offset = self.states.len();
        if offset
            .checked_add(child.states.len())
            .ok_or(ProgramError::Capacity)?
            > MAX_STATES
        {
            return Err(ProgramError::Capacity);
        }
        for (_, origin) in &mut child.origins {
            *origin = origin.checked_add(offset).ok_or(ProgramError::Capacity)?;
        }
        self.origins.extend(child.origins);
        self.states.extend(child.states);
        Ok(())
    }
    fn lower(mut self) -> Result<Vec<StateDeclaration>> {
        for index in 0..self.states.len() {
            let targets = self.states[index]
                .checkpoints
                .iter()
                .map(|checkpoint| {
                    let base = self
                        .origins
                        .iter()
                        .find(|(scope, _)| checkpoint.belongs_to(scope))
                        .map(|(_, offset)| *offset)
                        .ok_or(ProgramError::InvalidContract)?;
                    let target = base
                        .checked_add(checkpoint.offset)
                        .ok_or(ProgramError::Capacity)?;
                    if target > index
                        || self.states.get(target).is_none_or(|state| {
                            state.state.input_contract_ref() != &checkpoint.input
                        })
                    {
                        return Err(ProgramError::InvalidContract);
                    }
                    Ok(RecoveryTarget {
                        position: StatePosition::new(target).map_err(|_| ProgramError::Capacity)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            self.states[index].state.recovery_targets = targets;
        }
        Ok(self.states.into_iter().map(|state| state.state).collect())
    }
}
