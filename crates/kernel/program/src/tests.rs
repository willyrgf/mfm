use crate::*;
use mfm_program_derive::MfmValue;

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Value {
    number: u64,
}

struct Pass;
impl State for Pass {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn state_id() -> Result<StableId> {
        StableId::new("mfm.test.pass@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Pass {
    fn evaluate(
        input: Value,
    ) -> std::result::Result<ProposedStateOutcome<Value, Never>, StateExecutionError> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
type Cause = Incident<Never, Never, NoContext>;
struct Child<'a> {
    capture: Option<&'a Checkpoint<Value>>,
}
impl Operation for Child<'_> {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        if let Some(token) = self.capture {
            let mut family = Handlers::new();
            family.bind::<Cause, Stop>(NoParams)?;
            family.checkpoint::<Cause, Value>(token)?;
            scope.handlers(family)?;
        }
        scope.pure::<Pass, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(1024)?,
        )
    }
}
struct Parent {
    capture: bool,
    retries: u32,
}
impl Operation for Parent {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        scope.pure::<Pass, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(1024)?,
        )?;
        let token = scope.checkpoint::<Value>()?;
        let mut family = Handlers::new();
        family.bind::<Cause, Stop>(NoParams)?;
        family.checkpoint::<Cause, Value>(&token)?;
        scope.handlers(family)?;
        scope.allowances(RecoveryAllowances::new(self.retries, 1))?;
        scope.operation::<Child<'_>, Identity<Never>>(
            &Child {
                capture: self.capture.then_some(&token),
            },
            NoParams,
        )
    }
}
#[test]
fn emitted_linear_program_binds_recovery_and_relocates_inherited_checkpoints() {
    let entry = EntryPointId::new("mfm.test/linear@1").unwrap();
    let program = expand_program(
        entry.clone(),
        &Parent {
            capture: false,
            retries: 2,
        },
        &Value { number: 0 },
        ProgramLimits::new(2),
    )
    .unwrap();
    let target = program.declarations()[1].recovery_targets()[0];
    assert_eq!(target.position().index(), 1);
    assert_eq!(
        program.declarations()[1].allowances(),
        RecoveryAllowances::new(2, 1)
    );
    assert_eq!(
        program.declarations()[1].classifier().abi().source(),
        IncidentAbi::of::<Cause>().unwrap()
    );
    let cold = Program::decode_canonical(program.canonical_bytes()).unwrap();
    assert_eq!(cold, program);
    let changed = expand_program(
        entry.clone(),
        &Parent {
            capture: false,
            retries: 0,
        },
        &Value { number: 0 },
        ProgramLimits::new(2),
    )
    .unwrap();
    assert_ne!(changed.content_ref(), program.content_ref());
    assert!(matches!(
        expand_program(
            entry,
            &Parent {
                capture: true,
                retries: 2
            },
            &Value { number: 0 },
            ProgramLimits::new(2)
        ),
        Err(ProgramError::InvalidContract)
    ));
    let cost = program
        .history_bound(ConclusionBound::new(100).unwrap())
        .unwrap();
    assert_eq!(cost.frames(), 7);
    assert_eq!(cost.bytes(), 6244);
    let mut tampered: serde_json::Value =
        serde_json::from_slice(program.canonical_bytes()).unwrap();
    tampered["declarations"][1]["recovery_targets"] = serde_json::json!([2]);
    let bytes = PlainCanonicalJsonBytes::from_json_str(&tampered.to_string()).unwrap();
    assert!(Program::decode_canonical(bytes.as_bytes()).is_err());
    // Selected ABI parameters must match their retained value, and removed fields are rejected.
    for (field, replacement) in [
        ("canonical", serde_json::json!({"unexpected": true})),
        ("value", serde_json::json!(program.initial_value_ref())),
        (
            "contract",
            serde_json::json!(program.admitted_context_contract_ref()),
        ),
    ] {
        let mut wire: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).unwrap();
        wire["declarations"][0]["classifier"]["params"][field] = replacement;
        let bytes = PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).unwrap();
        assert!(Program::decode_canonical(bytes.as_bytes()).is_err());
    }
    let mut wire: serde_json::Value = serde_json::from_slice(program.canonical_bytes()).unwrap();
    wire["declarations"][0]["classifier"]["abi"]["params"] =
        serde_json::json!(program.admitted_context_contract_ref());
    let bytes = PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).unwrap();
    assert!(Program::decode_canonical(bytes.as_bytes()).is_err());
    tampered = serde_json::from_slice(program.canonical_bytes()).unwrap();
    tampered["domain"] = serde_json::json!("mfm.program.v4");
    let bytes = PlainCanonicalJsonBytes::from_json_str(&tampered.to_string()).unwrap();
    assert!(Program::decode_canonical(bytes.as_bytes()).is_err());
}

struct Effect;
impl EffectCapabilityContract for Effect {
    type Command = Value;
    type Evidence = Value;
    type OperationalError = NoContext;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.effect@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(_: &mfm_ids::EffectId, _: &Value, _: &Value) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}
