# E2E design: compose existing Operations and States

This is the target design for public usage 2 in the
[public interfaces RFC](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md). It specifies the experience to
implement, not a claim that the proposed APIs already exist. Current API limitations must not
become acceptance criteria. This document changes no executable test or managed task.

## Objective and user story

A Rust caller deploys a contract, configures its value to `42`, and observes `value()` at the
configuration transaction's receipt block. The caller selects two existing Operations and one
individual existing State, connects their typed inputs and outputs, binds explicit capabilities,
and receives a checked result through the framework's ordinary execution surface.

No new State is allowed in this scenario. In particular, a test-owned decoder, projection or
adapter State must not conceal missing public composition support. The test may own its context,
sequence and intentional policy. It does not require an Application product or transport.

The composition consists of:

1. `EvmTransaction<Initial, CreateAt<DeploymentSlot>>` creates the contract.
2. `EvmTransaction<Deployment, CallCreatedAt<ConfigurationSlot, DeploymentSlot>>` configures the
   created address using a checked call plan.
3. `ReadAnchoredContractCall<Configuration, ObserveAt<ObservationSlot, ConfigurationSlot>>`
   observes the configured target at the successful call's receipt anchor.

The result retains both complete transaction histories and the accepted anchored observation.
The dependencies are meaningful: the address does not exist before deployment, and the observation
must use the configuration receipt rather than an arbitrary latest block.

## Caller declarations and target calls

Named typed context fields are legitimate caller authoring. Their exact schemas change as existing
components replace plans with checked facts; unrelated fields remain intact. These declarations do
not introduce a dynamic context bag or a runtime-selected expression language.

The following is a proposed public API sketch. `operation`, `read`, `then`, `Runtime::prepare`,
`RuntimeDriver` and the checked ABI accessor are target improvements, not current signatures.
The context and transaction recipe concepts already exist. Routine checked fixture constructors
and imports are omitted; no hidden helper may implement the sequence or its execution machinery.

```rust
#[derive(MfmValue, MfmContext, Serialize, Deserialize)]
#[context(namespace = "example.configure-contract")]
struct ContractContext<D, C, O> {
    label: Label,
    deployment: D,
    configuration: C,
    observation: O,
}

type Initial = ContractContext<
    CheckedCreatePlan,
    CheckedCallPlan,
    CheckedObservationPlan,
>;
type Deploy = EvmTransaction<Initial, CreateAt<ContractContextDeploymentSlot>>;
type Deployment = <Deploy as Operation>::Output;
type Configure = EvmTransaction<
    Deployment,
    CallCreatedAt<ContractContextConfigurationSlot, ContractContextDeploymentSlot>,
>;
type Configuration = <Configure as Operation>::Output;
type Observe = ReadAnchoredContractCall<
    Configuration,
    ObserveAt<ContractContextObservationSlot, ContractContextConfigurationSlot>,
>;

let workflow = operation(Deploy::new(binding.clone()))
    .then(operation(Configure::new(binding.clone())))
    .then(read::<Observe, EvmAnchoredContractCallRead>(binding.route.clone()));

let input = Initial {
    label,
    deployment: checked_deployment_plan,
    configuration: checked_configuration_plan, // configure(uint256), argument 42
    observation: checked_observation_plan,     // value(), exact uint256 return
};

let prepared = Runtime::prepare(
    entry_point,
    workflow,
    input,
    admitted_policy,
    adapters,
)?;
let runtime = Runtime::new(prepared.assembly(), store);
let outcome = RuntimeDriver::new(runtime)
    .execute(run_id, &prepared, attempt_policy)
    .await?;

let success = expect_success(outcome); // Assertion helper only.
let observed = success.output();
assert_eq!(observed.label, expected_label);
assert_eq!(
    observed.observation.decode_return::<U256>(&value_function)?,
    U256::from(42),
);
```

The ABI accessor checks the selected function against the observation calldata and exact return
contract. It must not merely cast bytes into a requested type. Production owns the supported static
ABI codecs and checked plan constructors; the test owns the independently chosen expected value.
The successful workflow result is the typed observed context, without a duplicate fixture report.

## One authoring traversal and exact executable association

Program owns a typed sequence combinator: `Then<A, B>` requires `A::Output = B::Input`. This checks
adjacency without weakening exact checked value contracts. Existing context slots remain mechanical
field replacement; domain fact constructors continue owning transaction relationships.

Extend the existing sole `OperationExpansion` lowering traversal with a Program-owned generic
implementation sink. Operation expansion threads the sink through children and capability injection.
At each selected Pure/Read/Effect State, map, handler and required codec, the traversal exposes the
exact monomorphized contracts while the same private symbolic draft constructs Program.

