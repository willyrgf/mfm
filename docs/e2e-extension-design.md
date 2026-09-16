# E2E design: extend MFM with a new State

This is the third public usage acceptance design in the
[public interfaces and tests RFC](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md).
It specifies the desired experience, not interfaces already implemented. Proposed Rust names
and signatures illustrate obligations; implementation must establish one current public API.
[Design](design.md) continues to own execution and persistence contracts.

## Objective and story

A downstream developer adds one deterministic business rule to a workflow built from maintained
components. The rule assesses a proposed increase against an observed contract value and returns
an approval or a typed rejection. The developer supplies semantics and checked contracts, while
MFM supplies composition, association, execution, recovery, and result handling.

Use the same deployment, configuration, and anchored observation components as the composition
E2E, followed by exactly one new Pure State, `AssessIncrease`:

1. Deploy the first-party contract.
2. Configure its value to `42` through an existing transaction Operation.
3. Read `value()` at the configuration transaction's receipt anchor through an existing State.
4. Assess an admitted increase of `8` against an admitted ceiling of `100`.
5. Return the typed approval: observed `42`, proposed `50`, remaining headroom `50`.

A second run with ceiling `49` returns a typed business rejection with proposed value `50`.
The new State performs real policy evaluation. It is not an ABI decoder, context adapter, or
failure-map workaround. Its approval is a business result, not a transaction-authority capability.
It neither promises freshness for a future transaction nor submits another transaction. A future
case can consume the approval downstream if that becomes a required public workflow.

## Public value and State contracts

The caller owns these strict, canonical, secret-free value contracts:

- `IncreasePolicy`: `increment: NonZeroU64` and `ceiling: u64`.
- `Observed`: the caller's named context containing checked deployment/configuration facts,
  anchored observation facts, and the admitted policy. Existing components preserve the policy.
- `ApprovedIncrease`: the checked observation record, policy, observed current value, proposed
  value, and remaining headroom. It retains its declared evidence without an implicit Journal lookup.
- `IncreaseFailure`: `ArithmeticOverflow` or `CeilingExceeded`, each retaining the checked
  observation record, complete policy, and observed current value; the latter also retains the
  computed proposed value.

Constructors and decoding enforce the relationships claimed by these types. For success,
`current + increment == proposed`, `proposed <= ceiling`, and `headroom == ceiling - proposed`.
The observation must decode to the claimed current value. Overflow failures establish that the
addition overflows; ceiling failures establish that the addition succeeds and exceeds the ceiling.
Retained evidence is not a substitute for Runtime's qualified provenance.

Use normal public value-authoring support for schemas, checked decoding, and versioned identities.
Do not introduce a custom macro just for this test. Facts must not recursively nest prior workflow
contexts or duplicate complete transaction histories at every step.

The following is a proposed implementation sketch. Contract declarations above and checked
constructors are part of the author-owned example, not unspecified framework execution code.
The maintained checked ABI accessor shown here is a proposed specialization of the composition
scenario's checked return decoder, not another codec implementation. It validates the declared
getter contract, observed calldata and return shape before performing the checked integer projection. Its parser/range causes
must survive any conversion to `InvocationDiagnostic`; malformed data is not a business rejection.
The sketch also requires source-preserving identity-error conversion into the Program error type;
it must not replace constructor causes with a unit error.