impl EffectState<Effect> for Pass {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &Value,
        _: &Value,
        _: &NoContext,
    ) -> std::result::Result<NoContext, StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(input: &Value) -> std::result::Result<Value, PreparationError> {
        Ok(Value {
            number: input.number,
        })
    }
    fn interpret(
        input: Value,
        _: &Value,
    ) -> std::result::Result<ProposedStateOutcome<Value, Never>, StateExecutionError> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl CapabilityInjection<Pass> for Effect {
    type Setup = ();
    type ExpandedInput = Value;
    type ExpandedOutput = Value;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &()) -> Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(_: &()) -> Result<ContentRef> {
        nominal_contract_ref::<Value>()
    }
    fn write_before(_: &(), scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        scope.pure::<Pass, Identity<Never>>(NoParams, Occurrence::new(), ConclusionBound::new(100)?)
    }
    fn write_after(_: &(), scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        let token = scope.checkpoint::<Value>()?;
        let mut family = Handlers::new();
        family.bind::<Cause, Stop>(NoParams)?;
        family.checkpoint::<Cause, Value>(&token)?;
        scope.handlers(family)?;
        scope.pure::<Pass, Identity<Never>>(NoParams, Occurrence::new(), ConclusionBound::new(100)?)
    }
}
struct Inject;
impl Operation for Inject {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
        scope.pure::<Pass, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(100)?,
        )?;
        scope.effect::<Pass, Effect, Identity<Never>>(
            &(),
            NoParams,
            Occurrence::new(),
            EffectBounds::new(200, 300)?,
        )
    }
}
#[test]
fn injection_lowers_the_designated_effect_and_the_post_effect_checkpoint() {
    let program = expand_program(
        EntryPointId::new("mfm.test/injected@1").unwrap(),
        &Inject,
        &Value { number: 0 },
        ProgramLimits::new(1),
    )
    .unwrap();
    assert!(matches!(
        program.declarations()[2].execution(),
        Execution::Effect { .. }
    ));
    assert_eq!(
        program.declarations()[3].recovery_targets()[0]
            .position()
            .index(),
        3
    );
    assert_eq!(
        Program::decode_canonical(program.canonical_bytes()).unwrap(),
        program
    );
    let cost = program
        .history_bound(ConclusionBound::new(100).unwrap())
        .unwrap();
    assert_eq!(cost.frames(), 11);
    assert_eq!(cost.bytes(), 1700);
}

#[test]
fn capability_identity_distinguishes_operational_contracts_and_modes() {
    struct Observation<E>(std::marker::PhantomData<E>);
    impl<E: MfmValue> ReadCapabilityContract for Observation<E> {
        type Intent = Value;
        type Evidence = Value;
        type OperationalError = E;
        fn contract_id() -> mfm_capabilities::Result<StableId> {
            Effect::contract_id()
        }
        fn bind_evidence(_: &ContentRef, _: &Value, _: &Value) -> mfm_capabilities::Result<()> {
            Ok(())
        }
    }
    let first = capability_contract_ref::<Observation<NoContext>>().unwrap();
    let changed_error = capability_contract_ref::<Observation<Value>>().unwrap();
    let effect = effect_capability_contract_ref::<Effect>().unwrap();
    assert_ne!(first, changed_error);
    assert_ne!(first, effect);
}

#[test]
fn root_planning_checks_input_agreement_and_commits_exact_initial_value() {
    struct Planned {
        expected: u64,
    }
    impl Operation for Planned {
        type Input = Value;
        type Output = Value;
        type Failure = Never;
        fn validate_input(&self, input: &Value) -> Result<()> {
            if input.number == self.expected {
                Ok(())
            } else {
                Err(ProgramError::InvalidContract)
            }
        }
        fn expand(&self, scope: &mut OperationExpansion<Value, Value, Never>) -> Result<()> {
            scope.pure::<Pass, Identity<Never>>(
                NoParams,
                Occurrence::new(),
                ConclusionBound::new(1024)?,
            )
        }
    }
    let entry = EntryPointId::new("mfm.test/planned@1").unwrap();
    assert!(matches!(
        expand_program(
            entry.clone(),
            &Planned { expected: 7 },
            &Value { number: 8 },
            ProgramLimits::new(0)
        ),
        Err(ProgramError::InvalidContract)
    ));
    let first = expand_program(
        entry.clone(),
        &Planned { expected: 7 },
        &Value { number: 7 },
        ProgramLimits::new(0),
    )
    .unwrap();
    let second = expand_program(
        entry,
        &Planned { expected: 8 },
        &Value { number: 8 },
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_ne!(first.content_ref(), second.content_ref());
    let (_, reference) = mfm_values::canonicalize_mfm_value(&Value { number: 7 }).unwrap();
    assert_eq!(first.initial_value_ref(), &reference);
    assert_eq!(
        Program::decode_canonical(first.canonical_bytes()).unwrap(),
        first
    );
}

#[path = "tests/depth.rs"]
mod depth;
