# Accumulating contexts: implementation and verification

The current contracts live in [design](design.md) and [architecture](architecture.md). This
document maps their accumulating-context requirements to executable evidence and records the
bounded workflow capacity measurements. Git history retains the completed RFC and prototype.

## Requirement owners and evidence

| Requirement | Production boundary and regression evidence |
| --- | --- |
| 1. Production recipes, no preparation bridges | `contract_workflow.rs` composes `EvmTransaction<C, R>` with `CreateAt`, `CallCreatedAt`, and `ObserveAt`; the wallet follow-up uses `CallAt`. The managed Effect e2e executes this sequence. |
| 2. Complete successful report | `FixtureReport` retains both transactions' command, reservation, complete preparation evidence, settlement, typed outcome, exact anchored intent/evidence, request metadata, and decoded value. The live test asserts 42 and the retained commands; deterministic tests compare each stage's EffectId with accepted adapter evidence. |
| 3. Sibling preservation and multiple context shapes | `context_contract.rs` moves non-Clone fields through generated slots. `generic_transaction_runtime.rs` executes and cold-reloads distinct context shapes. The fixture and two-creation scenario retain unrelated request metadata through all stages. |
| 4. Typed requirements and semantic identity | Derive UI tests and the `TransactionRecipe` compile-fail rustdoc reject wrong layouts/stages. Generic Runtime tests distinguish same-typed source choices and custom recipe identities, and reject incompatible assembly before Store or adapter IO. Existing Program/Runtime tests retain exact typed-boundary and ABI checks. |
| 5. Checked plans | `transaction_contract.rs` and `anchored_call_contract.rs` test byte maxima, fee ceiling/order, zero-gas decoding, strict deserialization, and total late-target/anchor construction. Plan and complete-command factories share validation owners. |
| 6. Complete facts and hostile inputs | Private cumulative records preserve checked reservation, preparation, settlement, and projection relationships. Domain tests reject mismatched command references, nonce domains, settlement nonce/hash/action, projected addresses/targets, and anchored intent references/anchors. |
| 7. Recovery boundaries | Live unit tests retain cancellation, rejecting-signer prepared-wire reuse, custody acknowledgement loss, and ambiguous appends at every transaction Journal boundary. The managed Effect e2e retains the original external nonce advancement and reconstructed Runtime claims. |
| 8. Exact failure facts and cold terminals | Deterministic fixture tests cover both reversions, all three observation failure reasons, and invalid ABI bytes; they compare unchanged cold head/value, retained frames, and adapter counts. Distinct three-step and four-step root reports retain plans once with an executed evidence prefix, check continuation/outcome and target/observation linkage, and derive the failure reason. The managed test compares complete terminal reports and nonce `0 -> 2 -> 4`. |
| 9. Capacity | `context_capacity.rs` executes two creations, a call selecting the second creation, anchored observation, and ABI reporting. It covers all measured input sizes, every failure point at maximum inputs, and maximum returned evidence. It checks exact schema identity lengths, terminal bytes, each repeated frame object closure, frame maxima, total run bytes, and cold terminal equality under unchanged limits. |
| 10. Internal execution errors | `runtime/tests/support/callback_errors.rs` proves Pure/Read head preservation, Effect prepare preservation, exact retry command/EffectId, and no completed cold interpretation. The EVM tests reject an incompatible success mode before prepare append or adapter entry. |
| 11. Complete deletion | Current EVM domain/live source and current contract documentation contain no superseded caller-context wrappers, fixture preparation bridges, old State-ID constants, or local one-transaction wrappers. Superseded RFC, prototype, and pre-cutover composition descriptions are deleted; remaining production-reuse gaps are tracked separately in [the current problem inventory](../PROBLEM_LEAK_PROD_IMPLS.md). |

The test paths above are:

