use mfm_program_derive::{MfmContext, MfmValue};
use mfm_values::{canonicalize_mfm_value, ContextSlot, MfmValue as Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, MfmValue)]
struct Plan {
    quantity: u64,
}
#[derive(Debug, PartialEq, Serialize, Deserialize, MfmValue)]
struct Completed {
    quantity: u64,
}

// Neither the context nor its values implement Clone.
#[derive(Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.context.workflow")]
struct Workflow<A, B> {
    first: A,
    second: B,
    unrelated: String,
}

#[derive(Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.context.single")]
struct Single<T> {
    transaction: T,
    unrelated: u64,
}

fn complete<C: Value, S: ContextSlot<C, Value = Plan>>(context: C) -> S::With<Completed> {
    let quantity = S::get(&context).quantity;
    S::replace(context, Completed { quantity })
}

#[test]
fn typed_replacement_preserves_siblings_without_clone_or_shape_specific_forwarding() {
    let initial = Workflow {
        first: Plan { quantity: 11 },
        second: Plan { quantity: 22 },
        unrelated: "retained".to_owned(),
    };
    let first = complete::<_, WorkflowFirstSlot>(initial);
    assert_eq!(WorkflowFirstSlot::get(&first), &Completed { quantity: 11 });
    assert_eq!(WorkflowSecondSlot::get(&first), &Plan { quantity: 22 });
    let final_context = complete::<_, WorkflowSecondSlot>(first);
    assert_eq!(final_context.first, Completed { quantity: 11 });
    assert_eq!(final_context.second, Completed { quantity: 22 });
    assert_eq!(final_context.unrelated, "retained");
    let (bytes, reference) = canonicalize_mfm_value(&final_context).unwrap();
    let reloaded: Workflow<Completed, Completed> =
        serde_json::from_slice(bytes.as_bytes()).unwrap();
    assert_eq!(canonicalize_mfm_value(&reloaded).unwrap().1, reference);
    assert_eq!(reloaded.first.quantity, 11);
    assert_eq!(reloaded.second.quantity, 22);
    assert_eq!(reloaded.unrelated, "retained");

    let other = complete::<_, SingleTransactionSlot>(Single {
        transaction: Plan { quantity: 33 },
        unrelated: 44,
    });
    assert_eq!(other.transaction.quantity, 33);
    assert_eq!(other.unrelated, 44);
    let (bytes, _) = canonicalize_mfm_value(&other).unwrap();
    let reloaded: Single<Completed> = serde_json::from_slice(bytes.as_bytes()).unwrap();
    assert_eq!(reloaded.transaction.quantity, 33);
    assert_eq!(reloaded.unrelated, 44);
}

#[test]
fn slot_identities_name_the_explicit_namespace_and_field_independent_of_stage() {
    let first = <WorkflowFirstSlot as ContextSlot<Workflow<Plan, Plan>>>::slot_id().unwrap();
    assert_eq!(first.as_str(), "mfm.test.context.workflow/slot/first");
    assert_eq!(
        first,
        <WorkflowFirstSlot as ContextSlot<Workflow<Completed, Plan>>>::slot_id().unwrap()
    );
    assert_ne!(
        first,
        <WorkflowSecondSlot as ContextSlot<Workflow<Plan, Plan>>>::slot_id().unwrap()
    );
    assert_ne!(
        first,
        <SingleTransactionSlot as ContextSlot<Single<Plan>>>::slot_id().unwrap()
    );
}

#[test]
fn unsupported_contexts_and_wrong_required_field_types_are_compile_errors() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/context_*.rs");
}
