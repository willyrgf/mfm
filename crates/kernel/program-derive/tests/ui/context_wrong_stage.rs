use mfm_program_derive::{MfmContext, MfmValue};
use mfm_values::{ContextSlot, MfmValue as Value};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
struct Plan { quantity: u64 }
#[derive(Serialize, Deserialize, MfmValue)]
struct Completed { quantity: u64 }
#[derive(Serialize, Deserialize, MfmValue, MfmContext)]
#[context(namespace = "mfm.test.workflow")]
struct Workflow<T> { transaction: T }

fn requires_completed<C: Value, S: ContextSlot<C, Value = Completed>>(_: &C) {}

fn main() {
    let initial = Workflow { transaction: Plan { quantity: 1 } };
    requires_completed::<_, WorkflowTransactionSlot>(&initial);
}