Runtime implements the sink by installing executable hooks into its existing assembly. The injected
before/designated/after sequence uses the same sink, including reservation and preparation States.
A Program-only consumer can use a sink with no executable collection. Domain crates depend only on
Program's authoring contract, never Runtime, Store or live handles. No authoring callback is retained
for Runtime progression.

This replaces independently maintained executable lists. It does not introduce a parallel registry,
second lowering pass, second scheduler or semantic-identity fallback. Existing exact ABI association
remains authoritative; repeated equivalent registrations may deduplicate, and conflicting ones fail.
Failed expansion must leave no usable partial prepared result.

Explicit adapters provide capability bindings, provider, signer and transaction-authority handles.
They are distinct from State selection because they grant external authority. Preparation qualifies
the initial value, retains its exact commitment, lowers the sequence, collects exact executable
hooks, and checks required bindings without provider or Store calls. The resulting typed prepared
value carries the Program/input and terminal contracts; it is not a second persisted control format.

The implementation feasibility proof must exercise two context monomorphizations, nested Operations,
an individual State, explicit custom map and handler, injected State families, and failed expansion.
The sketch alone does not establish that the necessary Rust bounds and transactional construction
are correct. Complete this consuming proof before cutting over production callers.

## Ordinary failure composition

`Then<A, B>::Failure` is a framework-owned `SequenceFailure<A::Failure, B::Failure>` with typed
`Earlier` and `Next` variants and an exact deterministic generic descriptor. Nested composition
produces nested alternatives; it does not erase causes into JSON or a universal failure bag.
Classification delegates to the original selected payload.

Framework-owned branch-injection maps retain child root values in these alternatives. The same
traversal supplies their exact executable hooks. Callers write an explicit map only when they choose
a different public failure schema or intentional product semantics.

Runtime continues retaining the complete original execution failure and its provenance separately
from root projection. Operational failures may legitimately have no mapped domain root. Root/report
capacity rejection cannot discard the acknowledged original or unresolved command authority.
Typed cold decoding, custom-root mapping and small-limit overflow need focused coverage.

## Execution and checked results

`RuntimeDriver` is one maintained ordinary driver over Runtime, shared with Application and the
extension scenario. It owns bounded waiting, not transitions or new recovery authority. An attempt
policy controls its deadline and polling. The admitted Program policy owns semantic retry/restart
allowances; a longer deadline cannot change those allowances.

Deadline expiry is a stopped invocation, not a manufactured durable failure. It retains the exact
RunId and any last qualified observation. That observation can predate an acknowledged append whose
projection failed: the outcome must preserve known acknowledgement/authority separately and must
not label an older observation the latest acknowledged head. Cancellation and unavailable observations
must remain honest about what was actually observed or acknowledged.

