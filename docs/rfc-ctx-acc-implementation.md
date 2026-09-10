# Accumulating contexts: implementation and verification

The current contracts live in [design](design.md) and [architecture](architecture.md). This
document maps their accumulating-context requirements to executable evidence and identifies the retained regression coverage. Git history retains the completed RFC and prototype.

## Requirement owners and evidence

| Requirement | Production boundary and regression evidence |
| --- | --- |
| 1. Production recipes, no preparation bridges | `contract_workflow.rs` composes `EvmTransaction<C, R>` with `CreateAt`, `CallCreatedAt`, and `ObserveAt`; the wallet follow-up uses `CallAt`. The managed Effect e2e executes this sequence. |
| 2. Complete successful report | `FixtureReport` retains both transactions' command, reservation, complete preparation evidence, settlement, typed outcome, exact anchored intent/evidence, request metadata, and decoded value. The live test asserts 42 and the retained commands. |
| 3. Sibling preservation and multiple context shapes | `context_contract.rs` moves non-Clone fields through generated slots. `generic_transaction_runtime.rs` executes and cold-reloads distinct context shapes. The managed fixture retains unrelated request metadata through its stages. |
| 4. Typed requirements and semantic identity | Derive UI tests and the `TransactionRecipe` compile-fail rustdoc reject wrong layouts/stages. Generic Runtime tests distinguish same-typed source choices and custom recipe identities, and reject incompatible assembly before Store or adapter IO. Existing Program/Runtime tests retain exact typed-boundary and ABI checks. |
| 5. Checked plans | `transaction_contract.rs` and `anchored_call_contract.rs` test byte maxima, fee ceiling/order, zero-gas decoding, strict deserialization, and total late-target/anchor construction. Plan and complete-command factories share validation owners. |
| 6. Complete facts and hostile inputs | Private cumulative records preserve checked reservation, preparation, settlement, and projection relationships. Domain tests reject mismatched command references, nonce domains, settlement nonce/hash/action, projected addresses/targets, and anchored intent references/anchors. |
| 7. Recovery boundaries | Live unit tests retain cancellation, rejecting-signer prepared-wire reuse, custody acknowledgement loss, and ambiguous appends at every transaction Journal boundary. The managed Effect e2e retains the original external nonce advancement and reconstructed Runtime claims. |
| 8. Failure facts and cold terminals | Domain transaction and anchored-call tests validate retained failure facts. Generic Runtime and pending-failure tests exercise cold reconstruction. The managed test compares complete successful terminal reports and nonce `0 -> 2 -> 4`; it does not execute every composed fixture failure branch. |
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
- [Runtime callback error tests](../crates/kernel/runtime/tests/support/callback_errors.rs)

The former maximum-payload workflow matrix and duplicate deterministic fixture scenarios were
removed after reviewing their iteration cost and overlap with domain, generic Runtime and managed
integration tests. Their historical measurements are available in Git; they are not current
capacity acceptance evidence. Production schema, object, frame and run limits remain unchanged.

## Verification workflow

Use the scope-driven commands in [build and verification](build-and-verification.md), targeted
generic Runtime tests, and the managed `effect-e2e` task when that boundary changes. Changes to
production contracts require the final composed CI gate selected by the build guide; test-only
cleanup uses focused verification. Nixfied's task result and run summary own a composed gate's
result; focused results and this document do not substitute for it.

Live EVM uses existing workspace dependencies `mfm-values` and `mfm-program` for checked values
and the executable State bounds of its pure registration helper. Both were already live-EVM test
dependencies; this adds no external package. No signer, custody, provider, SQL, or production CLI/REST transaction policy was
expanded.

## Material uncertainties

Maximum-size composed workflows are no longer exercised by a dedicated capacity matrix. Do not
infer their admission from the retained small consuming scenarios; validate a concrete product
capacity requirement when one is established.