```rust
struct AssessIncrease;

impl State for AssessIncrease {
    type Input = Observed;
    type Output = ApprovedIncrease;
    type Failure = IncreaseFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("example.assess-increase@1")?)
    }
}

impl ClassifyError for IncreaseFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

impl PureState for AssessIncrease {
    fn evaluate(
        input: Observed,
    ) -> Result<
        ProposedStateOutcome<ApprovedIncrease, IncreaseFailure>,
        InvocationDiagnostic,
    > {
        let current = input.observation.decode_uint256_as_u64()?;
        let increment = input.policy.increment.get();
        let ceiling = input.policy.ceiling;

        let Some(proposed) = current.checked_add(increment) else {
            return Ok(ProposedStateOutcome::Failure {
                failure: IncreaseFailure::arithmetic_overflow(
                    input.observation,
                    input.policy,
                    current,
                )?,
            });
        };

        if proposed > ceiling {
            return Ok(ProposedStateOutcome::Failure {
                failure: IncreaseFailure::ceiling_exceeded(
                    input.observation,
                    input.policy,
                    current,
                    proposed,
                )?,
            });
        }

        Ok(ProposedStateOutcome::Success {
            output: ApprovedIncrease::checked(
                input.observation,
                input.policy,
                current,
                proposed,
                ceiling - proposed,
            )?,
        })
    }
}
```

These are deterministic callbacks without IO, clock access, randomness, or mutable globals.
Checked business constructors are legitimate extension work. They must use the existing
source-preserving diagnostic conventions rather than flattening errors to strings.

## Shared composition and execution surface

Use the same current `OperationExpansion` DSL and single Runtime execution surface as the
composition E2E. This case reuses the unchanged maintained operation (observed value `42`), then
adds the new semantic State; it does not insert scenario 2's `+42` State. Existing components carry
the admitted policy as a context sibling. The final `Observed` contract matches the State input.

As in the composition sketch, exact generic/map arguments below are abbreviated. Production
component descriptors supply ordinary requirements; the extension author owns any intentional
new business failure projection. The closure authors a sequence and performs no runtime work.

```rust
let original = ContractWorkflow::new(&config)?;
let workflow = config.author_extension(|body| {
    body.operation(&original)?;
    body.pure::<AssessIncrease>()
})?;

let runtime = Runtime::builder(store)
    .capabilities(capabilities)
    .execution_policy(attempt_policy)
    .build()?;
let outcome = runtime
    .execute(run_id, workflow, config.initial_input())
    .await?;

match outcome {
    ExecutionOutcome::Succeeded(report) => {
        let approval = report.output();
        assert_eq!(approval.current(), 42);
        assert_eq!(approval.proposed(), 50);
        assert_eq!(approval.headroom(), 50);
    }
    other => fail_with_public_diagnostics(other),
}
```

`author_extension` is an illustrative facade for the current authoring DSL, not a second compiler.
Unlike scenario 2's same-contract recomposition, this extension declares its new output/failure
contracts. Exact signatures and root-map selection must be validated in the consuming example.
Runtime qualifies the input, Program and exact implementation requirements before external execution.
Existing lowering validates adjacent contracts; failure projections preserve originals and intentional
product semantics. No new chaining algebra or universal failure representation is required.

One authoring traversal must supply exact generic State/map/capability selections to Runtime
association. A Program-owned implementation sink remains a candidate; domain crates acquire no
Runtime dependency and callers maintain no second registration list.

