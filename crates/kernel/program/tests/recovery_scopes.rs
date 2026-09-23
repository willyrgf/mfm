//! Construction owns scoped policy association; these tests do not authorize Runtime recovery.
#[allow(dead_code)]
#[path = "phase_a/contracts.rs"]
mod contracts;
use contracts::{Add, Deployed};
use mfm_program::*;

struct Marker;
impl CheckpointMarker for Marker {
    type Context = Deployed;
}
struct Parent;
impl OperationDefaults for Parent {
    type Handler = Stop;
    type Targets = (Marker,);
}
impl ResolveDefaults<Deployed> for Parent {
    fn resolve(_: &Deployed) -> Result<PolicyValues<Stop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(1),
        })
    }
}
struct Clear;
impl OperationDefaults for Clear {
    type Handler = Stop;
    type Targets = ();
}
impl ResolveDefaults<Deployed> for Clear {
    fn resolve(_: &Deployed) -> Result<PolicyValues<Stop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(0),
        })
    }
}
struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = (
        Operation<(Checkpoint<Marker>, Pure<Add>), Parent>,
        Operation<(Pure<Add>,), Configured>,
    );
}

#[test]
fn child_policy_replaces_targets_and_inherited_parent_targets_do_not_rebind() {
    let program = compile(
        mfm_ids::EntryPointId::new("mfm.proof.recovery/scope@1").unwrap(),
        &Operation::<
            (
                Checkpoint<Marker>,
                Pure<Add>,
                Operation<(Checkpoint<Marker>, Pure<Add>)>,
                Operation<(Pure<Add>,), Clear>,
                Pure<Add>,
            ),
            Parent,
        >::default(),
        &Deployed { value: 7 },
        &Resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    // The child's marker is at position1. Inherited targets remain bound to parent position0,
    // explicit Clear removes the unit, and leaving that child restores the parent unit.
    let targets: Vec<Vec<usize>> = program
        .declarations()
        .iter()
        .map(|declaration| {
            declaration
                .recovery_targets()
                .iter()
                .map(|target| target.position().index())
                .collect()
        })
        .collect();
    assert_eq!(targets, [vec![0], vec![0], vec![], vec![0]]);
    let cold = load(program.canonical_bytes(), &Resources).unwrap();
    assert_eq!(cold.content_ref(), program.content_ref());
    assert!(cold
        .executable(mfm_ids::StatePosition::new(0).unwrap())
        .unwrap()
        .is_checkpoint());
    assert!(!cold
        .executable(mfm_ids::StatePosition::new(1).unwrap())
        .unwrap()
        .is_checkpoint());
}

#[test]
fn newly_installed_targets_reject_foreign_and_forward_markers() {
    let foreign = compile(
        mfm_ids::EntryPointId::new("mfm.proof.recovery/foreign@1").unwrap(),
        &Operation::<(Checkpoint<Marker>, Operation<(Pure<Add>,), Parent>)>::default(),
        &Deployed { value: 7 },
        &Resources,
        ProgramLimits::new(1),
    );
    let Err(ProgramError::Diagnostic(cause)) = foreign else {
        panic!("checkpoint cause")
    };
    assert_eq!(cause.operation(), "lower_checkpoint");
    assert_eq!(cause.details().as_value()["reason"], "marker_outside_scope");
    assert_eq!(cause.details().as_value()["position"], 0);
    let forward = compile(
        mfm_ids::EntryPointId::new("mfm.proof.recovery/forward@1").unwrap(),
        &Operation::<(Pure<Add>, Checkpoint<Marker>, Pure<Add>), Parent>::default(),
        &Deployed { value: 7 },
        &Resources,
        ProgramLimits::new(1),
    );
    let Err(ProgramError::Diagnostic(cause)) = forward else {
        panic!("checkpoint cause")
    };
    assert_eq!(cause.operation(), "lower_checkpoint");
    assert_eq!(cause.details().as_value()["reason"], "forward_target");
    assert_eq!(cause.details().as_value()["position"], 0);
    assert_eq!(cause.details().as_value()["target_position"], 1);
}

struct ConfiguredHandler;
impl Handler for ConfiguredHandler {
    type Params = Deployed;
    fn implementation_id() -> Result<mfm_ids::StableId> {
        Ok(mfm_ids::StableId::new("mfm.test.recovery/configured@1")?)
    }
    fn handle(
        params: &Deployed,
        _: Classification,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, mfm_values::InvocationDiagnostic> {
        Ok(if params.value == 10 {
            RecoveryRequest::RetryState
        } else {
            RecoveryRequest::Stop
        })
    }
}
struct Configured;
impl OperationDefaults for Configured {
    type Handler = ConfiguredHandler;
    type Targets = ();
}
impl ResolveDefaults<u64> for Configured {
    fn resolve(params: &u64) -> Result<PolicyValues<ConfiguredHandler>> {
        Ok(PolicyValues {
            handler: Some(Deployed { value: *params }),
            retries: Some(2),
            restarts: Some(0),
        })
    }
}
struct ConfiguredOperation {
    params: u64,
}
impl OperationDefinition for ConfiguredOperation {
    type Body = Pure<Add>;
}
impl Plan<Deployed> for ConfiguredOperation {
    type Config = u64;
    fn plan<'a>(&'a self, _: &'a Deployed) -> Result<(&'a u64, Self::Body)> {
        Ok((&self.params, Default::default()))
    }
}

#[test]
fn explicit_nondefault_definition_keeps_maintained_defaults_and_cold_handler_identity() {
    let context = RecoveryContext::new(
        ExecutionPhase::Read,
        RecoveryAllowances::new(2, 0),
        2,
        &[],
        &[],
    );
    let mut previous = None;
    for (params, expected) in [
        (10, RecoveryRequest::RetryState),
        (11, RecoveryRequest::Stop),
    ] {
        let program = compile(
            mfm_ids::EntryPointId::new("mfm.proof.recovery/parameters@1").unwrap(),
            &Operation::<_, Configured>::from(ConfiguredOperation { params }),
            &Deployed { value: 7 },
            &Resources,
            ProgramLimits::new(2),
        )
        .unwrap();
        assert_ne!(previous.as_ref(), Some(program.content_ref()));
        previous = Some(program.content_ref().clone());
        let cold = load(program.canonical_bytes(), &Resources).unwrap();
        assert_eq!(cold.content_ref(), program.content_ref());
        assert_eq!(
            cold.executable(mfm_ids::StatePosition::new(0).unwrap())
                .unwrap()
                .request(Classification::Retryable, &context)
                .unwrap(),
            expected
        );
        let mut wire: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).unwrap();
        wire["declarations"][0]["handler"]["params"] =
            serde_json::to_value(PolicyParams::new(&NoParams).unwrap()).unwrap();
        let malformed = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&wire).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            load(malformed.as_bytes(), &Resources),
            Err(ProgramError::InvalidContract)
        ));
    }
}

