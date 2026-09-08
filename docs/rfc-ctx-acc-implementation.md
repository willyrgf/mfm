# Accumulating contexts: implementation and verification

[RFC_CTX_ACC.md](../RFC_CTX_ACC.md) defines the complete cutover. The
[prototype handoff](rfc-ctx-acc-validation.md) records earlier feasibility evidence; production
contracts live in [design](design.md) and [architecture](architecture.md).

## Requirement owners and evidence

| RFC acceptance | Production boundary and regression evidence |
| --- | --- |
| 1. Production recipes, no preparation bridges | `contract_workflow.rs` composes `EvmTransaction<C, R>` with `CreateAt`, `CallCreatedAt`, and `ObserveAt`; the wallet follow-up uses `CallAt`. The managed Effect e2e executes this graph. |
| 2. Complete successful report | `FixtureReport` retains both transactions' command, reservation, complete preparation evidence, settlement, typed outcome, exact anchored intent/evidence, request metadata, and decoded value. The live test asserts 42 and the retained commands; deterministic tests compare each stage's EffectId with accepted adapter evidence. |
| 3. Sibling preservation and multiple context shapes | `context_contract.rs` moves non-Clone fields through generated slots. `generic_transaction_runtime.rs` executes and cold-reloads distinct context shapes. The fixture and two-creation scenario retain unrelated request metadata through all stages. |
| 4. Typed requirements and semantic identity | Derive UI tests and the `TransactionRecipe` compile-fail rustdoc reject wrong layouts/stages. Generic Runtime tests distinguish same-typed source choices and custom recipe identities, and reject incompatible assembly before Store or adapter IO. Existing Program/Runtime tests retain exact-edge and ABI checks. |
| 5. Checked plans | `transaction_contract.rs` and `anchored_call_contract.rs` test byte maxima, fee ceiling/order, zero-gas decoding, strict deserialization, and total late-target/anchor construction. Plan and complete-command factories share validation owners. |
| 6. Complete facts and hostile inputs | Private cumulative records preserve checked reservation, preparation, settlement, and projection relationships. Domain tests reject mismatched command references, nonce domains, settlement nonce/hash/action, projected addresses/targets, and anchored intent references/anchors. |
| 7. Recovery boundaries | Live unit tests retain cancellation, rejecting-signer prepared-wire reuse, custody acknowledgement loss, and ambiguous appends at every transaction Journal boundary. The managed Effect e2e retains the original external nonce advancement and reconstructed Runtime claims. |
| 8. Exact failure facts and cold terminals | Deterministic fixture tests cover both reversions, all three observation failure reasons, and invalid ABI bytes; they compare unchanged cold head/value, retained frames, and adapter counts. Distinct three-step and four-step root reports retain plans once with an executed evidence prefix, check continuation/outcome and target/observation linkage, and derive the failure reason. The managed test compares complete terminal reports and nonce `0 -> 2 -> 4`. |
| 9. Capacity | `context_capacity.rs` executes two creations, a call selecting the second creation, anchored observation, and ABI reporting. It covers all measured input sizes, every failure point at maximum inputs, and maximum returned evidence. It checks exact schema identity lengths, terminal bytes, each repeated frame object closure, frame maxima, total run bytes, and cold terminal equality under unchanged limits. |
| 10. Internal execution errors | `runtime/tests/support/callback_errors.rs` proves Pure/Read head preservation, Effect prepare preservation, exact retry command/EffectId, and no completed cold interpretation. The EVM tests reject an incompatible success mode before prepare append or adapter entry. |
| 11. Complete deletion | Current EVM domain/live source and current contract documentation contain no superseded caller-context wrappers, fixture preparation bridges, old State-ID constants, or local one-transaction wrappers. Original RFC/problem descriptions remain historical specification material. |

The test paths above are:

