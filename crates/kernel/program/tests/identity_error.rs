use mfm_ids::{CheckedStringError, CheckedStringErrorReason, StableId};
use mfm_program::{state_implementation_ref, Never, ProgramError, State};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Value {}
struct InvalidIdentity;
impl State for InvalidIdentity {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("")?)
    }
}

#[test]
fn state_identity_derivation_retains_the_checked_grammar_cause() {
    let error = state_implementation_ref::<InvalidIdentity>().unwrap_err();
    let ProgramError::Identity(cause) = &error else {
        panic!("checked cause was replaced")
    };
    assert_eq!(cause.reason(), &CheckedStringErrorReason::Empty);
    assert_eq!(
        std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<CheckedStringError>(),
        Some(cause)
    );
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["identity"]["reason"], "empty");
    assert_eq!(wire["identity"]["grammar"], cause.grammar());
}

impl mfm_capabilities::ReadCapabilityContract for InvalidIdentity {
    type Intent = Value;
    type Evidence = Value;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("")?)
    }
    fn bind_evidence(
        _: &mfm_ids::ContentRef,
        _: &Value,
        _: &mfm_ids::ContentRef,
        _: &Value,
    ) -> Result<(), mfm_values::InvocationDiagnostic> {
        Ok(())
    }
}

#[test]
fn capability_identity_keeps_both_boundary_layers_and_the_grammar_rejection() {
    let error = mfm_program::capability_contract_ref::<InvalidIdentity>().unwrap_err();
    let source = std::error::Error::source(&error).unwrap();
    assert!(source
        .downcast_ref::<mfm_capabilities::CapabilityError>()
        .is_some());
    let cause = source
        .source()
        .unwrap()
        .downcast_ref::<CheckedStringError>()
        .unwrap();
    assert_eq!(cause.reason(), &CheckedStringErrorReason::Empty);
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["capability"]["identity"]["reason"], "empty");
    assert_eq!(wire["capability"]["identity"]["grammar"], cause.grammar());
}