Runtime follows the [RFC action contract](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md#concrete-architect-designs)
and owns bounded ordinary progression. It never grants extra recovery allowances or changes admitted
policy. Deadline returns the last qualified observation with available recovery identity, which need
not be the current head after an ambiguous append. A stopped attempt differs from durable failure.

## Real infrastructure and independent oracle

Use managed PostgreSQL run history and the separate transaction-authority schema, pinned Reth,
the compiled first-party Solidity fixture, and explicit signer and adapter handles. Retain the
development-only settlement scope. No in-memory replacement establishes this acceptance result.
Fixture provisioning, funding, Solidity compilation, and deliberate fault hooks remain distinct
from the public authoring journey. Secrets stay outside admitted values, reports, and diagnostics.

The test independently establishes:

- Deployment and configuration succeeded for the expected contract and receipt block.
- A separate fixture RPC read at that exact anchor returns `42`. The oracle does not rely solely
  on the production result decoder under test.
- The approval contains current `42`, increment `8`, ceiling `100`, proposed `50`, and headroom `50`.
- The retained observation target, anchor, and evidence linkage agree with the transaction facts.
- The ceiling-`49` run ends with the original typed `CeilingExceeded` failure, including the
  complete policy and observation, retained through the declared root failure projection.
- Rebuilt Runtime and Store handles decode the same checked terminal result and failure.
- Repeated terminal observation/resume preserves the head, output identity, and sender nonce.

Cold reconstruction means fresh handles, not host-process key recovery. Unchanged head/output/
nonce does not alone prove that no provider calls occurred. Prove a stronger no-IO claim only with
explicit instrumentation. Test helpers may assert results; they must not recreate ordinary
execution policy or maintain a second business implementation as the sole expected-value oracle.

## Named case runs

| Case | Status | Independent expectation |
| --- | --- | --- |
| `approves_observed_increase` | Baseline | The existing live workflow feeds the custom State; approval is `50`, headroom `50`. |
| `rejects_increase_above_ceiling` | Baseline | Ceiling `49` yields the permanent typed rejection with retained original facts. |
| `reloads_terminal_approval_and_rejection` | Baseline | Fresh handles recover exact checked contracts; terminal read/resume leaves authority unchanged. |
| `resumes_before_custom_state` | Additional | Stop after acknowledged observation, rebuild, then evaluate; compare semantic result against an uninterrupted run. |
| `rejects_wrong_custom_implementation` | Additional | An incompatible implementation identity fails exact association without append. |
| `preserves_custom_failure_after_append_ambiguity` | Future | Resume resolves committed-or-absent append without inventing a durable audit record. |

Focused tests cover overflow, zero increment rejection, malformed custom values and impossible
success/failure field combinations. Consuming-crate compile-fail coverage checks wrong adjacent
contracts. These cases do not need repeated live infrastructure merely because they concern the
extension API.

Existing transaction fault coverage stays with the composition scenario or its named focused
owners. This extension scenario does not replace those assertions by implication. Future cases
cannot justify deleting currently exercised guarantees.

## Ownership, deletion, and verification

The caller owns business policy, checked business contracts, implementation identity, arithmetic,
classification, chosen sequence, and intentional recovery decisions. MFM owns canonical machinery,
exact association, ordinary failure injection, maintained ABI decoding, capability binding,
Runtime progression, persistence, recovery, and checked result delivery.

Implement shared composition/association first with the composition E2E, deleting superseded
parallel registration paths in that cutover. Implement the Runtime execution surface and ABI/result
support once, deleting replaced fixture polling/decoding when callers switch. Then add this
public-import-only extension example, focused contract tests, managed selection, and documentation.
Remove the predecessor E2E organization only after its complete coverage mapping is satisfied.

Do not add an extension registry, custom scheduler, expression language, or second execution model.
Necessary complexity is the new semantic State and its contracts plus shared public conveniences;
report actual production LOC changes during implementation, not a speculative reduction here.

Follow [build and verification](build-and-verification.md): focused affected crates and consuming
examples run inside Nix, followed by the managed scenario. Cross-crate public APIs and task changes
require affected focused coverage and one final CI on the exact candidate. This design document
alone requires link review and whitespace checking, not executable gates.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Typed named contexts preserve admitted policy through existing components without custom glue. | The final composition API is proposed. | A helper State could conceal the composition defect. | Compile a downstream example sharing the exact components with E2E 2 before adding wrappers. |
| Maintained ABI support offers checked `uint256` to `u64` projection. | This convenience is not established today. | Ad hoc decoding or lossy range handling enters the business State. | Specify bounded decoding and preserved range/parser causes; full-width checked integers are an alternative if simpler. |
| Terminal business approval adequately demonstrates the extension objective. | It does not exercise dynamically constructing a later transaction. | Another extension case may be needed for that separate requirement. | Accept the approval/rejection baseline, and add downstream use only for a concrete required workflow. |
