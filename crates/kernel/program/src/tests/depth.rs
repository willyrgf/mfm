use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Copy)]
enum Nesting {
    Child,
    Injection,
    Mixed,
}
#[derive(Clone)]
struct Nested {
    remaining: usize,
    nesting: Nesting,
    entered: Arc<AtomicUsize>,
    suffixes: Arc<AtomicUsize>,
}
impl Nested {
    fn emit(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        // A rejected subtree must not leak this successfully authored prefix into its parent.
        scope.pure::<Pass, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(1024)?,
        )?;
        if self.remaining == 1 {
            return Ok(());
        }
        let child = Self {
            remaining: self.remaining - 1,
            ..self.clone()
        };
        if matches!(self.nesting, Nesting::Child)
            || matches!(self.nesting, Nesting::Mixed) && self.remaining.is_multiple_of(2)
        {
            scope.operation::<Self, Identity<Never>>(&child, NoParams)
        } else {
            scope.effect::<Pass, NestedEffect, Identity<Never>>(
                &child,
                NoParams,
                Occurrence::new(),
                EffectBounds::new(1024, 1024)?,
            )
        }
    }
}
impl Operation for Nested {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn validate_input(&self, _: &Value) -> Result<()> {
        Ok(())
    }
    fn expand(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        self.emit(scope)
    }
}
struct NestedEffect;
impl EffectCapabilityContract for NestedEffect {
    type Command = Value;
    type Evidence = Value;
    type OperationalError = NoContext;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Effect::contract_id()
    }
    fn bind_evidence(
        id: &mfm_ids::EffectId,
        command: &Value,
        evidence: &Value,
    ) -> mfm_capabilities::Result<()> {
        Effect::bind_evidence(id, command, evidence)
    }
}
impl EffectState<NestedEffect> for Pass {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &Value,
        _: &Value,
        _: &NoContext,
    ) -> std::result::Result<NoContext, StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(input: &Value) -> std::result::Result<Value, PreparationError> {
        <Self as EffectState<Effect>>::prepare(input)
    }
    fn interpret(
        input: Value,
        evidence: &Value,
    ) -> std::result::Result<ProposedStateOutcome<Value, Never>, StateExecutionError> {
        <Self as EffectState<Effect>>::interpret(input, evidence)
    }
}
impl CapabilityInjection<Pass> for NestedEffect {
    type Setup = Nested;
    type ExpandedInput = Value;
    type ExpandedOutput = Value;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &Nested) -> Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(_: &Nested) -> Result<ContentRef> {
        nominal_contract_ref::<Value>()
    }
    fn write_before(
        setup: &Nested,
        scope: &mut OperationExpansion<Value, Value, Never>,
    ) -> Result<()> {
        setup.emit(scope)
    }
    fn write_after(
        setup: &Nested,
        scope: &mut OperationExpansion<Value, Value, Never>,
    ) -> Result<()> {
        setup.suffixes.fetch_add(1, Ordering::SeqCst);
        let checkpoint = scope.checkpoint::<Value>()?;
        let mut handlers = Handlers::new();
        handlers.bind::<Cause, Stop>(NoParams)?;
        handlers.checkpoint::<Cause, Value>(&checkpoint)?;
        scope.handlers(handlers)?;
        scope.pure::<Pass, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(1024)?,
        )
    }
}
struct Catch(Nested);
impl Operation for Catch {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn validate_input(&self, _: &Value) -> Result<()> {
        Ok(())
    }
    fn expand(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        assert_eq!(
            scope.operation::<Nested, Identity<Never>>(&self.0, NoParams),
            Err(ProgramError::Capacity)
        );
        scope.pure::<Pass, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(1024)?,
        )
    }
}

#[test]
fn child_and_injection_callbacks_share_the_sixteen_level_limit_without_partial_merge() {
    for nesting in [Nesting::Child, Nesting::Injection, Nesting::Mixed] {
        let entered = Arc::new(AtomicUsize::new(0));
        let suffixes = Arc::new(AtomicUsize::new(0));
        let root = Nested {
            remaining: 16,
            nesting,
            entered: entered.clone(),
            suffixes: suffixes.clone(),
        };
        let entry = EntryPointId::new("mfm.test/depth@1").unwrap();
        let input = Value { number: 0 };
        expand_program(entry.clone(), &root, &input, ProgramLimits::new(0)).unwrap();
        assert_eq!(entered.swap(0, Ordering::SeqCst), 16);
        suffixes.store(0, Ordering::SeqCst);
        let rejected = Nested {
            remaining: 17,
            ..root.clone()
        };
        assert!(matches!(
            expand_program(entry.clone(), &rejected, &input, ProgramLimits::new(0)),
            Err(ProgramError::Capacity)
        ));
        assert_eq!(entered.swap(0, Ordering::SeqCst), 16);
        assert_eq!(suffixes.load(Ordering::SeqCst), 0);
        let caught =
            expand_program(entry, &Catch(rejected), &input, ProgramLimits::new(0)).unwrap();
        assert_eq!(caught.declarations().len(), 1);
        assert!(caught.declarations()[0].recovery_targets().is_empty());
        assert_eq!(entered.load(Ordering::SeqCst), 15);
        assert_eq!(suffixes.load(Ordering::SeqCst), 0);
    }
}