- [slot consuming tests](../crates/kernel/program-derive/tests/context_contract.rs)
- [transaction domain tests](../crates/domains/evm/tests/transaction_contract.rs)
- [anchored domain tests](../crates/domains/evm/tests/anchored_call_contract.rs)
- [generic Runtime tests](../crates/live/evm/tests/generic_transaction_runtime.rs)
- [live recovery tests](../crates/live/evm/src/transaction_tests.rs)
- [managed Effect e2e](../crates/live/evm/tests/evm_contract_effect_e2e.rs)
- [fixture graph and checked reports](../crates/live/evm/tests/support/contract_workflow.rs)
- [deterministic fixture tests](../crates/live/evm/tests/support/accumulating_contract.rs)
- [capacity tests](../crates/live/evm/tests/support/context_capacity.rs)
- [Runtime callback error tests](../crates/kernel/runtime/tests/support/callback_errors.rs)

## Production capacity measurements

Schema identities below describe the current shared-parameter and prefix-report types. Payload measurements below are from the pre-reduction cutover and will be refreshed by its capacity acceptance run. Schema identities are
an independent admission constraint, bounded at 65,536 bytes.

| Value | Schema identity bytes |
| --- | ---: |
| Initial two-creation context | 19,285 |
| After first creation | 30,788 |
| After second creation | 42,291 |
| After configuration | 53,786 |
| After observation | 59,846 |
| Successful report | 60,666 |
| Normalized failure report | 53,665 |

Successful runs retain 24 frames. Input sizes are per creation and per call/observation. All
measurements below are bytes; returned success evidence contains the 32-byte ABI word.

| Creation input | Call/observation input | Terminal value | Total frame bytes | Largest frame |
| ---: | ---: | ---: | ---: | ---: |
| 3 | 3 | 7,201 | 147,147 | 34,828 |
| 1,024 | 1,024 | 12,649 | 242,487 | 40,276 |
| 16,384 | 16,384 | 94,569 | 1,676,087 | 122,196 |
| 49,152 | 49,152 | 269,329 | 4,734,387 | 337,630 |
| 49,152 | 131,072 | 487,783 | 8,448,105 | 665,311 |

Failure paths use 49,152-byte creation inputs and 131,072-byte call/observation inputs.

| Failure | Terminal value | Total frame bytes | Largest frame |
| --- | ---: | ---: | ---: |
| First creation reverted | 484,437 | 3,144,933 | 515,410 |
| Second creation reverted | 485,902 | 5,296,728 | 515,410 |
| Configuration reverted | 487,426 | 7,782,372 | 515,410 |
| Observation rejected | 487,976 | 8,448,009 | 665,022 |
| Observation safe failure | 487,984 | 8,448,029 | 665,034 |
| Observation integrity blocked | 487,994 | 8,448,054 | 665,049 |
| Invalid ABI with 131,072 returned bytes | 662,843 | 8,972,605 | 1,014,751 |

These are bounded product scenarios, not a promise of arbitrary workflow length. Full snapshots
are copied into subsequent frames; long chains can grow retained bytes quadratically. Neither
schema/object/frame/run limits nor the Journal wire or SQL baselines changed. No implicit
reference/delta representation or old-context reader exists.

## Verification workflow

The cutover uses the focused commands in RFC section 11, targeted generic/fixture tests, and the
managed `effect-e2e` task. The final candidate requires `nix run .#ci`, including managed DB,
client/Effect e2e, documentation, and capacity tasks. Nixfied's task result and run summary own
that final gate's result; focused results and this document do not substitute for it.

The newly normal dependency from live EVM to `mfm-values` supplies `MfmValue`/`ContextSlot` bounds
for the required pure registration helper. It was already a live-EVM test dependency and adds no
external package. No signer, custody, provider, SQL, or production CLI/REST transaction policy was
expanded.

## Material uncertainties

none for the specified bounded scenarios. Longer or different products need their own capacity
measurements before being admitted as supported boundaries.
