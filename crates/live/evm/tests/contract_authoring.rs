//! Pure selection and downstream State authoring through the maintained component environment.
//! Effect, maintained Operation and mixed Operation/State consumers live in evm_contract_effect_e2e.

use std::sync::Arc;

use mfm_chain::{CheckedAdd, CheckedAddition};
use mfm_evm_live::client::contract::ContractResources;
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    compile, load, Classification, ClassifyError, ProgramLimits, ProposedStateOutcome, Pure,
    PureState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::Runtime;
use mfm_store::MemoryStore;
use mfm_values::{InvocationDiagnostic, Unsigned256};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
struct ZeroSum {}
impl ClassifyError for ZeroSum {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

struct RequireNonZero;
impl State for RequireNonZero {
    type Input = Unsigned256;
    type Output = Unsigned256;
    type Failure = ZeroSum;

    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.example.require-nonzero@1")?)
    }
}
impl PureState for RequireNonZero {
    fn evaluate(
        input: Unsigned256,
    ) -> Result<ProposedStateOutcome<Unsigned256, ZeroSum>, InvocationDiagnostic> {
        Ok(if input.is_zero() {
            ProposedStateOutcome::Failure {
                failure: ZeroSum {},
            }
        } else {
            ProposedStateOutcome::Success { output: input }
        })
    }
}

#[tokio::test]
async fn pure_selection_and_new_semantics_use_the_same_cold_execution_path() {
    let resources: ContractResources = ContractResources::new(vec![], vec![]).unwrap();
    let input = CheckedAddition::new("42", "42").unwrap();
    let state = Pure::<CheckedAdd>::default();
    let program = compile(
        EntryPointId::new("mfm.example/add@1").unwrap(),
        &state,
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let program = load(program.canonical_bytes(), &resources).unwrap();
    let runtime = Runtime::new(Arc::new(MemoryStore::new()));
    let result = runtime
        .execute(
            RunId::from_digest(DigestBytes::from_array([1; 32])),
            &program,
            &input,
        )
        .await
        .unwrap();
    assert_eq!(
        result
            .success()
            .unwrap()
            .decode::<Unsigned256>()
            .unwrap()
            .to_string(),
        "84"
    );

    // Only the genuinely new source is published; no maintained source list is repeated.
    let resources: ContractResources<Pure<RequireNonZero>> =
        ContractResources::new(vec![], vec![]).unwrap();
    let states = (
        Pure::<CheckedAdd>::default(),
        Pure::<RequireNonZero>::default(),
    );
    for (ordinal, operand) in [(2, "42"), (3, "0")] {
        let input = CheckedAddition::new(operand, operand).unwrap();
        let program = compile(
            EntryPointId::new("mfm.example/nonzero-sum@1").unwrap(),
            &states,
            &input,
            &resources,
            ProgramLimits::new(1),
        )
        .unwrap();
        let program = load(program.canonical_bytes(), &resources).unwrap();
        let run = RunId::from_digest(DigestBytes::from_array([ordinal; 32]));
        let result = runtime
            .execute(run.clone(), &program, &input)
            .await
            .unwrap();
        let cold = runtime.read(&run, &program).await.unwrap();
        if operand == "42" {
            assert_eq!(
                cold.success()
                    .unwrap()
                    .decode::<Unsigned256>()
                    .unwrap()
                    .to_string(),
                "84"
            );
        } else {
            let failure = cold.failure().unwrap();
            assert_eq!(
                failure.canonical_bytes(),
                result.failure().unwrap().canonical_bytes()
            );
            assert_eq!(
                failure.failure().original().decode::<ZeroSum>().unwrap(),
                ZeroSum {}
            );
        }
    }
}
