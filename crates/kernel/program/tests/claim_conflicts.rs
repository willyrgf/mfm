//! Installed-code ownership checks through the public construction boundary.
#[allow(dead_code)]
#[path = "phase_a/contracts.rs"]
mod contracts;
use contracts::{Add, Deployed};
use mfm_ids::{EntryPointId, StableId};
use mfm_program as source;
use mfm_program::*;
use mfm_values::InvocationDiagnostic;
use std::marker::PhantomData;

struct ShadowAdd;
impl State for ShadowAdd {
    type Input = Deployed;
    type Output = Deployed;
    type Failure = Never;
    fn state_id() -> source::Result<StableId> {
        Add::state_id()
    }
}
impl PureState for ShadowAdd {
    fn evaluate(
        _: Deployed,
    ) -> std::result::Result<ProposedStateOutcome<Deployed, Never>, InvocationDiagnostic> {
        panic!("construction must not evaluate a State")
    }
}
struct ShadowStop;
impl Handler for ShadowStop {
    type Params = NoParams;
    fn implementation_id() -> source::Result<StableId> {
        Stop::implementation_id()
    }
    fn handle(
        _: &NoParams,
        _: Classification,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, InvocationDiagnostic> {
        panic!("construction must not invoke a recovery handler")
    }
}
struct ShadowDefaults;
impl OperationDefaults for ShadowDefaults {
    type Handler = ShadowStop;
    type Targets = ();
}
impl<C: ?Sized> ResolveDefaults<C> for ShadowDefaults {
    fn resolve(_: &C) -> source::Result<PolicyValues<ShadowStop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: None,
            restarts: None,
        })
    }
}
struct Resources<S>(PhantomData<fn() -> S>);
impl<S> ProgramEnvironment for Resources<S> {
    type Sources = S;
}

#[test]
fn fresh_and_cold_construction_reject_conflicting_state_and_handler_owners() {
    let input = Deployed { value: 7 };
    let installed = Resources::<Pure<Add>>(PhantomData);
    let program = compile(
        EntryPointId::new("mfm.proof/claims@1").unwrap(),
        &Pure::<Add>::default(),
        &input,
        &installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    let Err(ProgramError::Diagnostic(cause)) = compile(
        EntryPointId::new("mfm.proof/claims@1").unwrap(),
        &Pure::<ShadowAdd>::default(),
        &input,
        &installed,
        ProgramLimits::new(0),
    ) else {
        panic!("shadow State accepted");
    };
    assert_eq!(cause.operation(), "select_state");
    assert_eq!(cause.details().as_value()["reason"], "conflicting_owner");
    assert!(load(
        program.canonical_bytes(),
        &Resources::<(Pure<Add>, Pure<ShadowAdd>)>(PhantomData)
    )
    .is_err());
    type ShadowPolicy = Operation<(Pure<Add>,), ShadowDefaults>;
    let Err(ProgramError::Diagnostic(cause)) = compile(
        EntryPointId::new("mfm.proof/claims@1").unwrap(),
        &ShadowPolicy::default(),
        &input,
        &installed,
        ProgramLimits::new(0),
    ) else {
        panic!("shadow handler accepted");
    };
    assert_eq!(cause.operation(), "select_handler");
    assert_eq!(cause.details().as_value()["reason"], "conflicting_owner");
    assert!(load(
        program.canonical_bytes(),
        &Resources::<(Pure<Add>, ShadowPolicy)>(PhantomData)
    )
    .is_err());
}

#[test]
fn repeated_exact_claims_construct_and_cold_load_without_ambiguity() {
    let resources = Resources::<(Pure<Add>, Pure<Add>)>(PhantomData);
    let program = compile(
        EntryPointId::new("mfm.proof/repeated@1").unwrap(),
        &(Pure::<Add>::default(), Pure::<Add>::default()),
        &Deployed { value: 7 },
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let cold = load(program.canonical_bytes(), &resources).unwrap();
    assert_eq!(cold.canonical_bytes(), program.canonical_bytes());
    assert_eq!(cold.content_ref(), program.content_ref());
}

#[test]
fn compilation_requires_installed_semantics_but_recomposition_needs_no_root_publication() {
    let entry = EntryPointId::new("mfm.proof/installed@1").unwrap();
    let input = Deployed { value: 7 };
    let Err(ProgramError::Diagnostic(cause)) = compile(
        entry.clone(),
        &Pure::<Add>::default(),
        &input,
        &Resources::<(Identity<Deployed>, Identity<Never>)>(PhantomData),
        ProgramLimits::new(0),
    ) else {
        panic!("uninstalled State accepted");
    };
    assert_eq!(cause.operation(), "select_state");
    assert_eq!(cause.details().as_value()["reason"], "state_not_installed");

    let Err(ProgramError::Diagnostic(cause)) = compile(
        entry.clone(),
        &Identity::<Deployed>::default(),
        &input,
        &Resources::<()>(PhantomData),
        ProgramLimits::new(0),
    ) else {
        panic!("uninstalled value accepted");
    };
    assert_eq!(cause.details().as_value()["reason"], "value_not_installed");

    let installed = Resources::<Pure<Add>>(PhantomData);
    let grouped = Operation::new((Pure::<Add>::default(), Pure::<Add>::default()));
    let program = compile(entry, &grouped, &input, &installed, ProgramLimits::new(0)).unwrap();
    assert_eq!(
        load(program.canonical_bytes(), &installed)
            .unwrap()
            .content_ref(),
        program.content_ref()
    );
}
