use mfm_ids::StableId;
use mfm_program::{
    components, ComponentKind, Operation, OperationDefinition, ProgramEnvironment,
    ProposedStateOutcome, Pure, PureState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
struct Value {}
struct Leaf;
impl State for Leaf {
    type Input = Value;
    type Output = Value;
    type Failure = mfm_program::Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("test.inspection.leaf@1")?)
    }
    fn description() -> &'static str {
        "An installed deterministic leaf."
    }
}
impl PureState for Leaf {
    fn evaluate(
        input: Value,
    ) -> Result<ProposedStateOutcome<Value, Self::Failure>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
struct Named;
impl OperationDefinition for Named {
    type Body = Vec<(Pure<Leaf>, Pure<Leaf>)>;
    fn metadata() -> Option<(&'static str, &'static str)> {
        Some((
            "test.inspection.operation@1",
            "An installed operation with no planner.",
        ))
    }
}
struct Installed;
impl ProgramEnvironment for Installed {
    type Sources = (Operation<Named>, Pure<Leaf>, Operation<Named>);
}
#[test]
fn type_only_discovery_inspects_and_deduplicates_without_plan_or_resources() {
    let found = components::<Installed>().unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].kind(), ComponentKind::Operation);
    assert_eq!(found[0].id().as_str(), "test.inspection.operation@1");
    assert_eq!(found[1].kind(), ComponentKind::PureState);
    assert_eq!(found[1].description(), "An installed deterministic leaf.");
}
struct Conflicting;
impl OperationDefinition for Conflicting {
    type Body = Pure<Leaf>;
    fn metadata() -> Option<(&'static str, &'static str)> {
        Some(("test.inspection.operation@1", "Conflicting owner metadata."))
    }
}
struct Conflict;
impl ProgramEnvironment for Conflict {
    type Sources = (Operation<Named>, Operation<Conflicting>);
}
#[test]
fn conflicting_metadata_retains_both_claims() {
    let error = components::<Conflict>().unwrap_err();
    let encoded = serde_json::to_value(error).unwrap();
    let text = encoded.to_string();
    assert!(text.contains("conflicting_metadata"), "{encoded}");
    assert!(
        text.contains("An installed operation with no planner."),
        "{encoded}"
    );
    assert!(text.contains("Conflicting owner metadata."), "{encoded}");
}