struct StateOnly;
impl ProgramEnvironment for StateOnly {
    type Sources = Pure<Add>;
}
struct Unpublished<const SELECT: bool>;
impl<const SELECT: bool> OperationDefaults for Unpublished<SELECT> {
    type Handler = ConfiguredHandler;
    type Targets = ();
}
impl<const SELECT: bool> ResolveDefaults<Deployed> for Unpublished<SELECT> {
    fn resolve(_: &Deployed) -> Result<PolicyValues<ConfiguredHandler>> {
        Ok(PolicyValues {
            handler: SELECT.then_some(Deployed { value: 10 }),
            retries: None,
            restarts: None,
        })
    }
}

#[test]
fn unused_handlers_need_no_installation_and_preserve_effective_stop_on_cold_load() {
    let entry = mfm_ids::EntryPointId::new("mfm.proof.recovery/unused@1").unwrap();
    let input = Deployed { value: 7 };
    let expected = compile(
        entry.clone(),
        &Pure::<Add>::default(),
        &input,
        &StateOnly,
        ProgramLimits::new(0),
    )
    .unwrap();
    for (case, result) in [
        (
            "absent handler inherits Stop",
            compile(
                entry.clone(),
                &Operation::<(Pure<Add>,), Unpublished<false>>::default(),
                &input,
                &StateOnly,
                ProgramLimits::new(0),
            ),
        ),
        (
            "empty scope does not use its handler",
            compile(
                entry.clone(),
                &(
                    Operation::<(Identity<Deployed>,), Unpublished<true>>::default(),
                    Pure::<Add>::default(),
                ),
                &input,
                &StateOnly,
                ProgramLimits::new(0),
            ),
        ),
        (
            "child Stop overrides unused parent handler",
            compile(
                entry,
                &Operation::<(Operation<(Pure<Add>,), Clear>,), Unpublished<true>>::default(),
                &input,
                &StateOnly,
                ProgramLimits::new(0),
            ),
        ),
    ] {
        let program = result.unwrap_or_else(|cause| panic!("{case}: {cause:?}"));
        assert_eq!(
            program.canonical_bytes(),
            expected.canonical_bytes(),
            "{case}"
        );
        let cold = load(program.canonical_bytes(), &StateOnly).unwrap();
        assert_eq!(cold.content_ref(), expected.content_ref(), "{case}");
    }
}

#[test]
fn unpublished_handler_used_by_a_state_rejects_with_selection_cause() {
    let Err(ProgramError::Diagnostic(cause)) = compile(
        mfm_ids::EntryPointId::new("mfm.proof.recovery/unpublished@1").unwrap(),
        &Operation::<(Pure<Add>,), Unpublished<true>>::default(),
        &Deployed { value: 7 },
        &StateOnly,
        ProgramLimits::new(0),
    ) else {
        panic!("unpublished handler accepted")
    };
    assert_eq!(cause.operation(), "select_handler");
    assert_eq!(
        cause.details().as_value()["reason"],
        "handler_not_installed"
    );
    assert_eq!(
        cause.details().as_value()["handler"],
        serde_json::to_value(HandlerAbi::of::<ConfiguredHandler>().unwrap()).unwrap()
    );
}