The driver follows the [shared action contract](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md#concrete-architect-designs):
`RecoveryStopped` and invocation/Store failures end automatic driving. Ordinary pending polling
reconciles only the same retained command. It must not silently repeat ambiguous admission/appends
or replace pending authority. Cold
continuation uses the same RunId and checks retained Program identity; exact schema checks govern
hot and cold output access. Deliberate boundary cases may call Runtime progression directly, but
ordinary callers must not rebuild `drive_to_success` or manually decode terminal JSON.

## Infrastructure, oracle and baseline assertions

Use pinned Reth, split-role PostgreSQL with the optional transaction-authority surface, and the
first-party contract compiled with pinned solc. Compiler output stays temporary. A fresh funded
signer has explicit custody lifetime; no secret enters admitted input, Program, output or diagnostics.
Fixed checked gas/fee options suffice. Wallet generation and funding remain setup, not new workflow
capabilities required by this design. Settlement retains its non-reorging development-only scope.

The baseline independently asserts label preservation, expected command parameters and binding,
creation success, configuration target equal to the created address, and the observation's target
and block hash/number equal to the configuration receipt. The checked getter result equals `42`.

A separate node query checks deployed code and receipt identity. Its independently decoded expected
result must not reuse the workflow's own report construction to prove correctness. Fresh reservations
use nonces `0` and `1`, and external pending nonce becomes `2`. Assertions retain the returned facts;
they do not reconstruct the transaction protocol inside the test.

The test owns order, fields, recipe choices, policy, RunId and expected answers. Production owns
typed association, ordinary failure injection, ABI support, execution and result access. Fixture
code owns services, funding, independent observations and explicit fault injection. Fault controls
must not become production operation options.

## Case runs and replacement coverage

| Named case | Status and evidence |
| --- | --- |
| `composes_transactions_and_anchored_state` | New clean baseline; both Operations and the individual existing State produce the expected typed result. |
| `lost_reservation_ack_resumes_same_command` | Preserve committed reservation acknowledgement loss and Runtime reconstruction from the existing Effect E2E. Keep the same keystore owner alive; do not claim process-restart key recovery. |
| `external_nonce_advance_affects_only_fresh_transaction` | Preserve independent external transfer, then a fresh ordinary call reserving nonce `3`; pending nonce moves `2 -> 4`. |
| `cold_terminal_observation_preserves_head_output_nonce` | Preserve exact terminal head/output/nonce under cold read and resume. These assertions do not establish zero provider calls. |
| `authority_sql_failure_preserves_hot_cold_causes` | Preserve actual authority SQL failure and retained layers through cold Application observation; no reservation precedes successful authority access. |
| `retained_epoch_mismatch_is_internal_without_append` | Preserve internal classification and unchanged acknowledged history. |
| `closed_signer_owner_retains_distinct_failure` | Preserve signer cause, classification and unchanged transaction authority through cold observation. |
| Prepared-wire/rejecting-signer recovery, cancellation, ambiguous transaction appends | Preserve existing focused `transaction_tests.rs` ownership unless explicitly relocated with equivalent assertions. |
| Mismatched adjacency, missing binding, wrong typed result, malformed ABI return | Required focused consuming/compile-fail checks for the proposed APIs; no live node required where the owning boundary suffices. |
| Driver deadline, cancellation and ambiguous acknowledgement | Required focused outcome/authority checks for the new driver; additional live case runs are future enhancements. |

The starting assertions live in
[the Effect E2E](../crates/live/evm/tests/evm_contract_effect_e2e.rs), with fixture composition in
[contract_workflow.rs](../crates/live/evm/tests/support/contract_workflow.rs) and focused protocol
coverage in [transaction_tests.rs](../crates/live/evm/src/transaction_tests.rs). Before deletion,
record each old assertion's executable replacement and managed owner. Future cases never justify
removing currently exercised guarantees. The managed `effect-e2e` identity can initially remain while
its selection and documentation change with the executable replacement.

## Complete cutovers and verification

1. Implement typed composition and one-traversal executable selection with consuming tests. Update
   all affected authoring/registration users and authoritative contracts together; delete superseded
   registration machinery. Preserve explicit capability injection and custom mapping/handler support.
2. Implement the shared driver, exact typed results and necessary static ABI support; remove ordinary
   copied drivers/codecs. Combine with the first cutover if they cannot form coherent separate commits.
3. Replace the Effect E2E with this scenario and preserved named cases. Remove superseded
   `EffectFixtureOperation`, `register_fixture_states`, `DecodeValue`, duplicate `FixtureReport`
   validation and mechanical failure reconstruction/maps. Retain independent oracles and fault setup.
4. Update managed coverage and run focused affected Program/Runtime/domain/live tests, managed case
   runs and one final CI according to the [build guide](build-and-verification.md).

The necessary added concepts are typed composition, typed failure alternatives and an implementation
sink. They are justified only by deleting duplicate assembly/mapping obligations. This design revision
changes production-code LOC by zero; implementation commits must report actual additions/removals.
Documentation-only validation is link/contract review and `git diff --check`, without Rust gates.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| One generic sink captures exact hooks through the existing lowering. | Operation, map, handler and injection bounds must also preserve failed-expansion atomicity. | A second registration inventory or lowering could reappear. | Prove nested Operations, two contexts, custom maps/handlers, injection and failure rollback in a consuming crate before cutover. |
| Typed sequence alternatives suffice for ordinary failure composition. | Exact descriptors, classification and aggregate report limits interact. | Cold decoding or original-cause retention could regress. | Exercise every branch, operational failure without root, custom roots, small-limit overflow and cold decoding. |
| Static checked ABI support suffices for this scenario. | Maintained function/return types do not yet expose the whole desired call surface. | The public sketch could hide unchecked fixture codecs. | Prove configure/getter encoding and wrong-selector/return-shape rejection without adding arbitrary runtime expressions. |
| One bounded driver preserves every authority distinction. | Deadline, cancellation, uncertain admission and post-acknowledgement projection failure differ. | A stopped result could duplicate work or overclaim its observed head. | Define and test the outcome/action matrix, exact recovery identity and acknowledged-authority versus observation distinction before implementation. |