- [slot consuming tests](../crates/kernel/program-derive/tests/context_contract.rs)
- [transaction domain tests](../crates/domains/evm/tests/transaction_contract.rs)
- [anchored domain tests](../crates/domains/evm/tests/anchored_call_contract.rs)
- [generic Runtime tests](../crates/live/evm/tests/generic_transaction_runtime.rs)
- [live recovery tests](../crates/live/evm/src/transaction_tests.rs)
- [managed Effect e2e](../crates/live/evm/tests/evm_contract_effect_e2e.rs)
- [fixture sequence and checked reports](../crates/live/evm/tests/support/contract_workflow.rs)
- [deterministic fixture tests](../crates/live/evm/tests/support/accumulating_contract.rs)
- [capacity tests](../crates/live/evm/tests/support/context_capacity.rs)
- [Runtime callback error tests](../crates/kernel/runtime/tests/support/callback_errors.rs)

## Production capacity measurements

These measurements come from `two_creations_call_observation_and_reports_fit_the_unchanged_capacity_envelope` with the
current shared-parameter and prefix-report types. Schema identity size is an independent admission
constraint, bounded at 65,536 bytes.

| Value | Schema identity bytes |
| --- | ---: |
| Initial two-creation context | 19,285 |
| After first creation | 30,788 |
| After second creation | 42,291 |
| After configuration | 53,786 |
| After observation | 59,846 |
| Successful report | 60,666 |
| Prefix failure report | 53,665 |

Successful runs retain 24 frames. Input sizes are per creation and per call/observation. All
measurements below are bytes; returned success evidence contains the 32-byte ABI word.

| Creation input | Call/observation input | Terminal value | Total frame bytes | Largest frame |
| ---: | ---: | ---: | ---: | ---: |
| 3 | 3 | 7,201 | 315,578 | 202,883 |
| 1,024 | 1,024 | 12,649 | 410,918 | 208,331 |
| 16,384 | 16,384 | 94,569 | 1,844,541 | 290,274 |
| 49,152 | 49,152 | 269,329 | 4,902,841 | 465,034 |
| 49,152 | 131,072 | 487,783 | 8,616,559 | 683,488 |

Failure paths use 49,152-byte creation inputs and 131,072-byte call/observation inputs.

| Failure | Terminal value | Total frame bytes | Largest frame |
| --- | ---: | ---: | ---: |
| First creation reverted | 969,419 | 3,312,479 | 969,619 |
| Second creation reverted | 972,166 | 5,464,205 | 972,367 |
| Configuration reverted | 975,024 | 7,949,787 | 975,225 |
| Observation rejected | 1,151,123 | 8,790,434 | 1,327,895 |
| Observation safe failure | 1,151,135 | 8,790,450 | 1,327,911 |
| Observation integrity blocked | 1,151,150 | 8,790,470 | 1,327,931 |
| Invalid ABI with 131,072 returned bytes | 1,675,520 | 9,315,661 | 1,014,811 |

The prefix representation retains original plans, including the observation plan alongside its
accepted intent. This increases observation-failure payloads while removing independently supplied
reason and outcome fields. All measured schemas, values, frames, and runs remain within the fixed
limits.

These are bounded product scenarios, not a promise of arbitrary workflow length. Full snapshots
are copied into subsequent frames; long chains can grow retained bytes quadratically. Schema/object/frame/run ceilings and SQL baselines remain unchanged. These measurements use
Program v4 and Journal frame v3; terminal failures include the canonical Runtime FailureReport. No implicit
reference/delta representation or old-context reader exists.

## Verification workflow

Use the scope-driven commands in [build and verification](build-and-verification.md), targeted
generic/fixture tests, and the managed `effect-e2e` task. The final candidate requires `nix run .#ci`, including managed DB,
client/Effect e2e, documentation, and capacity tasks. Nixfied's task result and run summary own
that final gate's result; focused results and this document do not substitute for it.

Live EVM uses existing workspace dependencies `mfm-values` and `mfm-program` for checked values
and the executable State bounds of its pure registration helper. Both were already live-EVM test
dependencies; this adds no external package. No signer, custody, provider, SQL, or production CLI/REST transaction policy was
expanded.

## Material uncertainties

none for the specified bounded scenarios. Longer or different products need their own capacity
measurements before being admitted as supported boundaries.
